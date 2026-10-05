# Test fixtures — provenance

Never fabricate or "improve" a fixture to make a test pass; regenerate it with the commands here.

## Real files from independent producers (checked in)

| File | Producer | How it was made |
|---|---|---|
| `report-chromium.pdf` | Chromium headless (Skia/PDF) | Chromium headless “print to PDF” of `src/report.html` with default Skia/PDF output (exact flags were not recorded; regenerating may produce a byte-different but equivalent file) — fonts: DejaVu Sans/Serif/Mono as Type0 Identity-H subsets; one `Tj` per glyph with `Td`. |
| `report-cairo.pdf` | poppler `pdftocairo` | `pdftocairo -pdf report-chromium.pdf report-cairo.pdf` — re-render through Cairo: simple TrueType WinAnsi subsets, `TJ` arrays, one `Tm` per line. |
| `src/report.html` | written for this repository | original text (no third-party content) |

Three pages: "Quarterly Report" (ligature text), "Revenue Overview" (Unicode, a table),
"Appendix" (the known string `ALPHA-7731` used by text-editing tests).

## Programmatic fixtures (`crates/test-support/src/fixtures.rs`, built with `pdf-writer`)

`helvetica_lines`, `shared_content_stream`, `image_page`, `odd_geometry` (rotation, crop box,
flipped CTM), `mixed_sizes`, `reused_form_and_transparency`, `existing_annotations`,
`acroform_basic`, `form_rich` (hierarchical names, radio group, combo with export/display pairs),
`signature_structure_only` (detection tests only — **not** a valid signature), `malformed`,
`many_pages(n)` and `vector_a0(n)` (performance).

Write them to disk for manual checks: `cargo xtask fixtures` → `target/fixtures/`.

## Oracles

Tests that call poppler (`pdftotext`, `pdftoppm`) skip those assertions when the tool is not
installed, **unless** `FERRUM_REQUIRE_ORACLES=1` is set (CI sets it on Linux/macOS), in which
case a missing tool or a mismatch fails the test.
