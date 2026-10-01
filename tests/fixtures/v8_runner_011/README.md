# Published runner extension envelopes

Captured 2026-09-22 from the published darwin-arm64 binary of
IngvarConsulting/v8-runner-rust v0.11.0, source commit
`e57f8459b61a9fb2c279b5e65c59b4f67e045dd9`, binary SHA-256
`f59ef38c3a17672b61931b1437d70de9e6d142bcd0f8851a6ec4450e4789cf15`.
Platform: 1C 8.3.27.2074, provider ibcmd, an isolated empty file infobase.
Only machine-specific workspace/platform prefixes in prose were replaced by
`/workspace` and `/platform`; structured fields and receipts are captured bytes.

Every command used `--config <isolated>/v8project.yaml --json-message`, with
`providers.extensions: ibcmd`. Each ran first with `--dry-run`, then without:

- `extensions create --name UnicaRunnerProbe --name-prefix URP_ --purpose patch`
- `extensions info --name UnicaRunnerProbe`
- `extensions activate --name UnicaRunnerProbe --active no`
- `extensions activate --name UnicaRunnerProbe --active yes`
- `extensions delete --name UnicaRunnerProbe`
- `extensions list` (empty after deletion)

Separate `info` calls verified false/true activity after each change. The same
cycle was also executed through the locally built Unica MCP, including durable
compatibility Tasks, non-executing preview and revision-fenced apply. The raw
MCP responses are local evidence under `.build/runner-011/live/mcp-*.json`.
These fixtures attest the runner boundary, not a portable guarantee that every
host has a working platform installation.
