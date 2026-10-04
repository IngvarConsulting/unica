---
id: INV.RUNTIME.V13-INFOBASE-EXPORTS
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_exports.rs::runner_011_provider_receipt_replaces_selection_for_all_three_operations
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_infobase_exports_are_implemented_with_closed_agent_facing_arguments
  - crates/unica-coder/src/infrastructure/daemon/server.rs::v5_infobase_exports_prepare_before_source_admission_and_run_without_a_revision_gate
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_exports.rs::preview_is_non_mutating_and_returns_no_revision_or_raw_command
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_exports.rs::apply_repeats_preflight_and_returns_an_independent_file_receipt
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_exports.rs::direct_export_executes_without_a_previous_preview_or_revision
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_exports.rs::apply_rejects_a_runner_receipt_for_a_different_output
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_exports.rs::parser_rejects_provider_controls_and_output_escape
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_exports.rs::cfe_and_dt_invocations_use_only_their_closed_runner_arguments
gap: https://github.com/IngvarConsulting/unica/issues/974
---

# Успешная выгрузка базы подтверждается файлом назначения

Квитанция раннера использует `provider` с выбранным исполнителем
и происхождением выбора. Снятый `selection` и выдуманный список кандидатов
не являются входом контракта. Публичная проекция не раскрывает пути override
и пояснения о пропущенных кандидатах.

`download` и `infobase.dump` доступны без source set. Preview вызывает
раннер с `--dry-run`, возвращает план и не создаёт файл. Операции требуют
явный boolean `dryRun`: `true` выбирает preview, `false` проверяет текущий
план и исполняет выгрузку без предварительного вызова preview.

Закрытая схема `download` принимает `state` (`working` или `database`),
`output` и необязательное имя `extension`; схема `infobase.dump` — только
`output`. Назначение задаётся внутри рабочего пространства; выбор
провайдера не принимается в аргументах MCP. Успешный ответ раннера для другого
назначения отвергается. Unica отдельно проверяет полученный файл:
отсутствующий или пустой файл не даёт успеха, размер и SHA-256 вычисляются
по его байтам. Командная строка, stdout и внутренние пути не входят
в публичную квитанцию.

Поле `command` в ответе preview и применения должно называть вызванную
команду раннера. Например, для `download` это
`infobase.configuration.export`; ответ с `infobase.dump` или с публичным
именем `download` отклоняется как `invalid_result`.

Проверки используют управляемый раннер и реальные файлы; полный путь
preview/apply проверен на CF, аргументы CFE и DT — отдельно.
Платформа 1С в этих проверках не запускается.

`run` не принимает `ifRev` и не выдаёт `rev`. Отдельный preview не фиксирует
проект, состояние назначения или информационной базы между вызовами.
Внутри вызова проектный файл, его локальное дополнение и файл назначения
сверяются до и после внутреннего preview раннера. Их изменение даёт
`concurrent_change` до исполнения.

Квитанция подтверждает обычный файл: каталог, символическая ссылка или
специальный файл не считаются успешной выгрузкой. Сырые stderr и данные
аутентификации не попадают в результат.

Отказ для всех перечисленных видов назначения и отсутствие секретов
во всех ветвях ошибок отдельно не проверены; эти сценарии перечислены в gap.
