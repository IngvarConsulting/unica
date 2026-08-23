---
id: DEC.2026-08-23.WORKSPACE-ACTOR-SLICE
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/workspace_actor.rs::workspace_actor_registry_keys_exact_identity_and_separates_worktrees_and_source_roots
supersedes: []
superseded-by: null
establishes: [INV.APP.WORKSPACE-ACTOR-IDENTITY]
design: docs/design/2026-08-23-v0-13-execution-surface-design.md
---

# Daemon изолирует рабочие пространства акторами

**Решение.** Daemon владеет реестром акторов. Ключ актора состоит из
канонического корня workspace, детерминированно упорядоченных пар
`(sourceSetName, canonical source root)` и точного provider profile. Git
repository identity в ключ не входит. Одна и та же пара имени и корня повторно
использует актор; другое worktree, переназначение имени, набор корней или
provider profile получает другой актор.

WorkspaceActor владеет прежним workspace runtime state, source revision
registry, provider-root binding, index binding и эксклюзивной границей
публикации. Чтения могут выполняться параллельно. Публикация получает акторную
аренду и повторно доказывает source revision до записи результата.

Существующий workspace-service CLI остаётся compatibility adapter, чей runtime
state вложен в WorkspaceActor. Обычный v0.12 stdio ещё не направляется через
daemon; это делает следующий срез, поэтому запланированное решение всей v0.13
поверхности остаётся `planned`.

**Почему.** Два worktree одного репозитория и два логических source set с
одинаковым физическим путём не должны делить rev, index, provider state или
write lease по неявному default alias.

**Цена.** До переключения Invocation routing daemon держит неиспользуемый
реестр, а legacy helper создаёт одноэлементный compatibility actor вокруг
прежнего runtime payload.
