# План: снять `unica.template.*` и `unica.help.add` как дубли `meta.edit` (#375)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Цель:** второй срез критической цепочки волны v0.13 (план #517, фаза 2) —
`unica.template.add`, `unica.template.remove` и `unica.help.add` удаляются из
`tools/list` и runtime без псевдонимов совместимости; регистрацию и снятие
макета выполняет `meta.edit` по коллекции `templates` (умеет уже сегодня),
добавление встроенной справки становится вариантом `addHelp` общего union
операций `meta.add`/`meta.edit`. Поверхность: 74 → 71.

**Решения (ADR-0072, тем же PR):**
- Развилка #375 по справке решена по дефолту ищью — растворение в `meta.edit`
  (довод «содержимое богато» ослаблен: #368 закрыт в v0.12).
- `addHelp` — девятый вариант union (`{op: "addHelp", lang?}`, `lang` по
  умолчанию `ru`, значения как у сегодняшнего `validate_help_lang`); семантика
  сегодняшнего `help.add` без изменений: create-only (`Ext/Help.xml` +
  `Ext/Help/<lang>.html` + `IncludeHelpInContents` формам владельца), повтор —
  отказ. Владелец адресуется `sourceSet + metadataPath` — третий диалект
  селектора (`ObjectName` = путь `Catalogs/Имя` под `srcDir`) исчезает.
- `template.rs` перестаёт быть вторым владельцем генерации содержимого макета:
  обработчик удаляется целиком, эмиттер пустого табличного документа остаётся
  в `mxl.rs` (`empty_spreadsheet_document_xml`), точка входа — только
  `meta.edit`. meta-путь не зовёт `template.rs` уже сейчас (проверено rg).

## Собранная карта фактов (сессия 17.08)

- Виды макета совпадают: `template.add` принимает `HTML, Text,
  SpreadsheetDocument, BinaryData, DataCompositionSchema`
  ([template.rs:494](crates/unica-coder/src/infrastructure/native_operations/template.rs));
  `MetadataTemplateType` в
  [ports.rs:145](crates/unica-coder/src/application/ports.rs) — те же пять
  (HtmlDocument/TextDocument/...). Приёмочный тест покрытия обязателен: все
  пять создаются через `meta.edit add templates`, шестой — `unsupported_kind`.
- `help.add` сегодня ([help.rs:31](crates/unica-coder/src/infrastructure/native_operations/help.rs)):
  args `objectName|ObjectName|processorName|ProcessorName` (путь!), `lang`
  (default ru), `srcDir` (default src); пишет `Ext/Help.xml` (по
  `ACTIVE_FORMAT_PROFILE`), `Ext/Help/<lang>.html`, добавляет
  `IncludeHelpInContents` во все Forms/*.xml владельца; гварды preimage,
  format, support, post-validation `validate_metadata_owner_shape_8_3_27`,
  `validate_help_form_owner_8_3_27`. Всё это переиспользуется из meta-пути —
  help.rs рефакторится в разделяемые функции (вход — каталог объекта),
  публичный обработчик удаляется.
- Union meta: ровно один на `meta.add`+`meta.edit`, 8 вариантов
  (тест `meta_contract_schema_snapshots...`, tool_contracts.rs:4390) →
  станет 9; тег — `MetaEditOperationTag`
  ([operations.rs:1108](crates/unica-coder/src/domain/metadata/operations.rs)).
- Реестр: записи трёх инструментов в
  [mod.rs:2882,3035,3046](crates/unica-coder/src/application/mod.rs); список
  на 6735-6737 (тест). Ретайр — по образцу
  `retired_meta_routes_fail_as_unknown_tools`.
- Скиллы: `plugins/unica/skills/template-add`, `template-remove`, `help-add` —
  удалить; provenance: записи в `spec/provenance/skill-upstreams.json`
  (help-add:640, template-add:1172, template-remove рядом) и
  `plugins/unica/ATTRIBUTIONS.md`; фикстуры
  `tests/fixtures/unica_mcp_script_parity/unica_reference_models/help-add` и
  template-*; тест `test_skill_provenance.py` следит за полнотой — записи
  переводятся в removed/retired форму, какую поддерживает схема файла
  (посмотреть соседний прецедент снятого скилла, напр. meta-compile/validate из
  v0.12).
- Corpus: кейсы `help-add-object` (tool unica.help.add, branch object-help,
  corpus.rs:174) и `template-add-spreadsheet-document` (+соседние, :312) —
  перестроить на `meta.edit` (тот же XML-выход, состав файлов манифеста не
  меняется) либо переименовать кейсы по новому маршруту; правило приёмки #375:
  «каждый сценарий ведомости воспроизводится через meta.edit и покрыт тестом».
- Python-refs на три инструмента: `test_unica_mcp_script_parity.py`,
  `test_unica_skills.py`, `test_reference_format_profile.py`,
  `test_skill_provenance.py`, `test_unica_mcp_smoke.py`,
  `test_format_profile_contract.py`.
- Ведомость: `tool-surface-review.json` — три записи удалить; regen
  `generate-tool-surface.py`; итоговая строка «Инструментов: 71».
- README пакета: секция миграции (3 строки: `template.add` →
  `meta.edit {op:"add", collection:"templates", ...}`; `template.remove` →
  `{op:"remove", collection:"templates", names:[...]}`; `help.add` →
  `{op:"addHelp", lang}`), валидный JSON с представительными значениями.
- ARG_DESCRIPTIONS: осиротевшие описания (`templateType`, `lang`?,
  `objectName`-диалект и пр.) — проверить публикуемость после снятия и удалить
  осиротевшее (страж «запись не переживает аргумент»).
- Локальный гейт: fmt → clippy → `cargo test --no-fail-fast` → tests/ci →
  tests/dev, python строго `/opt/homebrew/bin/python3.12`.

## Task 1: ADR-0072 + план — commit checkpoint
- [ ] `spec/decisions/0072-retire-template-help-into-meta-edit.md` (accepted в
  этом PR; Решение — три пункта выше; Верификация — ретайр-тест, тест покрытия
  пяти видов, addHelp-паритет с прежним help.add на фикстуре).
- [ ] Индекс `spec/decisions/README.md`.
- [ ] Этот план закоммичен.

## Task 2: `addHelp` в union meta — TDD (аддитивная часть, до снятий)
- [ ] Красный контрактный тест: union из 9 вариантов, девятый
  `{op:"addHelp", lang?}`, закрыт, required `["op"]`+, `lang` enum как у
  `validate_help_lang`.
- [ ] Домен: `MetaEditOperationTag::AddHelp`, разбор, типизированная операция.
- [ ] Writer-путь meta: резолв каталога владельца из логического адреса →
  общие функции из `help.rs` (рефакторинг в `help`-модуль без публичного
  обработчика); create-only, квитанция с созданными файлами.
- [ ] Красный тест паритета: `meta.edit {addHelp}` на фикстуре с формой даёт
  тот же набор файлов и `IncludeHelpInContents`, что старый `help.add`
  (пока он ещё жив — прямое сравнение на одном воркспейсе).
- [ ] Тест покрытия макетов: пять `TemplateType` через
  `meta.edit add templates` создаются и валидны; повтор — отказ/дубль по
  текущей семантике; неизвестный вид — `unsupported_kind`.
- [ ] Зелёно: `cargo test -p unica-coder --no-fail-fast` (кроме известного
  lib-флака project_health), fmt, clippy. Commit.

## Task 3: снятие трёх инструментов
- [ ] Реестр mod.rs: три записи долой; ретайр-тест
  `retired_template_help_routes_fail_as_unknown_tools`; диспатч/дескрипторы/
  списки аргументов/описания подчищены; `template.rs` удалён (эмиттеры,
  нужные meta-пути, переезжают куда зовутся), `help.rs` сжат до разделяемых
  функций meta-пути.
- [ ] Corpus-кейсы перестроены на `meta.edit`; полный corpus-прогон зелёный,
  состав файлов манифеста не изменился.
- [ ] Commit.

## Task 4: python-контур, ведомость, скиллы, README
- [ ] Скиллы `template-add`/`template-remove`/`help-add` удалены; маршрутизация
  в `meta-edit`/`meta-add` SKILL.md дополнена сценариями макета и справки;
  provenance/ATTRIBUTIONS/фикстуры parity обновлены по прецеденту прошлых
  снятий; `test_skill_provenance.py`, `test_unica_skills.py`,
  `test_unica_mcp_script_parity.py` зелёные.
- [ ] `tool-surface-review.json` минус три записи; ведомость regen + `--check`;
  smoke-снапшоты/guard-тесты, `test_reference_format_profile.py`,
  `test_format_profile_contract.py` зелёные.
- [ ] README: секция «Template and help migration» с таблицей и
  `unknown unica tool` семантикой отказа.
- [ ] Полный гейт: fmt → clippy → cargo `--no-fail-fast` → tests/ci →
  tests/dev. Commit, push, PR «feat(meta)!: template.* и help.add растворены в
  meta.edit (#375, ADR-0072)».

## Self-review чеклист
- Приёмка #375: tools/list без трёх ✓ (T3), сценарии ведомости через meta.edit
  ✓ (T2/T3), пять видов TemplateType ✓ (T2), одна точка входа генерации ✓
  (T3), таблица миграции ✓ (T4).
- Единый union meta.add/meta.edit сохранён (тест равенства не трогаем).
- Семантика help не изменена — только адресация и владелец маршрута.

## Разведка (продолжение, точные указатели для исполнителя)

- Разбор операций: `parse_edit_operation` —
  `crates/unica-coder/src/application/metadata.rs:563-712`;
  `operation_property_names()` там же :699 (добавить `lang`).
- Публикуемый union: `host_visible_operation_schema` —
  `application/metadata.rs:1871-2078`; вариант addHelp вставляется в `oneOf`
  перед `metadata_relation_operation_schema()` (стал девятым; в тесте
  `meta_contract_schema_snapshots...` tool_contracts.rs:4390 вариант идёт
  между variants[6] (remove predefined) и editRelations — согласовать индексы).
- Доменный enum: `MetaEditOperation` —
  `domain/metadata/operations.rs:1150`; тег `MetaEditOperationTag` :1108.
- Исполнители: `infrastructure/metadata_operations.rs` (61 упоминание) и
  `infrastructure/native_operations/meta/edit.rs` (131) — матчи по вариантам
  enum; компилятор после добавления варианта покажет все места.
- `validate_help_lang` (паттерн `^[A-Za-z0-9_-]+$`) — help.rs:282; вход
  help-функций: `add_help_with_data` help.rs:31 (создание файлов + формы +
  гварды); рефакторить в функции от каталога объекта.
- lang в схеме: `{"type":"string","minLength":1,"pattern":"^[A-Za-z0-9_-]+$",
  "default":"ru"}`.
