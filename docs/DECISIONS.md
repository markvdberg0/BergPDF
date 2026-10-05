# Decision log

Each entry: decision, why, what would change it. Open decisions are marked **OPEN**.

## D-001 — Project licence — **OPEN (owner’s decision)**
`Cargo.toml` uses `license = "LicenseRef-Undecided"` and `publish = false`. No licence file has
been added on purpose: choosing one grants rights and is the repository owner’s call. All
dependencies are permissive or dual-licensed with a permissive option (`DEPENDENCIES.md`), so
MIT/Apache-2.0, a proprietary licence, or a copyleft licence are all feasible for the
application itself. `deny.toml` encodes what dependencies may use.

## D-002 — Rust only at runtime; egui/eframe + wgpu for UI
Why: one language/toolchain, no webview/Electron attack surface or footprint, GPU-composited
canvas, immediate-mode UI is quick to iterate. Cost: no native widgets/IME polish/accessibility
parity; accepted for now and recorded as a risk. Revisit if the accessibility audit
(`PLATFORM_CHECKLIST.md`) fails badly.

## D-003 — `hayro` for rendering and text, `lopdf` for editing
Why: both pure Rust. `hayro` renders and exposes glyph geometry; `lopdf` gives a mutable object
model. They parse the same bytes independently, so the application renders from an immutable
byte snapshot of each revision and never shares object state between them. Cost: two parsers
(more memory, rare disagreements). `lopdf`’s crypto features are disabled → no encrypted PDFs.

## D-004 — Edits are transactions with object-level before/after images; saving is incremental
Why: precise undo/redo, original bytes preserved (signature-friendly, small saves, easy to
verify as a prefix), and corrupt-write safety by re-opening the written bytes before renaming.
Cost: each revision snapshot copies the original byte buffer (memory grows with revisions of a
huge file) — listed as a known limitation.

## D-005 — `pdf-writer` is used only to build test fixtures
Different author/library from the code under test, so a fixture cannot encode the same bug as
the reader/writer being tested. Two real third-party producer files (Chromium/Skia, Cairo) are
checked in as well.

## D-006 — Content editing is per text run, in place, or it refuses and says why
No overlay-patching. If a run uses a font we cannot re-encode (unknown encoding, no glyph),
we report the characters/fonts and offer a deliberate “use substitute font” action rather than
silently changing typography. Reflow is out of scope for now.

## D-007 — Form copies: Independent (default) / Linked / Flatten
Duplicating a form page is ambiguous, so the user chooses, with consequences spelled out.
Independent renames fields (`name` → `name_2`), keeping radio groups intact as one new group;
Linked shares one field across widgets (same document only); Flatten bakes appearances in and
removes the fields on the copy. Cross-document operations refuse Linked.

## D-008 — Measurements are standard annotations plus a private extension
Each measurement carries `/Measure` (RL) so other viewers can interpret it, and a private
`/BergMeasure` dictionary with exact kind/category/scale. Scale factors are stored as decimal
**text** because PDF reals are f32 in the library (found by a failing test: 10.000008 instead
of 10). The scale registry lives in a private catalog entry `/BergScales`.

## D-009 — Redaction stays gated; OCR and signing were opened on request
A visible black box is not redaction. Until true content removal can be shown by an
independent extractor on adversarial fixtures, no UI or command claims redaction. OCR and
signing were originally gated; the owner asked for both (2026-10-05) and they are now implemented
under D-014 and D-015.

## D-010 — Bundled font for new text and form appearances: DejaVu Sans / Bold (subset-embedded)
Chosen for broad Latin/Greek/Cyrillic coverage and a permissive licence; copied from the Debian
`fonts-dejavu-core` package. **Needs owner/legal review** of the Bitstream Vera licence terms
for embedding subsets in user documents (`DEPENDENCIES.md`). Characters outside it are refused
with a clear message instead of rendering tofu.

## D-011 — Recovery files are only offered when old enough to be a crash
A live instance rewrites its recovery file every 30 s while a document is dirty, so files younger
than 90 s belong to another running instance. Tokens include PID and start time so a new
instance never overwrites or deletes a crashed instance’s file before the user chooses.
Trade-off: a second crash within 90 s of a restart is only offered on the next launch.

## D-012 — No telemetry, no network access in core flows
There is no HTTP client in the runtime dependency graph. External links open only after a
confirmation dialog and only for allow-listed URI schemes (`platform::links`).

## D-013 — Name and logo: “BergPDF”, a Mont Blanc outline
The owner’s surname means “mountain”. The logo is plain vector data in the `brand` crate (outline of
the Mont Blanc massif with a copper snow line), drawn by the UI painter and rasterised by a tiny
built-in rasteriser for the window icon; `cargo xtask icons` writes PNG/ICO/ICNS into
`assets/icons`. “BergPDF” has **not** been cleared as a product name (trademark search pending).
Internal private PDF keys were renamed with the product (`/BergMeasure`, `/BergScales`); no files
using the old names were ever released.

## D-014 — Digital signatures: PKCS#7 detached, pure-Rust crypto, integrity-only checking
Signing = append a revision with a signature field, patch `/ByteRange` and the reserved `/Contents`
space in the serialised bytes, embed a CMS blob built with RustCrypto (`pdf-sign`). Input is a
PKCS#12 file the user picks; RSA and ECDSA P-256 with SHA-256. Chosen over OS key stores and
hardware tokens because those are platform-specific and could not be tested here. We verify our own
output with two independent tools (poppler `pdfsig`, OpenSSL). Checking a signature reports only
what BergPDF can establish (bytes unchanged, signature maths valid, file extended or not); it never
says "trusted". Signing clears the undo history. A handwritten signature is a separate, clearly
labelled picture (ink annotation). Known advisory on `rsa` accepted with a written reason.

## D-015 — OCR: `ocrs` on CPU, models fetched on demand, invisible text layer
Pure-Rust engine (no C++/Tesseract), 300 dpi render → detect → recognise → map boxes to user space →
render-mode-3 text with the bundled font (stretched to the word box with `Tz`) so search/copy work
and the page looks identical. Models are *not* in the repo or binary: the trained weights come from
a CC BY-SA 4.0 dataset and their redistribution terms are unverified; offline use is preserved by
an explicit, checksum-verified `cargo xtask fetch-ocr-models`. Known limits: ASCII alphabet (Dutch
accents are lost), no handwriting, no rotated text. Pages that already have text are skipped unless
the user insists, so OCR is never layered twice by accident.

## D-016 — Pictures open as PDFs
A PNG/JPEG becomes a one-page document sized to A4's long side with its own aspect ratio, pixels
embedded unchanged, EXIF rotation applied via `/Rotate`. The document is "new" (dirty, no path) so
Save asks for a PDF name. Other formats are not supported rather than half-supported.
