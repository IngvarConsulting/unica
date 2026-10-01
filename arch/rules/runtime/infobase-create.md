---
id: INV.WIRE.INFOBASE-CREATE-ONLY-CREATES-AN-ABSENT-ONE
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::preview_plans_the_infobase_without_creating_anything_or_naming_its_path
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::an_existing_infobase_is_refused_at_preview_and_points_to_import
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::a_project_that_needs_an_edt_workspace_is_refused
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::apply_creates_and_takes_its_receipt_from_a_repeated_preview
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::apply_refuses_a_stale_revision_and_an_infobase_created_elsewhere
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::apply_refuses_a_receipt_that_still_plans_to_create
gap: https://github.com/IngvarConsulting/unica/issues/950
---

# Создание базы подтверждается повторным опросом раннера

Адаптер 0.11.2 создаёт только пустую отсутствующую базу. Исходники
и память синхронизации не инициализируются; результат явно сообщает
`initializesSources: false` и `generationProtection: false`.

`infobase.create` берёт соединение из проектного файла и не принимает
аргументов соединения. Операция доступна без исходников, требует preview
и его `ifRev` для применения. Preview допускает только запланированное
создание отсутствующей базы (`planned`), без запуска платформы. Существующая
база получает отказ с указанием на `infobase.import`; запланированное
EDT-пространство также отклоняется.

Ревизия preview связывает проектный файл, его локальное дополнение
и версию раннера. Если любой из этих входов изменился, прежний `ifRev`
не разрешает создание: нужен новый preview.

Если при применении раннер сообщает, что база уже появилась и создание
пропущено, ответ — `concurrent_change`. Успех требует повторного preview,
который подтверждает, что создавать больше нечего. Результат явно называет
провайдера источником сведений о состоянии базы; пути базы и установки
платформы наружу не передаются.

Проверки используют управляемые ответы раннера. Они не проверяют состояние
настоящей информационной базы.
