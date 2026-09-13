---
name: meta-add
description: Создать объект метаданных 1С и при необходимости настроить его в том же вызове одной транзакцией.
argument-hint: <at> <operations>
allowed-tools:
  - Read
  - Glob
---

# /unica:meta-add — создание объекта метаданных

## MCP routing

- Preferred path: use MCP `unica` tool `unica.apply` с операцией
  `object.create`.
- `object.create` адресуется корнем конфигурации — `<набор>:Configuration`, —
  а вид и имя нового объекта передаются в `values`. Следующие операции того же
  вызова адресуются уже самим объектом. Физическое расположение XML внутри
  выгрузки — внутренняя деталь; наружу путь отдаёт только аварийный
  `unica.resolve`.
- Имя набора даёт `unica.view {}`. `"main"` ниже только пример.
- Всегда сначала `dryRun: true`. Применяй `dryRun: false` только когда
  пользователь явно попросил внести именно эту правку, и только с `ifRev` из
  предпросмотра.
- Настройка при создании — это следующие операции того же вызова: они видят
  результат предыдущих и публикуются одной транзакцией, либо все, либо ни
  одной.
- Читай из ответа `changed`, `effects` по порядку операций и диагностики.
  Ответ описывает правку семантическими `effects`, а не возвращает
  полный XML созданного объекта.
- Один вызов создаёт один объект. Несколько объектов — несколько вызовов.

## Операции создания и настройки

| Операция | Что делает |
|---|---|
| `object.create` | Создаёт объект метаданных по адресу |
| `props.set` | Задаёт свойства объекта именами платформы |
| `attribute.add`, `tabularSection.add`, `dimension.add`, `resource.add`, `enumValue.add`, `column.add` | Добавляют элементы коллекции; `at` — сам объект, элементы идут в `items` |
| `template.add`, `command.add` | Регистрируют макет и команду |
| `predefinedItem.add` | Добавляет предопределённый элемент |
| `relation.add`, `relation.replace`, `relation.remove` | Задают ссылки объекта наружу |
| `help.create` | Заводит встроенную справку владельца |

Виды, свойства, коллекции и варианты типов берутся из опубликованной схемы
операции: схема является контрактом, поэтому перечень здесь не дублируется.
Общие прикладные правила — в
[соглашениях по метаданным](../../references/platform/metadata-conventions.md).

Источник подписки на событие задаётся `relation.replace` со связью `source`;
отдельной операции под источник нет. `Event` и `Handler` передавай через
`props.set` в том же вызове, если выбранный шаблоном обработчик итоговой
связке не подходит.

Тип уникального идентификатора задаётся закрытым вариантом `{"kind": "uuid"}`.
Свойство наблюдения `mutationCapability` из ответа читателя во вход писателя
не передаётся, и неизвестный QName нельзя копировать из XML в аргументы.

Если схема не представляет сценарий, остановись и скажи прямо, что оставшийся
шаг выполняется в Конфигураторе. Не подставляй составное значение строкой и не
заводи временный файл определения.

## MCP examples

### Создать и настроить одним вызовом

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.apply",
    "arguments": {
      "at": "main:Configuration",
      "ops": [
        {
          "op": "object.create",
          "args": {
            "at": "main:Configuration",
            "values": {"kind": "Catalog", "name": "НовыйСправочник"}
          }
        },
        {
          "op": "props.set",
          "args": {
            "at": "main:Catalog.НовыйСправочник",
            "values": {"Comment": "Создан и настроен одним вызовом"}
          }
        },
        {
          "op": "attribute.add",
          "args": {
            "at": "main:Catalog.НовыйСправочник",
            "items": [
              {
                "name": "ВнешнийИдентификатор",
                "type": {"variants": [{"kind": "uuid"}]},
                "required": true
              }
            ]
          }
        }
      ],
      "dryRun": true
    }
  }
}
```

### Создать подписку с типизированным источником

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.apply",
    "arguments": {
      "at": "main:Configuration",
      "ops": [
        {
          "op": "object.create",
          "args": {
            "at": "main:Configuration",
            "values": {
              "kind": "EventSubscription",
              "name": "ПередЗаписьюНоменклатуры"
            }
          }
        },
        {
          "op": "relation.replace",
          "args": {
            "at": "main:EventSubscription.ПередЗаписьюНоменклатуры",
            "values": {
              "relation": "source",
              "targets": [
                {"kind": "object", "metadataPath": "Catalog.Номенклатура"}
              ]
            }
          }
        },
        {
          "op": "props.set",
          "args": {
            "at": "main:EventSubscription.ПередЗаписьюНоменклатуры",
            "values": {
              "Event": "BeforeWrite",
              "Handler": "CommonModule.ОбработчикиПодписок.ПередЗаписьюНоменклатуры"
            }
          }
        }
      ],
      "dryRun": true
    }
  }
}
```

Объект каталога передаёт событию `BeforeWrite` параметр `Cancel`; обработчик
должен быть экспортной процедурой `(Source, Cancel)` в общем модуле с явными
`Global=false`, `Server=true`. Пустой итоговый `Source`, примитивы, ссылки,
неизвестное событие или несовместимая сигнатура отклоняются до публикации.
