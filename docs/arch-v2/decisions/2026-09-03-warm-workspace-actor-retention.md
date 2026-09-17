---
id: DEC.2026-09-03.WARM-WORKSPACE-ACTOR-RETENTION
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/daemon/server.rs::daemon_workspace_actor_admission_is_concurrent_bounded_and_fail_closed
supersedes: []
superseded-by: null
establishes: [INV.APP.DAEMON-ACTOR-CAPACITY]
design: docs/design/2026-09-03-warm-workspace-actor-retention-design.md
---

# Повторное использование актора и проверка ревизии

Удержание продлевает окно, в котором actor-owned `SourceRevisionService`
остаётся доверенным, а платформенный fence наблюдает дерево: повторный
`view`/`find` на неизменённом workspace проходит admission и final
confirmation быстрым путём fence вместо полного retained-прохода.
Семантика ревизии не меняется: потеря доверия fence или `Unsupported`
fence по-прежнему ведут к полному reconcile по прежним правилам.
