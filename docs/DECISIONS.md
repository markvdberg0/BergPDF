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

## D-012 — No telemetry, no network access in core flows (amended by D-020)
There is no telemetry and the core flows (open, edit, save, sign, OCR, optimise, convert) make no
network requests. External links open only after a confirmation dialog and only for allow-listed URI
schemes (`platform::links`). **Amendment (2026-10-05):** the original text said the dependency graph
had no HTTP client. At the owner's request there is now exactly one, confined to the `ai-client` crate
and used only by PDF Copilot / Translate in answer to a button press (D-020).

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

## D-017 — Snapping uses the page's own vector geometry, built off the UI thread
Content streams and Form XObjects are walked once per page and revision (`pdf_engine::snap`); a grid
index answers "best target within N pixels" in microseconds. Priority: end points/corners, then
intersections, midpoints, then nearest point on a line or curve. Curves are flattened for the last case
only (their interior is never an end point); clip-only paths are ignored. The index is built on a
worker thread so a huge drawing cannot freeze the window (cap 600,000 segments). Snapping is a
preference (on by default) and a ribbon toggle. Not snapped: circle centres, text outlines, raster images.

## D-018 — "Save As Optimized" rewrites a private copy and never touches the open file
Steps (each can only shrink or leave alone): drop thumbnails, optional image downsampling by *effective*
resolution where placed, merge identical streams, recompress Flate at the best level, prune unreferenced
objects, write object + xref streams, validate by re-reading; if the result is not smaller the input is
returned unchanged and the dialog says so. Three presets (lossless; 150 dpi/JPEG 75; 96 dpi/JPEG 60);
"balanced" is the default and the lossy ones say they are permanent in the copy. Only 8-bit Gray/RGB
JPEG and plain Flate images are resampled; photographs become JPEG, few-colour art stays Flate.
Digital signatures do not survive (the file is rewritten) and the dialog warns.

## D-019 — PDF/A-2b: repair what is safe, refuse what would change the document
Done automatically: file identifier, XMP (`pdfaid` 2/B, consistent with Info), generated sRGB ICC output
intent (no third-party profile is redistributed), PDF 1.7, annotation flags, removal of JavaScript /
launch / sound / attachments / XFA / transfer functions / external-file stream keys, explicit
`/CIDToGIDMap /Identity`. **Refused with the reason**: fonts that are not embedded, CMYK without a
CMYK output intent, annotations without appearance streams, encryption. BergPDF never substitutes a font
or converts colours to force a pass. The application carries no validator; the *tests* validate with
veraPDF 1.28 (an independent implementation) and the UI says "prepared as PDF/A-2b", never "certified".

## D-020 — AI features (PDF Copilot, Translate) are opt-in, user-keyed and isolated
* **Network**: only `crates/ai-client` talks to the network (ureq + rustls, certificates from the
  operating-system store so corporate proxies work). Calls happen on worker threads, only after the
  person pressed a button. OpenAI/Anthropic must be https; plain http is accepted only for the *Custom*
  provider and loopback so a local model server keeps everything on the machine.
* **Consent**: the first use per provider shows what is sent and where; it can be withdrawn.
* **Key**: entered in Preferences, stored in its own file (D-021), never in preferences, never in a URL,
  log line or error message (`scrub`, unit-tested).
* **Models**: no model is bundled or recommended in code beyond defaults; the model name is a setting.
  Anthropic default `claude-opus-5-5`, OpenAI default `gpt-4.1-mini`. No sampling parameters, no
  prefill, no forced tool use (rejected by current models); plain messages only.
* **Answers are checked, not trusted**: the model must quote verbatim; quotes that are not found in the
  extracted page text are flagged "not found in the document" and cannot be highlighted; quotes found on
  another page have the page corrected. The document text is labelled DATA in the system prompt (prompt
  injection hardening is a mitigation, not a guarantee).
* **Scope of what is sent**: question → page text up to a configurable character budget (default 60,000);
  Explain → selected text plus neighbouring pages; Translate → the chosen pages' text; Test connection →
  a one-word ping. Translation is saved as a *new* text PDF in the built-in font; layout is not reproduced.
* **Language detection** runs locally (`whatlang`), before anything is sent.

## D-021 — API key storage: a plain file with owner-only permissions (for now)
`platform::secrets` writes `<config dir>/BergPDF/ai-key` (mode 0600 on Unix; inherits the profile folder's
ACL on Windows), or reads `BERGPDF_AI_KEY`. It is **not encrypted**; the Preferences text says so and
advises a provider-side spending limit. An OS key store (Keychain / Credential Manager / Secret Service)
is the better design but needs per-platform code that cannot be exercised here; it is on the roadmap.

## D-022 — More fonts for text: five bundled families, chosen per text, remembered per annotation
Text is still never drawn from system fonts (so a document looks the same everywhere and nothing is
fetched): five bundled families (Liberation Sans/Serif/Mono, DejaVu Sans/Serif) with bold and, where the
family has it, italic. The choice applies to Add Text, text boxes/callouts (the box's font is stored in
the annotation as `/BergFont`, so it survives saving; before this, a text box's *bold* was lost on
re-open) and to *replacing* the font of one existing run in Edit Text (an explicit, labelled change; the
text keeps its position, there is still no reflow). Editing an existing run without choosing a font still
keeps its original font. The last choice becomes the default for new text (Preferences ▸ Default font).
Not offered: system fonts, font upload, per-character formatting inside one run, CJK (no bundled font has it —
text with such characters is refused with the font named, never silently replaced).
