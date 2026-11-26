/**
 * Verify Windows/Unix early Cordis disposal using the actual pinned MCP SDK.
 * Run with: node tests/research/platform/dsh-cold-start.mjs <installed npm profile>
 */
import assert from 'node:assert/strict';
import {
  copyFileSync, cpSync, existsSync, linkSync, mkdirSync, mkdtempSync, readFileSync,
  rmSync, symlinkSync, unlinkSync, writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath, pathToFileURL } from 'node:url';

if (!process.argv[2]) throw new Error('Pass an npm profile with the Unica DSH dependencies installed');
const modules = resolve(process.argv[2], 'node_modules');
assert.ok(existsSync(join(modules, '@modelcontextprotocol', 'client')));
const root = mkdtempSync(join(tmpdir(), 'unica-dsh-cold-'));
const cwd = process.cwd();
let child;
try {
  symlinkSync(modules, join(root, 'node_modules'), process.platform === 'win32' ? 'junction' : 'dir');
  const source = fileURLToPath(new URL('../../../plugins/unica/dsh/', import.meta.url));
  cpSync(source, join(root, 'dsh'), { recursive: true });
  const target = { 'win32-x64': 'win-x64', 'darwin-arm64': 'darwin-arm64', 'linux-x64': 'linux-x64' };
  const host = target[`${process.platform}-${process.arch}`];
  assert.ok(host, 'Use a supported Unica host');
  const exe = join(root, 'bootstrap', 'bin', host, process.platform === 'win32' ? 'unica-bootstrap.exe' : 'unica-bootstrap');
  mkdirSync(dirname(exe), { recursive: true });
  try { linkSync(process.execPath, exe); } catch { copyFileSync(process.execPath, exe); }
  // node invoked as `unica-bootstrap run --plugin-root ...` executes this
  // fixture. It records the first real SDK request, then ignores EOF.
  writeFileSync(join(root, 'run'), `
    const fs = require('node:fs');
    const rl = require('node:readline').createInterface({input:process.stdin});
    rl.once('line', line => fs.writeFileSync('child.json', JSON.stringify({
      pid:process.pid, ppid:process.ppid, method:JSON.parse(line).method,
      inherited:['GH_TOKEN','UNICA_PROJECT_DIR','CLAUDE_PROJECT_DIR','ZCODE_PROJECT_DIR'].filter(k=>k in process.env),
    })));
    setInterval(()=>{},30_000);
  `);
  process.chdir(root);
  const { apply } = await import(pathToFileURL(join(root, 'dsh', 'tools.mjs')));
  const fiber = { uid: 1 };
  let onPlugin, dispose;
  const ctx = {
    fiber,
    on(name, fn, options) { assert.equal(name, 'internal/plugin'); assert.equal(options.global, true); onPlugin = fn; },
    inject() {}, effect(fn) { dispose = fn(); },
    tools: { register() { assert.fail('A cold runtime cannot advertise tools'); } },
    logger: { warn() {} },
  };
  for (const key of ['GH_TOKEN', 'UNICA_PROJECT_DIR', 'CLAUDE_PROJECT_DIR', 'ZCODE_PROJECT_DIR']) {
    process.env[key] = 'fixture-only';
  }
  const ready = apply(ctx);
  for (let i = 0; !existsSync('child.json') && i < 200; i++) await delay(50);
  child = JSON.parse(readFileSync('child.json'));
  assert.equal(child.ppid, process.pid);
  assert.deepEqual(child.inherited, []);
  assert.equal(child.method, 'initialize');
  assert.equal(onPlugin({ uid: null }), undefined);
  process.kill(child.pid, 0);
  fiber.uid = null;
  await Promise.race([
    Promise.all([onPlugin(fiber), ready, dispose()]),
    delay(15_000, undefined, {ref:false}).then(() => assert.fail('Cold process outlived early plugin disposal')),
  ]);
  assert.throws(() => process.kill(child.pid, 0), {code:'ESRCH'});
  child = undefined;
  console.log('Cold bootstrap stopped before initialize on plugin disposal');
} finally {
  if (child?.ppid === process.pid) {
    try { process.kill(child.pid, 'SIGKILL'); } catch (error) { if (error.code !== 'ESRCH') throw error; }
  }
  process.chdir(cwd);
  if (existsSync(join(root, 'node_modules'))) unlinkSync(join(root, 'node_modules'));
  rmSync(root, { recursive:true, force:true, maxRetries:5, retryDelay:100 });
}
