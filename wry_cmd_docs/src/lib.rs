//! Auto-generate one Markdown file per service listing its commands (with links)
//! and referenced structs (with field docs).
//!
//! # Example (in build.rs)
//!
//! ```rust
//! use std::{env, path::PathBuf};
//! fn main() {
//!     let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
//!     let src_dir     = manifest_dir.join("src");
//!     let docs_dir    = manifest_dir.join("docs/commands");
//!
//!     wry_cmd_docs::generate_docs(&[src_dir], &docs_dir)
//!         .expect("failed to generate command docs");
//!
//!     println!("cargo:rerun-if-changed=src");
//! }
//! ```
use std::{collections::HashMap, fs, path::Path};

use quote::ToTokens;
use quote::quote;
use syn::{
    Attribute, Expr, ExprLit, Field, File, FnArg, ImplItem, ImplItemFn, Item, ItemFn, ItemImpl,
    ItemStruct, Lit, MetaNameValue, ReturnType, parse_file, punctuated::Punctuated, token::Comma,
};
use walkdir::WalkDir;

struct CommandDoc {
    service: String,
    name: String,
    args: Option<String>,
    ret: Option<String>,
    description: String,
}

struct StructDoc {
    name: String,
    description: String,
    fields: Vec<(String, String, String)>, // (field_name, field_type, field_doc)
}

/// For each service (and for free commands), generate `<service>.md` under `out_dir`.
pub fn generate_docs(
    src_dirs: &[impl AsRef<Path>],
    out_dir: impl AsRef<Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut cmds = Vec::new();
    let mut structs = HashMap::<String, StructDoc>::new();

    // 1) Parse all files, collect commands & structs
    for src in src_dirs {
        for entry in WalkDir::new(src.as_ref())
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "rs"))
        {
            let text = fs::read_to_string(entry.path())?;
            let ast: File = parse_file(&text)?;
            collect_commands(&ast.items, &mut cmds)?;
            collect_structs(&ast.items, &mut structs)?;
        }
    }

    // 2) Group commands by service
    let mut by_service: HashMap<String, Vec<CommandDoc>> = HashMap::new();
    for cmd in cmds {
        by_service.entry(cmd.service.clone()).or_default().push(cmd);
    }

    // 3) Ensure output directory
    let out = out_dir.as_ref();
    fs::create_dir_all(out)?;

    // 4) For each service, emit a file
    for (service, mut list) in by_service {
        // sort commands by name
        list.sort_by(|a, b| a.name.cmp(&b.name));

        // determine filename and title
        let (filename, title) = if service == "_free_" {
            ("free_commands.md".to_string(), "Free Commands".to_string())
        } else {
            (format!("{}.md", service.to_lowercase()), service.clone())
        };

        let mut md = String::new();
        md.push_str(&format!("# {} Commands\n\n", title));

        // index table
        md.push_str("| Command | Args | Return | Description |\n");
        md.push_str("|---------|------|--------|-------------|\n");
        for cmd in &list {
            let args = cmd.args.as_deref().unwrap_or("_none_");
            let ret = cmd.ret.as_deref().unwrap_or("_none_");
            let desc = if cmd.description.is_empty() {
                ""
            } else {
                &cmd.description
            };
            md.push_str(&format!(
                "| [{}](#{}) | `{}` | `{}` | {} |\n",
                cmd.name,
                cmd.name.to_lowercase(),
                if args == "_none_" {
                    "()".to_string()
                } else {
                    args.to_string()
                },
                if ret == "_none_" {
                    "()".to_string()
                } else {
                    ret.to_string()
                },
                desc,
            ));
        }

        // detail sections
        for cmd in &list {
            md.push_str(&format!("\n## {}\n\n", cmd.name));
            md.push_str(&format!(
                "**Signature:** `fn {}({}) -> {}`\n\n",
                cmd.name,
                cmd.args.as_deref().filter(|a| *a != "_none_").unwrap_or(""),
                cmd.ret.as_deref().unwrap_or("()"),
            ));
            if !cmd.description.is_empty() {
                md.push_str("**Description:**  \n");
                md.push_str(&cmd.description);
                md.push_str("\n\n");
            }
        }

        // struct reference
        let mut used = Vec::new();
        for cmd in &list {
            for ty in [&cmd.args, &cmd.ret] {
                if let Some(t) = ty {
                    let bare = t.split('<').next().unwrap().to_string();
                    if structs.contains_key(&bare) && !used.contains(&bare) {
                        used.push(bare);
                    }
                }
            }
        }
        if !used.is_empty() {
            md.push_str("\n# Struct Reference\n\n");
            for name in used {
                if let Some(sd) = structs.get(&name) {
                    md.push_str(&format!("## `{}`\n\n", sd.name));
                    if !sd.description.is_empty() {
                        md.push_str(&format!("{}\n\n", sd.description));
                    }
                    md.push_str("| Field | Type | Description |\n");
                    md.push_str("|-------|------|-------------|\n");
                    for (fname, ftype, fdoc) in &sd.fields {
                        md.push_str(&format!(
                            "| `{}` | `{}` | {} |\n",
                            fname,
                            ftype,
                            if fdoc.is_empty() { "" } else { fdoc }
                        ));
                    }
                    md.push_str("\n");
                }
            }
        }

        // write out
        fs::write(out.join(filename), md)?;
    }

    Ok(())
}

/// Walk items and collect all commands
fn collect_commands(
    items: &[Item],
    out: &mut Vec<CommandDoc>,
) -> Result<(), Box<dyn std::error::Error>> {
    for item in items {
        match item {
            // #[commands] impl ... { ... }
            Item::Impl(imp) if imp.attrs.iter().any(|a| a.path().is_ident("commands")) => {
                let service = if let Some((_, path, _)) = &imp.trait_ {
                    path.segments.last().unwrap().ident.to_string()
                } else if let syn::Type::Path(tp) = &*imp.self_ty {
                    tp.path.segments.last().unwrap().ident.to_string()
                } else {
                    "_".into()
                };
                for inner in &imp.items {
                    if let ImplItem::Fn(m) = inner {
                        let cmd = parse_method(m, &service)?.unwrap();
                        out.push(cmd);
                    }
                }
            }

            // free fn #[command]
            Item::Fn(f) if f.attrs.iter().any(|a| a.path().is_ident("command")) => {
                let cmd = parse_fn(f, "_free_")?.unwrap();
                out.push(cmd);
            }

            // fallback: individual methods #[command]
            Item::Impl(imp) if imp.trait_.is_none() || imp.trait_.is_some() => {
                let service = if let Some((_, path, _)) = &imp.trait_ {
                    path.segments.last().unwrap().ident.to_string()
                } else if let syn::Type::Path(tp) = &*imp.self_ty {
                    tp.path.segments.last().unwrap().ident.to_string()
                } else {
                    "_".into()
                };
                for inner in &imp.items {
                    if let ImplItem::Fn(m) = inner {
                        if m.attrs.iter().any(|a| a.path().is_ident("command")) {
                            let cmd = parse_method(m, &service)?.unwrap();
                            out.push(cmd);
                        }
                    }
                }
            }

            // commands! macro invocation
            Item::Macro(mac) if mac.mac.path.is_ident("commands") => {
                let nested: File = syn::parse2(mac.mac.tokens.clone())?;
                collect_commands(&nested.items, out)?;
            }

            _ => {}
        }
    }
    Ok(())
}

/// Walk items and collect all structs
fn collect_structs(
    items: &[Item],
    out: &mut HashMap<String, StructDoc>,
) -> Result<(), Box<dyn std::error::Error>> {
    for item in items {
        if let Item::Struct(ItemStruct {
            ident,
            attrs,
            fields,
            ..
        }) = item
        {
            let name = ident.to_string();
            let description = collect_doc_comments(attrs);
            let mut field_docs = Vec::new();

            // Iterate each field and only process those with an identifier
            for field in fields.iter() {
                if let Some(fident) = &field.ident {
                    let fname = fident.to_string();
                    let ftype = field.ty.to_token_stream().to_string();
                    let fdoc = collect_doc_comments(&field.attrs);
                    field_docs.push((fname, ftype, fdoc));
                }
            }

            out.insert(
                name.clone(),
                StructDoc {
                    name,
                    description,
                    fields: field_docs,
                },
            );
        }
    }
    Ok(())
}

/// Parse a free function into a CommandDoc
fn parse_fn(f: &ItemFn, service: &str) -> Result<Option<CommandDoc>, Box<dyn std::error::Error>> {
    let name = override_name(&f.attrs, f.sig.ident.to_string());
    let args = first_arg(&f.sig.inputs);
    let ret = first_return(&f.sig.output);
    let description = collect_doc_comments(&f.attrs);
    Ok(Some(CommandDoc {
        service: service.into(),
        name,
        args,
        ret,
        description,
    }))
}

/// Parse an impl method into a CommandDoc
fn parse_method(
    m: &ImplItemFn,
    service: &str,
) -> Result<Option<CommandDoc>, Box<dyn std::error::Error>> {
    let name = override_name(&m.attrs, m.sig.ident.to_string());
    let args = first_arg(&m.sig.inputs);
    let ret = first_return(&m.sig.output);
    let description = collect_doc_comments(&m.attrs);
    Ok(Some(CommandDoc {
        service: service.into(),
        name,
        args,
        ret,
        description,
    }))
}

/// Look for `name = "..."` in #[command(...)]
fn override_name(attrs: &[Attribute], default: String) -> String {
    let mut name = default;
    for a in attrs.iter().filter(|a| a.path().is_ident("command")) {
        let nvs: Punctuated<MetaNameValue, Comma> = a
            .parse_args_with(Punctuated::parse_terminated)
            .unwrap_or_default();
        for nv in nvs {
            if nv.path.is_ident("name") {
                if let Expr::Lit(ExprLit {
                    lit: Lit::Str(s), ..
                }) = nv.value
                {
                    name = s.value();
                }
            }
        }
    }
    name
}

/// Extract the first typed argument
fn first_arg(inputs: &Punctuated<FnArg, Comma>) -> Option<String> {
    for inp in inputs {
        if let FnArg::Typed(pt) = inp {
            return Some(pt.ty.to_token_stream().to_string());
        }
    }
    None
}

/// Extract the return type
fn first_return(output: &ReturnType) -> Option<String> {
    if let ReturnType::Type(_, ty) = output {
        Some(ty.to_token_stream().to_string())
    } else {
        None
    }
}

/// Gather `///` doc comments
fn collect_doc_comments(attrs: &[Attribute]) -> String {
    let mut lines = Vec::new();
    for attr in attrs.iter().filter(|a| a.path().is_ident("doc")) {
        let joined = quote! { #attr };
        if let Ok(MetaNameValue { value, .. }) = syn::parse2::<MetaNameValue>(joined) {
            if let Expr::Lit(ExprLit {
                lit: Lit::Str(s), ..
            }) = value
            {
                lines.push(s.value().trim().to_string());
            }
        }
    }
    lines.join(" ")
}
