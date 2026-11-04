// Draws the Microsoft Store screenshots with the app's debug screenshot mode (BERG_SHOTS, see
// apps/desktop/src/debug_shots.rs): every interface language, light and dark.
//
//   node tools/store-shots/run.mjs [variant ...] [-- job ...]
//
// variants: en-Light en-Dark nl-Light … zh-Dark (none = all eight); jobs: names in JOBS below (none = all).
// Writes <OUT>/<Language>/<Theme>/<nn-name>-<theme>.png. Same settings as tools/website-shots (SHOTS_EXE,
// SHOTS_DOCS, SHOTS_PROFILE, SHOTS_OCR_MODELS) plus SHOTS_OUT. The documents are made as in
// tools/website-shots/README.md.
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const env = (k, d) => process.env[k] || d;
const EXE = env('SHOTS_EXE', path.resolve(HERE, '../../target/debug/bergpdf.exe'));
const DOCS = env('SHOTS_DOCS', path.resolve(HERE, '../website-shots/docs'));
const OUT = env('SHOTS_OUT', path.join(HERE, 'out'));
const PROFILE = env('SHOTS_PROFILE', 'C:/Users/Public');
const MODELS = env('SHOTS_OCR_MODELS', path.join(env('APPDATA', ''), 'BergPDF/ocr-models'));
const BS = String.fromCharCode(92);
const win = (p) => p.replaceAll('/', BS);

// Files the start screen lists, newest first, with how long ago they were opened (seconds).
const RECENT = [
  ['report-chromium.pdf', 20 * 60],
  ['form_rich.pdf', 3 * 3600],
  ['floor_plan.pdf', 26 * 3600],
  ['scan.pdf', 3 * 86400],
  ['helvetica_lines.pdf', 9 * 86400],
];

const JOBS = [
  { out: '01-start-screen', scenario: 'store_start', doc: null },
  { out: '02-edit-text', scenario: 'dblclick_text', doc: 'report-chromium.pdf' },
  { out: '03-comments-and-stamps', scenario: 'store_annotate', doc: 'report-chromium.pdf' },
  { out: '04-forms', scenario: 'store_forms', doc: 'form_rich.pdf' },
  { out: '05-headers-and-watermarks', scenario: 'store_marks', doc: 'report-chromium.pdf' },
  { out: '06-compare', scenario: 'store_compare', doc: null },
  { out: '07-measure', scenario: 'measure_plan', doc: 'floor_plan.pdf' },
  // Needs the OCR models (cargo xtask fetch-ocr-models), so only when asked for: -- 08-ocr
  { out: '08-ocr', scenario: 'ocr_search', doc: 'scan.pdf', models: true, optional: true },
];
const LANG = { en: 'English', nl: 'Dutch', de: 'German', zh: 'Chinese' };
const FOLDER = { en: 'English', nl: 'Nederlands', de: 'Deutsch', zh: '中文 (简体)' };

const argv = process.argv.slice(2);
const cut = argv.indexOf('--');
const [vArgs, jArgs] = cut < 0 ? [argv, []] : [argv.slice(0, cut), argv.slice(cut + 1)];
const variants = [];
for (const l of Object.keys(LANG)) for (const t of ['Light', 'Dark']) if (!vArgs.length || vArgs.includes(`${l}-${t}`)) variants.push([l, t]);
const jobs = JOBS.filter((j) => (jArgs.length ? jArgs.includes(j.out) : !j.optional));

const now = Math.floor(Date.now() / 1000);
const q = (s) => JSON.stringify(s);
const prefs = (lang, theme) => `theme = "${theme}"
language = "${LANG[lang]}"
density = "Comfortable"
ui_scale = 1.0
workspace = "Professional"
default_zoom = "FitPage"
first_run_done = true
update_check = "Off"
author = "Anna de Vries"
recent_files = [${RECENT.map(([f]) => q(win(path.join(DOCS, f)))).join(', ')}]
pinned_files = [${q(win(path.join(DOCS, 'form_rich.pdf')))}]

[recent_opened]
${RECENT.map(([f, ago]) => `${q(win(path.join(DOCS, f)))} = ${now - ago}`).join('\n')}
`;

const roaming = path.join(PROFILE, 'AppData/Roaming/BergPDF');
const run = (lang, theme, job, dir, docs) => {
  fs.rmSync(path.join(roaming, 'recovery'), { recursive: true, force: true });
  fs.mkdirSync(roaming, { recursive: true });
  fs.writeFileSync(path.join(roaming, 'preferences.toml'), prefs(lang, theme));
  fs.rmSync(dir, { recursive: true, force: true });
  const e = {
    ...process.env,
    APPDATA: win(path.join(PROFILE, 'AppData/Roaming')),
    LOCALAPPDATA: win(path.join(PROFILE, 'AppData/Local')),
    BERG_MULTI_INSTANCE: '1',
    BERG_SHOTS: `${win(dir)};${job.scenario}`,
  };
  if (job.models) e.BERG_OCR_MODELS = win(MODELS);
  return spawnSync(EXE, docs, { env: e, timeout: 240000, encoding: 'utf8' });
};

// The start screen shows a picture of page 1 of each recent file; closing a document makes it.
if (jobs.some((j) => j.scenario === 'store_start')) {
  const tmp = path.join(OUT, '_warmup');
  run('en', 'Light', JOBS[0], tmp, RECENT.map(([f]) => path.join(DOCS, f)));
  fs.rmSync(tmp, { recursive: true, force: true });
}

for (const [lang, theme] of variants) {
  for (const job of jobs) {
    const dir = path.join(OUT, '_' + job.out);
    const t0 = Date.now();
    const r = run(lang, theme, job, dir, job.doc ? [path.join(DOCS, job.doc)] : []);
    const png = path.join(dir, `${job.scenario}.png`);
    const destDir = path.join(OUT, FOLDER[lang], theme);
    fs.mkdirSync(destDir, { recursive: true });
    if (fs.existsSync(png)) {
      fs.renameSync(png, path.join(destDir, `${job.out}-${theme.toLowerCase()}.png`));
      fs.rmSync(dir, { recursive: true, force: true });
      console.log('ok  ', lang, theme, job.out, `${((Date.now() - t0) / 1000).toFixed(1)}s`);
    } else {
      console.log('FAIL', lang, theme, job.out, 'status', r.status, r.signal, (r.stderr || '').slice(-400));
    }
  }
}
