# Feature matrix

Status vocabulary (deliberately strict):

| Status | Meaning |
|---|---|
| **Planned** | Not started. |
| **Partial** | Some of it exists; the gaps are listed. |
| **Implemented** | Works and is covered by the evidence named in the row. Runtime-tested **on Linux only**. |
| **Verified** | Implemented **and** exercised on both Windows 11 x86_64 and macOS Apple Silicon. |
| **Gated / Unavailable** | Deliberately not offered until a separate safety gate is passed. |

**Nothing is “Verified”.** The development sandbox is Linux; the Windows and macOS builds have
never been compiled or run by the author of this matrix (CI definitions exist in
`.github/workflows/ci.yml` but have not been executed). See `PLATFORM_CHECKLIST.md`.

“Evidence” names automated tests (`crate::module` or `tests/file.rs`) and/or manual GUI checks
that were actually run (Xvfb + software Vulkan, screenshots read by the author). “Oracle” means
an independent tool (poppler) was used to check the result, not just this program re-reading its
own output.

## Viewing and navigation

| Feature | Status | Evidence / gaps |
|---|---|---|
| Open PDF (dialog, drag-and-drop, command line) | Implemented | manual GUI; hayro + lopdf parse |
| Open PNG/JPEG pictures: become a one-page PDF (A4 long side, aspect kept, JPEG bytes embedded unchanged, EXIF rotation 3/6/8 honoured); Save writes the PDF | Implemented | `m5_image_docs` (5 tests, poppler render oracle); GUI open. Other formats (TIFF, WebP, GIF, BMP, HEIC) are not supported |
| Insert pictures as pages of an open document (Organize → Image Page) | Implemented | `m5_image_docs`; native picker not exercised headless |
| Default zoom on open (Fit page / Fit width / 100 %, default Fit page); Esc leaves any tool for Select | Implemented | manual GUI |
| Continuous / single / facing layout, zoom, fit width/page, view rotation | Implemented | `editor_core::view` unit tests; manual GUI |
| Tiled, cached, cancellable rendering (hayro) with placeholders | Implemented | `editor_core::jobs`, `tiles` tests; manual GUI |
| Thumbnails (sidebar) with drag-to-reorder | Implemented | manual GUI |
| Bookmarks / outline panel (named destinations resolved) | Implemented (read-only) | `pdf_engine::nav`; no outline *editing* |
| Internal and external links (external always asks first) | Implemented | `nav` tests; manual GUI |
| Text selection, copy | Implemented | `pdf_engine::text` tests; manual GUI |
| Search (ligature-aware, all pages, cancellable) | Implemented | `text` tests; 1000-page timing in `PERFORMANCE.md` |
| Dark **page** view (render-time filter; never edits the file) | Implemented | `pagefilter` unit test; manual GUI |
| Dark **application** theme (independent of the above) | Implemented | `prefs` test; manual GUI |
| Tabs, recent files, session restore of open files | Partial | tabs + recent: yes; reopen-last-session: no |
| Encrypted / password-protected PDFs | **Not supported** | opening reports a clear error; no decryption (crypto features of the PDF library are disabled) |
| Accessibility (screen reader, keyboard-only use of all dialogs) | Partial | keyboard shortcuts everywhere; egui AccessKit exposure not audited |

## Annotations (comment layer)

| Feature | Status | Evidence / gaps |
|---|---|---|
| Highlight, underline, strikeout, squiggly | Implemented | `m0_roundtrip`, annot unit tests; oracle render |
| Sticky note, text box, callout | Implemented | `m0_roundtrip`; GUI |
| Rectangle, ellipse, line, arrow, polygon, polyline, pencil | Implemented | annot tests; GUI |
| Text stamp | Implemented | stamp text limited to the bundled font’s glyphs; no image/dynamic stamps |
| Select, move, resize, duplicate, delete, edit properties | Implemented | GUI; undo tests in `editor_core::session` |
| Comments panel | Implemented | no reply threads, status or filtering by author |
| Existing foreign annotations preserved | Implemented | `existing_annotations` fixture, `m0_roundtrip` |
| File attachment, sound, 3D, redact annotations | Planned | not created or edited |

## Page organisation and document operations

| Feature | Status | Evidence |
|---|---|---|
| Rotate, delete, insert blank, reorder, duplicate pages | Implemented | `pdf_engine::pageops` tests, `m0_roundtrip`; GUI |
| Extract pages to a new file / merge documents | Implemented | `m4_forms` (import), GUI duplicate flow; merge file dialog not GUI-verified |
| Dead bookmarks/links removed when pages are deleted | Implemented | `pageops` tests |
| Crop box editing | Partial | engine `set_crop_box` exists and is tested; **no UI** |
| Document properties: title, author, subject, keywords (XMP is *not* updated; the dialog says so) | Implemented | `m4_meta_export` (poppler `pdfinfo` reads the values); GUI |
| Export current page as PNG (150 dpi, reduced for huge pages, rendered off the UI thread) | Implemented | engine: `m4_meta_export`; the native save dialog was not exercised headless |
| Page labels / numbering, headers, footers, watermarks | Planned | |
| Split by size/bookmarks, batch operations | Planned | |

## Content editing (changes the page’s real content stream)

| Feature | Status | Evidence / gaps |
|---|---|---|
| Edit existing text runs (supported simple + Type0 Identity-H fonts) | Implemented | `m3_text_edit` (10 tests, poppler text + render oracle); GUI edit of “ALPHA-7731” |
| Missing-glyph handling: explains, lists characters, optional substitute font | Implemented | `m3_text_edit`; GUI |
| Unsupported text is outlined grey with the reason | Implemented | `m3_text_edit`; GUI |
| Copy-on-write of shared content streams/resources | Implemented | `m3_text_edit` (shared stream fixture) |
| Add text (bundled DejaVu, embedded as subset) / add image (PNG, JPEG) | Implemented | `m3_objects` (rotated/cropped/flipped pages) |
| Move, resize, replace, delete images | Implemented | `m3_objects` |
| Reflow of paragraphs, font matching of arbitrary fonts, vector/path editing | Planned | text edits are per text run, not reflowed |

## Forms

| Feature | Status | Evidence / gaps |
|---|---|---|
| Read the field tree (hierarchical names, kinds, options) | Implemented | `m4_forms::reads_the_field_tree` |
| Fill text (Unicode via embedded font), checkbox, radio, combo/list | Implemented | `m4_forms` (poppler render oracle); GUI fill of text, box and radio |
| Field highlighting; Forms ribbon tab only when a form exists | Implemented | GUI screenshot |
| Flatten all form fields in the document (confirmation first; values without a stored appearance are drawn before baking) | Implemented | `m4_forms::flatten_document…` (poppler render); GUI + `pdfinfo` reports `Form: none` |
| Page duplicate/extract/merge policy: **Independent / Linked / Flatten** with dialog | Implemented | `m4_forms` (6 tests incl. name collisions, shared radio group, flatten render); GUI dialog |
| XFA, JavaScript, calculations | Detected, **never executed** | `m4_forms::xfa_and_javascript_are_detected_never_executed` |
| Form creation / field design | Planned | |
| Existing signature fields | Displayed; signed documents warn on open | see the Signing section for what can be checked |

## Measurement and quantity take-off

| Feature | Status | Evidence / gaps |
|---|---|---|
| Calibrate from a known length; preset ratios (1:n); custom `paper = real` | Implemented | `measure` unit tests; GUI calibration (200.2 pt ↔ 20 m) |
| Scale scopes: document, page, region (smallest region wins) | Implemented | `m4_measure::scale_registry_roundtrips…`; region drawing not GUI-verified |
| Distance, path length, polygon area, rectangle, radius, angle | Implemented | `m4_measure`; poppler render of labels |
| Count tool with categories, per-page totals | Implemented | `m4_measure::counts_summarise…`; GUI |
| Standard `/Measure` dictionary on annotations + private extension | Implemented | `m4_measure` asserts `/Measure /Subtype /RL`, `/IT` |
| Uncalibrated warning (status bar, notice, CSV flag) | Implemented | GUI |
| Snapping to existing vertices, Shift angle lock | Implemented | manual GUI only |
| “Update to current scale” when a scale changes | Implemented | `m4_measure::rescale_page…` |
| CSV export (formula-injection safe) | Implemented | `measure` tests; the native save dialog is not exercised headless |
| Scale detection from title blocks/dimensions, perimeter snapping to geometry | Planned | |
| `/UserUnit` ≠ 1 | Not handled | results would be wrong; documented limitation |

## Signing, scanned pages

| Feature | Status | Evidence / gaps |
|---|---|---|
| **Digital signature** with a certificate file (.p12/.pfx; RSA or ECDSA P-256, SHA-256, `adbe.pkcs7.detached`), invisible or visible with a generated appearance, reason/location/contact; signs pending edits; saved as the signed file immediately | Implemented | `m6_signing` (8 tests): poppler `pdfsig` reports “Signature is Valid / Total document signed”, OpenSSL `cms -verify` accepts, tamper detected; `session::sign_and_save` tests. Legacy 3DES and AES PKCS#12 both load. **The certificate/password picker and save dialog are native dialogs and were not driven headless** |
| Several signatures on one document; edits after signing keep earlier signatures intact | Implemented | `m6_signing::two_signatures…`, `edits_after_signing…` |
| **Signature check** (Sign → Signatures, and “Signed” chip): are the signed bytes unchanged, does the signature maths verify, was the file extended after signing | Implemented, **integrity only** | `m6_signing`; never checks certificate trust, expiry, revocation or the signing time (the dialog says so). RSA/P-256 + SHA-256 only; other algorithms show “could not be checked” |
| Certification (DocMDP) signatures, timestamps (RFC 3161), PAdES-LT/LTA, hardware tokens, OS certificate stores | Planned | not offered |
| Handwritten signature: draw once (saved locally), place as an ink annotation | Implemented | `handwriting` unit tests; GUI draw + place. **Only a picture** — no legal/cryptographic weight; the UI says so |
| **OCR** — scanned pages become searchable with an invisible text layer (300 dpi, offline, pure-Rust `ocrs`) | Implemented; models by verified in-app download or opt-in bundling (**redistribution licence unverified**) | `editor_core::ocr` test with real models + poppler text/word-box oracle (recall ≥ 80 %, words within 8 pt of where printed, picture unchanged); GUI: scan opened as PDF, OCR run, in-app search finds the text. Latin alphabet, ASCII-only recognition alphabet (accents lost); handwriting/rotated/vertical text poor; download/bundling tested against a local mock server and the real host (hashes verified); GUI download button not clicked end to end; licence must be reviewed before bundling |

## Third round (2026-10-05)

| Feature | Status | Evidence / gaps |
|---|---|---|
| **Right-click menu** in the page: Copy; Highlight / Underline / Strikeout / Squiggly; Find "…"; Explain / Translate (Copilot); on an annotation Delete / Duplicate / Properties; on empty page Add note, Select all text, Tool and View submenus | Implemented | GUI screenshots `14-…`, `15-…`, `16-…`, `28-…`; markup from the menu creates the annotation (undoable). Right-click selects the word / annotation under the pointer first |
| **Snap to drawing geometry** for Distance, Path, Area, Rectangle, Radius, Angle, Count and Calibrate: end points & corners, intersections, midpoints, nearest point on a line/curve; marker shape + caption; toggle in Measure ▸ Snap and Preferences | Implemented | `pdf_engine::snap` 8 unit tests, `m7_snap` 5 tests (Form XObjects at two placements, T-junction, crossing, door arc, 20 000-segment page: 2000 queries ≈ 25 ms debug); GUI `18-…` (measured 360.6 pt = √(300²+200²) between a snapped corner and junction), `19-…`. No circle centres, text or image edges; index capped at 600 000 segments; built in a background thread |
| **Save As Optimized** (File ▸ Optimize): lossless / balanced / smallest, report of what shrank | Implemented | `m7_optimize` 7 tests: 8.4 MB → 9.9 KB on a bloated fixture; lossless render is pixel-identical (poppler, mean diff 0); poppler reports no syntax errors; real Chromium/Cairo files never grow; second pass stable. Found and fixed: stale `/Length` after recompression (poppler diagnostics test now guards it). Limits: encrypted files refused; signatures lost; only 8-bit Gray/RGB JPEG/Flate images are resampled (no CCITT/JBIG2/predictor/Indexed/masked images); fonts are not re-subsetted. Native save picker not driven |
| **Convert to PDF/A-2b** (File ▸ PDF/A): preview of changes or the exact reasons it cannot be done | Implemented, **validated by veraPDF on the test set** | `m7_pdfa` 11 tests with veraPDF 1.28.2 *required*: Chromium and Cairo reports, PNG and gray-JPEG picture documents, transparency, documents with editor-made annotations, text added by the editor's embedded fonts, a file bolted full of JavaScript/Launch/attachments, double conversion, and the Translate export are all "compliant"; the originals and a deliberately broken output are rejected (so the validator is not a rubber stamp). **Refuses** unembedded fonts (e.g. plain Helvetica), CMYK without CMYK intent, annotations without appearance, encryption. Not offered: PDF/A-1/3, PDF/UA, font substitution, colour conversion. The app has no validator of its own; the result is "prepared", not "certified". Signatures are invalidated |
| **PDF Copilot** panel: summarize, main points, annotation summary, free questions, suggested questions, answers with page references; "p. N" jumps; verified quotes become highlights (one or all); Explain from the right-click menu | Implemented, **not yet run against a live provider with a valid key** | `ai-client` 25 tests incl. a local mock server (headers, bodies, 401/429/500/404, garbage, refusal, timeout, nothing sent on invalid config, quote verification and page correction, prompt shapes); GUI against a mock server: consent dialog, answer, flagged invented quote, "Highlight all" highlighting a passage wrapped over two lines (`23-…`, `24-…`); real Anthropic endpoint reached over TLS and rejected a dummy key with 401 (`tls-probe-…`). OpenAI endpoint not reachable from the sandbox. Quality of answers depends on the model and was not evaluated |
| **Translate document, inline**: local language auto-detection, target language, all/current page; paragraphs are found on each page, translated in batches, and put back **in place** in a copy that opens in a new tab (original unchanged); also save translated PDF / text / copy | Implemented, **same caveat as Copilot** (never run with a valid key) | `m11_inline_translate` 4 tests with poppler (paragraphs found in two producers' files, original words removed from the page content, translated text extractable at the same spot, no syntax errors; CJK via an installed font; unavailable characters reported as "?"), `ai-client` mock test (batching, JSON replies, fallback per block, cancel); GUI against the mock server (`36-…`: copy opens in a new tab, layout and bold headings kept). Limits: paragraph detection is heuristic (tables, multi-column, headers/footers may be split badly); text inside pictures is not touched (run OCR first); the original text is removed where the editor can edit the font and *covered* with the sampled background otherwise; a smaller font is used when the translation is longer; right-to-left scripts are not shaped |
| **AI key in Preferences** (OpenAI, Anthropic, Custom/local), model/address/answer language/characters-sent settings, Test connection, Remove key, "forget consent" | Implemented | `platform::secrets` tests (0600, atomic replace, rejects empty/multi-line); GUI: key saved to its own 0600 file and absent from `preferences.toml` (`27-…`). **Unencrypted file** (D-021) |
| **Installed (system) fonts** in every font picker: scanned in the background from the OS font folders (names and flags only), searchable list, embedded as subsets; fonts whose `fsType` forbids embedding or subsetting are left out; used automatically for translated characters the built-in fonts lack (e.g. Chinese) | Implemented on Linux; **not run on Windows/macOS** | `m10_sysfonts` 3 tests + 3 scanner unit tests (styles from a scanned folder, restricted fonts skipped, broken files ignored, embedded as a subset and extractable, remembered in a text box across saving, a covering font is found); GUI `35-…`. TrueType (`glyf`) outlines only: CFF `.otf` fonts are not offered. A text box in a font that is not installed on another computer falls back to DejaVu Sans when edited there. The `fsType` check is a technical flag check, not legal advice | 
| **Choose the font for text**: Liberation Sans / Serif / Mono and DejaVu Sans / Serif, with Bold and Italic (where the family has one), in Add Text, text boxes and callouts (also changeable later in Properties), and for *replacing* the font of an existing text run in Edit Text; the pickers and the text box preview each font; the last choice is the default | Implemented | `m8_fonts` 5 tests + 3 unit tests: all 16 faces embedded as subsets (poppler `pdffonts`), text extractable with accents/Greek/Cyrillic, sans/serif/italic render differently, text-box font survives save + reopen, replaced run reopens in the chosen face, missing-glyph message names the family, documents using every face convert to PDF/A and validate in veraPDF; GUI `29-…`–`32-…`. See the installed-fonts row for system fonts. No per-character formatting, no reflow |
| **Side panels**: collapse to a slim rail with one click (and View menu / F4 / F5), proper inner padding | Implemented | GUI `17-…` |
| **Open at the top, centred** (scroll and page reset, fit applied on first frame) | Partial | verified on the fixtures and after window resizes; the specific offset seen on the Intel NUC was **not reproduced**, so the root cause is unconfirmed (see PLAN) |

## Saving, safety, recovery

| Feature | Status | Evidence |
|---|---|---|
| Incremental save that preserves original bytes | Implemented | `m0_roundtrip`, `serialize` tests, poppler oracle |
| Atomic write: temp file → fsync → re-open/validate → rename | Implemented | `pdf_engine::save`, `editor_core::session::failed_save_keeps_the_original_intact` |
| External modification detection | Implemented | `save` tests |
| Unlimited undo/redo with “clean” tracking | Implemented | `session` tests |
| Autosave recovery files + startup recovery dialog | Implemented | `platform::recovery` tests; GUI (restore flow) |
| Crash isolation of the render workers | Partial | worker **threads** with panic containment (`jobs` tests); **not** a separate process |
| Secure **redaction** | **Unavailable (gated)** | no feature offers it; “black rectangle” annotations are never presented as redaction. (OCR and signing are now implemented; redaction remains gated.) |
| Encryption (password-protected PDFs) | Not supported | no decryption |

## Productivity and platform

| Feature | Status | Evidence |
|---|---|---|
| Command palette (searchable commands + settings) | Implemented | `command` tests; GUI |
| Remappable shortcuts with conflict detection, per-platform defaults | Implemented | `command` tests; GUI |
| Progressive-disclosure UI (Essential / Professional workspace) | Implemented | GUI |
| Density (compact/comfortable/touch), UI scale, theme | Implemented | GUI |
| Preferences stored locally, searchable | Implemented | `prefs` tests |
| No telemetry, no network use in core flows | Implemented | no network crates in the runtime graph other than what eframe/wgpu pull for windowing; see `SECURITY.md` |
| Windows 11 x86_64 / macOS Apple Silicon | **Untested** | `PLATFORM_CHECKLIST.md` |
| Printing | Planned | no implementation |
| File associations / “Open with” / Finder & Explorer integration | Planned | |
| Installer / signed packages | Planned | `cargo xtask dist` makes an *unsigned* folder or `.app` layout (layout unit-tested; never launched on macOS) |

## Fourth round (2026-10-05)

| Feature | Status | Evidence / gaps |
|---|---|---|
| **Callout in two clicks** with a live preview: click the point, then click where the box goes; the box follows the pointer with its leader line | Implemented | GUI `33-…` (on a landscape page) |
| **Text boxes, callouts, stamps and added text are upright on rotated pages** and can be rotated (⟲ ⟳ / Upright in Properties; Direction in Add Text); the box turns with its text | Implemented | `m9_rotation` 7 tests with poppler word boxes on pages shown at 0/90/180/270° (horizontal when upright, vertical when not, stamps too, rotation survives saving, callout box no longer swallows its leader line on re-open); GUI `34-…`. Measurement labels are not rotated |
| **Properties sliders apply on release** (line width, opacity, colours; arrow keys on key release), interface scale slider too | Implemented | GUI-checked only by use; no automated test for pointer timing |
| **No blank flashes while zooming/editing**: nearest cached tiles are drawn scaled until the new ones arrive | Implemented | reasoned from the tile cache; not measured on a GPU |
| **AI model dropdown** per provider with a cheap default (Haiku 4.5; GPT-4.1 mini), "Other…" for any model id | Implemented | list is a snapshot of known model names; unavailable ones are the provider's error |
| **Graphics settings** (Preferences): adapter in use, drawing API (Auto / DirectX 12 / Vulkan / OpenGL, after restart), frame pacing (vsync / low latency / uncapped, immediate); notice when only a software renderer is available | Implemented | adapter line seen under Mesa lavapipe; the resize slowness reported on Windows was **not reproduced** (see PLAN) |
