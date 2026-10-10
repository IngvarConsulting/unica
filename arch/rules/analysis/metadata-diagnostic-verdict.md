---
id: INV.ANALYSIS.METADATA-DIAGNOSTIC-VERDICT
check:
  - crates/unica-coder/src/domain/metadata/diagnostics.rs::diagnostic_codes_serialize_to_the_stable_exhaustive_vocabulary
  - crates/unica-coder/src/infrastructure/native_operations/meta/validation.rs::legacy_metadata_warnings_use_a_warning_code_and_keep_error_verdicts
  - crates/unica-coder/src/infrastructure/native_operations/meta/validation.rs::internal_malformed_xml_is_a_hard_failure_matching_legacy_classification
  - crates/unica-coder/src/application/meta_info_surface_tests.rs::info_localizes_an_unknown_but_valid_platform_type_as_a_warning
  - crates/unica-coder/src/application/meta_info_surface_tests.rs::info_localizes_an_unmodelled_constant_type_with_the_warning_code
  - tests/ci/test_acceptance_scenarios.py::AcceptanceCorpusRunTests.test_every_wire_answers_its_frozen_classes
---

# Предупреждение метаданных отличается от ошибки кодом и вердиктом

Общее предупреждение типизированного валидатора метаданных и предупреждение
о синтаксически допустимом, но не моделируемом типе имеют код
`validation_warning` и уровень `warning`. Они сохраняют адрес, поле и язык,
если эти сведения присутствуют у находки. Предметные коды предупреждений
сохраняются; код не извлекается из текста.

Одни предупреждения не меняют `passed` на `failed`. Настоящая ошибка
валидации сохраняет `validation_failed` и уровень `error`; при смешанном
результате сохраняются обе находки и `failed`. Повреждённый XML по-прежнему
даёт ошибку, даже если чтение прекращается до запуска валидатора.

Публичные S347–S348 проходят существующий маршрут `unica.check` журнала
документов: пустая коллекция зарегистрированных документов вызывает
предупреждение, а колонка с пустыми References добавляет семантическую
ошибку. Типизированные проверки общего модуля и наблюдённых типов отдельно
проверяют внутренние источники предупреждений.
