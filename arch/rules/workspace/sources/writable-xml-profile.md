---
id: INV.SOURCE.WRITABLE-PROFILE
check:
  - crates/unica-coder/src/infrastructure/format_guard.rs::single_writable_platform_xml_profile_is_exact
  - crates/unica-coder/src/domain/format_profile.rs::rejects_numeric_equivalents_of_the_exact_supported_literal
  - crates/unica-coder/src/application/mod.rs::entity_spelled_supported_format_is_invalid_at_the_public_boundary
  - crates/unica-coder/src/application/mod.rs::numeric_equivalent_noncanonical_format_warns_on_read_and_blocks_public_mutator
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
