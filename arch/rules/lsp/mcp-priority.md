---
id: INV.LSP.MCP-PRIORITY
check:
  - crates/unica-coder/tests/platform/workspace_services_integration.rs::an_mcp_call_cancels_a_yielding_call_that_holds_the_analyzer
  - crates/unica-coder/tests/platform/workspace_services_integration.rs::an_expired_analyzer_call_keeps_the_warm_process_for_the_next_one
gap: https://github.com/IngvarConsulting/unica/issues/2173
---

# Запросы LSP не задерживают вызовы MCP

Запросы редактора исполняются так, что вызов MCP той же рабочей области
не ждёт их в очереди и не замедляется ими сверх названного допуска.

Устаревший запрос отменяется: после `$/cancelRequest` или новой версии
документа, к которой он относился, работа по нему прекращается и его
результат не выдаётся за ответ о текущем состоянии. Запрос редактора не
запускает полный проход по корпусу ради одного ответа.
