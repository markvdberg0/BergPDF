# BergPDF (working name)

A native, offline desktop PDF editor written in Rust: viewing, annotation, page organisation,
in-place text/image editing, form filling, and calibrated measurement/count take-off.
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

Not implemented (deliberately gated or not yet built): secure redaction, OCR, digital
signatures, password-protected PDFs, printing, installers. Nothing is signed or published.
