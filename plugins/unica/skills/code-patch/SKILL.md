---
name: code-patch
description: "Безопасное точечное изменение BSL-модулей 1С через `unica.code.patch`: заполнение нового пустого module stub, вставка кода в тело известного метода или около точного anchor, замена выбранного фрагмента, dry-run diff, cardinality guard и сохранение BOM/EOL. Используй для доработок существующих и новых `.bsl`-модулей, когда нельзя ограничиться чтением кода или XML-редактором метаданных."
---

# Code Patch

## MCP routing

- Использовать MCP `unica` и его публичный инструмент `unica.code.patch`.
- Не вызывать внутренние writer/runtime adapters и не редактировать `.bsl` вручную, если задачу покрывает контракт инструмента.
- Сначала исследовать модуль через `unica.code.outline`, `unica.code.definition`, `unica.code.grep` или `unica.code.search`.
- Перед apply всегда выполнить тот же payload с `dryRun: true` и проверить target, cardinality, pre/post hash, changed ranges и unified diff.
- Передать `dryRun: false` только когда пользователь запросил изменение.

## Выбор цели

Передать ровно один корень:

- `sourceSet` — точное имя configuration source-set из `unica.project.map`; либо
- `sourceDir` — альтернативное указание того же корня: canonical path должен
  точно и однозначно совпасть с `platform_xml`/`CONFIGURATION` entry из
  непустого списка `source-set` основного `v8project.yaml`.

Передать `modulePath` как относительный путь к существующему `.bsl` внутри выбранного корня. Не использовать absolute path, `..`, symlink escape или EDT source-set. Для нового объекта сначала создать metadata/stub соответствующим `unica.meta.*` инструментом, затем заполнить существующий stub через `unica.code.patch`.

Standalone/autodetected XML-корень и symlink alias через `sourceDir` не являются
runtime topology и отклоняются одинаково в preview/apply: иначе mutation нельзя
доказуемо завершить обычным build/dump. Объяви уникальный entry с обязательными
`name`, `type` и `path` в `v8project.yaml` либо выбери `sourceSet` из
`unica.project.map`; не удаляй source-set и не меняй его root между preview,
apply и последующим build.

## Выбор селектора

- `selector: "module"` — выбрать содержимое модуля после необязательного UTF-8 BOM. Использовать для BOM-only/пустого stub или осознанной замены всего модуля.
- `selector: "method"` + `methodName` — выбрать только тело процедуры/функции, сохранив аннотации, объявление, сигнатуру и завершающее ключевое слово.
- `selector: "anchor"` + `anchor` — выбрать точное BSL-aware совпадение. При необходимости ограничить поиск через `methodName`. Совпадения в строках и `//`-комментариях не считаются.

Всегда передавать положительный `expectedCount`. Обычно использовать `1`. При несовпадении cardinality не ослаблять guard вслепую: повторно исследовать модуль и уточнить selector.

## Операции

Выбрать одну операцию:

- `insertBefore` — вставить content перед каждым выбранным диапазоном;
- `insertAfter` — вставить content после каждого выбранного диапазона;
- `replace` — заменить выбранный диапазон content.

Передавать непустой `content`. Инструмент нормализует переносы content к EOL исходника и сохраняет BOM, terminal newline и байты вне изменяемых диапазонов. До записи planner моделирует следующий идентичный вызов: если payload создаёт дополнительный anchor, скрывает anchor строкой/комментарием либо меняет границы method selector, запрос отклоняется как недоказуемо идемпотентный. Повтор принятого apply возвращает byte-identical no-op без события изменения.

## Workflow

1. Вызвать `unica.project.map` и определить `platform_xml` configuration source-set.
2. Найти точный модуль и контекст селектора инструментами чтения кода.
3. Сформировать минимальный patch с ожидаемым `expectedCount`.
4. Выполнить `dryRun: true` и проверить:
   - affected `target`, стабильный `moduleId`, source root и selector;
   - фактический `matchCount`, равный ожидаемой cardinality;
   - `preHash`/`postHash`;
   - `changedRanges`;
   - unified `diff`;
   - отсутствие записи и cache event.
5. При `noOp: true` не выполнять лишний apply. Иначе повторить тот же payload с `dryRun: false`.
6. Сверить apply target/diff/hashes с принятым preview: apply заново планирует актуальные байты и не принимает отдельный `expectedPreHash` от preview.
7. Убедиться, что apply вернул `applied` либо ожидаемый `noOp`, `moduleId` и `matchCount` не изменились, а `ModuleChanged` относится к целевому модулю.
8. Проверить `details.affectedTargets`: target должен содержать ожидаемые
   `sourceSet`, owner selector и raw-byte pre/post manifest.
9. Повторить preview для проверки byte-identical no-op, затем запустить `unica.code.diagnostics`, тесты и обычный typed build. Dirty target снимается только по успешному terminal step того же source-set.

Если support guard блокирует типовой объект, не обходить его сырой правкой. Выбрать расширение через `cfe-*` либо отдельно согласовать изменение состояния поддержки.

## Проверка платформой

- Использовать `platformSyntax: "none"` по умолчанию.
- Использовать `platformSyntax: "configuredInfobase"`, когда нужна дополнительная проверка текущей настроенной ИБ.
- Считать эту проверку non-transactional: patch уже записан, а ошибка syntax не откатывает файл.
- Не считать её проверкой изменённого исходника: инструмент не выполняет build/load. Проверять patched source полным безопасным workflow после apply.
- Читать terminal `status` и `logPath` из `details.platformSyntax`; при `failed`, `timeout` или `unavailable` изучить лог и не скрывать факт уже выполненной записи. Для preview ожидается `skippedDryRun`, для точного no-op — `skippedNoOp` без запуска runtime.

## Пример

Сначала вызвать с `dryRun: true`, затем без изменения payload — с `dryRun: false`:

```json
{
  "cwd": "<workspace>",
  "sourceSet": "main",
  "modulePath": "CommonModules/SampleService/Ext/Module.bsl",
  "selector": "anchor",
  "methodName": "ЗаписатьДанные",
  "anchor": "    МенеджерЗаписи.Записать();",
  "operation": "insertBefore",
  "content": "    ПодготовитьДанные();\n    ПроверитьДанные();\n",
  "expectedCount": 1,
  "platformSyntax": "none",
  "dryRun": true
}
```

При добавлении семи операторов перед единственным вызовом передать все семь строк одним `content`, не выполнять семь независимых patch-вызовов.
Инструмент не автоформатирует BSL: включить нужные отступы во все строки `content` и проверить их в diff. Удобнее включить ведущий отступ и в exact anchor, чтобы исходный вызов сохранил его после вставки.
