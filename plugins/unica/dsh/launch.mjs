#!/usr/bin/env node
/**
 * Starts the Unica stdio MCP server for DeepSeek Harness.
 *
 * DSH launches an MCP server with one fixed command, while Unica ships in two
 * forms that need different commands: an installed package runs the native
 * bootstrap that verifies the pinned runtime, and a source checkout runs the
 * crate through cargo. This launcher picks the form that is actually present,
 * so the same plugin directory serves DSH as it serves the other hosts through
 * `.mcp.json`.
 *
 * The workspace is the directory the Host was started in. DSH's MCP bridge
 * sends neither request metadata nor `roots/list`, so the launcher hands the
 * project to Unica through `UNICA_PROJECT_DIR`, the host-neutral override the
 * packaged runtime accepts; a value already in the Host environment wins.
 */

import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

/** Plugin root: this file lives in `<plugin root>/dsh/`. */
const PLUGIN_ROOT = dirname(dirname(fileURLToPath(import.meta.url)));

/**
 * Bootstrap binary per host target. The matrix mirrors `bootstrap/launch.sh`,
 * which stays the entry for the hosts that run a POSIX shell; DSH reaches the
 * same binary without one.
 */
const BOOTSTRAP_TARGETS = {
  'darwin-arm64': ['darwin-arm64', 'unica-bootstrap'],
  'linux-x64': ['linux-x64', 'unica-bootstrap'],
  'win32-x64': ['win-x64', 'unica-bootstrap.exe'],
};

/** Codes shared with `bootstrap/launch.sh`: a host the release does not serve
 * and a missing executable. The bootstrap itself hands out neither. */
const EXIT_NO_INPUT = 66;
const EXIT_UNSUPPORTED_HOST = 78;
/** Shell convention for a command that cannot be started at all. */
const EXIT_UNAVAILABLE = 127;

function fail(message, code) {
  process.stderr.write(`unica: ${message}\n`);
  process.exit(code);
}

/** Workspace: the installer's override, else the directory the Host started in.
 *
 * Unica validates the path and refuses an unusable one with `invalid_state`,
 * so a bad value is never silently replaced by another project here.
 */
function applyProjectDirectory() {
  process.env.UNICA_PROJECT_DIR = process.env.UNICA_PROJECT_DIR || process.cwd();
}

/**
 * Installed package: the bootstrap verifies the pinned runtime and starts it.
 * Source checkout: the command the development `.mcp.json` carries.
 */
function selectCommand() {
  const target = BOOTSTRAP_TARGETS[`${process.platform}-${process.arch}`];
  const binary = target && join(PLUGIN_ROOT, 'bootstrap', 'bin', target[0], target[1]);
  if (binary && existsSync(binary)) {
    return { command: binary, args: ['run', '--plugin-root', PLUGIN_ROOT] };
  }
  const manifest = resolve(PLUGIN_ROOT, '..', '..', 'Cargo.toml');
  if (existsSync(manifest)) {
    return {
      command: 'cargo',
      args: ['run', '--quiet', '--manifest-path', manifest, '--bin', 'unica', '--'],
    };
  }
  if (!target) {
    fail(`Unsupported Unica host: ${process.platform}-${process.arch}`, EXIT_UNSUPPORTED_HOST);
  }
  fail(`Unica bootstrap is missing for ${target[0]}: ${binary}`, EXIT_NO_INPUT);
}

function main() {
  applyProjectDirectory();
  const selected = selectCommand();
  const child = spawn(selected.command, selected.args, { stdio: 'inherit' });
  child.on('error', (error) => {
    fail(`failed to start ${selected.command}: ${error.message}`, EXIT_UNAVAILABLE);
  });
  child.on('exit', (code, signal) => {
    if (signal) {
      process.kill(process.pid, signal);
      return;
    }
    process.exit(code ?? EXIT_UNAVAILABLE);
  });
  // The Host closes the connection by terminating this process; hand the
  // signal on so the server shuts down its own children in turn.
  for (const signal of ['SIGINT', 'SIGTERM', 'SIGHUP']) {
    process.on(signal, () => child.kill(signal));
  }
}

main();
