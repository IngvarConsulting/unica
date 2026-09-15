---
name: test-authoring
description: "Проектирование тестов 1С: YaXUnit и Vanessa Automation. Используй когда нужно написать тест, подобрать сценарии или подготовить all/module запуск; сам запуск тестов на поверхности v0.13 не опубликован и остаётся отдельным шагом."
---

# Test Authoring

## MCP routing

- Preferred path: use MCP `unica` tools `unica.search`, `unica.view {}`, and `unica.check`.
- Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`ifRevRequiredOnApply`. Контракт вызова бери оттуда, а не из этого текста;
выбирай только операцию с `implemented: true` и не выдумывай аргументов
записи с `argsSchema: null`; превью исполнением не является. Не обходи
контракт прямым runner-ом.
- Use `unica.docs` with `source: "development-standard"` only when test design depends on a `development-standard`. Expected platform API or mechanics require `unica.docs` with `source: "platform-help"`.
- Do not call internal runtime, analyzer, or package adapters directly. They are hidden behind MCP `unica`.

## Workflow

1. Define the behavior under test before choosing the framework: pure BSL unit, object lifecycle, form behavior, integration contract, or regression around a diagnostic.
2. Search existing tests and fixtures with `unica.search`; follow local naming, setup, teardown, and assertion style.
3. Prefer YaXUnit for module/unit-level BSL behavior and Vanessa Automation for UI/business scenarios that require a client.
4. Build the smallest stable fixture. Avoid dependence on production data unless the user explicitly requests an integration test.
5. Check the new test module with `unica.check {at}` after adding test code (`unica.check {}` alone judges workspace readiness); a YaXUnit or Vanessa Automation run is not on the v0.13 surface, so it is not launched from here.
6. Report that runtime verification was not performed. If separate test evidence is supplied, report the exact failing test, expected/actual behavior, and whether the failure is test setup or product behavior.

## Verification gate

- For implementation plans, every stated behavior gets either an executable test,
  a syntax/diagnostic check, or an explicit residual risk.
- For public API, integration, release, or metadata behavior, include impact
  analysis evidence from the relevant `unica.*` tools before treating the test
  plan as complete.
- Do not call donor-specific check commands. Use `unica.check {at}` on the module and
  `unica.view` on the object for available static checks; a test run
  needs separate execution evidence.

## Scenario design

- Read `../../references/platform/integration-contracts.md` when tests verify HTTP/API/OData/JSON/XML/file-exchange behavior.
- Read `../../references/platform/runtime-diagnostics.md` when a test is meant to reproduce a user-facing runtime failure.
- Treat tests as executable debugging: one test should prove the intended user/API scenario, the failure mode, and the regression boundary.
- For API scenarios, cover success, validation error, auth error, duplicate/idempotent retry, remote timeout, and stable error semantics.
- For UI or web-client scenarios, name the 1C test suite run as a separate step. Hand a concrete autonomous URL to an external browser-testing tool only when that URL and its running environment were supplied independently.

## MCP example

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.check",
    "arguments": {
      "at": "main:CommonModule.ТестДокументаЗаказКлиента"
    }
  }
}
```
