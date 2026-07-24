// _hooks.mjs v0.5 — автономный стенд + testlevel-хуки + title slides + per-context badge
//
// `prepare()` поднимает изолированный стенд по smart-логике:
//   1) Если нужно пересоздавать БД (config-rebuild или --reload-data) — web-stop
//      (Apache держит блокировку БД).
//   2) [config-hash изменился или --rebuild-config]    → пересобрать XML.
//   3) [нужна пересборка БД]                            → drop+create+load+update.
//   4) [epf-hash изменился или --rebuild-epf]           → пересобрать EPF.
//   5) Apache:
//      - если БД пересоздавалась → web-publish + probe ready.
//      - иначе probe-first: жив → ничего не делаем; мёртв → publish + probe.
//
// Идемпотентность — через sha256-локи в `tests/skills/.cache/webtest-stand/`.
// На warm-старте (ничего не менялось, Apache жив) prepare() сводится к ~200ms:
// чтение локов + probe.
//
// Поддерживаемые hookArgs (`node run.mjs test ... -- <args>`):
//   --rebuild-config   принудительно пересобрать XML + БД
//   --reload-data      принудительно пересоздать БД из существующего XML
//   --rebuild-epf      принудительно пересобрать EPF
//   --rebuild-stand    эквивалент всех трёх флагов сразу
//
// Cross-platform: на не-Windows можно задать env WEBTEST_HOOKS_RUNTIME=python,
// тогда зеркальные py-порты скиллов будут вызваны вместо ps1.

import { existsSync, mkdirSync, rmSync, readFileSync, writeFileSync, statSync } from 'fs';
import { join, resolve, dirname } from 'path';
import { fileURLToPath } from 'url';
import { createHash } from 'crypto';
import {
  getProjectInfo,
  loadBuildSteps,
  platformLoadSteps,
  runSteps,
  execSkill,
  resolveScript,
} from '../skills/build-webtest-db.mjs';

const __filename = fileURLToPath(import.meta.url);
const REPO_ROOT  = resolve(dirname(__filename), '../..');
const LOCK_DIR   = join(REPO_ROOT, 'tests/skills/.cache/webtest-stand');

// ── Configurable knobs ─────────────────────────────────────────────────────────

const APACHE_APPNAME = 'webtest-runner';
const APACHE_PORT    = 9191;
const READY_URL      = `http://localhost:${APACHE_PORT}/${APACHE_APPNAME}/ru_RU/`;
const READY_TIMEOUT  = 30_000;
const RUNTIME        = process.env.WEBTEST_HOOKS_RUNTIME || 'powershell';

// EPF spec: версионируется через epf.lock (sha256 от JSON.stringify(this)).
// Любое изменение → автоматический rebuild.
const EPF_SPEC = {
  v8path: 'C:\\Program Files\\1cv8\\8.3.24.1691\\bin',
  srcDir:   'test-tmp/13-openfile/src',
  buildDir: 'test-tmp/13-openfile/build',
  name: 'ТестОткрытия',
  synonym: 'Тест открытия из файла',
  formName: 'Форма',
  form: {
    title: 'Тест открытия',
    elements: [
      { label: 'Заголовок', title: 'Это тестовая обработка для проверки openFile' },
    ],
  },
};

// ── Args parsing ──────────────────────────────────────────────────────────────

function parseHookArgs(hookArgs) {
  const out = { rebuildConfig: false, reloadData: false, rebuildEpf: false, rebuildStand: false };
  for (const a of hookArgs || []) {
    if (a === '--rebuild-config') out.rebuildConfig = true;
    else if (a === '--reload-data')   out.reloadData = true;
    else if (a === '--rebuild-epf')   out.rebuildEpf = true;
    else if (a === '--rebuild-stand') out.rebuildStand = true;
  }
  if (out.rebuildStand) {
    out.rebuildConfig = true;
    out.reloadData    = true;
    out.rebuildEpf    = true;
  }
  return out;
}

// ── Hash-lock helpers ─────────────────────────────────────────────────────────

function sha256(s) {
  return createHash('sha256').update(s, 'utf8').digest('hex');
}

function readLock(name) {
  const f = join(LOCK_DIR, `${name}.lock`);
  return existsSync(f) ? readFileSync(f, 'utf8').trim() : null;
}

function writeLock(name, hash) {
  mkdirSync(LOCK_DIR, { recursive: true });
  writeFileSync(join(LOCK_DIR, `${name}.lock`), hash + '\n', 'utf8');
}

// ── Apache helpers ────────────────────────────────────────────────────────────

async function webStop(log) {
  try {
    const script = resolveScript('web-stop/scripts/web-stop', RUNTIME);
    await execSkill(script, [], RUNTIME);
    log('apache stopped');
  } catch (e) {
    log(`apache stop: ${e.message.split('\n')[0]}`);
  }
}

async function webPublish(dbPath, v8path, log) {
  const script = resolveScript('web-publish/scripts/web-publish', RUNTIME);
  await execSkill(script, [
    '-InfoBasePath', dbPath,
    '-V8Path',       v8path,
    '-Port',         String(APACHE_PORT),
    '-AppName',      APACHE_APPNAME,
  ], RUNTIME);
  log(`apache published: ${READY_URL}`);
}

async function probeReady(url, timeoutMs, log) {
  const t0 = Date.now();
  let attempt = 0;
  while (Date.now() - t0 < timeoutMs) {
    attempt++;
    try {
      const res = await fetch(url, { signal: AbortSignal.timeout(2000) });
      if (res.status >= 200 && res.status < 500) {
        log(`ready after ${((Date.now() - t0) / 1000).toFixed(1)}s (status=${res.status}, attempts=${attempt})`);
        return;
      }
    } catch { /* retry */ }
    await new Promise(r => setTimeout(r, 500));
  }
  throw new Error(`Apache not ready at ${url} within ${timeoutMs}ms`);
}

// Лёгкий probe для одной попытки — для проверки «жив ли Apache уже сейчас».
// Возвращает true если в течение `timeoutMs` пришёл ответ 200-499 (т.е. сервер
// откликается). Не бросает — fail-quiet.
async function probeAlive(url, timeoutMs = 1500) {
  try {
    const res = await fetch(url, { signal: AbortSignal.timeout(timeoutMs) });
    return res.status >= 200 && res.status < 500;
  } catch {
    return false;
  }
}

// ── EPF build ─────────────────────────────────────────────────────────────────

async function buildEpf(spec, log) {
  const srcDir   = resolve(REPO_ROOT, spec.srcDir);
  const buildDir = resolve(REPO_ROOT, spec.buildDir);
  const srcXml   = join(srcDir, `${spec.name}.xml`);
  const epfPath  = join(buildDir, `${spec.name}.epf`);
  const formDir  = join(srcDir, `${spec.name}/Forms/${spec.formName}`);
  const formXml  = join(formDir, 'Ext/Form.xml');

  // Полный rebuild: чистим и собираем заново.
  if (existsSync(srcDir))   rmSync(srcDir,   { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  if (existsSync(buildDir)) rmSync(buildDir, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  mkdirSync(srcDir, { recursive: true });
  mkdirSync(buildDir, { recursive: true });

  // 1. epf-init
  await execSkill(
    resolveScript('epf-init/scripts/init', RUNTIME),
    ['-Name', spec.name, '-Synonym', spec.synonym, '-SrcDir', srcDir],
    RUNTIME,
  );
  log('epf-init OK');

  // 2. form-add
  await execSkill(
    resolveScript('form-add/scripts/form-add', RUNTIME),
    ['-ObjectPath', srcXml, '-FormName', spec.formName],
    RUNTIME,
  );
  log('form-add OK');

  // 3. form-compile
  const formJsonPath = join(buildDir, '__form.json');
  writeFileSync(formJsonPath, JSON.stringify(spec.form, null, 2), 'utf8');
  await execSkill(
    resolveScript('form-compile/scripts/form-compile', RUNTIME),
    ['-JsonPath', formJsonPath, '-OutputPath', formXml],
    RUNTIME,
  );
  rmSync(formJsonPath);
  log('form-compile OK');

  // 4. epf-build
  await execSkill(
    resolveScript('epf-build/scripts/epf-build', RUNTIME),
    ['-SourceFile', srcXml, '-OutputFile', epfPath, '-V8Path', spec.v8path],
    RUNTIME,
  );
  if (!existsSync(epfPath)) throw new Error(`epf-build did not produce ${epfPath}`);
  log(`epf-build OK (${statSync(epfPath).size} bytes)`);
  return epfPath;
}

function epfArtifactExists(spec) {
  const epfPath = resolve(REPO_ROOT, spec.buildDir, `${spec.name}.epf`);
  return existsSync(epfPath);
}

// ── prepare / cleanup ─────────────────────────────────────────────────────────

export async function prepare({ hookArgs, log, config }) {
  const flags = parseHookArgs(hookArgs);
  const t0 = Date.now();
  log(`stand prepare: flags=${JSON.stringify(flags)} runtime=${RUNTIME}`);

  // Project info (paths, db registration)
  const { v8path, v8exe, configSrc, dbPath } = getProjectInfo();
  if (!existsSync(v8exe)) throw new Error(`1cv8.exe not found at ${v8exe} (check .v8-project.json v8path)`);

  // Hashes
  const buildSteps   = await loadBuildSteps();
  const configHash   = sha256(JSON.stringify(buildSteps));
  const epfHash      = sha256(JSON.stringify(EPF_SPEC));
  const prevConfig   = readLock('config');
  const prevEpf      = readLock('epf');

  const needConfig = flags.rebuildConfig || prevConfig !== configHash;
  const needData   = needConfig || flags.reloadData;
  const needEpf    = flags.rebuildEpf || prevEpf !== epfHash || !epfArtifactExists(EPF_SPEC);

  log(`config-hash=${configHash.slice(0, 12)}... prev=${prevConfig?.slice(0, 12) || 'none'}... ${needConfig ? 'REBUILD' : 'skip'}`);
  log(`epf-hash=${epfHash.slice(0, 12)}... prev=${prevEpf?.slice(0, 12) || 'none'}... ${needEpf ? 'REBUILD' : 'skip'}`);
  log(`data-${needData ? 'RELOAD' : 'skip'}`);

  // 1. Apache stop — только если будем пересоздавать БД (Apache держит lock-файл).
  //    На warm-старте (никаких rebuild) — НЕ трогаем Apache, иначе впустую отдадим
  //    5-8 секунд на restart при каждом прогоне.
  if (needData) {
    await webStop(log);
  }

  // 2. Config rebuild
  if (needConfig) {
    log(`rebuild config XML → ${configSrc}`);
    if (existsSync(configSrc)) rmSync(configSrc, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
    mkdirSync(configSrc, { recursive: true });
    const paths = { workDir: configSrc, v8path, dbPath };
    const r = await runSteps(buildSteps, paths, RUNTIME, log);
    if (!r.ok) throw new Error(`config rebuild failed at step #${r.failedAt + 1}`);
    writeLock('config', configHash);
  }

  // 3. DB reload
  if (needData) {
    log(`reload DB → ${dbPath}`);
    if (existsSync(dbPath)) rmSync(dbPath, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
    const paths = { workDir: configSrc, v8path, dbPath };
    const r = await runSteps(platformLoadSteps(), paths, RUNTIME, log);
    if (!r.ok) throw new Error(`DB reload failed at step #${r.failedAt + 1}`);
  }

  // 4. EPF rebuild
  if (needEpf) {
    log('rebuild EPF');
    await buildEpf(EPF_SPEC, log);
    writeLock('epf', epfHash);
  }

  // 5. Apache: publish + probe (smart logic)
  //    - needData=true → Apache был остановлен в #1, нужно публиковать заново
  //    - needData=false → probe сначала: если жив, ничего не делаем (warm-старт);
  //                       если мёртв (упал/не поднимали) → publish
  if (needData) {
    await webPublish(dbPath, v8path, log);
    await probeReady(READY_URL, READY_TIMEOUT, log);
  } else if (await probeAlive(READY_URL)) {
    log(`apache already live at ${READY_URL} (warm start)`);
  } else {
    log(`apache not responding — publishing`);
    await webPublish(dbPath, v8path, log);
    await probeReady(READY_URL, READY_TIMEOUT, log);
  }

  log(`stand ready in ${((Date.now() - t0) / 1000).toFixed(1)}s`);
}

export async function cleanup({ log }) {
  // MVP: оставляем стенд поднятым для отладки. Для full-shutdown — ручной /web-stop
  // или следующий запуск с --rebuild-stand.
  log('cleanup: stand left running (use /web-stop or run with `-- --rebuild-stand` to reset)');
}

// ── Testlevel hooks (M7.4) ────────────────────────────────────────────────────
//
// Shared mutable state, импортируется индикатором `00-hooks.test.mjs` для
// проверки порядка вызовов. Хуки — counter-only, никакой реальной работы:
// `prepare()` уже подготовил стенд, дефолтное приземление 1С после входа
// уже показывает панель разделов (разведка 2026-05-13: navigateSection
// в beforeAll не нужен).
//
// `events` — последовательность строк, по которой индикатор восстанавливает
// порядок (`beforeAll`, `beforeEach:01-navigation.test.mjs`, ...).

export const _state = {
  beforeAll: 0,
  afterAll: 0,
  beforeEach: 0,
  afterEach: 0,
  afterOpenContext: 0,
  beforeCloseContext: 0,
  events: [],
  lastTestResult: null,
};

export async function beforeAll(_ctx) {
  _state.beforeAll++;
  _state.events.push('beforeAll');
}

export async function afterAll(_ctx) {
  _state.afterAll++;
  _state.events.push('afterAll');
}

// Длительность показа title slide перед телом теста (секунды). Эмпирически
// 1.5с хватает чтобы в записанном видео слайд успел зацепиться кадром,
// и не слишком долго на тестах вроде 14-routing (~2.5с целиком).
const TITLE_SLIDE_SEC = 1.5;

export async function beforeEach(ctx) {
  _state.beforeEach++;
  _state.events.push(`beforeEach:${ctx.testInfo?.file || '?'}`);

  // M7.5: title slide для `--record`-прогонов. Под обычным регрессом
  // (isRecording === false) пропускаем — лишние ~1.5s × N тестов
  // не нужны.
  if (ctx.isRecording?.()) {
    const info = ctx.testInfo;
    const primary = info.contexts?.[info.primaryContext];
    const subtitle = primary?.displayName || '';
    try {
      await ctx.showTitleSlide(info.name, { subtitle });
      await ctx.wait(TITLE_SLIDE_SEC);
      await ctx.hideTitleSlide();
    } catch {
      // Не валим тест из-за оформления — recorder/page-state могут
      // не сложиться в редких сценариях (race на старте контекста).
    }
  }
}

export async function afterEach(ctx) {
  _state.afterEach++;
  // Снимок testResult без тяжёлого steps[]: индикатор проверяет только
  // status/duration/attempts/error.
  if (ctx.testResult) {
    const { status, duration, attempts, error } = ctx.testResult;
    _state.lastTestResult = { status, duration, attempts, error };
  } else {
    _state.lastTestResult = null;
  }
  _state.events.push(`afterEach:${ctx.testInfo?.file || '?'}:${ctx.testResult?.status || '?'}`);
}

// ── Per-context hooks (M8) ────────────────────────────────────────────────────
//
// `afterOpenContext` инжектит persistent DOM-badge с displayName в правый
// верхний угол страницы контекста — в записанном видео всегда видно, какая
// вкладка к какому пользователю относится. Badge переживает любые
// перерисовки 1С (это собственный div с z-index, не часть SPA).
//
// `beforeCloseContext` — counter-only (страница вот-вот закроется, делать
// что-либо с DOM бессмысленно).

async function injectContextBadge(ctx, name, spec) {
  const label = spec?.displayName || name;
  // ctx может быть scoped (auto-setActiveContext) или flat — в любом случае
  // getPage() возвращает активную страницу, которая на момент afterOpenContext
  // = только что созданный контекст.
  const page = ctx.getPage?.();
  if (!page) return;
  await page.evaluate((text) => {
    let div = document.getElementById('__web_test_ctx_badge');
    if (!div) {
      div = document.createElement('div');
      div.id = '__web_test_ctx_badge';
      document.body.appendChild(div);
    }
    div.style.cssText = [
      'position:fixed', 'top:8px', 'right:8px',
      'padding:4px 10px',
      'background:rgba(30,30,46,0.85)', 'color:#fff',
      'font:600 13px Segoe UI,Arial,sans-serif',
      'border-radius:4px', 'box-shadow:0 2px 6px rgba(0,0,0,0.25)',
      'z-index:999998', 'pointer-events:none',
      'letter-spacing:0.3px',
    ].join(';');
    div.textContent = text;
  }, label);
}

export async function afterOpenContext(ctx, name, spec) {
  _state.afterOpenContext++;
  _state.events.push(`afterOpenContext:${name}`);
  try {
    await injectContextBadge(ctx, name, spec);
  } catch {
    // Не валим прогон если badge не сел — это чисто визуальный bonus.
  }
}

export async function beforeCloseContext(_ctx, name, _spec) {
  _state.beforeCloseContext++;
  _state.events.push(`beforeCloseContext:${name}`);
}
