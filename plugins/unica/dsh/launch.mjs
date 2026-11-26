import { existsSync, readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const pluginRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const targets = {
  'darwin-arm64': ['darwin-arm64', 'unica-bootstrap'],
  'linux-x64': ['linux-x64', 'unica-bootstrap'],
  'win32-x64': ['win-x64', 'unica-bootstrap.exe'],
};

/** The SDK owns the native bootstrap (or the development cargo process)
 * directly, including while waiting for the initial MCP handshake. */
export function serverCommand(root = pluginRoot, host = `${process.platform}-${process.arch}`) {
  const target = targets[host];
  if (target) {
    const bootstrap = join(root, 'bootstrap', 'bin', ...target);
    if (existsSync(bootstrap)) return { command: bootstrap, args: ['run', '--plugin-root', root] };
  }
  const checkout = resolve(root, '..', '..');
  const manifest = join(checkout, 'Cargo.toml');
  if (root === join(checkout, 'plugins', 'unica') &&
      existsSync(manifest) && existsSync(join(checkout, 'crates', 'unica-product', 'Cargo.toml')) &&
      JSON.parse(readFileSync(join(root, 'runtime-manifest.json'), 'utf8')).development === true) {
    return { command: 'cargo', args: ['run', '--quiet', '--manifest-path', manifest, '--bin', 'unica', '--'] };
  }
  if (!target) throw new Error(`Unica does not support this host: ${host}`);
  throw new Error(`Unica bootstrap is missing for ${target[0]}`);
}

/** StdioClientTransport adds its SDK allowlist for the current OS. */
export function serverEnvironment(command, env = process.env) {
  const home = env.DSH_HOME || join(homedir(), '.dsh');
  const extra = {
    UNICA_HOST_CONTEXT_REQUIRED: '1',
    UNICA_RUNTIME_CACHE_DIR: join(home, 'unica', 'runtimes'),
    UNICA_PROVIDER_STATE_DIR: join(home, 'unica', 'provider-state'),
  };
  if (command === 'cargo') {
    for (const key of ['CARGO_TARGET_DIR', 'CARGO_HOME', 'RUSTUP_HOME', 'CARGO_INCREMENTAL']) {
      if (env[key]) extra[key] = env[key];
    }
  }
  return extra;
}
