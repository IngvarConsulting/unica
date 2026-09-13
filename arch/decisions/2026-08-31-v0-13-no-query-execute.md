---
id: DEC.2026-08-31.V0-13-NO-QUERY-EXECUTE
status: superseded
governs: product
realized: crates/unica-coder/src/application/v13/tool_catalog.rs::v13_run_dictionary_has_twelve_operations_without_query_execution
supersedes: []
superseded-by: DEC.2026-09-02.DIRECTIONAL-RUNTIME-OPERATIONS
establishes: [INV.APP.V13-RUN-DICTIONARY]
design: docs/design/2026-08-31-v0-13-no-query-execute-design.md
---

# v0.13 не поставляет исполнение запросов

**Решение.** Закрытый словарь `unica.run` v0.13 содержит двенадцать операций и
не содержит `query.execute`. Старый сценарий исполнения запросов не получает
преемника, alias или скрытой capability в v0.13. Попытка прямого вызова
удалённого имени обрабатывается как неизвестная каноническая операция.

Остальная первая вертикаль сохраняется: `syntax.check` остаётся единственной
реализованной Run-операцией, а фактическое покрытие восьми предметных и трёх
compatibility Task-инструментов ведётся отдельным реестром.

**Почему.** Наличие неподдержанного имени всё равно обещает пользователю
направление продукта и заставляет клиентов учитывать операцию. v0.13 не должна
поставлять обещание исполнения запросов.

**Цена.** Миграция старого query-сценария заканчивается disposition `removed`;
возврат операции в будущей версии потребует нового публичного решения и полного
контракта безопасности исполнения запросов.
