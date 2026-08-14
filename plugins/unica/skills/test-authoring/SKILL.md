---
name: test-authoring
description: "Проектирование тестов 1С и preview команд YaXUnit/Vanessa Automation. Используй когда нужно написать тест, подобрать сценарии или подготовить all/module запуск; текущий Unica runtime не выполняет applied-запуск."
---

# Test Authoring

## MCP routing

- Preferred path: use MCP `unica` tools `unica.code.search`, `unica.project.map`, `unica.runtime.execute`, and the relevant `unica.*.info` tools.
- По INV-MCP-RUNTIME-RECEIPT текущий runtime-контракт: `unica.runtime.execute` — preview-only и вызывается только с `dryRun: true`; любой applied-режим возвращает fail-closed до workspace discovery и process spawn. Preview не является runtime verification. Не обходи этот отказ прямым runner-ом, через `unica.build.*` или fallback через `unica.runtime.job.*`.
- Use `unica.standards.search` or `unica.standards.explain` only when test design depends on a `development-standard`. Expected platform API or mechanics require a `platform-help` source; if public MCP `unica` does not expose one, report the contract gap.
- Do not call internal runtime, analyzer, or package adapters directly. They are hidden behind MCP `unica`.

## Workflow

1. Define the behavior under test before choosing the framework: pure BSL unit, object lifecycle, form behavior, integration contract, or regression around a diagnostic.
2. Search existing tests and fixtures with `unica.code.search`; follow local naming, setup, teardown, and assertion style.
3. Prefer YaXUnit for module/unit-level BSL behavior and Vanessa Automation for UI/business scenarios that require a client.
4. Build the smallest stable fixture. Avoid dependence on production data unless the user explicitly requests an integration test.
5. Preview `unica.runtime.execute` with `operation=syntax` after adding test code, then preview `operation=test` with `testRunner=yaxunit` or `testRunner=va`; neither call executes the test suite.
6. Report that runtime verification was not performed. If separate test evidence is supplied, report the exact failing test, expected/actual behavior, and whether the failure is test setup or product behavior.

## Verification gate

- For implementation plans, every stated behavior gets either an executable test,
  a syntax/diagnostic check, or an explicit residual risk.
- For public API, integration, release, or metadata behavior, include impact
  analysis evidence from the relevant `unica.*` tools before treating the test
  plan as complete.
- Do not call donor-specific check commands. Use `unica.code.diagnostics` and
  focused `unica.*.info` tools for available static checks; use
  `unica.runtime.execute` only to preview the intended runtime request.

## Scenario design

- Read `../../references/platform/integration-contracts.md` when tests verify HTTP/API/OData/JSON/XML/file-exchange behavior.
- Read `../../references/platform/runtime-diagnostics.md` when a test is meant to reproduce a user-facing runtime failure.
- Treat tests as executable debugging: one test should prove the intended user/API scenario, the failure mode, and the regression boundary.
- For API scenarios, cover success, validation error, auth error, duplicate/idempotent retry, remote timeout, and stable error semantics.
- For UI or web-client scenarios, preview `operation=test` for the 1C test suite. Hand a concrete autonomous URL to an external browser-testing tool only when that URL and its running environment were supplied independently.

## MCP examples

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
      "module": "ТестДокументаЗаказКлиента",
      "dryRun": true
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
      "testRunner": "va",
      "dryRun": true
    }
  }
}
```
