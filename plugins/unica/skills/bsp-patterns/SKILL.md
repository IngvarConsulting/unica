---
name: bsp-patterns
description: "Поиск и применение паттернов БСП. Используй когда задача про длительные операции, профили групп доступа, безопасное хранение, дополнительные обработки, HTTP/файлы, уведомления или готовую функцию БСП."
---

# BSP Patterns

## MCP routing

- Preferred path: use MCP `unica` tools `unica.code.search`, `unica.meta.info`, `unica.form.info`, `unica.role.info`, `unica.standards.search`, `unica.standards.explain`, and `unica.runtime.execute`.
- Use `epf-bsp-init` and `epf-bsp-add-command` only for BSP external processing registration helpers.
- Do not call internal analyzer, standards, runtime, or package adapters directly. They are hidden behind MCP `unica`.

## Workflow

1. Identify the BSP subsystem or library pattern by intent, not by guessed module name.
2. Search existing project usage with `unica.code.search` before writing new code. Prefer local project conventions over generic snippets.
3. Inspect affected metadata, forms, roles, and external processing registration with `unica.*.info` skills.
4. Use `unica.standards.search` for platform/BSP guidance when the pattern affects security, rights, background jobs, files, or external calls.
5. Implement the smallest integration point and verify with `unica.runtime.execute` syntax/tests.

## References

- Read `references/platform/compatibility-modes.md` when BSP code gates
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
      "query": "СведенияОВнешнейОбработке",
      "limit": 20
    }
  }
}
```
