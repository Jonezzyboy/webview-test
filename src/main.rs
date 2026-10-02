mod session;
mod settings;

use std::env;
use std::process::Command;
use std::thread;

use serde_json::Value;
use tao::{
    dpi::LogicalSize,
    event::{ElementState, Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    keyboard::KeyCode,
    window::WindowBuilder,
};
use wry::{NewWindowResponse, WebViewBuilder};

use settings::Settings;

const SETUP_HTML: &str = include_str!("setup.html");

// Reports the loaded document back to stdout so the app is verifiable headlessly.
const REPORT_LOAD: &str = r#"
window.addEventListener('DOMContentLoaded', () => {
    window.ipc.postMessage(JSON.stringify({
        url: location.href,
        title: document.title,
        chars: document.documentElement.innerHTML.length,
    }));
});
"#;

// The host half of the hp-app handshake. wry has no inbound channel of its own, so
// messages to the page are delivered by evaluating this.
const SEND_INIT: &str = r#"window.hpApp && window.hpApp.receive({type: 'hp-app:init'})"#;
const SEND_BACK: &str = r#"window.hpApp && window.hpApp.receive({type: 'hp-app:back'})"#;

/// What the page, or a background task, asked the host to do.
#[derive(Debug)]
enum HostEvent {
    /// The page is listening; answer with init.
    Ready(String),
    /// The flow moved on. Carries the page type.
    Step(String),
    /// The page believes the purchase finished. Carries the raw order payload —
    /// order and product references, which a real host verifies against its own
    /// backend before unlocking anything.
    Complete(String),
    /// The customer backed out.
    Cancel,
    /// A URL that must not open in this window: legal documents, PayPal, any 3DS
    /// that breaks out. In a top-level webview it would replace the order form
    /// mid-purchase with no way back.
    External(String),
    /// The page reported a problem.
    Error(String),
    /// The setup screen asked for a session from these settings.
    SetupSession(Settings),
    /// The setup screen asked to open the URL in these settings.
    SetupUrl(Settings),
    /// A session request finished: the URL to load, or why it failed.
    SessionResult(Result<String, String>),
    /// Anything else worth seeing, including the load report.
    Other(String),
}

fn main() -> wry::Result<()> {
    let start_url = env::args().nth(1);

    let event_loop = EventLoopBuilder::<HostEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let ipc_proxy = proxy.clone();

    let size = LogicalSize::new(648.0, 552.0);
    let window = WindowBuilder::new()
        .with_title("webview-test")
        .with_inner_size(size)
        .build(&event_loop)
        .unwrap();

    if let Some(monitor) = event_loop.primary_monitor() {
        let screen = monitor.size().to_logical::<f64>(monitor.scale_factor());
        window.set_outer_position(tao::dpi::LogicalPosition::new(
            monitor.position().x as f64 + (screen.width - size.width) / 2.0,
            monitor.position().y as f64 + (screen.height - size.height) / 2.0,
        ));
    }

    let builder = WebViewBuilder::new()
        .with_devtools(true)
        .with_initialization_script(REPORT_LOAD)
        .with_ipc_handler(move |req| {
            let _ = ipc_proxy.send_event(classify(req.body()));
        })
        // window.open in the page: hand it to the browser rather than opening a
        // second webview the customer cannot get back from. The page also asks via
        // hp-app:external, so either route works.
        .with_new_window_req_handler(|url, _features| {
            open_externally(&url);
            NewWindowResponse::Deny
        });

    // Whether the setup screen is showing. Every page shares the IPC channel, so
    // setup messages are only acted on while it is: a loaded order form must not
    // be able to request sessions or rewrite the saved settings.
    let mut on_setup = start_url.is_none();
    let webview = match &start_url {
        Some(url) => {
            println!("loading {url}");
            builder.with_url(url).build(&window)?
        }
        None => builder.with_html(setup_page(&settings::load())).build(&window)?,
    };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::UserEvent(HostEvent::Ready(channels)) => {
                println!("hp-app ready (channels {channels}) — sending init");
                if let Err(err) = webview.evaluate_script(SEND_INIT) {
                    eprintln!("failed to send init: {err}");
                }
            }

            Event::UserEvent(HostEvent::Step(step)) => println!("hp-app step {step}"),

            // Stays open so the confirmation page can be checked; a real host
            // would close here once its backend confirms the order.
            Event::UserEvent(HostEvent::Complete(order)) => {
                println!("hp-app complete {order}");
                println!("verify these against your own backend before unlocking anything");
            }

            Event::UserEvent(HostEvent::Cancel) => {
                println!("hp-app cancel");
                *control_flow = ControlFlow::Exit;
            }

            Event::UserEvent(HostEvent::External(url)) => {
                println!("hp-app external {url}");
                open_externally(&url);
            }

            Event::UserEvent(HostEvent::Error(code)) => eprintln!("hp-app error {code}"),

            Event::UserEvent(HostEvent::SetupSession(settings)) if on_setup => {
                settings::save(&settings);
                println!("requesting session from {}", settings.endpoint);
                let proxy = proxy.clone();
                // Off the event loop: the request blocks, and the window must keep
                // painting while it does.
                thread::spawn(move || {
                    let _ = proxy.send_event(HostEvent::SessionResult(session::request(&settings)));
                });
            }

            Event::UserEvent(HostEvent::SetupUrl(settings)) if on_setup => {
                settings::save(&settings);
                on_setup = false;
                load(&webview, &settings.url);
            }

            Event::UserEvent(HostEvent::SessionResult(result)) if on_setup => match result {
                Ok(url) => {
                    on_setup = false;
                    load(&webview, &url);
                }
                Err(message) => {
                    eprintln!("session request failed: {message}");
                    let arg = serde_json::to_string(&message).unwrap_or_default();
                    let _ = webview.evaluate_script(&format!("window.setupError({arg})"));
                }
            },

            Event::UserEvent(HostEvent::Other(body)) => println!("loaded {body}"),

            Event::WindowEvent {
                event: WindowEvent::KeyboardInput { event, .. },
                ..
            } if event.state == ElementState::Pressed => match event.physical_key {
                // Escape steps back through the flow rather than closing the
                // window, standing in for a native back control.
                KeyCode::Escape => {
                    let _ = webview.evaluate_script(SEND_BACK);
                }
                KeyCode::F2 => {
                    on_setup = true;
                    let _ = webview.load_html(&setup_page(&settings::load()));
                }
                _ => {}
            },

            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *control_flow = ControlFlow::Exit,

            _ => {}
        }
    });
}

/// The setup screen, prefilled with the saved settings.
fn setup_page(settings: &Settings) -> String {
    // "</" would end the script block the settings are injected into.
    let json = serde_json::to_string(settings).unwrap_or_else(|_| "null".into()).replace("</", "<\\/");
    SETUP_HTML.replace("/*SETTINGS*/null", &json)
}

fn load(webview: &wry::WebView, url: &str) {
    println!("loading {url}");
    if let Err(err) = webview.load_url(url) {
        eprintln!("failed to load {url}: {err}");
    }
}

/// Turns an IPC body into a host event.
fn classify(body: &str) -> HostEvent {
    let Ok(msg) = serde_json::from_str::<Value>(body) else {
        return HostEvent::Other(body.to_string());
    };
    let field = |key: &str| msg.get(key).and_then(Value::as_str).map(str::to_string);
    let settings = || serde_json::from_value::<Settings>(msg.get("settings").cloned().unwrap_or_default()).unwrap_or_default();

    match field("type").as_deref() {
        Some("hp-app:ready") => HostEvent::Ready(msg.get("channels").map(Value::to_string).unwrap_or_default()),
        Some("hp-app:step" | "hp-app:rendered") => HostEvent::Step(field("step").unwrap_or_else(|| "-".into())),
        Some("hp-app:complete") => HostEvent::Complete(body.to_string()),
        Some("hp-app:cancel") => HostEvent::Cancel,
        Some("hp-app:external") => match field("url") {
            Some(url) => HostEvent::External(url),
            None => HostEvent::Other(body.to_string()),
        },
        Some("hp-app:error") => HostEvent::Error(field("code").unwrap_or_else(|| "unknown".into())),
        Some("setup:session") => HostEvent::SetupSession(settings()),
        Some("setup:url") => HostEvent::SetupUrl(settings()),
        _ => HostEvent::Other(body.to_string()),
    }
}

/// Opens a URL in the OS browser. Only ever called with URLs the page hands us,
/// and only http(s) — a host that shells out to anything a page names is a hole.
fn open_externally(url: &str) {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        eprintln!("refusing to open non-http url: {url}");
        return;
    }

    let opener = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(target_os = "windows") {
        ("cmd", vec!["/C", "start", "", url])
    } else {
        ("xdg-open", vec![url])
    };

    if let Err(err) = Command::new(opener.0).args(opener.1).spawn() {
        eprintln!("failed to open {url}: {err}");
    }
}
