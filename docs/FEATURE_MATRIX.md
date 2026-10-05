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
| OCR | **Unavailable** | needs its own gate: engine, models, licences, language packs |

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
| Signature fields | Displayed only | **Signing and signature validation are out of scope** (own gate); modified signed documents warn on open |

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

## Saving, safety, recovery

| Feature | Status | Evidence |
|---|---|---|
| Incremental save that preserves original bytes | Implemented | `m0_roundtrip`, `serialize` tests, poppler oracle |
| Atomic write: temp file → fsync → re-open/validate → rename | Implemented | `pdf_engine::save`, `editor_core::session::failed_save_keeps_the_original_intact` |
| External modification detection | Implemented | `save` tests |
| Unlimited undo/redo with “clean” tracking | Implemented | `session` tests |
| Autosave recovery files + startup recovery dialog | Implemented | `platform::recovery` tests; GUI (restore flow) |
| Crash isolation of the render workers | Partial | worker **threads** with panic containment (`jobs` tests); **not** a separate process |
| Secure **redaction** | **Unavailable (gated)** | no feature offers it; “black rectangle” annotations are never presented as redaction |
| Digital signatures (create/validate), encryption | Planned / out of scope | |

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
