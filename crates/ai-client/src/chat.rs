//! One chat request to an AI provider.
//!
//! Two wire formats are supported: Anthropic's Messages API and the OpenAI chat-completions
//! format (also spoken by many local and third-party servers). Request building and response
//! parsing are plain functions so they can be tested without a network; [`chat`] adds the HTTP
//! call.
//!
//! Safety properties kept on purpose:
//! * the API key goes only into a request header, never into a URL, log line or error text;
//! * the official OpenAI and Anthropic endpoints must be `https`; plain `http` is accepted only
//!   for the *custom* provider (typically a server on this machine or the local network);
//! * nothing is sent unless the caller calls [`chat`]; there is no background traffic.

use serde_json::{Value, json};
use std::time::Duration;

/// Wire format / service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    /// OpenAI chat completions.
    OpenAi,
    /// Anthropic Messages API.
    Anthropic,
    /// A server that speaks the OpenAI chat format (may be plain http).
    Custom,
}

/// Connection settings. `Debug` hides the key.
#[derive(Clone)]
pub struct Config {
    /// Service.
    pub provider: Provider,
    /// Endpoint root without trailing slash, e.g. `https://api.openai.com/v1`.
    pub base_url: String,
    /// API key (may be empty for custom servers that need none).
    pub api_key: String,
    /// Model name.
    pub model: String,
    /// Whole-request timeout.
    pub timeout: Duration,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("provider", &self.provider)
            .field("base_url", &self.base_url)
            .field(
                "api_key",
                &if self.api_key.is_empty() {
                    "(none)"
                } else {
                    "(hidden)"
                },
            )
            .field("model", &self.model)
            .finish()
    }
}

/// Who wrote a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The person.
    User,
    /// The model.
    Assistant,
}

/// One conversation turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// Author.
    pub role: Role,
    /// Text.
    pub content: String,
}

impl Message {
    /// A user message.
    pub fn user(s: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: s.into(),
        }
    }
    /// An assistant message.
    pub fn assistant(s: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: s.into(),
        }
    }
}

/// A request.
#[derive(Clone, Debug)]
pub struct Request<'a> {
    /// System prompt (instructions plus the document text).
    pub system: &'a str,
    /// Conversation so far, ending with the user's new message.
    pub messages: &'a [Message],
    /// Upper bound on the reply length in tokens.
    pub max_tokens: u32,
}

/// A successful reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    /// Reply text.
    pub text: String,
    /// The provider stopped because of the token limit, so the text may be cut off.
    pub truncated: bool,
    /// Input tokens billed, when reported.
    pub input_tokens: Option<u64>,
    /// Output tokens billed, when reported.
    pub output_tokens: Option<u64>,
}

/// What can go wrong.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AiError {
    /// No key has been saved.
    #[error("No API key is set. Add one in Preferences ▸ PDF Copilot.")]
    NoKey,
    /// The configuration cannot be used.
    #[error("{0}")]
    Config(String),
    /// The server could not be reached or the connection broke.
    #[error("Could not reach the AI service: {0}")]
    Network(String),
    /// The server answered with an error status.
    #[error("{message}")]
    Status {
        /// HTTP status.
        code: u16,
        /// Message for the person (already tailored for common codes).
        message: String,
    },
    /// The reply was not understood.
    #[error("The AI service sent a reply BergPDF could not read: {0}")]
    Parse(String),
    /// The provider declined to answer.
    #[error("The AI service declined to answer this request{0}")]
    Refused(String),
    /// The reply was empty.
    #[error("The AI service returned an empty answer.")]
    Empty,
    /// The user cancelled.
    #[error("Cancelled.")]
    Cancelled,
}

/// A request ready to send: URL, headers and JSON body.
#[derive(Debug)]
pub struct Prepared {
    /// Full URL.
    pub url: String,
    /// Header name/value pairs (contains the key).
    pub headers: Vec<(String, String)>,
    /// JSON body.
    pub body: Value,
}

fn is_loopback_host(url: &str) -> bool {
    let rest = url.split("://").nth(1).unwrap_or("");
    let host = rest.split(['/', ':']).next().unwrap_or("");
    matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "::1")
}

/// Check the endpoint is acceptable for the provider.
pub fn validate_endpoint(cfg: &Config) -> Result<(), AiError> {
    let u = cfg.base_url.trim();
    if !(u.starts_with("https://") || u.starts_with("http://")) {
        return Err(AiError::Config(
            "The server address must start with https:// (or http:// for a custom server).".into(),
        ));
    }
    if u.starts_with("http://") && cfg.provider != Provider::Custom && !is_loopback_host(u) {
        return Err(AiError::Config(
            "OpenAI and Anthropic must be reached over https so the key is not sent in clear text."
                .into(),
        ));
    }
    if cfg.model.trim().is_empty() {
        return Err(AiError::Config("No model name is set.".into()));
    }
    if cfg.provider != Provider::Custom && cfg.api_key.trim().is_empty() {
        return Err(AiError::NoKey);
    }
    Ok(())
}

/// Build the HTTP request for `req`.
pub fn prepare(cfg: &Config, req: &Request<'_>) -> Result<Prepared, AiError> {
    validate_endpoint(cfg)?;
    let base = cfg.base_url.trim().trim_end_matches('/');
    let role = |r: Role| match r {
        Role::User => "user",
        Role::Assistant => "assistant",
    };
    match cfg.provider {
        Provider::Anthropic => {
            let msgs: Vec<Value> = req
                .messages
                .iter()
                .map(|m| json!({"role": role(m.role), "content": m.content}))
                .collect();
            // The system block carries the (large) document text; marking it cacheable makes
            // follow-up questions on the same document much cheaper.
            let body = json!({
                "model": cfg.model.trim(),
                "max_tokens": req.max_tokens,
                "system": [{
                    "type": "text",
                    "text": req.system,
                    "cache_control": {"type": "ephemeral"}
                }],
                "messages": msgs,
            });
            Ok(Prepared {
                url: format!("{base}/messages"),
                headers: vec![
                    ("x-api-key".into(), cfg.api_key.trim().into()),
                    ("anthropic-version".into(), "2023-06-01".into()),
                    ("content-type".into(), "application/json".into()),
                ],
                body,
            })
        }
        Provider::OpenAi | Provider::Custom => {
            let mut msgs = vec![json!({"role": "system", "content": req.system})];
            msgs.extend(
                req.messages
                    .iter()
                    .map(|m| json!({"role": role(m.role), "content": m.content})),
            );
            let mut body = json!({"model": cfg.model.trim(), "messages": msgs});
            // OpenAI's newer models only accept `max_completion_tokens`; other servers
            // understand the classic `max_tokens`.
            let key = if cfg.provider == Provider::OpenAi {
                "max_completion_tokens"
            } else {
                "max_tokens"
            };
            body[key] = json!(req.max_tokens);
            let mut headers = vec![("content-type".into(), "application/json".into())];
            if !cfg.api_key.trim().is_empty() {
                headers.push((
                    "authorization".into(),
                    format!("Bearer {}", cfg.api_key.trim()),
                ));
            }
            Ok(Prepared {
                url: format!("{base}/chat/completions"),
                headers,
                body,
            })
        }
    }
}

fn error_message(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    let e = v.get("error")?;
    e.get("message")
        .and_then(Value::as_str)
        .or_else(|| e.as_str())
        .map(|s| s.chars().take(400).collect())
}

/// Turn an HTTP error status into a message a person can act on.
pub fn status_error(code: u16, body: &str) -> AiError {
    let detail = error_message(body);
    let tail = detail
        .as_deref()
        .map(|d| format!(" ({d})"))
        .unwrap_or_default();
    let message = match code {
        401 | 403 => format!(
            "The AI service rejected the API key (HTTP {code}). Check the key in Preferences.{tail}"
        ),
        404 => format!(
            "The AI service does not know this address or model (HTTP 404). Check the model name and server address.{tail}"
        ),
        408 | 504 => {
            format!("The AI service took too long to answer (HTTP {code}). Try again.{tail}")
        }
        413 => format!(
            "The document text is too large for this model (HTTP 413). Lower the amount of text sent in Preferences.{tail}"
        ),
        429 => format!(
            "The AI service says you are over a rate or usage limit (HTTP 429). Wait a moment or check your plan.{tail}"
        ),
        400 => format!("The AI service refused the request (HTTP 400){tail}"),
        500..=599 => format!("The AI service had a problem (HTTP {code}). Try again later.{tail}"),
        _ => format!("The AI service answered with HTTP {code}.{tail}"),
    };
    AiError::Status { code, message }
}

fn usage(v: &Value, input: &str, output: &str) -> (Option<u64>, Option<u64>) {
    let u = v.get("usage");
    (
        u.and_then(|u| u.get(input)).and_then(Value::as_u64),
        u.and_then(|u| u.get(output)).and_then(Value::as_u64),
    )
}

/// Parse a successful response body.
pub fn parse_reply(provider: Provider, body: &str) -> Result<Reply, AiError> {
    let v: Value = serde_json::from_str(body).map_err(|e| AiError::Parse(e.to_string()))?;
    match provider {
        Provider::Anthropic => {
            let stop = v.get("stop_reason").and_then(Value::as_str).unwrap_or("");
            if stop == "refusal" {
                let why = v
                    .pointer("/stop_details/explanation")
                    .and_then(Value::as_str)
                    .map(|s| format!(": {s}"))
                    .unwrap_or_default();
                return Err(AiError::Refused(why));
            }
            let text: String = v
                .get("content")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::Parse("no content".into()))?
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("");
            if text.trim().is_empty() {
                return Err(AiError::Empty);
            }
            let (i, o) = usage(&v, "input_tokens", "output_tokens");
            Ok(Reply {
                text,
                truncated: stop == "max_tokens",
                input_tokens: i,
                output_tokens: o,
            })
        }
        Provider::OpenAi | Provider::Custom => {
            let choice = v
                .pointer("/choices/0")
                .ok_or_else(|| AiError::Parse("no choices".into()))?;
            if let Some(r) = choice.pointer("/message/refusal").and_then(Value::as_str)
                && !r.is_empty()
            {
                return Err(AiError::Refused(format!(": {r}")));
            }
            let finish = choice
                .get("finish_reason")
                .and_then(Value::as_str)
                .unwrap_or("");
            if finish == "content_filter" {
                return Err(AiError::Refused(String::new()));
            }
            let text = choice
                .pointer("/message/content")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if text.trim().is_empty() {
                return Err(AiError::Empty);
            }
            let (i, o) = usage(&v, "prompt_tokens", "completion_tokens");
            Ok(Reply {
                text,
                truncated: finish == "length",
                input_tokens: i,
                output_tokens: o,
            })
        }
    }
}

/// Send `req` and return the reply. Blocks; call it from a worker thread.
pub fn chat(cfg: &Config, req: &Request<'_>) -> Result<Reply, AiError> {
    let p = prepare(cfg, req)?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(cfg.timeout))
        .http_status_as_error(false)
        .https_only(cfg.provider != Provider::Custom && !is_loopback_host(&cfg.base_url))
        // Trust what the operating system trusts (Windows/macOS certificate stores, the system
        // bundle on Linux), so company proxies that re-sign traffic with an installed CA work.
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .user_agent("BergPDF")
        .build()
        .into();
    let mut r = agent.post(&p.url);
    for (k, v) in &p.headers {
        r = r.header(k, v);
    }
    let body = serde_json::to_string(&p.body).map_err(|e| AiError::Parse(e.to_string()))?;
    let mut resp = r
        .send(body)
        .map_err(|e| AiError::Network(scrub(&e.to_string(), &cfg.api_key)))?;
    let code = resp.status().as_u16();
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| AiError::Network(scrub(&e.to_string(), &cfg.api_key)))?;
    if !(200..300).contains(&code) {
        return Err(status_error(code, &text));
    }
    parse_reply(cfg.provider, &text)
}

/// Remove the key from any text before it can reach a log or the screen.
fn scrub(s: &str, key: &str) -> String {
    let key = key.trim();
    if key.len() >= 6 {
        s.replace(key, "(key)")
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn cfg(p: Provider) -> Config {
        Config {
            provider: p,
            base_url: match p {
                Provider::OpenAi => "https://api.openai.com/v1".into(),
                Provider::Anthropic => "https://api.anthropic.com/v1/".into(),
                Provider::Custom => "http://localhost:11434/v1".into(),
            },
            api_key: "sk-secret-key-123".into(),
            model: "m".into(),
            timeout: Duration::from_secs(5),
        }
    }

    #[test]
    fn anthropic_request_shape() {
        let msgs = [
            Message::user("hi"),
            Message::assistant("yo"),
            Message::user("q"),
        ];
        let p = prepare(
            &cfg(Provider::Anthropic),
            &Request {
                system: "SYS",
                messages: &msgs,
                max_tokens: 1000,
            },
        )
        .unwrap();
        assert_eq!(p.url, "https://api.anthropic.com/v1/messages");
        let h = |n: &str| {
            p.headers
                .iter()
                .find(|(k, _)| k == n)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(h("x-api-key"), Some("sk-secret-key-123"));
        assert_eq!(h("anthropic-version"), Some("2023-06-01"));
        assert_eq!(p.body["model"], "m");
        assert_eq!(p.body["max_tokens"], 1000);
        assert_eq!(p.body["system"][0]["text"], "SYS");
        assert_eq!(p.body["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(p.body["messages"].as_array().unwrap().len(), 3);
        assert_eq!(p.body["messages"][1]["role"], "assistant");
        // No sampling parameters and no prefill: both are rejected by current models.
        assert!(p.body.get("temperature").is_none());
        assert!(!p.url.contains("sk-secret"));
    }

    #[test]
    fn openai_and_custom_request_shapes() {
        let msgs = [Message::user("q")];
        let r = Request {
            system: "SYS",
            messages: &msgs,
            max_tokens: 500,
        };
        let o = prepare(&cfg(Provider::OpenAi), &r).unwrap();
        assert_eq!(o.url, "https://api.openai.com/v1/chat/completions");
        assert_eq!(o.body["messages"][0]["role"], "system");
        assert_eq!(o.body["max_completion_tokens"], 500);
        assert!(o.body.get("max_tokens").is_none());
        assert!(
            o.headers
                .iter()
                .any(|(k, v)| k == "authorization" && v == "Bearer sk-secret-key-123")
        );
        let c = prepare(&cfg(Provider::Custom), &r).unwrap();
        assert_eq!(c.body["max_tokens"], 500);
        let mut nokey = cfg(Provider::Custom);
        nokey.api_key.clear();
        let c = prepare(&nokey, &r).unwrap();
        assert!(!c.headers.iter().any(|(k, _)| k == "authorization"));
    }

    #[test]
    fn endpoint_rules() {
        let mut c = cfg(Provider::OpenAi);
        c.base_url = "http://api.openai.com/v1".into();
        assert!(matches!(validate_endpoint(&c), Err(AiError::Config(_))));
        c.base_url = "ftp://x".into();
        assert!(matches!(validate_endpoint(&c), Err(AiError::Config(_))));
        let mut a = cfg(Provider::Anthropic);
        a.api_key.clear();
        assert_eq!(validate_endpoint(&a), Err(AiError::NoKey));
        let mut m = cfg(Provider::Anthropic);
        m.model = " ".into();
        assert!(matches!(validate_endpoint(&m), Err(AiError::Config(_))));
        // Plain http is fine for a custom server, and for loopback.
        assert!(validate_endpoint(&cfg(Provider::Custom)).is_ok());
        let mut l = cfg(Provider::OpenAi);
        l.base_url = "http://localhost:8080/v1".into();
        assert!(validate_endpoint(&l).is_ok());
    }

    #[test]
    fn anthropic_replies() {
        let ok = r#"{"content":[{"type":"thinking","thinking":""},{"type":"text","text":"Hello "},{"type":"text","text":"world"}],"stop_reason":"end_turn","usage":{"input_tokens":12,"output_tokens":3}}"#;
        let r = parse_reply(Provider::Anthropic, ok).unwrap();
        assert_eq!(r.text, "Hello world");
        assert!(!r.truncated);
        assert_eq!((r.input_tokens, r.output_tokens), (Some(12), Some(3)));
        let cut = r#"{"content":[{"type":"text","text":"abc"}],"stop_reason":"max_tokens"}"#;
        assert!(parse_reply(Provider::Anthropic, cut).unwrap().truncated);
        let refusal = r#"{"content":[],"stop_reason":"refusal","stop_details":{"type":"refusal","category":"cyber","explanation":"nope"}}"#;
        assert_eq!(
            parse_reply(Provider::Anthropic, refusal),
            Err(AiError::Refused(": nope".into()))
        );
        assert_eq!(
            parse_reply(
                Provider::Anthropic,
                r#"{"content":[],"stop_reason":"end_turn"}"#
            ),
            Err(AiError::Empty)
        );
        assert!(matches!(
            parse_reply(Provider::Anthropic, "<html>"),
            Err(AiError::Parse(_))
        ));
    }

    #[test]
    fn openai_replies() {
        let ok = r#"{"choices":[{"message":{"content":"Hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":1}}"#;
        let r = parse_reply(Provider::OpenAi, ok).unwrap();
        assert_eq!(r.text, "Hi");
        assert_eq!((r.input_tokens, r.output_tokens), (Some(5), Some(1)));
        let len = r#"{"choices":[{"message":{"content":"Hi"},"finish_reason":"length"}]}"#;
        assert!(parse_reply(Provider::Custom, len).unwrap().truncated);
        let refusal = r#"{"choices":[{"message":{"content":null,"refusal":"cannot"},"finish_reason":"stop"}]}"#;
        assert_eq!(
            parse_reply(Provider::OpenAi, refusal),
            Err(AiError::Refused(": cannot".into()))
        );
        let filtered =
            r#"{"choices":[{"message":{"content":""},"finish_reason":"content_filter"}]}"#;
        assert_eq!(
            parse_reply(Provider::OpenAi, filtered),
            Err(AiError::Refused(String::new()))
        );
        assert!(matches!(
            parse_reply(Provider::OpenAi, "{}"),
            Err(AiError::Parse(_))
        ));
    }

    #[test]
    fn status_messages_are_actionable_and_never_echo_much() {
        let body = r#"{"error":{"message":"Incorrect API key provided: sk-abc","type":"invalid_request_error"}}"#;
        let AiError::Status { code, message } = status_error(401, body) else {
            panic!()
        };
        assert_eq!(code, 401);
        assert!(message.contains("rejected the API key"));
        assert!(message.contains("Incorrect API key"));
        for c in [404u16, 413, 429, 500, 503, 418] {
            let AiError::Status { message, .. } = status_error(c, "not json") else {
                panic!()
            };
            assert!(message.contains(&c.to_string()), "{message}");
        }
        let long = format!(r#"{{"error":{{"message":"{}"}}}}"#, "x".repeat(5000));
        let AiError::Status { message, .. } = status_error(400, &long) else {
            panic!()
        };
        assert!(message.len() < 700);
    }

    #[test]
    fn debug_never_prints_the_key() {
        let s = format!("{:?}", cfg(Provider::OpenAi));
        assert!(!s.contains("sk-secret"));
        assert!(s.contains("hidden"));
        assert_eq!(
            scrub("fail sk-secret-key-123 here", "sk-secret-key-123"),
            "fail (key) here"
        );
    }
}
