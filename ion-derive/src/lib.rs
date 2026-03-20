use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Fields};

#[proc_macro_derive(IonType)]
pub fn derive_ion_type(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let name_str = name.to_string();

    match &input.data {
        Data::Struct(data) => derive_struct(name, &name_str, data),
        Data::Enum(data) => derive_enum(name, &name_str, data),
        Data::Union(_) => {
            syn::Error::new_spanned(name, "IonType cannot be derived for unions")
                .to_compile_error()
                .into()
        }
    }
}

fn derive_struct(
    name: &syn::Ident,
    name_str: &str,
    data: &syn::DataStruct,
) -> TokenStream {
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

    // to_ion: convert each field
    let to_ion_fields = field_names.iter().zip(field_name_strs.iter()).map(|(ident, name_s)| {
        quote! {
            fields.insert(#name_s.to_string(), ion_core::host_types::IonType::to_ion(&self.#ident));
        }
    });

    // from_ion: extract each field
    let from_ion_fields = field_names.iter().zip(field_name_strs.iter()).map(|(ident, name_s)| {
        quote! {
            #ident: {
                let v = fields.get(#name_s)
                    .ok_or_else(|| format!("missing field '{}' in {}", #name_s, #name_str))?;
                ion_core::host_types::IonType::from_ion(v)?
            },
        }
    });

    // ion_type_def: field name list
    let def_fields = field_name_strs.iter().map(|s| {
        quote! { #s.to_string() }
    });

    let expanded = quote! {
        impl ion_core::host_types::IonType for #name {
            fn to_ion(&self) -> ion_core::value::Value {
                let mut fields = indexmap::IndexMap::new();
                #(#to_ion_fields)*
                ion_core::value::Value::HostStruct {
                    type_name: #name_str.to_string(),
                    fields,
                }
            }

            fn from_ion(val: &ion_core::value::Value) -> Result<Self, String> {
                if let ion_core::value::Value::HostStruct { type_name, fields } = val {
                    if type_name != #name_str {
                        return Err(format!("expected {}, got {}", #name_str, type_name));
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
                        name: #name_str.to_string(),
                        fields: vec![#(#def_fields),*],
                    }
                )
            }
        }
    };

    expanded.into()
}

fn derive_enum(
    name: &syn::Ident,
    name_str: &str,
    data: &syn::DataEnum,
) -> TokenStream {
    let variants = &data.variants;

    // ion_type_def: variant definitions
    let variant_defs = variants.iter().map(|v| {
        let vname = v.ident.to_string();
        let arity = match &v.fields {
            Fields::Unit => 0usize,
            Fields::Unnamed(f) => f.unnamed.len(),
            Fields::Named(f) => f.named.len(),
        };
        quote! {
            ion_core::host_types::HostVariantDef {
                name: #vname.to_string(),
                arity: #arity,
            }
        }
    });

    // to_ion arms
    let to_ion_arms = variants.iter().map(|v| {
        let vident = &v.ident;
        let vname = v.ident.to_string();
        match &v.fields {
            Fields::Unit => {
                quote! {
                    #name::#vident => ion_core::value::Value::HostEnum {
                        enum_name: #name_str.to_string(),
                        variant: #vname.to_string(),
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
                        enum_name: #name_str.to_string(),
                        variant: #vname.to_string(),
                        data: vec![#(#to_ions),*],
                    },
                }
            }
            Fields::Named(_) => {
                quote! {
                    _ => unimplemented!("named enum fields not yet supported"),
                }
            }
        }
    });

    // from_ion arms
    let from_ion_arms = variants.iter().map(|v| {
        let vident = &v.ident;
        let vname = v.ident.to_string();
        match &v.fields {
            Fields::Unit => {
                quote! {
                    #vname => {
                        if !data.is_empty() {
                            return Err(format!("{}::{} takes no arguments", #name_str, #vname));
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
                    #vname => {
                        if data.len() != #count {
                            return Err(format!("{}::{} expects {} arguments, got {}", #name_str, #vname, #count, data.len()));
                        }
                        Ok(#name::#vident(#(#extracts),*))
                    }
                }
            }
            Fields::Named(_) => {
                quote! {
                    _ => Err(format!("named enum fields not yet supported"))
                }
            }
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
                if let ion_core::value::Value::HostEnum { enum_name, variant, data } = val {
                    if enum_name != #name_str {
                        return Err(format!("expected {}, got {}", #name_str, enum_name));
                    }
                    match variant.as_str() {
                        #(#from_ion_arms)*
                        _ => Err(format!("unknown variant '{}' in {}", variant, #name_str)),
                    }
                } else {
                    Err(format!("expected {}, got {}", #name_str, val.type_name()))
                }
            }

            fn ion_type_def() -> ion_core::host_types::IonTypeDef {
                ion_core::host_types::IonTypeDef::Enum(
                    ion_core::host_types::HostEnumDef {
                        name: #name_str.to_string(),
                        variants: vec![#(#variant_defs),*],
                    }
                )
            }
        }
    };

    expanded.into()
}
