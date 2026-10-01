---
id: INV.SOURCE.SIDECAR-REMOVAL-ADVICE
check:
  - crates/unica-coder/src/domain/project_health.rs::runtime_sidecar_remediation_keeps_unusual_path_in_one_argv_item
  - crates/unica-coder/src/domain/project_health.rs::runtime_sidecar_aggregation_never_publishes_a_partial_command
  - crates/unica-coder/src/domain/project_health.rs::ambiguous_config_dump_info_never_has_a_removal_command
---

# Команда снятия служебных файлов с учёта требует полного списка

Проверка готовности предлагает снять `ConfigDumpInfo.xml` с учёта Git,
только если он распознан как служебный файл и в рекомендацию помещается
полный список затронутых путей. Неоднозначная классификация или сокращённый
список примеров не дают команды удаления.

Команда задаётся отдельными `program`, `argv` и `cwd`: `git rm --cached`
с буквальным толкованием путей (`--literal-pathspecs` и `--`). Необычное имя
сохраняется одним аргументом. Предлагается изменение индекса Git, а не удаление
рабочего файла.

Тесты проверяют формирование рекомендации по классифицированным фактам;
они не исполняют команду и не проверяют сам классификатор XML.
