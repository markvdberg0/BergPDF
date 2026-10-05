# Evidence log

> Product name note: this log was started when the product was still called “Ferrum PDF”; it was renamed BergPDF afterwards and names below were updated mechanically. The first sections describe the state at the end of milestone M5; the last section (“After the first review”) covers the additions of 2026-10-05.

Everything here was produced by commands run in the authoring sandbox (Linux x86_64); outputs
are pasted as printed, not paraphrased. **Nothing here was run on Windows or macOS.**

## Environment

- date: 2026-10-05T06:17:25Z
- rustc: rustc 1.97.0 (2d8144b78 2026-07-07)  cargo: cargo 1.97.0 (c980f4866 2026-06-30)
- uname: Linux 6.18.44-fc-v70 x86_64
- pdftotext: pdftotext version 24.02.0
- GUI checks: Xvfb 1600×1000, Mesa lavapipe (software Vulkan), `xdotool`, ImageMagick `import`.
  The window was given X focus explicitly before typing (a test-harness requirement, not an
  application behaviour).

## Quality gate — `BERG_REQUIRE_ORACLES=1 cargo xtask check`

(= `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`; with oracles *required*, so a missing poppler would have failed.)

```
Running unittests src/lib.rs  =>  test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.07s
Running tests/jobs.rs  =>  test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 1.03s
Running tests/session.rs  =>  test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.33s
Running unittests src/main.rs  =>  test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.00s
Running unittests src/lib.rs  =>  test result: ok. 43 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.01s
Running tests/m0_roundtrip.rs  =>  test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.31s
Running tests/m3_objects.rs  =>  test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.08s
Running tests/m3_text_edit.rs  =>  test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.19s
Running tests/m4_forms.rs  =>  test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.09s
Running tests/m4_measure.rs  =>  test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.11s
Running tests/m4_meta_export.rs  =>  test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.10s
Running unittests src/lib.rs  =>  test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.21s
Running unittests src/lib.rs  =>  test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.00s
Running unittests src/main.rs  =>  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.00s
Doc-tests editor_core  =>  test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.00s
Doc-tests pdf_engine  =>  test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.00s
Doc-tests platform  =>  test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.00s
Doc-tests test_support  =>  test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.00s
xtask check exit code: 0
```

Total: 123 tests passed, 0 failed (19 editor-core unit, 4 jobs, 7 session, 3 desktop, 43
pdf-engine unit, 4 m0_roundtrip, 6 m3_objects, 10 m3_text_edit, 11 m4_forms, 6 m4_measure,
2 m4_meta_export, 7 platform, 1 xtask).

Bugs the gate/GUI testing caught and that were fixed before this log (so the history is not
rosy): thumbnails requested before the worker pool existed stayed “in flight” forever; a
per-frame window-title command caused a 250 % CPU repaint loop; sliders/checkboxes were
invisible in the custom theme; “Save As” to the same path wrongly ran the external-modification
check; PDF reals are f32 so a stored scale gave 10.000008 instead of 10 (now stored as text);
the first `cargo xtask check` after adding tooling failed `fmt` and `clippy` on the new files; flattening a form first dropped the value of a combo box that had no stored appearance (now generated before baking).

## Supply chain — `cargo deny check` (cargo-deny 0.20.2, live advisory DB)

```
advisories ok, bans ok, licenses ok, sources ok
```

Passes with **one documented ignore**: RUSTSEC-2026-0192 (`ttf-parser` 0.25.1 unmaintained).
See `DEPENDENCIES.md`. Before adding that ignore, the check failed on: `LicenseRef-Undecided`
for first-party crates (now `licenses.private.ignore`), path-dependency wildcards, and this
advisory. Duplicate-version warnings (windows-sys, skrifa, …) remain as warnings.

## Packaging smoke test — `cargo xtask dist` (run before the properties/export/flatten additions; the final gate above was re-run after them)

```
    Finished `dist` profile [optimized + debuginfo] target(s) in 7m 27s
unsigned distribution folder: /home/user/pdf-editor/dist/bergpdf-0.1.0-linux-x86_64
(not signed, not notarised, not an installer — see docs/PLATFORM_CHECKLIST.md)
      4250  DEPENDENCIES.md
      2012  README.md
     43297  THIRD_PARTY_LICENSES.md
  33621120  bergpdf
```

The stripped Linux binary was launched (screenshot `evidence/01-launch-release-binary.png`),
opened `tests/fixtures/report-chromium.pdf`, and after settling used 0.0 % CPU
(`top -b -n 3 -d 3`) with 190 MB RSS under the *software* GPU. The macOS `.app` layout generator
is unit-tested (`xtask::bundle`), but no bundle has ever been launched on a Mac.

## Performance — `cargo xtask bench` (release)

```

== report-chromium.pdf (3 pages, embedded fonts) (41.2 KB)
open + parse                                                     0.8 ms   (peak RSS 5 MB)
page list                                                        0.0 ms   (peak RSS 5 MB)
hayro load + render page 1 @ 1.0x (fit-width-like)              10.2 ms   (peak RSS 13 MB)
TOTAL time to first rendered page                               11.3 ms

== 1000-page text document (2950.4 KB)
open + parse                                                    18.1 ms   (peak RSS 18 MB)
page list                                                        3.1 ms   (peak RSS 18 MB)
hayro load + render page 1 @ 1.0x (fit-width-like)              21.9 ms   (peak RSS 27 MB)
TOTAL time to first rendered page                               49.5 ms

== A0 page, 60 000 vector shapes (6544.2 KB)
open + parse                                                    11.1 ms   (peak RSS 30 MB)
page list                                                        0.0 ms   (peak RSS 30 MB)
hayro load + render page 1 @ 1.0x (fit-width-like)             739.5 ms   (peak RSS 152 MB)
TOTAL time to first rendered page                              757.3 ms

== 1000-page document operations
page ids (walk page tree)                                        1.2 ms   (peak RSS 152 MB)
render page 500 thumbnail (0.2x)                                 6.6 ms   (peak RSS 152 MB)
render 50 thumbnails (0.2x) in one session                     137.0 ms   (peak RSS 152 MB)
search 'quick' across all 1000 pages (text extraction)         819.8 ms   (peak RSS 152 MB)
add one annotation (transaction)                                 0.2 ms   (peak RSS 152 MB)
incremental snapshot after the edit                              1.1 ms   (peak RSS 152 MB)
incremental save size                                            800 bytes appended to a 3021246 byte original
delete 500 pages (one transaction)                               2.3 ms   (peak RSS 152 MB)

```

Interpretation and caveats: `PERFORMANCE.md`.

## GUI checks actually performed (screenshots in `docs/evidence/`)

| # | What was done | What was observed |
|---|---|---|
| 01 | Launched the release binary with a real Chromium-made PDF | Window, ribbon, thumbnails and page rendered; no warnings other than X11/DRI3 messages from the software stack |
| 02 | Clicked the “name” field of `form_rich.pdf`, typed text, OK; clicked the checkbox and the second radio button | Value drawn in the field (embedded-font appearance), checkbox and radio marks drawn; document became dirty; undo arrow enabled |
| 03 | Duplicate Pages on a page with fields | Form-policy dialog with Independent / Linked / Flatten explained; Continue produced a 3rd page whose fields are independent |
| 04 | Calibrated page 2 (200.2 pt ↔ 20 m), measured a distance (12.87 m), a polygon area (36.15 m²) and two count markers | Labels drawn, side panel lists measurements and totals, status bar showed the scale |
| 05 | Saved from the app (Ctrl+S), rendered the saved file with **poppler** (`pdftoppm`) | Field values, marks, measurement labels and count markers all present in the independent render |
| 06 | Created stale recovery files, started the app | “Recover unsaved work?” dialog; “Restore as new tab” opened the recovered document (including unsaved page-2 measurements) as a dirty tab and wrote a fresh recovery file |
| 07 | Document properties (set title and author), then Flatten Form Fields via the palette, confirm, Ctrl+S; inspected the saved file with poppler `pdfinfo`/`pdftotext` | `pdfinfo`: `Title: Quarterly forms`, `Author: Mark vd Berg`, `Form: none`; page text now contains the field value as page content; field tint and Forms tab gone; combo value “Red” (which had no stored appearance) baked in |

Earlier (pre-compaction) GUI checks, not re-screenshotted here: palette (Ctrl+K), highlight and
rectangle tools, text-box dialog, Ctrl+S growing the file incrementally, Edit Text changing
`ALPHA-7731` → `ALPHA-1377` in the Chromium document, and the missing-glyph guidance panel.

## Not run / not proven

Windows and macOS builds or tests; real-GPU rendering; HiDPI; IME; accessibility tooling;
printing; process isolation (not implemented); fuzzing; long-session memory; native CSV/PNG save dialogs and other native file dialogs headless (PNG export is tested at engine level only); merge-documents flow in the GUI; region-scale drawing
in the GUI; password-protected PDFs (unsupported).


## After the first review (2026-10-05): rename, logo, pictures, signing, OCR

Run with `BERG_REQUIRE_ORACLES=1 BERG_REQUIRE_OCR=1 BERG_OCR_MODELS=<dir> cargo test --workspace` after
`cargo fmt --check` (clean) and `cargo clippy --workspace --all-targets -- -D warnings` (clean):

```
Running unittests src/main.rs  =>  test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.00s
Running unittests src/lib.rs  =>  test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.02s
Running unittests src/lib.rs  =>  test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.07s
Running tests/jobs.rs  =>  test results_are_tagged_with_revision_so_stale_ones_can_be_dropped ... ok
Running tests/jobs.rs  =>  test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 1.03s
Running tests/ocr.rs  =>  test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 6.81s
Running tests/session.rs  =>  test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.08s
Running unittests src/lib.rs  =>  test result: ok. 43 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.01s
Running tests/m0_roundtrip.rs  =>  test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.28s
Running tests/m3_objects.rs  =>  test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.06s
Running tests/m3_text_edit.rs  =>  test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.16s
Running tests/m4_forms.rs  =>  test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.12s
Running tests/m4_measure.rs  =>  test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.10s
Running tests/m4_meta_export.rs  =>  test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.07s
Running tests/m5_image_docs.rs  =>  test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.05s
Running tests/m6_signing.rs  =>  test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.22s
Running unittests src/lib.rs  =>  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.00s
Running unittests src/lib.rs  =>  test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.03s
Running unittests src/lib.rs  =>  test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 0.18s
Running unittests src/main.rs  =>  test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; in 3.08s
TOTAL: 150 passed, 0 failed
cargo deny check  =>  advisories ok, bans ok, licenses ok, sources ok   (two documented advisory ignores)
```

Independent checks of the new features (all executed, outputs saved under `docs/evidence/`):

* **Signing** — `pdfsig` (poppler 24.02) on files written by BergPDF: `Signature Validation: Signature is
  Valid.`, `Total document signed`, `Certificate Validation: Certificate issuer isn't Trusted.` (honest:
  self-signed test certificates) — `evidence/pdfsig-output-rsa.txt`, `…-chain.txt`. OpenSSL
  `cms -verify` accepts the same signatures; flipping one signed byte makes both our checker and `pdfsig`
  reject it; RSA, ECDSA P-256, legacy 3DES and chained certificates all verified.
* **OCR** — real models, scan built by rendering page 1 of the Chromium fixture at 300 dpi:
  `OCR: 66 words recognised; recall of printed words 0.96 (48/50)`; poppler extracts the text and its word
  boxes sit within 8 pt of the printed words; the page renders identically (invisible text). About 1 s per
  A4 page on 4 cores (release build). GUI: image opened as a PDF, OCR run from the dialog, Find located
  “quick brown” on the scan (`evidence/08-…`). Without models the dialog says where to put them (`13-…`).
* **Pictures** — PNG/JPEG → PDF: JPEG bytes appear verbatim in the file, EXIF orientation 6 renders the right
  way up in poppler, oversized/unsupported inputs refused.
* **UI fixes** — ribbon captions no longer overlap (`11-…`), Esc returns to the Select tool, default zoom is
  Fit page (preference), logo in the title bar/welcome screen/window icon (`12-…`).

Also found and fixed while doing this: the dev build directory grew to 24 GB with full debug info and
filled the sandbox disk (`debug = "line-tables-only"` now; 1.8 GB); the first ribbon fix still
overlapped because centre-aligned galleys are anchored at their centre (caught by a screenshot);
flatten/sign unit expectations (widgets are annotations too).

Not exercised headless: the native certificate picker and the save dialog of the signing flow (the
engine and session paths are tested), the native PNG/CSV save dialogs, OCR on Windows/macOS, any
Windows/macOS behaviour at all.

---

## Addendum — third round (2026-10-05)

Full gate on Linux with **every oracle required** (`BERG_REQUIRE_ORACLES=1 BERG_REQUIRE_OCR=1
BERG_VERAPDF=<jars> BERG_REQUIRE_VERAPDF=1`): `cargo fmt --check` clean, `cargo clippy --workspace
--all-targets -D warnings` clean, **217 tests passed, 0 failed** (per-binary lines in
`evidence/test-run-2026-10-05-217-tests.txt`), `cargo deny check` → advisories ok, bans ok, licenses ok,
sources ok (one more licence allowed with a reason: CDLA-Permissive-2.0 for Mozilla's root lists).

**How veraPDF was obtained** (test oracle only; not in the product): `mvn dependency:copy-dependencies`
of `org.verapdf.apps:greenfield-apps:1.28.2` from Maven Central into a scratch directory, run as
`java -cp "<dir>/*" org.verapdf.apps.GreenfieldCliWrapper --flavour 2b --format xml file.pdf`
(`test_support::verapdf`). veraPDF's own site was blocked by the sandbox egress policy; Maven Central was not.

What the independent tools said:

* **PDF/A** — before conversion veraPDF rejects the Chromium/Cairo/picture fixtures for exactly the reasons
  expected (6.1.3 file ID, 6.6.2.1 XMP, 6.2.4.3 device colour without output intent, 6.2.11.4.1 unembedded
  font, 6.2.11.3.2 CIDToGIDMap); after conversion it reports **compliant** for the documents listed in
  FEATURE_MATRIX. A mutation test (output intent and `/ID` removed again) is rejected with 6.2.4.3 and 6.1.3.
  The BergPDF-made embedded subset fonts (text added in the editor; the Translate export) validate too.
* **Optimizer** — poppler `pdftotext`/`pdftoppm` identical (lossless: mean pixel difference 0.0); poppler
  stderr empty. The test originally passed while poppler printed "incorrect stream length" because the
  assertion looked only at `pdfinfo`; fixed by `test_support::poppler_diagnostics` and a real fix (update
  `/Length` after recompressing).
* **Snap** — GUI: two clicks on a snapped corner and a snapped T-junction measured 360.6 pt, the exact
  √(300²+200²) (`evidence/18-…`); intersection and midpoint markers (`19-…`).
* **Copilot / Translate** — GUI driven against `mock_ai_server` (Custom provider): consent dialog →
  summary with passages, an invented quote flagged, "Highlight all" creating highlights over a passage
  that wraps across two lines; Explain from the right-click menu sent only the neighbouring pages (1,568
  system-prompt characters vs 1,720 for the whole document); Translate detected "English (100 % sure)",
  warned that the target was the same language, then translated three pages with progress.
  Key saved from Preferences: file mode `-rw-------`, 0 occurrences in `preferences.toml`.
* **Live endpoint** — `cargo run -p ai-client --example tls_probe -- anthropic` (dummy key): TLS handshake
  succeeded through the sandbox proxy and the real service answered `401 API key is invalid`, shown as a
  readable message (`evidence/tls-probe-anthropic-dummy-key.txt`). This first failed with `UnknownIssuer`
  (bundled roots only); switching to the operating-system certificate store fixed it. OpenAI's host is not
  reachable from the sandbox.

Found and fixed while doing this: the collapse buttons first appeared right next to the tab labels instead of
at the panel edge (`ui.horizontal` does not span the panel; `egui::Sides` does); the first optimizer test
passed while poppler printed stream-length errors (see above); clippy lints in the new code. Noted, not
explained: in one early headless run a right-click on empty page area showed no menu right after pressing
Escape; the next runs showed it every time, so it is recorded here as unexplained rather than as fixed.
`pkill -f` killed its own shell once during GUI testing (harness only).

**Not verified, and why it matters**

* No successful AI call with a valid key; answer quality, token costs and each provider's exact error texts
  are untested. Anthropic and OpenAI request shapes follow the vendors' documented formats and the mock
  server checks our side of them.
* The fit-page offset the owner saw on the NUC: not reproduced (fixtures, several window sizes, panels
  open/collapsed all centre correctly). The open path now resets scroll/page and the first frame fits.
* Native file pickers (optimize/PDF-A/translation saves, certificate picker) are not driven headless.
* Windows and macOS: nothing was compiled or run; the OS certificate verifiers and the key-file permissions
  there are untested (PLATFORM_CHECKLIST).

---

## Addendum — fonts for text (2026-10-05)

`cargo fmt`, `clippy -D warnings` clean; **225 tests passed, 0 failed** with poppler, OpenSSL, OCR models and
veraPDF required (`evidence/test-run-2026-10-05-fonts.txt`); `cargo deny check` ok.

* All 16 distinct faces (5 families × bold/italic where available) added to a blank page: poppler extracts every
  sample line (accents, Greek, Cyrillic, €), `pdffonts` lists each as an **embedded subset** under its own
  name, and upright/italic/serif/sans pages render measurably differently.
* A text box in Liberation Serif Bold Italic reopens with that exact font (`/BergFont`); before this change a
  text box's bold flag was not stored and was lost on re-open.
* Replacing the font of an existing Chromium/Cairo run with Liberation Mono Bold reopens in that face; the
  edit report names both fonts. A document using every face, plus a text box, converts to PDF/A and veraPDF
  reports it compliant.
* GUI (`evidence/29-…`–`32-…`): picker with each name drawn in its own typeface, the Add-Text box previewing the
  chosen font, Edit Text → Font → Liberation Mono (bold pre-ticked from the original name), the text-box
  dialog and Properties panel with the font picker.
* Harness note: typing into the app with `xdotool` before a dialog is visible fires keyboard shortcuts
  (it scrolled to page 3 once); the final runs screenshot before typing. `xdotool` also cannot type `é`.

Not covered: system fonts, CJK, mixed fonts inside one text run, Windows/macOS.
