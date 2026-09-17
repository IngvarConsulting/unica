---
id: DEC.2026-08-25.LOGICAL-TREE-CORE-SLICE
status: active
governs: product
realized:
  - crates/unica-coder/src/infrastructure/logical_tree.rs::logical_tree_routes_branches_to_existing_typed_readers
  - crates/unica-coder/src/infrastructure/logical_tree.rs::logical_tree_delegates_representative_addresses_to_current_typed_reader_adapters
  - crates/unica-coder/src/infrastructure/logical_tree.rs::task14_profiles_route_to_their_real_typed_readers_without_skips
  - crates/unica-coder/src/infrastructure/logical_tree.rs::platform_capability_controls_logical_existence_without_filesystem_evidence
  - crates/unica-coder/src/infrastructure/logical_tree.rs::deep_invalid_module_suffix_cannot_hide_below_a_valid_module_prefix
supersedes: []
superseded-by: null
establishes: [CTR.SOURCE.QUALIFIED-LOGICAL-ADDRESS, INV.SOURCE.PLATFORM-CAPABILITY-EXISTENCE]
design: docs/design/2026-08-23-v0-13-module-contract-design.md
---

# Скрытое ядро v0.13 адресует полное логическое дерево

**Решение.** Канонический результат внутреннего профиля v0.13 использует
обязательный префикс набора исходников и чередующиеся пары вида и прикладного
имени произвольной глубины. Контекстный вход вправе опустить префикс только при
единственном доступном наборе; разрешённая identity всё равно квалифицирована.
Ветка может завершаться видом. Виды принимают каноническое английское имя или
доказанный русский псевдоним и всегда возвращаются по-английски; прикладное имя
не нормализуется.

Существование модуля определяет единый профиль возможностей платформы, а не
наличие файла. Допустимый отсутствующий модуль остаётся логическим узлом;
недопустимая роль отвечает `not_found`. Физическая раскладка не участвует в
идентичности и может появиться только как диагностическое поле `file`.

Публичный профиль и действующая v0.12 identity не меняются. Широкие решения
`DEC.2026-08-23.V0-13-EXECUTION-SURFACE` и
`DEC.2026-08-23.MODULE-CONTRACT` остаются `planned`; переключение разрешено
только единым cutover Task 22 по утверждённому проекту
`docs/design/2026-08-23-v0-13-execution-surface-design.md`.

**Почему.** Tasks 13–21 нужен один адресный и capability-каталог до публикации
новой поверхности, иначе каждый потребитель построит свою несовместимую модель.

**Цена.** До Task 22 код существует только как скрытая внутренняя основа рядом
с неизменным production v0.12.
