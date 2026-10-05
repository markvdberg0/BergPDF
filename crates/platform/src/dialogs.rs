//! Native file dialogs (Windows common dialogs, macOS NSOpenPanel/NSSavePanel, Linux XDG portal).

use std::path::{Path, PathBuf};

/// Ask for one or more PDFs to open.
pub fn pick_open_pdfs(start_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut d = rfd::FileDialog::new()
        .set_title("Open PDF")
        .add_filter("PDF documents", &["pdf"]);
    if let Some(dir) = start_dir {
        d = d.set_directory(dir);
    }
    d.pick_files().unwrap_or_default()
}

/// Ask where to save a PDF. `suggested` is the default file name.
pub fn pick_save_pdf(suggested: &str, start_dir: Option<&Path>) -> Option<PathBuf> {
    let mut d = rfd::FileDialog::new()
        .set_title("Save PDF")
        .add_filter("PDF documents", &["pdf"])
        .set_file_name(suggested);
    if let Some(dir) = start_dir {
        d = d.set_directory(dir);
    }
    let mut p = d.save_file()?;
    if p.extension().is_none() {
        p.set_extension("pdf");
    }
    Some(p)
}

/// Ask for an image to place on a page.
pub fn pick_image() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Choose image")
        .add_filter("Images", &["png", "jpg", "jpeg"])
        .pick_file()
}

/// Ask for a CSV destination.
pub fn pick_save_csv(suggested: &str) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("CSV", &["csv"])
        .set_file_name(suggested)
        .save_file()
}

/// Ask where to save an exported PNG image.
pub fn pick_save_png(suggested: &str) -> Option<PathBuf> {
    let mut p = rfd::FileDialog::new()
        .set_title("Export page as image")
        .add_filter("PNG image", &["png"])
        .set_file_name(suggested)
        .save_file()?;
    if p.extension().is_none() {
        p.set_extension("png");
    }
    Some(p)
}
