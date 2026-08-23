---
id: DEC.2026-08-23.WORKSPACE-ACTOR-SLICE
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/workspace_actor.rs::workspace_actor_registry_keys_exact_identity_and_separates_worktrees_and_source_roots
supersedes: []
superseded-by: null
establishes: [INV.APP.WORKSPACE-ACTOR-CAPABILITIES, INV.APP.WORKSPACE-ACTOR-IDENTITY, INV.CACHE.WORKSPACE-ACTOR-STATE-SCOPE]
design: docs/design/2026-08-23-v0-13-execution-surface-design.md
---

# Daemon изолирует рабочие пространства акторами

**Решение.** Daemon владеет реестром акторов. Ключ актора состоит из
канонического корня workspace, детерминированно упорядоченных пар
`(sourceSetName, canonical source root)` и точного provider profile. Git
repository identity в ключ не входит. Одна и та же пара имени и корня повторно
использует актор; другое worktree, переназначение имени, набор корней или
provider profile получает другой актор.

Два имени одного канонического или физического source root неоднозначны и
отклоняются. Каждый экземпляр актора получает непубликуемую capability identity:
его provider binding и revision fence нельзя использовать во втором экземпляре
даже с тем же структурным ключом. Актор удерживает no-follow descriptor каждого
корня; встроенные чтения идут относительно descriptor, а внешние path-based
provider и index операции проверяют физическую identity до и после работы и не
публикуют результат после подмены.

WorkspaceActor владеет прежним workspace runtime state, source revision
registry, provider-root/index binding и эксклюзивной границей публикации.
Публикационная аренда не раскрывает ambient root или unchecked callback и
проверяет source revision до выдачи. Descriptor-relative writer появится вместе
с маршрутизацией writers, не в этом срезе.

Generic daemon actor выводит ограниченный domain-separated state scope из всего
структурного ключа и разделяет revision, index, provider cache, coordination и
background state. Старый workspace-service adapter до Task 22 явно использует
`LegacyPhysical`: его v0.12 пути состояния и wire-поведение не меняются.

Существующий workspace-service CLI остаётся compatibility adapter, чей runtime
state вложен в WorkspaceActor. Обычный v0.12 stdio ещё не направляется через
daemon; это делает следующий срез, поэтому запланированное решение всей v0.13
поверхности остаётся `planned`.

**Почему.** Worktree, logical binding, provider profile и физический объект
каталога — независимые границы изоляции; одного repo/path текста для безопасного
повторного использования runtime state недостаточно.

**Цена.** До переключения Invocation routing daemon держит неиспользуемый
реестр, а legacy helper создаёт одноэлементный compatibility actor вокруг
прежнего runtime payload.
