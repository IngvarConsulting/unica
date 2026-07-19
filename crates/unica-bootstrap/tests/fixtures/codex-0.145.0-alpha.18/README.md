# Codex CLI 0.145.0-alpha.18 contract fixture

This directory preserves the migration-relevant shape observed from the real
`codex-cli 0.145.0-alpha.18` discovery commands named in `metadata.json`.
Absolute profile paths are replaced with `${CODEX_HOME}`.

The raw output was minimized deterministically for review: unrelated plugin
records were removed. The unrelated `openai-curated` marketplace and the local
Windows `unica` marketplace retain the source-less shape observed from this CLI;
the latter is attributable only because its root is exactly
`${CODEX_HOME}/marketplaces/unica-local`. The installed Unica record preserves
the v0.6.1 local-marketplace shape used by the migration classifier.

To capture another version, create an isolated profile containing the legacy
installation and run:

```sh
python3.12 scripts/ci/capture-codex-contract.py \
  --codex /path/to/codex \
  --codex-home /path/to/isolated-codex-home \
  --expected-version "codex-cli 0.145.0-alpha.18" \
  --output-dir /tmp/codex-contract
```

The capture command refuses unexpected CLI versions, malformed JSON, existing
output directories, and absolute paths outside the isolated `CODEX_HOME`.
