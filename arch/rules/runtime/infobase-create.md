---
id: INV.WIRE.INFOBASE-CREATE-ONLY-CREATES-AN-ABSENT-ONE
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::v5_infobase_create_prepares_before_source_admission_and_keeps_the_revision_gate
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::preview_plans_the_infobase_without_creating_anything_or_naming_its_path
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::an_existing_infobase_is_refused_at_preview_and_points_to_import
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::a_project_that_needs_an_edt_workspace_is_refused
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::apply_creates_and_takes_its_receipt_from_a_repeated_preview
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::apply_refuses_a_stale_revision_and_an_infobase_created_elsewhere
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::apply_refuses_a_receipt_that_still_plans_to_create
---

# Создание базы подтверждается повторным опросом раннера

`infobase.create` берёт соединение из проектного файла и не принимает
аргументов соединения. Операция доступна без исходников, требует preview
и его `ifRev` для применения. Preview допускает только запланированное
создание отсутствующей базы (`planned`), без запуска платформы. Существующая
база получает отказ с указанием на `infobase.import`; запланированное
EDT-пространство также отклоняется.

Если при применении раннер сообщает, что база уже появилась и создание
пропущено, ответ — `concurrent_change`. Успех требует повторного preview,
который подтверждает, что создавать больше нечего. Результат явно называет
провайдера источником сведений о состоянии базы; пути базы и установки
платформы наружу не передаются.

Проверки используют управляемые ответы раннера. Они не проверяют состояние
настоящей информационной базы.
