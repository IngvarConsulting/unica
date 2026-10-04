---
id: INV.WIRE.ARTIFACT-BUILD-PUBLISHES-INSIDE-THE-WORKSPACE
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::v5_artifact_build_prepares_before_source_admission_and_runs_without_a_revision_gate
  - crates/unica-coder/src/infrastructure/daemon/v13_artifact_build.rs::arguments_are_closed_and_each_refusal_names_the_fix
  - crates/unica-coder/src/infrastructure/daemon/v13_artifact_build.rs::preview_refuses_another_output_kind_or_set_and_a_published_preview
  - crates/unica-coder/src/infrastructure/daemon/v13_artifact_build.rs::apply_without_prior_preview_returns_an_independent_file_receipt
  - crates/unica-coder/src/infrastructure/daemon/v13_artifact_build.rs::apply_refuses_a_missing_or_empty_artifact
---

# Сборка подтверждает файл внутри рабочего пространства

`make` принимает относительный `output` для CF или CFE внутри
рабочего пространства. EPF и ERF отклоняются как `unsupported_operation`.
Preview подтверждает запрошенные вид, выход и объявленный набор исходников,
не публикуя файл. Операция требует явный boolean `dryRun`: `true` показывает
план, `false` проверяет текущий план и выполняет сборку без предварительного
preview. `run` не принимает `ifRev` и не выдаёт `rev`.
Внутри вызова проектный файл, его локальное дополнение, объявленный состав
наборов и файл назначения сверяются до и после внутреннего preview раннера.
Их изменение даёт `concurrent_change` до сборки.

Успех требует непустого файла по согласованному пути. Размер и SHA-256
в квитанции Unica получает чтением файла, а не из сообщения раннера.
Отсутствующий или пустой файл вызывает отказ. Отдельный preview не закрепляет
исходники: их изменение может дать другой файл.

Проверки используют управляемый раннер и реальные файлы рабочего
пространства; они не запускают сборку платформой 1С.
