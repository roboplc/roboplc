extern crate proc_macro;
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, Lit, Meta, MetaNameValue, NestedMeta};

fn lowercase_first_letter(s: &str) -> String {
    s.chars()
        .enumerate()
        .map(|(i, c)| {
            if i == 0 {
                c.to_lowercase().to_string()
            } else {
                c.to_string()
            }
        })
        .collect()
}

/// Automatically implements the `WorkerOptions` trait for a worker struct
///
/// Provides an attribute `worker_opts` to specify the worker options. The attribute can be
/// specifieid multiple times.
///
/// Atrribute arguments:
///
/// * `name` - Specifies the name of the worker. The value must be a quoted string. The name must
/// be unique and must be 15 characters or less. If not specified, the default is the the structure
/// name with the first letter in lowercase
///
/// * `stack_size` - Specifies the stack size for the worker
///
/// * `blocking` - Specifies if the worker is blocking. The value can be `true` or `false`. A hint
/// for task supervisors that the worker blocks the thread (e.g. listens to a socket or has got a
/// big interval in the main loop, does not return any useful result and should not be joined)
///
/// * `scheduling` - Specifies the scheduling policy for the worker. The value can be one of the:
/// `roundrobin`, `fifo`, `idle`, `batch`, `deadline`, `other`. If not specified, the default is
/// `other`
///
/// * `priority` - Specifies the real-time priority for the worker, higher is better. If specified,
/// the scheduling policy must be `fifo`, `roundrobin` or `deadline`
///
/// * `cpu` - Specifies the CPU affinity for the worker. The value can be a single CPU number or a
/// range of CPUs separated by a dash. The value can be a quoted string or an integer
///
/// Example:
///
/// ```rust
/// use roboplc::controller::prelude::*;
///
/// #[derive(WorkerOpts)]
/// #[worker_opts(name = "my_worker", stack_size = 8192, scheduling = "fifo", priority = 80, cpu = "0-3")]
/// struct MyWorker {
///  // some fields
/// }
///
/// #[derive(WorkerOpts)]
/// #[worker_opts(name = "my_worker2", scheduling = "fifo", priority = 80, cpu = 1)]
/// struct MyWorker2 {
///  // some fields
/// }
/// ```
///
///
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
    let mut blocking = false;

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
                            } else {
                                panic!("worker stack size must be usize");
                            }
                        } else if path.is_ident("blocking") {
                            if let Lit::Bool(lit_bool) = lit {
                                blocking = lit_bool.value;
                            } else {
                                panic!("worker blocking must be bool");
                            }
                        } else if path.is_ident("scheduling") {
                            scheduling = Some(parse_scheduling(lit));
                        } else if path.is_ident("priority") {
                            if let Lit::Int(lit_int) = lit {
                                priority = Some(lit_int.base10_parse::<i32>().unwrap());
                            } else {
                                panic!("worker priority must be i32");
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
                        } else {
                            panic!("Unknown attribute: {:?}", path);
                        }
                    }
                }
            } else {
                panic!("unable to parse worker_opts attribute");
            }
        }
    }

    let worker_name = worker_name.unwrap_or_else(|| lowercase_first_letter(&name.to_string()));

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
            "other" => Some(quote! { ::roboplc::thread_rt::Scheduling::Other }),
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
    let blocking_impl = if blocking {
        quote! {
            fn worker_is_blocking(&self) -> bool {
                true
            }
        }
    } else {
        quote! {}
    };
    let expanded = quote! {
        impl ::roboplc::controller::WorkerOptions for #name {
            fn worker_name(&self) -> &str {
                #worker_name
            }

            #stack_size_impl
            #scheduling_impl
            #priority_impl
            #cpus_impl
            #blocking_impl

        }
    };

    expanded.into()
}

fn parse_scheduling(lit: &Lit) -> String {
    match lit {
        Lit::Str(lit_str) => lit_str.value(),
        Lit::Int(lit_int) => lit_int.to_string(),
        _ => "other".to_string(),
    }
}
