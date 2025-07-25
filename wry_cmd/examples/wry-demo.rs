use serde::Deserialize;
use tao::{
    event::{Event, WindowEvent},
    event_loop::{self, ControlFlow, EventLoop},
    window::WindowBuilder,
};
use wry::WebViewBuilder;
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

fn main() -> wry::Result<()> {
    let html = r#"
    <!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <title>Wry Custom Protocol Example</title>
</head>
<body>
  <input id="name" placeholder="Enter your name" />
  <button onclick="sendGreet()">Greet</button>
  <p id="response"></p>

  <script>
async function sendGreet() {
  const name = document.getElementById('name').value;
  const res = await fetch(`http://mado.greet`, {
    method: 'POST',
    body: JSON.stringify({ name }),
    headers: { 'Content-Type': 'application/json' }
  });
  const data = await res.json();
  document.getElementById('response').textContent =
    data.error ? 'Error: ' + data.error : data.message;
}
</script>
</body>
</html>

    "#;

    // Build the Wry window + webview
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .build(&event_loop)
        .expect("Failed to create window");
    let wv = WebViewBuilder::new()
        .with_transparent(true)
        .with_background_color((0, 0, 0, 0)) // transparent background
        .with_html(html)
        .with_asynchronous_custom_protocol("mado".to_string(), use_wry_cmd_protocol!("mado"))
        .build(&window)
        .expect("Failed to build WebView");
    let event_loop = EventLoop::new();
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            *control_flow = ControlFlow::Exit;
        }
    });
}
