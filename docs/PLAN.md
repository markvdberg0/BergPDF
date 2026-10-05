# BergPDF — plan, status and evidence

The product name “BergPDF” has not been checked against existing trademarks.

Goal: a native, offline, Rust-only desktop PDF editor in the class of PDF-XChange Editor, for
Windows 11 x86_64 and macOS Apple Silicon, with an original look and feel.

This file records what each milestone delivered, **how that was checked**, and what is left.
Where something was only checked on Linux or only by reading code, it says so.

## Environment the work was done in

* A Linux (x86_64, 4 cores) cloud sandbox. **No Windows machine, no Mac.** Everything under
  “tested” below ran on Linux.
* Rust 1.97.0 (pinned in `rust-toolchain.toml`), edition 2024.
* GUI checks: `Xvfb` + Mesa lavapipe (software Vulkan) so eframe/wgpu really created a window;
  interaction by `xdotool`; screenshots saved and looked at. This proves the UI draws and
  responds on Linux/X11 with a software rasteriser — nothing about GPU drivers, HiDPI, IME,
  Windows or macOS.
* Independent oracles: poppler (`pdftotext`, `pdftoppm`, `pdfinfo`, `pdffonts`, `pdftocairo`).
  Used to read back and render files the application wrote.

## Architecture in one paragraph

Cargo workspace. `pdf-engine` (no UI): parsing/editing with `lopdf`, rendering/text with
`hayro`, transactions with object-level undo images, incremental writer, content-stream editing,
annotations, page operations, forms, measurements. `editor-core` (no UI toolkit): document
session, history, commands/shortcuts, view model, tile planning, render job pool, preferences.
`platform`: file dialogs, recovery files, directories. `apps/desktop`: egui/eframe + wgpu shell.
`test-support`: fixtures (generated with `pdf-writer`, a different library from the one the app
edits with) and oracle helpers. `xtask`: quality gate, licence inventory, bench, unsigned dist.
Details: `ARCHITECTURE.md`.

## Milestones

### M0 — feasibility spike — **done**
Render, text extraction, annotation add, page edit, incremental save and reopen, with fixtures from
two independent producers (Chromium/Skia and Cairo, both real files in `tests/fixtures`).
Evidence: `crates/pdf-engine/examples/spike.rs`, `tests/m0_roundtrip.rs` (4 tests; poppler
re-reads and renders the saved file; original bytes preserved byte-for-byte as a prefix).

### M1 — engine core and viewer shell — **done**
Geometry (rotation/crop/flip-safe page↔view transforms, property-tested), text search with
ligature expansion, tiled render pipeline with cancellation and bounded caches, tabs,
thumbnails, bookmarks, links, theming, command palette, preferences.

### M2 — annotations, page organiser, undo/redo, safe saving — **done**
All annotation kinds in the feature matrix; page rotate/delete/insert/duplicate/reorder/extract/
merge; unlimited undo/redo with clean-state tracking; atomic save with validation and
external-modification detection; autosave.

### M3 — genuine content editing — **done for the supported subset**
Existing text runs in simple fonts and Type0/Identity-H subset fonts are edited in place (no
overlay), including glyph-availability checks, shared-stream copy-on-write, and honest error
reporting for everything else. Images can be added, moved, resized, replaced, deleted.
Evidence: `m3_text_edit` (10) and `m3_objects` (6) tests with poppler text/render oracles; GUI
edit of a real Chromium-made document.

### M4 — forms and measurement — **done for the scope in the feature matrix**
* Forms: read/fill/flatten; duplicate/extract/merge policies Independent / Linked / Flatten with
  a dialog; document-wide flatten; XFA/JS detected and never run. 11 engine tests + GUI check.
* Measurement: calibrated scales per document/page/region, six measurement kinds, Count tool
  with categories, standard `/Measure` annotations, CSV export. 9 unit + 6 integration tests,
  poppler-rendered label check, GUI calibration/measure/count check.

### M5 — release hardening — **partly done**
Done: recovery dialog, CI definitions for three targets, `cargo-deny` policy, licence inventory,
xtask tooling, performance numbers, security notes, documentation, unsigned packaging layout.
**Not done** (honest list): process-isolated render worker, Windows/macOS execution of anything,
signed/notarised packages, installer, printing, password-protected PDFs, accessibility audit,
fuzzing campaign, a long-running memory soak test. See “Remaining work”.

## Additions after the first review (2026-10-05)

Requested by the owner after running the first build on an Intel NUC (Ubuntu, `WGPU_BACKEND=gl`):
rename to BergPDF with a Mont Blanc outline logo; digital signing; default zoom Fit page; Esc leaves
the current tool; ribbon captions that no longer overlap; pictures open as PDFs; OCR. All are
implemented (see FEATURE_MATRIX.md, DECISIONS.md D-013…D-016, EVIDENCE.md). Still true: nothing has
run on Windows or macOS, and the certificate/save pickers of the signing flow and the PNG/CSV export
dialogs were not driven in the headless sandbox.

## Additions after the second review (2026-10-05)

Requested after the owner tried build 2 on the Intel NUC: snapping for measurements, right-click menu,
open-at-top centring, Save As Optimized, PDF/A, translation, an AI key in Preferences with PDF Copilot,
collapsible and properly padded side panels. All are implemented (FEATURE_MATRIX "Third round",
DECISIONS D-017…D-021, EVIDENCE). Honest gaps: the NUC's fit-page offset could not be reproduced, so the
fix is defensive (reset scroll/page on open, fit on first frame) rather than a confirmed root cause;
Copilot/Translate were exercised against a mock server and a real endpoint only up to a 401, never with
a valid key; PDF/A is validated by veraPDF in tests but the app ships no validator; D-012's "no HTTP
client" is replaced by "one, isolated, opt-in".

## Additions after the third review (2026-10-05)

Requested after testing on Windows: calmer resizing, no flashes while zooming, properties sliders that apply
on release, an AI model dropdown, inline translation, two-click callouts, upright text on rotated pages (with
rotation), and installed fonts. All implemented (FEATURE_MATRIX "Fourth round", DECISIONS D-024…D-026). Honest
gaps: the Windows resize slowness could not be reproduced here — the application's own work per frame measured
about 0.5–0.8 ms, so the likely cause is the graphics path (a software renderer, or the DirectX swap-chain), and
the build now reports the adapter and offers other APIs/pacing instead of guessing; installed fonts and inline
translation have only been run on Linux and against a mock AI server.

## Quality gate (what “green” means here)

`cargo xtask check` = `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`,
`cargo test --workspace`. The last full run on Linux is recorded in `docs/EVIDENCE.md`
(commands and counts as printed, not paraphrased).

## Remaining work, in the order recommended

1. **Run it on real Windows 11 and macOS arm64** — follow `PLATFORM_CHECKLIST.md`; fix what
   breaks; only then may rows in the feature matrix become “Verified”.
2. Clear the product name (the project licence is decided: GPL-3.0-or-later, D-001).
3. Process-isolate rendering/parsing (see `SECURITY.md`): the current design contains panics
   and bounds memory per tile/cache, but a hostile file that exhausts memory or hits a stack
   overflow in a dependency still takes the whole app down.
4. Password-protected PDFs (needs decryption support in the PDF library; read-only first).
5. Printing (per-platform), then installer + signing (needs the owner’s certificates).
6. Page labels, headers/footers/watermarks, crop-box UI, XMP metadata editing.
7. Replace direct `ttf-parser` use (unmaintained advisory) with `skrifa`/`read-fonts`.
8. Fuzz `lopdf`/`hayro` entry points and the content-stream scanner; add corpus tests.
9. Secure redaction behind its own gate (true removal from content streams, images, metadata,
   annotations, bookmarks and incremental-save remnants, with an independent extraction test).
10. AI follow-ups: OS key store for the API key; streaming answers; a build without the `ai-client` crate;
    local-model quickstart; evaluate answer quality on real documents; optional-content-aware text.
11. Signing follow-ups: RFC 3161 timestamps, PAdES-LT, certification signatures, OS certificate
    stores / hardware tokens; replace `rsa` when a fixed release exists. OCR follow-ups: accented
    Latin characters (other models), rotated pages, review model licence and bundle or not.

## Rules followed about authorisation

Commits were pushed to the branch `claude/ferrum-pdf-editor` only, after the session's stop hook
asked for uncommitted changes to be committed and pushed. No pull request was opened, nothing was
published as a release, no document was uploaded, and nothing was signed with a real certificate
(only throw-away test keys). No paid service was used. No machine security setting was changed.
System packages were installed **in the disposable sandbox only** (Mesa software Vulkan, Xvfb,
xdotool, poppler) to run the checks above.
