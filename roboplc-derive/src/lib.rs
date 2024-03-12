extern crate proc_macro;
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Fields, Lit, Meta, MetaNameValue, NestedMeta};

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
                                delivery_policy_value = match nested_meta {
                                    NestedMeta::Meta(meta) => parse_delivery_policy(
                                        meta.path()
                                            .get_ident()
                                            .map(|v| v.to_string().to_lowercase())
                                            .as_deref(),
                                    ),
                                    NestedMeta::Lit(lit) => match lit {
                                        Lit::Str(lit_str) => parse_delivery_policy(Some(
                                            &lit_str.value().to_lowercase(),
                                        )),
                                        _ => panic!("data_delivery value must be a string"),
                                    },
                                };
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

fn parse_delivery_policy(s: Option<&str>) -> proc_macro2::TokenStream {
    match s {
        Some("single") => quote! { ::roboplc::DeliveryPolicy::Single },
        Some("single_optional") => quote! { ::roboplc::DeliveryPolicy::SingleOptional },
        Some("optional") => quote! { ::roboplc::DeliveryPolicy::Optional },
        Some("always") => quote! { ::roboplc::DeliveryPolicy::Always },
        Some(v) => panic!("Unknown policy variant: {}", v),
        None => panic!("Policy variant not specified"),
    }
}

/// # Panics
///
/// Will panic if the worker name is not specified or is invalid
#[allow(clippy::too_many_lines)]
#[proc_macro_derive(WorkerOpts, attributes(worker_opts))]
pub fn worker_opts_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;

    let mut worker_name = None;
    let mut stack_size = None;
    let mut scheduling = None;
    let mut priority = None;
    let mut cpus = Vec::new();

    for attr in input.attrs {
        if attr.path.is_ident("worker_opts") {
            if let Ok(Meta::List(meta_list)) = attr.parse_meta() {
                for meta in &meta_list.nested {
                    if let NestedMeta::Meta(Meta::NameValue(MetaNameValue { path, lit, .. })) = meta
                    {
                        if path.is_ident("name") {
                            if let Lit::Str(lit_str) = lit {
                                worker_name = Some(lit_str.value());
                            } else {
                                panic!("worker name must be a quoted string");
                            }
                        } else if path.is_ident("stack_size") {
                            if let Lit::Int(lit_int) = lit {
                                stack_size = Some(lit_int.base10_parse::<usize>().unwrap());
                            }
                        } else if path.is_ident("scheduling") {
                            scheduling = Some(parse_scheduling(lit));
                        } else if path.is_ident("priority") {
                            if let Lit::Int(lit_int) = lit {
                                priority = Some(lit_int.base10_parse::<i32>().unwrap());
                            }
                        } else if path.is_ident("cpu") {
                            if let Lit::Int(lit_int) = lit {
                                cpus.push(lit_int.base10_parse::<usize>().unwrap());
                            } else if let Lit::Str(lit_str) = lit {
                                let value = lit_str.value();
                                if value.contains('-') {
                                    let bounds: Vec<&str> = value.split('-').collect();
                                    if bounds.len() == 2 {
                                        if let (Ok(start), Ok(end)) =
                                            (bounds[0].parse::<usize>(), bounds[1].parse::<usize>())
                                        {
                                            for cpu in start..=end {
                                                cpus.push(cpu);
                                            }
                                        }
                                    }
                                } else if let Ok(cpu) = value.parse::<usize>() {
                                    cpus.push(cpu);
                                } else {
                                    panic!("Invalid cpu value: {}", value);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let worker_name = worker_name.expect("worker name is not specified");

    assert!(
        worker_name.len() <= 15,
        "Worker name must be 15 characters or less"
    );

    let stack_size_impl = if let Some(s) = stack_size {
        quote! {
            fn worker_stack_size(&self) -> Option<usize> {
                Some(#s)
            }
        }
    } else {
        quote! {}
    };
    let priority_impl = if let Some(p) = priority {
        quote! {
            fn worker_priority(&self) -> Option<i32> {
                Some(#p)
            }
        }
    } else {
        quote! {}
    };
    let cpus_impl = if cpus.is_empty() {
        quote! {}
    } else {
        quote! {
            fn worker_cpu_ids(&self) -> Option<&[usize]> {
                Some(&[#(#cpus),*])
            }
        }
    };
    let sched = if let Some(sched) = scheduling {
        match sched.to_lowercase().as_str() {
            "roundrobin" => Some(quote! { ::roboplc::thread_rt::Scheduling::RoundRobin }),
            "fifo" => Some(quote! { ::roboplc::thread_rt::Scheduling::FIFO }),
            "idle" => Some(quote! { ::roboplc::thread_rt::Scheduling::Idle }),
            "batch" => Some(quote! { ::roboplc::thread_rt::Scheduling::Batch }),
            "deadline" => Some(quote! { ::roboplc::thread_rt::Scheduling::DeadLine }),
            "normal" => Some(quote! { ::roboplc::thread_rt::Scheduling::Normal }),
            v => panic!("Unknown scheduling policy: {}", v),
        }
    } else {
        None
    };
    let scheduling_impl = if let Some(s) = sched {
        quote! {
            fn worker_scheduling(&self) -> ::roboplc::thread_rt::Scheduling {
                #s
            }
        }
    } else {
        quote! {}
    };
    let expanded = quote! {
        impl WorkerOptions for #name {
            fn worker_name(&self) -> &str {
                #worker_name
            }

            #stack_size_impl
            #scheduling_impl
            #priority_impl
            #cpus_impl

        }
    };

    expanded.into()
}

fn parse_scheduling(lit: &Lit) -> String {
    match lit {
        Lit::Str(lit_str) => lit_str.value(),
        Lit::Int(lit_int) => lit_int.to_string(),
        _ => "normal".to_string(),
    }
}
