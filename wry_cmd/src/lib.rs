//! # wry_cmd
//!
//! A lightweight, async-compatible command system for [`wry`](https://docs.rs/wry),
//! inspired by Tauri's `#[command]` and `invoke` architecture.
//!
//! ## Features
//! - `#[command]` proc-macro for registering sync or async Rust functions.
//! - Automatically exposes commands via a `with_asynchronous_custom_protocol` hook.
//! - Simple message format: `POST mado://commandName` with a JSON body.
//! Note: for **Windows**, you may need to use `http://{protocol}.{commandName}` instead, due to wry limitations.
//!
//! ## Example
//! ```rust,no_run
//! use serde::{Deserialize, Serialize};
//! use wry::{WebViewBuilder, application::event_loop::EventLoop, application::window::WindowBuilder};
//! use wry_cmd::{use_wry_cmd_protocol, command};
//!
//! #[derive(Deserialize)]
//! struct GreetArgs { name: String }
//!
//! #[derive(Serialize)]
//! struct GreetReply { message: String }
//!
//! #[command]
//! fn greet(args: GreetArgs) -> GreetReply {
//!     GreetReply { message: format!("Hello, {}!", args.name) }
//! }
//!
//! fn main() -> wry::Result<()> {
//!     let event_loop = EventLoop::new();
//!     let window = WindowBuilder::new().build(&event_loop)?;
//!     let webview = WebViewBuilder::new(window, event_loop)
//!         .with_asynchronous_custom_protocol("mado".into(), use_wry_cmd_protocol!("mado"))
//!         .with_url("data:text/html,...") // See README for working example
//!         .build()?;
//!     webview.run();
//!     Ok(())
//! }
//! ```
//!
//! ## Frontend JavaScript
//! ```js
//! const res = await fetch("mado://greet", {
//!   method: "POST",
//!   body: JSON.stringify({ name: "Alice" }),
//!   headers: { "Content-Type": "application/json" }
//! });
//! const reply = await res.json(); // { message: "Hello, Alice!" }
//! ```
pub use wry_cmd_core::*;
#[cfg(feature = "macros")]
pub use wry_cmd_macro::*;
