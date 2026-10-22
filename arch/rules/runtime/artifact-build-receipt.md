---
id: INV.WIRE.ARTIFACT-BUILD-PUBLISHES-INSIDE-THE-WORKSPACE
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::v5_artifact_build_prepares_before_source_admission_and_runs_without_a_revision_gate
  - crates/unica-coder/src/infrastructure/daemon/v13_artifact_build.rs::arguments_are_closed_and_each_refusal_names_the_fix
  - crates/unica-coder/src/infrastructure/daemon/v13_artifact_build.rs::preview_refuses_another_output_kind_or_set_and_a_published_preview
  - crates/unica-coder/src/infrastructure/daemon/v13_artifact_build.rs::apply_without_prior_preview_returns_an_independent_file_receipt
  - crates/unica-coder/src/infrastructure/daemon/v13_artifact_build.rs::apply_refuses_a_missing_or_empty_artifact
  - crates/unica-coder/src/infrastructure/daemon/v13_artifact_build.rs::external_build_publishes_one_verified_file_and_preserves_neighbors
  - crates/unica-coder/src/infrastructure/daemon/v13_artifact_build.rs::external_preview_names_owner_and_configuration_without_creating_output
  - crates/unica-coder/src/infrastructure/daemon/v13_artifact_build.rs::external_invalid_builds_and_cancellation_preserve_previous_output
  - crates/unica-coder/src/infrastructure/daemon/v13_artifact_build.rs::external_cancellation_after_publication_lock_never_commits
  - crates/unica-coder/src/infrastructure/daemon/v13_artifact_build.rs::external_prerequisites_and_ambiguity_are_refused_before_dispatch
---

# Сборка подтверждает файл внутри рабочего пространства

`make` принимает относительный `output` для CF, CFE, EPF или ERF внутри
рабочего пространства. Для EPF/ERF обязателен `sourceSet` с единственным
объектом соответствующего вида в Designer/platform XML и объявленный набор
основной конфигурации.
Отсутствующий prerequisite, неверный вид или неоднозначный набор отклоняются
до запуска раннера. Preview называет точный логический объект.
Раннер собирает внешние артефакты только в приватный каталог. Адаптер
публикует единственный проверенный обычный файл, сохраняя соседние файлы
и исходники. Отмена до публикации сохраняет прежний output.
Preview подтверждает запрошенные вид, выход и объявленный набор исходников,
не публикуя файл. Используется [режим previewApply](../mcp/run-operation-names.md).
Исполнение проверяет текущий план перед сборкой.
Внутри вызова проектный файл, его локальное дополнение, объявленный состав
наборов и файл назначения сверяются до и после внутреннего preview раннера.
Их изменение даёт `concurrent_change` до сборки.

Успех требует непустого файла по согласованному пути. Размер и SHA-256
в квитанции Unica получает чтением файла, а не из сообщения раннера.
Отсутствующий или пустой файл вызывает отказ. Изменение исходников после
отдельного preview может дать другой файл.

Проверки используют управляемый раннер и реальные файлы рабочего
пространства; они не запускают сборку платформой 1С.
