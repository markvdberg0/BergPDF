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
`/FerrumMeasure` dictionary with exact kind/category/scale. Scale factors are stored as decimal
**text** because PDF reals are f32 in the library (found by a failing test: 10.000008 instead
of 10). The scale registry lives in a private catalog entry `/FerrumScales`.

## D-009 — Redaction, OCR and signatures are separate gates and are *not* offered
A visible black box is not redaction. Until true content removal can be shown by an
independent extractor on adversarial fixtures, no UI or command claims redaction. OCR and
signing need engine/licence/key-handling decisions of their own.

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
