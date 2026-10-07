# Security notes

Scope: a desktop application that opens **untrusted PDF files**. This is a design record, not an
audit. No external security review and no fuzzing campaign has been done (see “Not done”).

## What the application never does

* **Runs no document code.** No JavaScript, no XFA, no form calculations, no launch/URI/import
  actions on its own. Such content is *detected* and reported when a file opens (`caps.rs`,
  `forms::read_form`), never executed.
* **Makes no network requests** in core flows and has **no telemetry**. The one exception is the
  optional AI features (PDF Copilot, Translate), which exist only in the `ai-client` crate, run only
  after a button press and a one-time consent per provider, and use the user's own key (see D-020 /
  D-021), the OCR model download (button or installer question, pinned SHA-256) and the update check (off until the user says yes, or pressed by hand; D-033: one GET to the GitHub releases API, only a link is shown, nothing is downloaded or run). `cargo tree -p ai-client` shows the whole network stack; nothing else links it.
  On Linux, `rfd` talks to the desktop portal over local D-Bus.
* **Opens external links only after explicit confirmation**, only for `http`, `https`, `mailto`,
  without control characters, under 2048 bytes (`platform::links`, unit-tested).
* **Never overwrites a file before the new bytes are verified.** Save = write temp file in the
  same directory → fsync → re-open and validate the written bytes → rename. A failed save leaves
  the original intact (`editor_core::session::failed_save_keeps_the_original_intact`).
  A file changed on disk since it was opened is not silently overwritten.

## Resource limits (untrusted input)

| Surface | Limit | Where |
|---|---|---|
| File size accepted | 2 GiB | `OpenOptions::max_file_size` |
| Per-stream decompression while loading | 256 MiB | `OpenOptions::max_decompressed_size` |
| Content stream decompression for editing | 128 MiB | `pagecontent::MAX_CONTENT_BYTES` |
| Content operators scanned | 4,000,000, nesting 64 | `content.rs` |
| Rendered tile | 4096² px, cache byte-bounded (LRU) | `render.rs`, `tiles::ByteLru` |
| Pending render jobs | 256, cancellable by priority/revision | `jobs.rs` |
| Image decode | 16,384 px per side, 120 M pixels, checked from the header *before* decoding | `imageembed.rs` |
| Form fields / hierarchy depth | 100,000 / 32 | `forms.rs` |
| Object-graph import depth | 512 | `pageops::Importer` |
| Measurement vertices | 10,000 | `measure.rs` |
| Recovery files | 24 files, 2 GiB total, owner-only permissions on Unix | `platform::recovery` |

## Isolation

Rendering and text extraction run on a pool of worker **threads** fed immutable byte snapshots;
a panic in a job is caught and reported for that job (`jobs` tests). This is *not* process
isolation: an out-of-memory condition, stack overflow or `abort` inside a dependency still ends
the application. A sandboxed worker process (with job objects on Windows, App Sandbox/seatbelt
on macOS, rlimits/seccomp on Linux) is the intended next step and is **not implemented**.
`unsafe` is forbidden in all first-party crates (`[workspace.lints.rust] unsafe_code =
"forbid"`).

## Data handling

* Recovery files contain document contents: private permissions where the OS supports them,
  never logged, bounded, deleted on clean save/close.
* Log output is local and does not include document text.
* CSV export neutralises spreadsheet formula injection (`=`, `+`, `-`, `@`, tab, CR prefixes)
  and quotes fields (`measure::csv_field`, unit-tested).
* Form fill and text-add refuse characters they cannot render rather than substituting silently.

## Signing, OCR and pictures (added 2026-10-05)

* **Signing.** Certificate files are read only when the user picks them (rejected above 2 MB), the
  password lives in the dialog state and is cleared when it closes, nothing is stored or logged. The
  document is only written after the signature is computed, via the same atomic write+validate path as
  Save; a failed write leaves the original untouched. Signing clears undo history because the file on
  disk is the signed state. Signature checking never presents a signature as "trusted".
* **OCR.** Runs on a background thread over an immutable snapshot (cancellable). Model files are
  loaded from a user-controlled folder (`BERG_OCR_MODELS` or the data directory) and a malformed model
  could in principle exercise bugs in the `rten` model loader — they are verified by SHA-256 only when
  fetched with `cargo xtask fetch-ocr-models`, not on every load. Rendering for OCR is bounded
  (≤ 16 M pixels per page; 40 M-pixel guard in `pdf-ocr`).
* **Pictures.** Opened pictures go through the same header-first size limits as inserted images.
  JPEG data is embedded unchanged; EXIF is read only for the orientation tag (bounds-checked parser).
* The handwritten-signature file in the config directory is treated as untrusted on load (point
  and stroke counts and coordinate ranges are clamped).

## Known weaknesses / not done

* No process isolation (above). No fuzzing of `lopdf`, `hayro` or `content.rs`. No memory
  soak test. No third-party review.
* `cargo deny` / `cargo audit` are configured for CI; see `EVIDENCE.md` for what was runnable
  in the authoring sandbox.
* **Password-protected PDFs** are decrypted into a plain working copy in memory (D-035). Protected documents are never
  written to recovery files and the operations that would write an unprotected copy are refused. The permission flags
  are honoured by BergPDF only; other software may ignore them, so they are a courtesy, not protection.
* **Redaction** (D-036) replaces each marked page by a picture with the marks burned into the pixels, so nothing under a
  mark stays in the page; the file is rewritten completely (no old revision) and the report lists other places that still
  show a removed word. It does not remove information a person typed elsewhere (file names, other documents) and has not
  been reviewed by a third party. Drawing a black rectangle *annotation* is still not redaction; only *Apply Redactions* is.
* **`unsafe` code** exists in exactly one place: `crates/printing/src/windows.rs` (the GDI printing API).
* Signed documents: integrity is checked on request (digest + signature maths with the embedded
  certificate). Certificate trust, validity period, revocation, DocMDP permissions and signing time
  are **not** evaluated.

## AI features: what to know

* Text from your document leaves the machine when you press Send/Summarize/Explain/Translate, to the
  provider you configured. For a server on your own computer pick the *Custom* provider.
* The API key is stored unencrypted in a file only your account can read (Unix) / in your profile folder
  (Windows). Use a key with a spending limit. It is never written to preferences, logs or error text.
* Documents are untrusted input to the model: the prompt marks them as data, and every quotation in an
  answer is verified against the real page text before it can be highlighted. A model can still be
  misled; treat answers as drafts.
* Redirects/proxies: system proxy variables are honoured; HTTPS only for OpenAI/Anthropic.
