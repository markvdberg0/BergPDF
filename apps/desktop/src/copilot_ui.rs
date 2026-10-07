//! PDF Copilot: the chat panel, the background jobs that talk to the AI provider, the consent
//! dialog and turning quoted passages into highlights.
//!
//! Network traffic only happens inside the worker threads started here, and only after the
//! person pressed a button (Send, a suggestion, Explain, Test…). The document text sent is
//! bounded by the "characters sent" setting, and the first use per provider needs consent.

use crate::dialogs::modal;
use crate::i18n::tr;
use crate::state::*;
use crate::tf;
use ai_client::docqa::{Ask, Mode};
use ai_client::{AiError, Answer, Config, PageText, Provider, build_document};
use editor_core::prefs::AiProvider;
use editor_core::session::DocId;
use egui::{Align, Layout, RichText};
use pdf_engine::geom::{Point, Quad};
use pdf_engine::render::with_session;
use pdf_engine::text::TextPage;
use std::collections::{HashMap, HashSet};
use std::panic::catch_unwind;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant};

// ---- chat model -------------------------------------------------------------------------------

/// A supporting passage shown under an answer.
#[derive(Clone, Debug)]
pub struct ChatPoint {
    /// 1-based page.
    pub page: usize,
    pub quote: String,
    pub note: String,
    /// The passage was found in the document text.
    pub verified: bool,
    /// A highlight was already created from it.
    pub applied: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatKind {
    User,
    Assistant,
    Error,
    Info,
}

#[derive(Clone, Debug)]
pub struct ChatMsg {
    pub kind: ChatKind,
    pub text: String,
    pub points: Vec<ChatPoint>,
}

/// One conversation per open document.
#[derive(Default)]
pub struct CopilotChat {
    pub msgs: Vec<ChatMsg>,
    pub input: String,
    pub stick_bottom: bool,
}

impl CopilotChat {
    /// Earlier (question, answer) pairs for the provider.
    fn history(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut q: Option<&str> = None;
        for m in &self.msgs {
            match m.kind {
                ChatKind::User => q = Some(&m.text),
                ChatKind::Assistant => {
                    if let Some(q) = q.take() {
                        out.push((q.to_string(), m.text.clone()));
                    }
                }
                _ => {}
            }
        }
        out
    }
}

// ---- runtime ---------------------------------------------------------------------------------

/// What to do once the person agreed to send text to the provider.
#[derive(Clone, Debug)]
pub enum AiPending {
    Ask(AiAction),
    OpenTranslate,
    TranslateSelection(String),
}

/// A request for the model.
#[derive(Clone, Debug)]
pub struct AiAction {
    /// What the person sees as their message.
    pub shown: String,
    pub mode: Mode,
    /// The question, or the passage for Explain.
    pub question: String,
    /// Restrict the document text to the neighbourhood of this page (0-based).
    pub around_page: Option<usize>,
}

enum Outcome {
    Answer {
        doc: DocId,
        rev: u64,
        answer: Answer,
        pages: Option<Arc<Vec<PageText>>>,
    },
    Translated {
        doc: DocId,
        text: String,
        source: Option<String>,
        target: String,
    },
    Tested(Result<String, String>),
    Failed {
        doc: DocId,
        error: String,
    },
}

struct Job {
    rx: Receiver<Outcome>,
    doc: DocId,
    cancel: Arc<AtomicBool>,
    label: String,
    started: Instant,
}

/// Application-wide Copilot state.
#[derive(Default)]
pub struct AiRuntime {
    job: Option<Job>,
    /// The API key, loaded once and refreshed when it changes.
    key: Option<Option<String>>,
    /// Text of the key field in Preferences (never persisted by itself).
    pub key_input: String,
    pub key_show: bool,
    pub test_result: Option<Result<String, String>>,
    /// Extracted page text per (document, revision).
    pages: HashMap<DocId, (u64, Arc<Vec<PageText>>)>,
    /// Action waiting for consent.
    pub pending: Option<AiPending>,
}

fn to_provider(p: AiProvider) -> Provider {
    match p {
        AiProvider::OpenAi => Provider::OpenAi,
        AiProvider::Anthropic => Provider::Anthropic,
        AiProvider::Custom => Provider::Custom,
    }
}

/// The host part of a URL, for messages.
pub fn host_of(url: &str) -> String {
    url.split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', '?'])
        .next()
        .unwrap_or("")
        .to_string()
}

/// Text of every page.
pub fn extract_all(
    bytes: Arc<Vec<u8>>,
    geoms: &[(usize, pdf_engine::geom::PageGeometry)],
    cancel: &AtomicBool,
) -> Result<Vec<PageText>, String> {
    extract_pages(bytes, geoms, None, &HashSet::new(), cancel).map(|(p, _)| p)
}

fn extract_pages(
    bytes: Arc<Vec<u8>>,
    geoms: &[(usize, pdf_engine::geom::PageGeometry)],
    only: Option<&HashSet<usize>>,
    keep: &HashSet<usize>,
    cancel: &AtomicBool,
) -> Result<(Vec<PageText>, HashMap<usize, TextPage>), String> {
    with_session(bytes, |s| {
        let mut pages = Vec::new();
        let mut kept = HashMap::new();
        for (i, g) in geoms {
            if cancel.load(Ordering::Relaxed) {
                return Ok((pages, kept));
            }
            if only.is_some_and(|o| !o.contains(i)) {
                continue;
            }
            let tp = s.extract_text(*i, g)?;
            pages.push(PageText {
                number: i + 1,
                text: tp.plain_text(),
            });
            if keep.contains(i) {
                kept.insert(*i, tp);
            }
        }
        Ok((pages, kept))
    })
    .map_err(|e| e.to_string())
}

/// Document, revision, title, bytes and page geometries of the active tab.
type DocInputs = (
    DocId,
    u64,
    String,
    Arc<Vec<u8>>,
    Vec<(usize, pdf_engine::geom::PageGeometry)>,
);

/// One annotation, flattened for the summary prompt.
#[derive(Clone)]
struct AnnoRow {
    page: usize,
    subtype: String,
    author: String,
    contents: String,
    quads: Vec<Quad>,
}

fn text_under(tp: &TextPage, quads: &[Quad]) -> String {
    let boxes: Vec<_> = quads.iter().map(Quad::bounds).collect();
    let mut s = String::new();
    for g in &tp.glyphs {
        let b = g.quad.bounds();
        let c = Point::new((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0);
        if boxes.iter().any(|r| r.inflate(1.0, 1.0).contains(c)) {
            s.push_str(&g.text);
        }
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn annotation_block(rows: &[AnnoRow], texts: &HashMap<usize, TextPage>) -> String {
    let clip = |s: &str, n: usize| ai_client::text::clip_chars(s, n).to_string();
    let mut s = String::from("<annotations>\n");
    for r in rows.iter().take(400) {
        let quoted = texts
            .get(&r.page)
            .map(|tp| text_under(tp, &r.quads))
            .unwrap_or_default();
        s.push_str(&format!("page {} | {}", r.page + 1, r.subtype));
        if !r.author.is_empty() {
            s.push_str(&format!(" | by {}", clip(&r.author, 60)));
        }
        if !quoted.is_empty() {
            s.push_str(&format!(" | marked text: «{}»", clip(&quoted, 300)));
        }
        if !r.contents.trim().is_empty() {
            s.push_str(&format!(
                " | comment: {}",
                clip(&r.contents.replace('\n', " "), 600)
            ));
        }
        s.push('\n');
    }
    s.push_str("</annotations>");
    s
}

// ---- finding a quote on a page ------------------------------------------------------------------

/// Glyph range of `quote` in `tp`, tolerant of line breaks, hyphenation, ligatures and quote
/// styles (the same normalisation that verified the quote against the extracted text).
pub fn locate_quote(tp: &TextPage, quote: &str) -> Option<std::ops::Range<usize>> {
    let needle: Vec<char> = ai_client::text::normalize(quote).chars().collect();
    if needle.len() < 4 {
        return None;
    }
    let mut hay: Vec<(char, usize)> = Vec::new();
    for (idx, g) in tp.glyphs.iter().enumerate() {
        if g.text.trim().is_empty() {
            if hay.last().is_some_and(|(c, _)| *c != ' ') {
                hay.push((' ', idx));
            }
            continue;
        }
        for c in ai_client::text::normalize(&g.text).chars() {
            if c == ' ' {
                if hay.last().is_some_and(|(c, _)| *c != ' ') {
                    hay.push((' ', idx));
                }
            } else {
                hay.push((c, idx));
            }
        }
    }
    // Drop line-end hyphens: "cus- tomer" → "customer".
    let mut k = 0;
    while k + 2 < hay.len() {
        if hay[k].0 == '-' && hay[k + 1].0 == ' ' && hay[k + 2].0.is_alphabetic() {
            hay.drain(k..k + 2);
        } else {
            k += 1;
        }
    }
    let n = needle.len();
    let start =
        (0..=hay.len().checked_sub(n)?).find(|&s| (0..n).all(|j| hay[s + j].0 == needle[j]))?;
    Some(hay[start].1..hay[start + n - 1].1 + 1)
}

impl App {
    // ---- configuration --------------------------------------------------------------------

    /// The key currently available (environment variable or key file), cached.
    pub fn ai_key(&mut self) -> Option<String> {
        if self.ai.key.is_none() {
            self.ai.key = Some(platform::secrets::load_ai_key());
        }
        self.ai.key.clone().flatten()
    }

    pub fn ai_key_changed(&mut self) {
        self.ai.key = None;
    }

    /// Connection settings from preferences and the stored key.
    pub fn ai_config(&mut self) -> Result<Config, AiError> {
        let s = &self.prefs.ai;
        let provider = to_provider(s.provider);
        let (model, base_url) = (s.model().to_string(), s.base_url());
        let key = self.ai_key().unwrap_or_default();
        if key.is_empty() && provider != Provider::Custom {
            return Err(AiError::NoKey);
        }
        Ok(Config {
            provider,
            base_url,
            api_key: key,
            model,
            timeout: Duration::from_secs(180),
        })
    }

    pub fn ai_cached_pages(&self, doc: DocId, rev: u64) -> Option<Arc<Vec<PageText>>> {
        self.ai
            .pages
            .get(&doc)
            .filter(|(r, _)| *r == rev)
            .map(|(_, p)| p.clone())
    }

    pub fn ai_store_pages(&mut self, doc: DocId, rev: u64, pages: Arc<Vec<PageText>>) {
        self.ai.pages.insert(doc, (rev, pages));
    }

    pub fn ai_busy(&self) -> bool {
        self.ai.job.is_some()
    }

    /// Ask for consent when the provider has not been agreed to yet. Returns true to go ahead.
    pub fn ai_consent_or_ask(&mut self, pending: AiPending) -> bool {
        if self.prefs.ai_consent_provider == Some(self.prefs.ai.provider) {
            return true;
        }
        self.ai.pending = Some(pending.clone());
        self.dialog = Some(Dialog::AiConsent);
        false
    }

    pub fn dialog_ai_consent(&mut self, ctx: &egui::Context) -> bool {
        let mut allow = false;
        let mut cancel = false;
        let provider = self.prefs.ai.provider;
        let host = host_of(&self.prefs.ai.base_url());
        let dim = self.pal.text_dim;
        modal(ctx, "ai_consent", |ui| {
            ui.heading(tr("Send text to an AI service?"));
            ui.label(tf!("PDF Copilot and Translate send text from your document (for a question: the pages' text; for Explain: the selection and nearby pages) to {} ({}) using your API key.", provider.title(), host));
            ui.add_space(4.0);
            ui.label(tr(
                "• Nothing is sent until you press a button such as Send, Summarize or Translate.",
            ));
            ui.label(tr(
                "• BergPDF itself has no other network traffic and does not collect anything.",
            ));
            ui.label(tr("• What the service does with the text is governed by your agreement with that provider."));
            ui.label(
                RichText::new(tr("Do not use this with documents you are not allowed to share with that provider."))
                    .size(12.0)
                    .color(dim),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(tr("Allow for this provider")).clicked() {
                    allow = true;
                }
                if ui.button(tr("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    cancel = true;
                }
            });
        });
        if allow {
            self.prefs.ai_consent_provider = Some(provider);
            self.prefs_dirty = true;
            let pending = self.ai.pending.take();
            self.dialog = None;
            if let Some(p) = pending {
                self.run_pending(ctx, p);
            }
            return false;
        }
        if cancel {
            self.ai.pending = None;
            return false;
        }
        true
    }

    pub fn run_pending(&mut self, ctx: &egui::Context, p: AiPending) {
        match p {
            AiPending::Ask(a) => self.start_ask(ctx, a),
            AiPending::OpenTranslate => self.open_translate_dialog(ctx),
            AiPending::TranslateSelection(t) => self.start_translate_selection(ctx, t),
        }
    }

    // ---- entry points ----------------------------------------------------------------------

    fn show_copilot_panel(&mut self) {
        self.prefs.show_right_sidebar = true;
        self.right_tab = RightTab::Copilot;
        self.prefs.show_copilot = true;
        self.prefs_dirty = true;
    }

    pub fn toggle_copilot(&mut self) {
        if self.prefs.show_right_sidebar && self.right_tab == RightTab::Copilot {
            self.prefs.show_right_sidebar = false;
            self.prefs.show_copilot = false;
        } else {
            self.show_copilot_panel();
        }
        self.prefs_dirty = true;
    }

    /// Ask with consent handling (used by the panel, the ribbon and the context menu).
    pub fn ai_ask(&mut self, ctx: &egui::Context, a: AiAction) {
        self.show_copilot_panel();
        if self.tabs.is_empty() {
            return;
        }
        if let Err(e) = self.ai_config() {
            self.chat_push(ChatKind::Error, e.to_string());
            return;
        }
        if self.ai_consent_or_ask(AiPending::Ask(a.clone())) {
            self.start_ask(ctx, a);
        }
    }

    pub fn copilot_summarize(&mut self, ctx: &egui::Context) {
        self.ai_ask(
            ctx,
            AiAction {
                shown: tr("Summarize this document").into(),
                mode: Mode::Summarize,
                question: String::new(),
                around_page: None,
            },
        );
    }

    pub fn copilot_summarize_annotations(&mut self, ctx: &egui::Context) {
        self.ai_ask(
            ctx,
            AiAction {
                shown: tr("Summarize the annotations").into(),
                mode: Mode::Annotations,
                question: String::new(),
                around_page: None,
            },
        );
    }

    pub fn copilot_explain(&mut self, ctx: &egui::Context, text: &str, page: usize) {
        let t = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let shown = tf!("Explain: “{}”", ai_client::text::clip_chars(&t, 160));
        self.ai_ask(
            ctx,
            AiAction {
                shown,
                mode: Mode::Explain,
                question: ai_client::text::clip_chars(&t, 2000).to_string(),
                around_page: Some(page),
            },
        );
    }

    pub fn copilot_translate_selection(&mut self, ctx: &egui::Context, text: &str) {
        self.show_copilot_panel();
        if let Err(e) = self.ai_config() {
            self.chat_push(ChatKind::Error, e.to_string());
            return;
        }
        let t = ai_client::text::clip_chars(text.trim(), 6000).to_string();
        if self.ai_consent_or_ask(AiPending::TranslateSelection(t.clone())) {
            self.start_translate_selection(ctx, t);
        }
    }

    fn chat_push(&mut self, kind: ChatKind, text: String) {
        if let Some(t) = self.tabs.get_mut(self.active) {
            t.ui.copilot.msgs.push(ChatMsg {
                kind,
                text,
                points: Vec::new(),
            });
            t.ui.copilot.stick_bottom = true;
        }
    }

    // ---- jobs --------------------------------------------------------------------------------

    fn doc_inputs(&mut self) -> Option<DocInputs> {
        let tab = self.tabs.get_mut(self.active)?;
        let pages = tab.session.pages().ok()?;
        let geoms = pages
            .iter()
            .enumerate()
            .map(|(i, p)| (i, p.geometry))
            .collect();
        let snap = tab.session.snapshot().ok()?;
        Some((
            tab.session.id,
            snap.revision,
            tab.session.title.clone(),
            snap.bytes,
            geoms,
        ))
    }

    fn annotation_rows(&mut self) -> Vec<AnnoRow> {
        let Some(tab) = self.tabs.get(self.active) else {
            return Vec::new();
        };
        let Ok(ids) = tab.session.doc().page_ids() else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        for (i, id) in ids.into_iter().enumerate() {
            for a in self.annots_for(id).iter() {
                if matches!(a.subtype.as_str(), "Link" | "Popup" | "Widget") {
                    continue;
                }
                let quads = match a.spec.as_ref().map(|s| &s.kind) {
                    Some(pdf_engine::annot::AnnotationKind::Highlight { quads })
                    | Some(pdf_engine::annot::AnnotationKind::Underline { quads })
                    | Some(pdf_engine::annot::AnnotationKind::StrikeOut { quads })
                    | Some(pdf_engine::annot::AnnotationKind::Squiggly { quads }) => quads.clone(),
                    _ => Vec::new(),
                };
                rows.push(AnnoRow {
                    page: i,
                    subtype: a.subtype.clone(),
                    author: a.author.clone(),
                    contents: a.contents.clone(),
                    quads,
                });
            }
        }
        rows
    }

    fn start_ask(&mut self, ctx: &egui::Context, a: AiAction) {
        if self.ai.job.is_some() || self.tabs.is_empty() {
            return;
        }
        let cfg = match self.ai_config() {
            Ok(c) => c,
            Err(e) => {
                self.chat_push(ChatKind::Error, e.to_string());
                return;
            }
        };
        let rows = if a.mode == Mode::Annotations {
            let r = self.annotation_rows();
            if r.is_empty() {
                self.chat_push(ChatKind::User, a.shown.clone());
                self.chat_push(
                    ChatKind::Info,
                    tr("This document has no comments or markup to summarize.").into(),
                );
                return;
            }
            r
        } else {
            Vec::new()
        };
        let Some((doc, rev, title, bytes, geoms)) = self.doc_inputs() else {
            return;
        };
        let history = self.tabs[self.active].ui.copilot.history();
        self.chat_push(ChatKind::User, a.shown.clone());
        let cached = self
            .ai
            .pages
            .get(&doc)
            .filter(|(r, _)| *r == rev)
            .map(|(_, p)| p.clone());
        let max_chars = self.prefs.ai.max_chars as usize;
        let lang = self.prefs.ai.answer_language.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let (c2, label) = (cancel.clone(), a.shown.clone());
        std::thread::spawn(move || {
            let out = catch_unwind(std::panic::AssertUnwindSafe(
                || -> Result<Outcome, String> {
                    let keep: HashSet<usize> = rows
                        .iter()
                        .filter(|r| !r.quads.is_empty())
                        .map(|r| r.page)
                        .collect();
                    let around: Option<HashSet<usize>> = a
                        .around_page
                        .map(|p| [p.saturating_sub(1), p, p + 1].into_iter().collect());
                    let (pages, kept, store): (Arc<Vec<PageText>>, HashMap<usize, TextPage>, bool) =
                        match (&cached, around.as_ref(), keep.is_empty()) {
                            (Some(p), _, true) => (p.clone(), HashMap::new(), false),
                            _ => {
                                let (p, k) =
                                    extract_pages(bytes, &geoms, around.as_ref(), &keep, &c2)?;
                                (Arc::new(p), k, around.is_none())
                            }
                        };
                    if c2.load(Ordering::Relaxed) {
                        return Err("Cancelled.".into());
                    }
                    let subset: Vec<PageText> = match &around {
                        Some(set) => pages
                            .iter()
                            .filter(|p| set.contains(&(p.number - 1)))
                            .cloned()
                            .collect(),
                        None => pages.as_ref().clone(),
                    };
                    let doc_text = build_document(&subset, max_chars);
                    if doc_text.pages_with_text == 0 {
                        return Err(
                        tr("This document has no text to work with. If it is a scan, run OCR first (Edit ▸ Recognize Text).").into(),
                    );
                    }
                    let extra =
                        (a.mode == Mode::Annotations).then(|| annotation_block(&rows, &kept));
                    let ans = ai_client::ask(&Ask {
                        cfg: &cfg,
                        title: &title,
                        doc: &doc_text,
                        pages: &subset,
                        answer_language: &lang,
                        history: &history,
                        mode: a.mode,
                        question: &a.question,
                        extra: extra.as_deref(),
                    })
                    .map_err(|e| e.to_string())?;
                    Ok(Outcome::Answer {
                        doc,
                        rev,
                        answer: ans,
                        pages: store.then_some(pages),
                    })
                },
            ));
            let msg = match out {
                Ok(Ok(o)) => o,
                Ok(Err(e)) => Outcome::Failed { doc, error: e },
                Err(_) => Outcome::Failed {
                    doc,
                    error: tr("The AI request stopped unexpectedly.").into(),
                },
            };
            let _ = tx.send(msg);
        });
        self.ai.job = Some(Job {
            rx,
            doc,
            cancel,
            label,
            started: Instant::now(),
        });
        ctx.request_repaint();
    }

    fn start_translate_selection(&mut self, ctx: &egui::Context, text: String) {
        if self.ai.job.is_some() {
            return;
        }
        let cfg = match self.ai_config() {
            Ok(c) => c,
            Err(e) => {
                self.chat_push(ChatKind::Error, e.to_string());
                return;
            }
        };
        let Some(doc) = self.tabs.get(self.active).map(|t| t.session.id) else {
            return;
        };
        let target = self.prefs.ai.translate_to.clone();
        let shown = tf!(
            "Translate to {}: “{}”",
            target,
            ai_client::text::clip_chars(
                &text.split_whitespace().collect::<Vec<_>>().join(" "),
                120
            )
        );
        self.chat_push(ChatKind::User, shown.clone());
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let c2 = cancel.clone();
        std::thread::spawn(move || {
            let source = ai_client::translate::detect(&text).map(|d| d.name);
            let r = catch_unwind(std::panic::AssertUnwindSafe(|| {
                ai_client::translate::translate_text(&cfg, &text, source.as_deref(), &target, &c2)
                    .map(|t| Outcome::Translated {
                        doc,
                        text: t,
                        source: source.clone(),
                        target: target.clone(),
                    })
                    .map_err(|e| e.to_string())
            }));
            let msg = match r {
                Ok(Ok(o)) => o,
                Ok(Err(e)) => Outcome::Failed { doc, error: e },
                Err(_) => Outcome::Failed {
                    doc,
                    error: tr("The AI request stopped unexpectedly.").into(),
                },
            };
            let _ = tx.send(msg);
        });
        self.ai.job = Some(Job {
            rx,
            doc,
            cancel,
            label: shown,
            started: Instant::now(),
        });
        ctx.request_repaint();
    }

    /// Check the key and server with one tiny request that contains no document text.
    pub fn ai_test_connection(&mut self, ctx: &egui::Context) {
        if self.ai.job.is_some() {
            return;
        }
        let cfg = match self.ai_config() {
            Ok(c) => c,
            Err(e) => {
                self.ai.test_result = Some(Err(e.to_string()));
                return;
            }
        };
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let r = ai_client::chat(
                &cfg,
                &ai_client::Request {
                    system: "Reply with the single word OK.",
                    messages: &[ai_client::Message::user("ping")],
                    max_tokens: 64,
                },
            );
            let _ = tx.send(Outcome::Tested(match r {
                Ok(_) => Ok(tf!(
                    "Connected to {} (model {}).",
                    host_of(&cfg.base_url),
                    cfg.model
                )),
                Err(e) => Err(e.to_string()),
            }));
        });
        self.ai.test_result = None;
        self.ai.job = Some(Job {
            rx,
            doc: DocId(u64::MAX),
            cancel: Arc::new(AtomicBool::new(false)),
            label: tr("Testing the connection").into(),
            started: Instant::now(),
        });
        ctx.request_repaint();
    }

    pub fn cancel_ai(&mut self) {
        if let Some(j) = self.ai.job.take() {
            j.cancel.store(true, Ordering::Relaxed);
            if j.doc.0 != u64::MAX
                && let Some(t) = self.tabs.iter_mut().find(|t| t.session.id == j.doc)
            {
                t.ui.copilot.msgs.push(ChatMsg {
                    kind: ChatKind::Info,
                    text: "Cancelled.".into(),
                    points: Vec::new(),
                });
            }
        }
    }

    /// Collect a finished job (called every frame).
    pub fn poll_ai(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.ai.job else { return };
        let outcome = match job.rx.try_recv() {
            Ok(o) => Some(o),
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(120));
                None
            }
            Err(TryRecvError::Disconnected) => Some(Outcome::Failed {
                doc: job.doc,
                error: tr("The AI request stopped unexpectedly.").into(),
            }),
        };
        let Some(o) = outcome else { return };
        self.ai.job = None;
        let push = |app: &mut App, doc: DocId, m: ChatMsg| {
            if let Some(t) = app.tabs.iter_mut().find(|t| t.session.id == doc) {
                t.ui.copilot.msgs.push(m);
                t.ui.copilot.stick_bottom = true;
            }
        };
        match o {
            Outcome::Answer {
                doc,
                rev,
                answer,
                pages,
            } => {
                if let Some(p) = pages {
                    self.ai.pages.insert(doc, (rev, p));
                }
                let mut text = answer.text;
                if answer.reply_truncated {
                    text.push_str("\n[The answer was cut off by the length limit.]");
                }
                let points = answer
                    .points
                    .into_iter()
                    .map(|p| ChatPoint {
                        page: p.page,
                        quote: p.quote,
                        note: p.note,
                        verified: p.verified,
                        applied: false,
                    })
                    .collect();
                push(
                    self,
                    doc,
                    ChatMsg {
                        kind: ChatKind::Assistant,
                        text,
                        points,
                    },
                );
            }
            Outcome::Translated {
                doc,
                text,
                source,
                target,
            } => {
                let head = match source {
                    Some(s) => format!("{s} → {target}\n"),
                    None => format!("→ {target}\n"),
                };
                push(
                    self,
                    doc,
                    ChatMsg {
                        kind: ChatKind::Assistant,
                        text: format!("{head}{text}"),
                        points: Vec::new(),
                    },
                );
            }
            Outcome::Tested(r) => self.ai.test_result = Some(r),
            Outcome::Failed { doc, error } => push(
                self,
                doc,
                ChatMsg {
                    kind: ChatKind::Error,
                    text: error,
                    points: Vec::new(),
                },
            ),
        }
        ctx.request_repaint();
    }

    // ---- highlights from points --------------------------------------------------------------

    /// Create a highlight annotation for `points[idx]` of message `mi`. Returns whether it worked.
    pub fn highlight_point(&mut self, mi: usize, pi: usize) -> bool {
        let ti = self.active;
        let Some(p) = self
            .tabs
            .get(ti)
            .and_then(|t| t.ui.copilot.msgs.get(mi))
            .and_then(|m| m.points.get(pi))
            .cloned()
        else {
            return false;
        };
        if p.page == 0 || !p.verified {
            return false;
        }
        let Ok(pages) = self.tabs[ti].session.pages() else {
            return false;
        };
        let Some(info) = pages.get(p.page - 1) else {
            return false;
        };
        let (page_id, geom) = (info.id, info.geometry);
        let Ok(snap) = self.tabs[ti].session.snapshot() else {
            return false;
        };
        let range = with_session(snap.bytes, |s| {
            let tp = s.extract_text(p.page - 1, &geom)?;
            Ok(locate_quote(&tp, &p.quote).map(|r| tp.selection_quads(r)))
        })
        .ok()
        .flatten();
        let Some(quads) = range.filter(|q| !q.is_empty()) else {
            self.notify_error(tr("Could not find that passage on the page any more."));
            return false;
        };
        let spec = self.new_spec(pdf_engine::annot::AnnotationKind::Highlight { quads });
        self.add_annotation(page_id, spec, tr("Highlight (Copilot)"));
        if let Some(pt) = self.tabs[ti]
            .ui
            .copilot
            .msgs
            .get_mut(mi)
            .and_then(|m| m.points.get_mut(pi))
        {
            pt.applied = true;
        }
        true
    }

    // ---- the panel ---------------------------------------------------------------------------

    pub fn copilot_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let pal = self.pal;
        let has_key = self.ai_key().is_some() || self.prefs.ai.provider == AiProvider::Custom;
        let host = host_of(&self.prefs.ai.base_url());
        let busy = self.ai_busy();
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("PDF Copilot")).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button(tr("Settings")).clicked() {
                    self.dialog = Some(Dialog::Preferences {
                        filter: "ai".into(),
                    });
                }
                if ui
                    .small_button(tr("Clear"))
                    .on_hover_text(tr("Clear this conversation"))
                    .clicked()
                    && let Some(t) = self.tabs.get_mut(self.active)
                {
                    t.ui.copilot.msgs.clear();
                }
            });
        });
        ui.add_space(2.0);
        if !self.text_use_allowed() {
            ui.label(tr("The author of this document did not allow copying its text, so PDF Copilot and Translate are not available for it."));
            return;
        }
        if !has_key {
            ui.label(tr("Add your own API key to turn on Copilot. It can summarize this document, answer questions with page references and highlights, explain selected text, summarize comments and translate."));
            ui.add_space(6.0);
            if ui.button(tr("Set up an AI provider…")).clicked() {
                self.dialog = Some(Dialog::Preferences {
                    filter: "ai".into(),
                });
            }
            ui.add_space(6.0);
            ui.label(
                RichText::new(tr("BergPDF has no AI of its own and sends nothing anywhere until you set this up and press a button."))
                    .size(11.0)
                    .color(pal.text_dim),
            );
            return;
        }
        // Suggestions.
        let mut action: Option<AiAction> = None;
        let mut translate = false;
        ui.horizontal_wrapped(|ui| {
            for (label, mode, shown) in [
                (
                    tr("Summarize"),
                    Mode::Summarize,
                    tr("Summarize this document"),
                ),
                (
                    tr("Main points"),
                    Mode::Ask,
                    tr("List the main points of this document"),
                ),
                (
                    tr("Annotation summary"),
                    Mode::Annotations,
                    tr("Summarize the annotations"),
                ),
                (
                    tr("Key obligations"),
                    Mode::Ask,
                    tr("What are the key obligations, deadlines and amounts in this document?"),
                ),
            ] {
                if ui.add_enabled(!busy, egui::Button::new(label)).clicked() {
                    action = Some(AiAction {
                        shown: shown.into(),
                        mode,
                        question: shown.into(),
                        around_page: None,
                    });
                }
            }
            if ui
                .add_enabled(!busy, egui::Button::new(tr("Translate…")))
                .clicked()
            {
                translate = true;
            }
        });
        ui.separator();
        // Conversation.
        let footer_h = 96.0;
        let ti = self.active;
        let avail = ui.available_height() - footer_h;
        let mut jump: Option<usize> = None;
        let mut hl: Option<(usize, usize)> = None;
        let mut hl_all: Option<usize> = None;
        let stick = self.tabs[ti].ui.copilot.stick_bottom;
        egui::ScrollArea::vertical()
            .max_height(avail.max(120.0))
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                let msgs = self.tabs[ti].ui.copilot.msgs.clone();
                if msgs.is_empty() {
                    ui.label(
                        RichText::new(tr("Ask a question about this document, or use a suggestion above. Select text and right-click ▸ Explain for a quick explanation."))
                            .color(pal.text_dim),
                    );
                }
                for (mi, m) in msgs.iter().enumerate() {
                    match m.kind {
                        ChatKind::User => {
                            egui::Frame::new()
                                .fill(pal.accent_soft)
                                .corner_radius(8.0)
                                .inner_margin(8.0)
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.label(&m.text);
                                });
                        }
                        ChatKind::Assistant => {
                            ui.add(egui::Label::new(&m.text).selectable(true));
                            if !m.points.is_empty() {
                                ui.add_space(4.0);
                                ui.label(RichText::new(tr("Passages")).size(11.0).color(pal.text_dim));
                                for (pi, p) in m.points.iter().enumerate() {
                                    egui::Frame::new()
                                        .stroke(egui::Stroke::new(1.0, pal.border))
                                        .corner_radius(6.0)
                                        .inner_margin(6.0)
                                        .show(ui, |ui| {
                                            ui.set_width(ui.available_width());
                                            ui.horizontal(|ui| {
                                                if p.page > 0
                                                    && ui.small_button(format!("p. {}", p.page)).on_hover_text(tr("Go to this page")).clicked()
                                                {
                                                    jump = Some(p.page - 1);
                                                }
                                                if p.verified {
                                                    let label = if p.applied { tr("Highlighted") } else { tr("Highlight") };
                                                    if ui.add_enabled(!p.applied, egui::Button::new(label).small()).clicked() {
                                                        hl = Some((mi, pi));
                                                    }
                                                } else {
                                                    ui.label(RichText::new(tr("not found in the document")).size(11.0).color(pal.danger))
                                                        .on_hover_text(tr("The assistant quoted text that is not on the page, so it cannot be highlighted. Treat this point with care."));
                                                }
                                            });
                                            ui.label(RichText::new(format!("“{}”", p.quote)).italics().size(12.0));
                                            if !p.note.is_empty() {
                                                ui.label(RichText::new(&p.note).size(11.0).color(pal.text_dim));
                                            }
                                        });
                                }
                                if m.points.iter().filter(|p| p.verified && !p.applied).count() > 1
                                    && ui.small_button(tr("Highlight all passages")).clicked()
                                {
                                    hl_all = Some(mi);
                                }
                            }
                        }
                        ChatKind::Error => {
                            ui.colored_label(pal.danger, &m.text);
                        }
                        ChatKind::Info => {
                            ui.label(RichText::new(&m.text).color(pal.text_dim));
                        }
                    }
                    ui.add_space(8.0);
                }
                if busy {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        let label = self.ai.job.as_ref().map(|j| format!("{} ({}s)", j.label, j.started.elapsed().as_secs())).unwrap_or_default();
                        ui.label(RichText::new(ai_client::text::clip_chars(&label, 60)).color(pal.text_dim));
                        if ui.small_button(tr("Cancel")).clicked() {
                            self.cancel_ai();
                        }
                    });
                }
                if stick {
                    ui.scroll_to_cursor(Some(Align::BOTTOM));
                }
            });
        self.tabs[ti].ui.copilot.stick_bottom = false;
        // Input.
        ui.add_space(2.0);
        let mut send = false;
        {
            let input = &mut self.tabs[ti].ui.copilot.input;
            let resp = ui.add(
                crate::ui_kit::multiline(input)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .hint_text(tr("Ask PDF Copilot…")),
            );
            if resp.has_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift)
            {
                // Enter sends, Shift+Enter adds a line (the newline already typed is removed).
                while input.ends_with('\n') {
                    input.pop();
                }
                send = !input.trim().is_empty();
            }
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !busy && !self.tabs[ti].ui.copilot.input.trim().is_empty(),
                    egui::Button::new(tr("Send")),
                )
                .clicked()
            {
                send = true;
            }
            ui.label(
                RichText::new(tf!("Sent to {} only when you press Send", host))
                    .size(10.0)
                    .color(pal.text_dim),
            );
        });
        if send && !busy {
            let q = std::mem::take(&mut self.tabs[ti].ui.copilot.input);
            action = Some(AiAction {
                shown: q.trim().to_string(),
                mode: Mode::Ask,
                question: q.trim().to_string(),
                around_page: None,
            });
        }
        // Apply what was clicked.
        if let Some(p) = jump {
            self.go_to(p);
        }
        if let Some((mi, pi)) = hl {
            self.highlight_point(mi, pi);
        }
        if let Some(mi) = hl_all {
            let n = self.tabs[ti]
                .ui
                .copilot
                .msgs
                .get(mi)
                .map_or(0, |m| m.points.len());
            let mut done = 0;
            for pi in 0..n {
                let ok = self.tabs[ti]
                    .ui
                    .copilot
                    .msgs
                    .get(mi)
                    .and_then(|m| m.points.get(pi))
                    .is_some_and(|p| p.verified && !p.applied);
                if ok && self.highlight_point(mi, pi) {
                    done += 1;
                }
            }
            if done > 0 {
                self.notify(tf!("Highlighted {} passage(s).", done));
            }
        }
        if let Some(a) = action {
            self.ai_ask(ctx, a);
        }
        if translate && self.ai_consent_or_ask(AiPending::OpenTranslate) {
            self.open_translate_dialog(ctx);
        }
    }
}

// ---- preferences -----------------------------------------------------------------------------

impl App {
    /// The "PDF Copilot" block of the Preferences dialog. Returns whether a preference changed.
    pub fn ai_prefs_section(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) -> bool {
        let pal = self.pal;
        let mut changed = false;
        let previous_provider = self.prefs.ai.provider;
        ui.horizontal(|ui| {
            ui.label(tr("Provider"));
            for p in [
                AiProvider::OpenAi,
                AiProvider::Anthropic,
                AiProvider::Custom,
            ] {
                changed |= ui
                    .selectable_value(&mut self.prefs.ai.provider, p, tr(p.title()))
                    .changed();
            }
        });
        let provider = self.prefs.ai.provider;
        if provider != previous_provider {
            // A model id of one provider means nothing to another.
            self.prefs.ai.model.clear();
            self.ai_custom_model = false;
        }
        egui::Grid::new("ai_grid")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label(tr("Model"));
                ui.vertical(|ui| {
                    let models = provider.models();
                    if !models.is_empty() {
                        let current = self.prefs.ai.model().to_string();
                        let known = models.iter().any(|(id, _)| *id == current);
                        let shown = if self.ai_custom_model || !known {
                            tr("Other…").to_string()
                        } else {
                            let d = models
                                .iter()
                                .find(|(id, _)| *id == current)
                                .map_or("", |m| m.1);
                            let tag = if self.prefs.ai.model.trim().is_empty() {
                                tr(" (default)")
                            } else {
                                ""
                            };
                            format!("{current} — {d}{tag}")
                        };
                        egui::ComboBox::from_id_salt("ai_model")
                            .selected_text(shown)
                            .width(300.0)
                            .show_ui(ui, |ui| {
                                for (id, desc) in models {
                                    let label = if *id == provider.default_model() {
                                        tf!("{} — {} (default)", id, desc)
                                    } else {
                                        format!("{id} — {desc}")
                                    };
                                    if ui
                                        .selectable_label(
                                            known && !self.ai_custom_model && current == *id,
                                            label,
                                        )
                                        .clicked()
                                    {
                                        // The default is stored as "empty" so it follows future default changes.
                                        self.prefs.ai.model = if *id == provider.default_model() {
                                            String::new()
                                        } else {
                                            (*id).to_string()
                                        };
                                        self.ai_custom_model = false;
                                        changed = true;
                                    }
                                }
                                if ui
                                    .selectable_label(self.ai_custom_model || !known, tr("Other…"))
                                    .clicked()
                                {
                                    self.ai_custom_model = true;
                                }
                            });
                    }
                    if models.is_empty()
                        || self.ai_custom_model
                        || !models.iter().any(|(id, _)| *id == self.prefs.ai.model())
                    {
                        changed |= ui
                            .add(
                                crate::ui_kit::singleline(&mut self.prefs.ai.model)
                                    .hint_text(provider.default_model())
                                    .desired_width(300.0),
                            )
                            .changed();
                    }
                });
                ui.end_row();
                ui.label(tr("Server address"));
                changed |= ui
                    .add(
                        crate::ui_kit::singleline(&mut self.prefs.ai.base_url)
                            .hint_text(provider.default_base_url())
                            .desired_width(260.0),
                    )
                    .changed();
                ui.end_row();
                ui.label(tr("Answer language"));
                changed |= ui
                    .add(
                        crate::ui_kit::singleline(&mut self.prefs.ai.answer_language)
                            .hint_text(tr("same as the question"))
                            .desired_width(260.0),
                    )
                    .changed();
                ui.end_row();
                ui.label(tr("Translate into"));
                changed |= ui
                    .add(
                        crate::ui_kit::singleline(&mut self.prefs.ai.translate_to)
                            .hint_text("English")
                            .desired_width(260.0),
                    )
                    .changed();
                ui.end_row();
                ui.label(tr("Text sent per question"));
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut self.prefs.ai.max_chars)
                            .range(5_000..=400_000)
                            .speed(1000.0)
                            .suffix(tr(" characters")),
                    )
                    .changed();
                ui.end_row();
            });
        ui.add_space(6.0);
        ui.label(RichText::new(tr("API key")).strong());
        match platform::secrets::key_source() {
            platform::secrets::KeySource::Environment => {
                ui.label(tr(
                    "Using the key from the BERGPDF_AI_KEY environment variable.",
                ));
            }
            platform::secrets::KeySource::File => {
                ui.label(tf!(
                    "A key is saved on this computer: {}",
                    platform::secrets::key_path().display()
                ));
                if ui.button(tr("Remove saved key")).clicked() {
                    match platform::secrets::delete_ai_key() {
                        Ok(()) => {
                            self.ai_key_changed();
                            self.ai.test_result = None;
                            self.notify(tr("Saved key removed."));
                        }
                        Err(e) => self.notify_error(tf!("Could not remove the key: {}", e)),
                    }
                }
            }
            platform::secrets::KeySource::None => {
                ui.label(RichText::new(tr("No key saved yet.")).color(pal.text_dim));
            }
        }
        ui.horizontal(|ui| {
            ui.add(
                crate::ui_kit::singleline(&mut self.ai.key_input)
                    .password(!self.ai.key_show)
                    .hint_text(tr("Paste your API key"))
                    .desired_width(260.0),
            );
            ui.checkbox(&mut self.ai.key_show, tr("Show"));
            let can = !self.ai.key_input.trim().is_empty();
            if ui
                .add_enabled(can, egui::Button::new(tr("Save key")))
                .clicked()
            {
                match platform::secrets::save_ai_key(&self.ai.key_input) {
                    Ok(()) => {
                        self.ai.key_input.clear();
                        self.ai_key_changed();
                        self.ai.test_result = None;
                        self.notify(tr("API key saved on this computer."));
                    }
                    Err(e) => self.notify_error(tf!("Could not save the key: {}", e)),
                }
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.ai_busy(), egui::Button::new(tr("Test connection")))
                .on_hover_text(tr(
                    "Sends one tiny message (no document text) to check the key and address",
                ))
                .clicked()
            {
                self.ai_test_connection(ctx);
            }
            if self.ai_busy() && self.ai.test_result.is_none() {
                ui.spinner();
            }
            match &self.ai.test_result {
                Some(Ok(m)) => {
                    ui.label(RichText::new(m).color(egui::Color32::from_rgb(0x2E, 0x9E, 0x5B)));
                }
                Some(Err(e)) => {
                    ui.colored_label(pal.danger, e);
                }
                None => {}
            }
        });
        ui.add_space(4.0);
        ui.label(
            RichText::new(
                tr("The key is stored unencrypted in your user profile (readable only by your account on macOS/Linux). Anyone who can read your files can use it, so give it a spending limit at your provider. \
Text from your documents is sent to the provider only when you press a Copilot or Translate button, and you are asked to agree the first time. With the Custom provider you can point BergPDF at a server on your own computer so nothing leaves it."),
            )
            .size(11.0)
            .color(pal.text_dim),
        );
        if self.prefs.ai_consent_provider.is_some()
            && ui
                .small_button(tr("Forget my consent (ask again)"))
                .clicked()
        {
            self.prefs.ai_consent_provider = None;
            changed = true;
        }
        changed
    }
}
