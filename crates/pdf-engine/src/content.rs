//! Content-stream scanning and graphics/text-state simulation.
//!
//! The scanner records the *byte span* of every operator so edits can be surgical
//! replacements inside the decoded stream; nothing is decoded and re-encoded wholesale, which
//! keeps every byte we do not touch (comments, inline images, odd spacing) intact.
//!
//! The simulation tracks the CTM, the text matrices and the text state so that every
//! text-showing operator can be located on the page (hit-testing, boxes, advances) and image
//! `Do` operators can be placed.
//!
//! Supported subset (anything else is reported, not guessed): horizontal writing, text
//! render modes 0/1/2 (fill/stroke), `q/Q/cm`, `BT/ET`, `Tf Tc Tw Tz TL Ts Tr`,
//! `Td TD Tm T*`, `Tj TJ ' "`, and `Do`. Text inside Form XObjects is not scanned here.

use crate::error::{EngineError, Result};
use std::ops::Range;
use std::sync::Arc;

/// Maximum number of operators scanned in one page (guards pathological streams).
pub const MAX_OPS: usize = 4_000_000;
const MAX_NEST: usize = 64;

/// 2×3 affine matrix in the PDF row-vector convention: `[a b c d e f]` maps `(x, y)` to
/// `(a·x + c·y + e, b·x + d·y + f)`. `m.then(n)` applies `m` first, then `n`
/// (i.e. the PDF product `m × n`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat(pub [f64; 6]);

impl Mat {
    /// Identity.
    pub const IDENTITY: Mat = Mat([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    /// Translation.
    pub fn translate(x: f64, y: f64) -> Mat {
        Mat([1.0, 0.0, 0.0, 1.0, x, y])
    }

    /// Scale.
    pub fn scale(sx: f64, sy: f64) -> Mat {
        Mat([sx, 0.0, 0.0, sy, 0.0, 0.0])
    }

    /// `self × other` (apply `self` first).
    pub fn then(&self, o: &Mat) -> Mat {
        let [a, b, c, d, e, f] = self.0;
        let [oa, ob, oc, od, oe, of] = o.0;
        Mat([
            a * oa + b * oc,
            a * ob + b * od,
            c * oa + d * oc,
            c * ob + d * od,
            e * oa + f * oc + oe,
            e * ob + f * od + of,
        ])
    }

    /// Transform a point.
    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        let [a, b, c, d, e, f] = self.0;
        (a * x + c * y + e, b * x + d * y + f)
    }

    /// Determinant of the linear part.
    pub fn det(&self) -> f64 {
        self.0[0] * self.0[3] - self.0[1] * self.0[2]
    }

    /// Inverse, if the matrix is invertible.
    pub fn inverse(&self) -> Option<Mat> {
        let det = self.det();
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        let [a, b, c, d, e, f] = self.0;
        Some(Mat([
            d / det,
            -b / det,
            -c / det,
            a / det,
            (c * f - d * e) / det,
            (b * e - a * f) / det,
        ]))
    }

    /// Conjugate a user-space transform `t` into the coordinate system whose CTM is `self`:
    /// the matrix `A` such that inserting `A cm` just before an object drawn under CTM `self`
    /// applies `t` to the object *in outer user space* after all of its own transforms.
    pub fn conjugate(&self, t: &Mat) -> Option<Mat> {
        Some(self.then(t).then(&self.inverse()?))
    }

    /// Format as the six operands of `cm`/`Tm`.
    pub fn operands(&self) -> String {
        self.0
            .iter()
            .map(|v| crate::objutil::fmt_num_prec(*v))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// A parsed operand (only what editing needs).
#[derive(Clone, Debug, PartialEq)]
pub enum Operand {
    /// Number.
    Num(f64),
    /// Name (without the slash, `#xx` decoded).
    Name(Vec<u8>),
    /// String (decoded bytes).
    Str(Vec<u8>),
    /// Array.
    Array(Vec<Operand>),
    /// Boolean / null / dictionary — not needed for editing.
    Other,
}

impl Operand {
    /// Numeric value.
    pub fn num(&self) -> Option<f64> {
        match self {
            Operand::Num(n) => Some(*n),
            _ => None,
        }
    }
}

/// One operator with its operands and byte spans in the scanned buffer.
#[derive(Clone, Debug)]
pub struct Op {
    /// Operator keyword (e.g. `Tj`).
    pub name: Vec<u8>,
    /// Operands in order.
    pub operands: Vec<Operand>,
    /// Start of the first operand (or of the operator if it has none).
    pub start: usize,
    /// Start of the operator token itself.
    pub op_start: usize,
    /// End (exclusive) of the operator token (for `BI`, the end of `EI`).
    pub end: usize,
}

impl Op {
    /// Whole span of the operation.
    pub fn span(&self) -> Range<usize> {
        self.start..self.end
    }

    /// Operator as text.
    pub fn name_str(&self) -> &str {
        std::str::from_utf8(&self.name).unwrap_or("?")
    }
}

fn is_ws(b: u8) -> bool {
    matches!(b, 0 | 9 | 10 | 12 | 13 | 32)
}

fn is_delim(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

struct Lexer<'a> {
    d: &'a [u8],
    p: usize,
}

impl Lexer<'_> {
    fn skip_ws(&mut self) {
        while self.p < self.d.len() {
            let b = self.d[self.p];
            if is_ws(b) {
                self.p += 1;
            } else if b == b'%' {
                while self.p < self.d.len() && !matches!(self.d[self.p], 10 | 13) {
                    self.p += 1;
                }
            } else {
                break;
            }
        }
    }

    fn string(&mut self) -> Vec<u8> {
        // self.p is just past '('
        let mut depth = 1;
        let mut out = Vec::new();
        while self.p < self.d.len() {
            let b = self.d[self.p];
            self.p += 1;
            match b {
                b'\\' => {
                    let Some(&n) = self.d.get(self.p) else { break };
                    self.p += 1;
                    match n {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'(' | b')' | b'\\' => out.push(n),
                        b'\r' => {
                            if self.d.get(self.p) == Some(&b'\n') {
                                self.p += 1;
                            }
                        }
                        b'\n' => {}
                        b'0'..=b'7' => {
                            let mut v = u32::from(n - b'0');
                            for _ in 0..2 {
                                match self.d.get(self.p) {
                                    Some(&o @ b'0'..=b'7') => {
                                        v = v * 8 + u32::from(o - b'0');
                                        self.p += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push((v & 0xFF) as u8);
                        }
                        other => out.push(other),
                    }
                }
                b'(' => {
                    depth += 1;
                    out.push(b);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push(b);
                }
                _ => out.push(b),
            }
        }
        out
    }

    fn hex(&mut self) -> Vec<u8> {
        // self.p is just past '<'
        let mut out = Vec::new();
        let mut hi: Option<u8> = None;
        while self.p < self.d.len() {
            let b = self.d[self.p];
            self.p += 1;
            if b == b'>' {
                break;
            }
            let v = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                b'A'..=b'F' => b - b'A' + 10,
                _ => continue,
            };
            match hi.take() {
                None => hi = Some(v),
                Some(h) => out.push(h * 16 + v),
            }
        }
        if let Some(h) = hi {
            out.push(h * 16);
        }
        out
    }

    fn name(&mut self) -> Vec<u8> {
        // self.p is just past '/'
        let mut out = Vec::new();
        while self.p < self.d.len() {
            let b = self.d[self.p];
            if is_ws(b) || is_delim(b) {
                break;
            }
            self.p += 1;
            if b == b'#' && self.p + 1 < self.d.len() {
                let h = |c: u8| (c as char).to_digit(16);
                if let (Some(a), Some(b2)) = (h(self.d[self.p]), h(self.d[self.p + 1])) {
                    out.push((a * 16 + b2) as u8);
                    self.p += 2;
                    continue;
                }
            }
            out.push(b);
        }
        out
    }

    /// Parse one operand-ish value. Returns `None` when the next token is a keyword/operator.
    fn operand(&mut self, depth: usize) -> Result<Option<Operand>> {
        if depth > MAX_NEST {
            return Err(EngineError::LimitExceeded(
                "content stream nesting too deep".into(),
            ));
        }
        self.skip_ws();
        let Some(&b) = self.d.get(self.p) else {
            return Ok(None);
        };
        match b {
            b'/' => {
                self.p += 1;
                Ok(Some(Operand::Name(self.name())))
            }
            b'(' => {
                self.p += 1;
                Ok(Some(Operand::Str(self.string())))
            }
            b'<' if self.d.get(self.p + 1) == Some(&b'<') => {
                self.p += 2;
                self.skip_dict(depth + 1)?;
                Ok(Some(Operand::Other))
            }
            b'<' => {
                self.p += 1;
                Ok(Some(Operand::Str(self.hex())))
            }
            b'[' => {
                self.p += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_ws();
                    match self.d.get(self.p) {
                        None => break,
                        Some(b']') => {
                            self.p += 1;
                            break;
                        }
                        Some(_) => match self.operand(depth + 1)? {
                            Some(o) => items.push(o),
                            None => {
                                // A keyword inside an array (e.g. `true`); consume and ignore.
                                self.keyword();
                                items.push(Operand::Other);
                            }
                        },
                    }
                }
                Ok(Some(Operand::Array(items)))
            }
            b']' | b')' | b'>' | b'{' | b'}' => {
                // Stray delimiter: skip it so scanning always makes progress.
                self.p += 1;
                Ok(Some(Operand::Other))
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => {
                let start = self.p;
                self.p += 1;
                while self.p < self.d.len() && !is_ws(self.d[self.p]) && !is_delim(self.d[self.p]) {
                    self.p += 1;
                }
                let tok = &self.d[start..self.p];
                match parse_number(tok) {
                    Some(n) => Ok(Some(Operand::Num(n))),
                    None => {
                        self.p = start;
                        Ok(None)
                    }
                }
            }
            _ => Ok(None),
        }
    }

    fn skip_dict(&mut self, depth: usize) -> Result<()> {
        loop {
            self.skip_ws();
            match (self.d.get(self.p), self.d.get(self.p + 1)) {
                (None, _) => return Ok(()),
                (Some(b'>'), Some(b'>')) => {
                    self.p += 2;
                    return Ok(());
                }
                _ => {
                    if self.operand(depth)?.is_none() {
                        self.keyword();
                    }
                }
            }
        }
    }

    fn keyword(&mut self) -> Vec<u8> {
        let start = self.p;
        while self.p < self.d.len() && !is_ws(self.d[self.p]) && !is_delim(self.d[self.p]) {
            self.p += 1;
        }
        if self.p == start {
            self.p += 1; // never stall
        }
        self.d[start..self.p].to_vec()
    }
}

fn parse_number(tok: &[u8]) -> Option<f64> {
    let s = std::str::from_utf8(tok).ok()?;
    // PDF numbers: optional sign, digits, optional single dot. Be tolerant of "--5" and "1.2.3".
    let t = s.trim_start_matches(['+', '-']);
    let neg = s.starts_with('-');
    if t.is_empty() || !t.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return None;
    }
    let mut parts = t.splitn(2, '.');
    let int = parts.next().unwrap_or("");
    let frac = parts.next().unwrap_or("").split('.').next().unwrap_or("");
    let v: f64 = format!(
        "{}.{}",
        if int.is_empty() { "0" } else { int },
        if frac.is_empty() { "0" } else { frac }
    )
    .parse()
    .ok()?;
    Some(if neg { -v } else { v })
}

/// Scan a content stream into operators with byte spans.
pub fn scan(data: &[u8]) -> Result<Vec<Op>> {
    let mut lx = Lexer { d: data, p: 0 };
    let mut ops = Vec::new();
    let mut operands: Vec<Operand> = Vec::new();
    let mut first: Option<usize> = None;
    loop {
        lx.skip_ws();
        if lx.p >= data.len() {
            break;
        }
        let tok_start = lx.p;
        match lx.operand(0)? {
            Some(o) => {
                first.get_or_insert(tok_start);
                operands.push(o);
                if operands.len() > 4096 {
                    // Operand stack overflow: pathological; drop the oldest.
                    operands.remove(0);
                }
            }
            None => {
                let kw = lx.keyword();
                if ops.len() >= MAX_OPS {
                    return Err(EngineError::LimitExceeded(
                        "content stream has too many operators".into(),
                    ));
                }
                let op_start = tok_start;
                let mut end = lx.p;
                if kw == b"BI" {
                    end = skip_inline_image(data, lx.p);
                    lx.p = end;
                }
                if matches!(kw.as_slice(), b"true" | b"false" | b"null") {
                    first.get_or_insert(tok_start);
                    operands.push(Operand::Other);
                    continue;
                }
                ops.push(Op {
                    name: kw,
                    operands: std::mem::take(&mut operands),
                    start: first.take().unwrap_or(op_start),
                    op_start,
                    end,
                });
            }
        }
    }
    Ok(ops)
}

/// Return the end offset (exclusive) of an inline image whose `BI` keyword ended at `p`.
fn skip_inline_image(d: &[u8], p: usize) -> usize {
    // Find "ID" token, then the data, then "EI".
    let mut i = p;
    let mut id_end = None;
    while i + 1 < d.len() {
        if d[i] == b'I'
            && d[i + 1] == b'D'
            && (i == 0 || is_ws(d[i - 1]))
            && d.get(i + 2).is_none_or(|b| is_ws(*b))
        {
            id_end = Some(i + 3);
            break;
        }
        i += 1;
    }
    let Some(mut j) = id_end else { return d.len() };
    while j + 2 < d.len() {
        if d[j] == b'E'
            && d[j + 1] == b'I'
            && is_ws(d[j - 1])
            && d.get(j + 2).is_none_or(|b| is_ws(*b))
        {
            return j + 2;
        }
        j += 1;
    }
    d.len()
}

// -------------------------------------------------------------------------------------
// Simulation
// -------------------------------------------------------------------------------------

/// Font access needed by the simulation.
pub trait FontMetrics {
    /// Split a string operand into character codes.
    fn codes(&self, s: &[u8]) -> Vec<u32>;
    /// Advance width of a code in 1/1000 em.
    fn width1000(&self, code: u32) -> f64;
    /// Whether `Tw` word spacing applies (single-byte code 32).
    fn is_word_space(&self, code: u32) -> bool;
    /// Unicode text of a code, when known.
    fn text_of(&self, _code: u32) -> Option<String> {
        None
    }
    /// Ascent in 1/1000 em (for boxes).
    fn ascent1000(&self) -> f64 {
        800.0
    }
    /// Descent in 1/1000 em, negative (for boxes).
    fn descent1000(&self) -> f64 {
        -200.0
    }
}

/// Text state parameters (part of the graphics state).
#[derive(Clone, Debug)]
pub struct TextParams {
    /// Character spacing.
    pub tc: f64,
    /// Word spacing.
    pub tw: f64,
    /// Horizontal scale (1.0 = 100%).
    pub th: f64,
    /// Leading.
    pub tl: f64,
    /// Font resource name.
    pub font: Vec<u8>,
    /// Font size.
    pub size: f64,
    /// Rendering mode.
    pub render_mode: i64,
    /// Text rise.
    pub rise: f64,
}

impl Default for TextParams {
    fn default() -> Self {
        Self {
            tc: 0.0,
            tw: 0.0,
            th: 1.0,
            tl: 0.0,
            font: Vec::new(),
            size: 0.0,
            render_mode: 0,
            rise: 0.0,
        }
    }
}

/// One element of a text-showing operator.
#[derive(Clone, Debug, PartialEq)]
pub enum RunItem {
    /// String of codes.
    Codes(Vec<u8>),
    /// `TJ` adjustment (thousandths of text size).
    Adjust(f64),
}

/// A located text-showing operator.
#[derive(Clone)]
pub struct Run {
    /// Index of the operator in the scanned list.
    pub op_index: usize,
    /// Byte span of the whole operation (operands + operator) in the scanned buffer.
    pub span: Range<usize>,
    /// `Tj`, `TJ`, `'` or `"`.
    pub operator: String,
    /// Items shown.
    pub items: Vec<RunItem>,
    /// Text parameters in effect.
    pub params: TextParams,
    /// Text matrix at the start of the run.
    pub tm: Mat,
    /// Line matrix at the start of the run (after any ' pre-move).
    pub tlm: Mat,
    /// Text matrix after the run.
    pub tm_end: Mat,
    /// CTM at the start of the run.
    pub ctm: Mat,
    /// Total advance in text space (already includes `Tfs`, `Tc`, `Tw`, `Th`, `TJ`).
    pub advance: f64,
    /// Per code: (code, x start in text space, x end in text space).
    pub glyphs: Vec<(u32, f64, f64)>,
    /// Font metrics used.
    pub font: Option<Arc<dyn FontMetrics>>,
    /// For `'` and `"`: the operation moved to the next line first.
    pub moves_line: bool,
    /// Whether this operator executes inside a `BT … ET`.
    pub in_text_object: bool,
}

impl Run {
    /// Box corners `[bl, br, tr, tl]` in user space.
    pub fn quad(&self) -> [(f64, f64); 4] {
        let (asc, desc) = self
            .font
            .as_ref()
            .map_or((800.0, -200.0), |f| (f.ascent1000(), f.descent1000()));
        let (y0, y1) = (
            self.params.rise + desc / 1000.0 * self.params.size,
            self.params.rise + asc / 1000.0 * self.params.size,
        );
        let m = self.tm.then(&self.ctm);
        let x1 = self.advance;
        [
            m.apply(0.0, y0),
            m.apply(x1, y0),
            m.apply(x1, y1),
            m.apply(0.0, y1),
        ]
    }

    /// Advance in user-space units along the text baseline.
    pub fn advance_user(&self) -> f64 {
        let m = self.tm.then(&self.ctm);
        let (x0, y0) = m.apply(0.0, 0.0);
        let (x1, y1) = m.apply(self.advance, 0.0);
        (x1 - x0).hypot(y1 - y0)
    }

    /// Whether the run draws visible glyphs.
    pub fn visible(&self) -> bool {
        self.params.render_mode != 3 && self.params.render_mode != 7
    }
}

/// A located `Do` of a named XObject.
#[derive(Clone, Debug)]
pub struct DoUse {
    /// Operator index.
    pub op_index: usize,
    /// Byte span of `/Name Do`.
    pub span: Range<usize>,
    /// Resource name.
    pub name: Vec<u8>,
    /// CTM at the call (an image occupies the unit square under it).
    pub ctm: Mat,
}

impl DoUse {
    /// Unit-square corners `[bl, br, tr, tl]` in user space.
    pub fn quad(&self) -> [(f64, f64); 4] {
        [
            self.ctm.apply(0.0, 0.0),
            self.ctm.apply(1.0, 0.0),
            self.ctm.apply(1.0, 1.0),
            self.ctm.apply(0.0, 1.0),
        ]
    }
}

/// Result of simulating a content buffer.
pub struct Walk {
    /// All located text runs in stream order.
    pub runs: Vec<Run>,
    /// All `Do` operators.
    pub dos: Vec<DoUse>,
    /// CTM after the last operator (relevant when appending content).
    pub end_ctm: Mat,
    /// Unbalanced `q` depth at the end of the buffer.
    pub open_q: usize,
    /// Number of constructs that were skipped because they are outside the supported subset.
    pub unsupported: Vec<String>,
}

#[derive(Clone)]
struct Gs {
    ctm: Mat,
    tp: TextParams,
}

/// Simulate `ops` (scanned from `data`). `font_for` resolves a font resource name.
/// Resolver from font resource name to metrics.
pub type FontResolver<'a> = dyn FnMut(&[u8]) -> Option<Arc<dyn FontMetrics>> + 'a;

/// Simulate `ops` (see [`walk`]).
pub fn walk(ops: &[Op], font_for: &mut FontResolver<'_>) -> Walk {
    let mut gs = Gs {
        ctm: Mat::IDENTITY,
        tp: TextParams::default(),
    };
    let mut stack: Vec<Gs> = Vec::new();
    let mut tm = Mat::IDENTITY;
    let mut tlm = Mat::IDENTITY;
    let mut in_bt = false;
    let mut out = Walk {
        runs: Vec::new(),
        dos: Vec::new(),
        end_ctm: Mat::IDENTITY,
        open_q: 0,
        unsupported: Vec::new(),
    };
    let num = |op: &Op, i: usize| op.operands.get(i).and_then(Operand::num);
    for (idx, op) in ops.iter().enumerate() {
        match op.name.as_slice() {
            b"q" => stack.push(gs.clone()),
            b"Q" => {
                if let Some(s) = stack.pop() {
                    gs = s;
                }
            }
            b"cm" => {
                if let [Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)] = [
                    num(op, 0),
                    num(op, 1),
                    num(op, 2),
                    num(op, 3),
                    num(op, 4),
                    num(op, 5),
                ] {
                    gs.ctm = Mat([a, b, c, d, e, f]).then(&gs.ctm);
                }
            }
            b"BT" => {
                in_bt = true;
                tm = Mat::IDENTITY;
                tlm = Mat::IDENTITY;
            }
            b"ET" => in_bt = false,
            b"Tc" => gs.tp.tc = num(op, 0).unwrap_or(0.0),
            b"Tw" => gs.tp.tw = num(op, 0).unwrap_or(0.0),
            b"Tz" => gs.tp.th = num(op, 0).unwrap_or(100.0) / 100.0,
            b"TL" => gs.tp.tl = num(op, 0).unwrap_or(0.0),
            b"Ts" => gs.tp.rise = num(op, 0).unwrap_or(0.0),
            b"Tr" => gs.tp.render_mode = num(op, 0).unwrap_or(0.0) as i64,
            b"Tf" => {
                if let Some(Operand::Name(n)) = op.operands.first() {
                    gs.tp.font = n.clone();
                }
                gs.tp.size = num(op, 1).unwrap_or(gs.tp.size);
            }
            b"Td" => {
                if let (Some(x), Some(y)) = (num(op, 0), num(op, 1)) {
                    tlm = Mat::translate(x, y).then(&tlm);
                    tm = tlm;
                }
            }
            b"TD" => {
                if let (Some(x), Some(y)) = (num(op, 0), num(op, 1)) {
                    gs.tp.tl = -y;
                    tlm = Mat::translate(x, y).then(&tlm);
                    tm = tlm;
                }
            }
            b"T*" => {
                tlm = Mat::translate(0.0, -gs.tp.tl).then(&tlm);
                tm = tlm;
            }
            b"Tm" => {
                if let [Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)] = [
                    num(op, 0),
                    num(op, 1),
                    num(op, 2),
                    num(op, 3),
                    num(op, 4),
                    num(op, 5),
                ] {
                    tlm = Mat([a, b, c, d, e, f]);
                    tm = tlm;
                }
            }
            b"Tj" | b"TJ" | b"'" | b"\"" => {
                let name = op.name_str().to_string();
                let mut moves_line = false;
                match op.name.as_slice() {
                    b"'" => {
                        tlm = Mat::translate(0.0, -gs.tp.tl).then(&tlm);
                        tm = tlm;
                        moves_line = true;
                    }
                    b"\"" => {
                        if let (Some(aw), Some(ac)) = (num(op, 0), num(op, 1)) {
                            gs.tp.tw = aw;
                            gs.tp.tc = ac;
                        }
                        tlm = Mat::translate(0.0, -gs.tp.tl).then(&tlm);
                        tm = tlm;
                        moves_line = true;
                    }
                    _ => {}
                }
                let items: Vec<RunItem> = match op.operands.last() {
                    Some(Operand::Str(s)) => vec![RunItem::Codes(s.clone())],
                    Some(Operand::Array(a)) => a
                        .iter()
                        .filter_map(|o| match o {
                            Operand::Str(s) => Some(RunItem::Codes(s.clone())),
                            Operand::Num(n) => Some(RunItem::Adjust(*n)),
                            _ => None,
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                let font = font_for(&gs.tp.font);
                let start_tm = tm;
                let mut x = 0.0;
                let mut glyphs = Vec::new();
                for it in &items {
                    match it {
                        RunItem::Adjust(n) => x += -n / 1000.0 * gs.tp.size * gs.tp.th,
                        RunItem::Codes(s) => {
                            if let Some(f) = &font {
                                for code in f.codes(s) {
                                    let w = f.width1000(code) / 1000.0;
                                    let ws = if f.is_word_space(code) { gs.tp.tw } else { 0.0 };
                                    let adv = (w * gs.tp.size + gs.tp.tc + ws) * gs.tp.th;
                                    glyphs.push((code, x, x + adv));
                                    x += adv;
                                }
                            }
                        }
                    }
                }
                if font.is_none() {
                    out.unsupported.push(format!(
                        "font /{} is not available",
                        String::from_utf8_lossy(&gs.tp.font)
                    ));
                }
                if !in_bt {
                    out.unsupported.push("text shown outside BT/ET".into());
                }
                tm = Mat::translate(x, 0.0).then(&tm);
                out.runs.push(Run {
                    tlm,
                    tm_end: tm,
                    op_index: idx,
                    span: op.span(),
                    operator: name,
                    items,
                    params: gs.tp.clone(),
                    tm: start_tm,
                    ctm: gs.ctm,
                    advance: x,
                    glyphs,
                    font,
                    moves_line,
                    in_text_object: in_bt,
                });
            }
            b"Do" => {
                if let Some(Operand::Name(n)) = op.operands.first() {
                    out.dos.push(DoUse {
                        op_index: idx,
                        span: op.span(),
                        name: n.clone(),
                        ctm: gs.ctm,
                    });
                }
            }
            b"gs" => {
                // ExtGState may carry /Font; we do not follow it.
            }
            _ => {}
        }
    }
    out.end_ctm = gs.ctm;
    out.open_q = stack.len();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Mono;
    impl FontMetrics for Mono {
        fn codes(&self, s: &[u8]) -> Vec<u32> {
            s.iter().map(|b| u32::from(*b)).collect()
        }
        fn width1000(&self, _: u32) -> f64 {
            500.0
        }
        fn is_word_space(&self, c: u32) -> bool {
            c == 32
        }
    }

    fn font(_: &[u8]) -> Option<Arc<dyn FontMetrics>> {
        Some(Arc::new(Mono))
    }

    #[test]
    fn matrix_algebra() {
        let t = Mat::translate(10.0, 20.0);
        let s = Mat::scale(2.0, 3.0);
        // translate then scale: (1,1) -> (11,21) -> (22,63)
        assert_eq!(t.then(&s).apply(1.0, 1.0), (22.0, 63.0));
        let m = Mat([1.0, 0.5, -0.5, 2.0, 7.0, -3.0]);
        let i = m.inverse().unwrap();
        let (x, y) = m.then(&i).apply(4.0, 9.0);
        assert!((x - 4.0).abs() < 1e-9 && (y - 9.0).abs() < 1e-9);
        assert!(Mat([0.0, 0.0, 0.0, 0.0, 1.0, 1.0]).inverse().is_none());
    }

    #[test]
    fn conjugation_applies_a_user_space_move_after_the_objects_own_transform() {
        // Object drawn under a page flip + its own scale; move it by (+5, +7) in *outer* space.
        let ctm = Mat([2.0, 0.0, 0.0, -2.0, 10.0, 400.0]);
        let a = ctm.conjugate(&Mat::translate(5.0, 7.0)).unwrap();
        // The defining property: a·ctm == ctm·T  (row-vector convention).
        let lhs = a.then(&ctm);
        let rhs = ctm.then(&Mat::translate(5.0, 7.0));
        for i in 0..6 {
            assert!((lhs.0[i] - rhs.0[i]).abs() < 1e-9, "{lhs:?} vs {rhs:?}");
        }
    }

    #[test]
    fn lexer_spans_strings_names_arrays_comments() {
        let src = b"q 1 0 0 1 10 20 cm % comment (x\nBT /F1 12 Tf [(A\\)B) -20 <4142>] TJ (a(b)c) Tj ET Q";
        let ops = scan(src).unwrap();
        let names: Vec<&str> = ops.iter().map(|o| o.name_str()).collect();
        assert_eq!(names, ["q", "cm", "BT", "Tf", "TJ", "Tj", "ET", "Q"]);
        let tj = &ops[4];
        assert_eq!(&src[tj.span()], b"[(A\\)B) -20 <4142>] TJ");
        assert_eq!(
            tj.operands[0],
            Operand::Array(vec![
                Operand::Str(b"A)B".to_vec()),
                Operand::Num(-20.0),
                Operand::Str(b"AB".to_vec())
            ])
        );
        assert_eq!(ops[5].operands[0], Operand::Str(b"a(b)c".to_vec()));
        assert_eq!(&src[ops[1].span()], b"1 0 0 1 10 20 cm");
    }

    #[test]
    fn inline_images_are_skipped_not_misparsed() {
        let src = b"BI /W 2 /H 1 /BPC 8 /CS /G ID \x00\x01 EI\nBT (x) Tj ET";
        let ops = scan(src).unwrap();
        assert_eq!(ops[0].name_str(), "BI");
        assert_eq!(
            ops.iter().map(Op::name_str).collect::<Vec<_>>(),
            ["BI", "BT", "Tj", "ET"]
        );
    }

    #[test]
    fn simulation_tracks_position_and_advances() {
        let src = b"1 0 0 -1 0 800 cm BT /F1 10 Tf 100 700 Td (Hello) Tj [(W) 500 (o)] TJ ET";
        let ops = scan(src).unwrap();
        let w = walk(&ops, &mut font);
        assert_eq!(w.runs.len(), 2);
        let r0 = &w.runs[0];
        // 5 glyphs × 0.5 em × 10 pt = 25
        assert!((r0.advance - 25.0).abs() < 1e-9);
        let q = r0.quad();
        // Page flip: user y = 800 - 700 = 100 at the baseline origin.
        let (ox, oy) = r0.tm.then(&r0.ctm).apply(0.0, 0.0);
        assert!((ox - 100.0).abs() < 1e-9 && (oy - 100.0).abs() < 1e-9);
        assert!(q[0].0 < q[1].0 && (q[1].0 - q[0].0 - 25.0).abs() < 1e-9);
        // Second run starts where the first ended; TJ adjust 500 => -5 pt.
        let r1 = &w.runs[1];
        assert!((r1.tm.0[4] - 125.0).abs() < 1e-9, "{}", r1.tm.0[4]);
        assert!((r1.advance - (5.0 + 5.0 - 5.0)).abs() < 1e-9);
    }

    #[test]
    fn q_restores_ctm_but_not_text_matrix_and_spacing_applies() {
        let src = b"q 2 0 0 2 0 0 cm Q BT /F1 10 Tf 3 Tc 2 Tw 0 0 Td (a b) Tj ET";
        let ops = scan(src).unwrap();
        let w = walk(&ops, &mut font);
        assert_eq!(w.runs[0].ctm, Mat::IDENTITY);
        // a, space, b : 3 × (5 + 3) + 2 word space = 26
        assert!(
            (w.runs[0].advance - 26.0).abs() < 1e-9,
            "{}",
            w.runs[0].advance
        );
        assert_eq!(w.open_q, 0);
    }

    #[test]
    fn do_operators_are_located_with_their_ctm() {
        let src = b"q 100 0 0 50 30 40 cm /Im1 Do Q";
        let ops = scan(src).unwrap();
        let w = walk(&ops, &mut font);
        assert_eq!(w.dos.len(), 1);
        assert_eq!(w.dos[0].name, b"Im1");
        let q = w.dos[0].quad();
        assert_eq!(q[0], (30.0, 40.0));
        assert_eq!(q[2], (130.0, 90.0));
    }

    #[test]
    fn garbage_does_not_hang_or_panic() {
        for src in [
            &b"((((("[..],
            b"<<<<<<",
            b"[[[[[[[[",
            b"BI ID",
            b")))>>>}}}",
            b"/",
            b"1.2.3 -.5 +7 Tj",
            b"\xff\xfe\x00BT",
        ] {
            let _ = scan(src);
        }
        let deep = vec![b'['; 10_000];
        assert!(scan(&deep).is_err());
    }
}

// -------------------------------------------------------------------------------------
// Grouping
// -------------------------------------------------------------------------------------

/// A user-editable text run: consecutive show operators on one baseline with identical text
/// state (a word, a phrase or a line — whatever the producer emitted contiguously).
#[derive(Clone, Debug)]
pub struct TextGroup {
    /// Range of indices into [`Walk::runs`].
    pub runs: Range<usize>,
    /// Range of operator indices covered (first show op ..= last show op).
    pub ops: Range<usize>,
    /// Byte span in the scanned buffer.
    pub span: Range<usize>,
}

fn same_params(a: &TextParams, b: &TextParams) -> bool {
    a.font == b.font
        && (a.size - b.size).abs() < 1e-9
        && (a.tc - b.tc).abs() < 1e-9
        && (a.tw - b.tw).abs() < 1e-9
        && (a.th - b.th).abs() < 1e-9
        && (a.rise - b.rise).abs() < 1e-9
        && a.render_mode == b.render_mode
}

/// Group consecutive compatible show operators.
pub fn group_runs(w: &Walk, ops: &[Op]) -> Vec<TextGroup> {
    let mut groups: Vec<TextGroup> = Vec::new();
    for (k, r) in w.runs.iter().enumerate() {
        let extend = groups.last().is_some_and(|g| {
            let prev = &w.runs[g.runs.end - 1];
            if !(matches!(prev.operator.as_str(), "Tj" | "TJ")
                && matches!(r.operator.as_str(), "Tj" | "TJ"))
            {
                return false;
            }
            if !(prev.in_text_object && r.in_text_object) || !same_params(&prev.params, &r.params) {
                return false;
            }
            // Only positioning (and redundant Tf) operators may sit between the two.
            let between = &ops[prev.op_index + 1..r.op_index];
            if !between
                .iter()
                .all(|o| matches!(o.name.as_slice(), b"Td" | b"TD" | b"Tm" | b"T*" | b"Tf"))
            {
                return false;
            }
            let (m0, m1) = (prev.tm.then(&prev.ctm), r.tm.then(&r.ctm));
            if (0..4).any(|i| (m0.0[i] - m1.0[i]).abs() > 1e-6 * (1.0 + m0.0[i].abs())) {
                return false;
            }
            let Some(inv) = m0.inverse() else {
                return false;
            };
            let (ox, oy) = m1.apply(0.0, 0.0);
            let (lx, ly) = inv.apply(ox, oy);
            let size = prev.params.size.abs().max(1e-6);
            let gap = lx - prev.advance;
            ly.abs() < 0.3 * size && gap > -0.5 * size && gap < 0.6 * size
        });
        if extend {
            if let Some(g) = groups.last_mut() {
                g.runs.end = k + 1;
                g.ops.end = r.op_index + 1;
                g.span.end = r.span.end;
            }
        } else {
            groups.push(TextGroup {
                runs: k..k + 1,
                ops: r.op_index..r.op_index + 1,
                span: r.span.clone(),
            });
        }
    }
    groups
}

#[cfg(test)]
mod group_tests {
    use super::*;

    struct Mono;
    impl FontMetrics for Mono {
        fn codes(&self, s: &[u8]) -> Vec<u32> {
            s.iter().map(|b| u32::from(*b)).collect()
        }
        fn width1000(&self, _: u32) -> f64 {
            500.0
        }
        fn is_word_space(&self, _: u32) -> bool {
            false
        }
    }
    fn font(_: &[u8]) -> Option<Arc<dyn FontMetrics>> {
        Some(Arc::new(Mono))
    }

    #[test]
    fn per_glyph_td_runs_merge_into_one_group_but_other_state_splits_them() {
        // Chromium style: one Tj per glyph, positioned with Td.
        let src = b"BT /F1 10 Tf 1 0 0 1 50 700 Tm (H) Tj 5 0 Td (i) Tj 5 0 Td (!) Tj ET BT /F1 10 Tf 1 0 0 1 50 650 Tm (a) Tj ET BT /F1 10 Tf 1 0 0 1 50 600 Tm (x) Tj 0 1 0 rg (y) Tj /F1 12 Tf (z) Tj ET";
        let ops = scan(src).unwrap();
        let w = walk(&ops, &mut font);
        let g = group_runs(&w, &ops);
        assert_eq!(g.len(), 5, "{g:?}");
        assert_eq!(g[0].runs, 0..3);
        assert_eq!(
            &src[g[0].span.clone()],
            b"(H) Tj 5 0 Td (i) Tj 5 0 Td (!) Tj"
        );
        // The colour change (x|y) and the size change (y|z) each start a new group.
        assert!(g[2..].iter().all(|x| x.runs.len() == 1));
    }

    #[test]
    fn far_apart_runs_on_a_line_are_separate_groups() {
        let src = b"BT /F1 10 Tf 1 0 0 1 50 700 Tm (A) Tj 200 0 Td (B) Tj ET BT /F1 10 Tf 1 0 0 1 50 650 Tm (A) Tj 0 -12 Td (B) Tj ET";
        let ops = scan(src).unwrap();
        let w = walk(&ops, &mut font);
        let g = group_runs(&w, &ops);
        assert_eq!(g.len(), 4);
    }

    #[test]
    fn end_matrices_are_recorded() {
        let src = b"BT /F1 10 Tf 1 0 0 1 50 700 Tm (AB) Tj ET";
        let ops = scan(src).unwrap();
        let w = walk(&ops, &mut font);
        assert_eq!(w.runs[0].tlm, Mat([1.0, 0.0, 0.0, 1.0, 50.0, 700.0]));
        assert_eq!(w.runs[0].tm_end, Mat([1.0, 0.0, 0.0, 1.0, 60.0, 700.0]));
    }
}
