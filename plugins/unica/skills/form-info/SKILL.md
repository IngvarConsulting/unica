---
name: form-info
description: Анализ структуры управляемой формы 1С (Form.xml) — элементы, реквизиты, команды, события. Используй для понимания формы — при написании модуля формы, анализе обработчиков и элементов
argument-hint: <FormPath>
allowed-tools:
  - Bash
  - Read
  - Glob
---

# /form-info — Компактная сводка формы

## MCP routing

- Preferred path: use MCP `unica` tool `unica.form.info`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.form.info`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.

Читает Form.xml и выводит дерево элементов, реквизиты с типами, команды, события. Заменяет чтение тысяч строк XML.

В выводе показывает `Поддержка` для объекта-владельца формы по `Ext/ParentConfigurations.bin`. Если форма принадлежит объекту поставщика на замке, сначала оцени CFE/release-support путь; сам `unica.form.info` только читает состояние.

## MCP вызов

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.form.info",
    "arguments": {
      "cwd": "<workspace>",
      "FormPath": "src/Catalogs/Номенклатура/Forms/ФормаЭлемента"
    }
  }
}
```

## Параметры

| Параметр | Обязательный | Описание |
|----------|:------------:|----------|
| FormPath | один из двух | Путь к файлу Form.xml |
| sourceSet | один из двух | Имя набора исходников из `v8project.yaml` |
| metadataPath | один из двух | Логический адрес, например `Catalog.<Объект>.Form.<Форма>` |
| Expand   | нет | Раскрыть свёрнутую секцию по имени или title, `*` — все |
| Limit    | нет | Макс. строк (по умолчанию 150) |
| Offset   | нет | Пропустить N строк (пагинация) |

Селектор цели ровно один: либо `sourceSet` + `metadataPath`, либо `FormPath`.
Оба сразу отклоняются кодом `selector_conflict` (ADR-0048).

Вывод самодокументирован. `[Group:AH]`/`[Group:AV]` = AlwaysHorizontal/AlwaysVertical.

## Логический адрес вместо пути

`unica.form.info` принимает либо логический селектор, либо файловый путь —
ровно один из двух. Оба сразу отклоняются кодом `selector_conflict`.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.form.info",
    "arguments": {
      "cwd": "<workspace>",
      "sourceSet": "<имя набора>",
      "metadataPath": "Catalog.<Объект>.Form.<Форма>"
    }
  }
}
```

Имя набора даёт `unica.project.map`, адрес — `unica.source.resolve`, а `unica.source.locate` переводит
в адрес путь, найденный иначе. Файловый селектор сохраняется до
отдельного среза его снятия (ADR-0048).
