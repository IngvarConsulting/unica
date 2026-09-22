---
id: DEC.2026-09-22.RUNNER-ONE-TARGET-VOCABULARY
status: active
governs: product
realized: crates/unica-coder/src/application/v13/tool_catalog.rs::runner_one_vocabulary_replaces_the_previous_public_dictionary
supersedes: [DEC.2026-09-15.RUN-NAMES-READ-AS-LAYER-AND-DIRECTION, DEC.2026-09-22.INSTALLED-EXTENSIONS-AND-RUNNER-RECEIPTS]
superseded-by: null
establishes: [INV.WIRE.RUN-NAMES-ARE-LAYER-AND-DIRECTION, INV.WIRE.SOURCE-IMPORT-APPLIES-THE-PREVIEWED-MODES, INV.WIRE.SOURCE-EXPORT-TARGET-STAYS-INSIDE-THE-WORKSPACE, INV.WIRE.INFOBASE-CREATE-ONLY-CREATES-AN-ABSENT-ONE, INV.APP.V13-RUN-DICTIONARY, INV.RUNTIME.EXTENSION-OPERATIONS, INV.RUNTIME.RUNNER-PROVIDER-RECEIPT, INV.RUNTIME.V13-INFOBASE-EXPORTS, INV.SURFACE.RUN-INTENTS-DIRECTIONAL, INV.WIRE.ARTIFACT-BUILD-PUBLISHES-INSIDE-THE-WORKSPACE, INV.WIRE.CF-IMPORT-KEEPS-ITS-SOURCE-INTACT, INV.WIRE.TERMINAL-RUN-HAS-NO-FENCE, CTR.WIRE.TOOL-SURFACE, INV.WIRE.RUNNER-ONE-VOCABULARY, INV.RUNTIME.RUNNER-ONE-CAPABILITIES, INV.RUNTIME.RUNNER-011-CONFIG-PROJECTION]
changes: [CTR.WIRE.TOOL-SURFACE]
design: docs/design/2026-09-22-runner-one-target-vocabulary-design.md
---

# Unica 0.13 принимает целевой словарь раннера 1.0

**Решение.** До первого пререлиза 0.13 словарь runtime-операций Unica
переходит на целевые имена раннера 1.0. Владелец выбрал это направление
22 сентября 2026 года. Публичная граница остаётся типизированным
`unica.run`, а не произвольной строкой CLI. Пробел между командой и
подкомандой представляется точкой: `infobase.dump`, `extensions.set`.

Контракт операции задаёт смысл, аргументы, эффекты и гарантии независимо
от исполняемой версии раннера. Адаптер 0.11 реализует только доказанное
подмножество; неподдерживаемый режим отвергается до побочного действия.
Адаптер 1.0 расширяет поддержку того же контракта. Имена старого словаря
Unica не становятся публичными синонимами нового словаря.

Словарь доступности различает известную операцию и исполнимый запрос.
Недоступность не маскируется успешным preview, а поддержка одного параметра
не объявляет реализованной всю операцию. Поколения базы и синхронизация
принадлежат раннеру; ifRev Unica не выдаётся за проверку поколения базы.

Переход включает проекции конфигурации и результатов, документацию и
контрактные тесты. Побуквенное копирование всего CLI в `run` не требуется:
локальные операции и неподдерживаемые форматы остаются отдельной границей.

Целевой словарь и ограниченный адаптер 0.11 реализованы. Неисполняемые
режимы не притворяются готовыми: их поддержка после выхода раннера 1.0
требует отдельного проверенного адаптера, а не снятия отказов по номеру версии.
