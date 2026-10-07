# Developing BergPDF

How to compile, run, test and package BergPDF yourself. For what the program does, see the
[README](README.md). The project is licensed under the [GPL-3.0-or-later](LICENSE).

## What you need

* **Rust**, installed with [rustup](https://rustup.rs). The repository pins the toolchain
  (`rust-toolchain.toml`); the first `cargo` command installs it.
* **Windows 11:** [Git](https://git-scm.com) and the *Visual Studio Build Tools* with the
  "Desktop development with C++" workload (the MSVC linker and Windows SDK; Visual Studio Code alone is not
  enough). With winget:
  ```powershell
  winget install --id Git.Git -e --source winget
  winget install --id Rustlang.Rustup -e --source winget
  winget install --id Microsoft.VisualStudio.2022.BuildTools -e --source winget --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
  ```
  Open a new terminal afterwards.
* **macOS:** the Xcode command line tools (`xcode-select --install`).
* **Linux:** a C toolchain for a few dependencies and a windowing/graphics stack
  (X11 or Wayland, plus Vulkan or OpenGL drivers). On Debian/Ubuntu: `sudo apt install build-essential pkg-config`.
  The native file dialogs use the XDG desktop portal.

## Build and run

```
git clone https://github.com/markvdberg0/BergPDF.git
cd BergPDF
cargo run --release -p bergpdf                       # starts the app
cargo run --release -p bergpdf -- path/to/file.pdf   # opens a file
```

The first release build takes several minutes. Use `cargo build -p bergpdf` (debug) while developing.
`BERG_FRAME_LOG=1` prints how long the application's own work per frame takes.

### Graphics problems

BergPDF draws with [wgpu](https://wgpu.rs). On Windows it tries DirectX 12 first. If the window does not open or
is slow, set another API for one run (`WGPU_BACKEND=vulkan`, `dx12` or `gl`; on PowerShell:
`$env:WGPU_BACKEND="gl"`), or use *Preferences ▸ Graphics* inside the app. Under a virtual machine you may need
`WGPU_BACKEND=gl`.

## Repository layout

| Path | What it is |
|---|---|
| `apps/desktop` | the application (egui/eframe); binary `bergpdf` |
| `crates/pdf-engine` | PDF parsing, rendering, editing, annotations, forms, measurement, OCR text layer, PDF/A, optimizer, fonts |
| `crates/editor-core` | document session, undo/redo, commands, view model, preferences (no UI toolkit) |
| `crates/platform` | file dialogs, recovery files, directories, API-key file |
| `crates/pdf-sign` | digital signatures (PKCS#7) |
| `crates/pdf-ocr` | OCR engine wrapper (`ocrs`/`rten`) and the pinned model list |
| `crates/ai-client` | the optional AI client and the verified downloader (the only networking code) |
| `crates/brand`, `assets/` | logo and icons |
| `crates/test-support`, `tests/fixtures` | test helpers and fixture documents |
| `xtask` | developer commands (below) |
| `docs/` | architecture, decisions, security notes, feature matrix, evidence ([`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) is the place to start) |

## Developer commands

```
cargo xtask check                    # fmt --check, clippy -D warnings, all tests (the CI gate)
cargo xtask fetch-ocr-models         # download + verify the OCR models into the user folder
cargo xtask icons                    # regenerate the PNG/ICO/ICNS icons from the vector logo
cargo xtask fixtures                 # write the generated fixtures to target/fixtures
cargo xtask bench                    # release-mode performance numbers (docs/PERFORMANCE.md)
cargo xtask licenses                 # regenerate docs/THIRD_PARTY_LICENSES.md
cargo xtask dist                     # release build + unsigned distribution folder in dist/
cargo xtask dist --with-ocr-models   # the same, with the verified OCR models in ocr-models/
cargo xtask msix                     # Microsoft Store package (MSIX), Windows SDK needed; see docs/MICROSOFT_STORE.md
cargo deny check                     # licences, advisories, sources (deny.toml)
```

Some tests use independent tools as oracles (poppler's `pdftotext`/`pdftoppm`/`pdfsig`, OpenSSL, the OCR models,
veraPDF for PDF/A). Without them those checks are skipped locally; CI requires them. To require them yourself:
`BERG_REQUIRE_ORACLES=1`, `BERG_REQUIRE_OCR=1` with `BERG_OCR_MODELS=<folder>`, and `BERG_REQUIRE_VERAPDF=1` with
`BERG_VERAPDF=<folder with the veraPDF jars>` (see [`docs/EVIDENCE.md`](docs/EVIDENCE.md) for how they were fetched).
On Debian/Ubuntu: `sudo apt install poppler-utils openssl`.

## Trying the AI features without an account

```
cargo run -p ai-client --example mock_ai_server -- 8099
```

then, in *Preferences ▸ PDF Copilot*, choose *Custom* with address `http://127.0.0.1:8099/v1` and any key. The
mock answers with canned text and "translates" by tagging each paragraph.

## Packaging

The installers are made by [cargo-packager](https://github.com/crabnebula-dev/cargo-packager) from
[`Packager.toml`](Packager.toml), on top of the folder `cargo xtask dist` builds (static C runtime on Windows).
The Windows installer uses a copy of cargo-packager's NSIS template with one extra section that asks about the
OCR models, so install exactly that version:

```
cargo install cargo-packager --version 0.11.8 --locked
cargo xtask dist
cargo packager --release -f nsis     # Windows: dist/packages/*-setup.exe
cargo packager --release -f dmg      # macOS
cargo packager --release -f deb      # Linux
```

For the **Microsoft Store** build an MSIX package instead (needs the Windows SDK; unsigned on purpose, the Store signs
it): `cargo xtask msix`. The identity comes from Partner Center; everything about it, trying the package and submitting
is in [`docs/MICROSOFT_STORE.md`](docs/MICROSOFT_STORE.md). The Store copy is recognised by a marker file in the package
and has no update check of its own (D-034); to see that without packaging start BergPDF with `BERG_DISTRIBUTION=store`.

`packaging/windows/bergpdf.iss` is an alternative Windows installer for [Inno Setup](https://jrsoftware.org/isinfo.php).
Keep `version` in `Packager.toml` equal to the workspace version in `Cargo.toml` (a test checks this).
Nothing is code-signed or notarised; that needs certificates of the publisher.

## Releasing

Pushing a tag `vX.Y.Z` that equals the workspace version runs
[`.github/workflows/release.yml`](.github/workflows/release.yml): it builds the installers for Windows, macOS and
Linux and attaches them with `SHA256SUMS.txt` to a **draft** GitHub release. Nothing is published until a person
presses *Publish release*. Because BergPDF is GPL software, keep the tag: it is the source for that binary.

To bring out a new version: raise `version` in **both** `Cargo.toml` (workspace) and `Packager.toml` (the workflow
checks they agree with the tag), commit and push to `main`, wait for CI, then `git tag vX.Y.Z && git push origin vX.Y.Z`,
write release notes in the draft and publish it. Installed copies that allowed the update check then show
"Update available" (see D-033); to try that without a real release run a local server that answers
`{"tag_name":"v9.9.9"}` and start BergPDF with `BERG_UPDATE_URL=http://127.0.0.1:PORT/latest`.

## Shipping notes

* **OCR models.** They are not in the repository or in the binary. By default users download them (OCR dialog,
  or the Windows installer's question): fetched from the model author's public host, checked against the SHA-256
  hashes pinned in `crates/pdf-ocr`, and only kept when the hash matches. `cargo xtask dist --with-ocr-models`
  bundles them into the distribution folder instead (macOS: `BergPDF.app/Contents/Resources/ocr-models`), with
  `NOTICE-OCR.txt`; the app looks there first. The trained weights come from a CC BY-SA 4.0 dataset and the
  model repository has no licence file, so whether the weights may be *redistributed* (bundling them, or hosting
  them yourself) has **not** been verified. Downloading from the author's host redistributes nothing.
  See [`docs/DECISIONS.md`](docs/DECISIONS.md) D-023 and D-029.
* **Unsigned.** Windows SmartScreen and macOS Gatekeeper warn about unsigned programs. Signing needs a
  Windows code-signing certificate and an Apple Developer account (for signing and notarisation).
* **Fonts.** 14 font files are embedded in the program (Liberation: SIL OFL 1.1; DejaVu: Bitstream Vera
  licence). Their licence texts are in `crates/pdf-engine/assets/fonts/LICENSE-*.txt` and ship with the app.
  Installed fonts can also be chosen for text; only fonts whose embedding flags allow it are offered.
* **GPL.** A binary release is a distribution: the source of that exact version must stay available (the release
  tag). `docs/THIRD_PARTY_LICENSES.md` is an inventory of the dependencies, not the full licence notices.
* **Product name.** "BergPDF" has not been checked against existing trademarks.

## Adding or fixing a translation

Texts are written in English in the code as `tr("…")` or `tf!("… {} …", value)`. The translations are the lists in
`apps/desktop/src/i18n/nl.rs` and `de.rs` (`("English text", "translation")`, same number of `{}` in both). To fix a
translation edit its line; to add a language add a catalog file, a `Lang` variant and a `Language` choice in
`crates/editor-core/src/prefs.rs`. `cargo test -p bergpdf i18n` lists texts that are missing
(`BERG_DUMP_MISSING=nl cargo test -p bergpdf every_text -- --nocapture`). Keep ribbon labels short; long words wrap
badly in the toolbar.

## Where the details are

* [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md): crates and invariants
* [`docs/DECISIONS.md`](docs/DECISIONS.md): why things are the way they are
* [`docs/FEATURE_MATRIX.md`](docs/FEATURE_MATRIX.md): what exists and how it was verified
* [`docs/SECURITY.md`](docs/SECURITY.md): what the app refuses to do and what is not isolated
* [`docs/DEPENDENCIES.md`](docs/DEPENDENCIES.md): what we depend on, licences, OCR model notes
* [`docs/MICROSOFT_STORE.md`](docs/MICROSOFT_STORE.md): the Microsoft Store package, Partner Center steps, listing and certification texts
* [`docs/PLATFORM_CHECKLIST.md`](docs/PLATFORM_CHECKLIST.md): what still has to be verified by hand on Windows and macOS
* [`docs/PLAN.md`](docs/PLAN.md), [`docs/EVIDENCE.md`](docs/EVIDENCE.md): history and test evidence
