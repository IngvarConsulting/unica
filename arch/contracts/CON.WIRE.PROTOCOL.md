---
id: CON.WIRE.PROTOCOL
status: active
version: 1
decision: null
producer: crates/unica-coder/src/interfaces/mcp.rs
consumers: [host]
check: crates/unica-coder/src/interfaces/mcp.rs::modern_list_results_carry_required_cache_fields_and_legacy_stays_clean
---

# Три ревизии протокола и только реализованные возможности
Сервер `unica` говорит ровно на трёх ревизиях протокола: `2025-06-18`,
`2025-11-25` и `2026-07-28`. Современная ветка несёт поля кеша SEP-2549 на
результатах перечисления и разбивает `tools/list` на страницы по 25; легаси
сохраняет прежнюю форму без курсора.

Декларируется только реализованное: `tools`. Prompts, resources, completions,
logging и tasks не рекламируются, пока их не существует.

**Решение не переписано в v2.** Контракт унаследован от архивных ADR-0069 и
ADR-0013; новая запись появится, когда протокольный слой будет пересмотрен.
