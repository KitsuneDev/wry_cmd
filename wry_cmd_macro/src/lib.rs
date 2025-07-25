//! Procedural macro crate for `#[command]`.
//!
//! Use through the `wry_cmd` crate unless you're building custom tools.

extern crate inflector;
extern crate proc_macro;
use inflector::Inflector;
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse_macro_input, parse_quote, AttributeArgs, FnArg, ImplItem, ItemFn, ItemImpl, Lit, LitStr,
    Meta, NestedMeta, PatType, ReturnType, Type,
};

/// Marks a function as a Wry IPC command.
/// The function must take a single argument implementing `Deserialize` and return a type implementing `Serialize`.
/// `#[command(name = "...")]` or just `#[command]`.
/// Registers a single fn under the given name (or fn name if omitted).
#[proc_macro_attribute]
pub fn command(attr: TokenStream, item: TokenStream) -> TokenStream {
    // 0. Parse optional `name = "..."` from attribute
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

    // 1. Parse the function itself
    let input_fn = parse_macro_input!(item as ItemFn);
    let fn_ident = &input_fn.sig.ident;
    let default_name = fn_ident.to_string().to_lowercase();
    let name_lit = override_name.unwrap_or_else(|| LitStr::new(&default_name, fn_ident.span()));

    // 2. Extract argument type or fall back to Value
    let arg_ty: Type = input_fn
        .sig
        .inputs
        .iter()
        .filter_map(|arg| {
            if let FnArg::Typed(PatType { ty, .. }) = arg {
                Some((**ty).clone())
            } else {
                None
            }
        })
        .next()
        .unwrap_or_else(|| syn::parse_quote!(serde_json::Value));

    // 3. Extract return type or default to ()
    let ret_ty: Type = match &input_fn.sig.output {
        ReturnType::Default => syn::parse_quote!(()),
        ReturnType::Type(_, ty) => (*ty.clone()),
    };

    // 4. Detect async vs sync
    let is_async = input_fn.sig.asyncness.is_some();

    // 5. Build the handler closure, importing FutureExt so .boxed() works
    let handler = if is_async {
        quote! {{
            use ::wry_cmd_core::futures::future::FutureExt;
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
        quote! {{
            use ::wry_cmd_core::futures::future::FutureExt;
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
    };

    // 6. Emit user fn + inventory registration
    let expanded = quote! {
        #input_fn

        ::wry_cmd_core::inventory::submit! {
            ::wry_cmd_core::Command {
                name: #name_lit,
                handler: #handler
            }
        }
    };
    expanded.into()
}

/// Attribute macro to auto-generate and register IPC commands from a trait impl.
///
/// Apply this to an `impl MyTrait for MyType { … }` block to turn each method into
/// a free function annotated with `#[command(name = "traitname/method")]` and register
/// it with the `wry_cmd` dispatcher.
///
/// # How it works
///
/// - **Trait name** is taken from the `impl MyTrait` header, lowercased.  
/// - **Method names** come from each `fn` or `async fn` in the impl block.  
/// - A wrapper function `__cmd_<trait>_<method>` is generated for each method:
///   ```ignore
///   #[command(name = "traitname/method")]
///   async fn __cmd_traitname_method(args: ArgType) -> RetType { … }
///   ```
///   or, for sync:
///   ```ignore
///   #[command(name = "traitname/method")]
///   fn __cmd_traitname_method(args: ArgType) -> RetType { … }
///   ```
/// - The generated wrapper delegates to a global `static INSTANCE: MyType`:
///   ```ignore
///   INSTANCE.method_name(args).await  // or without `.await` for sync
///   ```
///
/// # Naming convention
///
/// Commands are exposed under the URL-style name:
/// ```text
///    protocol://traitname/method
/// ```
/// e.g. given `trait MyCommands` with method `greet`, the command key is `"mycommands/greet"`.
///
/// # Requirements
///
/// 1. Must be used on a **trait impl** block, e.g. `impl MyCommands for MyApp { … }`.  
/// 2. A `static INSTANCE: MyApp = MyApp;` must be in scope for delegation.  
/// 3. Each method must have exactly one typed argument (or none), and its argument type
///    must implement `serde::Deserialize`.  
/// 4. Each method’s return type must implement `serde::Serialize` (or return `Result<…, String>`).
///
/// # Supported method signatures
///
/// ```rust
/// // sync
/// fn foo(&self, args: FooArgs) -> FooReply { … }
///
/// // async
/// async fn bar(&self, args: BarArgs) -> Result<BarReply, String> { … }
/// ```
///
/// # Example
///
/// ```rust
/// use serde::{Deserialize, Serialize};
/// use wry_cmd::{command, use_wry_cmd_protocol};
/// use wry_cmd_macro::commands;
///
/// #[derive(Deserialize)]
/// pub struct GreetArgs { pub name: String }
/// #[derive(Serialize)]
/// pub struct GreetReply { pub message: String }
///
/// pub trait MyCommands {
///     fn greet(&self, args: GreetArgs) -> GreetReply;
///     async fn fetch(&self, args: String) -> Result<String, String>;
/// }
///
/// struct MyApp;
/// static INSTANCE: MyApp = MyApp;
///
/// #[commands]
/// impl MyCommands for MyApp {
///     fn greet(&self, args: GreetArgs) -> GreetReply {
///         GreetReply { message: format!("Hello, {}!", args.name) }
///     }
///
///     async fn fetch(&self, args: String) -> Result<String, String> {
///         Ok(format!("Fetched: {}", args))
///     }
/// }
///
/// // In your wry setup:
/// // .with_asynchronous_custom_protocol("mado".into(), use_wry_cmd_protocol!("mado"))
/// ```
#[proc_macro_attribute]
pub fn commands(_attr: TokenStream, item: TokenStream) -> TokenStream {
    // 1. Parse the impl block
    let input_impl = parse_macro_input!(item as ItemImpl);
    let trait_path = input_impl
        .trait_
        .as_ref()
        .expect("`#[commands]` must be on a trait impl")
        .1
        .clone();
    let trait_name = trait_path
        .segments
        .last()
        .unwrap()
        .ident
        .to_string()
        .to_lowercase();

    // 2. Generate one wrapper per method
    let mut wrappers = Vec::new();
    for item in &input_impl.items {
        if let ImplItem::Method(m) = item {
            let method_ident = &m.sig.ident;
            // wrapper fn name: __cmd_<trait>_<method>
            let wrapper_ident = format_ident!("__cmd_{}_{}", trait_name, method_ident);

            // command name: "trait/method"
            let cmd_name = format!("{}/{}", trait_name, method_ident);

            // extract argument type or default to Value
            let arg_ty: Type = m
                .sig
                .inputs
                .iter()
                .filter_map(|arg| {
                    if let FnArg::Typed(PatType { ty, .. }) = arg {
                        Some((**ty).clone())
                    } else {
                        None
                    }
                })
                .next()
                .unwrap_or_else(|| parse_quote!(serde_json::Value));

            // return type or ()
            let ret_ty: Type = match &m.sig.output {
                ReturnType::Default => parse_quote!(()),
                ReturnType::Type(_, ty) => (*ty.clone()),
            };

            // async vs sync
            //println!("Generated Command {cmd_name}");
            let wrapper = if m.sig.asyncness.is_some() {
                quote! {
                    #[wry_cmd::command(name = #cmd_name)]
                    async fn #wrapper_ident(args: #arg_ty) -> #ret_ty {
                        INSTANCE.#method_ident(args).await
                    }
                }
            } else {
                quote! {
                    #[wry_cmd::command(name = #cmd_name)]
                    fn #wrapper_ident(args: #arg_ty) -> #ret_ty {
                        INSTANCE.#method_ident(args)
                    }
                }
            };

            wrappers.push(wrapper);
        }
    }

    // 3. Re-emit the impl plus all wrappers
    let expanded = quote! {
        #input_impl
        #(#wrappers)*
    };
    expanded.into()
}
