//! Translate a document: detect its language on this computer, translate paragraph by paragraph
//! through the configured AI provider, and put the translation *in* the document: each paragraph
//! is replaced where it stands (the original text is removed or covered, the translation is drawn
//! in the same box, shrunk if it needs more room). The result opens as a copy in a new tab; the
//! original document is never changed.

use crate::copilot_ui::host_of;
use crate::dialogs::modal;
use crate::state::*;
use ai_client::PageText;
use ai_client::translate::{Detected, detect, translate_blocks};
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

/// A page index, its paragraphs and the background colour behind each.
type PageBlocks = (
    usize,
    Vec<pdf_engine::inlinetr::Block>,
    Vec<(f32, f32, f32)>,
);

/// The finished translation: the translated copy, a summary and the text per page.
struct Outcome {
    bytes: Arc<Vec<u8>>,
    pages: Vec<PageText>,
    notes: Vec<String>,
}

enum Msg {
    Prepared(Result<Prepared, String>),
    Done(Result<Outcome, String>),
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
    total: Arc<AtomicUsize>,
    pub target: String,
    current_only: bool,
    current_page: usize,
    detected: Option<Detected>,
    pages: Option<Arc<Vec<PageText>>>,
    result: Vec<PageText>,
    copy: Option<Arc<Vec<u8>>>,
    notes: Vec<String>,
    geoms: Vec<(usize, pdf_engine::geom::PageGeometry)>,
    snapshot: Option<Arc<Vec<u8>>>,
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
        let geoms2 = geoms.clone();
        let snap_bytes = snap.bytes.clone();
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
            total: Arc::new(AtomicUsize::new(0)),
            target,
            current_only: false,
            current_page: current,
            detected: None,
            pages: None,
            result: Vec::new(),
            copy: None,
            notes: Vec::new(),
            geoms: geoms2,
            snapshot: Some(snap_bytes),
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
                        Ok(o) => {
                            st.result = o.pages;
                            st.copy = Some(o.bytes);
                            st.notes = o.notes;
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
        let mut open_copy = false;
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
                            "About {chars} characters will be sent to {host} (model {model}) with your API key. The usage is billed by your provider. The translation replaces the text in a copy of the document that opens in a new tab; your document is not changed."
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
                    let total = st.total.load(Ordering::Relaxed);
                    if total == 0 {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Finding the paragraphs on the pages…");
                        });
                    } else {
                        ui.add(
                            egui::ProgressBar::new(done as f32 / total.max(1) as f32)
                                .text(format!("Paragraph {} of {}", done.min(total), total)),
                        );
                        ui.label(
                            RichText::new("Waiting for the AI service…")
                                .size(12.0)
                                .color(dim),
                        );
                    }
                    if ui.button("Cancel").clicked() {
                        st.cancel.store(true, Ordering::Relaxed);
                        close = true;
                    }
                }
                Stage::Done => {
                    ui.label(format!(
                        "Translated {} page(s) into {}, in place in the document.",
                        st.result.len(),
                        st.used_target
                    ));
                    for n in &st.notes {
                        ui.label(RichText::new(n.as_str()).size(12.0).color(dim));
                    }
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
                        if ui.button("Open translated copy").clicked() {
                            open_copy = true;
                        }
                        if ui.button("Save translated PDF…").clicked() {
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
                        RichText::new("The copy keeps the page layout; paragraphs are re-flowed in the same box. Tables, headers and text inside pictures are not handled specially. Your original is unchanged.")
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
        if open_copy {
            self.open_translated_copy(st);
            close = true;
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
        let Some(bytes) = st.snapshot.clone() else {
            st.error = Some("The document is no longer available.".into());
            st.stage = Stage::Failed;
            return;
        };
        let page_texts = st.pages.clone().unwrap_or_default();
        let wanted: Vec<usize> = st
            .geoms
            .iter()
            .map(|(i, _)| *i)
            .filter(|i| !st.current_only || *i == st.current_page)
            .filter(|i| {
                page_texts
                    .iter()
                    .find(|p| p.number == i + 1)
                    .is_some_and(|p| !p.text.trim().is_empty())
            })
            .collect();
        let geoms = st.geoms.clone();
        st.progress = Arc::new(AtomicUsize::new(0));
        st.total = Arc::new(AtomicUsize::new(0));
        st.cancel = Arc::new(AtomicBool::new(false));
        st.used_target = st.target.trim().to_string();
        st.source_name = st.detected.as_ref().map(|d| d.name.clone());
        let (progress, total, cancel, target, source) = (
            st.progress.clone(),
            st.total.clone(),
            st.cancel.clone(),
            st.used_target.clone(),
            st.source_name.clone(),
        );
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let r = catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_inline_translation(
                    &cfg,
                    bytes,
                    &geoms,
                    &wanted,
                    source.as_deref(),
                    &target,
                    &cancel,
                    &progress,
                    &total,
                )
            }));
            let _ = tx.send(Msg::Done(r.unwrap_or_else(|_| {
                Err("The translation stopped unexpectedly.".into())
            })));
        });
        st.rx = Some(rx);
        st.stage = Stage::Running;
    }

    fn open_translated_copy(&mut self, st: &TranslateState) {
        let Some(bytes) = &st.copy else { return };
        let title = format!(
            "{} ({}).pdf",
            st.title.trim_end_matches(".pdf"),
            st.used_target
        );
        match editor_core::session::DocumentSession::open_bytes(bytes.as_ref().clone(), &title) {
            Ok(mut session) => {
                session.mark_unsaved();
                self.add_tab(session);
            }
            Err(e) => self.notify_error(format!("Could not open the translated copy: {e}")),
        }
    }

    fn save_translation_pdf(&mut self, st: &TranslateState) {
        let Some(bytes) = &st.copy else { return };
        let name = format!(
            "{}-{}.pdf",
            st.title.trim_end_matches(".pdf"),
            st.used_target.to_lowercase()
        );
        let Some(dest) = platform::dialogs::pick_save_pdf(&name, None) else {
            return;
        };
        match pdf_engine::save::write_atomic(
            &dest,
            bytes,
            &pdf_engine::save::SaveOptions::default(),
        ) {
            Ok(_) => self.notify(format!("Saved {}", dest.display())),
            Err(e) => self.notify_error(format!("Could not save: {e}")),
        }
    }
}

/// Everything the worker thread does: find paragraphs, translate them, write the copy.
#[allow(clippy::too_many_arguments)]
fn run_inline_translation(
    cfg: &ai_client::Config,
    bytes: Arc<Vec<u8>>,
    geoms: &[(usize, pdf_engine::geom::PageGeometry)],
    wanted: &[usize],
    source: Option<&str>,
    target: &str,
    cancel: &AtomicBool,
    progress: &AtomicUsize,
    total: &AtomicUsize,
) -> Result<Outcome, String> {
    use pdf_engine::inlinetr::{self, Item};
    // 1. Paragraphs and their background colours.
    let mut per_page: Vec<PageBlocks> = Vec::new();
    pdf_engine::render::with_session(bytes.clone(), |s| {
        for (i, g) in geoms.iter().filter(|(i, _)| wanted.contains(i)) {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let tp = s.extract_text(*i, g)?;
            let blocks = inlinetr::blocks_of(&tp);
            if blocks.is_empty() {
                continue;
            }
            let bmp = s.render_page(*i, *g, pdf_engine::geom::Rotation::R0, 1.0)?;
            let bg = blocks
                .iter()
                .map(|b| inlinetr::sample_background(&bmp, g, 1.0, b.rect))
                .collect();
            per_page.push((*i, blocks, bg));
        }
        Ok(())
    })
    .map_err(|e| e.to_string())?;
    if cancel.load(Ordering::Relaxed) {
        return Err("Cancelled.".into());
    }
    let texts: Vec<String> = per_page
        .iter()
        .flat_map(|(_, b, _)| b.iter().map(|b| b.text.clone()))
        .collect();
    if texts.is_empty() {
        return Err("No translatable text was found on the selected pages.".into());
    }
    total.store(texts.len(), Ordering::Relaxed);
    // 2. Translate.
    let translated = translate_blocks(cfg, &texts, source, target, cancel, &|d, _| {
        progress.store(d, Ordering::Relaxed);
    })
    .map_err(|e| e.to_string())?;
    // 3. Put the translations into a copy of the document.
    let mut doc = pdf_engine::doc::PdfDocument::open(
        bytes.as_ref().clone(),
        &pdf_engine::doc::OpenOptions::default(),
    )
    .map_err(|e| e.to_string())?;
    let page_ids = doc.page_ids().map_err(|e| e.to_string())?;
    let mut it = translated.into_iter();
    let (mut blocks_n, mut removed, mut shrunk, mut overflow) = (0, 0, 0, 0);
    let mut missing = std::collections::BTreeSet::new();
    let mut fonts = std::collections::BTreeSet::new();
    let mut pages_out = Vec::new();
    for (i, blocks, bgs) in per_page {
        let Some(page) = page_ids.get(i).copied() else {
            continue;
        };
        let items: Vec<Item> = blocks
            .into_iter()
            .zip(bgs)
            .filter_map(|(block, background)| {
                Some(Item {
                    translation: it.next()?,
                    background,
                    block,
                })
            })
            .collect();
        pages_out.push(PageText {
            number: i + 1,
            text: items
                .iter()
                .map(|x| x.translation.as_str())
                .collect::<Vec<_>>()
                .join("\n\n"),
        });
        let (rep, _) = doc
            .transact(|tx| inlinetr::apply_translation(tx, page, &items))
            .map_err(|e| e.to_string())?;
        blocks_n += rep.blocks;
        removed += rep.runs_removed;
        shrunk += rep.shrunk;
        overflow += rep.overflowing;
        missing.extend(rep.missing_chars);
        fonts.extend(rep.system_fonts);
    }
    let out = doc.snapshot_bytes().map_err(|e| e.to_string())?;
    let mut notes = vec![format!(
        "{blocks_n} paragraph(s) replaced; {removed} original text piece(s) removed from the page content, the rest covered."
    )];
    if shrunk > 0 {
        notes.push(format!(
            "{shrunk} paragraph(s) use a smaller font so the translation fits."
        ));
    }
    if overflow > 0 {
        notes.push(format!(
            "{overflow} paragraph(s) are still longer than their original box; check them."
        ));
    }
    if !fonts.is_empty() {
        notes.push(format!(
            "Installed font(s) used for characters the built-in fonts lack: {}.",
            fonts.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    if !missing.is_empty() {
        let s: String = missing.iter().take(10).collect();
        notes.push(format!(
            "No available font has {s}: shown as “?”. Save as text to keep them."
        ));
    }
    Ok(Outcome {
        bytes: Arc::new(out),
        pages: pages_out,
        notes,
    })
}

fn joined(p: &[PageText]) -> String {
    p.iter()
        .map(|p| format!("--- page {} ---\n{}", p.number, p.text.trim()))
        .collect::<Vec<_>>()
        .join("\n\n")
}
