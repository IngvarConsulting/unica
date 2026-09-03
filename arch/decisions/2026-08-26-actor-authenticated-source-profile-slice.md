---
id: DEC.2026-08-26.ACTOR-AUTHENTICATED-SOURCE-PROFILE-SLICE
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/daemon/server.rs::actor_authenticated_source_profile_contract_is_complete
supersedes: [DEC.2026-08-23.WORKSPACE-ACTOR-SLICE]
superseded-by: null
establishes: [INV.APP.ACTOR-AUTHENTICATED-SOURCE-CAPABILITIES, INV.APP.ACTOR-AUTHENTICATED-SOURCE-IDENTITY, INV.CACHE.ACTOR-AUTHENTICATED-STATE-SCOPE]
design: docs/design/2026-08-23-v0-13-execution-surface-design.md
---

# Актор аутентифицирует полный профиль набора исходников

**Решение.** Daemon владеет реестром акторов. Его ключ состоит из
канонического корня workspace, точного provider/runtime profile и
детерминированно упорядоченных наборов `{name, canonical retained root,
SourceSetKind, SourceFormat, exact platform/serialization profile}`. Git
repository identity и source-map digest в ключ не входят. Полностью совпавший
tuple повторно использует актор; изменение любого поля получает другой актор и
другой ограниченный domain-separated state scope.

Одинаковые имена и канонические либо физические aliases корней отклоняются.
Каждый экземпляр удерживает no-follow capability корней. Выданный им provider
binding несёт весь typed tuple и непубликуемую instance identity; daemon,
logical-read lease и reader выводят kind, format и platform profile только из
этого binding. Binding и revision fence нельзя воспроизвести в другом
экземпляре даже при одинаковой структурной identity. Descriptor-relative
чтения, path-based provider/index и publication повторно проверяют actor,
физический root, revision, deadline и cancellation согласно прежней границе.

Generic actor разделяет revision, index, provider cache, coordination и
background state по полному ключу. Канонические пути кодируются стабильными
native bytes. V0.12 workspace-service adapter явно объявляет typed legacy
compatibility identity и сохраняет `LegacyPhysical` namespace. V13 daemon
принимает только обнаруженный Platform XML профиль 8.3.27 / 2.20; EDT, invalid
и пустой набор не превращаются в синтетический Platform XML root. Уже
допущенное read-only чтение может завершиться на удержанном snapshot; финальность
выбора source map для apply остаётся следующим срезом.

**Почему.** Kind, физический формат и точная версия сериализации меняют смысл
планирования не меньше имени и пути; параллельные caller-supplied поля позволяли
исполнить один binding как другой источник.

**Цена.** Семантически изменившаяся source map создаёт новый actor/state scope,
а unsupported workspace теперь получает fail-closed admission вместо
неподтверждённого synthetic fallback.
