import { createHash } from 'node:crypto';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import { createMcpToolDefinition, resolveReconnectPolicy } from '@deepseek-ai/dsh-mcp-client';
import { scrubbedParentEnv } from '@deepseek-ai/dsh-subprocess';
import { Client } from '@modelcontextprotocol/client';
import { StdioClientTransport } from '@modelcontextprotocol/client/stdio';
import { callInWorkspace, requireWorkspaceCapability, UnsupportedRuntime, workspaceChannel } from './workspace.mjs';

export const name = 'unica-session-tools';
export const inject = ['tools', 'workingDirectory'];

// Keep the public names used by DSH's MCP client, including its disambiguating
// hash when a raw name such as unica.view contains a dot.
function publicToolName(rawName) {
  const original = `mcp__unica__${rawName}`;
  const normalized = original.replace(/[^A-Za-z0-9_-]/g, '_');
  if (original === normalized && normalized.length <= 64) return normalized;
  const hash = createHash('sha256').update(`unica\0${rawName}`).digest('hex').slice(0, 12);
  return `${normalized.slice(0, 51)}_${hash}`;
}

function transport() {
  const home = process.env.DSH_HOME || join(homedir(), '.dsh');
  return new StdioClientTransport({
    command: process.execPath,
    args: [fileURLToPath(new URL('./launch.mjs', import.meta.url))],
    env: {
      ...scrubbedParentEnv(),
      ELECTRON_RUN_AS_NODE: '1',
      UNICA_RUNTIME_CACHE_DIR: join(home, 'unica', 'runtimes'),
      UNICA_PROVIDER_STATE_DIR: join(home, 'unica', 'provider-state'),
    },
  });
}

async function connect(ctx, hostSignal, onReady) {
  const lifetime = new AbortController();
  const signal = AbortSignal.any([hostSignal, lifetime.signal]);
  const registrations = [];
  const client = new Client({ name: 'unica-dsh', version: '1' }, {
    capabilities: { experimental: { [workspaceChannel]: {} } },
    versionNegotiation: { mode: 'auto' },
  });
  let stop;
  const closed = new Promise(resolve => { stop = resolve; });
  client.onclose = () => {
    lifetime.abort();
    stop();
  };
  hostSignal.addEventListener('abort', stop, { once: true });
  try {
    signal.throwIfAborted();
    // A cold bootstrap can download the runtime before the MCP handshake.
    await client.connect(transport(), { signal, timeout: 900_000 });
    requireWorkspaceCapability(client.getServerCapabilities());
    const { tools } = await client.listTools(undefined, { signal });
    signal.throwIfAborted();
    for (const tool of tools) {
      const definition = createMcpToolDefinition(ctx, {
        name: publicToolName(tool.name),
        rawName: tool.name,
        description: tool.description ?? '',
        inputSchema: tool.inputSchema,
        outputSchema: tool.outputSchema,
        taskRequired: tool.execution?.taskSupport === 'required',
        call: (args, execution) =>
          callInWorkspace(ctx.workingDirectory, client, tool, args, execution, signal),
      });
      registrations.push(ctx.tools.register(definition));
    }
    onReady(client.getInstructions() ?? '');
    await closed;
  } finally {
    hostSignal.removeEventListener('abort', stop);
    lifetime.abort();
    for (const dispose of registrations) dispose();
    await client.close();
  }
}

export async function apply(ctx) {
  const lifetime = new AbortController();
  const policy = resolveReconnectPolicy(undefined, 'Unica');
  let instructions = '';
  ctx.inject(['systemPrompt'], inner => inner.systemPrompt.section({
    name: 'mcp:unica',
    order: inner.systemPrompt.getSectionOrder('MCP_SERVERS'),
    interpolate: false,
    text: () => instructions,
  }));
  let initial;
  const ready = new Promise(resolve => { initial = resolve; });
  async function supervise() {
    let failures = 0;
    while (!lifetime.signal.aborted) {
      try {
        await connect(ctx, lifetime.signal, text => {
          failures = 0;
          instructions = text;
          initial();
        });
      } catch (error) {
        if (!lifetime.signal.aborted) ctx.logger.warn(String(error.message ?? error));
        if (error instanceof UnsupportedRuntime) break;
      } finally {
        instructions = '';
        initial();
      }
      if (lifetime.signal.aborted || ++failures > policy.maxAttempts) break;
      const ms = Math.min(policy.maxDelayMs, policy.initialDelayMs * 2 ** (failures - 1));
      try {
        await delay(ms, undefined, { signal: lifetime.signal });
      } catch (error) {
        if (!lifetime.signal.aborted) throw error;
      }
    }
  }
  const done = supervise();
  ctx.effect(() => () => {
    lifetime.abort();
    return done;
  }, 'unica-session-tools.connection');
  await ready;
}
