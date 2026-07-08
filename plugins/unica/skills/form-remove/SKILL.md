---
name: form-remove
description: Удалить форму из объекта 1С (обработка, отчёт, справочник, документ и др.)
argument-hint: <ObjectName> <FormName>
disable-model-invocation: true
allowed-tools:
  - Bash
  - Read
  - Write
  - Edit
  - Glob
  - Grep
---

# /form-remove — Удаление формы

## MCP routing

- Preferred path: use MCP `unica` tool `unica.form.remove`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.form.remove`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.
- Vendor support guard runs inside `unica`; if it blocks a locked/read-only supported object, prefer CFE/release-support or an explicit support-state change plan instead of editing raw support metadata.

Удаляет форму и убирает её регистрацию из корневого XML объекта.

## Usage

Используй MCP `unica` tool `unica.form.remove` с `ObjectName`, `FormName` и `SrcDir`.

| Параметр   | Обязательный | По умолчанию | Описание                            |
|------------|:------------:|--------------|-------------------------------------|
| ObjectName | да           | —            | Имя объекта                         |
| FormName   | да           | —            | Имя формы для удаления              |
| SrcDir     | нет          | `src`        | Каталог исходников                  |

## MCP вызов

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.form.remove",
    "arguments": {
      "cwd": "<workspace>",
      "ObjectName": "Номенклатура",
      "FormName": "СтараяФорма",
      "SrcDir": "src/Catalogs",
      "dryRun": false
    }
  }
}
```

## Что удаляется

```
<SrcDir>/<ObjectName>/Forms/<FormName>.xml     # Метаданные формы
<SrcDir>/<ObjectName>/Forms/<FormName>/         # Каталог формы (рекурсивно)
```

## Что модифицируется

- `<SrcDir>/<ObjectName>.xml` — убирается `<Form>` из `ChildObjects`
- Если удаляемая форма была указана в `Default*Form`/`Auxiliary*Form` слоте объекта — значение этого слота очищается
- Опустевшие затронутые XML-элементы сохраняются в самозакрывающемся виде (`<ChildObjects/>`, `<Default*Form/>`)
