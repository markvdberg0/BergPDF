//! Offline text recognition for page images.
//!
//! Two neural models run on the pure-Rust `rten` runtime (CPU only): the stock `ocrs` detection
//! model finds the words and groups them into lines, and the PaddleOCR PP-OCRv5 Latin recognition
//! model reads each line. The Latin model knows the accented letters of the Western and Central
//! European languages (é, ë, ü, ß, ç, ø, ł, …), which the stock `ocrs` recognition model, with its
//! ASCII alphabet, could not produce. The model files are **not** part of this crate or the
//! repository: they are loaded from a directory chosen by the caller (see [`MODEL_FILES`] and
//! `cargo xtask fetch-ocr-models`), so that nothing is downloaded behind the user's back and the
//! model licences can be reviewed separately. The Latin model's character list is small and is
//! compiled in (`latin_dict.txt`).
//!
//! Limitations (not hidden): Latin script only; handwriting, tiny print, rotated and vertical text
//! do poorly.

use ocrs::{ImageSource, OcrEngine, OcrEngineParams};
use rten_imageproc::BoundingRect;
use rten_tensor::{AsView, Layout, NdTensor, NdTensorView};
use std::path::{Path, PathBuf};

/// File names expected in the model directory: `(detection, recognition)`.
pub const MODEL_FILES: (&str, &str) = ("text-detection.onnx", "latin-recognition.onnx");

/// `(file name, address, expected SHA-256 in lower-case hex)` of each model file. Everything that
/// fetches or bundles the models (the app's download button, `cargo xtask fetch-ocr-models`,
/// `cargo xtask dist --with-ocr-models`) uses this one list and refuses a file whose hash differs.
/// The detection model is the `ocrs` author's, as ONNX (downloaded 2026-11-03; the author no longer
/// serves the `.rten` file and warns that these addresses may change); the recognition model is
/// PaddleOCR's `latin_PP-OCRv5_mobile_rec` as ONNX, from a fixed commit of a Hugging Face
/// repository that converted PaddleOCR's models with `paddle2onnx` (downloaded 2026-11-02).
pub const MODEL_SOURCES: [(&str, &str, &str); 2] = [
    (
        "text-detection.onnx",
        "https://ocrs-models.s3-accelerate.amazonaws.com/text-detection.onnx",
        "a917b23dbd9524b465df7e922641b3ff2981623df4ded5a0234004ef2fee7cfe",
    ),
    (
        "latin-recognition.onnx",
        "https://huggingface.co/monkt/paddleocr-onnx/resolve/7b02d0a30a07ba2b92ad1ff5a8941ae2c633de65/languages/latin/rec.onnx",
        "614ffc2d6d3902d360fad7f1b0dd455ee45e877069d14c4e51a99dc4ef144409",
    ),
];

/// The host names [`MODEL_SOURCES`] connects to, for telling the user.
pub fn model_hosts() -> Vec<&'static str> {
    let mut hosts: Vec<&str> = MODEL_SOURCES
        .iter()
        .filter_map(|(_, url, _)| url.strip_prefix("https://")?.split('/').next())
        .collect();
    hosts.dedup();
    hosts
}

/// Attribution text shipped next to bundled or downloaded models.
pub const MODEL_NOTICE: &str = "OCR models for BergPDF\n\
======================\n\n\
text-detection.onnx is the stock text-detection model of the `ocrs` project\n\
(https://github.com/robertknight/ocrs, engine under MIT / Apache-2.0), published at\n\
https://github.com/robertknight/ocrs-models. According to that project it was trained\n\
exclusively on the HierText dataset (https://github.com/google-research-datasets/hiertext),\n\
which is licensed CC BY-SA 4.0 (https://creativecommons.org/licenses/by-sa/4.0/).\n\n\
latin-recognition.onnx is the latin_PP-OCRv5_mobile_rec model of PaddleOCR\n\
(https://github.com/PaddlePaddle/PaddleOCR, Copyright PaddlePaddle Authors), licensed under the\n\
Apache License 2.0 (https://www.apache.org/licenses/LICENSE-2.0), converted to ONNX with\n\
paddle2onnx and published at https://huggingface.co/monkt/paddleocr-onnx. Its character list\n\
(ppocrv5_latin_dict.txt, same licence) is built into BergPDF.\n\n\
The model files are not modified by BergPDF. Their own redistribution terms are those of the\n\
model authors; see the links above.\n";

/// Largest image (pixels) accepted in one call; keeps memory bounded.
pub const MAX_PIXELS: u64 = 40_000_000;

/// The characters of the Latin recognition model, one per line: class `i + 1` is line `i`, class
/// 0 is the CTC blank and the class after the last line is a space.
const LATIN_DICT: &str = include_str!("latin_dict.txt");

/// Height in pixels the Latin model reads lines at.
const LINE_HEIGHT: usize = 48;

/// Widest line image passed to the model; longer lines are squeezed to this width.
const MAX_LINE_WIDTH: usize = 4096;

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
    /// `ocrs` with only its detection model: finds words and lines.
    detector: OcrEngine,
    /// The Latin recognition model: one line image in, class scores per step out.
    recognizer: rten::Model,
    /// The model's characters (see [`LATIN_DICT`]).
    alphabet: Vec<char>,
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

fn latin_alphabet() -> Vec<char> {
    LATIN_DICT
        .lines()
        .filter_map(|l| l.chars().next())
        .collect()
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
        let detector = OcrEngine::new(OcrEngineParams {
            detection_model: Some(load(&det)?),
            ..Default::default()
        })
        .map_err(|e| OcrError::Load(e.to_string()))?;
        Ok(Engine {
            detector,
            recognizer: load(&rec)?,
            alphabet: latin_alphabet(),
        })
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
            .detector
            .prepare_input(src)
            .map_err(|e| OcrError::Recognize(e.to_string()))?;
        let words = self
            .detector
            .detect_words(&input)
            .map_err(|e| OcrError::Recognize(e.to_string()))?;
        let lines = self.detector.find_text_lines(&input, &words);
        let image = Rgb {
            px: rgb,
            w: w as usize,
            h: h as usize,
        };
        let mut out = Vec::new();
        for (li, line) in lines.iter().enumerate() {
            let boxes: Vec<PixBox> = line
                .iter()
                .map(|r| {
                    let b = r.bounding_rect();
                    PixBox {
                        x0: b.left(),
                        y0: b.top(),
                        x1: b.right(),
                        y1: b.bottom(),
                    }
                })
                .collect();
            for (text, b) in self.read_line(&image, &boxes)? {
                out.push(Word {
                    text,
                    x0: b.x0,
                    y0: b.y0,
                    x1: b.x1,
                    y1: b.y1,
                    line: li,
                });
            }
        }
        Ok(out)
    }

    /// Read one line whose detected words are `boxes`. Each word comes back with its detected box
    /// when the words read match the words detected one to one, else with a box estimated from
    /// where in the line its letters were read.
    fn read_line(&self, image: &Rgb, boxes: &[PixBox]) -> Result<Vec<(String, PixBox)>, OcrError> {
        let Some(line) = PixBox::union(boxes) else {
            return Ok(Vec::new());
        };
        // A margin like the one the model was trained with, clamped to the image.
        let pad = (line.y1 - line.y0) * 0.15;
        let crop = PixBox {
            x0: (line.x0 - pad).max(0.0).floor(),
            y0: (line.y0 - pad).max(0.0).floor(),
            x1: (line.x1 + pad).min(image.w as f32).ceil(),
            y1: (line.y1 + pad).min(image.h as f32).ceil(),
        };
        if crop.x1 - crop.x0 < 2.0 || crop.y1 - crop.y0 < 2.0 {
            return Ok(Vec::new());
        }
        let out = self
            .recognizer
            .run_one(image.line_tensor(&crop).into(), None)
            .map_err(|e| OcrError::Recognize(e.to_string()))?;
        let scores: NdTensor<f32, 3> = out.try_into().map_err(|_| {
            OcrError::Recognize("unexpected output of the recognition model".into())
        })?;
        if scores.size(2) != self.alphabet.len() + 2 {
            return Err(OcrError::Load(format!(
                "the recognition model has {} classes, its character list {}",
                scores.size(2),
                self.alphabet.len() + 2
            )));
        }
        let (chars, steps) = decode(&self.alphabet, scores.slice(0));
        let step = (crop.x1 - crop.x0) / steps.max(1) as f32;
        let mut read: Vec<(String, PixBox)> = Vec::new();
        let mut current: Option<(String, PixBox)> = None;
        for (c, t) in chars {
            if c == ' ' {
                read.extend(current.take());
                continue;
            }
            let x0 = (crop.x0 + t as f32 * step).max(line.x0);
            let x1 = (crop.x0 + (t + 1) as f32 * step).min(line.x1);
            let word = current.get_or_insert_with(|| {
                (
                    String::new(),
                    PixBox {
                        x0,
                        y0: line.y0,
                        x1,
                        y1: line.y1,
                    },
                )
            });
            word.0.push(c);
            word.1.x1 = x1;
        }
        read.extend(current);
        if read.len() == boxes.len() {
            let mut detected = boxes.to_vec();
            detected.sort_by(|a, b| a.x0.total_cmp(&b.x0));
            for ((_, b), d) in read.iter_mut().zip(detected) {
                *b = d;
            }
        }
        Ok(read)
    }
}

/// Greedy CTC decoding of `[steps, classes]` scores: the best class per step, repeats merged,
/// blanks dropped. Returns each character with the step it was read at, and the number of steps.
fn decode(alphabet: &[char], scores: NdTensorView<f32, 2>) -> (Vec<(char, usize)>, usize) {
    let (steps, classes) = (scores.size(0), scores.size(1));
    let mut chars = Vec::new();
    let mut prev = 0;
    for t in 0..steps {
        let mut best = 0;
        for k in 1..classes {
            if scores[[t, k]] > scores[[t, best]] {
                best = k;
            }
        }
        if best != 0 && best != prev {
            chars.push((alphabet.get(best - 1).copied().unwrap_or(' '), t));
        }
        prev = best;
    }
    (chars, steps)
}

/// An axis-aligned box in image pixels.
#[derive(Clone, Copy, Debug)]
struct PixBox {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

impl PixBox {
    fn union(boxes: &[PixBox]) -> Option<PixBox> {
        boxes.iter().copied().reduce(|a, b| PixBox {
            x0: a.x0.min(b.x0),
            y0: a.y0.min(b.y0),
            x1: a.x1.max(b.x1),
            y1: a.y1.max(b.y1),
        })
    }
}

/// The page image.
struct Rgb<'a> {
    px: &'a [u8],
    w: usize,
    h: usize,
}

impl Rgb<'_> {
    /// `crop` scaled (bilinear) to the model's line height, as the `[1, 3, 48, width]` tensor the
    /// Latin model expects: BGR channels (PaddleOCR reads images with OpenCV), values in [-1, 1].
    fn line_tensor(&self, crop: &PixBox) -> NdTensor<f32, 4> {
        let (cw, ch) = (crop.x1 - crop.x0, crop.y1 - crop.y0);
        let width = ((cw * LINE_HEIGHT as f32 / ch).round() as usize)
            .clamp(LINE_HEIGHT / 4, MAX_LINE_WIDTH);
        let (sx, sy) = (cw / width as f32, ch / LINE_HEIGHT as f32);
        let mut t = NdTensor::<f32, 4>::zeros([1, 3, LINE_HEIGHT, width]);
        let at = |x: usize, y: usize, c: usize| f32::from(self.px[(y * self.w + x) * 3 + c]);
        for y in 0..LINE_HEIGHT {
            let fy = (crop.y0 + (y as f32 + 0.5) * sy - 0.5).clamp(0.0, (self.h - 1) as f32);
            let (y0, wy) = (fy.floor() as usize, fy.fract());
            let y1 = (y0 + 1).min(self.h - 1);
            for x in 0..width {
                let fx = (crop.x0 + (x as f32 + 0.5) * sx - 0.5).clamp(0.0, (self.w - 1) as f32);
                let (x0, wx) = (fx.floor() as usize, fx.fract());
                let x1 = (x0 + 1).min(self.w - 1);
                for c in 0..3 {
                    let v = (at(x0, y0, c) * (1.0 - wx) + at(x1, y0, c) * wx) * (1.0 - wy)
                        + (at(x0, y1, c) * (1.0 - wx) + at(x1, y1, c) * wx) * wy;
                    t[[0, 2 - c, y, x]] = v / 127.5 - 1.0;
                }
            }
        }
        t
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn the_shared_model_list_matches_the_expected_file_names() {
        assert_eq!(MODEL_SOURCES[0].0, MODEL_FILES.0);
        assert_eq!(MODEL_SOURCES[1].0, MODEL_FILES.1);
        for (_, url, h) in MODEL_SOURCES {
            assert!(url.starts_with("https://"), "{url}");
            assert_eq!(h.len(), 64);
            assert!(
                h.bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            );
        }
        assert_eq!(
            model_hosts(),
            ["ocrs-models.s3-accelerate.amazonaws.com", "huggingface.co"]
        );
        assert!(MODEL_NOTICE.contains("CC BY-SA 4.0"));
        assert!(MODEL_NOTICE.contains("Apache License 2.0"));
    }

    #[test]
    fn the_character_list_has_one_character_per_line_including_accents() {
        assert!(LATIN_DICT.lines().all(|l| l.chars().count() == 1));
        let chars = latin_alphabet();
        assert_eq!(
            chars.len(),
            502,
            "the model has 502 characters + blank + space"
        );
        for c in "éèêëïöüäßçøåñłčšžőű€ÉÜÖÄ".chars() {
            assert!(chars.contains(&c), "{c} missing");
        }
    }

    #[test]
    fn decoding_merges_repeats_drops_blanks_and_keeps_the_steps() {
        let alphabet = latin_alphabet();
        let class = |c: char| alphabet.iter().position(|&a| a == c).unwrap_or(0) + 1;
        let space = alphabet.len() + 1;
        let seq = [class('é'), class('é'), 0, class('é'), space, 0, class('ß')];
        let mut scores = NdTensor::<f32, 2>::zeros([seq.len(), alphabet.len() + 2]);
        for (t, k) in seq.iter().enumerate() {
            scores[[t, *k]] = 1.0;
        }
        let (chars, steps) = decode(&alphabet, scores.view());
        assert_eq!(steps, 7);
        assert_eq!(
            chars,
            [('é', 0), ('é', 3), (' ', 4), ('ß', 6)],
            "a blank separates a doubled letter"
        );
    }

    #[test]
    fn missing_models_are_reported_not_panicked() {
        let d = std::env::temp_dir().join("berg-no-models-here");
        assert!(!models_present(&d));
        assert!(matches!(Engine::load(&d), Err(OcrError::MissingModel(_))));
    }

    #[test]
    fn an_install_with_only_the_old_rten_detection_file_is_offered_the_download() {
        let d = std::env::temp_dir().join(format!("berg-old-ocr-models-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("text-detection.rten"), b"old").unwrap();
        std::fs::write(d.join(MODEL_FILES.1), b"rec").unwrap();
        assert!(!models_present(&d));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
