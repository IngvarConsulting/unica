---
id: INV.WIRE.INFOBASE-CREATE-ONLY-CREATES-AN-ABSENT-ONE
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::preview_plans_the_infobase_without_creating_anything_or_naming_its_path
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::an_existing_infobase_is_refused_at_preview_and_points_to_import
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::a_project_that_needs_an_edt_workspace_is_refused
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::apply_without_prior_preview_creates_and_confirms_the_provider_receipt
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::apply_refuses_an_infobase_created_elsewhere
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::apply_refuses_a_receipt_that_still_plans_to_create
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::captured_runner_013_create_is_confirmed_by_its_repeated_preview
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::captured_first_push_after_create_loads_in_full_without_no_memory
gap: https://github.com/IngvarConsulting/unica/issues/950
---

# Создание базы подтверждается повторным опросом раннера

Адаптер раннера создаёт только пустую отсутствующую базу. Unica исходники
не загружает и базовой линии синхронизации не обещает; результат явно
сообщает `initializesSources: false`. Защита поколения — свойство `push`
([план импорта исходников](source-import-plan.md)), поэтому план создания
поля `generationProtection` не содержит.

Раннер 0.13 при создании записывает пустую память каждого набора,
объявленного на этот момент. Первый `push` после создания грузит такие наборы
целиком и не отказывает `no_memory`. Записи поколения после создания ещё нет,
и этот `push` поколение базы не сверяет: его `generationProtection: true`
называет режим, а не состоявшуюся сверку. Набор, объявленный позже, памяти
не получает. Живой замер снят на одном наборе конфигурации с провайдером
`designer` и файловой базой.

`infobase.create` берёт соединение из проектного файла и не принимает
аргументов соединения. Операция доступна без исходников и требует явный
boolean `dryRun`: `true` возвращает preview, `false` создаёт базу без
предварительного вызова preview. Preview допускает только запланированное
создание отсутствующей базы (`planned`), без запуска платформы. Существующая
база получает отказ с указанием на `infobase.import`; запланированное
EDT-пространство также отклоняется.

`run` не принимает `ifRev` и не выдаёт `rev`. Исполнение использует
текущую конфигурацию проекта; отдельный preview не фиксирует её состояние.
Проектный файл и его локальное дополнение сверяются до и после внутреннего
preview раннера в текущем вызове. Их изменение даёт `concurrent_change`
до создания базы.

Если при применении раннер сообщает, что база уже появилась и создание
пропущено, ответ — `concurrent_change`. Успех требует повторного preview,
который подтверждает, что создавать больше нечего. Результат явно называет
провайдера источником сведений о состоянии базы; пути базы и установки
платформы наружу не передаются.

Проверки используют управляемые ответы раннера, в том числе снятые
с раннера 0.13.0 вживую. Они не проверяют состояние настоящей
информационной базы.
