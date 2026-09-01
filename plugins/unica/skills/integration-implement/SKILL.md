---
name: integration-implement
description: "Реализация интеграций 1С. Используй когда нужно создать HTTP-сервис, REST-клиент, webhook, web service, file exchange, очереди, контракты, обработку ошибок и безопасное хранение секретов."
---

# Integration Implement

## MCP routing

- Preferred path: use MCP `unica` tools `unica.view {}`, `unica.meta.info`, `unica.meta.add`, `unica.meta.edit`, `unica.code.search`, `unica.standards.search`, `unica.standards.explain`, and `unica.runtime.execute`.
- По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `unica.runtime.execute` с `dryRun: true`
показывает запланированную команду без побочных эффектов, а с `dryRun: false`
исполняет классифицированную операцию и отвечает её терминальным результатом в
том же вызове, приложив названную причину риска (`runtime_risk_*`)
предупреждением; неклассифицированная операция по-прежнему отказывает
`runtime_operation_unbounded` до обнаружения рабочего пространства. Preview
исполнением не является. Работу, которую вызов ждать не должен, запускай через
`unica.runtime.job.start`. Не обходи контракт прямым runner-ом или через
`unica.build.*`.
- Use `unica.form.*`, `unica.role.*`, or `unica.cfe.*` tools when the integration requires UI, rights, or extension changes.
- Do not call internal metadata, analyzer, standards, runtime, or package adapters directly. They are hidden behind MCP `unica`.

## Workflow

1. Define the contract first: endpoint, method, auth, payload schema, idempotency key, retries, timeout, and error response shape.
2. Inspect existing integration modules and HTTP/web service metadata with `unica.code.search` and `unica.meta.info`.
3. Create or edit metadata through `unica.meta.add` / `unica.meta.edit`; keep source-set and format selected by `unica.view {}`.
4. Put reusable logic in common modules; keep HTTP service handlers thin and explicit about request parsing, validation, and response codes.
5. Handle secrets outside versioned modules and configs. Do not log tokens, passwords, full request bodies with personal data, or raw auth headers.
6. Use `unica.runtime.execute` to preview typed syntax/test arguments and, with `dryRun: false`, to run them and report runtime verification as unavailable; for live HTTP behavior require a user-provided debug URL and external evidence, because `autonomous-server` cannot currently launch through this contract.

## Contract detail

- Read `../../references/platform/integration-contracts.md` before changing HTTP/SOAP/OData/JSON/XML/file-exchange behavior.
- Decide state model explicitly: stateless call, authenticated session, queue, exchange message, cursor, or file batch.
- For OData, JSON, and XML, preserve field names, types, date/number semantics, encoding, null handling, and backward compatibility.
- Define auth and secret handling before code: token refresh, certificate or OpenID context, storage location, masking, and retry behavior.
- Make retries idempotent through external ids, message ids, or duplicate checks. Do not rely on remote retries being harmless.
- Stabilize error semantics: validation, auth, duplicate, temporary remote failure, permanent remote failure, and internal failure must be distinguishable.

## Review checklist

- Contract and versioning are explicit.
- Input validation rejects malformed data before business writes.
- Retries are idempotent or guarded by external ids.
- Error responses are stable and do not leak internals.
- Logs use structured logging fields and contain correlation ids but not secrets.
- Tests cover success, validation failure, duplicate/retry, and remote failure.

## MCP example

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.info",
    "arguments": {
      "sourceSet": "main",
      "metadataPath": "HTTPService.ExternalAPI"
    }
  }
}
```
