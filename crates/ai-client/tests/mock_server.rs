//! The HTTP path against a throw-away local server: headers, bodies, error mapping, timeouts,
//! and the document/translation flows end to end. No real AI service is contacted.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ai_client::docqa::{self, Ask, Mode};
use ai_client::translate;
use ai_client::{AiError, Config, Message, PageText, Provider, Request, build_document, chat};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug)]
struct Seen {
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

type Handler = Box<dyn Fn(&Seen) -> (u16, String) + Send + Sync>;

struct Server {
    url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    stop: Arc<AtomicBool>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Wake the accept loop.
        let _ = std::net::TcpStream::connect(self.url.trim_start_matches("http://"));
    }
}

fn serve(handler: Handler) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let (seen2, stop2) = (seen.clone(), stop.clone());
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            if stop2.load(Ordering::Relaxed) {
                break;
            }
            let Ok(mut s) = conn else { continue };
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            let header_end;
            loop {
                let n = s.read(&mut tmp).unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    header_end = i + 4;
                    let head = String::from_utf8_lossy(&buf[..i]).to_string();
                    let mut lines = head.lines();
                    let first = lines.next().unwrap_or("").to_string();
                    let path = first.split_whitespace().nth(1).unwrap_or("").to_string();
                    let headers: Vec<(String, String)> = lines
                        .filter_map(|l| l.split_once(':'))
                        .map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string()))
                        .collect();
                    let len: usize = headers
                        .iter()
                        .find(|(k, _)| k == "content-length")
                        .and_then(|(_, v)| v.parse().ok())
                        .unwrap_or(0);
                    while buf.len() < header_end + len {
                        let n = s.read(&mut tmp).unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                    }
                    let body = String::from_utf8_lossy(&buf[header_end..]).to_string();
                    let seen_req = Seen {
                        path,
                        headers,
                        body,
                    };
                    seen2.lock().unwrap().push(seen_req.clone());
                    let (code, resp) = handler(&seen_req);
                    let out = format!(
                        "HTTP/1.1 {code} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{resp}",
                        resp.len()
                    );
                    let _ = s.write_all(out.as_bytes());
                    break;
                }
            }
        }
    });
    Server {
        url: format!("http://{addr}"),
        seen,
        stop,
    }
}

fn cfg(p: Provider, url: &str) -> Config {
    Config {
        provider: p,
        base_url: url.to_string(),
        api_key: "sk-test-secret-0001".into(),
        model: "test-model".into(),
        timeout: Duration::from_secs(5),
    }
}

fn anthropic_body(text: &str) -> String {
    serde_json::json!({
        "content": [{"type": "text", "text": text}],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 10, "output_tokens": 5}
    })
    .to_string()
}

fn openai_body(text: &str) -> String {
    serde_json::json!({
        "choices": [{"message": {"content": text}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5}
    })
    .to_string()
}

fn header<'a>(s: &'a Seen, n: &str) -> Option<&'a str> {
    s.headers
        .iter()
        .find(|(k, _)| k == n)
        .map(|(_, v)| v.as_str())
}

#[test]
fn anthropic_round_trip_sends_the_expected_request() {
    // Loopback http is allowed even for the official provider type, so the wire format can be
    // tested without TLS.
    let srv = serve(Box::new(|_| (200, anthropic_body("Hello from Claude"))));
    let c = cfg(Provider::Anthropic, &format!("{}/v1", srv.url));
    let msgs = [Message::user("Hi")];
    let r = chat(
        &c,
        &Request {
            system: "SYSTEM TEXT",
            messages: &msgs,
            max_tokens: 321,
        },
    )
    .unwrap();
    assert_eq!(r.text, "Hello from Claude");
    assert_eq!(r.input_tokens, Some(10));
    let seen = srv.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let s = &seen[0];
    assert_eq!(s.path, "/v1/messages");
    assert_eq!(header(s, "x-api-key"), Some("sk-test-secret-0001"));
    assert_eq!(header(s, "anthropic-version"), Some("2023-06-01"));
    let body: serde_json::Value = serde_json::from_str(&s.body).unwrap();
    assert_eq!(body["model"], "test-model");
    assert_eq!(body["max_tokens"], 321);
    assert_eq!(body["system"][0]["text"], "SYSTEM TEXT");
    assert_eq!(body["messages"][0]["content"], "Hi");
    assert!(
        !s.path.contains("sk-test"),
        "the key must never be in the URL"
    );
    assert!(
        !s.body.contains("sk-test"),
        "the key must never be in the body"
    );
}

#[test]
fn openai_round_trip_uses_a_bearer_header() {
    let srv = serve(Box::new(|_| (200, openai_body("Hello from GPT"))));
    let c = cfg(Provider::Custom, &format!("{}/v1", srv.url));
    let msgs = [Message::user("Hi")];
    let r = chat(
        &c,
        &Request {
            system: "S",
            messages: &msgs,
            max_tokens: 50,
        },
    )
    .unwrap();
    assert_eq!(r.text, "Hello from GPT");
    let seen = srv.seen.lock().unwrap();
    assert_eq!(seen[0].path, "/v1/chat/completions");
    assert_eq!(
        header(&seen[0], "authorization"),
        Some("Bearer sk-test-secret-0001")
    );
    let body: serde_json::Value = serde_json::from_str(&seen[0].body).unwrap();
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["max_tokens"], 50);
}

#[test]
fn http_errors_become_readable_messages_without_the_key() {
    for (code, needle) in [
        (401u16, "rejected the API key"),
        (429, "rate or usage limit"),
        (500, "had a problem"),
        (404, "model"),
    ] {
        let srv = serve(Box::new(move |_| {
            (
                code,
                r#"{"error":{"message":"detail from server sk-test-secret-0001"}}"#.to_string(),
            )
        }));
        let c = cfg(Provider::Custom, &format!("{}/v1", srv.url));
        let e = chat(
            &c,
            &Request {
                system: "s",
                messages: &[Message::user("q")],
                max_tokens: 10,
            },
        )
        .unwrap_err();
        let AiError::Status { code: got, message } = e else {
            panic!("{e:?}")
        };
        assert_eq!(got, code);
        assert!(message.contains(needle), "{message}");
    }
}

#[test]
fn garbage_and_refusals_are_reported() {
    let srv = serve(Box::new(|_| (200, "<html>not json</html>".into())));
    let c = cfg(Provider::Anthropic, &format!("{}/v1", srv.url));
    let e = chat(
        &c,
        &Request {
            system: "s",
            messages: &[Message::user("q")],
            max_tokens: 10,
        },
    )
    .unwrap_err();
    assert!(matches!(e, AiError::Parse(_)), "{e:?}");
    let srv = serve(Box::new(|_| {
        (200, r#"{"content":[],"stop_reason":"refusal","stop_details":{"type":"refusal","explanation":"no"}}"#.into())
    }));
    let c = cfg(Provider::Anthropic, &format!("{}/v1", srv.url));
    let e = chat(
        &c,
        &Request {
            system: "s",
            messages: &[Message::user("q")],
            max_tokens: 10,
        },
    )
    .unwrap_err();
    assert!(matches!(e, AiError::Refused(_)), "{e:?}");
}

#[test]
fn unreachable_server_and_timeouts_are_network_errors() {
    // Nothing listens on this port.
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    let c = cfg(Provider::Custom, &format!("http://127.0.0.1:{port}/v1"));
    let e = chat(
        &c,
        &Request {
            system: "s",
            messages: &[Message::user("q")],
            max_tokens: 10,
        },
    )
    .unwrap_err();
    assert!(matches!(e, AiError::Network(_)), "{e:?}");
    assert!(!e.to_string().contains("sk-test"));
    // A server that never answers.
    let slow = serve(Box::new(|_| {
        std::thread::sleep(Duration::from_millis(1500));
        (200, openai_body("late"))
    }));
    let mut c = cfg(Provider::Custom, &format!("{}/v1", slow.url));
    c.timeout = Duration::from_millis(300);
    let t = std::time::Instant::now();
    let e = chat(
        &c,
        &Request {
            system: "s",
            messages: &[Message::user("q")],
            max_tokens: 10,
        },
    )
    .unwrap_err();
    assert!(matches!(e, AiError::Network(_)), "{e:?}");
    assert!(t.elapsed() < Duration::from_millis(1400));
}

#[test]
fn nothing_is_sent_when_the_configuration_is_invalid() {
    let srv = serve(Box::new(|_| (200, openai_body("x"))));
    let mut c = cfg(Provider::OpenAi, &format!("{}/v1", srv.url));
    // OpenAI over plain http to a non-loopback name is refused before any connection.
    c.base_url = "http://example.invalid/v1".into();
    assert!(matches!(
        chat(
            &c,
            &Request {
                system: "s",
                messages: &[Message::user("q")],
                max_tokens: 1
            }
        ),
        Err(AiError::Config(_))
    ));
    c.base_url = format!("{}/v1", srv.url);
    c.api_key.clear();
    assert_eq!(
        chat(
            &c,
            &Request {
                system: "s",
                messages: &[Message::user("q")],
                max_tokens: 1
            }
        ),
        Err(AiError::NoKey)
    );
    assert!(srv.seen.lock().unwrap().is_empty());
}

fn pages() -> Vec<PageText> {
    vec![
        PageText {
            number: 1,
            text: "Kickoff meeting notes. The budget is 40 000 euro.".into(),
        },
        PageText {
            number: 2,
            text: "The Client shall pay all invoices within 30 days of receipt.".into(),
        },
    ]
}

#[test]
fn asking_a_question_verifies_quotes_and_sends_the_document_once_per_request() {
    let srv = serve(Box::new(|req| {
        let body: serde_json::Value = serde_json::from_str(&req.body).unwrap();
        let sys = body["system"][0]["text"].as_str().unwrap();
        assert!(sys.contains("[[page 2]]") && sys.contains("pay all invoices"));
        let reply = serde_json::json!({
            "answer": "Invoices are due within 30 days.",
            "points": [
                {"page": 1, "quote": "The Client shall pay all invoices within 30 days of receipt.", "note": "payment term"},
                {"page": 2, "quote": "Late fees are 5 percent", "note": "invented"}
            ]
        })
        .to_string();
        (200, anthropic_body(&reply))
    }));
    let c = cfg(Provider::Anthropic, &format!("{}/v1", srv.url));
    let pg = pages();
    let doc = build_document(&pg, 50_000);
    let a = docqa::ask(&Ask {
        cfg: &c,
        title: "Contract",
        doc: &doc,
        pages: &pg,
        answer_language: "",
        history: &[("Who pays?".into(), "The client.".into())],
        mode: Mode::Ask,
        question: "When are invoices due?",
        extra: None,
    })
    .unwrap();
    assert_eq!(a.text, "Invoices are due within 30 days.");
    assert_eq!(a.points.len(), 2);
    assert_eq!(
        (a.points[0].page, a.points[0].verified),
        (2, true),
        "page corrected"
    );
    assert!(!a.points[1].verified, "invented quote flagged");
    let seen = srv.seen.lock().unwrap();
    let body: serde_json::Value = serde_json::from_str(&seen[0].body).unwrap();
    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 3, "one earlier turn plus the new question");
    assert_eq!(msgs[2]["content"], "When are invoices due?");
}

#[test]
fn translation_splits_long_pages_reports_progress_and_can_be_cancelled() {
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let calls2 = calls.clone();
    let srv = serve(Box::new(move |req| {
        let body: serde_json::Value = serde_json::from_str(&req.body).unwrap();
        let user = body["messages"][1]["content"].as_str().unwrap().to_string();
        let sys = body["messages"][0]["content"].as_str().unwrap();
        assert!(sys.contains("into English"), "{sys}");
        calls2.lock().unwrap().push(user.clone());
        (200, openai_body(&format!("EN({})", user.trim())))
    }));
    let c = cfg(Provider::Custom, &format!("{}/v1", srv.url));
    let long = "Een zin die herhaald wordt. ".repeat(500); // 14 000 chars → 3 chunks
    let pg = vec![
        PageText {
            number: 1,
            text: "Hallo wereld".into(),
        },
        PageText {
            number: 2,
            text: long,
        },
        PageText {
            number: 3,
            text: "   ".into(),
        },
    ];
    let progress = Mutex::new(Vec::new());
    let cancel = AtomicBool::new(false);
    let out = translate::translate_pages(&c, &pg, Some("Dutch"), "English", &cancel, &|d, t| {
        progress.lock().unwrap().push((d, t));
    })
    .unwrap();
    assert_eq!(out[0].text, "EN(Hallo wereld)");
    assert!(
        out[1].text.matches("EN(").count() >= 3,
        "{}",
        out[1].text.len()
    );
    assert_eq!(out[2].text, "", "blank pages are not sent");
    assert_eq!(
        calls.lock().unwrap().len(),
        1 + out[1].text.matches("EN(").count()
    );
    assert_eq!(*progress.lock().unwrap(), vec![(1, 3), (2, 3), (3, 3)]);
    // Cancelling stops before the next request.
    let before = calls.lock().unwrap().len();
    cancel.store(true, Ordering::Relaxed);
    let e = translate::translate_pages(&c, &pg, None, "English", &cancel, &|_, _| {}).unwrap_err();
    assert_eq!(e, AiError::Cancelled);
    assert_eq!(calls.lock().unwrap().len(), before);
}

#[test]
fn block_translation_batches_requests_and_falls_back_for_mangled_replies() {
    let requests = Arc::new(Mutex::new(Vec::<String>::new()));
    let r2 = requests.clone();
    let srv = serve(Box::new(move |req| {
        let body: serde_json::Value = serde_json::from_str(&req.body).unwrap();
        let user = body["messages"][1]["content"].as_str().unwrap().to_string();
        r2.lock().unwrap().push(user.clone());
        // A JSON list in → a JSON list out, except when a block says "BREAK": then the reply
        // has the wrong length, which must trigger the one-by-one fallback.
        match serde_json::from_str::<Vec<String>>(&user) {
            Ok(list) if list.iter().any(|b| b.contains("BREAK")) => {
                (200, openai_body("[\"only one\"]"))
            }
            Ok(list) => {
                let t: Vec<String> = list.iter().map(|b| format!("EN:{b}")).collect();
                (200, openai_body(&serde_json::to_string(&t).unwrap()))
            }
            Err(_) => (200, openai_body(&format!("EN:{user}"))),
        }
    }));
    let c = cfg(Provider::Custom, &format!("{}/v1", srv.url));
    let cancel = AtomicBool::new(false);
    let blocks: Vec<String> = (0..5).map(|i| format!("blok {i}")).collect();
    let progress = Mutex::new(Vec::new());
    let out =
        translate::translate_blocks(&c, &blocks, Some("Dutch"), "English", &cancel, &|d, t| {
            progress.lock().unwrap().push((d, t));
        })
        .unwrap();
    assert_eq!(
        out,
        (0..5).map(|i| format!("EN:blok {i}")).collect::<Vec<_>>()
    );
    assert_eq!(
        requests.lock().unwrap().len(),
        1,
        "five short blocks, one request"
    );
    assert_eq!(*progress.lock().unwrap(), vec![(5, 5)]);

    // A bad reply for a batch: every block of it is translated on its own.
    requests.lock().unwrap().clear();
    let blocks = vec![
        "één".to_string(),
        "BREAK twee".to_string(),
        "drie".to_string(),
    ];
    let out =
        translate::translate_blocks(&c, &blocks, None, "English", &cancel, &|_, _| {}).unwrap();
    assert_eq!(out, vec!["EN:één", "EN:BREAK twee", "EN:drie"]);
    assert_eq!(requests.lock().unwrap().len(), 1 + 3);

    // Many blocks are split over several requests.
    requests.lock().unwrap().clear();
    let many: Vec<String> = (0..95).map(|i| format!("b{i}")).collect();
    let out = translate::translate_blocks(&c, &many, None, "English", &cancel, &|_, _| {}).unwrap();
    assert_eq!(out.len(), 95);
    assert!(
        out.iter()
            .enumerate()
            .all(|(i, t)| *t == format!("EN:b{i}"))
    );
    assert_eq!(requests.lock().unwrap().len(), 3);
    // Cancelling stops before the next request.
    cancel.store(true, Ordering::Relaxed);
    let before = requests.lock().unwrap().len();
    let e =
        translate::translate_blocks(&c, &many, None, "English", &cancel, &|_, _| {}).unwrap_err();
    assert_eq!(e, AiError::Cancelled);
    assert_eq!(requests.lock().unwrap().len(), before);
}

// ---- verified downloads ---------------------------------------------------------------------

use ai_client::download::{DownloadError, fetch_verified, sha256_hex};

fn file_server(body: Vec<u8>, code: u16) -> Server {
    // The mock server answers text; for binary bodies use a Latin-1-safe payload.
    let text = String::from_utf8(body).unwrap();
    serve(Box::new(move |_| (code, text.clone())))
}

#[test]
fn a_download_with_the_right_checksum_is_stored_and_reports_progress() {
    let payload = "model-bytes-".repeat(5000);
    let srv = file_server(payload.clone().into_bytes(), 200);
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("sub/m.rten");
    let seen = Mutex::new(Vec::new());
    fetch_verified(
        &format!("{}/m.rten", srv.url),
        &sha256_hex(payload.as_bytes()),
        &dest,
        &|g, t| seen.lock().unwrap().push((g, t)),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), payload);
    assert!(!dest.with_extension("partial").exists());
    let seen = seen.lock().unwrap();
    assert_eq!(seen.last().unwrap().0 as usize, payload.len());
    assert_eq!(seen.last().unwrap().1, Some(payload.len() as u64));
}

#[test]
fn a_wrong_checksum_discards_the_file_and_never_replaces_a_good_one() {
    let srv = file_server(b"tampered".to_vec(), 200);
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("m.rten");
    std::fs::write(&dest, "good existing file").unwrap();
    let e = fetch_verified(
        &format!("{}/m.rten", srv.url),
        &sha256_hex(b"what we expected"),
        &dest,
        &|_, _| {},
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert_eq!(e, DownloadError::BadChecksum);
    assert_eq!(
        std::fs::read_to_string(&dest).unwrap(),
        "good existing file"
    );
    assert!(!dest.with_extension("partial").exists());
}

#[test]
fn http_errors_cancel_and_non_https_addresses_are_handled() {
    let srv = file_server(b"nope".to_vec(), 404);
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("m.rten");
    assert_eq!(
        fetch_verified(
            &format!("{}/x", srv.url),
            "00",
            &dest,
            &|_, _| {},
            &AtomicBool::new(false)
        ),
        Err(DownloadError::Status(404))
    );
    let ok = file_server(b"data".to_vec(), 200);
    assert_eq!(
        fetch_verified(
            &format!("{}/x", ok.url),
            &sha256_hex(b"data"),
            &dest,
            &|_, _| {},
            &AtomicBool::new(true)
        ),
        Err(DownloadError::Cancelled)
    );
    assert!(!dest.exists() && !dest.with_extension("partial").exists());
    // Plain http to a non-loopback host is refused before any connection.
    assert!(matches!(
        fetch_verified(
            "http://example.invalid/m",
            "00",
            &dest,
            &|_, _| {},
            &AtomicBool::new(false)
        ),
        Err(DownloadError::Url(_))
    ));
}
