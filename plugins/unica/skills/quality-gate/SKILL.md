---
name: quality-gate
description: "Расследование и воспроизводимая проверка 1С Quality Gate: BSL diagnostics, YaXUnit, Vanessa Automation, БСП, журналы и bootstrap рабочего контура. Используй когда нужно найти причину падения Quality Gate, подготовить безопасный план проверок или собрать доказательства перед изменением."
---

# Quality Gate для 1С

## MCP routing

- Preferred path: use MCP `unica` tools `unica.project.map`, `unica.code.diagnostics`, `unica.code.search`, `unica.meta.info`, `unica.standards.search`, `unica.standards.explain`, and `unica.runtime.execute`.
- This is an MCP-first workflow. Do not replace these calls with direct shell invocations of runtime, analyzer, YaXUnit, Vanessa Automation, or log parsers.
- Use `unica.standards.*` only for a `development-standard` or diagnostic explanation. Do not present it as evidence of platform API behavior; when that evidence is required and no public tool provides it, report a `platform-help contract gap`.

## Intake and preflight

1. State the gate target and the evidence already available: changed source-set, failing test, BSL diagnostic, journal registration, technological journal, CI log, or user scenario.
2. Call `unica.project.map` to identify the workspace and source-set before choosing a runtime action. Keep its source-set and effective project configuration in the result.
3. Categorize the failure before rerunning anything:
   - BSL/static quality → diagnostics and source context;
   - unit/module behavior → YaXUnit;
   - UI or end-to-end business scenario → Vanessa Automation;
   - БСП convention or metadata policy → metadata and standards evidence;
   - runtime exception, lock, timeout, or session issue → журнал регистрации or технологический журнал.
4. Do not download tools, launch a client, or run a mutating runtime operation until the target and user-approved environment are explicit. Use `dryRun: false` only for the requested verification run.

## Diagnostic branch

1. Start BSL investigation with `unica.code.diagnostics` using `mode=status` when readiness is unknown; then use `mode=file` for a known module or the default analyze mode for the selected source-set.
2. Group findings by root cause and use `unica.code.outline`, `unica.code.definition`, or `unica.code.search` only for the affected code context.
3. Explain diagnostic codes with `unica.standards.explain` or `unica.standards.search` as a `development-standard`. Preserve the original diagnostic id, file, line/range, and analyzer readiness state.
4. Run `unica.runtime.execute` with `operation=syntax` after a syntax-sensitive correction. A new error or critical diagnostic blocks the gate.

## Test branch

1. Use YaXUnit for module and unit behavior. Run `unica.runtime.execute` with `operation=test`, `testRunner=yaxunit`, and `testScope=module` for a focused regression; use `testScope=all` only when the gate requires the suite.
2. Use Vanessa Automation for UI or business scenarios that require a 1С client. If the managed payload is absent, first call `unica.runtime.execute` with `operation=tools-download` and `tool=vanessa`; then call `operation=test` with `testRunner=va`.
3. For an interactive Vanessa investigation, use the documented bounded `operation=launch` route from `v8-runner`; retain the platform `/Out` log, client stderr, timeout, and scenario/profile parameters.
4. If a test is unavailable, report the missing fixture or runtime capability as residual risk. Do not claim a Quality Gate passed from syntax alone.

## БСП and metadata branch

1. Inspect the relevant object with `unica.meta.info` and search affected modules with `unica.code.search`.
2. Use `unica.standards.*` only to substantiate a development-standard or БСП convention. Keep the cited source kind in the report.
3. When the gate depends on exact platform mechanics, version-sensitive behavior, or an undocumented runtime message, preserve the raw evidence and report a `platform-help contract gap` instead of extrapolating from standards.

## Runtime and log branch

1. Accept explicit journal registration exports, technological journal fragments, CI logs, or paths supplied by the user. Preserve timestamp, timezone, infobase, session/process id, transaction id, module/procedure, and correlation id.
2. Build a timeline before naming a root cause. Treat the first exception, lock, timeout, or startup failure as evidence; later rollback messages may be consequences.
3. Map module and metadata identifiers through `unica.code.search` and `unica.meta.info`; use `unica.code.diagnostics` for a related static defect.
4. Use `unica.runtime.execute` only for a bounded reproduction or verification, not as a substitute for journal evidence. Keep credentials, connection strings, tokens, and personal data out of reports.

## Gate result

Report one of: passed, failed, blocked, or inconclusive. Include:

- exact scope and source-set from `unica.project.map`;
- executed `unica.*` checks, parameters, and relevant bounded timeout;
- root cause or explicitly marked hypothesis with supporting diagnostics/tests/log lines;
- affected code and metadata paths;
- remaining failures, skipped checks, missing evidence, and any `platform-help contract gap`;
- the smallest next verification step.

## MCP examples

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.code.diagnostics",
    "arguments": {
      "cwd": "<workspace>",
      "sourceDir": "src",
      "mode": "file",
      "path": "CommonModules/Продажи/Ext/Module.bsl",
      "limit": 100
    }
  }
}
```

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "test",
      "testRunner": "yaxunit",
      "testScope": "module",
      "module": "ТестПродаж",
      "dryRun": false
    }
  }
}
```
