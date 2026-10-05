# Security notes

Scope: a desktop application that opens **untrusted PDF files**. This is a design record, not an
audit. No external security review and no fuzzing campaign has been done (see “Not done”).

## What the application never does

* **Runs no document code.** No JavaScript, no XFA, no form calculations, no launch/URI/import
  actions on its own. Such content is *detected* and reported when a file opens (`caps.rs`,
  `forms::read_form`), never executed.
* **Makes no network requests** in core flows and has **no telemetry**. The runtime dependency
  graph contains no HTTP client (`cargo tree` shows none of reqwest/hyper/ureq/curl/tokio).
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

## Known weaknesses / not done

* No process isolation (above). No fuzzing of `lopdf`, `hayro` or `content.rs`. No memory
  soak test. No third-party review.
* `cargo deny` / `cargo audit` are configured for CI; see `EVIDENCE.md` for what was runnable
  in the authoring sandbox.
* Encrypted PDFs are rejected (no decryption), so permissions flags are not honoured/relevant.
* **Redaction is not offered.** Drawing a black rectangle does not remove text underneath.
* Signed documents: structure is detected and the user is warned that editing invalidates the
  signature; signatures are **not validated**.
