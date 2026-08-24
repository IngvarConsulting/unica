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
переиспользуются между корнями. Runtime coordinator вызывается только через
`RuntimeJobService`: сервис под тем же lifecycle-lock удерживает no-follow
descriptor jobs-root и `active.lock`, повторно подтверждает их физическую
identity и точный UUIDv4 lease и выполняет join до освобождения guard. Movable
runtime key наружу не выдаётся; повтор аргументов не создаёт idempotency и не
объединяет destructive invocation.

На Windows authority дерева — успешно присоединённый Job Object. На Unix этот
срез поддерживает только bundled-runner contract: unreaped leader удерживается
через `waitid(WNOWAIT)` как generation anchor process group, а отдельный
cooperative ownership FD наследуется всеми принадлежащими потомками независимо
от stdout/stderr. Это не обещание containment для hostile arbitrary descendant
или другого process group. Потерянная или не установленная ownership capability,
ошибка post-spawn cleanup и невозможность доказать terminality дают закрытый
ownership-uncertain outcome и durable `Lost`: сигнал после потери generation authority не
посылается, `active.lock` остаётся quarantined, доказанная tree terminality и
resource release не публикуются. Compatibility phase `Lost` классифицирует
неопределённость для наблюдателя, но не является authority на освобождение ресурса.
Cancel и доказуемый cleanup делят одно абсолютное bounded окно с reap и обоими
output readers.

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
