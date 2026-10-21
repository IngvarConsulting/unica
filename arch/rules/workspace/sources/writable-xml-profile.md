---
id: INV.SOURCE.WRITABLE-PROFILE
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_template_read_warns_while_writes_keep_the_exact_owner_profile
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::canonical_dcs_validation_reports_readable_template_owner_versions
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::canonical_dcs_validation_keeps_exact_versionless_dcs_root_guard
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::canonical_dcs_validation_keeps_captured_owner_format_warnings
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::canonical_dcs_validation_keeps_large_configuration_owner_format_warning
  - crates/unica-source-designer/src/format_profile.rs::rejects_numeric_equivalents_of_the_exact_supported_literal
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/dcs_mxl_guard_tests.rs::canonical_dcs_supported_source_still_requires_each_exact_owner_version
  - crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs::platform_xml_object_target_rejects_xml_decoded_version_equality
  - crates/unica-coder/src/infrastructure/native_operations/v13_analysis.rs::newer_configuration_root_leads_with_the_format_warning
  - tests/ci/test_acceptance_scenarios.py::AcceptanceCorpusRunTests.test_every_wire_answers_its_frozen_classes
---

# Для редактирования нужен точный формат выгрузки

Записываемый профиль XML-выгрузки — платформа `8.3.27`, формат `2.20`.
Нативная операция отклоняет старый формат без изменения исходных байтов
и предлагает повторную выгрузку средствами платформы. Unica не меняет формат
как побочный эффект операции и не предоставляет нативную миграцию формата;
переход выполняется явной загрузкой и повторной выгрузкой платформой.

Значение `version` должно быть записано буквально как `2.20`. Численно равные
варианты `2.20.0`, `02.20`, `2.020` и написания через XML-сущности, например
`2.&#50;0`, недопустимы. Такой отказ не меняет исходники и не создаёт событий
или артефактов операции. Проверяется исходная запись атрибута до декодирования
XML-сущностей.

О совместимости при чтении сообщает каноническая проверка узла `unica.check`,
а не сам читатель исходников.
Для выгрузки `2.21` это предупреждение `platformVersionUnsupported`.
Корень без `version` считается форматом `1.0` и получает предупреждение
`formatMigrationAvailable`. Предварительный `apply` над такими исходниками
отказывает с `invalid_source` и указывает несовместимый формат владельца.
