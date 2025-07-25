// wry_cmd_core/src/lib.rs

//! # wry_cmd_core
//!
//! Core runtime for the Wry IPC command system.
//! Provides the command registry and `handle_command` dispatcher,
//! plus the `use_wry_ipc!()` macro for integrating with Wry.

// Re-export inventory so macros in consumer crates can refer to it
pub extern crate inventory;

pub use futures; // re-export futures for macro‐expansions
use futures::{future::BoxFuture, FutureExt};
use once_cell::sync::Lazy;
use percent_encoding::percent_decode_str;
use serde_json::Value;
use std::collections::HashMap;
/// Type alias for command handler functions.
pub type CommandHandler = fn(Value) -> BoxFuture<'static, Result<Value, String>>;

/// A single registered command.
pub struct Command {
    pub name: &'static str,
    pub handler: CommandHandler,
}

// Collect command registrations via `inventory`
inventory::collect!(Command);

/// Dispatch an IPC command by name with JSON arguments.
/// Supports names like `"mycommands/greet"` or even `"/mycommands/greet"`
/// and percent-encoded paths (e.g. `%2Fmycommands%2Fgreet`).
pub fn handle_command(raw_cmd: &str, args: Value) -> BoxFuture<'static, Result<Value, String>> {
    // 1) Normalize: strip leading/trailing slashes
    let cmd = raw_cmd.trim_matches('/');

    // 2) Percent-decode, falling back to the original if decoding fails
    let cmd = percent_decode_str(cmd)
        .decode_utf8()
        .map(|cow| cow.to_string())
        .unwrap_or_else(|_| cmd.to_string());

    // 3) Lookup in the registry
    for cmd_def in inventory::iter::<Command> {
        if cmd_def.name == cmd {
            return (cmd_def.handler)(args);
        }
    }

    // 4) Unknown command
    println!("Unknown command: {}", cmd);
    println!(
        "Available commands: {:?}",
        inventory::iter::<Command>
            .into_iter()
            .map(|c| c.name)
            .collect::<Vec<_>>()
    );
    futures::future::ready(Err(format!("Unknown command: {}", cmd))).boxed()
}

#[macro_export]
macro_rules! use_wry_cmd_protocol {
    ($scheme:expr) => {{
        // Capture scheme name as String
        let scheme = $scheme.to_string();

        move |_webview_id: wry::WebViewId<'_>,
              request: wry::http::Request<Vec<u8>>,
              responder: wry::RequestAsyncResponder| {
            use wry::http::{Method, Response, StatusCode};
            use ::std::borrow::Cow;
            use ::serde_json::Value;

            // Handle CORS preflight
            if request.method() == &Method::OPTIONS {
                let resp = Response::builder()
                    .status(StatusCode::NO_CONTENT)
                    .header("Access-Control-Allow-Origin", "*")
                    .header("Access-Control-Allow-Methods", "POST, OPTIONS")
                    .header("Access-Control-Allow-Headers", "Content-Type")
                    .body(Cow::Borrowed(&[][..]))
                    .unwrap();
                responder.respond(resp);
                return;
            }

            // Only POST is allowed
            if request.method() != &Method::POST {
                let resp = Response::builder()
                    .status(StatusCode::METHOD_NOT_ALLOWED)
                    .header("Allow", "POST, OPTIONS")
                    .header("Access-Control-Allow-Origin", "*")
                    .body(Cow::Borrowed(b"Method Not Allowed".as_ref()))
                    .unwrap();
                responder.respond(resp);
                return;
            }

            // Extract command name from URI: "mado://greet" → "greet"
            let uri = request.uri();
            // 1. Extract host (authority) and path separately
            let host = uri.authority().map(|a| a.as_str()).unwrap_or("");
            let path = uri.path_and_query()
                .map(|pq| pq.path())
                .unwrap_or("");

            // 2. Trim any leading slash on the path
            let path = path.trim_start_matches('/');

            // 3. Build the command name
            let cmd = if host.is_empty() {
                // no host, just path
                path.to_string()
            } else if path.is_empty() {
                // host only
                host.to_string()
            } else {
                // both host and path
                format!("{}/{}", host, path)
            };
            // Parse JSON args from body
            let args: Value = serde_json::from_slice(request.body()).unwrap_or_default();

            // Spawn a background thread to handle both sync & async commands
            std::thread::spawn(move || {
                // `handle_command` is your registry entrypoint, now returning a Future<Value>
                let fut = $crate::handle_command(&cmd, args);

                // Wait for the command (sync commands should return an immediately-ready future)
                let result_json = $crate::futures::executor::block_on(fut);

                // Wrap any error into {"error": "..."}
                let response_value = match result_json {
                    Ok(v) => v,
                    Err(e) => serde_json::json!({ "error": e }),
                };

                // Serialize response
                let body = serde_json::to_vec(&response_value).unwrap_or_default();
                let resp = Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "application/json")
                    .header("Access-Control-Allow-Origin", "*")
                    .body(Cow::Owned(body))
                    .unwrap();

                // Send it back
                responder.respond(resp);
            });
        }
    }};
}
