//! A tiny stand-in for an AI service, for trying BergPDF's Copilot without an account:
//! `cargo run -p ai-client --example mock_ai_server -- 8099`, then in Preferences ▸ PDF Copilot
//! choose "Custom", address `http://127.0.0.1:8099/v1`, any model name.
//!
//! It answers the document-question prompts with a fixed JSON answer that quotes a sentence from
//! the repository's report fixtures, and translates by tagging each line (or each block, for inline translation). It understands both the
//! OpenAI and Anthropic wire formats, and logs what it received (never real data).
use std::io::{Read, Write};
use std::net::TcpListener;

fn main() {
    let port = std::env::args().nth(1).unwrap_or_else(|| "8099".into());
    let Ok(l) = TcpListener::bind(format!("127.0.0.1:{port}")) else {
        eprintln!("cannot listen on port {port}");
        return;
    };
    eprintln!("mock AI server on http://127.0.0.1:{port}/v1");
    for c in l.incoming() {
        let Ok(mut s) = c else { continue };
        let mut buf = Vec::new();
        let mut tmp = [0u8; 8192];
        let (mut head_end, mut len) = (None, 0usize);
        loop {
            let n = s.read(&mut tmp).unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            if head_end.is_none()
                && let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n")
            {
                head_end = Some(i + 4);
                let head = String::from_utf8_lossy(&buf[..i]).to_lowercase();
                len = head
                    .lines()
                    .find_map(|l| {
                        l.strip_prefix("content-length:")
                            .map(|v| v.trim().parse().unwrap_or(0))
                    })
                    .unwrap_or(0);
            }
            if let Some(h) = head_end
                && buf.len() >= h + len
            {
                break;
            }
        }
        let body = head_end
            .map(|h| String::from_utf8_lossy(&buf[h..]).to_string())
            .unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        let (system, user) = if let Some(a) = v["system"][0]["text"].as_str() {
            (
                a.to_string(),
                v["messages"]
                    .as_array()
                    .and_then(|m| m.last())
                    .and_then(|m| m["content"].as_str())
                    .unwrap_or("")
                    .to_string(),
            )
        } else {
            let m = v["messages"].as_array().cloned().unwrap_or_default();
            (
                m.first()
                    .and_then(|m| m["content"].as_str())
                    .unwrap_or("")
                    .to_string(),
                m.last()
                    .and_then(|m| m["content"].as_str())
                    .unwrap_or("")
                    .to_string(),
            )
        };
        eprintln!(
            "request: {} chars system, {} chars user",
            system.len(),
            user.len()
        );
        std::thread::sleep(std::time::Duration::from_millis(600));
        let text = if system.contains("JSON array of text blocks") {
            // Block translation: a JSON list in, a JSON list out (each block tagged).
            let blocks: Vec<String> = serde_json::from_str(&user).unwrap_or_default();
            let tagged: Vec<String> = blocks.iter().map(|b| format!("[EN] {b}")).collect();
            serde_json::to_string(&tagged).unwrap_or_default()
        } else if system.contains("professional translator") {
            user.lines()
                .map(|l| {
                    if l.trim().is_empty() {
                        String::new()
                    } else {
                        format!("[EN] {l}")
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        } else if system.contains("Reply with the single word OK") {
            "OK".to_string()
        } else {
            serde_json::json!({
                "answer": "This is a canned answer from the mock server. The report opens with a pangram, and a second paragraph explains line wrapping.",
                "points": [
                    {"page": 1, "quote": "The quick brown fox jumps over the lazy dog", "note": "opening sentence"},
                    {"page": 1, "quote": "wider line of text that continues past the end of the line", "note": "line wrapping"},
                    {"page": 3, "quote": "This sentence does not exist in the document", "note": "invented on purpose"}
                ]
            })
            .to_string()
        };
        let resp = if v.get("system").is_some() {
            serde_json::json!({"content":[{"type":"text","text":text}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}})
        } else {
            serde_json::json!({"choices":[{"message":{"content":text},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1}})
        }
        .to_string();
        let out = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{resp}",
            resp.len()
        );
        let _ = s.write_all(out.as_bytes());
    }
}
