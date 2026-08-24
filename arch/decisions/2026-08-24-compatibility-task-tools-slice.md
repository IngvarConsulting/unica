---
id: DEC.2026-08-24.COMPATIBILITY-TASK-TOOLS-SLICE
status: active
governs: product
realized: crates/unica-coder/src/interfaces/mcp.rs::v13_compatibility_task_tools_are_profile_gated_durable_and_replay_free
supersedes: []
superseded-by: null
establishes: [INV.WIRE.V13-TASK-PROFILES, CTR.WIRE.COMPATIBILITY-TASK-TOOLS]
design: docs/design/2026-08-23-v0-13-execution-surface-design.md
---

# Скрытый V13 эмулирует Tasks тремя обычными инструментами

**Решение.** Hidden `SurfaceRelease::V13` выбирает поверхность отдельно для
каждого клиента и запроса. Exact authority нового протокола и Tasks capability
даёт восемь предметных инструментов и native SEP-2663; без этой authority
поверхность содержит те же восемь плюс только `unica.task.get`,
`unica.task.result`, `unica.task.cancel`. Legacy initialized session нельзя
повысить request metadata, а профиль одного клиента не кешируется для другого.

Три compatibility-инструмента — тонкие adapters к той же daemon-owned durable
Invocation. `get` читает сразу, `result` ждёт по умолчанию 7000 мс и принимает
`waitMs` от 0 до 7000, но один абсолютный budget `waitMs + 125 мс`, ограниченный
исходным frontend deadline, расходуется на connect, handshake, request и
response без перезапуска; `cancel` идемпотентен. Polling не вызывает предметный
handler повторно. Терминальный completed result байт-в-байт совпадает с direct
canonical `CallToolResult`.

Незавершённая Invocation возвращается обычным model-visible result с opaque
`taskId`, закрытым status, durable timestamps/TTL, poll interval и безопасным
следующим вызовом. Полей `job` и `work`, runtime prose, путей и секретов в нём
нет. До любого результата adapter проверяет закрытую матрицу
status/result/failure-presence; failure code/message не покидают daemon, а
невозможная форма становится `task_projection_failed`. Invalid, unknown,
expired, failed и cancelled остаются различимыми закрытыми результатами. После
reconnect/restart adapters читают тот же record; источник истины и execution
owner не меняются.

Package-selected V12 сохраняет прежнюю поверхность и wire до Task 22. В hidden
V13 нет `task.list`, `task.logs` и `unica.runtime.job.*`; native Tasks и эти три
инструмента одновременно не публикуются.

**Почему.** Старые hosts получают управляемый lifecycle без второго executor и
без утечки внутренних job-сущностей.
**Цена.** До публичного cutover существует два capability-зависимых hidden V13
профиля, хотя domain execution у них общий.
