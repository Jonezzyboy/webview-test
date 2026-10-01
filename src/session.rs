use serde::Deserialize;
use serde_json::{Map, Value};

use crate::settings::Settings;

#[derive(Deserialize)]
struct SessionResponse {
    url: String,
}

/// Requests an order form session from the session endpoint, as a host
/// application's backend would, and returns the URL to load.
pub fn request(settings: &Settings) -> Result<String, String> {
    if settings.endpoint.is_empty() || settings.app_id.is_empty() || settings.secret.is_empty() || settings.flow_id.is_empty() {
        return Err("Session endpoint, application ID, secret and flow ID are all required.".into());
    }

    let mut body = Map::new();
    body.insert("app_id".into(), Value::from(settings.app_id.as_str()));
    body.insert("flow_id".into(), Value::from(settings.flow_id.as_str()));
    if !settings.identity.is_empty() {
        body.insert("identity".into(), Value::from(settings.identity.as_str()));
    }
    if !settings.device.is_empty() {
        body.insert("device".into(), Value::from(settings.device.as_str()));
    }

    let mut resp = ureq::post(&settings.endpoint)
        .config()
        .http_status_as_error(false)
        .build()
        .header("Authorization", format!("Bearer {}", settings.secret))
        .send_json(Value::Object(body))
        .map_err(|err| format!("Request failed: {err}"))?;

    let status = resp.status();
    if !status.is_success() {
        let detail = resp.body_mut().read_to_string().unwrap_or_default();
        return Err(format!("Session endpoint answered {status}: {}", detail.trim()));
    }

    resp.body_mut()
        .read_json::<SessionResponse>()
        .map(|session| session.url)
        .map_err(|err| format!("Unexpected response: {err}"))
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    /// Serves one request with `status` and `body`, returning the raw request.
    fn serve_once(status: &str, body: &'static str) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/embed/session", listener.local_addr().unwrap());
        let status = status.to_string();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            stream.write_all(response.as_bytes()).unwrap();
            request
        });
        (endpoint, handle)
    }

    /// Reads the headers, then as much body as they say there is: the body can
    /// arrive in a later read than the headers.
    fn read_request(stream: &mut impl Read) -> String {
        let mut raw = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = stream.read(&mut buf).unwrap();
            raw.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&raw).to_string();
            if let Some(end) = text.find("\r\n\r\n") {
                let length = text[..end]
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                    .unwrap_or(0);
                if raw.len() >= end + 4 + length || n == 0 {
                    return text;
                }
            }
            if n == 0 {
                return text;
            }
        }
    }

    fn settings(endpoint: String) -> Settings {
        Settings { endpoint, app_id: "app_1".into(), secret: "s3cret".into(), flow_id: "flow_1".into(), device: "desktop".into(), ..Default::default() }
    }

    #[test]
    fn requests_a_session_and_returns_its_url() {
        let (endpoint, server) = serve_once("200 OK", r#"{"url":"https://pay.example.com/embed/order?t=abc"}"#);

        let url = request(&settings(endpoint)).unwrap();
        assert_eq!(url, "https://pay.example.com/embed/order?t=abc");

        let raw = server.join().unwrap();
        assert!(raw.contains("Bearer s3cret"), "no bearer secret in {raw}");
        let body: Value = serde_json::from_str(raw.split("\r\n\r\n").nth(1).unwrap_or("")).unwrap();
        assert_eq!(body["app_id"], "app_1");
        assert_eq!(body["flow_id"], "flow_1");
        assert!(body.get("identity").is_none(), "an empty identity was sent: {body}");
    }

    #[test]
    fn reports_a_rejected_request() {
        let (endpoint, server) = serve_once("401 Unauthorized", r#"{"error":"unauthorized"}"#);

        let err = request(&settings(endpoint)).unwrap_err();
        server.join().unwrap();
        assert!(err.contains("401") && err.contains("unauthorized"), "unhelpful error: {err}");
    }

    #[test]
    fn requires_the_credentials() {
        assert!(request(&Settings::default()).is_err());
    }
}
