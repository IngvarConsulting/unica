---
id: DEC.2026-09-08.ROOT-VERDICT-IN-CHECK
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_view_without_at_bootstraps_an_empty_workspace
supersedes: []
superseded-by: null
establishes: [INV.WIRE.ROOT-FACTS-AND-VERDICT]
changes: [CTR.WIRE.TOOL-SURFACE]
design: docs/design/2026-09-04-canonical-surface-distribution-design.md
---

# Корень отвечает так же, как узел: факты в `view`, вердикт в `check`

**Решение.** `unica.view {}` отвечает фактами: корень, состояние `v8project.yaml`,
наборы исходников, база, выбранный набор и рекомендуемое содержимое проектного
файла. `unica.check {}` отвечает вердиктом: `status`, `ready`, `discoveredReady`,
`repositoryReady`, `readinessState`, `checks` и диагностики с `remediation`.
Вердикт по рабочему пространству отвечается до допуска наборов — тем же
пред-допусковым маршрутом, что и факты, — потому что он и объясняет, почему
допуска нет. `unica.view {}` всегда указывает в `next` на `unica.check {}`.

**Что чинит.** Корень был единственным местом, где разделение стояло наоборот.
`unica.view {}` возвращал четырнадцать ключей, среди них двадцать записей `checks`,
диагностики уровня `error` с блоком `remediation` и четыре поля готовности, а
`unica.check {}` отвечал `{"status": "admitted", "sources": [...]}` — перечнем
допущенного и ни одной диагностикой. Маршрут `next` из отказа по пустому
набору вёл в `unica.check {}` за «вердиктом и советом, чем наполнить», и приходил в
ответ, где ни вердикта, ни совета не было.

**Чем держится.** На узле контракт уже такой: `unica.view {at}` даёт `props` и
`branches`, `unica.check {at}` — `status`, `validators`, `diagnostics`. Вопрос
«здорово ли» отличается от вопроса «что это» одинаково на всех уровнях, и
корень не исключение. `ok` у вердикта остаётся истиной: неготовое
пространство — законное состояние, а не сбой вызова, ровно как на узле
`status: "failed"` не делает вызов неуспешным.

**Цена.** Узнать готовность стало двумя вызовами вместо одного — та же цена,
что на узле. Перечень допущенных наборов из `unica.check {}` исчез: это факт, и он
уже был в `unica.view {}` полем `sourceSets`. Вместе с ним ушло поле `rev`: вердикт
собирается до допуска, аренды ревизии у него нет, и подписывать им ответ
означало бы обещать снимок, которого не брали.
