use indexmap::IndexMap;

use crate::ast::{Param, ParamKind};
use crate::error::IonError;
use crate::hash;
use crate::value::{HostSignature, Value};

#[derive(Debug, Clone)]
pub struct KeywordArg {
    pub source_name: String,
    pub name_hash: u64,
    pub value: Value,
}

impl KeywordArg {
    pub fn new(source_name: String, value: Value) -> Self {
        let name_hash = hash::h(source_name.as_str());
        Self {
            source_name,
            name_hash,
            value,
        }
    }
}

pub fn keyword_args_from_dict(map: IndexMap<String, Value>) -> Vec<KeywordArg> {
    map.into_iter()
        .map(|(name, value)| KeywordArg::new(name, value))
        .collect()
}

pub fn resolve_host_call(
    signature: &HostSignature,
    positional: Vec<Value>,
    named: Vec<KeywordArg>,
    line: usize,
    col: usize,
) -> Result<Vec<Value>, IonError> {
    let mut slots = resolve_host_slots(signature, positional, named, line, col)?;
    let mut resolved = Vec::with_capacity(signature.params.len());
    for (idx, param) in signature.params.iter().enumerate() {
        let value = match slots[idx].take() {
            Some(value) => value,
            None => match param.kind {
                ParamKind::VarArgs => Value::List(Vec::new()),
                ParamKind::VarKwargs => Value::Dict(IndexMap::new()),
                _ => param.default.clone().ok_or_else(|| {
                    IonError::runtime(
                        format!(
                            "{}{}",
                            ion_str!("missing argument "),
                            host_param_display(param.name_hash)
                        ),
                        line,
                        col,
                    )
                })?,
            },
        };
        resolved.push(value);
    }
    Ok(resolved)
}

pub fn resolve_ion_slots(
    params: &[Param],
    function_name: &str,
    positional: Vec<Value>,
    named: Vec<KeywordArg>,
    line: usize,
    col: usize,
) -> Result<Vec<Option<Value>>, IonError> {
    let mut slots = vec![None; params.len()];
    bind_positionals(params, &mut slots, positional, function_name, line, col)?;

    let var_kwargs_idx = params
        .iter()
        .position(|param| param.kind == ParamKind::VarKwargs);
    let mut leftover = IndexMap::new();
    for kw in named {
        let bindable = params.iter().position(|param| {
            param.name == kw.source_name
                && !matches!(
                    param.kind,
                    ParamKind::PositionalOnly | ParamKind::VarArgs | ParamKind::VarKwargs
                )
        });
        match bindable {
            Some(idx) => {
                if slots[idx].is_some() {
                    return Err(duplicate_argument(&kw.source_name, line, col));
                }
                slots[idx] = Some(kw.value);
            }
            None if var_kwargs_idx.is_some() => {
                if leftover.contains_key(&kw.source_name) {
                    return Err(duplicate_argument(&kw.source_name, line, col));
                }
                leftover.insert(kw.source_name, kw.value);
            }
            None => {
                return Err(IonError::runtime(
                    format!(
                        "{}'{}'{}'{}'",
                        ion_str!("unknown parameter '"),
                        kw.source_name,
                        ion_str!("' for function '"),
                        function_name
                    ),
                    line,
                    col,
                ));
            }
        }
    }

    if let Some(idx) = var_kwargs_idx {
        slots[idx] = Some(Value::Dict(leftover));
    }
    Ok(slots)
}

fn resolve_host_slots(
    signature: &HostSignature,
    positional: Vec<Value>,
    named: Vec<KeywordArg>,
    line: usize,
    col: usize,
) -> Result<Vec<Option<Value>>, IonError> {
    let mut slots = vec![None; signature.params.len()];
    bind_positionals(
        &signature.params,
        &mut slots,
        positional,
        ion_static_str!("host function"),
        line,
        col,
    )?;

    let var_kwargs_idx = signature
        .params
        .iter()
        .position(|param| param.kind == ParamKind::VarKwargs);
    let mut leftover = IndexMap::new();
    for kw in named {
        let bindable = signature.params.iter().position(|param| {
            param.name_hash == kw.name_hash
                && !matches!(
                    param.kind,
                    ParamKind::PositionalOnly | ParamKind::VarArgs | ParamKind::VarKwargs
                )
        });
        match bindable {
            Some(idx) => {
                if slots[idx].is_some() {
                    return Err(duplicate_argument(&kw.source_name, line, col));
                }
                slots[idx] = Some(kw.value);
            }
            None if var_kwargs_idx.is_some() => {
                if leftover.contains_key(&kw.source_name) {
                    return Err(duplicate_argument(&kw.source_name, line, col));
                }
                leftover.insert(kw.source_name, kw.value);
            }
            None => {
                return Err(IonError::runtime(
                    format!(
                        "{}'{}'",
                        ion_str!("unexpected keyword argument "),
                        kw.source_name
                    ),
                    line,
                    col,
                ));
            }
        }
    }
    if let Some(idx) = var_kwargs_idx {
        slots[idx] = Some(Value::Dict(leftover));
    }
    Ok(slots)
}

trait ParamView {
    fn kind(&self) -> ParamKind;
}

impl ParamView for Param {
    fn kind(&self) -> ParamKind {
        self.kind
    }
}

impl ParamView for crate::value::HostParam {
    fn kind(&self) -> ParamKind {
        self.kind
    }
}

fn bind_positionals<P: ParamView>(
    params: &[P],
    slots: &mut [Option<Value>],
    positional: Vec<Value>,
    function_name: &str,
    line: usize,
    col: usize,
) -> Result<(), IonError> {
    let supplied_count = positional.len();
    let mut pos_iter = positional.into_iter();
    let mut consumed = 0usize;
    for (idx, param) in params.iter().enumerate() {
        match param.kind() {
            ParamKind::VarArgs => {
                let rest: Vec<Value> = pos_iter.collect();
                consumed = supplied_count;
                slots[idx] = Some(Value::List(rest));
                break;
            }
            ParamKind::KeywordOnly | ParamKind::VarKwargs => break,
            ParamKind::Positional | ParamKind::PositionalOnly => match pos_iter.next() {
                Some(value) => {
                    consumed += 1;
                    slots[idx] = Some(value);
                }
                None => break,
            },
        }
    }
    if consumed < supplied_count {
        return Err(IonError::runtime(
            format!(
                "{}'{}'{}{}{}{}",
                ion_str!("function '"),
                function_name,
                ion_str!("' got "),
                supplied_count,
                ion_str!(" positional arguments but accepts "),
                consumed
            ),
            line,
            col,
        ));
    }
    Ok(())
}

fn duplicate_argument(name: &str, line: usize, col: usize) -> IonError {
    IonError::runtime(
        format!("{}'{}'", ion_str!("duplicate argument "), name),
        line,
        col,
    )
}

fn host_param_display(name_hash: u64) -> String {
    crate::names::lookup(name_hash)
        .map(str::to_string)
        .unwrap_or_else(|| format!("#{name_hash:016x}"))
}
