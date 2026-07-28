# 2. Ограничения

Ограничения ниже либо уже нормированы реестром инвариантов и решениями, либо
следуют прямо из кода. Там, где у правила есть владелец, глава ссылается на его
ID и не повторяет формулировку (INV-DOC-08).

## Продуктовые ограничения

- Unica ships as one plugin directory under `plugins/unica` that serves both
  supported hosts; only the manifest directories are host-specific
  (INV-PRODUCT-01).
- The public entry point for the model is declared in
  `plugins/unica/.mcp.json` and stays host-neutral (INV-MCP-02).
- Skills describe developer operations rather than the internal tool inventory
  (INV-PRODUCT-02).
- `v8project.yaml` marks the workspace root of a 1C project. Workspace
  discovery walks the ancestors of the working directory, stops at the first
  `v8project.yaml`, otherwise stops at a linked-worktree `.git` pointer file,
  and otherwise treats the working directory itself as the workspace root.

## Технические ограничения

- The Cargo workspace holds exactly two crates: `unica-coder`, which builds the
  public runtime binary `unica`, and `unica-bootstrap`, which builds the
  `unica-bootstrap` selector binary. Both crates inherit one workspace version.
- The public stdio server runs on the official Rust SDK `rmcp` with
  `default-features = false` and the features `server` and `transport-io`. The
  SDK tool macros stay unused because tool names, descriptions, and input
  schemas are data-driven
  ([ADR-0013](../../decisions/0013-mcp-transport-official-rust-sdk.md),
  INV-MCP-05, INV-MCP-06). The SDK brings a `tokio` runtime into a binary that
  was previously fully synchronous.
- The server answers `initialize`, `ping`, `tools/list`, and `tools/call`. It
  advertises the tools capability only, and the advertised protocol version
  comes from the SDK constant instead of a hard-coded literal.
- The published marketplace package carries no full runtime binary: its
  `.mcp.json` starts `unica-bootstrap`, which downloads the pinned host runtime
  from the approved release origin
  `https://github.com/IngvarConsulting/unica/releases/download/<tag>/` and
  verifies it before launch
  ([ADR-0008](../../decisions/0008-public-marketplace-thin-runtime.md),
  INV-PKG-02, INV-PKG-03).
- Bundled engine execution resolves the binary through the generated
  `plugins/unica/third-party/manifest.json` and verifies its recorded SHA-256
  before the process starts; no wrapper script stands between the runtime and a
  bundled tool.
- The runtime contains no Python, PowerShell, or shell operation backend
  (INV-APP-04); adapted operation scripts exist only as test fixtures
  (INV-SKILL-04).
- `.build/` is git-ignored and volatile. Orchestrator cache state lives under
  `<workspaceRoot>/.build/unica` unless `UNICA_CACHE_DIR` overrides the root
  (INV-CACHE-03).

## Процессные ограничения

- Adding, removing, or renaming a public MCP tool is one synchronized change
  across the Rust registry, the parity harness, the routing skill, and the
  owning decision record (INV-MCP-08).
- Changes to skill routing preserve routing through MCP `unica` only
  (INV-SKILL-01).
- Generated binaries are never committed (INV-PKG-01).
- A change that contradicts an accepted decision needs a new or superseding ADR
  before the affected registry entry and its check are edited.
