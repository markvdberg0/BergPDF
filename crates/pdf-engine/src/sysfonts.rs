//! Fonts installed on this computer, usable for new text.
//!
//! Scanning reads only the small tables of each font file (names, style flags, embedding
//! permissions), never the glyph data, so it is quick even with hundreds of fonts. Only plain
//! TrueType outlines (`glyf`) are offered, because that is what the PDF font embedding writes.
//! Fonts whose licence flags (`OS/2 fsType`) forbid embedding, or forbid subsetting, are skipped.
//! This is a technical check of the flags the font maker set — not legal advice about a font's
//! licence.
//!
//! Families live in a process-wide registry; a [`crate::fontembed::FontFamily::System`] value is
//! an index into it.

use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

/// Largest font file considered.
const MAX_FILE: u64 = 80 * 1024 * 1024;
/// Deepest folder level scanned below each root.
const MAX_DEPTH: usize = 5;

/// One face (weight and slant) of an installed family.
#[derive(Debug)]
pub struct SysFace {
    /// Font file.
    pub path: PathBuf,
    /// Index inside a `.ttc` collection (0 otherwise).
    pub index: u32,
    /// Bold weight.
    pub bold: bool,
    /// Italic or oblique.
    pub italic: bool,
    data: OnceLock<Option<&'static [u8]>>,
}

impl SysFace {
    fn new(path: PathBuf, index: u32, bold: bool, italic: bool) -> Self {
        Self {
            path,
            index,
            bold,
            italic,
            data: OnceLock::new(),
        }
    }

    /// The font program, read from disk on first use and kept for the life of the process.
    pub fn data(&self) -> Option<&'static [u8]> {
        *self.data.get_or_init(|| {
            let bytes = std::fs::read(&self.path).ok()?;
            Some(&*Box::leak(bytes.into_boxed_slice()))
        })
    }
}

/// An installed font family with its available faces.
#[derive(Debug)]
pub struct SysFamily {
    /// Name shown to the user, e.g. "Arial".
    pub name: &'static str,
    /// Stable key stored in preferences and documents: `sys:` plus the name.
    pub key: &'static str,
    /// Available faces.
    pub faces: Vec<SysFace>,
}

impl SysFamily {
    /// The face for a weight/slant, falling back to the regular face (never a synthetic one).
    pub fn face(&self, bold: bool, italic: bool) -> Option<&SysFace> {
        let pick = |b: bool, i: bool| self.faces.iter().find(|f| f.bold == b && f.italic == i);
        pick(bold, italic)
            .or_else(|| pick(bold, false))
            .or_else(|| pick(false, italic))
            .or_else(|| pick(false, false))
            .or_else(|| self.faces.first())
    }

    /// Whether a real italic face exists.
    pub fn has_italic(&self) -> bool {
        self.faces.iter().any(|f| f.italic)
    }

    /// Whether a real bold face exists.
    pub fn has_bold(&self) -> bool {
        self.faces.iter().any(|f| f.bold)
    }
}

fn registry() -> &'static RwLock<Vec<SysFamily>> {
    static R: OnceLock<RwLock<Vec<SysFamily>>> = OnceLock::new();
    R.get_or_init(|| RwLock::new(Vec::new()))
}

/// Number of installed families registered so far.
pub fn count() -> usize {
    registry().read().map_or(0, |r| r.len())
}

/// Run `f` on a family.
pub fn with_family<R>(id: u32, f: impl FnOnce(&SysFamily) -> R) -> Option<R> {
    registry().read().ok()?.get(id as usize).map(f)
}

/// Id of the family called `name` (case-insensitive).
pub fn find_by_name(name: &str) -> Option<u32> {
    let r = registry().read().ok()?;
    r.iter()
        .position(|f| f.name.eq_ignore_ascii_case(name.trim()))
        .map(|i| i as u32)
}

/// `(id, name)` of every family, sorted by name (case-insensitive).
pub fn list() -> Vec<(u32, &'static str)> {
    let Ok(r) = registry().read() else {
        return Vec::new();
    };
    let mut v: Vec<(u32, &'static str)> = r
        .iter()
        .enumerate()
        .map(|(i, f)| (i as u32, f.name))
        .collect();
    v.sort_by_key(|a| a.1.to_lowercase());
    v
}

/// A face found by a scan, before it is registered.
#[derive(Debug, Clone)]
pub struct Found {
    /// Family name (name ID 1).
    pub family: String,
    /// Font file.
    pub path: PathBuf,
    /// Index inside a collection.
    pub index: u32,
    /// Bold.
    pub bold: bool,
    /// Italic.
    pub italic: bool,
}

/// Add scanned faces to the registry (merging into families by name; duplicates are ignored).
pub fn install(found: Vec<Found>) {
    let Ok(mut r) = registry().write() else {
        return;
    };
    for f in found {
        let pos = r
            .iter()
            .position(|x| x.name.eq_ignore_ascii_case(&f.family));
        let idx = match pos {
            Some(i) => i,
            None => {
                let name: &'static str = Box::leak(f.family.clone().into_boxed_str());
                let key: &'static str = Box::leak(format!("sys:{}", f.family).into_boxed_str());
                r.push(SysFamily {
                    name,
                    key,
                    faces: Vec::new(),
                });
                r.len() - 1
            }
        };
        let fam = &mut r[idx];
        if fam
            .faces
            .iter()
            .any(|x| x.bold == f.bold && x.italic == f.italic)
        {
            continue;
        }
        fam.faces
            .push(SysFace::new(f.path, f.index, f.bold, f.italic));
    }
}

/// The places fonts are installed on this operating system.
pub fn default_roots() -> Vec<PathBuf> {
    let mut v = Vec::new();
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    let home = home.map(PathBuf::from);
    if cfg!(target_os = "windows") {
        let win = std::env::var_os("WINDIR")
            .or_else(|| std::env::var_os("SystemRoot"))
            .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
        v.push(win.join("Fonts"));
        if let Some(l) = std::env::var_os("LOCALAPPDATA") {
            v.push(PathBuf::from(l).join(r"Microsoft\Windows\Fonts"));
        }
    } else if cfg!(target_os = "macos") {
        v.push("/System/Library/Fonts".into());
        v.push("/System/Library/Fonts/Supplemental".into());
        v.push("/Library/Fonts".into());
        if let Some(h) = &home {
            v.push(h.join("Library/Fonts"));
        }
    } else {
        v.push("/usr/share/fonts".into());
        v.push("/usr/local/share/fonts".into());
        if let Some(h) = &home {
            v.push(h.join(".fonts"));
            v.push(h.join(".local/share/fonts"));
        }
    }
    v
}

/// Scan the default font folders and register what is found. Returns the number of families.
pub fn scan_and_register() -> usize {
    let found = scan(&default_roots());
    install(found);
    count()
}

/// Scan `roots` (recursively) for usable fonts.
pub fn scan(roots: &[PathBuf]) -> Vec<Found> {
    let mut out = Vec::new();
    let mut seen_dirs = HashSet::new();
    for r in roots {
        walk(r, 0, &mut seen_dirs, &mut out);
    }
    out
}

fn walk(dir: &Path, depth: usize, seen: &mut HashSet<PathBuf>, out: &mut Vec<Found>) {
    if depth > MAX_DEPTH {
        return;
    }
    // Symlink loops: remember where we have been.
    if let Ok(c) = dir.canonicalize()
        && !seen.insert(c)
    {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            walk(&p, depth + 1, seen, out);
        } else if is_font_file(&p) {
            out.extend(read_file(&p));
        }
    }
}

fn is_font_file(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "ttf" | "ttc" | "otf"))
}

fn be16(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn be32(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

fn read_at(f: &mut File, off: u64, len: usize) -> Option<Vec<u8>> {
    f.seek(SeekFrom::Start(off)).ok()?;
    let mut v = vec![0u8; len];
    f.read_exact(&mut v).ok()?;
    Some(v)
}

/// Every usable face in one font file (a collection can hold several).
pub fn read_file(path: &Path) -> Vec<Found> {
    let Ok(meta) = std::fs::metadata(path) else {
        return Vec::new();
    };
    if meta.len() > MAX_FILE || meta.len() < 64 {
        return Vec::new();
    }
    let Ok(mut f) = File::open(path) else {
        return Vec::new();
    };
    let Some(head) = read_at(&mut f, 0, 12) else {
        return Vec::new();
    };
    let offsets: Vec<u32> = if &head[0..4] == b"ttcf" {
        let n = be32(&head, 8).unwrap_or(0).min(64) as usize;
        let Some(tab) = read_at(&mut f, 12, n * 4) else {
            return Vec::new();
        };
        (0..n).filter_map(|i| be32(&tab, i * 4)).collect()
    } else {
        vec![0]
    };
    offsets
        .into_iter()
        .enumerate()
        .filter_map(|(i, off)| read_face(&mut f, u64::from(off), i as u32, path))
        .collect()
}

fn read_face(f: &mut File, off: u64, index: u32, path: &Path) -> Option<Found> {
    let hdr = read_at(f, off, 12)?;
    let tag = &hdr[0..4];
    // 0x00010000 = TrueType, "true" = Apple TrueType, "OTTO" = CFF (not supported).
    if !(tag == [0, 1, 0, 0] || tag == b"true") {
        return None;
    }
    let n = be16(&hdr, 4)? as usize;
    if n == 0 || n > 200 {
        return None;
    }
    let dir = read_at(f, off + 12, n * 16)?;
    let table = |t: &[u8; 4]| -> Option<(u64, usize)> {
        (0..n).find_map(|i| {
            let r = dir.get(i * 16..i * 16 + 16)?;
            (&r[0..4] == t).then(|| {
                (
                    u64::from(u32::from_be_bytes([r[8], r[9], r[10], r[11]])),
                    u32::from_be_bytes([r[12], r[13], r[14], r[15]]) as usize,
                )
            })
        })
    };
    table(b"glyf")?;
    table(b"cmap")?;
    let (name_off, name_len) = table(b"name")?;
    let (os2_off, os2_len) = table(b"OS/2")?;
    if name_len == 0 || name_len > 2 * 1024 * 1024 || os2_len < 64 {
        return None;
    }
    let os2 = read_at(f, os2_off, os2_len.min(96))?;
    let fs_type = be16(&os2, 8)?;
    // Bit 1: restricted licence (no embedding). Bit 8: no subsetting. Bit 9: bitmaps only.
    if fs_type & 0x0002 != 0 || fs_type & 0x0100 != 0 || fs_type & 0x0200 != 0 {
        return None;
    }
    let fs_selection = be16(&os2, 62)?;
    let mut italic = fs_selection & 1 != 0;
    let mut bold = fs_selection & 0x20 != 0;
    if let Some((head_off, head_len)) = table(b"head")
        && head_len >= 46
        && let Some(head) = read_at(f, head_off, 46)
        && let Some(mac) = be16(&head, 44)
    {
        // Older fonts only set the macStyle bits.
        bold |= mac & 1 != 0;
        italic |= mac & 2 != 0;
    }
    let names = read_at(f, name_off, name_len)?;
    let family = family_name(&names)?;
    Some(Found {
        family,
        path: path.to_path_buf(),
        index,
        bold,
        italic,
    })
}

/// Family name (name ID 1), preferring English Windows names.
fn family_name(t: &[u8]) -> Option<String> {
    let count = be16(t, 2)? as usize;
    let str_off = be16(t, 4)? as usize;
    let mut best: Option<(u8, String)> = None;
    for i in 0..count.min(400) {
        let r = 6 + i * 12;
        let (plat, enc, lang, id, len, off) = (
            be16(t, r)?,
            be16(t, r + 2)?,
            be16(t, r + 4)?,
            be16(t, r + 6)?,
            be16(t, r + 8)? as usize,
            be16(t, r + 10)? as usize,
        );
        if id != 1 {
            continue;
        }
        let raw = t.get(str_off + off..str_off + off + len)?;
        let (rank, s) = match (plat, enc) {
            (3, 1 | 10) => {
                let units: Vec<u16> = raw
                    .chunks_exact(2)
                    .map(|c| u16::from_be_bytes([c[0], c[1]]))
                    .collect();
                (
                    if lang == 0x409 { 3 } else { 2 },
                    String::from_utf16_lossy(&units),
                )
            }
            (1, 0) => (1, raw.iter().map(|&b| char::from(b)).collect()),
            _ => continue,
        };
        let s = s.trim().to_string();
        if s.is_empty() {
            continue;
        }
        if best.as_ref().is_none_or(|(r, _)| rank > *r) {
            best = Some((rank, s));
        }
    }
    best.map(|(_, s)| s)
}

/// A registered family that has a glyph for every character of `chars`, preferring families that
/// are known to be broad (so CJK text finds a CJK font first). `None` when nothing covers them.
pub fn family_covering(chars: &[char]) -> Option<u32> {
    const PREFERRED: &[&str] = &[
        "Arial Unicode MS",
        "Noto Sans CJK SC",
        "Noto Sans CJK JP",
        "Noto Sans CJK KR",
        "Noto Sans",
        "Microsoft YaHei",
        "Yu Gothic",
        "Malgun Gothic",
        "SimSun",
        "MS Gothic",
        "PingFang SC",
        "Hiragino Sans",
        "Apple SD Gothic Neo",
        "Segoe UI",
        "Segoe UI Symbol",
        "Arial",
        "Times New Roman",
    ];
    let covers = |id: u32| -> bool {
        with_family(id, |fam| {
            fam.face(false, false)
                .and_then(SysFace::data)
                .and_then(|d| {
                    ttf_parser::Face::parse(d, fam.face(false, false).map_or(0, |f| f.index)).ok()
                })
                .is_some_and(|face| chars.iter().all(|c| face.glyph_index(*c).is_some()))
        })
        .unwrap_or(false)
    };
    for p in PREFERRED {
        if let Some(id) = find_by_name(p)
            && covers(id)
        {
            return Some(id);
        }
    }
    list().into_iter().map(|(id, _)| id).find(|id| covers(*id))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn a_scanned_folder_yields_families_with_their_styles() {
        let dir = tempfile::tempdir().unwrap();
        let fonts: [(&str, &[u8]); 2] = [
            (
                "a-regular.ttf",
                include_bytes!("../assets/fonts/LiberationSans-Regular.ttf"),
            ),
            (
                "a-bolditalic.ttf",
                include_bytes!("../assets/fonts/LiberationSans-BoldItalic.ttf"),
            ),
        ];
        for (n, b) in fonts {
            std::fs::write(dir.path().join(n), b).unwrap();
        }
        std::fs::write(dir.path().join("notes.txt"), b"not a font").unwrap();
        let found = scan(&[dir.path().to_path_buf()]);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found.iter().all(|f| f.family == "Liberation Sans"));
        assert!(found.iter().any(|f| !f.bold && !f.italic));
        assert!(found.iter().any(|f| f.bold && f.italic));
    }

    #[test]
    fn fonts_that_forbid_embedding_or_subsetting_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let orig = include_bytes!("../assets/fonts/LiberationSans-Regular.ttf").to_vec();
        for (name, fs_type) in [
            ("ok.ttf", 0u16),
            ("restricted.ttf", 2),
            ("nosubset.ttf", 0x100),
        ] {
            let mut b = orig.clone();
            // Patch OS/2 fsType (offset 8 inside the table).
            let n = be16(&b, 4).unwrap() as usize;
            let rec = (0..n)
                .map(|i| 12 + i * 16)
                .find(|&r| &b[r..r + 4] == b"OS/2")
                .unwrap();
            let off = be32(&b, rec + 8).unwrap() as usize;
            b[off + 8..off + 10].copy_from_slice(&fs_type.to_be_bytes());
            std::fs::write(dir.path().join(name), b).unwrap();
        }
        let found = scan(&[dir.path().to_path_buf()]);
        let names: Vec<_> = found
            .iter()
            .map(|f| f.path.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, vec!["ok.ttf"], "{names:?}");
    }

    #[test]
    fn missing_or_broken_files_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("broken.ttf"), vec![0u8; 500]).unwrap();
        std::fs::write(dir.path().join("tiny.ttf"), b"x").unwrap();
        assert!(scan(&[dir.path().to_path_buf(), "/definitely/not/here".into()]).is_empty());
    }
}
