---
name: erf-init
description: Создать пустой make-ready scaffold внешнего отчёта 1С (ERF) в корне Designer/platform-XML external source-set, с модулем объекта и опциональной управляемой формой. Используй при запросе создать новый внешний отчёт с нуля; не используй для обычного Report внутри конфигурации.
---

# Создание внешнего отчёта ERF

## MCP routing

- Использовать MCP `unica` tool `unica.erf.init` для scaffold XML/BSL.
- Не вызывать внутренние adapters и не добавлять skill-local scripts.
- Для сборки результата использовать `v8-runner` через `unica.runtime.execute` с `operation=make`.

## Порядок работы

1. Убедиться, что `v8project.yaml` использует Designer mode: явно `format: DESIGNER` либо поле `format` отсутствует и действует Designer default v8-runner. Skill создаёт platform XML, а EDT external-project layout не поддерживается.
2. Сохранить существующие `workPath`, `infobase`, credentials и local overrides. Не заменять connection string и не инициализировать существующую ИБ ради scaffold.
3. Найти source-set с `type: EXTERNAL_REPORTS` и передать его `path` как `OutputDir` без вложенного подкаталога. v8-runner ищет descriptors непосредственно в корне source-set.
4. Если source-set ещё не объявлен, создать scaffold в выбранном новом каталоге, затем явно добавить этот каталог как корень Designer source-set. Проверить регистрацию через `unica.project.map`: `kind=external_report`, `sourceFormat=platform_xml`.
5. Передать `FormName`, только если нужна пустая управляемая форма. Без него создаются descriptor и `ObjectModule.bsl`; пустая СКД автоматически не добавляется.
6. Сначала проверить точный список файлов через `dryRun: true`; при явном запросе пользователя повторить с `dryRun: false`.
7. Добавлять СКД позже через `skd-*`/`template-*`, затем собрать `.erf` через `unica.runtime.execute operation=make`. Для make в `v8project.yaml` должна быть доступная `infobase.connection`.

`Name` и `FormName` должны быть идентификаторами 1С. Существующие descriptor или одноимённый каталог не перезаписываются. При `format: EDT` остановиться и объяснить несовместимость, не создавать Designer XML внутри EDT source-set.

В существующем валидном `v8project.yaml` добавить только этот фрагмент, используя ключ `source-set` в единственном числе и не затирая остальные поля:

```yaml
source-set:
  - name: external-reports
    type: EXTERNAL_REPORTS
    path: src/external-reports
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
  - name: external-reports
    type: EXTERNAL_REPORTS
    path: src/external-reports
```

`operation=init` допустима только по явному запросу или разрешению пользователя и только для новой изолированной пустой ИБ. Не выполнять `init` ради самого scaffold или для существующей проектной базы. Для существующей connection использовать её без переинициализации; при проблемах доступа следовать skill `v8-runner`/`db-auth-check`.

## Параметры

| Параметр | Назначение |
| --- | --- |
| `Name` | Имя внешнего отчёта, обязательно |
| `Synonym` | Русский синоним; по умолчанию равен `Name` |
| `OutputDir` | Корень external source-set, обязательно |
| `FormName` | Опциональная пустая управляемая форма |

## Примеры

Preview отчёта с пустой управляемой формой:

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.erf.init",
    "arguments": {
      "cwd": "<workspace>",
      "Name": "Остатки",
      "Synonym": "Остатки товаров",
      "OutputDir": "src/external-reports",
      "FormName": "ФормаОтчета",
      "dryRun": true
    }
  }
}
```

Создание отчёта с пустой управляемой формой:

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.erf.init",
    "arguments": {
      "cwd": "<workspace>",
      "Name": "Остатки",
      "Synonym": "Остатки товаров",
      "OutputDir": "src/external-reports",
      "FormName": "ФормаОтчета",
      "dryRun": false
    }
  }
}
```

## Верификация

`unica.erf.init` разбирает весь сгенерированный XML до публикации. Проверить, что созданы `<Name>.xml`, `<Name>/Ext/ObjectModule.bsl` и, если запрошена форма, три файла под `<Name>/Forms/`. Форму дополнительно проверить через `unica.form.validate` с путём к её `Ext/Form.xml`. `unica.meta.validate` не использовать: он не принимает root `ExternalReport`. Не создавать `Configuration.xml`, `ConfigDumpInfo.xml` или СКД без запроса.

Перед реальной сборкой проверить ту же команду с `dryRun: true`. Повторить с `dryRun: false` только если пользователь явно запросил сборку ERF:

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "make",
      "sourceSet": "external-reports",
      "output": "build/external",
      "dryRun": true
    }
  }
}
```

После проверки заменить только `dryRun` на `false`.

Не использовать `operation=load` для `.erf`.
