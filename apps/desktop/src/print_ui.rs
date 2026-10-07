//! Printing: the Print dialog (printer, copies, pages, sizing) and the background job
//! (`printing` crate, docs/DECISIONS.md D-037).

use crate::dialogs::modal;
use crate::docops_ui::ExportJob;
use crate::i18n::tr;
use crate::state::*;
use crate::tf;
use printing::{PrintRequest, Printer, Scale, parse_page_ranges, printers};

/// Which pages the dialog prints.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pages {
    All,
    Current,
    Range,
}

/// The Print dialog.
pub struct PrintDialog {
    printers: Vec<Printer>,
    selected: usize,
    copies: u32,
    pages: Pages,
    range: String,
    scale: Scale,
    total: usize,
    current: usize,
    error: Option<String>,
}

impl App {
    /// Whether the active document may be printed (a password-protected one can forbid it).
    pub fn print_allowed(&self) -> bool {
        self.active_tab()
            .and_then(|t| t.session.protection())
            .is_none_or(|p| p.rights.print)
    }

    /// Print…
    pub fn open_print_dialog(&mut self) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        if !self.print_allowed() {
            self.notify_error(tr(
                "The author of this document did not allow printing. Enter the owner password (Protect) to lift this.",
            ));
            return;
        }
        let total = tab.session.doc().page_count();
        let current = tab.session.view.current_page;
        let list = printers();
        if list.is_empty() {
            self.notify_error(tr("No printer was found on this computer."));
            return;
        }
        self.dialog = Some(Dialog::Print(Box::new(PrintDialog {
            selected: list.iter().position(|p| p.is_default).unwrap_or(0),
            printers: list,
            copies: 1,
            pages: Pages::All,
            range: String::new(),
            scale: Scale::ShrinkToFit,
            total,
            current,
            error: None,
        })));
    }

    /// The Print dialog. Returns whether it stays open.
    pub fn dialog_print(&mut self, ctx: &egui::Context, st: &mut PrintDialog) -> bool {
        let mut go = false;
        let mut cancel = false;
        let danger = self.pal.danger;
        modal(ctx, "print_dialog", |ui| {
            ui.heading(tr("Print"));
            ui.add_space(4.0);
            egui::ComboBox::from_label(tr("Printer"))
                .selected_text(
                    st.printers
                        .get(st.selected)
                        .map_or(String::new(), |p| p.name.clone()),
                )
                .width(300.0)
                .show_ui(ui, |ui| {
                    for (i, p) in st.printers.iter().enumerate() {
                        ui.selectable_value(&mut st.selected, i, &p.name);
                    }
                });
            ui.horizontal(|ui| {
                ui.label(tr("Copies"));
                ui.add(egui::DragValue::new(&mut st.copies).range(1..=99));
            });
            ui.add_space(4.0);
            ui.radio_value(&mut st.pages, Pages::All, tf!("All pages ({})", st.total));
            ui.radio_value(
                &mut st.pages,
                Pages::Current,
                tf!("This page ({})", st.current + 1),
            );
            ui.horizontal(|ui| {
                ui.radio_value(&mut st.pages, Pages::Range, tr("Pages"));
                let r = ui.add(
                    egui::TextEdit::singleline(&mut st.range)
                        .hint_text("1-3, 5")
                        .desired_width(140.0),
                );
                if r.changed() {
                    st.pages = Pages::Range;
                }
            });
            ui.add_space(4.0);
            ui.radio_value(
                &mut st.scale,
                Scale::ShrinkToFit,
                tr("Shrink large pages to fit the paper"),
            );
            ui.radio_value(&mut st.scale, Scale::ActualSize, tr("Actual size"));
            if let Some(e) = &st.error {
                ui.colored_label(danger, e);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(tr("Print")).clicked() {
                    go = true;
                }
                if ui.button(tr("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    cancel = true;
                }
            });
        });
        if cancel {
            return false;
        }
        if go {
            match self.start_print(st) {
                Ok(()) => return false,
                Err(e) => st.error = Some(e),
            }
        }
        true
    }

    fn start_print(&mut self, st: &PrintDialog) -> Result<(), String> {
        let pages = match st.pages {
            Pages::All => (0..st.total).collect(),
            Pages::Current => vec![st.current.min(st.total.saturating_sub(1))],
            // The parser's messages are plain English; they are shown as they are.
            Pages::Range => parse_page_ranges(&st.range, st.total)?,
        };
        let printer = st
            .printers
            .get(st.selected)
            .ok_or_else(|| tr("Choose a printer.").to_string())?
            .name
            .clone();
        let tab = self
            .tabs
            .get_mut(self.active)
            .ok_or_else(|| tr("There is no document to print.").to_string())?;
        let title = tab.session.title.clone();
        let snap = tab
            .session
            .snapshot()
            .map_err(|e| tf!("Could not prepare the document: {}", e))?;
        let req = PrintRequest {
            printer: printer.clone(),
            title,
            copies: st.copies.clamp(1, 99),
            pages,
            scale: st.scale,
            output_file: None,
        };
        let n = req.pages.len() * req.copies as usize;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(|| {
                printing::print(snap.bytes.clone(), &req, &mut |_, _| {})
                    .map(|()| tf!("Sent {} page(s) to {}.", n, req.printer))
                    .map_err(|e| tf!("Printing failed: {}", e))
            });
            let _ = tx.send(r.unwrap_or_else(|_| Err(tr("Printing stopped unexpectedly.").into())));
        });
        self.exports.push(ExportJob { rx });
        self.notify(tf!("Printing {} page(s) on {}…", n, printer));
        Ok(())
    }
}
