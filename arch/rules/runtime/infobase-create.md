---
id: INV.WIRE.INFOBASE-CREATE-ONLY-CREATES-AN-ABSENT-ONE
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::preview_plans_the_infobase_without_creating_anything_or_naming_its_path
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::an_existing_infobase_is_refused_at_preview_and_points_to_import
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::captured_existing_infobase_refusal_is_a_state_not_a_call_error
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::a_project_that_needs_an_edt_workspace_is_refused
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::apply_without_prior_preview_creates_and_confirms_the_provider_receipt
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::apply_refuses_an_infobase_created_elsewhere
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::apply_refuses_a_receipt_that_still_plans_to_create
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::captured_runner_014_create_is_confirmed_by_its_repeated_preview
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::a_cluster_infobase_is_created_without_sources
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::captured_first_pushes_after_create_skip_the_assembled_set_and_load_an_edit
  - crates/unica-coder/src/infrastructure/daemon/runner_014.rs::the_origin_kind_follows_the_merged_infobase_section
gap: https://github.com/IngvarConsulting/unica/issues/950
---

# Создание базы подтверждается повторным опросом раннера

Адаптер раннера создаёт только отсутствующую базу. Что в ней окажется, решает
раннер по виду цели, и ответ называет это, а не умалчивает. Раннер 0.14
собирает файловую базу сразу с основной конфигурацией проекта — набором
`CONFIGURATION` — и записывает память об этом наборе: ответ сообщает
`initializesSources: true` и имя набора в `sourceSet`. Базу в кластере, как и
файловую без набора конфигурации, раннер создаёт пустой: `initializesSources:
false`. Вид цели адаптер читает из секции `origin` обоих слоёв так же, как
раннер. Защита поколения — свойство `push`
([план импорта исходников](source-import-plan.md)), поэтому план создания
поля `generationProtection` не содержит.

Первый `push` после создания файловой базы набор основной конфигурации
не грузит, если исходники не менялись, а остальные объявленные наборы грузит
целиком; после пустого создания он грузит целиком все наборы. Ни один из них
не отказывает `no_memory`. Записи поколения после создания ещё нет, и первая
загрузка поколение базы не сверяет: её `generationProtection: true` называет
режим, а не состоявшуюся сверку. Живой замер снят на одном наборе
конфигурации и файловой базе с исполнителями раннера по умолчанию: создание —
`ibcmd`, загрузка — управляемый агент Конфигуратора.

`infobase.create` берёт соединение из проектного файла и не принимает
аргументов соединения. Операция требует явный boolean `dryRun`: `true`
возвращает preview, `false` создаёт базу без предварительного вызова preview.
Preview допускает только запланированное создание (`planned`), без запуска
платформы; отсутствие базы раннер проверяет только у файловой цели. Существующая база получает отказ
`invalid_state` с указанием на `infobase.restore`: раннер 0.14 отвергает её
отказом проверки ещё до запуска платформы, но вызов `infobase.create`
аргументов не имеет, и править в нём нечего. Запланированное EDT-пространство
также отклоняется.

`run` не принимает `ifRev` и не выдаёт `rev`. Исполнение использует
текущую конфигурацию проекта; отдельный preview не фиксирует её состояние.
Проектный файл и его локальное дополнение сверяются до и после внутреннего
preview раннера в текущем вызове. Их изменение даёт `concurrent_change`
до создания базы.

Если при применении раннер сообщает, что база уже появилась, ответ —
`concurrent_change`. Успех файловой базы требует повторного preview, который
подтверждает, что база есть: отказ раннера на существующей базе. Базу в
кластере раннер до создания не видит и планирует всегда, поэтому её создание
засвидетельствовано только шагом провайдера, и ответ говорит это в `receipt`.
Превью и ответ для кластера предупреждают `cluster_database_unverified`:
раннер создаёт базу с `CrSQLDB=Y`, и база данных СУБД с тем же именем
берётся молча.
Результат явно называет провайдера источником сведений о состоянии базы;
пути базы и установки платформы наружу не передаются.

Проверки используют управляемые ответы раннера, в том числе снятые
с раннера 0.14.0 вживую. Они не проверяют состояние настоящей
информационной базы.
