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

/// Ask for PDFs and/or pictures to open (pictures become new one-page PDFs).
pub fn pick_open_documents(start_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut d = rfd::FileDialog::new()
        .set_title("Open")
        .add_filter("PDF documents and images", &["pdf", "png", "jpg", "jpeg"])
        .add_filter("PDF documents", &["pdf"])
        .add_filter("Images (become a PDF when saved)", &["png", "jpg", "jpeg"]);
    if let Some(dir) = start_dir {
        d = d.set_directory(dir);
    }
    d.pick_files().unwrap_or_default()
}

/// Whether a path looks like a picture BergPDF can turn into a PDF (by extension).
pub fn is_image_path(p: &Path) -> bool {
    p.extension().is_some_and(|e| {
        let e = e.to_string_lossy().to_ascii_lowercase();
        matches!(e.as_str(), "png" | "jpg" | "jpeg")
    })
}

/// Ask for one or more pictures to insert as pages.
pub fn pick_open_images() -> Vec<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Insert images as pages")
        .add_filter("Images", &["png", "jpg", "jpeg"])
        .pick_files()
        .unwrap_or_default()
}

/// Ask for a certificate file (PKCS#12) used to sign documents.
pub fn pick_certificate() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Choose your signing certificate")
        .add_filter("Certificate with private key (PKCS#12)", &["p12", "pfx"])
        .pick_file()
}

/// Ask where to save a plain text file.
pub fn pick_save_text(suggested: &str) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Text", &["txt"])
        .set_file_name(suggested)
        .save_file()
}
