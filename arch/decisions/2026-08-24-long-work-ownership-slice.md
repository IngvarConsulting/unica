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
identity, source set, полной trusted revision, provider и profile. Producer
разделяют только один actor/revision; публикацию защищают actor fence и `BslIndexLock`.

Daemon владеет rootless ProviderHost coordinator `engine + target + capabilities`.
Разделяется только host startup; request/result/cache/`PersistentMcpSession`
остаются actor-bound. Runtime join доступен только через `RuntimeJobService` под
lifecycle-lock точного no-follow jobs-root, `active.lock` и UUIDv4 lease; movable
key и dedup по аргументам запрещены. Quarantine release удерживает jobs-root,
job directory и record; чтение и atomic publish descriptor-relative. Подмена
ambient A→B сохраняет A и байты B.

На Windows authority — присоединённый Job Object. Unix bundled-runner contract —
unreaped leader через `waitid(WNOWAIT)` и child-only inherited lifetime sentinel.
Pinned `v8-runner` `7ce1b062843d86644fe55741dbe0ee79f7ca767d` не закрывает
унаследованные descriptors перед обычным `Command` spawn. Sentinel передаёт
фактический dynamic FD, не затирая inherited descriptor; это не runner
handshake/acknowledgement и не hostile/cross-group containment.

Потеря capability, post-spawn cleanup error или недоказанная terminality дают
ownership-uncertain и durable `Lost`: signal/release запрещены, `active.lock`
quarantined. `Lost` не является authority. Canonical worker синхронно хранит
process и оба reader до terminal+EOF; local stale worker остаётся под той же
supervision, а reader error/panic sticky и не становится EOF. Cancel/cleanup,
reap/readers делят один monotonic deadline, Drop его не перезапускает.

Injected V13 service проверяет capability после durable Task handoff; production
subject handler dormant до Task 22. SharedWork phase/resume не персистятся,
V12 helpers и runtime-job tools остаются compatibility adapters; subject routing
и durable progress не заявлены.

**Почему.** In-process single-flight уменьшает повторную тяжёлую работу, но не
может ослаблять физическую root identity, source revision, provider cache или
cross-process resource lock.

**Цена.** До Task 22 capability доступны только injected canonical services, а
старые root-bound helpers и runtime-job surface продолжают существовать рядом.
