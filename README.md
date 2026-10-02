# webview-test

A minimal desktop host for testing an in-app order form: it opens the form in the
OS's built-in webview the way a host application would, and speaks the page's
`hp-app` bridge.

- `tao` — native windowing (the winit fork Tauri uses)
- `wry` — webview binding: WKWebView on macOS, WebView2 on Windows, WebKitGTK on Linux

No bundled browser engine, no Node toolchain.

## Run

```sh
cargo run                    # opens the setup screen
cargo run -- <order form url> # skips it and loads the URL
```

The window is centred on the primary monitor at 648x552.

## Setup screen

Fill in the application's details and choose **Open order form**. The app requests
a session from the session endpoint, as the host application's backend would, and
loads the URL it returns.

| Field | Sent as |
|---|---|
| Session endpoint | the URL `POST`ed to |
| Application ID | `app_id` |
| Secret | `Authorization: Bearer <secret>` |
| Flow ID | `flow_id` |
| Customer identity (optional) | `identity` — leave empty for a new shopper |
| Device | `device` |

Or paste an order form URL you already have under **Or open a URL**.

The values are remembered between runs in the OS config folder
(`~/Library/Application Support/webview-test/settings.json` on macOS), secret
included, readable by your user only.

## Keys

- **F2** — back to the setup screen
- **Escape** — sends `hp-app:back`, stepping back through the flow

## The bridge

The app is the host half of the page's `hp-app` protocol. Everything it hears is
printed to stdout:

| Page sends | The app |
|---|---|
| `hp-app:ready` | answers with `hp-app:init` |
| `hp-app:step`, `hp-app:rendered` | logs the step |
| `hp-app:external` | opens the URL in the system browser (http/https only) |
| `hp-app:complete` | logs the order payload and stays open so the confirmation can be checked |
| `hp-app:cancel` | closes the window |
| `hp-app:error` | logs the code |

`window.open` from the page is sent to the system browser too, rather than opening
a second window.

## Devtools

Built with the `devtools` feature — right-click → Inspect Element in the webview.

## Tests

```sh
cargo test
```

## Shipping it

`cargo build --release` produces a bare binary, not a `.app`. For a bundle with an
icon and code signing, either add [`cargo-bundle`](https://crates.io/crates/cargo-bundle)
or move to Tauri, which is this same tao+wry stack plus packaging and a JS bridge.
