---
name: erf-init
description: Создать пустой make-ready scaffold внешнего отчёта 1С (ERF) в корне Designer/platform-XML external source-set, с модулем объекта и опциональной управляемой формой. Используй при запросе создать новый внешний отчёт с нуля; не используй для обычного Report внутри конфигурации.
---

# Создание внешнего отчёта ERF

## MCP routing

- **Канонической операции создания внешнего отчёта на поверхности нет.** `unica.apply`
  правит существующий узел; дескриптор внешнего объекта вне конфигурации ни
  одна операция не заводит.
- Сформируй дескриптор и `ObjectModule.bsl` файловыми средствами по формату
  ниже, объяви набор нужного типа в `v8project.yaml`, затем проверь
  `unica.check {}` и прочитай `unica.view {}`: набор обязан читаться как
  `sourceFormat=platform_xml`.
- Сборку `.erf` словарь тоже не публикует: `artifact.build` собирает только
  `.cf` и `.cfe`. Сообщай это как пробел контракта, а не обходи runner-ом.
- Чтение и проверка идут через MCP `unica` (`unica.view`, `unica.check`); внутренние adapters и skill-local scripts не вызывать.
- Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`ifRevRequiredOnApply`. Контракт вызова бери оттуда, а не из этого текста;
выбирай только операцию с `implemented: true` и не выдумывай аргументов
записи с `argsSchema: null`; превью исполнением не является. Не обходи
контракт прямым runner-ом.
- Сборку `.epf`/`.erf` из исходников словарь `unica.run` не публикует: `artifact.build` собирает только `.cf` и `.cfe`. Сообщай это как пробел контракта, а не обходи runner-ом.

## Порядок работы

1. Убедиться, что `v8project.yaml` использует Designer mode: явно `format: DESIGNER` либо поле `format` отсутствует и действует Designer default v8-runner. Skill создаёт platform XML, а EDT external-project layout не поддерживается.
2. Сохранить существующие `workPath`, `infobase`, credentials и local overrides. Не заменять connection string и не инициализировать существующую ИБ ради scaffold.
3. Найти source-set с `type: EXTERNAL_REPORTS` и передать его `path` как `OutputDir` без вложенного подкаталога. v8-runner ищет descriptors непосредственно в корне source-set.
4. Если source-set ещё не объявлен, создать scaffold в выбранном новом каталоге, затем явно добавить этот каталог как корень Designer source-set. Проверить регистрацию через `unica.view {}`: `kind=external_report`, `sourceFormat=platform_xml`.
5. Передать `FormName`, только если нужна пустая управляемая форма. Без него создаются descriptor и `ObjectModule.bsl`; пустая СКД автоматически не добавляется.
6. Записав файлы, проверить набор: `unica.check {}` и `unica.view {}` — превью и применение тут не при чём, операции нет.
7. Добавлять СКД позже через `dcs-*`/`template-*`. Публикацию артефакта не обещать: `artifact.build` в `unica.run` `.erf` не собирает и отвечает `unsupported_operation`.

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

Не вызывай `infobase.create` ради scaffold или существующей проектной базы: он заводит базу, а для scaffold это не нужно. Для существующей connection сохранить настройки без переинициализации; `db-auth-check` может классифицировать только уже предоставленное runtime evidence и не запускает auth probe.

## Параметры

| Параметр | Назначение |
| --- | --- |
| `Name` | Имя внешнего отчёта, обязательно |
| `Synonym` | Русский синоним; по умолчанию равен `Name` |
| `OutputDir` | Корень external source-set, обязательно |
| `FormName` | Опциональная пустая управляемая форма |

## Примеры


Поля, которые должен нести записанный дескриптор:

```json
{
  "Name": "Остатки",
  "Synonym": "Остатки товаров",
  "OutputDir": "src/external-reports",
  "FormName": "ФормаОтчета"
}
```

## Верификация

Проверь результат до того, как объявлять объект готовым. Проверить, что созданы `<Name>.xml`, `<Name>/Ext/ObjectModule.bsl` и, если запрошена форма, три файла под `<Name>/Forms/`. Форму дополнительно проверить `unica.check` по адресу её узла. Отдельный generic Meta validator не использовать: он не принимает root `ExternalReport`. Не создавать `Configuration.xml`, platform-generated CDFI sidecar или СКД без запроса; legitimate external descriptor может называться `ConfigDumpInfo.xml`, если пользователь выбрал такое имя объекта.

Сборку и загрузку артефакта словарь `unica.run` не публикует: `artifact.build` отвечает `unsupported_operation` на `.epf`/`.erf`, а `cf.import` принимает только `.cf` и `.cfe`. Сообщай публикацию как пробел контракта Unica MCP.
