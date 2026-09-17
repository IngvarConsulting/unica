---
id: INV.WIRE.ROOT-FACTS-AND-VERDICT
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_view_without_at_bootstraps_an_empty_workspace
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_view_bootstrap_does_not_equate_git_presence_with_repository_readiness
---

# Факты о рабочем пространстве и его готовность запрашиваются отдельно

`unica.view {}` и `unica.check {}` отвечают до допуска наборов исходников.
В `data` ответа `view` находятся факты: настройки, наборы и база.
Поля `ready`, `discoveredReady`, `repositoryReady`, `readinessState`,
`checks` и `diagnostics` принадлежат корневому `check` и в `view` не выдаются.
Корневой `check` не возвращает перечень наборов или ревизию исходников.

Неготовое рабочее пространство получает успешный ответ `check` с
`status: "failed"` и диагностикой причины. Успешный `view` всегда предлагает
`unica.check {}` в `next`, в том числе из каталога без настроек и исходников.
