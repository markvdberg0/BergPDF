//! Interface language: English (the source text), Dutch and German.
//!
//! Every text shown to the user is written in English in the code and passed through [`tr`] (a plain
//! text) or [`tf!`](crate::tf) (a text with `{}` holes for values). The English text is the key into the
//! catalogs in `nl.rs` and `de.rs`; a text that has no entry is shown in English, so an untranslated
//! string is never blank or broken. A test checks that every text used in the code is in both catalogs
//! and that the `{}` holes match.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

mod de;
mod nl;

/// An interface language.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Nl,
    De,
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

/// Use `lang` from now on.
pub fn set(lang: Lang) {
    CURRENT.store(
        match lang {
            Lang::En => 0,
            Lang::Nl => 1,
            Lang::De => 2,
        },
        Ordering::Relaxed,
    );
}

/// The language in use.
pub fn current() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Lang::Nl,
        2 => Lang::De,
        _ => Lang::En,
    }
}

/// The operating system's language, if it is one we have.
pub fn system_language() -> Lang {
    let loc = sys_locale::get_locale().unwrap_or_default().to_lowercase();
    if loc.starts_with("nl") {
        Lang::Nl
    } else if loc.starts_with("de") {
        Lang::De
    } else {
        Lang::En
    }
}

/// Use the language chosen in the preferences (the system language for `System`).
pub fn apply(pref: editor_core::prefs::Language) {
    use editor_core::prefs::Language as P;
    set(match pref {
        P::System => system_language(),
        P::English => Lang::En,
        P::Dutch => Lang::Nl,
        P::German => Lang::De,
    });
}

/// Catalog of a language: English text → translation.
pub(crate) fn catalog(lang: Lang) -> &'static [(&'static str, &'static str)] {
    match lang {
        Lang::En => &[],
        Lang::Nl => nl::ENTRIES,
        Lang::De => de::ENTRIES,
    }
}

fn table(lang: Lang) -> &'static HashMap<&'static str, &'static str> {
    static NL: OnceLock<HashMap<&str, &str>> = OnceLock::new();
    static DE: OnceLock<HashMap<&str, &str>> = OnceLock::new();
    static EN: OnceLock<HashMap<&str, &str>> = OnceLock::new();
    let (cell, entries) = match lang {
        Lang::En => (&EN, catalog(Lang::En)),
        Lang::Nl => (&NL, catalog(Lang::Nl)),
        Lang::De => (&DE, catalog(Lang::De)),
    };
    cell.get_or_init(|| entries.iter().copied().collect())
}

/// `text` in the interface language (the English text when there is no translation). It takes and
/// returns static text, so it can be used anywhere a string literal was.
pub fn tr(text: &'static str) -> &'static str {
    match current() {
        Lang::En => text,
        lang => table(lang).get(text).copied().unwrap_or(text),
    }
}

/// Like [`tr`] for a text with `{}` holes, which are filled with `args` in order.
pub fn tf(template: &'static str, args: &[&dyn Display]) -> String {
    let text = tr(template);
    let mut out = String::with_capacity(text.len() + 16);
    let mut it = args.iter();
    let mut rest = text;
    while let Some(i) = rest.find("{}") {
        out.push_str(&rest[..i]);
        if let Some(a) = it.next() {
            out.push_str(&a.to_string());
        }
        rest = &rest[i + 2..];
    }
    out.push_str(rest);
    out
}

/// A translated text with values: `tf!("Page {} of {}", page, total)`.
#[macro_export]
macro_rules! tf {
    ($template:literal $(, $arg:expr)* $(,)?) => {
        $crate::i18n::tf($template, &[$(&$arg as &dyn ::std::fmt::Display),*])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holes(s: &str) -> usize {
        s.matches("{}").count()
    }

    #[test]
    fn catalogs_have_unique_keys_matching_holes_and_no_empty_translations() {
        for lang in [Lang::Nl, Lang::De] {
            let mut seen = std::collections::HashSet::new();
            for (k, v) in catalog(lang) {
                assert!(seen.insert(*k), "{lang:?}: duplicate key {k:?}");
                assert!(!v.trim().is_empty(), "{lang:?}: empty translation of {k:?}");
                assert_eq!(
                    holes(k),
                    holes(v),
                    "{lang:?}: {{}} holes differ in {k:?} / {v:?}"
                );
            }
        }
    }

    #[test]
    fn untranslated_text_falls_back_to_english_and_holes_are_filled() {
        set(Lang::Nl);
        assert_eq!(tr("A text nobody translated"), "A text nobody translated");
        assert_eq!(
            tf("A text with {} and {}", &[&1, &"two"]),
            "A text with 1 and two"
        );
        set(Lang::En);
    }

    /// Decode the escapes of a Rust string literal body (enough for what the UI texts use).
    fn unescape(body: &str) -> String {
        let mut out = String::new();
        let mut it = body.chars().peekable();
        while let Some(c) = it.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match it.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\'') => out.push('\''),
                Some('\\') => out.push('\\'),
                Some('u') => {
                    let hex: String = it.by_ref().skip(1).take_while(|c| *c != '}').collect();
                    if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        out.push(ch);
                    }
                }
                Some('\n') => {
                    // Line continuation: the next line's indentation is dropped too.
                    while it.peek().is_some_and(|c| c.is_whitespace()) {
                        it.next();
                    }
                }
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => {}
            }
        }
        out
    }

    /// Every text passed to `tr("…")` or `tf!("…", …)` in the application's source files.
    fn texts_in_source() -> Vec<String> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut out = Vec::new();
        let mut stack = vec![dir];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    let src = std::fs::read_to_string(&p).unwrap();
                    // Only the code before the test module counts.
                    let src = src.split("#[cfg(test)]").next().unwrap_or("");
                    for marker in ["tr(", "tf!("] {
                        let mut rest = src;
                        while let Some(i) = rest.find(marker) {
                            let before = &rest[..i];
                            // rustfmt may put the literal on the next line.
                            let args = rest[i + marker.len()..].trim_start();
                            let Some(after) = args.strip_prefix('"') else {
                                rest = &rest[i + marker.len()..];
                                continue;
                            };
                            // Skip longer identifiers such as `str(` or `ctr(`.
                            let is_call = !before
                                .chars()
                                .next_back()
                                .is_some_and(|c| c.is_alphanumeric() || c == '_');
                            // The literal ends at the first unescaped quote.
                            let mut end = 0;
                            let b = after.as_bytes();
                            while end < b.len() && b[end] != b'"' {
                                end += if b[end] == b'\\' { 2 } else { 1 };
                            }
                            if is_call && end <= b.len() {
                                out.push(unescape(&after[..end]));
                            }
                            rest = &after[end.min(after.len())..];
                        }
                    }
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// The texts that live in the other crates and are translated where they are shown.
    fn texts_in_core() -> Vec<&'static str> {
        use editor_core::command::{Category, REGISTRY};
        use editor_core::prefs::{AiProvider, GfxBackend, PresentChoice, SETTINGS};
        let mut v: Vec<&'static str> = Vec::new();
        for c in REGISTRY {
            v.push(c.title);
            v.push(c.description);
            if let Some(t) = editor_core::tools::Tool::from_command(c.id) {
                v.push(t.hint());
            }
        }
        for c in [
            Category::File,
            Category::Edit,
            Category::View,
            Category::Navigate,
            Category::Comment,
            Category::Content,
            Category::Forms,
            Category::Measure,
            Category::Sign,
            Category::Organize,
            Category::Help,
        ] {
            v.push(c.title());
        }
        for s in SETTINGS {
            v.push(s.title);
            v.push(s.description);
        }
        for k in [
            pdf_engine::measure::MeasureKind::Distance,
            pdf_engine::measure::MeasureKind::Perimeter,
            pdf_engine::measure::MeasureKind::Area,
            pdf_engine::measure::MeasureKind::RectArea,
            pdf_engine::measure::MeasureKind::Radius,
            pdf_engine::measure::MeasureKind::Angle,
            pdf_engine::measure::MeasureKind::Count,
        ] {
            v.push(k.title());
        }
        for b in [
            GfxBackend::Auto,
            GfxBackend::Dx12,
            GfxBackend::Vulkan,
            GfxBackend::Gl,
        ] {
            v.push(b.title());
        }
        for p in [
            PresentChoice::Smooth,
            PresentChoice::LowLatency,
            PresentChoice::Uncapped,
        ] {
            v.push(p.title());
        }
        for p in [
            AiProvider::OpenAi,
            AiProvider::Anthropic,
            AiProvider::Custom,
        ] {
            v.push(p.title());
        }
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Prints the texts that miss from a catalog (`BERG_DUMP_MISSING=nl|de`), one JSON string per line.
    #[test]
    fn every_text_in_the_application_has_a_dutch_and_a_german_translation() {
        let mut texts: Vec<String> = texts_in_source();
        texts.extend(texts_in_core().into_iter().map(str::to_string));
        texts.sort();
        texts.dedup();
        for (name, lang) in [("nl", Lang::Nl), ("de", Lang::De)] {
            let have: std::collections::HashSet<&str> =
                catalog(lang).iter().map(|(k, _)| *k).collect();
            let missing: Vec<&String> = texts
                .iter()
                .filter(|t| !have.contains(t.as_str()))
                .collect();
            if std::env::var("BERG_DUMP_MISSING").is_ok_and(|v| v == name) {
                for m in &missing {
                    println!("MISSING {}", serde_json_like(m));
                }
            }
            assert!(
                missing.is_empty(),
                "{} texts have no {name} translation, e.g. {:?}",
                missing.len(),
                missing.iter().take(5).collect::<Vec<_>>()
            );
        }
    }

    fn serde_json_like(s: &str) -> String {
        let mut o = String::from("\"");
        for c in s.chars() {
            match c {
                '"' => o.push_str("\\\""),
                '\\' => o.push_str("\\\\"),
                '\n' => o.push_str("\\n"),
                c => o.push(c),
            }
        }
        o.push('"');
        o
    }
}
