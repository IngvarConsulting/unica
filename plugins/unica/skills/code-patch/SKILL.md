---
name: code-patch
description: Точечно вставить или заменить BSL-код в логически адресованном модуле XML-выгрузки Configuration или Extension 1С. Используй для одной проверяемой операции insert или replace.
argument-hint: <at> <insert|replace> <text>
allowed-tools:
  - Read
  - Glob
---

# /code-patch — безопасная вставка и замена BSL

## MCP routing

- Preferred path: use MCP `unica` tool `unica.apply` с операциями
  `code.insert` и `code.replace`.
- Do not call internal MCP/CLI adapters directly. They are hidden behind
  `unica` and synchronized by the orchestrator.
- Всегда сначала `dryRun: true`. Применяй `dryRun: false` только после того,
  как пользователь явно попросил внести именно эту правку, и только с `ifRev`
  из предпросмотра.

**Селектор — это адрес.** Отдельного `selector` с `method` или `anchor` нет:
что править, называет `args.at`. Узел метода — `…Module.<Роль>.Method.<Имя>`,
тело модуля целиком — `…Module.<Роль>.Body`. Адрес и точнее селектора, и
переживает переименование файла, и уже проверен чтением.

Правится модуль существующего объекта метаданных в допущенном наборе
исходников формата платформы. Физический путь к `*Module.bsl` остаётся
внутренней деталью: наружу его отдаёт только аварийный `unica.resolve`, и в
обычном ходе работы он не нужен. Создать объект метаданных, удалить модуль
целиком, править EDT или внешние файлы, синхронизировать исходники с базой
этим путём нельзя.

Если правку нельзя выразить одной безопасной операцией, остановись: прочитай
предмет через `unica.view {at}` на узле модуля — ветвь `Method` перечисляет
методы, ветвь `Body` отдаёт строки парами `{line, text}` — и вернись с более
узкой операцией.

## Операции

| Операция | Что делает | `args` |
|---|---|---|
| `code.insert` | Вставляет текст в узел по адресу | `at`, `text` |
| `code.replace` | Заменяет содержимое узла по адресу | `at`, `text` |

Несколько операций одного вызова применяются как одна правка: либо все, либо
ни одной.

## Порядок

1. Найди адрес: `unica.search {corpus: "names"}` по имени объекта, затем
   `unica.view {at}` вниз по ветвям `Module` и `Method`.
2. Прочти предмет: `unica.view {at}` на узле метода даёт подпись, контекст
   компиляции и собственные строки.
3. Предпросмотр: `unica.apply` с `dryRun: true`. Ответ несёт план правки и
   `next` с готовым `ifRev`.
4. Применение: тот же вызов с `dryRun: false` и этим `ifRev`. Без него правка
   отказывает — забор ревизии связывает предпросмотр с применением.
5. Проверка: `unica.check {at}` на модуле. Новые находки важности `error`
   блокируют.

## MCP examples

### Предпросмотр замены тела метода

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.apply",
    "arguments": {
      "at": "main:CommonModule.Пример",
      "ops": [
        {
          "op": "code.replace",
          "args": {
            "at": "main:CommonModule.Пример.Module.Manager.Method.Выполнить.Body",
            "text": "    Возврат Истина;"
          }
        }
      ],
      "dryRun": true
    }
  }
}
```

### Применение с забором ревизии

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.apply",
    "arguments": {
      "at": "main:CommonModule.Пример",
      "ops": [
        {
          "op": "code.insert",
          "args": {
            "at": "main:CommonModule.Пример.Module.Manager.Body",
            "text": "Процедура Выполнить() Экспорт\nКонецПроцедуры"
          }
        }
      ],
      "dryRun": false,
      "ifRev": "<rev из предпросмотра>"
    }
  }
}
```
