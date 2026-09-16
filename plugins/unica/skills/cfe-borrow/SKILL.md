---
name: cfe-borrow
description: Заимствование объектов из конфигурации 1С в расширение (CFE). Используй когда нужно перехватить метод, изменить форму или добавить реквизит к существующему объекту конфигурации
argument-hint: -ExtensionPath <path> -ConfigPath <path> -Object "Catalog.Контрагенты.Form.ФормаЭлемента" -BorrowMainAttribute
allowed-tools:
  - Bash
  - Read
  - Glob
---

# /cfe-borrow — Заимствование объектов из конфигурации

## MCP routing

- **Канонической операции заимствования на поверхности нет.** Разбор трёх
  ролей (`own`, `borrowed`, корень расширения), проекция в `props` и форма
  будущей операции `object.borrow` держит отдельная
  архитектурная записка Unica; до её решения поверхность заимствование не
  пишет.
- Не зови внутренние адаптеры напрямую: они спрятаны за MCP `unica`.
- Готовность проверяет `unica.check`, результат читает `unica.view` по адресу.

Что делать сейчас: сформировать дескриптор заимствованного объекта по формату
ниже (`ObjectBelonging>Adopted` плюс `ExtendedConfigurationObject` с UUID
объекта родителя), зарегистрировать его в `ChildObjects` корня расширения,
затем проверить `unica.check` и прочитать `unica.view`. Признак заимствования —
именно `ExtendedConfigurationObject`: корень расширения тоже `Adopted`, и
читать его как заимствованный объект — дефект.

Если нужна операция — сообщи о пробеле контракта Unica MCP и сошлись на
записку.

## Примеры

### Заимствовать один объект

Поля, которые должен нести записанный файл:

```json
{
  "ExtensionPath": "src",
  "ConfigPath": "C:\\cfsrc\\erp",
  "Object": "Catalog.Контрагенты"
}
```

### Заимствовать форму

Поля, которые должен нести записанный файл:

```json
{
  "ExtensionPath": "src",
  "ConfigPath": "C:\\cfsrc\\erp",
  "Object": "Catalog.Контрагенты.Form.ФормаЭлемента"
}
```

### Несколько объектов за раз

Поля, которые должен нести записанный файл:

```json
{
  "ExtensionPath": "src",
  "ConfigPath": "C:\\cfsrc\\erp",
  "Object": "Catalog.Контрагенты ;; CommonModule.ОбщийМодуль ;; Enum.ВидыОплат"
}
```

### Заимствовать форму с основным реквизитом

Поля, которые должен нести записанный файл:

```json
{
  "ExtensionPath": "src",
  "ConfigPath": "C:\\cfsrc\\erp",
  "Object": "Catalog.Номенклатура.Form.ФормаЭлемента",
  "BorrowMainAttribute": true
}
```

### Заимствовать форму со всеми реквизитами объекта

Поля, которые должен нести записанный файл:

```json
{
  "ExtensionPath": "src",
  "ConfigPath": "C:\\cfsrc\\erp",
  "Object": "Catalog.Номенклатура.Form.ФормаЭлемента",
  "BorrowMainAttribute": "All"
}
```

## Верификация

Проверка расширения — `unica.check` на корне набора-расширения (`ext` — имя набора типа `EXTENSION` в `v8project.yaml`); валидатор `cfe` выбирается по виду набора, вердикт в `data.status`.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.check",
    "arguments": {
      "at": "ext:Configuration"
    }
  }
}
```
