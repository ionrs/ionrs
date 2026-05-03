use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Fields};

/// FNV-1a 64-bit hash. Mirrors `ion_core::hash::fnv1a64` so the macro can
/// fold name strings into `u64` literals at expansion time. Keep in sync.
const fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut i = 0;
    while i < bytes.len() {
        h ^= bytes[i] as u64;
        h = h.wrapping_mul(0x100000001b3);
        i += 1;
    }
    h
}

fn h(s: &str) -> u64 {
    fnv1a64(s.as_bytes())
}

#[proc_macro_derive(IonType)]
pub fn derive_ion_type(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let name_str = name.to_string();

    match &input.data {
        Data::Struct(data) => derive_struct(name, &name_str, data),
        Data::Enum(data) => derive_enum(name, &name_str, data),
        Data::Union(_) => syn::Error::new_spanned(name, "IonType cannot be derived for unions")
            .to_compile_error()
            .into(),
    }
}

fn derive_struct(name: &syn::Ident, name_str: &str, data: &syn::DataStruct) -> TokenStream {
    let fields = match &data.fields {
        Fields::Named(f) => &f.named,
        _ => {
            return syn::Error::new_spanned(name, "IonType only supports named struct fields")
                .to_compile_error()
                .into();
        }
    };

    let field_names: Vec<_> = fields.iter().map(|f| f.ident.as_ref().unwrap()).collect();
    let field_name_strs: Vec<String> = field_names.iter().map(|f| f.to_string()).collect();
    let field_hashes: Vec<u64> = field_name_strs.iter().map(|s| h(s)).collect();
    let type_hash: u64 = h(name_str);

    let to_ion_fields = field_names.iter().zip(field_hashes.iter()).map(|(ident, fh)| {
        quote! {
            fields.insert(#fh, ion_core::host_types::IonType::to_ion(&self.#ident));
        }
    });

    let from_ion_fields = field_names
        .iter()
        .zip(field_hashes.iter())
        .map(|(ident, fh)| {
            quote! {
                #ident: {
                    let v = fields.get(&#fh)
                        .ok_or_else(|| format!("missing field in {}", #name_str))?;
                    ion_core::host_types::IonType::from_ion(v)?
                },
            }
        });

    let def_field_hashes = field_hashes.iter().map(|fh| quote! { #fh });

    let expanded = quote! {
        impl ion_core::host_types::IonType for #name {
            fn to_ion(&self) -> ion_core::value::Value {
                let mut fields: indexmap::IndexMap<u64, ion_core::value::Value> =
                    indexmap::IndexMap::new();
                #(#to_ion_fields)*
                ion_core::value::Value::HostStruct {
                    type_hash: #type_hash,
                    fields,
                }
            }

            fn from_ion(val: &ion_core::value::Value) -> Result<Self, String> {
                if let ion_core::value::Value::HostStruct { type_hash, fields } = val {
                    if *type_hash != #type_hash {
                        return Err(format!("expected {}, got different host struct", #name_str));
                    }
                    Ok(Self {
                        #(#from_ion_fields)*
                    })
                } else {
                    Err(format!("expected {}, got {}", #name_str, val.type_name()))
                }
            }

            fn ion_type_def() -> ion_core::host_types::IonTypeDef {
                ion_core::host_types::IonTypeDef::Struct(
                    ion_core::host_types::HostStructDef {
                        name_hash: #type_hash,
                        fields: vec![#(#def_field_hashes),*],
                    }
                )
            }
        }
    };

    expanded.into()
}

fn derive_enum(name: &syn::Ident, name_str: &str, data: &syn::DataEnum) -> TokenStream {
    let variants = &data.variants;
    for variant in variants {
        if matches!(variant.fields, Fields::Named(_)) {
            return syn::Error::new_spanned(
                &variant.ident,
                "IonType does not support enum variants with named fields",
            )
            .to_compile_error()
            .into();
        }
    }

    let type_hash: u64 = h(name_str);

    // ion_type_def: variant definitions
    let variant_defs = variants.iter().map(|v| {
        let vh = h(&v.ident.to_string());
        let arity = match &v.fields {
            Fields::Unit => 0usize,
            Fields::Unnamed(f) => f.unnamed.len(),
            Fields::Named(_) => unreachable!("named enum fields rejected above"),
        };
        quote! {
            ion_core::host_types::HostVariantDef {
                name_hash: #vh,
                arity: #arity,
            }
        }
    });

    // to_ion arms
    let to_ion_arms = variants.iter().map(|v| {
        let vident = &v.ident;
        let vh = h(&v.ident.to_string());
        match &v.fields {
            Fields::Unit => {
                quote! {
                    #name::#vident => ion_core::value::Value::HostEnum {
                        enum_hash: #type_hash,
                        variant_hash: #vh,
                        data: vec![],
                    },
                }
            }
            Fields::Unnamed(fields) => {
                let bindings: Vec<_> = (0..fields.unnamed.len())
                    .map(|i| syn::Ident::new(&format!("f{}", i), proc_macro2::Span::call_site()))
                    .collect();
                let to_ions = bindings.iter().map(|b| {
                    quote! { ion_core::host_types::IonType::to_ion(#b) }
                });
                quote! {
                    #name::#vident(#(#bindings),*) => ion_core::value::Value::HostEnum {
                        enum_hash: #type_hash,
                        variant_hash: #vh,
                        data: vec![#(#to_ions),*],
                    },
                }
            }
            Fields::Named(_) => unreachable!("named enum fields rejected above"),
        }
    });

    // from_ion arms
    let from_ion_arms = variants.iter().map(|v| {
        let vident = &v.ident;
        let vh = h(&v.ident.to_string());
        match &v.fields {
            Fields::Unit => {
                quote! {
                    #vh => {
                        if !data.is_empty() {
                            return Err(format!("variant in {} takes no arguments", #name_str));
                        }
                        Ok(#name::#vident)
                    }
                }
            }
            Fields::Unnamed(fields) => {
                let count = fields.unnamed.len();
                let extracts: Vec<_> = (0..count)
                    .map(|i| {
                        quote! {
                            ion_core::host_types::IonType::from_ion(&data[#i])?
                        }
                    })
                    .collect();
                quote! {
                    #vh => {
                        if data.len() != #count {
                            return Err(format!("variant in {} expects {} arguments, got {}", #name_str, #count, data.len()));
                        }
                        Ok(#name::#vident(#(#extracts),*))
                    }
                }
            }
            Fields::Named(_) => unreachable!("named enum fields rejected above"),
        }
    });

    let expanded = quote! {
        impl ion_core::host_types::IonType for #name {
            fn to_ion(&self) -> ion_core::value::Value {
                match self {
                    #(#to_ion_arms)*
                }
            }

            fn from_ion(val: &ion_core::value::Value) -> Result<Self, String> {
                if let ion_core::value::Value::HostEnum { enum_hash, variant_hash, data } = val {
                    if *enum_hash != #type_hash {
                        return Err(format!("expected {}, got different host enum", #name_str));
                    }
                    match *variant_hash {
                        #(#from_ion_arms)*
                        _ => Err(format!("unknown variant in {}", #name_str)),
                    }
                } else {
                    Err(format!("expected {}, got {}", #name_str, val.type_name()))
                }
            }

            fn ion_type_def() -> ion_core::host_types::IonTypeDef {
                ion_core::host_types::IonTypeDef::Enum(
                    ion_core::host_types::HostEnumDef {
                        name_hash: #type_hash,
                        variants: vec![#(#variant_defs),*],
                    }
                )
            }
        }
    };

    expanded.into()
}
