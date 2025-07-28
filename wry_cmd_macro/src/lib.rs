//! Procedural macro crate for `#[command]`.
//! Use through the `wry_cmd` crate unless you're building custom tools.

extern crate inflector;
extern crate proc_macro;
use inflector::Inflector;
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse_macro_input, parse_quote, spanned::Spanned, AttributeArgs, FnArg, ImplItem, ItemFn,
    ItemImpl, Lit, LitStr, Meta, NestedMeta, PatType, ReturnType, Type,
};

/// Marks a function as a Wry IPC command.
/// The function can take zero or one argument implementing `Deserialize`
/// and return a type implementing `Serialize`. If omitted, no args or no return are supported.
/// Use `#[command(name = "...")]` or just `#[command]`.
#[proc_macro_attribute]
pub fn command(attr: TokenStream, item: TokenStream) -> TokenStream {
    // Parse optional `name = "..."` from attribute
    let args = parse_macro_input!(attr as AttributeArgs);
    let mut override_name: Option<LitStr> = None;
    for nested in args {
        if let NestedMeta::Meta(Meta::NameValue(nv)) = nested {
            if nv.path.is_ident("name") {
                if let Lit::Str(ls) = nv.lit {
                    override_name = Some(ls);
                }
            }
        }
    }

    // Parse the function
    let input_fn = parse_macro_input!(item as ItemFn);
    let fn_ident = &input_fn.sig.ident;

    // Determine command name literal
    let default_name = fn_ident.to_string().to_lowercase();
    let name_lit = override_name.unwrap_or_else(|| LitStr::new(&default_name, fn_ident.span()));

    // Determine if function has a typed argument (excluding receiver)
    let mut has_arg = false;
    let mut arg_ty: Type = syn::parse_quote!(serde_json::Value);
    for input in &input_fn.sig.inputs {
        if let FnArg::Typed(PatType { ty, .. }) = input {
            has_arg = true;
            arg_ty = (*ty.clone());
            break;
        }
    }

    // Determine return type or default to `()`
    let mut has_return = true;
    let ret_ty: Type = match &input_fn.sig.output {
        ReturnType::Default => {
            has_return = false;
            syn::parse_quote!(())
        }
        ReturnType::Type(_, ty) => (*ty.clone()),
    };

    // Detect async vs sync
    let is_async = input_fn.sig.asyncness.is_some();

    // Build the handler closure
    let handler = if is_async {
        if has_arg {
            quote! {{
                use ::wry_cmd::futures::future::FutureExt;
                |args: ::serde_json::Value| {
                    async move {
                        let args: #arg_ty = match ::serde_json::from_value(args) {
                            Ok(v) => v,
                            Err(e) => return Err(e.to_string()),
                        };
                        let ret = #fn_ident(args).await;
                        ::serde_json::to_value(&ret).map_err(|e| e.to_string())
                    }
                    .boxed()
                }
            }}
        } else {
            // no arguments
            quote! {{
                use ::wry_cmd::futures::future::FutureExt;
                |_: ::serde_json::Value| {
                    async move {
                        let ret = #fn_ident().await;
                        ::serde_json::to_value(&ret).map_err(|e| e.to_string())
                    }
                    .boxed()
                }
            }}
        }
    } else {
        if has_arg {
            quote! {{
                use ::wry_cmd::futures::future::FutureExt;
                |args: ::serde_json::Value| {
                    async move {
                        let args: #arg_ty = match ::serde_json::from_value(args) {
                            Ok(v) => v,
                            Err(e) => return Err(e.to_string()),
                        };
                        let ret = #fn_ident(args);
                        ::serde_json::to_value(&ret).map_err(|e| e.to_string())
                    }
                    .boxed()
                }
            }}
        } else {
            // no arguments
            quote! {{
                use ::wry_cmd::futures::future::FutureExt;
                |_: ::serde_json::Value| {
                    async move {
                        let ret = #fn_ident();
                        ::serde_json::to_value(&ret).map_err(|e| e.to_string())
                    }
                    .boxed()
                }
            }}
        }
    };

    // Emit the original function and inventory registration
    let expanded = quote! {
        #input_fn

        ::wry_cmd::inventory::submit! {
            ::wry_cmd::Command {
                name: #name_lit,
                handler: #handler
            }
        }
    };
    expanded.into()
}

/// Attribute macro to auto-generate and register IPC commands from an impl block.
///
/// Usage:
/// ```rust
/// // Trait impl – defaults to the trait name:
/// #[commands]
/// impl MyTrait for MyStruct { … }
///
/// // Inherent impl – defaults to the type name:
/// #[commands]
/// impl MyService { … }
///
/// // Override the service name:
/// #[commands(service = "foo")]
/// impl MyTrait for MyStruct { … }
/// ```
#[proc_macro_attribute]
pub fn commands(attr: TokenStream, item: TokenStream) -> TokenStream {
    // 1. Parse optional `service = "..."` from attribute
    let args = parse_macro_input!(attr as AttributeArgs);
    let mut override_service: Option<LitStr> = None;
    for nested in args {
        if let NestedMeta::Meta(Meta::NameValue(nv)) = nested {
            if nv.path.is_ident("service") {
                match nv.lit {
                    Lit::Str(ls) => override_service = Some(ls),
                    _ => panic!("`service` attribute must be a string, e.g. service = \"foo\""),
                }
            }
        }
    }

    // 2. Parse the impl block
    let input_impl = parse_macro_input!(item as ItemImpl);

    // 3. Determine the service name literal
    let service_lit = if let Some(s) = override_service {
        s
    } else if let Some((_, ref trait_path, _)) = input_impl.trait_ {
        // Trait impl: use the trait’s last segment
        let trait_ident = &trait_path.segments.last().unwrap().ident;
        LitStr::new(&trait_ident.to_string().to_lowercase(), trait_ident.span())
    } else {
        // Inherent impl: use the type’s last segment
        let ty_name = if let Type::Path(type_path) = &*input_impl.self_ty {
            type_path.path.segments.last().unwrap().ident.to_string()
        } else {
            panic!("`#[commands]` only supports impls on simple path types");
        };
        LitStr::new(&ty_name.to_lowercase(), input_impl.self_ty.span())
    };

    // 4. Build one wrapper per method
    let mut wrappers = Vec::new();
    for item in &input_impl.items {
        if let ImplItem::Method(m) = item {
            let method_ident = &m.sig.ident;
            let wrapper_ident = format_ident!("__cmd_{}_{}", service_lit.value(), method_ident);
            // final command name: "<service>/<method>"
            let cmd_name = LitStr::new(
                &format!("{}/{}", service_lit.value(), method_ident),
                method_ident.span(),
            );

            // detect if there’s a single typed argument
            let mut has_arg = false;
            let mut arg_ty: Type = parse_quote!(serde_json::Value);
            for input in &m.sig.inputs {
                if let FnArg::Typed(PatType { ty, .. }) = input {
                    has_arg = true;
                    arg_ty = (*ty.clone());
                    break;
                }
            }

            // detect return type
            let ret_ty: Type = match &m.sig.output {
                ReturnType::Default => parse_quote!(()),
                ReturnType::Type(_, ty) => (*ty.clone()),
            };

            // generate wrapper
            let wrapper = if m.sig.asyncness.is_some() {
                if has_arg {
                    quote! {
                        #[wry_cmd::command(name = #cmd_name)]
                        async fn #wrapper_ident(args: #arg_ty) -> #ret_ty {
                            INSTANCE.#method_ident(args).await
                        }
                    }
                } else {
                    quote! {
                        #[wry_cmd::command(name = #cmd_name)]
                        async fn #wrapper_ident() -> #ret_ty {
                            INSTANCE.#method_ident().await
                        }
                    }
                }
            } else {
                if has_arg {
                    quote! {
                        #[wry_cmd::command(name = #cmd_name)]
                        fn #wrapper_ident(args: #arg_ty) -> #ret_ty {
                            INSTANCE.#method_ident(args)
                        }
                    }
                } else {
                    quote! {
                        #[wry_cmd::command(name = #cmd_name)]
                        fn #wrapper_ident() -> #ret_ty {
                            INSTANCE.#method_ident()
                        }
                    }
                }
            };

            wrappers.push(wrapper);
        }
    }

    // 5. Re-emit the original impl plus all wrappers
    let expanded = quote! {
        #input_impl
        #(#wrappers)*
    };
    expanded.into()
}
