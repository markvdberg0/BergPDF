//! "Is there a newer BergPDF?": the one-time question, the daily check, and the dialog for the manual check.
//!
//! Nothing is downloaded or installed: when a newer release exists the user gets its page (a link) and
//! installs it as they did the first time. See `ai_client::update` and docs/DECISIONS.md D-033.

use crate::i18n::tr;
use crate::state::*;
use crate::tf;
use ai_client::update::{LATEST_RELEASE_API, Release};
use editor_core::prefs::UpdateCheck;
use egui::RichText;
use std::sync::mpsc::{self, TryRecvError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

type Outcome = Result<Option<Release>, String>;

/// A running check and what the last one found.
#[derive(Default)]
pub struct UpdateRuntime {
    rx: Option<(bool, mpsc::Receiver<Outcome>)>,
    /// A newer published version, once one was found.
    pub available: Option<Release>,
    asked: bool,
    auto_started: bool,
}

/// What the "Check for updates" dialog shows.
pub enum UpdateState {
    Checking,
    UpToDate,
    Newer(Release),
    Failed(String),
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The address asked; `BERG_UPDATE_URL` replaces it (https or loopback only) for trying this out.
fn api_url() -> String {
    std::env::var("BERG_UPDATE_URL").unwrap_or_else(|_| LATEST_RELEASE_API.to_string())
}

impl App {
    /// Start a check in the background. `manual`: the user asked, so the answer is shown in a dialog.
    pub fn start_update_check(&mut self, manual: bool) {
        if self.update.rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let url = api_url();
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(|| {
                ai_client::update::check(&url, env!("CARGO_PKG_VERSION")).map_err(|e| e.to_string())
            });
            let _ =
                tx.send(r.unwrap_or_else(|_| Err(tr("The check stopped unexpectedly.").into())));
        });
        self.update.rx = Some((manual, rx));
        if !manual {
            self.prefs.last_update_check = now_secs();
            self.prefs_dirty = true;
        } else {
            self.dialog = Some(Dialog::Update(UpdateState::Checking));
        }
    }

    /// Per frame: ask once, run the daily check, and take in an answer.
    pub fn poll_update(&mut self, ctx: &egui::Context) {
        if let Some((manual, rx)) = &self.update.rx {
            let manual = *manual;
            let outcome = match rx.try_recv() {
                Ok(o) => Some(o),
                Err(TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(200));
                    None
                }
                Err(TryRecvError::Disconnected) => {
                    Some(Err(tr("The check stopped unexpectedly.").into()))
                }
            };
            if let Some(o) = outcome {
                self.update.rx = None;
                if let Ok(Some(r)) = &o {
                    self.update.available = Some(r.clone());
                }
                if manual {
                    let st = match o {
                        Ok(Some(r)) => UpdateState::Newer(r),
                        Ok(None) => UpdateState::UpToDate,
                        Err(e) => UpdateState::Failed(e),
                    };
                    if matches!(self.dialog, Some(Dialog::Update(_))) || self.dialog.is_none() {
                        self.dialog = Some(Dialog::Update(st));
                    }
                }
            }
        }
        // Not while another dialog (the welcome choice, a recovery offer…) is open.
        if self.dialog.is_some() || !self.prefs.first_run_done {
            return;
        }
        match self.prefs.update_check {
            UpdateCheck::Ask if !self.update.asked => {
                self.update.asked = true;
                self.dialog = Some(Dialog::UpdateAsk);
            }
            UpdateCheck::On
                if !self.update.auto_started
                    && now_secs().saturating_sub(self.prefs.last_update_check) > 24 * 3600 =>
            {
                self.update.auto_started = true;
                self.start_update_check(false);
            }
            _ => {}
        }
    }

    /// The one-time question. Returns whether it stays open.
    pub fn dialog_update_ask(&mut self, ctx: &egui::Context) -> bool {
        let mut choice: Option<UpdateCheck> = None;
        crate::dialogs::modal(ctx, "update_ask", |ui| {
            ui.heading(tr("Look for updates?"));
            ui.label(tr("BergPDF can look once a day whether a newer version has been published. For that it contacts github.com and sends only its name and version number. Nothing is downloaded or installed: you get a link to the release page. You can change this in Preferences ▸ Updates."));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button(tr("Check for updates")).clicked() {
                    choice = Some(UpdateCheck::On);
                }
                if ui.button(tr("Never")).clicked() {
                    choice = Some(UpdateCheck::Off);
                }
            });
        });
        match choice {
            Some(c) => {
                self.prefs.update_check = c;
                self.save_prefs();
                if c == UpdateCheck::On {
                    self.update.auto_started = true;
                    self.start_update_check(false);
                }
                false
            }
            None => true,
        }
    }

    /// The manual check's dialog. Returns whether it stays open.
    pub fn dialog_update(&mut self, ctx: &egui::Context, st: &UpdateState) -> bool {
        let mut close = false;
        let mut open_url: Option<String> = None;
        crate::dialogs::modal(ctx, "update_dialog", |ui| {
            ui.heading(tr("Check for Updates…"));
            ui.add_space(4.0);
            match st {
                UpdateState::Checking => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new());
                        ui.label(tr("Looking for a newer version…"));
                    });
                }
                UpdateState::UpToDate => {
                    ui.label(tf!(
                        "You have the latest version ({}).",
                        env!("CARGO_PKG_VERSION")
                    ));
                }
                UpdateState::Newer(r) => {
                    ui.label(RichText::new(tf!("Version {} is available.", r.version)).strong());
                    ui.label(tf!("You have version {}.", env!("CARGO_PKG_VERSION")));
                    ui.add_space(4.0);
                    ui.label(tr("Download the installer from the release page and run it over the current installation; your settings are kept."));
                    ui.add_space(8.0);
                    if ui.button(tr("Open release page")).clicked() {
                        open_url = Some(r.url.clone());
                    }
                }
                UpdateState::Failed(e) => {
                    ui.colored_label(self.pal.danger, tr("Could not check for updates."));
                    ui.label(e.as_str());
                }
            }
            ui.add_space(10.0);
            if ui.button(tr("Close")).clicked() {
                close = true;
            }
        });
        if let Some(u) = open_url
            && let Err(e) = platform::links::open_confirmed(&u)
        {
            self.notify_error(e);
        }
        !close
    }

    /// Open the page of the newer release (status-bar link).
    pub fn open_update_page(&mut self) {
        if let Some(r) = &self.update.available
            && let Err(e) = platform::links::open_confirmed(&r.url)
        {
            self.notify_error(e);
        }
    }
}
