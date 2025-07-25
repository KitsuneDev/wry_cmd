# wry_cmd

Tauri-style `#[command]` macro and command system for [`wry`](https://docs.rs/wry), supporting async Rust functions and front-end integration via a custom protocol.

## 🚀 Features

- `#[command]` macro for both `async fn` and `fn`
- Auto-registers via inventory
- Uses Wry’s `with_asynchronous_custom_protocol`
- JSON-over-POST interface
- CORS preflight support

## 🔧 Usage

```rust

use wry_cmd::{command, use_wry_cmd_protocol};

#[derive(serde::Deserialize, Default)]
struct GreetArgs {
    name: String,
}

#[derive(serde::Serialize, Default)]
struct GreetReply {
    message: String,
}

#[command]
fn greet(args: GreetArgs) -> GreetReply {
    println!("Greet command called with: {:?}", args.name);
    GreetReply {
        message: format!("Hello, {}!", args.name),
    }
}

let wv = WebViewBuilder::new()
        .with_asynchronous_custom_protocol("proto".to_string(), use_wry_cmd_protocol!("proto"))
        .build(&window)
        .expect("Failed to build WebView");
```

Then in JS:

```js
async function sendGreet() {
  const name = document.getElementById("name").value;
  // You can use `http://proto.greet` for Windows compatibility
  // or `proto://greet` for other platforms.
  const res = await fetch(`http://proto.greet`, {
    method: "POST",
    body: JSON.stringify({ name }),
    headers: { "Content-Type": "application/json" },
  });
  const data = await res.json();
  document.getElementById("response").textContent = data.error
    ? "Error: " + data.error
    : data.message;
}
```

# AI Usage Disclaimer

Please note that AI has been used in order to properly document this crate.
