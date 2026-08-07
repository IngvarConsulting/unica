- Date: `2026-08-07`
- Status: `approved`
- Decision: `ADR-0029`

# Типизированные `predefinedItems` и логический `role.edit`

## Результат проектирования

Предопределённые элементы становятся ещё одной коллекцией существующей typed
metadata algebra. Права существующей роли меняет новый предметный инструмент
`unica.role.edit`, адресующий роль логически. Оба writer-а сохраняют остальной
XML, строят preview и публикуются через общую транзакцию рабочего пространства.

Проект меняет публичную MCP-поверхность, закрытую схему метаданных и контракт
мутации роли. Архитектурным выбором владеет ADR-0029; эта записка фиксирует
проектирование и нормативной не является.

## Исходное противоречие

`unica.meta.info` уже умеет явно читать `predefinedItems`, но
`unica.meta.edit` не представляет их структурой. Возвращать удалённые
`upsert-predefined`, строковые `Operation/Value` или `DefinitionFile` означало
бы создать вторую алгебру рядом с пятью принятыми вариантами ADR-0025.

Для роли физический селектор `RightsPath` раскрывает раскладку выгрузки,
top-level `ObjectName/Name/Value` не допускает атомарной последовательности, а
ответ с `stdout`, diff и путём расходится с typed result ADR-0023. Поэтому
старый проект PR не переносится поверх `main` буквально.

## Публичный контракт `predefinedItems`

Коллекция доступна в уже существующих вариантах:

```json
{"op":"add","collection":"predefinedItems","elements":[{"id":"<uuid>","name":"Основной"}]}
```

```json
{"op":"update","collection":"predefinedItems","elements":[{"id":"<uuid>","description":"Новое"}]}
```

```json
{"op":"remove","collection":"predefinedItems","ids":["<uuid>"]}
```

В `meta.edit` вид владельца выводится из канонического `metadataPath`; в
`meta.add.operations` — из `kind`. Допустимы только четыре владельца:

| Владелец | Поля кроме `id`, `name`, `code`, `description` |
| --- | --- |
| `Catalog` | `isFolder` |
| `ChartOfCharacteristicTypes` | `isFolder`, структурный `type` |
| `ChartOfAccounts` | `accountType`, `offBalance`, `order`, `accountingFlags`, `extDimensionTypes` |
| `ChartOfCalculationTypes` | `actionPeriodIsBase` |

Inline-схема операций по ADR-0025 остаётся независимым от владельца закрытым
надмножеством: она отсекает неизвестные поля и неверные типы, а сочетание поля с
конкретным `kind` или корнем `metadataPath` проверяет доменное преобразование с
`unsupported_kind`. Owner-specific `allOf`/`if`/`then` в схему не возвращаются.
`type` использует общий структурный `metadataType`, а не QName или строку с
префиксом. Для плана счетов `accountType` допускает `Active`, `Passive`, `ActivePassive`;
`accountingFlags` — закрытый объект `имя: boolean`; `extDimensionTypes` —
массив объектов с `name` и необязательными `turnover`, `accountingFlags`.
XML-пространства имён и `ref` остаются заботой writer-а.

`add` принимает образ нового корневого `Item`. Если UUID уже встречается на
любой глубине, эквивалентный образ даёт no-op, а отличный — `already_exists`.
`update` меняет только явно переданные поля элемента с данным UUID на любой
глубине. `remove` удаляет найденный элемент вместе с `ChildItems`. UUID должен
быть уникален в документе; неоднозначность отказывает до мутации.

Неизвестные дочерние узлы и поддержанные, но не переданные поля сохраняются.
Прямые дети ищутся относительно конкретного `Item`, чтобы одноимённый узел во
вложенной структуре не стал целью. Self-closing форма раскрывается только для
добавляемого прямого ребёнка.

## Чтение предопределённых элементов

При `sections: ["predefinedItems"]` `meta.info` возвращает:

```json
{
  "predefinedItems": {
    "items": [
      {
        "id": "<uuid>",
        "name": "Основной",
        "description": "Новое",
        "parentId": null
      }
    ],
    "total": 1,
    "returned": 1,
    "truncated": false
  }
}
```

Массив плоский и повторяет документный preorder: родитель идёт перед своим
поддеревом, соседние элементы сохраняют XML-порядок. В каждый item входят
только поля закрытого typed-варианта его владельца и `parentId`. `limit`
ограничивает возвращённый префикс, но не меняет `total` и признак `truncated`.

## Публичный контракт `unica.role.edit`

```json
{
  "sourceSet": "main",
  "metadataPath": "Role.Демо",
  "operations": [
    {
      "op": "setRight",
      "objectName": "Catalog.Демо",
      "right": "Delete",
      "value": false
    }
  ],
  "dryRun": true
}
```

`operations` непустой и упорядоченный; каждый эффект содержит
`operationIndex`. Schema и parser закрыты для неизвестных полей и явно
отказывают `RightsPath`, `Path`, `ObjectName`, `Name`, `Value` на верхнем
уровне. `sourceSet` берётся из `v8project.yaml`; `metadataPath` после
разрешения возвращается в канонической форме.

Успешный preview, apply и no-op возвращают один typed envelope:

```json
{
  "data": {
    "metadataPath": "Role.Демо",
    "changed": true,
    "effects": [
      {
        "operationIndex": 0,
        "operation": "setRight",
        "objectName": "Catalog.Демо",
        "right": "Delete",
        "before": true,
        "after": false,
        "action": "setRight",
        "changed": true
      }
    ],
    "validation": {"status": "passed"},
    "diagnostics": []
  }
}
```

Физический путь, полный XML, diff и `stdout` не публикуются. При повторе
эквивалентного запроса `data.changed` и `effects[].changed` равны `false`, а
эффект остаётся семантическим описанием no-op.

## Семантика writer-а роли

До изменения проверяется матрица допустимых прав для вида `objectName`, включая
вложенные права. Writer меняет только прямой `<right>` выбранного объектного
блока и сохраняет RLS, templates, глобальные флаги, остальные права и
неизвестные узлы. Платформенное исключение `DataProcessor.* / Use=false`
удаляет весь объектный блок, а не оставляет противоречивое право `Use`.

Self-closing контейнер корректно раскрывается при вставке первого права.
Повторяющийся или структурно неоднозначный прямой ребёнок отказывает до записи,
а не выбирается эвристикой.

## Профиль, разрешение и транзакция

Оба writer-а принимают только платформу `8.3.27` и формат `2.20`. Версия
берётся из разрешённого логического владельца; произвольную версию документа
сохранять и тем самым объявлять поддержанной нельзя. `PredefinedData`
классифицирует общий реестр ADR-0027 — локальная копия списка корней не
создаётся.

Разрешитель удерживает целевой файл внутри выбранного `sourceSet`, проверяет
support guard и symlink containment. Все документы, от которых зависит план,
включаются в preimage одной транзакции. Apply повторно сравнивает preimage;
конкурентный дрейф отказывает без частичной публикации. Ошибка после начала
публикации откатывает все файлы и cache events.

## Проверяемые сценарии

- schema/parser rejection неизвестных полей, владельцев, legacy aliases и
  пустого `operations`;
- preview, apply, повторный semantic no-op, effects и cache events;
- корневой add, update/remove вложенного UUID и удаление поддерева;
- BOM/EOL, document order, direct-child и self-closing XML;
- сохранение неизвестных узлов, RLS, templates, global flags и соседних прав;
- exact profile и support guards, symlink containment;
- concurrent preimage drift, mid-publish failure и полный rollback.

## Состояние corpus-доказательства

Текущий публичный инвентарь содержит 66 случаев: 63 исторических, уже
добавленный независимо `xdto-add-nested-property` и две ветки этого проекта —
`meta-edit-predefined-items` и `role-edit-set-right`. Два независимых запуска
генератора прошли строгую загрузку корпуса с одинаковым нормализованным
SHA-256 контракта:
`2f36c7ca3e4a113604bf86ddb129526513706ea7318d857c50897db62ad83459`.

Это подтверждает воспроизводимость генератора, но не заменяет platform
round-trip. До доступности закреплённой 8.3.27.2074 новый 66-case corpus не
считается платформенным доказательством; прежний PASS на 63 случаях не
распространяется на новые ветки.
