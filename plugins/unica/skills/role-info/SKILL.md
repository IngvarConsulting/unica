---
name: role-info
description: Компактная сводка прав роли 1С из Rights.xml — объекты, права, RLS, шаблоны ограничений. Используй для аудита прав — какие объекты и действия доступны, ограничения RLS
argument-hint: <RightsPath>
allowed-tools:
  - Bash
  - Read
---

# /role-info — анализ роли 1С

## MCP routing

- Preferred path: use MCP `unica` tool `unica.role.info`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.role.info`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.

Парсит `Rights.xml` роли и выдаёт компактную сводку: объекты сгруппированы по типу, показаны только разрешённые права. Сжатие: тысячи строк XML → 50–150 строк текста.

В выводе показывает `Поддержка` роли по `Ext/ParentConfigurations.bin`: на замке, редактируется, снято с поддержки, read-only или не на поддержке. Для роли поставщика на замке сначала фиксируй release-support решение, а не меняй права напрямую.

## Использование

```
/role-info <RightsPath>
```

**RightsPath** — путь к файлу `Rights.xml` роли (обычно `Roles/ИмяРоли/Ext/Rights.xml`).

## MCP вызов

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.role.info",
    "arguments": {
      "cwd": "<workspace>",
      "RightsPath": "<path>",
      "OutFile": "<output.txt>"
    }
  }
}
```

### Параметры

| Параметр | Обязательный | Описание |
|----------|:------------:|----------|
| `-RightsPath` | да | Путь к Rights.xml |
| `-ShowDenied` | нет | Показать запрещённые права (по умолчанию скрыты) |
| `-Limit` | нет | Макс. строк вывода (по умолчанию `150`). `0` = без ограничений |
| `-Offset` | нет | Пропустить N строк — для пагинации (по умолчанию `0`) |
| `-OutFile` | нет | Записать результат в файл (UTF-8 BOM). Без этого — вывод в консоль |

**Важно:** Всегда используй `-OutFile` и читай результат через Read tool. Прямой вывод в консоль через bash ломает кириллицу.

Для большой роли при усечении вывода:
```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.role.info",
    "arguments": {
      "cwd": "<workspace>",
      "RightsPath": "<path>",
      "Offset": 150,
      "OutFile": "<output.txt>"
    }
  }
}
```
