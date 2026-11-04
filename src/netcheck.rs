//! Key validation, model discovery and reachability. All network calls are
//! blocking and are expected to run on a worker thread in the TUI.

use std::time::Duration;

use serde_json::Value;

const TIMEOUT: Duration = Duration::from_secs(15);
const REACH_TIMEOUT: Duration = Duration::from_secs(6);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AuthStyle {
    Bearer,
    XApiKey,
}

impl AuthStyle {
    pub fn header(self, key: &str) -> (&'static str, String) {
        match self {
            AuthStyle::Bearer => ("Authorization", format!("Bearer {key}")),
            AuthStyle::XApiKey => ("x-api-key", key.to_string()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Validation {
    pub ok: bool,
    pub status: Option<u16>,
    pub auth: Option<AuthStyle>,
    pub models: Vec<String>,
    pub message: String,
}

impl Validation {
    fn fail(status: Option<u16>, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            status,
            auth: None,
            models: Vec::new(),
            message: message.into(),
        }
    }

    /// A 5xx means the service failed, not that the key was rejected.
    pub fn server_error(&self) -> bool {
        self.status.is_some_and(|status| status >= 500)
    }
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .build()
        .into()
}

fn request_models(url: &str, key: &str, style: AuthStyle) -> Result<(u16, String), String> {
    let (header_name, header_value) = style.header(key);
    let mut request = agent(TIMEOUT)
        .get(url)
        .header("Accept", "application/json")
        .header("User-Agent", "redhat-ai-configurator")
        .header(header_name, header_value);
    if style == AuthStyle::XApiKey {
        request = request.header("anthropic-version", "2023-06-01");
    }

    match request.call() {
        Ok(mut response) => {
            let status = response.status().as_u16();
            let body = response
                .body_mut()
                .read_to_string()
                .unwrap_or_default();
            Ok((status, body))
        }
        Err(error) => Err(error.to_string()),
    }
}

/// Validate a key against a full models URL (for example
/// `https://host/v1/models`). A rejected auth style is retried with the other
/// header, and so is a 5xx (some gateways answer 500 for the wrong auth style).
/// The response body of a 5xx is included so the user can see what the service
/// actually said.
pub fn validate(url: &str, raw_key: &str) -> Validation {
    let key: String = raw_key.chars().filter(|c| !c.is_whitespace()).collect();
    if key.is_empty() {
        return Validation::fail(None, "Paste your API key first.");
    }

    let mut attempts: Vec<(u16, String)> = Vec::new();
    for style in [AuthStyle::Bearer, AuthStyle::XApiKey] {
        match request_models(url, &key, style) {
            Ok((status, body)) if (200..300).contains(&status) => {
                let models = parse_models(&body);
                let count = models.len();
                return Validation {
                    ok: true,
                    status: Some(status),
                    auth: Some(style),
                    message: if count == 0 {
                        "Key accepted.".to_string()
                    } else {
                        format!(
                            "Key accepted. {count} model{} available.",
                            if count == 1 { "" } else { "s" }
                        )
                    },
                    models,
                };
            }
            Ok((status, body)) => {
                let retry = matches!(status, 401 | 403) || status >= 500;
                attempts.push((status, body));
                if !retry {
                    break;
                }
            }
            Err(error) => {
                if attempts.is_empty() {
                    return Validation::fail(
                        None,
                        format!(
                            "Could not reach the service ({error}). Check your network and the VPN requirement, then try again."
                        ),
                    );
                }
                break;
            }
        }
    }

    // A real auth rejection is more informative than a 5xx; prefer it.
    let chosen = attempts
        .iter()
        .find(|(status, _)| matches!(*status, 401 | 403))
        .or_else(|| attempts.iter().find(|(status, _)| *status >= 500))
        .or_else(|| attempts.first());

    let Some((status, body)) = chosen else {
        return Validation::fail(None, "The key could not be validated.");
    };
    let status = *status;

    let message = match status {
        401 => "That key was rejected. Check that you copied the whole key, or create a new one.".to_string(),
        403 => "This key has no access to this service. Check your entitlement or create a new key.".to_string(),
        404 => "The service did not recognise this endpoint. Check the VPN requirement for this step.".to_string(),
        status if status >= 500 => {
            let detail = body_snippet(body, &key);
            match detail {
                Some(detail) => format!(
                    "The service failed with HTTP {status} (a service error, not a key rejection). Response: {detail}"
                ),
                None => format!(
                    "The service failed with HTTP {status} (a service error, not a key rejection). Try again later."
                ),
            }
        }
        status => format!("The service answered with HTTP {status}. Try again, or create a new key."),
    };
    Validation::fail(Some(status), message)
}

/// Short, single-line, key-redacted view of an error body.
fn body_snippet(body: &str, key: &str) -> Option<String> {
    let redacted = body.replace(key, "***");
    let collapsed = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    let limit = 300;
    if collapsed.chars().count() > limit {
        Some(format!("{}…", collapsed.chars().take(limit).collect::<String>()))
    } else {
        Some(collapsed)
    }
}

fn parse_models(body: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for key in ["data", "models"] {
        if let Some(Value::Array(items)) = value.get(key) {
            for item in items {
                let name = item
                    .get("id")
                    .and_then(Value::as_str)
                    .or_else(|| item.get("name").and_then(Value::as_str));
                if let Some(name) = name {
                    out.push(name.to_string());
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Reachability probe: any non-5xx answer counts as reachable.
pub fn reachable(url: &str) -> bool {
    match agent(REACH_TIMEOUT)
        .get(url)
        .header("Accept", "application/json,text/html")
        .header("User-Agent", "redhat-ai-configurator")
        .call()
    {
        Ok(response) => response.status().as_u16() < 500,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_openai_and_anthropic_lists() {
        assert_eq!(
            parse_models(r#"{"data":[{"id":"b"},{"id":"a"}]}"#),
            vec!["a", "b"]
        );
        assert_eq!(
            parse_models(r#"{"models":[{"name":"m2"},{"name":"m1"}]}"#),
            vec!["m1", "m2"]
        );
        assert!(parse_models("not json").is_empty());
    }

    #[test]
    fn empty_key_fails_without_network() {
        let result = validate("http://127.0.0.1:1/v1/models", "   ");
        assert!(!result.ok);
        assert!(result.status.is_none());
    }

    type Paths = std::sync::Arc<std::sync::Mutex<Vec<String>>>;

    fn serve(
        responses: Vec<(u16, &'static str)>,
    ) -> (String, Paths, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let paths: Paths = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = paths.clone();
        let handle = std::thread::spawn(move || {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0u8; 4096];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("")
                    .to_string();
                recorded.lock().unwrap().push(path);
                let reason = match status {
                    200 => "OK",
                    401 => "Unauthorized",
                    500 => "Internal Server Error",
                    _ => "Status",
                };
                let payload = if body.is_empty() { "{}" } else { body };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                    payload.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (format!("http://{address}/v1/models"), paths, handle)
    }

    #[test]
    fn requests_the_models_url_exactly_once() {
        let (base, paths, server) = serve(vec![(200, r#"{"data":[]}"#)]);
        let result = validate(&base, "secret");
        server.join().unwrap();
        assert!(result.ok);
        let paths = paths.lock().unwrap();
        assert_eq!(paths.as_slice(), ["/v1/models"]);
    }

    #[test]
    fn accepts_a_bearer_key() {
        let (base, _paths, server) = serve(vec![(200, r#"{"data":[{"id":"m1"},{"id":"m2"}]}"#)]);
        let result = validate(&base, "secret");
        server.join().unwrap();
        assert!(result.ok);
        assert_eq!(result.auth, Some(AuthStyle::Bearer));
        assert_eq!(result.models, vec!["m1", "m2"]);
    }

    #[test]
    fn retries_with_x_api_key_after_401() {
        let (base, _paths, server) = serve(vec![
            (401, ""),
            (200, r#"{"models":[{"name":"claude"}]}"#),
        ]);
        let result = validate(&base, "secret");
        server.join().unwrap();
        assert!(result.ok);
        assert_eq!(result.auth, Some(AuthStyle::XApiKey));
        assert_eq!(result.models, vec!["claude"]);
    }

    #[test]
    fn retries_with_x_api_key_after_a_server_error() {
        let (base, _paths, server) = serve(vec![
            (500, r#"{"error":"unknown auth style"}"#),
            (200, r#"{"data":[{"id":"m1"}]}"#),
        ]);
        let result = validate(&base, "secret");
        server.join().unwrap();
        assert!(result.ok);
        assert_eq!(result.auth, Some(AuthStyle::XApiKey));
    }

    #[test]
    fn surfaces_a_rejected_key() {
        let (base, _paths, server) = serve(vec![(401, ""), (401, "")]);
        let result = validate(&base, "secret");
        server.join().unwrap();
        assert!(!result.ok);
        assert_eq!(result.status, Some(401));
        assert!(result.message.contains("rejected"));
    }

    #[test]
    fn includes_the_body_of_a_server_error_and_redacts_the_key() {
        let (base, _paths, server) = serve(vec![
            (500, r#"{"error":"upstream router failed","key":"secret"}"#),
            (500, r#"{"error":"upstream router failed","key":"secret"}"#),
        ]);
        let result = validate(&base, "secret");
        server.join().unwrap();
        assert!(!result.ok);
        assert!(result.server_error());
        assert!(result.message.contains("not a key rejection"));
        assert!(result.message.contains("upstream router failed"));
        assert!(!result.message.contains("secret"));
    }
}
