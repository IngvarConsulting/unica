---
id: INV.WIRE.ROOT-FACTS-AND-VERDICT
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_view_bootstrap_does_not_offer_an_unparseable_logical_address
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_view_bootstrap_recognizes_an_infobase_only_workspace
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_view_without_at_bootstraps_an_empty_workspace
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_view_bootstrap_does_not_equate_git_presence_with_repository_readiness
gap: https://github.com/IngvarConsulting/unica/issues/970
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

Пустой каталог наблюдается без записи файлов: `view` называет отсутствие
настроек, а `check` возвращает одну причину отсутствия исходников.
Проект только с соединением информационной базы пригоден для runtime-операций. `view` не предлагает ему настройку исходников: он возвращает
preview-вызовы `download` и `dump`, затем корневой `check`.

Набор с именем, из которого нельзя построить логический адрес, не получает
ссылку на чтение. Корневая проверка сообщает `source_set.logical_name_invalid`
и `ready: false`.

Проверка готовности использует оставшийся срок запроса. Неполный обход
не объявляется полной проверкой: ответ отмечает `readinessState: incomplete`.
Проверка истечения срока через текущий публичный маршрут остаётся в `gap`.

Проверка готовности не расходует запас времени, оставленный на сериализацию
и передачу ответа. Проверка этого ограничения на полном маршруте указана
в `gap`.
