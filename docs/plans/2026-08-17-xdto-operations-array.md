# План: `unica.xdto.edit` на типизированный массив `operations` (#374)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Цель:** первый срез волны адресации v0.13 — `unica.xdto.edit` принимает
непустой упорядоченный массив `operations` (закрытое размеченное объединение в
`properties.operations.items`) вместо одиночной `operation` и пяти плоских
аргументов; операции одного вызова видят результаты предыдущих и публикуются
одной транзакцией; результат несёт эффект на операцию по `operationIndex`.

**Архитектура:** форма полезной нагрузки повторяет `unica.meta.edit`
(ADR-0025 §4: объединение прямо в `items.oneOf`, без `allOf`/`if`/`then`/`$ref`;
тег — `op`, значения camelCase). Внутри обработчика операции применяются
последовательно к аккумулируемому тексту пакета (`writer::plan` на текст шага
i), публикация — одна `CompileTransaction::replace_bytes` целого файла, как
сейчас. Семантика каждой операции не меняется — источник истины по полям
варианта — то, что `writer.rs` читает сегодня.

**Стек:** Rust (crates/unica-coder), python-контур tests/ci, прозу правим в
skills и README.

## Global Constraints

- ADR-0021 §13: один самостоятельно проверяемый PR — реестр, схема, обработчик,
  скилл, примеры, ADR, приёмка одним набором изменений.
- ADR-0025 §4: закрытое размеченное объединение непосредственно в
  `properties.operations.items`; обязательные поля каждого варианта названы.
- Запись решения: **ADR-0071** (дополняет ADR-0024 — владелец домена не
  меняется; фиксирует форму `operations`, транзакцию и `operationIndex`).
- Старая форма отказывает стабильным кодом, называющим `operations`
  (`legacy_arguments_removed`); таблица миграции — в README пакета.
- Тесты гонять через `/opt/homebrew/bin/python3.12` (lxml).
- `dryRun` по умолчанию `true`; preview возвращает те же `effects`, что apply.

## Отображение контракта

| Старое (снимается) | Новое |
| --- | --- |
| `operation: "add-value-type"` + `name`, `base` | `{"op": "addValueType", "name", "base"}` |
| `operation: "add-object-type"` + `name` | `{"op": "addObjectType", "name"}` |
| `operation: "add-property"` + `typeName`, `property` [, `propertyPath`] | `{"op": "addProperty", "typeName", "property" [, "propertyPath"]}` |
| `operation: "remove-type"` + `name` | `{"op": "removeType", "name"}` |
| `operation: "remove-property"` + `typeName`, `name` [, `propertyPath`] | `{"op": "removeProperty", "typeName", "name" [, "propertyPath"]}` |

Топ-уровень схемы: `sourceSet`, `metadataPath`, `operations` (minItems 1),
`dryRun`; `required = ["sourceSet","metadataPath","operations"]`. Поля
`operation`, `name`, `base`, `typeName`, `propertyPath`, `property` с
топ-уровня исчезают; топ-уровневый `oneOf` у инструмента исчезает.

Форма результата (`XdtoEditData`): `sourceSet`, `metadataPath`, `location`,
`noOp` (все операции no-op), `effects: [{operationIndex, op, noOp, change?,
findings[]}]`. Байтовые диапазоны `change` каждого эффекта относятся к
состоянию документа на момент применения этой операции (последовательная
семантика). Ошибка планирования операции i — `operations[i]: <прежний код>`;
частичной записи нет (публикация единая).

### Task 1: ADR-0071 + план

- [ ] `spec/decisions/0071-xdto-edit-typed-operations-array.md` — статус
  proposed→accepted в этом же PR (записи вне целевой ветки правимы); Context
  называет #374 и этот план; Решение: форма union, транзакция, operationIndex,
  судьба старой формы; Check: контрактные тесты `tool_contracts.rs` +
  `test_tool_surface_ledger.py`.
- [ ] Этот файл плана закоммичен.

### Task 2: контракт (schema + validation) — TDD

Files: `crates/unica-coder/src/application/tool_contracts.rs`,
`crates/unica-coder/src/application/operation_descriptors.rs`.

- [ ] Тест: `xdto_edit_contract_publishes_closed_operations_union` — схема:
  properties ровно `{sourceSet, metadataPath, operations, dryRun}`, required
  `["sourceSet","metadataPath","operations"]`, `operations.minItems == 1`,
  `items.oneOf` — 5 вариантов, каждый `additionalProperties: false`, тег
  `op` enum из одного значения, required варианта по таблице; у инструмента
  нет топ-уровневого `oneOf`/`allOf`/`not`.
- [ ] Тест: `xdto_edit_rejects_legacy_flat_arguments` — вызов со старым
  `operation`/`name`/... отказывает до диспетчеризации стабильным
  `legacy_arguments_removed`, называющим `operations`.
- [ ] Тест: валидация элементов — не-объект, неизвестный `op`, недостающее
  обязательное поле варианта, лишнее поле варианта → ошибка с
  `operations[<i>]`.
- [ ] Реализация: `XDTO_EDIT_REQUIRED = ["sourceSet","metadataPath","operations"]`;
  `XDTO_EDIT_ARGS = ["sourceSet","metadataPath","operations"]` (+`dryRun`
  добавляет мутаторный слой); снести `xdto_edit_schema_branch` и топ-`oneOf`,
  собрать union builder; переписать `validate_xdto_arguments` edit-ветку;
  описания аргументов (`operations` union, описания полей вариантов) в
  реестре описаний.
- [ ] `cargo test -p unica-coder tool_contracts` зелёный; коммит.

### Task 3: обработчик — TDD

Files: `crates/unica-coder/src/infrastructure/native_operations/xdto.rs`
(edit-путь, типы результата), `xdto/writer.rs` не трогаем по семантике.

- [ ] Тест: два ops в одном вызове — `addValueType` затем `addProperty` к
  созданному типу — apply даёт валидный пакет, второй эффект видит первый
  (транзакция: операции видят результаты предыдущих).
- [ ] Тест: ошибка второй операции — файл не изменён вовсе (нет частичной
  записи), ошибка несёт `operations[1]`, эффект первой в `data.effects`.
- [ ] Тест: `dryRun` (по умолчанию) возвращает те же `effects`, файл не
  изменён; parity с apply по `data`.
- [ ] Тест: no-op последовательность → `noOp: true`, файл не открыт на запись.
- [ ] Реализация: разбор массива в типизированные операции (внутренний enum
  остаётся; parse от camelCase-тегов); цикл `writer::plan(&text_i, &args_i,
  op)` с синтезированной картой аргументов варианта; `ValidationDiff` на
  каждый шаг (baseline шага — findings предыдущего состояния), блокировка —
  без публикации; единая публикация как сейчас; `XdtoEditData` → effects[].
- [ ] Существующие xdto-тесты в `xdto.rs`/`tool_contracts.rs` переведены на
  новую форму. `cargo test -p unica-coder xdto` зелёный; коммит.

### Task 4: python-контур и ведомость

- [ ] `tests/ci/test_smoke_unica_mcp.py`, `test_unica_mcp_smoke.py`,
  `test_tool_surface_ledger.py`, `test_unica_mcp_script_parity.py` — места с
  `xdto.edit` переведены на `operations`; ledger-ожидания аргументов обновлены.
- [ ] `spec/architecture/tool-surface.md` — секция `unica.xdto.edit` (схема
  вызова, пример с двумя операциями, отказ старой формы).
- [ ] `spec/architecture/tool-surface-review.json` — запись xdto.edit, если
  она несёт список аргументов.
- [ ] Прогон `/opt/homebrew/bin/python3.12 -m pytest tests/ci -x -q` зелёный;
  коммит.

### Task 5: проза и миграция

- [ ] `plugins/unica/skills/xdto/SKILL.md` — примеры на `operations`,
  транзакционный сценарий «тип + свойства одним вызовом».
- [ ] `plugins/unica/README.md` — секция «XDTO operations migration» с
  таблицей из «Отображения контракта» (по образцу соседних секций миграции).
- [ ] Ссылка на ADR-0071 из ADR-0024 не нужна (0024 не редактируем — он в
  main); связь фиксирует 0071.
- [ ] Полный прогон: `cargo clippy -D warnings`, `cargo test`, python ci;
  коммит; PR "feat(xdto)!: typed operations array (#374, ADR-0071)".

## Self-review чеклист

- Приёмка #374 покрыта: union в items ✓ (Task 2), отказ старой формы ✓
  (Task 2), порядок+транзакция ✓ (Task 3), dryRun по-операционно ✓ (Task 3),
  README-таблица ✓ (Task 5).
- Имена вариантов согласованы с meta.edit (camelCase, тег `op`).
- Семантика операций не изменена — writer.rs не переписывается.
