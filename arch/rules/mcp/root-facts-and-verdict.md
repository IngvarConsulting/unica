---
id: INV.WIRE.ROOT-FACTS-AND-VERDICT
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_view_bootstrap_does_not_offer_an_unparseable_logical_address
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_view_bootstrap_recognizes_an_infobase_only_workspace
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_view_without_at_bootstraps_an_empty_workspace
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_view_bootstrap_does_not_equate_git_presence_with_repository_readiness
  - crates/unica-coder/src/infrastructure/daemon/v13_workspace_bootstrap.rs::root_inspection_discovers_sources_after_response_handoff
  - crates/unica-coder/src/infrastructure/daemon/v13_workspace_bootstrap.rs::root_check_repeats_after_eol_timeout_and_finishes_with_shared_checkpoint
  - crates/unica-coder/src/infrastructure/daemon/v13_workspace_bootstrap.rs::root_check_does_not_recommend_repeat_for_fixed_failure_or_full_checkpoint
  - crates/unica-coder/src/infrastructure/daemon/v13_workspace_bootstrap.rs::root_eol_timeout_keeps_earlier_attribute_failure
  - crates/unica-coder/src/infrastructure/project_health/resources.rs::continued_repository_eol_resumes_after_a_staged_timeout_and_rechecks_working_bytes
  - crates/unica-coder/src/infrastructure/project_health/resources.rs::continued_working_eol_rejects_a_new_file_and_late_cancellation
  - crates/unica-coder/src/infrastructure/project_health/resources.rs::continued_staged_eol_malformed_protocol_keeps_attribute_findings
  - crates/unica-coder/src/infrastructure/project_health/resources.rs::final_index_change_invalidates_earlier_attribute_findings
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
Если незавершённый EOL-обход превысил срок и ёмкость сохранённого состояния
позволяет продолжить, ответ предлагает повторить `unica.check {}`. Статический
сбой такой рекомендации не получает. При исчерпании ёмкости ответ сообщает об
этом диагностикой и не обещает прогресс от повторения того же вызова. Новый
вызов получает собственный срок и может продолжить ограниченный по памяти
обход ресурсов.
До полного вердикта он заново сверяет Git index, атрибуты и изменяемые
рабочие файлы; утрата или устаревание сохранённого состояния не означает
готовности репозитория.
Проверка истечения срока через текущий публичный маршрут остаётся в `gap`.

Проверка готовности не расходует запас времени, оставленный на сериализацию
и передачу ответа. Проверка этого ограничения на полном маршруте указана
в `gap`.
