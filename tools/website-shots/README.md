# Screens for the website's feature pages

`run.mjs` starts a **debug** build of the app once per screen, in English and Dutch, light and dark, and writes
`out/<lang>-<Theme>/<name>.png` (2380 x 1540 on a 175 % display, the size of the other website screens). The
scenarios are in `apps/desktop/src/debug_shots.rs` (`BERG_SHOTS`, see DEVELOPING.md); this folder only drives them.

```
cargo build -p bergpdf
cargo run -p test-support --example gen_fixtures -- tools/website-shots/docs      # floor_plan, form_rich, helvetica_lines
cp tests/fixtures/report-chromium.pdf tools/website-shots/docs/
cargo run -p pdf-engine --example make_scan -- tools/website-shots/docs/report-chromium.pdf tools/website-shots/docs/scan.pdf
node tools/website-shots/run.mjs                      # everything
node tools/website-shots/run.mjs en-Light -- optimize  # one variant, one screen
```

- The app runs with its own profile (`SHOTS_PROFILE`, default `C:/Users/Public`) and `BERG_MULTI_INSTANCE=1`, so your
  own preferences and an already running BergPDF are not touched. The profile's `preferences.toml` is rewritten for every run.
- `ocr-search` needs the OCR models (`SHOTS_OCR_MODELS`, default `%APPDATA%/BergPDF/ocr-models`; `cargo xtask
  fetch-ocr-models` puts them there). `ocr-models` is run with an empty models folder to show the download offer.
- Nothing is sent anywhere. Copilot and the consent/translate dialogs use a dummy key and are never asked to send;
  `translate-inline` talks to `mock-ai.mjs` on `127.0.0.1:8099`, which "translates" the text of `report-chromium.pdf` into Dutch.
- Other settings: `SHOTS_EXE`, `SHOTS_DOCS`, `SHOTS_OUT`.

The website wants them as `src/assets/app/<en|nl>/<light|dark>/<name>.webp` (quality 82).
