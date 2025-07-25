//! Procedural macro crate for `#[command]`.
//!
//! Use through the `wry_cmd` crate unless you're building custom tools.

extern crate proc_macro;
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, FnArg, ItemFn, PatType, Type};

/// Marks a function as a Wry IPC command.
/// The function must take a single argument implementing `Deserialize` and return a type implementing `Serialize`.
#[proc_macro_attribute]
pub fn command(_attr: TokenStream, item: TokenStream) -> TokenStream {
    // 1. Parse the user fn
    let input = parse_macro_input!(item as ItemFn);
    let vis = &input.vis;
    let sig = &input.sig;
    let attrs = &input.attrs;
    let block = &input.block;
    let fn_ident = &sig.ident;
    let name_str = fn_ident.to_string();

    // 2. Extract the argument type or default to Value
    let arg_type: Type = if let Some(FnArg::Typed(PatType { ty, .. })) = sig.inputs.first() {
        // ty is Box<Type>, clone inner Type
        (**ty).clone()
    } else {
        syn::parse_str("serde_json::Value").unwrap()
    };

    // 3. Build the handler closure, wrapping in a block that imports FutureExt
    let handler = if sig.asyncness.is_some() {
        // async fn foo(args: T) -> R
        quote! {
            {
                // bring `.boxed()` into scope
                use ::wry_cmd_core::futures::future::FutureExt;
                |args: ::serde_json::Value| {
                    // deserialize
                    let args: #arg_type = ::serde_json::from_value(args)
                        .unwrap_or_default();
                    // call async fn and box the future
                    async move {
                        let ret = #fn_ident(args).await;
                        ::serde_json::to_value(&ret).map_err(|e| e.to_string())
                    }
                    .boxed()
                }
            }
        }
    } else {
        // sync fn foo(args: T) -> R
        quote! {
            {
                use ::wry_cmd_core::futures::future::FutureExt;
                |args: ::serde_json::Value| {
                    let args: #arg_type = ::serde_json::from_value(args)
                        .unwrap_or_default();
                    async move {
                        let ret = #fn_ident(args);
                        ::serde_json::to_value(&ret).map_err(|e| e.to_string())
                    }
                    .boxed()
                }
            }
        }
    };

    // 4. Emit the original fn + inventory registration
    let expanded = quote! {
        #(#attrs)*
        #vis #sig #block

        ::wry_cmd_core::inventory::submit! {
            ::wry_cmd_core::Command {
                name: #name_str,
                handler: #handler
            }
        }
    };

    TokenStream::from(expanded)
}
