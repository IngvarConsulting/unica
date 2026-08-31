---
id: CTR.APP.DAEMON-LONG-WORK-CAPABILITIES
status: active
governs: product
decision: DEC.2026-08-24.LONG-WORK-OWNERSHIP-SLICE
check: crates/unica-coder/src/infrastructure/daemon/server.rs::daemon_exact_long_work_ownership_contract
scope: [app]
version: 1
producer: crates/unica-coder/src/infrastructure/daemon/server.rs
consumers: [review]
---

# Скрытые capability долгой работы daemon

`ActorBoundExecution` предоставляет injected canonical service три закрытых
capability: actor-owned Index join, daemon-owned rootless ProviderHost join и
daemon-owned RuntimeResource join через `RuntimeJobService`, который проверяет
retained jobs-root/`active.lock` и выполняет join под lifecycle guard, не выдавая
movable key. Каждый KnownLong вызов сначала возвращает
durable Task, затем ждёт producer; polling не запускает producer повторно.
Capability не являются MCP tools, не добавляют TaskStore phase и не обходят
ProviderRootBinding, revision fence, `BslIndexLock` или runtime `active.lock`.
Unix runtime tree в этом контракте означает только retained unreaped leader и
установленный Unica child-only inherited lifetime sentinel dynamic FD,
сохраняемый текущим pinned runner без handshake/acknowledgement и без overwrite
чужого inherited FD. Ошибка startup, fallback, stale local poll, output или
persistence оставляет canonical worker supervising retained authority; reader
failure sticky. Поздний release читает и атомарно публикует record через retained
исходный job-directory, который принимается до initial spawn либо до принятия
attach process ownership и не переоткрывается по имени на retry; синхронизирует
renamed handle и подтверждает исходный named
job-directory/`record.json` под lifecycle lane до удаления lease. Эта final
confirmation линеаризует release; post-commit mutation вне lane не заявлена.
Capability остаётся привязан к исходному physical jobs-root.
Activation/fallback ownership-transfer record transitions после capability
admission также публикуются только через этот retained job-directory; normal
poll/observation publication остаётся вне этого узкого среза.
Production subject handler и durable progress остаются границей Task 22.
