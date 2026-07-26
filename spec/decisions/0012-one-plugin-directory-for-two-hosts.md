# ADR-0012: One plugin directory serves Codex and Claude Code

- Status: accepted
- Date: 2026-07-26

## Context

Unica is published to the Codex marketplace as a thin package under
[ADR-0008](0008-public-marketplace-thin-runtime.md). Supporting Claude Code adds
a second host with the same shape: a plugin directory holding `skills/`, a
manifest directory, and a root `.mcp.json`.

The two hosts agree on almost everything. Both scan `skills/<name>/SKILL.md`,
both read `.mcp.json` from the plugin root, and Claude Code accepts the existing
skill frontmatter unchanged. They disagree on two points. Each host reads its
own manifest directory, `.codex-plugin/` or `.claude-plugin/`. And each resolves
the plugin root differently: Codex launches the server with the plugin directory
as the working directory, while Claude Code substitutes `${CLAUDE_PLUGIN_ROOT}`
into the server configuration and does not guarantee a working directory.

Measurements on Claude Code 2.1.49 settled three open questions:

- A manifest-declared `mcpServers` path does not replace the root `.mcp.json`;
  both are loaded. Two host-specific MCP files in one directory would therefore
  start two servers under the same `unica` key.
- `${CLAUDE_PLUGIN_ROOT}` is substituted inside an arbitrary `args` string, so a
  Git alias can receive an absolute plugin root.
- An unrecognized manifest or catalog key is a hard load error on older clients,
  not a warning. `git-subdir` is likewise unparsable before 2.1.69.

## Decision

One plugin directory serves both hosts.

Both manifests live side by side and are held at the same version by the version
contract. Each host reads its own and ignores the other.

`.mcp.json` stays single and host-neutral. The packaged Git alias resolves
`root="${CLAUDE_PLUGIN_ROOT}"` first and falls back to `$PWD/${GIT_PREFIX:-}`
when that is empty. Claude Code rewrites the token before the shell runs; Codex
leaves it unset and the original resolution applies unchanged. The runtime cache
travels the same way through `UNICA_RUNTIME_CACHE_DIR`, and the bootstrap
discards a value still containing `${` so a host that does not substitute the
token cannot create a directory named after it.

The Claude catalog is generated from the Claude manifest rather than maintained
by hand, and is pinned with `git-subdir` to the same immutable tag as the Codex
catalog. Both manifests and both catalog entries are restricted to keys the
oldest supported client accepts; a current client only warns about the rest, so
this is enforced by test rather than by `claude plugin validate`.

Claude Code 2.1.69 is the minimum supported version, because `git-subdir` is the
only source type that both pins to a tag and addresses a subdirectory.

## Consequences

- The bootstrap matrix ships once, not once per host; a per-host directory would
  have added roughly 9 MB to every marketplace release.
- Staged plugin bytes stay unserved for both hosts. Each catalog keeps naming the
  previous tag until its promotion PR moves it, preserving the ADR-0008
  invariant.
- Skills, references, and the MCP boundary have no host-specific variants, so
  host support does not fan out into skill prose.
- Adding an optional manifest or catalog key is a compatibility decision, not a
  cosmetic one, and raises the minimum supported client when the key is newer.
- Consumers below Claude Code 2.1.69 cannot load the catalog at all. Lowering the
  floor would mean giving up either tag pinning or the shared directory.
