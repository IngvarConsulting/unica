---
name: bsp-patterns
description: "Поиск и применение паттернов БСП. Используй когда задача про длительные операции, профили групп доступа, безопасное хранение, дополнительные обработки, HTTP/файлы, уведомления или готовую функцию БСП."
---

# BSP Patterns

## MCP routing

- Preferred path: use MCP `unica` tools `unica.code.search`, `unica.meta.info`, `unica.view` on the form and role nodes, `unica.docs`, and `unica.runtime.execute`.
- По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `unica.runtime.execute` с `dryRun: true`
показывает запланированную команду без побочных эффектов, а с `dryRun: false`
исполняет классифицированную операцию и отвечает её терминальным результатом в
том же вызове, приложив названную причину риска (`runtime_risk_*`)
предупреждением; неклассифицированная операция по-прежнему отказывает
`runtime_operation_unbounded` до обнаружения рабочего пространства. Preview
исполнением не является. Работу, которую вызов ждать не должен, запускай через
`unica.runtime.job.start`. Не обходи контракт прямым runner-ом или через
`unica.build.*`.
- Use `epf-bsp-init` and `epf-bsp-add-command` only for BSP external processing registration helpers.
- Do not call internal analyzer, standards, runtime, or package adapters directly. They are hidden behind MCP `unica`.

## Workflow

1. Identify the BSP subsystem or library pattern by intent, not by guessed module name.
2. Search existing project usage with `unica.code.search` before writing new code. Prefer local project conventions over generic snippets.
3. Inspect affected metadata, forms, roles, and external processing registration with `unica.*.info` skills.
4. Use `unica.docs` with `source: "development-standard"` only for a `development-standard` that constrains the pattern. Do not treat it as platform or BSP documentation. Exact platform mechanics require `unica.docs` with `source: "platform-help"`. Treat local BSP code as corroborating implementation evidence, not as the platform contract.
5. Implement the smallest integration point; use `unica.runtime.execute` to preview typed syntax/test arguments and, with `dryRun: false`, to run them, and do not claim runtime verification from that preview.

## References

- Read `../../references/platform/compatibility-modes.md` when BSP code gates
  behavior by a platform version or compatibility mode. Platform guidance
  remains the contract source; BSP code is corroborating implementation
  evidence that must be reconciled with that contract.

## Pattern hints

- Long operations: background job, progress feedback, cancellation, and idempotent restart.
- Access: role/profile interaction, privileged mode boundaries, and safe reads.
- External processing: `СведенияОВнешнейОбработке`, command descriptions, form opening, server command execution.
- Secure data: avoid plaintext secrets in modules, constants, logs, and versioned configs.
- Notifications/files: check cleanup and user-visible error path, not only happy path.

## MCP example

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.code.search",
    "arguments": {
      "cwd": "<workspace>",
      "sourceSet": "<source-set-from-unica-view>",
      "query": "СведенияОВнешнейОбработке",
      "limit": 20
    }
  }
}
```
