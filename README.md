# BergPDF (working name)

A native, offline desktop PDF editor written in Rust: viewing, annotation, page organisation,
in-place text/image editing, form filling, calibrated measurement/count take-off, digital and
handwritten signatures, and OCR for scanned pages. Pictures (PNG/JPEG) open as one-page PDFs.
Windows 11 x86_64 and macOS Apple Silicon are the target platforms.

**Status: development build. Only ever run on Linux so far.** Read `docs/FEATURE_MATRIX.md`
for exactly what exists and what has (not) been verified, and `docs/PLAN.md` for evidence and
remaining work. Project licence: undecided (`docs/DECISIONS.md` D-001).

## Build and run

```
rustup show                       # installs the pinned toolchain (rust-toolchain.toml)
cargo run --release -p bergpdf # opens the app; pass PDF paths as arguments to open them
cargo run --release -p bergpdf -- tests/fixtures/report-chromium.pdf
```

Linux needs the usual windowing stack (X11 or Wayland) and a Vulkan or GL driver; on Linux
the native file dialog uses the XDG desktop portal. No C compiler is needed for first-party code.

## Developer commands

```
cargo xtask check      # fmt --check, clippy -D warnings, all tests (the CI gate)
cargo xtask fetch-ocr-models  # OPTIONAL: download + verify the OCR models (needed for OCR)
cargo run -p ai-client --example mock_ai_server -- 8099   # fake AI service to try Copilot without an account
cargo xtask icons      # regenerate the logo icons (PNG/ICO/ICNS) from the vector logo
cargo xtask fixtures   # write generated fixtures to target/fixtures
cargo xtask bench      # release-mode performance numbers
cargo xtask licenses   # regenerate docs/THIRD_PARTY_LICENSES.md
cargo xtask dist       # release build + UNSIGNED distribution folder (macOS: .app layout)
```

## Orientation

* `docs/ARCHITECTURE.md` — crates and invariants
* `docs/SECURITY.md` — what the app refuses to do, limits, what is not isolated
* `docs/PERFORMANCE.md` — measured numbers and their caveats
* `docs/DEPENDENCIES.md` — what we depend on and what needs legal attention
* `docs/PLATFORM_CHECKLIST.md` — what a human must verify on Windows and macOS
* `tests/fixtures/README.md` — where test files come from

Not implemented (deliberately gated or not yet built): secure redaction, password-protected
PDFs, printing, installers. Digital signatures check integrity only (never certificate trust).
OCR needs model files that are not included (see above). Nothing is signed or published.

## Optional AI features (PDF Copilot, Translate)

Off until you add your own key: Preferences ▸ PDF Copilot (OpenAI, Anthropic, or a *Custom* OpenAI-compatible
server such as a local model). Text is sent only when you press a button, after a one-time consent. The key is
kept in a file only your account can read (not encrypted — use a spending limit). To try the feature without an
account, run the mock server above and choose Custom with address `http://127.0.0.1:8099/v1`.

Independent PDF/A validation in the tests is optional: set `BERG_VERAPDF` to a directory containing the veraPDF
jars (see docs/EVIDENCE.md for how they were fetched) and `BERG_REQUIRE_VERAPDF=1` to make it mandatory.

## Shipping notes

* **OCR models are not part of the build.** The OCR command is present in every build, but the recognition
  models (≈12 MB) are neither in the repository nor in `cargo xtask dist` output. Without them the OCR dialog
  says where to put them and nothing else happens. The trained weights come from a CC BY-SA 4.0 dataset whose
  redistribution terms have not been reviewed (docs/DECISIONS.md D-015), so bundling them is the owner's call.
* **Fonts**: 14 font files are embedded in the binary (Liberation — SIL OFL 1.1; DejaVu — Bitstream Vera licence);
  ship their licence texts (`crates/pdf-engine/assets/fonts/LICENSE-*.txt`) with the app.
