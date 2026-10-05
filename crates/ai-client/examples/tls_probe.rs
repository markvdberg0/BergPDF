//! Connectivity check: `cargo run -p ai-client --example tls_probe -- <anthropic|openai> <key>`.
//! Sends one tiny request ("ping"); with a dummy key the service answers 401, which proves TLS,
//! DNS and headers work without using any account.
use ai_client::{Config, Message, Provider, Request, chat};
use std::time::Duration;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (provider, base, model) = match a.get(1).map(String::as_str) {
        Some("openai") => (
            Provider::OpenAi,
            "https://api.openai.com/v1",
            "gpt-4.1-mini",
        ),
        _ => (
            Provider::Anthropic,
            "https://api.anthropic.com/v1",
            "claude-opus-5-5",
        ),
    };
    let cfg = Config {
        provider,
        base_url: base.into(),
        api_key: a
            .get(2)
            .cloned()
            .unwrap_or_else(|| "sk-dummy-not-a-real-key".into()),
        model: model.into(),
        timeout: Duration::from_secs(20),
    };
    let r = chat(
        &cfg,
        &Request {
            system: "ping",
            messages: &[Message::user("ping")],
            max_tokens: 8,
        },
    );
    println!("{r:?}");
}
