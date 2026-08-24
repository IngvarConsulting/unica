---
id: DEC.2026-08-24.LONG-WORK-OWNERSHIP-SLICE
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/daemon/server.rs::daemon_exact_long_work_ownership_contract
supersedes: []
superseded-by: null
establishes: [INV.APP.EXACT-LONG-WORK-OWNERSHIP, INV.APP.RUNTIME-RESOURCE-TREE, CTR.APP.DAEMON-LONG-WORK-CAPABILITIES]
design: docs/design/2026-08-23-v0-13-execution-surface-design.md
---

# Daemon владеет точными readiness capability индекса, provider и runtime

**Решение.** `WorkspaceActor` владеет Index coordinator с ключом из actor-safe
identity, имени source set, полной trusted revision, provider и profile. Один
actor и одна revision разделяют producer; другой worktree или новая revision
намеренно не разделяются, а публикация остаётся под actor root/revision fence и
существующим cross-process `BslIndexLock`.

Daemon владеет rootless ProviderHost coordinator с ключом
`engine + target + capabilities`. Разделяется только запуск host: root request,
результат, cache и существующая `PersistentMcpSession` остаются actor-bound и не
переиспользуются между корнями. Runtime coordinator ключуется выведенной из
реального resource authority identity и точным UUIDv4 job lease, который уже
владеет `active.lock`; повтор аргументов не создаёт idempotency и не объединяет
destructive invocation.

Leader exit runtime-процесса не терминален, пока retained Unix process group или
Windows Job Object не доказан пустым. Cancel, startup failure и drop завершают и
пожинают дерево в одном bounded окне; terminal publication и release ресурса
следуют только после этой проверки.

Canonical V13 capability испытывается injected service после durable Task
handoff, но production subject handler остаётся dormant до Task 22. SharedWork
phase не персистится и resume owner не регистрируется: orphan non-resumable
остаётся terminal interrupted/resume_unsupported. V12 workspace helpers и шесть
runtime-job tools остаются compatibility adapters; этот срез не утверждает
subject routing или durable progress.

**Почему.** In-process single-flight уменьшает повторную тяжёлую работу, но не
может ослаблять физическую root identity, source revision, provider cache или
cross-process resource lock.

**Цена.** До Task 22 capability доступны только injected canonical services, а
старые root-bound helpers и runtime-job surface продолжают существовать рядом.
