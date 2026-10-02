---
id: INV.WIRE.V13-CAN-DICTIONARY
check:
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_catalog_locks_the_eight_domain_contracts_without_publishing_them
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_refusals_answer_one_diagnostics_channel_from_the_closed_code_set
  - crates/unica-coder/src/infrastructure/daemon/v13_read_modes.rs::sections_keep_identity_and_only_selected_optional_slots
  - crates/unica-coder/src/infrastructure/daemon/v13_read_modes.rs::uncomputed_sections_answer_typed_unsupported_section
---

# Словарь операций узла появляется по явному запросу can

`unica.view` публикует словарь операций только при запросе
`filter.sections: ["can"]`. Состав зависит от вида узла и строится
из реестра, которым проверяется `apply`. Элемент называет `op`,
ожидаемую форму `args` и признак `implemented`; наличие имени в словаре
само по себе не означает реализованность. Схема `apply` принимает имя
операции строкой и не дублирует словарь перечислением имён или объединением
ветвей операций.

Для вида без вычисляемого словаря запрос даёт `unsupported_section`.
Операция не того вида не попадает в его словарь. Ошибка формы аргументов
называет ожидаемый ключ, а неизвестная или неприменимая операция
направляет к `can` того же узла. `diff` не принимает вычисляемую секцию `can`.

Проверки проходят настоящий вызов `view` для объекта метаданных и
отдельно проверяют проекции корня, модуля, неизвестного вида и операции
без реализации. Они не перебирают все пары вида узла и операции.
