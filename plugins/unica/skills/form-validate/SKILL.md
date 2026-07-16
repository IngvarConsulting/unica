---
name: form-validate
description: Валидация управляемой формы 1С. Используй после создания или модификации формы для проверки корректности. При наличии BaseForm автоматически проверяет callType и ID расширений
argument-hint: <FormPath> [-Detailed] [-MaxErrors 30]
allowed-tools:
  - Bash
  - Read
  - Glob
---

# /form-validate — валидация управляемой формы 1С

## MCP routing

- Preferred path: use MCP `unica` tool `unica.form.validate`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.form.validate`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.

Проверяет Form.xml на структурные ошибки: уникальность ID, наличие companion-элементов, корректность ссылок DataPath и команд. События формы и элементов проверяются по платформенной матрице с типом прямого главного реквизита; duplicate, пустой handler, неверный element/event и неподходящий контекст возвращают стабильные коды `FORM_EVENT_*`. `callType` допустим только в форме с прямым `BaseForm` и только со значениями `Before`, `After`, `Override`. Если у заимствованной формы нет доступного прямого главного реквизита ни в `Form`, ни в `BaseForm`, object-specific binding помечается предупреждением `FORM_EVENT_CONTEXT_UNKNOWN` как непроверенный; editor при добавлении такого binding остаётся консервативным и отклоняет изменение.

## Параметры

| Параметр  | Обяз. | Умолч. | Описание                                |
|-----------|:-----:|---------|-----------------------------------------|
| FormPath  | да    | —       | Путь к файлу Form.xml                   |
| Detailed  | нет   | —       | Подробный вывод (все проверки, включая успешные) |
| MaxErrors | нет   | 30      | Остановиться после N ошибок              |

## MCP вызов

### Каталог формы

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.form.validate",
    "arguments": {
      "cwd": "<workspace>",
      "FormPath": "Catalogs/Номенклатура/Forms/ФормаЭлемента"
    }
  }
}
```

### Прямой путь к Form.xml

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.form.validate",
    "arguments": {
      "cwd": "<workspace>",
      "FormPath": "src/МояОбработка/Forms/Форма/Ext/Form.xml"
    }
  }
}
```
