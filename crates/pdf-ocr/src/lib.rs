//! Offline text recognition for page images.
//!
//! A thin wrapper around the pure-Rust `ocrs` engine (neural text detection + recognition running
//! on the `rten` runtime, CPU only). The two model files are **not** part of this crate or the
//! repository: they are loaded from a directory chosen by the caller (see [`MODEL_FILES`] and
//! `cargo xtask fetch-ocr-models`), so that nothing is downloaded behind the user's back and the
//! model licence can be reviewed separately.
//!
//! Limitations of the stock models (not hidden): Latin alphabet only, and the recognition
//! alphabet is basic ASCII — accented letters such as é or ë are read as their plain letters or
//! mis-read; handwriting, tiny print, rotated and vertical text do poorly.

use ocrs::{ImageSource, OcrEngine, OcrEngineParams, TextItem};
use std::path::{Path, PathBuf};

/// File names expected in the model directory: `(detection, recognition)`.
pub const MODEL_FILES: (&str, &str) = ("text-detection.rten", "text-recognition.rten");

/// Largest image (pixels) accepted in one call; keeps memory bounded.
pub const MAX_PIXELS: u64 = 40_000_000;

/// Errors from OCR.
#[derive(Debug, thiserror::Error)]
pub enum OcrError {
    /// A model file is missing.
    #[error("OCR model file not found: {0}")]
    MissingModel(PathBuf),
    /// A model file could not be loaded.
    #[error("could not load the OCR models: {0}")]
    Load(String),
    /// The image is unusable.
    #[error("unsuitable image: {0}")]
    BadImage(String),
    /// Recognition failed.
    #[error("text recognition failed: {0}")]
    Recognize(String),
}

/// A recognised word with its box in image pixels (y grows downward).
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    /// Text of the word.
    pub text: String,
    /// Left edge.
    pub x0: f32,
    /// Top edge.
    pub y0: f32,
    /// Right edge.
    pub x1: f32,
    /// Bottom edge.
    pub y1: f32,
    /// Index of the text line the word belongs to (reading order).
    pub line: usize,
}

/// Loaded OCR engine.
pub struct Engine {
    inner: OcrEngine,
}

/// Paths of the two model files inside `dir`.
pub fn model_paths(dir: &Path) -> (PathBuf, PathBuf) {
    (dir.join(MODEL_FILES.0), dir.join(MODEL_FILES.1))
}

/// Whether both model files exist in `dir`.
pub fn models_present(dir: &Path) -> bool {
    let (d, r) = model_paths(dir);
    d.is_file() && r.is_file()
}

impl Engine {
    /// Load the engine from the model directory.
    pub fn load(model_dir: &Path) -> Result<Engine, OcrError> {
        let (det, rec) = model_paths(model_dir);
        for p in [&det, &rec] {
            if !p.is_file() {
                return Err(OcrError::MissingModel(p.clone()));
            }
        }
        let load = |p: &Path| rten::Model::load_file(p).map_err(|e| OcrError::Load(e.to_string()));
        let inner = OcrEngine::new(OcrEngineParams {
            detection_model: Some(load(&det)?),
            recognition_model: Some(load(&rec)?),
            ..Default::default()
        })
        .map_err(|e| OcrError::Load(e.to_string()))?;
        Ok(Engine { inner })
    }

    /// Recognise the words in an RGB image (`rgb.len() == w * h * 3`), in reading order.
    pub fn recognize_rgb(&self, rgb: &[u8], w: u32, h: u32) -> Result<Vec<Word>, OcrError> {
        if w < 16 || h < 16 {
            return Err(OcrError::BadImage("image is too small".into()));
        }
        if u64::from(w) * u64::from(h) > MAX_PIXELS {
            return Err(OcrError::BadImage("image is too large".into()));
        }
        if rgb.len() != (w as usize) * (h as usize) * 3 {
            return Err(OcrError::BadImage(
                "pixel buffer does not match the size".into(),
            ));
        }
        let src =
            ImageSource::from_bytes(rgb, (w, h)).map_err(|e| OcrError::BadImage(e.to_string()))?;
        let input = self
            .inner
            .prepare_input(src)
            .map_err(|e| OcrError::Recognize(e.to_string()))?;
        let words = self
            .inner
            .detect_words(&input)
            .map_err(|e| OcrError::Recognize(e.to_string()))?;
        let lines = self.inner.find_text_lines(&input, &words);
        let text = self
            .inner
            .recognize_text(&input, &lines)
            .map_err(|e| OcrError::Recognize(e.to_string()))?;
        let mut out = Vec::new();
        for (li, line) in text.into_iter().flatten().enumerate() {
            for word in line.words() {
                let t: String = word.to_string();
                if t.trim().is_empty() {
                    continue;
                }
                let r = word.bounding_rect();
                out.push(Word {
                    text: t,
                    x0: r.left() as f32,
                    y0: r.top() as f32,
                    x1: r.right() as f32,
                    y1: r.bottom() as f32,
                    line: li,
                });
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_models_are_reported_not_panicked() {
        let d = std::env::temp_dir().join("berg-no-models-here");
        assert!(!models_present(&d));
        assert!(matches!(Engine::load(&d), Err(OcrError::MissingModel(_))));
    }
}
