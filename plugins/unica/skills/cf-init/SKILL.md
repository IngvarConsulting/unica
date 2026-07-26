---
name: cf-init
description: Создать пустую конфигурацию 1С (scaffold XML-исходников). Используй когда нужно начать новую конфигурацию с нуля
argument-hint: <Name> [-Synonym <name>] [-OutputDir src]
allowed-tools:
  - Bash
  - Read
  - Glob
---

# /cf-init — Создание пустой конфигурации 1С

## MCP routing

- Preferred path: use MCP `unica` tool `unica.cf.init`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.cf.init`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.

Создаёт scaffold исходников пустой конфигурации 1С: `Configuration.xml`, `Languages/Русский.xml`.

## MCP параметры

| Параметр | Описание |
|----------|----------|
| `Name` | Имя конфигурации (обязат.) |
| `Synonym` | Синоним (= Name если не указан) |
| `OutputDir` | Каталог для создания (default: `src`) |
| `Version` | Версия конфигурации |
| `Vendor` | Поставщик |
| `CompatibilityMode` | Режим совместимости (default: `Version8_3_27`) |

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cf.init",
    "arguments": {
      "cwd": "<workspace>",
      "Name": "DemoConfiguration",
      "Synonym": "Демо конфигурация",
      "OutputDir": "src",
      "Vendor": "Ingvar Consulting",
      "dryRun": false
    }
  }
}
```

## Примеры

### Базовая конфигурация

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cf.init",
    "arguments": {
      "cwd": "<workspace>",
      "Name": "МояКонфигурация",
      "Synonym": "Моя конфигурация",
      "OutputDir": "test-tmp/cf",
      "dryRun": false
    }
  }
}
```

### С версией и поставщиком

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cf.init",
    "arguments": {
      "cwd": "<workspace>",
      "Name": "TestCfg",
      "Synonym": "Тестовая",
      "Version": "1.0.0.1",
      "Vendor": "Фирма 1С",
      "OutputDir": "test-tmp/cf2",
      "dryRun": false
    }
  }
}
```

### Другой режим совместимости

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cf.init",
    "arguments": {
      "cwd": "<workspace>",
      "Name": "TestCfg",
      "CompatibilityMode": "Version8_3_27",
      "OutputDir": "test-tmp/cf3",
      "dryRun": false
    }
  }
}
```

## Верификация

После создания проверь каталог конфигурации через MCP `unica` read-only инструменты.

### Проверить созданную структуру

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cf.info",
    "arguments": {
      "cwd": "<workspace>",
      "ConfigPath": "test-tmp/cf"
    }
  }
}
```

### Валидировать XML конфигурации

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cf.validate",
    "arguments": {
      "cwd": "<workspace>",
      "ConfigPath": "test-tmp/cf"
    }
  }
}
```
