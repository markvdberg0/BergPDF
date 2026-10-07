# Dependencies

Full machine-generated inventory (647 third-party crates across all targets, with declared SPDX
licence): `THIRD_PARTY_LICENSES.md` (`cargo xtask licenses`). That file is an inventory, **not**
a legal notice bundle. This file covers what a human has to look at.

## Direct dependencies and why

| Crate | Version | Purpose | Notes |
|---|---|---|---|
| hayro | 0.8 | rendering, text/glyph geometry | pure Rust; features: embedded fonts/CMaps, SIMD |
| lopdf | 0.45 (default features off) | object model, parsing, editing, standard-security-handler encryption | MIT; its loader mishandles owner passwords of RC4/AES-128 files, worked around in `pdf_engine::protect` (D-035) |
| md-5, getrandom | 0.11, 0.4 | owner-password recovery (MD5), random file key for new AES-256 protection | MIT/Apache-2.0; already in the tree through lopdf |
| windows | 0.62 (Windows only) | GDI printing API in `crates/printing` | MIT/Apache-2.0; the only crate that uses `unsafe` |
| pdf-writer | 0.15 | build test fixtures only | dev-dependency |
| pdf-base14-metrics | 0.0.2 | widths of the 14 standard fonts | **very young crate**; licence `MIT AND APAFML` (Adobe font metrics notice) |
| pdfboss-encoding | 2.13 | PDF simple-font encodings | young/small crate; verified by tests against poppler output |
| subsetter | 0.2 | subset TrueType for embedding | |
| ttf-parser | 0.25 | font metrics/glyph lookup | |
| image | 0.25 (png, jpeg only) | decode user images before embedding | decoders are an attack surface; dimensions are capped before decoding (`imageembed`) |
| flate2 | 1 (`rust_backend`) | zlib for streams we write | |
| eframe / egui | 0.36 | window, input, UI, wgpu integration | pulls `epaint_default_fonts` (OFL-1.1, Ubuntu Font Licence) |
| rfd | 0.17 (xdg-portal only) | native file dialogs | on Linux uses D-Bus (zbus) to the desktop portal — local IPC, not network |
| serde, toml | 1 / 0.9 | preferences file | |
| thiserror, tracing, tracing-subscriber | | errors, logging (local stderr only) | |
| proptest, tempfile, png | | tests | dev-dependencies |
| cms 0.2, x509-cert 0.2, der 0.7, spki 0.7 | | build the PKCS#7/CMS signature | RustCrypto "formats"; `cms` 0.2 is the stable line |
| rsa 0.9, p256 0.13, sha2 0.10, pkcs8 0.10 | | RSA/ECDSA signing, SHA-256 | **rsa 0.9 has RUSTSEC-2023-0071 (Marvin), no fixed release** — see below |
| p12-keystore 0.3 | | read PKCS#12 (.p12/.pfx) incl. legacy 3DES | pulls pre-release `cms 0.3.0-pre`, `pkcs12 0.2.0-pre`, `x509-parser`, newer `der`/`sha2` (duplicate versions of RustCrypto crates in the tree) |
| ocrs 0.13, rten 0.26 | | offline OCR (text detection + recognition on CPU) | pure Rust; **models are separate files; optional download or opt-in bundling, see D-023** |
| serde_json, png, sha2 | | xtask (licence report, icons, model checksum) | tooling only |
| ureq 3 (rustls + ring, `platform-verifier`), whatlang 0.16, serde_json | | HTTP to the user's AI provider (`ai-client` only) and offline language detection | pulls `rustls`, `ring` (C/assembly crypto primitives), `rustls-platform-verifier` (OS certificate stores), `webpki-roots`/`webpki-root-certs` (**CDLA-Permissive-2.0**, Mozilla root lists — allowed in `deny.toml` with a reason). The only networking code in the workspace |
| embed-resource 3 | | Windows build only: embeds the icon and version info in the exe | MIT; build dependency, needs the Windows SDK resource compiler (`rc.exe`) |

## Items needing attention before any distribution

1. **Licence notices.** Dependencies are MIT / Apache-2.0 / BSD / ISC / Zlib / Unicode-3.0 /
   Unlicense / BSL-1.0 or dual-licensed with one of those. Dual-licensed crates with a
   copyleft alternative (`self_cell`: Apache-2.0 **OR** GPL-2.0; `r-efi`: … **OR** LGPL-2.1+) are
   used under the permissive option. Attribution text for all of them must ship with a binary.
2. **Fonts.**
   * DejaVu Sans/Bold (Bitstream Vera licence + public-domain changes) are compiled into the
     binary and subset-embedded into user PDFs. Licence file: `crates/pdf-engine/assets/fonts/
     LICENSE-DejaVu.txt`. The Vera licence restricts *modified* fonts to being renamed; whether
     subset embedding counts needs a decision by the owner (D-010).
   * **Liberation Sans / Serif / Mono** (12 faces, regular/bold/italic/bold-italic; metric-compatible with
     Arial, Times New Roman and Courier New) and **DejaVu Serif** (regular/bold) were added on 2026-10-05
     so users can choose a font for new and replaced text. Liberation is **SIL OFL 1.1** (notice in
     `crates/pdf-engine/assets/fonts/LICENSE-Liberation.txt`): it may be bundled and embedded, and the OFL
     explicitly does not apply to documents made with the fonts. The fonts add ~5 MB to the binary. The
     same files are registered with egui so pickers preview each face. They come from the Debian
     `fonts-liberation` / `fonts-dejavu-core` packages (unmodified); licence review by the owner is still
     advisable before distribution, like D-010.
   * `epaint_default_fonts` (via egui): MIT/Apache-2.0 for the crate, plus **OFL-1.1** and
     **Ubuntu Font Licence** for the fonts inside; notices required.
3. **Adobe font metrics** (`APAFML`) inside `pdf-base14-metrics`: notice must be preserved.
4. **Maintenance risk:** `pdf-base14-metrics` 0.0.2 and `pdfboss-encoding` are young, low-adoption
   crates; `hayro` is actively developed but pre-1.0 (API and rendering behaviour can change).
   Versions are pinned by `Cargo.lock`; updates should be reviewed, not automatic.
5. **Native/OS components at runtime** (not bundled): GPU drivers (Vulkan/DX12/Metal via wgpu),
   on Linux the XDG desktop portal and X11/Wayland libraries. On Windows/macOS: system UI APIs
   only. The project builds with no C/C++ compiler requirement for first-party code; some
   transitive crates (platform bindings) are `-sys` style FFI wrappers.
6. **Advisories.** `cargo deny check` (v0.20.2) was run in the authoring sandbox against the
   live advisory database: advisories, bans, licenses and sources pass **with two documented
   ignores** (the second is RUSTSEC-2023-0071 for `rsa`, see below) — RUSTSEC-2026-0192: `ttf-parser` 0.25.1 is *unmaintained* (no known vulnerability).
   It is used directly by `pdf-engine` (glyph availability in bundled **and document-embedded**
   fonts) and transitively by winit/egui, so removing our own use reduces but does not eliminate
   it. Follow-up: migrate `fontembed`/`textfont` to `skrifa`/`read-fonts` (already in the tree
   via hayro). `cargo audit` was not run separately (cargo-deny covers the same database).

## Signing and OCR specifics (added 2026-10-05)

* **rsa 0.9 / RUSTSEC-2023-0071 (Marvin timing side channel, unpatched).** `cargo deny` ignores it
  with a written reason: BergPDF signs locally on the user's own machine on request, so there is no
  network service or other oracle an attacker could time. A local attacker who can already time the
  process is outside this application's threat model, but the ignore must be revisited when a fixed
  `rsa` release exists (or the RSA path moved to another implementation).
* **Crypto provenance.** All signing/verification code is pure Rust (RustCrypto). It has had no
  external review; correctness is checked against poppler `pdfsig` and OpenSSL on generated files.
  Key material is held only in memory while signing; the password field is cleared when the dialog
  closes. Certificates are not trusted or validated by BergPDF.
* **OCR models** (`text-detection.rten` 2.4 MB, `text-recognition.rten` 9.3 MB; SHA-256 pinned in
  `crates/pdf-ocr/src/lib.rs`) are from https://github.com/robertknight/ocrs-models, trained on HierText
  (**CC BY-SA 4.0**). The *code* is MIT/Apache-2.0; the licence status of the trained weights has
  **not** been established (the Hugging Face model card could not be read from the authoring
  sandbox). They are not committed. The app can download them (verified) and `cargo xtask dist --with-ocr-models`
  can bundle them; whether redistributing them is allowed is the owner's decision (D-023). The attribution
  text shipped with them is `pdf_ocr::MODEL_NOTICE` (written to `NOTICE-OCR.txt`).
* Test certificates in `crates/pdf-sign/tests/fixtures` are throw-away keys published on purpose.

## Policy

No new dependency is added without recording purpose, licence and maintenance state here.
`unsafe_code = "forbid"` is set for all first-party crates.

## AI client (added 2026-10-05)

* `ai-client` is the only crate that opens sockets. It adds ~140 crates (rustls, ring, http parsing,
  platform verifier). First-party code remains `unsafe_code = "forbid"`; `ring` contains C/assembly.
* TLS trusts the **operating system's** certificate store (found necessary: this sandbox's proxy re-signs
  traffic and a bundled-roots-only build failed with `UnknownIssuer`). The Windows/macOS verifiers have
  not been compiled or run by the author.
* `whatlang` (MIT) detects the document language locally; confidence is reported to the user.
* Verified live only as far as: TLS handshake and a 401 from the real Anthropic endpoint with a
  deliberately invalid key (`docs/evidence/tls-probe-anthropic-dummy-key.txt`). OpenAI's endpoint was
  not reachable from the sandbox (egress policy). **No successful call with a real key has been made.**
* veraPDF (Java, MPL-2.0/GPL-3.0 dual) is used **only by the test suite** as an oracle, fetched from Maven
  Central into a scratch directory; it is not part of the product or its dependency graph.
