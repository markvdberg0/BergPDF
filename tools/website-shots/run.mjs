// Draws the screens of the website's feature pages with the app's debug screenshot mode (see README.md),
// in English and Dutch, light and dark.
//
//   node tools/website-shots/run.mjs [variant ...] [-- job ...]
//
// variants: en-Light en-Dark nl-Light nl-Dark (none = all); jobs: names in JOBS below (none = all).
// Settings come from the environment, see README.md.
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const env = (k, d) => process.env[k] || d;
const EXE = env('SHOTS_EXE', path.resolve(HERE, '../../target/debug/bergpdf.exe'));
const DOCS = env('SHOTS_DOCS', path.join(HERE, 'docs'));
const OUT = env('SHOTS_OUT', path.join(HERE, 'out'));
// The per-user folders the app is started with; the paths the app prints (OCR models folder) show this.
const PROFILE = env('SHOTS_PROFILE', 'C:/Users/Public');
const MODELS = env('SHOTS_OCR_MODELS', path.join(env('APPDATA', ''), 'BergPDF/ocr-models'));
const EMPTY_MODELS = path.join(PROFILE, 'AppData/Roaming/BergPDF/ocr-models');
const BS = String.fromCharCode(92);
const win = (p) => p.replaceAll('/', BS);

// out: file name on the website; scenario: name in apps/desktop/src/debug_shots.rs; doc: file in DOCS.
const JOBS = [
  { out: 'copilot-answer', scenario: 'copilot_answer', doc: 'report-chromium.pdf', ai: true },
  { out: 'copilot-consent', scenario: 'copilot_consent', doc: 'report-chromium.pdf', ai: true },
  { out: 'preferences-ai', scenario: 'prefs_ai', doc: 'report-chromium.pdf', ai: true },
  { out: 'translate-dialog', scenario: 'translate_dialog', doc: 'report-chromium.pdf', ai: true },
  { out: 'translate-inline', scenario: 'translate_inline', doc: 'report-chromium.pdf', ai: true, mock: true },
  { out: 'ocr-models', scenario: 'ocr_models', doc: 'scan.pdf', models: EMPTY_MODELS },
  { out: 'ocr-search', scenario: 'ocr_search', doc: 'scan.pdf', models: MODELS },
  { out: 'measure-floorplan', scenario: 'measure_plan', doc: 'floor_plan.pdf' },
  { out: 'measure-area-count', scenario: 'measure_panel', doc: 'floor_plan.pdf' },
  { out: 'optimize', scenario: 'optimize', doc: 'report-chromium.pdf' },
  { out: 'pdfa-convert', scenario: 'pdfa', doc: 'report-chromium.pdf' },
  { out: 'pdfa-refused', scenario: 'pdfa', doc: 'helvetica_lines.pdf' },
  { out: 'forms-policy', scenario: 'forms_policy', doc: 'form_rich.pdf' },
  { out: 'crash-recovery', scenario: 'recovery', doc: 'report-chromium.pdf' },
];
const LANG = { en: 'English', nl: 'Dutch' };

const argv = process.argv.slice(2);
const cut = argv.indexOf('--');
const [vArgs, jArgs] = cut < 0 ? [argv, []] : [argv.slice(0, cut), argv.slice(cut + 1)];
const variants = [];
for (const l of ['en', 'nl']) for (const t of ['Light', 'Dark']) if (!vArgs.length || vArgs.includes(`${l}-${t}`)) variants.push([l, t]);
const jobs = JOBS.filter((j) => !jArgs.length || jArgs.includes(j.out));

const prefs = (lang, theme, mock) => `theme = "${theme}"
language = "${LANG[lang]}"
density = "Comfortable"
ui_scale = 1.0
workspace = "Professional"
default_zoom = "FitPage"
first_run_done = true
update_check = "Off"
author = "Anna de Vries"
recent_files = []

[ai]
provider = "${mock ? 'Custom' : 'Anthropic'}"
translate_to = "Dutch"
${mock ? 'base_url = "http://127.0.0.1:8099/v1"\nmodel = "mock-model"\n' : ''}`;

let mockProc = null;
const startMock = () => {
  mockProc ??= spawn('node', [path.join(HERE, 'mock-ai.mjs'), '8099'], { stdio: 'inherit' });
};

for (const [lang, theme] of variants) {
  for (const job of jobs) {
    const roaming = path.join(PROFILE, 'AppData/Roaming/BergPDF');
    fs.rmSync(EMPTY_MODELS, { recursive: true, force: true });
    fs.rmSync(path.join(roaming, 'recovery'), { recursive: true, force: true });
    fs.mkdirSync(roaming, { recursive: true });
    if (job.models === EMPTY_MODELS) fs.mkdirSync(EMPTY_MODELS, { recursive: true });
    fs.writeFileSync(path.join(roaming, 'preferences.toml'), prefs(lang, theme, job.mock));
    if (job.mock) startMock();
    const dir = path.join(OUT, `${lang}-${theme}`, `_${job.out}`);
    fs.rmSync(dir, { recursive: true, force: true });
    const e = {
      ...process.env,
      APPDATA: win(path.join(PROFILE, 'AppData/Roaming')),
      LOCALAPPDATA: win(path.join(PROFILE, 'AppData/Local')),
      BERG_MULTI_INSTANCE: '1',
      BERG_SHOTS: `${win(dir)};${job.scenario}`,
    };
    if (job.ai) e.BERGPDF_AI_KEY = 'sk-ant-api03-demo-key-not-used';
    if (job.models) e.BERG_OCR_MODELS = win(job.models);
    const t0 = Date.now();
    const r = spawnSync(EXE, [path.join(DOCS, job.doc)], { env: e, timeout: 240000, encoding: 'utf8' });
    const png = path.join(dir, `${job.scenario}.png`);
    const dest = path.join(OUT, `${lang}-${theme}`, `${job.out}.png`);
    if (fs.existsSync(png)) {
      fs.renameSync(png, dest);
      fs.rmSync(dir, { recursive: true, force: true });
      console.log('ok  ', lang, theme, job.out, `${((Date.now() - t0) / 1000).toFixed(1)}s`);
    } else {
      console.log('FAIL', lang, theme, job.out, 'status', r.status, r.signal, (r.stderr || '').slice(-400));
    }
  }
}
mockProc?.kill();
fs.rmSync(EMPTY_MODELS, { recursive: true, force: true });
