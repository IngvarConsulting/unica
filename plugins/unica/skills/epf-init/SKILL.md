---
name: epf-init
description: Создать пустой make-ready scaffold внешней обработки 1С (EPF) в корне Designer/platform-XML external source-set, с модулем объекта и опциональной управляемой формой. Используй при запросе создать новую внешнюю обработку с нуля; не используй для обычного DataProcessor внутри конфигурации.
---

# Создание внешней обработки EPF

## MCP routing

- Использовать MCP `unica` tool `unica.epf.init` для scaffold XML/BSL.
- Не вызывать внутренние adapters и не добавлять skill-local scripts.
- Текущий runtime-контракт: `unica.runtime.execute` — preview-only и вызывается только с `dryRun: true`; любой applied-режим возвращает fail-closed до workspace discovery и process spawn. Preview не является runtime verification. Не обходи этот отказ прямым runner-ом, через `unica.build.*` или fallback через `unica.runtime.job.*`.
- Для будущей сборки результата предпросмотреть `operation=make` через `v8-runner`; текущий вызов не создаёт артефакт.

## Порядок работы

1. Убедиться, что `v8project.yaml` использует Designer mode: явно `format: DESIGNER` либо поле `format` отсутствует и действует Designer default v8-runner. Skill создаёт platform XML, а EDT external-project layout не поддерживается.
2. Сохранить существующие `workPath`, `infobase`, credentials и local overrides. Не заменять connection string и не инициализировать существующую ИБ ради scaffold.
3. Найти source-set с `type: EXTERNAL_DATA_PROCESSORS` и передать его `path` как `OutputDir` без вложенного подкаталога. v8-runner ищет descriptors непосредственно в корне source-set.
4. Если source-set ещё не объявлен, создать scaffold в выбранном новом каталоге, затем явно добавить этот каталог как корень Designer source-set. Проверить регистрацию через `unica.project.map`: `kind=external_processor`, `sourceFormat=platform_xml`.
5. Передать `FormName`, только если нужна пустая управляемая форма. Без него создаются descriptor и `ObjectModule.bsl`.
6. Сначала проверить точный список файлов через `dryRun: true`; при явном запросе пользователя повторить с `dryRun: false`.
7. Предпросмотреть `unica.runtime.execute operation=make`; для будущей applied-сборки в `v8project.yaml` потребуется доступная `infobase.connection`. Текущий preview не собирает `.epf`.

`Name` и `FormName` должны быть идентификаторами 1С. Существующие descriptor или одноимённый каталог не перезаписываются. При `format: EDT` остановиться и объяснить несовместимость, не создавать Designer XML внутри EDT source-set.

В существующем валидном `v8project.yaml` добавить только этот фрагмент, используя ключ `source-set` в единственном числе и не затирая остальные поля:

```yaml
source-set:
  - name: external-processors
    type: EXTERNAL_DATA_PROCESSORS
    path: src/external-processors
```

Для нового изолированного workspace полный минимальный пример имеет также обязательный runtime-контекст:

```yaml
workPath: build/runtime
execution_timeout: 300000
format: DESIGNER
builder: DESIGNER
infobase:
  connection: 'File=build/ib'
source-set:
  - name: external-processors
    type: EXTERNAL_DATA_PROCESSORS
    path: src/external-processors
```

`operation=init` допустима только по явному запросу или разрешению пользователя и только для новой изолированной пустой ИБ. Не выполнять `init` ради самого scaffold или для существующей проектной базы. Для существующей connection использовать её без переинициализации; при проблемах доступа следовать skill `v8-runner`/`db-auth-check`.

## Параметры

| Параметр | Назначение |
| --- | --- |
| `Name` | Имя внешней обработки, обязательно |
| `Synonym` | Русский синоним; по умолчанию равен `Name` |
| `OutputDir` | Корень external source-set, обязательно |
| `FormName` | Опциональная пустая управляемая форма |

## Примеры

Preview обработки с формой:

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.epf.init",
    "arguments": {
      "cwd": "<workspace>",
      "Name": "ИмпортТоваров",
      "Synonym": "Импорт товаров",
      "OutputDir": "src/external-processors",
      "FormName": "ОсновнаяФорма",
      "dryRun": true
    }
  }
}
```

Создание после проверки preview:

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.epf.init",
    "arguments": {
      "cwd": "<workspace>",
      "Name": "ИмпортТоваров",
      "Synonym": "Импорт товаров",
      "OutputDir": "src/external-processors",
      "FormName": "ОсновнаяФорма",
      "dryRun": false
    }
  }
}
```

## Верификация

`unica.epf.init` разбирает весь сгенерированный XML до публикации. Проверить, что созданы `<Name>.xml`, `<Name>/Ext/ObjectModule.bsl` и, если запрошена форма, три файла под `<Name>/Forms/`. Форму дополнительно проверить через `unica.form.validate` с путём к её `Ext/Form.xml`. Отдельный generic Meta validator не использовать: он не принимает root `ExternalDataProcessor`. Не создавать `Configuration.xml` или platform-generated CDFI sidecar; legitimate external descriptor может называться `ConfigDumpInfo.xml`, если пользователь выбрал такое имя объекта.

Предпросмотреть будущую команду сборки только с `dryRun: true`:

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "make",
      "sourceSet": "external-processors",
      "output": "build/external",
      "dryRun": true
    }
  }
}
```

Не заменять `dryRun` на `false`: applied `make` сейчас fail-closed.

Не использовать `operation=load` для `.epf`.
