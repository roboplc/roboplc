extern crate proc_macro;
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Fields, Meta, NestedMeta};

/// # Panics
///
/// Will panic on parse errors
#[allow(clippy::too_many_lines)]
#[proc_macro_derive(DataPolicy, attributes(data_delivery, data_priority, data_expires))]
pub fn data_policy_derive(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);

    match ast.data {
        Data::Enum(ref data_enum) => {
            let enum_name = &ast.ident;
            let mut delivery_policy_cases = vec![];
            let mut priority_cases = vec![];
            let mut expires_cases = vec![];
            let mut default_policy_impl = true;
            let mut default_priority_impl = true;
            let mut default_expires_impl = true;

            for variant in &data_enum.variants {
                let variant_name = &variant.ident;
                let mut priority_value = quote! { 100 };
                let mut delivery_policy_value = quote! { ::roboplc::DeliveryPolicy::Always };
                let mut expires_value = quote! { false };

                for attr in &variant.attrs {
                    if attr.path.is_ident("data_delivery") {
                        default_policy_impl = false;
                        if let Meta::List(meta_list) = attr.parse_meta().unwrap() {
                            for nested_meta in meta_list.nested {
                                if let NestedMeta::Meta(meta) = nested_meta {
                                    match meta
                                        .path()
                                        .get_ident()
                                        .map(ToString::to_string)
                                        .as_deref()
                                    {
                                        Some("single") => {
                                            delivery_policy_value =
                                                quote! { ::roboplc::DeliveryPolicy::Single }
                                        }
                                        Some("single_optional") => {
                                            delivery_policy_value =
                                                quote! { ::roboplc::DeliveryPolicy::SingleOptional }
                                        }
                                        Some("optional") => {
                                            delivery_policy_value =
                                                quote! { ::roboplc::DeliveryPolicy::Optional }
                                        }
                                        Some("always") => {
                                            delivery_policy_value =
                                                quote! { ::roboplc::DeliveryPolicy::Always }
                                        }
                                        Some(v) => panic!("Unknown policy variant: {}", v),
                                        None => panic!("Policy variant not specified"),
                                    }
                                }
                            }
                        } else {
                            panic!("unable to parse data_delivery attribute");
                        }
                    } else if attr.path.is_ident("data_expires") {
                        default_expires_impl = false;
                        if let Meta::List(meta_list) = attr.parse_meta().unwrap() {
                            for nested_meta in meta_list.nested {
                                if let NestedMeta::Meta(lit) = nested_meta {
                                    expires_value = quote! { #lit(value) }
                                } else {
                                    panic!("data_expires value must be a function",);
                                }
                            }
                        } else {
                            panic!("unable to parse data_expires attribute");
                        }
                    } else if attr.path.is_ident("data_priority") {
                        default_priority_impl = false;
                        if let Ok(Meta::List(meta_list)) = attr.parse_meta() {
                            for nested_meta in meta_list.nested {
                                if let NestedMeta::Lit(lit_int) = nested_meta {
                                    priority_value = quote! { #lit_int };
                                } else {
                                    panic!("data_priority value must be an integer");
                                }
                            }
                        } else {
                            panic!("unable to parse data_priority attribute");
                        }
                    }
                }

                let pattern = match &variant.fields {
                    Fields::Unnamed(_) => quote! { #enum_name::#variant_name(..) },
                    Fields::Named(_) => quote! { #enum_name::#variant_name{..} },
                    Fields::Unit => quote! { #enum_name::#variant_name },
                };

                let pattern_expires = match &variant.fields {
                    Fields::Unnamed(_) => quote! { #enum_name::#variant_name(value, ..) },
                    Fields::Named(_) => quote! { #enum_name::#variant_name{value, ..} },
                    Fields::Unit => quote! { #enum_name::#variant_name },
                };

                delivery_policy_cases.push(quote! {
                    #pattern => #delivery_policy_value,
                });

                priority_cases.push(quote! {
                    #pattern => #priority_value,
                });

                expires_cases.push(quote! {
                    #pattern_expires => #expires_value,
                });
            }

            let fn_delivery_policy = if default_policy_impl {
                quote! {
                        fn delivery_policy(&self) -> ::roboplc::DeliveryPolicy {
                            ::roboplc::DeliveryPolicy::Always
                        }
                }
            } else {
                quote! {
                        fn delivery_policy(&self) -> ::roboplc::DeliveryPolicy {
                            match self {
                                #(#delivery_policy_cases)*
                            }
                        }
                }
            };
            let fn_priority = if default_priority_impl {
                quote! {
                        fn priority(&self) -> usize {
                            100
                        }
                }
            } else {
                quote! {
                        fn priority(&self) -> usize {
                            match self {
                                #(#priority_cases)*
                            }
                        }
                }
            };
            let fn_expires = if default_expires_impl {
                quote! {
                        fn is_expired(&self) -> bool {
                            false
                        }
                }
            } else {
                quote! {
                        fn is_expired(&self) -> bool {
                            match self {
                                #(#expires_cases)*
                            }
                        }
                }
            };

            let generated = quote! {
                    impl ::roboplc::DataDeliveryPolicy for #enum_name {
                        #fn_delivery_policy
                        #fn_priority
                        #fn_expires
                }
            };

            generated.into()
        }
        _ => panic!("DataPolicy can only be derived for enums"),
    }
}
