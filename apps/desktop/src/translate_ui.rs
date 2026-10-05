//! Translate a document: detect its language on this computer, translate page by page through
//! the configured AI provider, then save the result as a PDF or text file.
//!
//! The translation is a new document (text only, in the built-in font). The original is never
//! changed and its layout is not reproduced.

use crate::copilot_ui::host_of;
use crate::dialogs::modal;
use crate::state::*;
use ai_client::PageText;
use ai_client::translate::{Detected, detect, translate_pages};
use egui::RichText;
use std::panic::catch_unwind;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

const LANGUAGES: &[&str] = &[
    "English",
    "Dutch",
    "German",
    "French",
    "Spanish",
    "Italian",
    "Portuguese",
    "Polish",
    "Swedish",
    "Danish",
    "Norwegian",
    "Finnish",
    "Czech",
    "Turkish",
    "Russian",
    "Ukrainian",
    "Greek",
    "Romanian",
    "Hungarian",
    "Arabic",
    "Hebrew",
    "Chinese",
    "Japanese",
    "Korean",
    "Hindi",
];

/// Page texts, the revision they belong to, and the detected language.
type Prepared = (Arc<Vec<PageText>>, u64, Option<Detected>);

enum Msg {
    Prepared(Result<Prepared, String>),
    Done(Result<Vec<PageText>, String>),
}

#[derive(PartialEq, Eq)]
enum Stage {
    Preparing,
    Setup,
    Running,
    Done,
    Failed,
}

/// State of the Translate dialog.
pub struct TranslateState {
    stage: Stage,
    rx: Option<Receiver<Msg>>,
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicUsize>,
    total: usize,
    pub target: String,
    current_only: bool,
    current_page: usize,
    detected: Option<Detected>,
    pages: Option<Arc<Vec<PageText>>>,
    result: Vec<PageText>,
    error: Option<String>,
    title: String,
    source_name: Option<String>,
    used_target: String,
}

impl App {
    pub fn open_translate_dialog(&mut self, ctx: &egui::Context) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let current = tab.session.view.current_page;
        let title = tab.session.title.clone();
        let (Ok(pages), Ok(snap)) = (tab.session.pages(), tab.session.snapshot()) else {
            self.notify_error("Could not prepare the document for translation.");
            return;
        };
        let doc = tab.session.id;
        let rev = snap.revision;
        let geoms: Vec<_> = pages
            .iter()
            .enumerate()
            .map(|(i, p)| (i, p.geometry))
            .collect();
        let cached = self.ai_cached_pages(doc, rev);
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        std::thread::spawn(move || {
            let r = catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<_, String> {
                let pages = match cached {
                    Some(p) => p,
                    None => Arc::new(crate::copilot_ui::extract_all(snap.bytes, &geoms, &c2)?),
                };
                let sample: String = pages
                    .iter()
                    .map(|p| p.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                let d = detect(&sample);
                Ok((pages, rev, d))
            }));
            let _ = tx.send(Msg::Prepared(r.unwrap_or_else(|_| {
                Err("Reading the document text failed unexpectedly.".into())
            })));
        });
        let target = self.prefs.ai.translate_to.clone();
        self.dialog = Some(Dialog::Translate(Box::new(TranslateState {
            stage: Stage::Preparing,
            rx: Some(rx),
            cancel,
            progress: Arc::new(AtomicUsize::new(0)),
            total: 0,
            target,
            current_only: false,
            current_page: current,
            detected: None,
            pages: None,
            result: Vec::new(),
            error: None,
            title,
            source_name: None,
            used_target: String::new(),
        })));
        let _ = doc;
        ctx.request_repaint();
    }

    pub fn dialog_translate(&mut self, ctx: &egui::Context, st: &mut TranslateState) -> bool {
        // Collect worker messages.
        if let Some(rx) = &st.rx {
            match rx.try_recv() {
                Ok(Msg::Prepared(r)) => {
                    st.rx = None;
                    match r {
                        Ok((pages, rev, d)) => {
                            if let Some(t) = self.tabs.get(self.active) {
                                self.ai_store_pages(t.session.id, rev, pages.clone());
                            }
                            st.detected = d;
                            st.pages = Some(pages);
                            st.stage = Stage::Setup;
                        }
                        Err(e) => {
                            st.error = Some(e);
                            st.stage = Stage::Failed;
                        }
                    }
                }
                Ok(Msg::Done(r)) => {
                    st.rx = None;
                    match r {
                        Ok(p) => {
                            st.result = p;
                            st.stage = Stage::Done;
                        }
                        Err(e) => {
                            st.error = Some(e);
                            st.stage = Stage::Failed;
                        }
                    }
                }
                Err(TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100))
                }
                Err(TryRecvError::Disconnected) => {
                    st.rx = None;
                    st.error = Some("The translation stopped unexpectedly.".into());
                    st.stage = Stage::Failed;
                }
            }
        }
        let dim = self.pal.text_dim;
        let danger = self.pal.danger;
        let host = host_of(&self.prefs.ai.base_url());
        let model = self.prefs.ai.model().to_string();
        let mut close = false;
        let mut start = false;
        let mut save_pdf = false;
        let mut save_txt = false;
        let mut copy = false;
        let mut retry = false;
        modal(ctx, "translate", |ui| {
            ui.set_max_width(560.0);
            ui.heading("Translate document");
            match st.stage {
                Stage::Preparing => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Reading the text and detecting the language…");
                    });
                }
                Stage::Setup => {
                    let pages = st.pages.clone().unwrap_or_default();
                    let with_text = pages.iter().filter(|p| !p.text.trim().is_empty()).count();
                    match &st.detected {
                        Some(d) => {
                            ui.label(format!(
                                "Detected language: {} ({:.0} % sure{})",
                                d.name,
                                d.confidence * 100.0,
                                if d.reliable { "" } else { ", low confidence" }
                            ));
                            ui.label(
                                RichText::new("Detected on this computer; nothing was sent.")
                                    .size(11.0)
                                    .color(dim),
                            );
                        }
                        None => {
                            ui.label("The language could not be detected (too little text). The translator will work it out.");
                        }
                    }
                    if with_text == 0 {
                        ui.add_space(4.0);
                        ui.colored_label(danger, "This document has no text to translate. If it is a scan, run OCR first (Edit ▸ Recognize Text).");
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label("Translate into");
                        egui::ComboBox::from_id_salt("tr_lang")
                            .selected_text(st.target.clone())
                            .width(150.0)
                            .show_ui(ui, |ui| {
                                for l in LANGUAGES {
                                    ui.selectable_value(&mut st.target, (*l).to_string(), *l);
                                }
                            });
                        ui.add(
                            egui::TextEdit::singleline(&mut st.target)
                                .desired_width(120.0)
                                .hint_text("or type one"),
                        );
                    });
                    if let Some(d) = &st.detected
                        && d.name.eq_ignore_ascii_case(st.target.trim())
                    {
                        ui.colored_label(
                            danger,
                            format!("The document already seems to be in {}.", d.name),
                        );
                    }
                    ui.horizontal(|ui| {
                        ui.radio_value(
                            &mut st.current_only,
                            false,
                            format!("All pages ({with_text} with text)"),
                        );
                        ui.radio_value(
                            &mut st.current_only,
                            true,
                            format!("Current page ({})", st.current_page + 1),
                        );
                    });
                    let chars: usize = pages
                        .iter()
                        .filter(|p| !st.current_only || p.number == st.current_page + 1)
                        .map(|p| p.text.chars().count())
                        .sum();
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!(
                            "About {chars} characters will be sent to {host} (model {model}) with your API key. The usage is billed by your provider. The translation is saved as a new file; your document is not changed."
                        ))
                        .size(12.0)
                        .color(dim),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                with_text > 0 && !st.target.trim().is_empty(),
                                egui::Button::new("Translate"),
                            )
                            .clicked()
                        {
                            start = true;
                        }
                        if ui.button("Cancel").clicked()
                            || ui.input(|i| i.key_pressed(egui::Key::Escape))
                        {
                            close = true;
                        }
                    });
                }
                Stage::Running => {
                    let done = st.progress.load(Ordering::Relaxed);
                    ui.add(
                        egui::ProgressBar::new(done as f32 / st.total.max(1) as f32).text(format!(
                            "Page {} of {}",
                            done.min(st.total),
                            st.total
                        )),
                    );
                    ui.label(
                        RichText::new("Waiting for the AI service…")
                            .size(12.0)
                            .color(dim),
                    );
                    if ui.button("Cancel").clicked() {
                        st.cancel.store(true, Ordering::Relaxed);
                        close = true;
                    }
                }
                Stage::Done => {
                    ui.label(format!(
                        "Translated {} page(s) into {}.",
                        st.result.len(),
                        st.used_target
                    ));
                    ui.label(
                        RichText::new(
                            "Machine translation: check important passages against the original.",
                        )
                        .size(12.0)
                        .color(dim),
                    );
                    ui.add_space(4.0);
                    let mut preview: String = st
                        .result
                        .iter()
                        .map(|p| format!("— page {} —\n{}", p.number, p.text.trim()))
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    preview = ai_client::text::clip_chars(&preview, 6000).to_string();
                    egui::ScrollArea::vertical()
                        .max_height(260.0)
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut preview)
                                    .desired_width(f32::INFINITY)
                                    .interactive(false),
                            );
                        });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("Save as PDF…").clicked() {
                            save_pdf = true;
                        }
                        if ui.button("Save as text…").clicked() {
                            save_txt = true;
                        }
                        if ui.button("Copy").clicked() {
                            copy = true;
                        }
                        if ui.button("Close").clicked()
                            || ui.input(|i| i.key_pressed(egui::Key::Escape))
                        {
                            close = true;
                        }
                    });
                    ui.label(
                        RichText::new("The PDF is plain text in the built-in font (Latin, Greek and Cyrillic); the original layout is not reproduced.")
                            .size(11.0)
                            .color(dim),
                    );
                }
                Stage::Failed => {
                    ui.colored_label(danger, st.error.clone().unwrap_or_default());
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if st.pages.is_some() && ui.button("Back").clicked() {
                            retry = true;
                        }
                        if ui.button("Close").clicked()
                            || ui.input(|i| i.key_pressed(egui::Key::Escape))
                        {
                            close = true;
                        }
                    });
                }
            }
        });
        if retry {
            st.stage = Stage::Setup;
            st.error = None;
        }
        if start {
            self.start_translation(st);
        }
        if copy {
            let text = joined(&st.result);
            ctx.copy_text(text);
            self.notify("Translation copied.");
        }
        if save_txt {
            let name = format!(
                "{}-{}.txt",
                st.title.trim_end_matches(".pdf"),
                st.used_target.to_lowercase()
            );
            if let Some(dest) = platform::dialogs::pick_save_text(&name) {
                match std::fs::write(&dest, joined(&st.result)) {
                    Ok(()) => self.notify(format!("Saved {}", dest.display())),
                    Err(e) => self.notify_error(format!("Could not save: {e}")),
                }
            }
        }
        if save_pdf {
            self.save_translation_pdf(st);
        }
        !close
    }

    fn start_translation(&mut self, st: &mut TranslateState) {
        let cfg = match self.ai_config() {
            Ok(c) => c,
            Err(e) => {
                st.error = Some(e.to_string());
                st.stage = Stage::Failed;
                return;
            }
        };
        let pages: Vec<PageText> = st
            .pages
            .as_ref()
            .map(|p| {
                p.iter()
                    .filter(|p| !p.text.trim().is_empty())
                    .filter(|p| !st.current_only || p.number == st.current_page + 1)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        st.total = pages.len();
        st.progress = Arc::new(AtomicUsize::new(0));
        st.cancel = Arc::new(AtomicBool::new(false));
        st.used_target = st.target.trim().to_string();
        st.source_name = st.detected.as_ref().map(|d| d.name.clone());
        let (progress, cancel, target, source) = (
            st.progress.clone(),
            st.cancel.clone(),
            st.used_target.clone(),
            st.source_name.clone(),
        );
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let r = catch_unwind(std::panic::AssertUnwindSafe(|| {
                translate_pages(
                    &cfg,
                    &pages,
                    source.as_deref(),
                    &target,
                    &cancel,
                    &|d, _| {
                        progress.store(d, Ordering::Relaxed);
                    },
                )
                .map_err(|e| e.to_string())
            }));
            let _ = tx.send(Msg::Done(r.unwrap_or_else(|_| {
                Err("The translation stopped unexpectedly.".into())
            })));
        });
        st.rx = Some(rx);
        st.stage = Stage::Running;
    }

    fn save_translation_pdf(&mut self, st: &TranslateState) {
        let name = format!(
            "{}-{}.pdf",
            st.title.trim_end_matches(".pdf"),
            st.used_target.to_lowercase()
        );
        let Some(dest) = platform::dialogs::pick_save_pdf(&name, None) else {
            return;
        };
        let sections: Vec<(String, String)> = st
            .result
            .iter()
            .map(|p| (format!("Page {}", p.number), p.text.clone()))
            .collect();
        let note = format!(
            "{} · machine translation by {} through BergPDF Copilot. Check important passages against the original.",
            match &st.source_name {
                Some(s) => format!("{s} → {}", st.used_target),
                None => format!("→ {}", st.used_target),
            },
            self.prefs.ai.model()
        );
        let title = format!("Translation of {}", st.title);
        match pdf_engine::textdoc::build_text_pdf(&title, &note, &sections) {
            Ok((bytes, rep)) => {
                match pdf_engine::save::write_atomic(
                    &dest,
                    &bytes,
                    &pdf_engine::save::SaveOptions::default(),
                ) {
                    Ok(_) => {
                        let mut msg = format!("Saved {} ({} page(s))", dest.display(), rep.pages);
                        if !rep.replaced.is_empty() {
                            let s: String = rep.replaced.iter().take(8).collect();
                            msg.push_str(&format!(
                                ". {} character(s) such as {s} are not in the built-in font and were replaced by “?”; save as text to keep them",
                                rep.replaced.len()
                            ));
                        }
                        self.notify(msg);
                    }
                    Err(e) => self.notify_error(format!("Could not save: {e}")),
                }
            }
            Err(e) => self.notify_error(format!("Could not create the PDF: {e}")),
        }
    }
}

fn joined(p: &[PageText]) -> String {
    p.iter()
        .map(|p| format!("--- page {} ---\n{}", p.number, p.text.trim()))
        .collect::<Vec<_>>()
        .join("\n\n")
}
