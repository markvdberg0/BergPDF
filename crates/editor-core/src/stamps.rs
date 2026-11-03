//! The user's own stamps: a text with its own wording and colour, or a picture (a logo, a company
//! stamp, a scanned signature). The list is one small TOML file; pictures are kept as PNG files next to
//! it, named in the list. Everything read from disk is treated as untrusted.

use serde::{Deserialize, Serialize};

/// Most stamps kept.
pub const MAX_STAMPS: usize = 60;
/// Longest stamp name, in characters.
pub const MAX_NAME: usize = 40;
/// Longest stamp text, in characters.
pub const MAX_LABEL: usize = 40;
/// Longest second line of a dynamic stamp, in characters.
pub const MAX_DETAIL: usize = 80;

/// What a dynamic stamp's placeholders stand for when it is placed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StampValues {
    /// `{author}`: the name in Preferences.
    pub author: String,
    /// `{date}`: e.g. `2026-11-03`.
    pub date: String,
    /// `{time}`: e.g. `14:05`.
    pub time: String,
}

/// Fill in `{author}`, `{date}`, `{time}` and `{datetime}`.
pub fn expand_placeholders(template: &str, v: &StampValues) -> String {
    template
        .replace("{datetime}", &format!("{} {}", v.date, v.time))
        .replace("{author}", &v.author)
        .replace("{date}", &v.date)
        .replace("{time}", &v.time)
        .trim()
        .to_string()
}

/// Whether a text has placeholders that are filled in when the stamp is placed.
pub fn is_dynamic(text: &str) -> bool {
    ["{author}", "{date}", "{time}", "{datetime}"]
        .iter()
        .any(|p| text.contains(p))
}

/// What a stamp shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum StampKind {
    /// A word or phrase in a box, in `color` (red, green, blue 0–1).
    Text {
        /// What it says.
        label: String,
        /// Colour of the box and the text.
        color: [f32; 3],
        /// A smaller second line, usually with placeholders: a dynamic stamp such as
        /// "{author} · {date} {time}".
        #[serde(default)]
        detail: String,
    },
    /// A picture kept in `file` (a PNG in the stamps folder).
    Image {
        /// File name, without a folder.
        file: String,
    },
}

/// One saved stamp.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StampDef {
    /// Shown in the list.
    pub name: String,
    /// What it looks like.
    pub kind: StampKind,
}

/// All saved stamps.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StampLibrary {
    /// In the order they were added.
    pub stamps: Vec<StampDef>,
}

/// Whether `s` is a plain file name of a PNG (letters, digits, `-`, `_`, one `.png`): no folders, no
/// surprises.
pub fn is_safe_image_name(s: &str) -> bool {
    let Some(stem) = s.strip_suffix(".png") else {
        return false;
    };
    !stem.is_empty()
        && stem.len() <= 64
        && stem
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl StampLibrary {
    /// Serialise to TOML.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string(self)
    }

    /// Parse from TOML, dropping what is out of bounds.
    pub fn from_toml(s: &str) -> Option<Self> {
        let mut v: Self = toml::from_str(s).ok()?;
        v.stamps.retain(|d| match &d.kind {
            StampKind::Text { label, color, .. } => {
                !label.trim().is_empty() && color.iter().all(|c| c.is_finite())
            }
            StampKind::Image { file } => is_safe_image_name(file),
        });
        v.stamps.truncate(MAX_STAMPS);
        for d in &mut v.stamps {
            d.name = d.name.chars().take(MAX_NAME).collect();
            if let StampKind::Text {
                label,
                color,
                detail,
            } = &mut d.kind
            {
                *label = label.chars().take(MAX_LABEL).collect();
                *detail = detail
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(MAX_DETAIL)
                    .collect();
                for c in color.iter_mut() {
                    *c = c.clamp(0.0, 1.0);
                }
            }
        }
        Some(v)
    }

    /// Add a stamp; one with the same name is replaced. Returns its index.
    pub fn add(&mut self, mut def: StampDef) -> Result<usize, String> {
        def.name = def.name.trim().chars().take(MAX_NAME).collect();
        if def.name.is_empty() {
            return Err("give the stamp a name".into());
        }
        match &mut def.kind {
            StampKind::Text { label, detail, .. } => {
                *label = label.trim().chars().take(MAX_LABEL).collect();
                if label.is_empty() {
                    return Err("the stamp has no text".into());
                }
                *detail = detail
                    .trim()
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(MAX_DETAIL)
                    .collect();
            }
            StampKind::Image { file } => {
                if !is_safe_image_name(file) {
                    return Err("the picture has no usable file name".into());
                }
            }
        }
        if let Some(i) = self.stamps.iter().position(|d| d.name == def.name) {
            self.stamps[i] = def;
            return Ok(i);
        }
        if self.stamps.len() >= MAX_STAMPS {
            return Err(format!("at most {MAX_STAMPS} stamps can be kept"));
        }
        self.stamps.push(def);
        Ok(self.stamps.len() - 1)
    }

    /// Remove a stamp.
    pub fn remove(&mut self, index: usize) -> Option<StampDef> {
        (index < self.stamps.len()).then(|| self.stamps.remove(index))
    }

    /// A file name for a new picture that no stamp uses yet.
    pub fn new_image_name(&self) -> String {
        (1..)
            .map(|n| format!("stamp-{n}.png"))
            .find(|f| {
                !self
                    .stamps
                    .iter()
                    .any(|d| matches!(&d.kind, StampKind::Image { file } if file == f))
            })
            .unwrap_or_else(|| "stamp.png".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(name: &str, label: &str) -> StampDef {
        StampDef {
            name: name.into(),
            kind: StampKind::Text {
                label: label.into(),
                color: [0.8, 0.0, 0.0],
                detail: String::new(),
            },
        }
    }

    #[test]
    fn stamps_round_trip_and_replace_by_name() {
        let mut lib = StampLibrary::default();
        lib.add(text("Paid", "PAID")).unwrap();
        lib.add(StampDef {
            name: "Logo".into(),
            kind: StampKind::Image {
                file: "stamp-1.png".into(),
            },
        })
        .unwrap();
        assert_eq!(lib.add(text("Paid", "PAID IN FULL")).unwrap(), 0);
        assert_eq!(lib.stamps.len(), 2);
        let back = StampLibrary::from_toml(&lib.to_toml().unwrap()).unwrap();
        assert_eq!(back, lib);
        assert_eq!(lib.new_image_name(), "stamp-2.png");
    }

    #[test]
    fn what_comes_from_disk_is_checked() {
        let bad = r#"
            [[stamps]]
            name = "x"
            [stamps.kind]
            kind = "Image"
            file = "../../etc/passwd.png"

            [[stamps]]
            name = "ok"
            [stamps.kind]
            kind = "Text"
            label = "DONE"
            color = [5.0, -1.0, 0.5]
        "#;
        let lib = StampLibrary::from_toml(bad).unwrap();
        assert_eq!(lib.stamps.len(), 1, "a path in a file name is dropped");
        let StampKind::Text { color, .. } = &lib.stamps[0].kind else {
            panic!()
        };
        assert_eq!(*color, [1.0, 0.0, 0.5], "colours are clamped");
        assert!(!is_safe_image_name("a/b.png") && !is_safe_image_name("a.png.exe"));
        assert!(StampLibrary::from_toml("not toml {{").is_none());
    }

    #[test]
    fn nameless_or_empty_stamps_and_too_many_are_refused() {
        let mut lib = StampLibrary::default();
        assert!(lib.add(text("  ", "A")).is_err());
        assert!(lib.add(text("A", "   ")).is_err());
        for i in 0..MAX_STAMPS {
            lib.add(text(&format!("s{i}"), "X")).unwrap();
        }
        assert!(lib.add(text("one more", "X")).is_err());
    }

    #[test]
    fn dynamic_stamps_fill_in_name_date_and_time() {
        let v = StampValues {
            author: "Mark".into(),
            date: "2026-11-03".into(),
            time: "14:05".into(),
        };
        assert_eq!(
            expand_placeholders("{author} · {date} {time}", &v),
            "Mark · 2026-11-03 14:05"
        );
        assert_eq!(
            expand_placeholders("on {datetime}", &v),
            "on 2026-11-03 14:05"
        );
        assert!(is_dynamic("by {author}") && !is_dynamic("PAID"));
        // Older stamp files have no second line.
        let lib = StampLibrary::from_toml(
            "[[stamps]]
name = \"Paid\"
[stamps.kind]
kind = \"Text\"
label = \"PAID\"
color = [1.0, 0.0, 0.0]
",
        )
        .unwrap();
        assert!(matches!(&lib.stamps[0].kind, StampKind::Text { detail, .. } if detail.is_empty()));
    }
}
