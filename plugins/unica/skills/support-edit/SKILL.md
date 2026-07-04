---
name: support-edit
description: "Изменение состояния поддержки 1С. Используй когда нужно явно включить возможность изменения конфигурации поставщика или переключить объект в editable/off-support/locked."
argument-hint: "-Path <path> -Capability on|off OR -Path <path> -Set editable|off-support|locked"
allowed-tools:
  - Bash
  - Read
---

# Support Edit

## MCP routing

- Preferred path: use MCP `unica` tool `unica.support.edit`.
- Do not edit `Ext/ParentConfigurations.bin` manually and do not call donor scripts directly.
- This is a release-support decision. Run `unica.cf.info` / `unica.meta.info` before and after the change and keep evidence in the project.
- Mutating tools default to dry run. Pass `dryRun: false` only after the user explicitly confirms the support-state change.

## Parameters

| Parameter | Description |
| --- | --- |
| `Path` | Configuration dump directory, object XML, form XML, or another path inside the source tree |
| `Capability` | `on` or `off`; toggles global editing capability |
| `Set` | `editable`, `off-support`, or `locked`; changes the selected object rule |
| `dryRun` | Default `true` for this mutating tool |

Provide exactly one of `Capability` or `Set`.

## Examples

Enable configuration editing while keeping vendor objects locked:

```json
{
  "name": "unica.support.edit",
  "arguments": {
    "cwd": "<workspace>",
    "Path": "src/configuration",
    "Capability": "on",
    "dryRun": false
  }
}
```

Allow editing of one object while preserving vendor support:

```json
{
  "name": "unica.support.edit",
  "arguments": {
    "cwd": "<workspace>",
    "Path": "src/configuration/Catalogs/Items.xml",
    "Set": "editable",
    "dryRun": false
  }
}
```

Remove one object from vendor support:

```json
{
  "name": "unica.support.edit",
  "arguments": {
    "cwd": "<workspace>",
    "Path": "src/configuration/Catalogs/Items.xml",
    "Set": "off-support",
    "dryRun": false
  }
}
```

## Stop Rules

- If `Capability=on` has not been applied, object-level `Set=*` must fail.
- If the configuration is not on support or support is already fully removed, the tool returns a safe no-op.
- If the target object cannot be resolved to a UUID, stop and inspect with `unica.meta.info` or `unica.form.info` before retrying.
