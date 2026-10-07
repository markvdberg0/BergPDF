//! Pictures as signatures and the user's own stamps.
//!
//! A signature can be a scan or photo (PNG/JPEG) instead of a drawing; the paper is made transparent so it
//! sits on the page like ink. A stamp is a text with its own wording and colour, or a picture (a logo, a
//! company stamp). Both are kept on this computer (docs/DECISIONS.md D-038) and placed as stamp
//! annotations whose appearance is the picture, so they can be moved, resized, deleted and saved like any
//! other annotation.

use crate::canvas::ViewCtx;
use crate::dialogs::modal;
use crate::i18n::tr;
use crate::state::*;
use crate::tf;
use editor_core::stamps::{MAX_LABEL, MAX_NAME, StampDef, StampKind, StampLibrary};
use editor_core::tools::Tool;
use egui::{Color32, RichText, Sense, Stroke, Vec2};
use pdf_engine::annot::{AnnotationKind, AnnotationSpec, Rgb};
use pdf_engine::doc::PageId;
use pdf_engine::geom::{Point, Rect as PRect};
use pdf_engine::stampimage::StampImage;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Width of a placed picture stamp or signature in points when it is placed by a click.
const PLACED_WIDTH_PT: f64 = 140.0;
/// Largest picture file read, in bytes.
const MAX_FILE: u64 = 30 * 1024 * 1024;

fn stamps_dir() -> PathBuf {
    platform::dirs::config_dir().join("stamps")
}

fn stamps_file() -> PathBuf {
    platform::dirs::config_dir().join("stamps.toml")
}

pub(crate) fn signature_image_file() -> PathBuf {
    platform::dirs::config_dir().join("signature.png")
}

/// Write a file by writing a neighbour first, so a crash never leaves half a picture behind.
fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

fn read_picture_file(path: &Path) -> Result<Vec<u8>, String> {
    let len = std::fs::metadata(path)
        .map_err(|e| tf!("Could not read the picture: {}", e))?
        .len();
    if len > MAX_FILE {
        return Err(tr("That picture file is too large (over 30 MB).").into());
    }
    std::fs::read(path).map_err(|e| tf!("Could not read the picture: {}", e))
}

/// A picture being chosen: the file's bytes, how they are read and what came out.
pub struct PictureState {
    source: Vec<u8>,
    /// The white paper becomes transparent and the empty margins are cut away.
    ink_on_paper: bool,
    image: Option<Arc<StampImage>>,
    texture: Option<egui::TextureHandle>,
    error: Option<String>,
    /// The file's name without extension, a suggestion for the stamp's name.
    file_stem: String,
}

impl PictureState {
    fn load(path: &Path) -> Result<Self, String> {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self::from_bytes(read_picture_file(path)?, stem)
    }

    /// The picture as it will be kept.
    #[cfg(debug_assertions)]
    pub(crate) fn image(&self) -> Option<Arc<StampImage>> {
        self.image.clone()
    }

    /// From the bytes of a PNG or JPEG file.
    pub(crate) fn from_bytes(source: Vec<u8>, file_stem: String) -> Result<Self, String> {
        // A picture that already has transparency keeps it; one on paper gets its paper removed.
        let as_is = StampImage::from_bytes(&source, false).map_err(|e| e.to_string())?;
        let mut st = Self {
            ink_on_paper: as_is.alpha.is_none(),
            source,
            image: None,
            texture: None,
            error: None,
            file_stem,
        };
        st.reread();
        Ok(st)
    }

    fn reread(&mut self) {
        self.texture = None;
        match StampImage::from_bytes(&self.source, self.ink_on_paper) {
            Ok(i) => {
                self.image = Some(Arc::new(i));
                self.error = None;
            }
            Err(e) => {
                self.image = None;
                self.error = Some(e.to_string());
            }
        }
    }

    /// The picture on a white sheet, at most `max` big, plus the "paper" checkbox.
    fn show(&mut self, ui: &mut egui::Ui, max: Vec2, danger: Color32) {
        if let Some(img) = &self.image {
            let tex = self.texture.get_or_insert_with(|| {
                ui.ctx().load_texture(
                    "picture_preview",
                    egui::ColorImage::from_rgba_unmultiplied(
                        [img.width as usize, img.height as usize],
                        &img.to_rgba(),
                    ),
                    egui::TextureOptions::LINEAR,
                )
            });
            let scale = (max.x / img.width as f32)
                .min(max.y / img.height as f32)
                .min(4.0);
            let size = Vec2::new(img.width as f32, img.height as f32) * scale;
            let (rect, _) = ui.allocate_exact_size(Vec2::new(max.x, max.y), Sense::hover());
            ui.painter().rect_filled(rect, 4.0, Color32::WHITE);
            let target = egui::Rect::from_center_size(rect.center(), size);
            ui.painter().image(
                tex.id(),
                target,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        if let Some(e) = &self.error {
            ui.colored_label(danger, e);
        }
        if ui
            .checkbox(
                &mut self.ink_on_paper,
                tr("The picture is ink on white paper: make the paper transparent"),
            )
            .changed()
        {
            self.reread();
        }
    }
}

/// The "New stamp" dialog.
pub struct NewStampState {
    name: String,
    picture_mode: bool,
    label: String,
    color: [f32; 3],
    picture: Option<PictureState>,
    error: Option<String>,
}

impl NewStampState {
    /// A filled-in dialog for the debug screens.
    #[cfg(debug_assertions)]
    pub(crate) fn demo(picture: bool) -> Self {
        Self {
            name: "Betaald".into(),
            picture_mode: picture,
            label: "BETAALD".into(),
            ..Default::default()
        }
    }

    #[cfg(debug_assertions)]
    pub(crate) fn set_demo_picture(&mut self, png: Vec<u8>) {
        self.picture = PictureState::from_bytes(png, "scan".into()).ok();
    }
}

impl Default for NewStampState {
    fn default() -> Self {
        Self {
            name: String::new(),
            picture_mode: false,
            label: String::new(),
            color: [0.78, 0.1, 0.1],
            picture: None,
            error: None,
        }
    }
}

impl App {
    // ---- loading ---------------------------------------------------------------------------

    /// The saved stamps (startup).
    pub fn load_stamp_library() -> StampLibrary {
        platform::dirs::read_text(&stamps_file())
            .and_then(|t| StampLibrary::from_toml(&t))
            .unwrap_or_default()
    }

    /// The saved signature picture, if the signature was made from one (startup).
    pub fn load_signature_image() -> Option<Arc<StampImage>> {
        let bytes = std::fs::read(signature_image_file()).ok()?;
        StampImage::from_bytes(&bytes, false).ok().map(Arc::new)
    }

    /// The picture of a stamp, read from disk the first time.
    pub fn stamp_image(&mut self, file: &str) -> Option<Arc<StampImage>> {
        if let Some(i) = self.stamp_images.get(file) {
            return Some(i.clone());
        }
        if !editor_core::stamps::is_safe_image_name(file) {
            return None;
        }
        let bytes = std::fs::read(stamps_dir().join(file)).ok()?;
        let img = Arc::new(StampImage::from_bytes(&bytes, false).ok()?);
        self.stamp_images.insert(file.to_string(), img.clone());
        Some(img)
    }

    /// The stamp chosen in the stamp tool's panel.
    pub fn current_stamp_def(&self) -> Option<StampDef> {
        let name = self.current_stamp.as_ref()?;
        self.stamps.stamps.iter().find(|d| &d.name == name).cloned()
    }

    // ---- keeping ---------------------------------------------------------------------------

    fn save_stamp_library(&mut self) -> Result<(), String> {
        let text = self.stamps.to_toml().map_err(|e| e.to_string())?;
        platform::dirs::write_text_atomic(&stamps_file(), &text).map_err(|e| e.to_string())
    }

    /// Keep a stamp (and its picture) for good.
    fn add_stamp(&mut self, def: StampDef, picture: Option<&StampImage>) -> Result<(), String> {
        let old_file = self
            .stamps
            .stamps
            .iter()
            .find(|d| d.name == def.name.trim())
            .and_then(|d| match &d.kind {
                StampKind::Image { file } => Some(file.clone()),
                StampKind::Text { .. } => None,
            });
        let mut def = def;
        if let (StampKind::Image { file }, Some(img)) = (&mut def.kind, picture) {
            *file = self.stamps.new_image_name();
            let png = img.to_png().map_err(|e| e.to_string())?;
            write_bytes_atomic(&stamps_dir().join(&*file), &png).map_err(|e| e.to_string())?;
            self.stamp_images
                .insert(file.clone(), Arc::new(img.clone()));
        }
        let name = def.name.trim().to_string();
        self.stamps.add(def)?;
        if let Some(f) = old_file {
            let still_used = self
                .stamps
                .stamps
                .iter()
                .any(|d| matches!(&d.kind, StampKind::Image { file } if *file == f));
            if !still_used {
                let _ = std::fs::remove_file(stamps_dir().join(&f));
                self.stamp_images.remove(&f);
            }
        }
        self.save_stamp_library()?;
        self.current_stamp = Some(name);
        Ok(())
    }

    /// Forget a stamp and its picture.
    pub fn delete_stamp(&mut self, index: usize) {
        if let Some(def) = self.stamps.remove(index) {
            if let StampKind::Image { file } = &def.kind {
                let _ = std::fs::remove_file(stamps_dir().join(file));
                self.stamp_images.remove(file);
            }
            if self.current_stamp.as_deref() == Some(def.name.as_str()) {
                self.current_stamp = None;
            }
            if let Err(e) = self.save_stamp_library() {
                self.notify_error(tf!("Could not save the stamps: {}", e));
            }
        }
    }

    // ---- placing ---------------------------------------------------------------------------

    /// Put a picture on a page as a stamp annotation.
    pub(crate) fn place_picture(
        &mut self,
        page: PageId,
        image: Arc<StampImage>,
        rect: PRect,
        subject: &str,
        label: &str,
    ) {
        let mut spec = AnnotationSpec::new(AnnotationKind::ImageStamp { rect, image });
        spec.author = self.prefs.author.clone();
        spec.subject = subject.to_string();
        self.add_annotation(page, spec, label);
    }

    /// A box for `image` that is `width` points wide and centred on `c`.
    fn picture_box(image: &StampImage, c: Point, width: f64) -> PRect {
        let h = width * image.aspect();
        PRect::new(
            c.x - width / 2.0,
            c.y - h / 2.0,
            c.x + width / 2.0,
            c.y + h / 2.0,
        )
    }

    /// The saved signature as a picture: put it where the user clicked.
    pub(crate) fn place_signature_picture(&mut self, page: PageId, center: Point) -> bool {
        let Some(img) = self.signature_image.clone() else {
            return false;
        };
        let rect = Self::picture_box(&img, center, PLACED_WIDTH_PT);
        self.place_picture(
            page,
            img,
            rect,
            tr("Signature picture"),
            tr("Place signature"),
        );
        self.set_tool(Tool::Select);
        true
    }

    /// Place the chosen custom stamp. `r` is the box the user dragged; `tiny` when it was a click or a
    /// very small drag, which gives the stamp its own size at `at`.
    pub(crate) fn place_custom_stamp(
        &mut self,
        vc: &ViewCtx,
        page_index: usize,
        r: PRect,
        at: Point,
        tiny: bool,
    ) {
        let Some(def) = self.current_stamp_def() else {
            return;
        };
        let page = vc.pages[page_index].id;
        match def.kind {
            StampKind::Image { file } => {
                let Some(img) = self.stamp_image(&file) else {
                    self.notify_error(tr("The picture of this stamp is missing."));
                    return;
                };
                let rect = if tiny {
                    Self::picture_box(&img, at, PLACED_WIDTH_PT)
                } else {
                    // The picture fits the dragged box and keeps its proportions.
                    let (mut w, mut h) = (r.width(), r.width() * img.aspect());
                    if h > r.height() {
                        h = r.height();
                        w = h / img.aspect();
                    }
                    let c = r.center();
                    PRect::new(c.x - w / 2.0, c.y - h / 2.0, c.x + w / 2.0, c.y + h / 2.0)
                };
                self.place_picture(page, img, rect, &def.name, tr("Add stamp"));
            }
            StampKind::Text { label, color } => {
                let rect = if tiny {
                    Self::screen_box_at(vc, page_index, at, 140.0, 36.0)
                } else {
                    r
                };
                let mut spec = self.new_spec(AnnotationKind::StampText { rect, label });
                spec.color = Rgb(color[0], color[1], color[2]);
                spec.rotation = self.upright_rotation(page);
                spec.subject = def.name;
                self.add_annotation(page, spec, tr("Add stamp"));
            }
        }
    }

    // ---- signature from a picture ------------------------------------------------------------

    /// Sign ▸ Signature from Picture…
    pub fn open_signature_from_image(&mut self) {
        let Some(path) = platform::dialogs::pick_image() else {
            return;
        };
        match PictureState::load(&path) {
            Ok(st) => self.dialog = Some(Dialog::ImageSignature(Box::new(st))),
            Err(e) => self.notify_error(e),
        }
    }

    /// The dialog that shows the picture before it becomes the signature. Returns whether it stays open.
    pub fn dialog_image_signature(&mut self, ctx: &egui::Context, st: &mut PictureState) -> bool {
        let mut save = false;
        let mut other = false;
        let mut cancel = false;
        let danger = self.pal.danger;
        modal(ctx, "image_signature", |ui| {
            ui.heading(tr("Signature from a picture"));
            ui.label(
                RichText::new(tr("A scan or photo of your signature on white paper works best. This is a picture of your signature, not a digital signature."))
                    .size(12.0)
                    .color(self.pal.text_dim),
            );
            ui.add_space(6.0);
            st.show(ui, Vec2::new(420.0, 150.0), danger);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if crate::ui_kit::primary_enabled(ui, st.image.is_some(), tr("Save signature"))
                    .clicked()
                {
                    save = true;
                }
                if ui.button(tr("Other picture…")).clicked() {
                    other = true;
                }
                if ui.button(tr("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    cancel = true;
                }
            });
        });
        if other && let Some(p) = platform::dialogs::pick_image() {
            match PictureState::load(&p) {
                Ok(n) => *st = n,
                Err(e) => st.error = Some(e),
            }
        }
        if cancel {
            return false;
        }
        if save && let Some(img) = st.image.clone() {
            let kept = img.to_png().map_err(|e| e.to_string()).and_then(|png| {
                write_bytes_atomic(&signature_image_file(), &png).map_err(|e| e.to_string())
            });
            match kept {
                Ok(()) => {
                    // One saved signature: the picture replaces a drawn one.
                    let _ =
                        std::fs::remove_file(platform::dirs::config_dir().join("signature.toml"));
                    self.handwriting = Default::default();
                    self.signature_image = Some(img);
                    self.notify(tr(
                        "Signature saved. Use Place Signature to put it on a page.",
                    ));
                    self.set_tool(Tool::PlaceSignature);
                    return false;
                }
                Err(e) => st.error = Some(tf!("Could not save the signature: {}", e)),
            }
        }
        true
    }

    // ---- new stamp -------------------------------------------------------------------------

    /// Comment ▸ New Stamp…
    pub fn open_new_stamp(&mut self) {
        self.dialog = Some(Dialog::NewStamp(Box::default()));
    }

    /// The "New stamp" dialog. Returns whether it stays open.
    pub fn dialog_new_stamp(&mut self, ctx: &egui::Context, st: &mut NewStampState) -> bool {
        let mut save = false;
        let mut cancel = false;
        let mut choose = false;
        let danger = self.pal.danger;
        modal(ctx, "new_stamp", |ui| {
            ui.heading(tr("New stamp"));
            ui.label(
                RichText::new(tr("Make a stamp of your own: a text in your colour, or a picture such as a logo. It is kept on this computer and shows up in the Stamp tool."))
                    .size(12.0)
                    .color(self.pal.text_dim),
            );
            ui.add_space(6.0);
            ui.add(
                crate::ui_kit::singleline(&mut st.name)
                    .hint_text(tr("Name of the stamp"))
                    .char_limit(MAX_NAME)
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.radio_value(&mut st.picture_mode, false, tr("Text"));
                ui.radio_value(&mut st.picture_mode, true, tr("Picture"));
            });
            ui.add_space(4.0);
            if st.picture_mode {
                if ui.button(tr("Choose picture…")).clicked() {
                    choose = true;
                }
                if let Some(p) = &mut st.picture {
                    p.show(ui, Vec2::new(420.0, 140.0), danger);
                }
            } else {
                ui.horizontal(|ui| {
                    ui.add(
                        crate::ui_kit::singleline(&mut st.label)
                            .hint_text(tr("Text on the stamp"))
                            .char_limit(MAX_LABEL)
                            .desired_width(280.0),
                    );
                    ui.color_edit_button_rgb(&mut st.color);
                });
                let (rect, _) = ui.allocate_exact_size(Vec2::new(420.0, 64.0), Sense::hover());
                ui.painter().rect_filled(rect, 4.0, Color32::WHITE);
                if !st.label.trim().is_empty() {
                    let c = Color32::from_rgb(
                        (st.color[0] * 255.0) as u8,
                        (st.color[1] * 255.0) as u8,
                        (st.color[2] * 255.0) as u8,
                    );
                    let galley = ui.painter().layout_no_wrap(
                        st.label.trim().to_string(),
                        egui::FontId::proportional(24.0),
                        c,
                    );
                    let boxr = egui::Rect::from_center_size(
                        rect.center(),
                        galley.size() + Vec2::splat(16.0),
                    );
                    ui.painter().rect_stroke(
                        boxr,
                        2.0,
                        Stroke::new(2.0, c),
                        egui::StrokeKind::Middle,
                    );
                    ui.painter().galley(boxr.min + Vec2::splat(8.0), galley, c);
                }
            }
            if let Some(e) = &st.error {
                ui.colored_label(danger, e);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if crate::ui_kit::primary_button(ui, tr("Save stamp")).clicked() {
                    save = true;
                }
                if ui.button(tr("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    cancel = true;
                }
            });
        });
        if choose && let Some(p) = platform::dialogs::pick_image() {
            match PictureState::load(&p) {
                Ok(pic) => {
                    if st.name.trim().is_empty() {
                        st.name = pic.file_stem.chars().take(MAX_NAME).collect();
                    }
                    st.picture = Some(pic);
                    st.error = None;
                }
                Err(e) => st.error = Some(e),
            }
        }
        if cancel {
            return false;
        }
        if !save {
            return true;
        }
        let (kind, picture) = if st.picture_mode {
            let Some(img) = st.picture.as_ref().and_then(|p| p.image.clone()) else {
                st.error = Some(tr("Choose a picture first.").into());
                return true;
            };
            (
                StampKind::Image {
                    file: String::new(),
                },
                Some(img),
            )
        } else {
            (
                StampKind::Text {
                    label: st.label.clone(),
                    color: st.color,
                },
                None,
            )
        };
        let name = st.name.trim().to_string();
        match self.add_stamp(
            StampDef {
                name: name.clone(),
                kind,
            },
            picture.as_deref(),
        ) {
            Ok(()) => {
                self.set_tool(Tool::Stamp);
                self.current_stamp = Some(name);
                self.notify(tr(
                    "Stamp saved. Drag a box on the page, or click, to place it.",
                ));
                false
            }
            Err(e) => {
                st.error = Some(tf!("Could not save the stamp: {}", e));
                true
            }
        }
    }

    // ---- the stamp tool's panel ------------------------------------------------------------

    /// "My stamps": the list in the options of the Stamp tool.
    pub fn stamp_picker(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new(tr("My stamps")).strong());
        let mut choose: Option<Option<String>> = None;
        let mut delete: Option<usize> = None;
        crate::ui_kit::card(ui, |ui| {
            if ui
                .selectable_label(
                    self.current_stamp.is_none(),
                    tr("Ask for the text each time"),
                )
                .clicked()
            {
                choose = Some(None);
            }
            for (i, d) in self.stamps.stamps.iter().enumerate() {
                ui.horizontal(|ui| {
                    let kind = match d.kind {
                        StampKind::Text { .. } => tr("text"),
                        StampKind::Image { .. } => tr("picture"),
                    };
                    if ui
                        .selectable_label(
                            self.current_stamp.as_deref() == Some(d.name.as_str()),
                            format!("{}  ·  {}", d.name, kind),
                        )
                        .clicked()
                    {
                        choose = Some(Some(d.name.clone()));
                    }
                    if ui
                        .small_button("✕")
                        .on_hover_text(tr("Delete this stamp"))
                        .clicked()
                    {
                        delete = Some(i);
                    }
                });
            }
            if self.stamps.stamps.is_empty() {
                ui.label(
                    RichText::new(tr("You have no stamps of your own yet."))
                        .size(12.0)
                        .color(self.pal.text_dim),
                );
            }
        });
        if let Some(c) = choose {
            self.current_stamp = c;
        }
        if let Some(i) = delete {
            self.delete_stamp(i);
        }
        ui.add_space(4.0);
        if ui.button(tr("New stamp…")).clicked() {
            self.open_new_stamp();
        }
        ui.add_space(8.0);
    }
}
