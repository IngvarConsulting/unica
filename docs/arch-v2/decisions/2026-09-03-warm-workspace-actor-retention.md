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

# Daemon удерживает ограниченный тёплый набор недавних WorkspaceActor

**Решение.** Реестр акторов сохраняет weak-карту как authority admission и
дополнительно удерживает сильные ссылки на не более чем восемь недавно
использованных акторов в порядке MRU с TTL простоя 600 секунд. Просроченные
тёплые акторы демон освобождает в accept-цикле без нового admission. При
исчерпании ёмкости 64 тёплые акторы освобождаются раньше, чем реестр
откажет отличной identity; актор, удерживаемый живой инвокацией, не
вытесняется. Тёплый актор, чей именованный корень перестал быть той же
директорией, при следующем admission забывается и пересоздаётся.

Удержание продлевает окно, в котором actor-owned `SourceRevisionService`
остаётся доверенным, а платформенный fence наблюдает дерево: повторный
`view`/`find` на неизменённом workspace проходит admission и final
confirmation быстрым путём fence вместо полного retained-прохода.
Семантика ревизии не меняется: потеря доверия fence или `Unsupported`
fence по-прежнему ведут к полному reconcile по прежним правилам.

**Почему.** На конфигурации уровня УТ (48 тысяч файлов) каждый вызов
платил 26–43 секунды за полный проход только потому, что актор умирал
вместе с ответом и следующий вызов начинал с недоверенного состояния.

**Цена.** Демон держит до восьми наборов no-follow дескрипторов и потоков
fence между вызовами; Linux и Windows без реализации fence выигрыша не
получают до отдельного решения.
