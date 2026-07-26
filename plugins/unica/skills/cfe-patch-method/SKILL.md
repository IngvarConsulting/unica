---
name: cfe-patch-method
description: Генерация перехватчика процедуры в расширении 1С (CFE). Используй для Before/After-перехвата существующей процедуры без параметров у заимствованного объекта
argument-hint: -ExtensionPath <path> -ModulePath "Catalog.X.ObjectModule" -MethodName "ОбновитьДанные" -InterceptorType Before
allowed-tools:
  - Bash
  - Read
  - Glob
---

# /cfe-patch-method — Генерация перехватчика процедуры

## MCP routing

- Preferred path: use MCP `unica` tool `unica.cfe.patch_method`; `unica` owns XML/JSON DSL work and refreshes related workspace caches after mutations.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Execution path: call MCP `unica` tool `unica.cfe.patch_method`; skill-local operation scripts are not part of the workflow.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested the change; otherwise keep the default dry run.

Генерирует `.bsl` файл с декоратором `&Перед` или `&После` для
заимствованного объекта расширения. Создаёт файл или дописывает в существующий
и в той же атомарной транзакции помечает соответствующее свойство модуля как
`xr:State=Extended` в XML-дескрипторе. Для заимствованной формы это состояние
уже создаёт `cfe.borrow`, поэтому повторная XML-запись идемпотентна.

## Предусловие

Объект должен быть заимствован в расширение (`/cfe-borrow`) и зарегистрирован
в его `Configuration.xml`. Для формы также обязательны заимствованные wrapper и
`Form.xml`. Инструмент использует непустой `NamePrefix` расширения для имени
процедуры и отказывает до записи, если ownership или профиль `2.20` не
подтверждены.

Если дескриптор уже содержит несовместимое или дублирующее состояние того же
свойства, инструмент отказывает до записи BSL. При конкурентном изменении
дескриптора или модуля транзакция также завершается без частичной публикации.

Текущий v1 поддерживает только существующую процедуру без параметров. Он не
читает исходный модуль базовой конфигурации и поэтому не доказывает наличие и
сигнатуру метода. Перед вызовом нужно самостоятельно убедиться, что
`MethodName` — реальная процедура без параметров. Функции, параметры,
`ModificationAndControl`, `Around` и специальные form-handler semantics будут
спроектированы отдельно.

## Параметры

| Параметр | Описание | По умолчанию |
|----------|----------|--------------|
| `ExtensionPath` | Путь к расширению (обязат.) | — |
| `ModulePath` | Путь к модулю (обязат.) | — |
| `MethodName` | Имя перехватываемой процедуры без параметров (обязат.) | — |
| `InterceptorType` | `Before` / `After` (обязат.) | — |
| `Context` | Явная директива контекста, если она допустима для роли модуля | см. ниже |
| `IsFunction` | Зарезервировано; в v1 допускается только `false` | false |

### Контекст

- Для `ObjectModule`, `ManagerModule`, `RecordSetModule` и
  `ValueManagerModule` параметр `Context` не указывается, директива не
  генерируется.
- Для `CommonModule` отсутствие `Context` сохраняет контексты, заданные
  свойствами модуля. `НаСервере` допустимо только при `Server=true`, а
  `НаКлиенте` — при включённом клиентском контексте.
- Для модуля формы по умолчанию используется `НаСервере`; также допустимы
  `НаКлиенте` и `НаСервереБезКонтекста`.

## Формат ModulePath

Ниже приведены только репрезентативные примеры. Точная матрица EDT 8.3.27,
принятая текущей грамматикой, содержит 51 допустимое сочетание типа и роли
модуля. Они сводятся к шести физическим BSL-компоновкам:
`CommonModule`, `ObjectModule`, `ManagerModule`, `RecordSetModule`, `Form` и
`ValueManagerModule`.

| ModulePath | Файл |
|------------|------|
| `Catalog.X.ObjectModule` | `Catalogs/X/Ext/ObjectModule.bsl` |
| `Catalog.X.ManagerModule` | `Catalogs/X/Ext/ManagerModule.bsl` |
| `Catalog.X.Form.Y` | `Catalogs/X/Forms/Y/Ext/Form/Module.bsl` |
| `CommonModule.X` | `CommonModules/X/Ext/Module.bsl` |
| `Constant.X.ValueManagerModule` | `Constants/X/Ext/ValueManagerModule.bsl` |
| `Document.X.ObjectModule` | `Documents/X/Ext/ObjectModule.bsl` |
| `Document.X.Form.Y` | `Documents/X/Forms/Y/Ext/Form/Module.bsl` |

Таблица не является полным перечнем 51 пути. Роли доступны только для типов,
где они объявлены платформой 8.3.27.

## Типы перехвата

| InterceptorType | Декоратор | Назначение |
|-----------------|-----------|------------|
| `Before` | `&Перед` | Код до вызова оригинального метода |
| `After` | `&После` | Код после вызова оригинального метода |

## MCP вызов

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cfe.patch_method",
    "arguments": {
      "cwd": "<workspace>",
      "ExtensionPath": "src/extensions/MyExtension",
      "ModulePath": "Catalog.Контрагенты.ObjectModule",
      "MethodName": "ОбновитьДанные",
      "InterceptorType": "Before",
      "IsFunction": false,
      "dryRun": false
    }
  }
}
```

## Примеры

### Перехват Перед в модуле объекта

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cfe.patch_method",
    "arguments": {
      "cwd": "<workspace>",
      "ExtensionPath": "src",
      "ModulePath": "Catalog.Контрагенты.ObjectModule",
      "MethodName": "ОбновитьДанные",
      "InterceptorType": "Before",
      "dryRun": false
    }
  }
}
```

### Перехват обычной процедуры формы После на клиенте

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cfe.patch_method",
    "arguments": {
      "cwd": "<workspace>",
      "ExtensionPath": "src",
      "ModulePath": "Document.Заказ.Form.ФормаДокумента",
      "MethodName": "ОбновитьОтображение",
      "InterceptorType": "After",
      "Context": "НаКлиенте",
      "dryRun": false
    }
  }
}
```

### Перехват процедуры общего модуля

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.cfe.patch_method",
    "arguments": {
      "cwd": "<workspace>",
      "ExtensionPath": "src",
      "ModulePath": "CommonModule.ОбщийМодуль",
      "MethodName": "ОбновитьДанные",
      "InterceptorType": "Before",
      "dryRun": false
    }
  }
}
```
## Генерируемый код (Before)

```bsl
&Перед("ОбновитьДанные")
Процедура Расш1_ОбновитьДанные()
	// TODO: код перед вызовом оригинального метода
КонецПроцедуры
```
