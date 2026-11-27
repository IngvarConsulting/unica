"""The DSH session service owns the project of each call on a shared MCP connection."""

import subprocess
import unittest
from pathlib import Path


class DshSessionContextTests(unittest.TestCase):
    def test_concurrent_sessions_switching_directories_and_missing_context(self):
        script = """
import assert from 'node:assert/strict';
import { resolve } from 'node:path';
import { callInWorkspace, requireWorkspaceCapability, workspaceChannel } from './plugins/unica/dsh/workspace.mjs';

const capabilities = { experimental: { [workspaceChannel]: {} } };
requireWorkspaceCapability(capabilities);
assert.throws(() => requireWorkspaceCapability({ experimental: {} }));
assert.throws(() => requireWorkspaceCapability(undefined));
const life = new AbortController();
const user = new AbortController();
const a = {}, b = {};
const dirs = new Map([[a, resolve('A')], [b, resolve('B')]]);
const cwd = { ensure: async agent => dirs.get(agent) };
const calls = [];
const client = { async callTool(request, options) {
  calls.push({ request, options });
  return { content: [], structuredContent: { cwd: request._meta[workspaceChannel].cwd } };
} };
const tool = { name: 'unica.view' };
const args = {at: 'main:Configuration'};
const run = agent => callInWorkspace(cwd, client, tool, args, {agent, signal:user.signal}, life.signal);
const [first, second] = await Promise.all([run(a), run(b)]);
assert.equal(first.structuredContent.cwd, dirs.get(a));
assert.equal(second.structuredContent.cwd, dirs.get(b));
dirs.set(a, resolve('C'));
assert.equal((await run(a)).structuredContent.cwd, dirs.get(a));
for (const {request, options} of calls) {
  assert.equal(request.arguments, args);
  assert.equal(request.name, tool.name);
  assert.equal(options.toolDefinition, tool);
}
for (const bad of [undefined, '', '.', 'relative/path']) {
  dirs.set(a, bad);
  await assert.rejects(run(a));
}
await assert.rejects(run(undefined));
user.abort();
await assert.rejects(run(b));
assert.ok(calls.every(call => call.options.signal.aborted));
assert.equal(calls.length, 3);
"""
        subprocess.run(
            ["node", "--input-type=module", "--eval", script], check=True,
            cwd=Path(__file__).resolve().parents[2],
        )
