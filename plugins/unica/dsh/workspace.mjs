import { isAbsolute } from 'node:path';

export const workspaceChannel = 'pro.ingvar/unica-dsh-workspace';

export class UnsupportedRuntime extends Error {}

export function requireWorkspaceCapability(capabilities) {
  if (!Object.hasOwn(capabilities?.experimental ?? {}, workspaceChannel)) {
    throw new UnsupportedRuntime('This Unica runtime does not support a workspace per DSH session');
  }
}

/** Resolve the current agent directory anew. Calls from different workspaces
 * share the transport, so keep the directory in this invocation's metadata. */
export async function callInWorkspace(workingDirectory, client, tool, args, execution, lifetime) {
  if (!execution.agent) throw new Error('Unica requires a DSH agent session');
  const signal = AbortSignal.any([execution.signal, lifetime]);
  signal.throwIfAborted();
  const cwd = await workingDirectory.ensure(execution.agent, signal);
  if (typeof cwd !== 'string' || !isAbsolute(cwd) ||
      (process.platform === 'win32' && !/^(?:[a-z]:|[/\\]{2})/i.test(cwd))) {
    throw new Error('Unica requires an absolute working directory for this DSH session');
  }
  signal.throwIfAborted();
  return client.callTool({
    name: tool.name,
    arguments: args,
    _meta: { [workspaceChannel]: { cwd } },
  }, { signal, toolDefinition: tool, timeout: 60_000 });
}
