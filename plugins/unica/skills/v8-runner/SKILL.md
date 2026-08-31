---
name: v8-runner
description: "Используй когда задача про runtime 1С, информационная база или workspace: v8project.yaml, первый workspace, build/dump/convert source-set, load/make CF/CFE, build/dump/make EPF/ERF external source-set, syntax/tests/launch, extensions, tools-download. Не используй для точечного чтения или редактирования XML метаданных, форм, СКД, MXL, ролей, подсистем."
argument-hint: "[config-init|init|build|dump|make|load|syntax|test|launch|extensions|tools-download] [connection|sourceSet|path|output]"
allowed-tools:
  - Bash
  - Read
  - Glob
  - AskUserQuestion
---

# /v8-runner — runtime workflows через MCP Unica

## MCP routing

- Preferred path: use MCP `unica` tool `unica.runtime.execute` both to preview typed v8-runner arguments and to run them.
- По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `dryRun: true` показывает запланированную команду без побочных эффектов, а `dryRun: false` действительно исполняет операцию и возвращает её терминальный результат в этом же вызове. Preview исполнением не является — не выдавай его за проверку runtime. Перед применённым вызовом скажи пользователю, чем операция рискует: результат несёт названную причину (`runtime_risk_critical_non_abortable`, `runtime_risk_publication_without_bounded_recovery`, `runtime_risk_unproven_process_ownership`, `runtime_risk_detached_child`) предупреждением.
- Длинную работу, которую вызов ждать не должен, запускай через `unica.runtime.job.start` как явно выбранный workflow с наблюдением через `status`/`wait`/`logs`. Не используй `unica.runtime.job.start` как запасной путь. Не обходи контракт прямым runner-ом или через `unica.build.*`.
- Do not start internal runner MCP servers, package launchers, or shell runners directly, including for maintainer/debug workflows. Report the public contract gap instead.

## Lifecycle одного вызова

- Применённый `unica.runtime.execute` принадлежит одному `tools/call`: он исполняет операцию синхронно и возвращает её терминальный результат. Прогресс по фазам этот путь не отдаёт — наблюдаемость длинной работы принадлежит долговременному заданию.
- Общая конфигурация пакета не увеличивает крайний срок хоста. Поэтому долгий применённый вызов может быть обрублен хостом по его сроку, и тогда терминальный результат теряется: работу, которую нельзя терять, запускай долговременным заданием.
- Целостность информационной базы после принудительного завершения раннера не обещана: отмена доводится до дочернего процесса, но непрерываемую критическую фазу свернуть безопасно нельзя. Предупреди об этом до применённого `build`, `load`, `init`, `test` и `extensions`.
- Операция, которую платформа исполняет отдельно сгруппированным процессом (`syntax` в режимах Designer/EDT, `launch`), несёт риск недоказанного владения этим процессом: результат назовёт причину, а очистка на всех путях ошибки не гарантирована.
- Не используй `unica.runtime.job.*` как fallback, продолжение или повтор `unica.runtime.execute`: долговременное задание — отдельный явно выбранный workflow, а не способ получить потерянный receipt.

## Работа, которую вызов ждать не должен

Если сборка длинная, а терять её результат нельзя, запусти её отдельным
долговременным заданием: оно живёт в отсоединённом процессе и переживает обрыв
вызова. После успешного `unica.project.status` предупреди пользователя, что
работа пойдёт фоном, и вызови:

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.job.start",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "build",
      "sourceSet": "<source-set>",
      "dryRun": false
    }
  }
}
```

Сохрани возвращённый `jobId`. Наблюдай фазу и heartbeat через
`unica.runtime.job.status`, ограниченно жди через `unica.runtime.job.wait`,
диагностические хвосты запрашивай через `unica.runtime.job.logs`. У обычной
сборки логи могут оставаться пустыми до завершения — это не признак зависания,
различать помогают фаза и heartbeat.

## Project health preflight

After clone or workspace initialization, and before `build` or `dump`, call
`unica.project.status` for the workspace. Read its two flags independently:

- `ready: false` blocks source operations until the source-set diagnostics are
  fixed; `sourceSet.path: .` is an error and should be replaced with a strict
  child such as `src/` in `v8project.yaml` after the sources are moved safely;
- `repositoryReady: false` means portable Git policy has not been proved. It
  does not mean that Unica is unusable without Git, but it blocks a claim that
  the project is ready for team work or another clone.

Explain `diagnostics[].remediation.steps` to the user. Entries under
`diagnostics[].remediation.commands` are structured suggestions, not permission
to edit `.gitignore`, `.gitattributes`, the Git index, or files. Never execute
them automatically; obtain the authority required for the particular change,
then call `unica.project.status` again after the approved fix.

## Быстрый выбор операции

| Намерение | MCP `operation` | `dryRun: false` |
|---|---|---|
| Создать `v8project.yaml` | `config-init` | пишет файл конфигурации |
| Инициализировать базу или workspace | `init` | создаёт runtime-состояние |
| Загрузить XML/EDT исходники в базу | `build` | применяет исходники; непрерываемая фаза |
| Выгрузить базу в исходники | `dump` | пишет исходники; без ограниченного восстановления |
| Конвертировать Designer/EDT sources | `convert` | пишет результат конвертации |
| Собрать CF/CFE/EPF/ERF артефакт | `make` | публикует артефакт |
| Загрузить CF/CFE артефакт | `load` | применяет артефакт; непрерываемая фаза |
| Проверить Designer-синтаксис | `syntax`, `mode=designer-*` | запускает конфигуратор; владение процессом не доказано |
| Проверить EDT-синтаксис | `syntax`, `mode=edt` | запускает EDT; владение процессом не доказано |
| Прогнать тесты | `test` | запускает раннер тестов; непрерываемая фаза |
| Запустить клиент с ожиданием завершения | `launch`, `waitForExit=true` | ждёт клиент в границах `waitTimeoutMs` |
| Синхронизировать extension properties | `extensions` | пишет свойства расширений |
| Скачать runner tools | `tools-download` | публикует инструменты в кеш |

Долгую работу, которую вызов ждать не должен, запускай тем же `operation` через
`unica.runtime.job.start` и наблюдай через `status`/`wait`/`logs`.

## Auth/license stop rules

- Если вывод операции похож на проблему лицензии 1С (`лиценз`, `license`, `HASP`, `nethasp`, `LM`, `No license`, `Лицензия не найдена`), остановись. Не лечи лицензию, не меняй службы, реестр, `nethasp.ini` или программную лицензию.
- Если база без указанного пользователя/пароля, не запускай auth probe: попроси пользователя указать credentials. Уже предоставленное свидетельство можно классифицировать только для явно проверенных `Администратор` или `Admin` с пустым паролем; после подтверждённых отказов спроси пользователя.
- Не сохраняй пароль в `v8project.yaml` молча. Если credentials нужно записать в connection string, предупреди пользователя и не коммить такой файл.
- Если `tools-download` падает на `failed to fetch latest release … 403`, это анонимный лимит GitHub API, а не ошибка проекта и не отказ 1С. Не повторяй вызов по кругу: назови пользователю причину прямо. Аутентифицировать запрос runner не умеет — переменной с токеном он не читает. Выходы: подождать сброса лимита; направить `V8TR_GITHUB_API_BASE_URL` на зеркало или прокси, которое добавит авторизацию (Unica пробрасывает окружение в runner, поэтому переменная доходит); либо положить готовый артефакт по настроенному пути вручную — для client MCP это `tools.client_mcp.extension.artifact.path`.

## Workspace init

Для пустого репозитория сначала создай `src/`, предпросмотри команду создания
`v8project.yaml`, затем реши с пользователем, применять ли её: применённый
`config-init` исполняется и несёт `runtime_risk_publication_without_bounded_recovery`,
потому что закреплённый runner пишет конфиг вне прерываемой транзакции —
прерванный запуск может оставить частичный результат. Не обходи вызов прямым
запуском runner-а.

Если исходники отсутствуют или `src/` пустой, считай существующую базу
источником правды и предпросмотри синхронный полный `dump`. Применённый dump
исполняется и несёт `runtime_risk_publication_without_bounded_recovery`: его
проверенная private-stage публикация защищает формат и rollback, но
постпроцессинг не имеет доказанного верхнего срока для terminal receipt. Если исходники уже есть, не выполняй `build` автоматически: спроси,
база или Git является источником правды.

### Предпросмотр нового `v8project.yaml`

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "config-init",
      "config": "./v8project.yaml",
      "connection": "File=build/ib",
      "dryRun": true
    }
  }
}
```

### Предпросмотр первичной инициализации runtime state

`init` содержит непрерываемую фазу: применённый запуск исполняется и несёт
`runtime_risk_critical_non_abortable`, поэтому отмена откладывается до конца
этой фазы.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "init",
      "dryRun": true
    }
  }
}
```

### Предпросмотр первичной выгрузки в `src/`

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "dump",
      "mode": "full",
      "dryRun": true
    }
  }
}
```

## Configuration examples

### Конфиг с серверной базой

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "config-init",
      "config": "./v8project.yaml",
      "connection": "Srvr=\"srv01\";Ref=\"dev\";",
      "dryRun": true
    }
  }
}
```

### EDT source format

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "config-init",
      "config": "./v8project.yaml",
      "format": "edt",
      "builder": "IBCMD",
      "dryRun": true
    }
  }
}
```

### Локальный overlay

Используй `v8project.local.yaml` для локальных `workPath`, `infobase.connection`, credentials, `tools`, `tests` и `mcp`. Не передавай local overlay как `config`. Не добавляй туда `source-set`, `format`, `builder` или `execution_timeout`: эти поля должны жить в основном проектном конфиге.

Бюджет runner-а задаётся через `execution_timeout` в `v8project.yaml` (миллисекунды, default `300000`, диапазон `1..=86400000`); одно это поле не переводит неклассифицированную операцию в допущенные. Не прокидывай отдельный `timeoutMs` в `unica.runtime.execute`: Unica не владеет таймаутом runner-а.

Если ignored EPF workspace уже содержит основной `v8project.yaml` только с
`EXTERNAL_DATA_PROCESSORS`, можно предпросмотреть привязку к личной локальной ИБ
через `config-init` с явными `config`, `sourceSet` и `connection`. Применённая
запись local overlay исполняется и несёт тот же
`runtime_risk_publication_without_bounded_recovery`, что и обычный `config-init`.
Не обходи её прямым запуском runner-а; в preview не передавай `format`,
`builder` или `force`.

## Build/load/artifacts

Примеры `build` и `load` ниже показаны с предпросмотром. Их применённые фазы
непрерываемы: отмена откладывается ради целостности информационной базы, поэтому
перед запуском с `dryRun: false` предупреди пользователя, а результат прочитай
вместе с названной причиной риска.

### Предпросмотр обычного build

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "build",
      "dryRun": true
    }
  }
}
```

Политика повтора живёт только в долговременном задании. Применённый
`unica.runtime.execute` по INV-MCP-RUNTIME-RECEIPT отвечает терминальным
результатом одной попытки и сам её не повторяет; предпросмотр показывает
нормализованную команду, но никакой попытки не делает. Для `unica.runtime.job.start` с операцией `build` без `fullRebuild: true`
Unica сначала запускает обычную сборку и не читает состояние поддержки
Platform XML.
Ровно одну полную повторную попытку вызывает только корректный структурированный
результат v8-runner после внешнего кода `4`, доказывающий завершившийся ошибкой
шаг partial load. Это фиксация сбоя, а не диагноз: классификатор
не определяет причину ошибки и на поддержку поставщика не ссылается.

Такой `build` запускается с `--json-message`, и v8-runner печатает не текстовый
прогресс, а один структурированный конверт в момент завершения процесса. Поэтому
`unica.runtime.job.logs` у долгого build остаётся пустым до конца сборки:
промежуточных строк не будет ни в `stdout.log`, ни в `stderr.log`. Не жди их и не
считай пустой лог признаком зависания — смотри `phase` и `heartbeat` через
`unica.runtime.job.status`. Явный `fullRebuild: true` сохраняет обычный текстовый
вывод раннера.

Перед каждой попыткой Unica повторно связывает основной `config` и соседний
`v8project.local.yaml` с тем же рабочим пространством. Изменение, появление или
удаление локального файла между попытками запрещает полный повтор: он может
менять `workPath`, информационную базу и исполняемые инструменты.

Явный `fullRebuild: true` запускает одну полную сборку без fallback. Произвольная
или неструктурированная ошибка, сбой другого шага, ошибка запуска процесса,
отмена, тайм-аут внешнего процесса, зафиксированный Unica, или усечённый вывод
повторную попытку не запускают. Закреплённая квитанция не содержит метаданные
отложенного внутреннего тайм-аута критического шага `v8-runner`: если такой шаг
после своего срока завершился точным структурированным partial-отказом, временный
слой не может отличить его от обычного отказа и допускает полный повтор. Если
полная повторная попытка тоже завершилась ошибкой, третьей попытки нет.
Комплексная переработка runtime/runner для v14 остаётся отдельной задачей, а этот
временный fallback её не заменяет.

### Предпросмотр build одного source-set

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "build",
      "sourceSet": "main",
      "dryRun": true
    }
  }
}
```

### Предпросмотр полной пересборки после branch switch/rebase

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "build",
      "fullRebuild": true,
      "dryRun": true
    }
  }
}
```

### Предпросмотр загрузки CF/CFE

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "load",
      "path": "build/config.cf",
      "mode": "load",
      "dryRun": true
    }
  }
}
```

### Предпросмотр загрузки с merge settings

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "load",
      "path": "build/config.cf",
      "mode": "merge",
      "settings": "merge-settings.xml",
      "dryRun": true
    }
  }
}
```

### Предпросмотр загрузки расширения

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "load",
      "path": "build/MyExtension.cfe",
      "extension": "MyExtension",
      "mode": "load",
      "dryRun": true
    }
  }
}
```

`operation=load` поддерживает только `mode=load` и `mode=merge`. Для `mode=merge` обязательно передавай `settings`; `mode=update` v8-runner отвергает.

## Dump/convert/artifacts

Перед `dump` проверь `git status --short`, чтобы не смешать чужие изменения с выгрузкой из базы.

`ConfigDumpInfo.xml` с корнем `<ConfigDumpInfo>` — platform-generated CDFI sidecar
и локальное состояние конкретной ИБ: не добавляй его в Git
и не используй как XML-исходник. Это правило не относится к metadata-файлу
реального объекта: legitimate metadata descriptor (включая external EPF/ERF)
с именем `ConfigDumpInfo.xml` remains source и должен храниться в Git.
На Windows, macOS и Linux verified transactional publication описывает
синхронный full dump (`mode=full`) только для DESIGNER source-set типа
`CONFIGURATION` или `EXTENSION`. Он исполняется и несёт названный риск: проверка
и публикация не имеют доказанного ограниченного восстановления, поэтому
прерванный прогон верхнего срока не гарантирует. Unica независимо проверяет установленную
платформу 8.3.27, подменяет выбранный target на private staging, проверяет
владельца и все XML version-bearing roots на exact raw `2.20`, затем атомарно с
rollback публикует целое дерево. Контракт публикации принадлежит ADR-0016:
привязку preimage и обязательный видимый отказ rollback уточняют
`INV-SOURCE-BOUND-PREIMAGES` и `INV-SOURCE-ROLLBACK-VISIBLE`, а OS-зависимая
реализация остаётся за `INV-PLATFORM-OS-BEHIND-FACADE`.

Применённый синхронный full dump для CONFIGURATION и EXTENSION исполняется и
несёт `runtime_risk_publication_without_bounded_recovery`. Асинхронный full dump
и dump для external source-set остаются недоступны отдельными гарантиями —
не классификацией ADR-0074, а отсутствием проверки private staged tree на
границе фонового задания и ограничением full dump на CONFIGURATION/EXTENSION;
для них доступен только preview. `incremental` и
`partial` исполняются, но до private
CDFI, точного receipt и divergence-safe merge (alkoleft/v8-runner-rust#30) их
результату в Git-visible root доверять нельзя без сверки исходников.

На Windows Unica проверяет локальную системную установку через no-follow
handles: доверенный владелец и DACL должны защищать install tree от изменения
вызывающим non-elevated пользователем, а ancestry — от удаления, замены или
перенаправления компонентов пути. На macOS и Linux Unica сверяет физические
маркеры DESIGNER, получает exact 8.3.27.x через sibling `ibcmd --version` и
требует root-owned, link-free install tree без group/world write и ACL; recovery
хранится отдельно от effective config и не содержит credentials.
Пользовательская или изменяемая установка отклоняется до запуска
`ibcmd`/`v8-runner`; остальные Unix пока fail-closed.

### Incremental dump

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "dump",
      "mode": "incremental",
      "dryRun": true
    }
  }
}
```

### Partial dump объекта

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "dump",
      "mode": "partial",
      "object": "Catalog:Номенклатура",
      "dryRun": true
    }
  }
}
```

### Partial dump нескольких объектов

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "dump",
      "mode": "partial",
      "objects": ["Catalog:Номенклатура", "Document:ЗаказПокупателя"],
      "dryRun": true
    }
  }
}
```

### Dump расширения или source-set

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "dump",
      "mode": "full",
      "extension": "MyExtension",
      "sourceSet": "MyExtension",
      "dryRun": true
    }
  }
}
```

### Convert Designer/EDT

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "convert",
      "sourceSet": "main",
      "output": "build/convert",
      "dryRun": true
    }
  }
}
```

### Предпросмотр экспорта CF/CFE/EPF/ERF

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "make",
      "sourceSet": "main",
      "output": "build/config.cf",
      "dryRun": true
    }
  }
}
```

### Предпросмотр публикации внешних обработок EPF

Для external source-set `EXTERNAL_DATA_PROCESSORS` параметр `output` задает каталог публикации, а не имя одного файла. Runner сам опубликует `.epf` по именам внешних обработок внутри source-set.

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

### Предпросмотр публикации внешних отчётов ERF

Для external source-set `EXTERNAL_REPORTS` параметр `output` также задает каталог публикации.

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

### Выгрузка внешних обработок/отчётов из базы

Выгрузка EPF/ERF теперь идет не через отдельный файл-скрипт, а через configured external source-set в `v8project.yaml`.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "dump",
      "mode": "full",
      "sourceSet": "external-processors",
      "dryRun": true
    }
  }
}
```

### Предпросмотр загрузки external source-set в базу

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "build",
      "sourceSet": "external-processors",
      "dryRun": true
    }
  }
}
```

## Syntax/tests/extensions

Все режимы `syntax`, `test` и `extensions` исполняются с названным риском. Даже
Designer syntax может породить отдельную группу процесса 1С, владение которой
закреплённый runner не доказывает на каждом аварийном пути; интерактивная
EDT-сессия и build/extension-фазы также не имеют ограниченного восстановления.

### Designer module syntax

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "syntax",
      "mode": "designer-modules",
      "server": true,
      "thinClient": true,
      "dryRun": true
    }
  }
}
```

### EDT syntax by projects

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "syntax",
      "mode": "edt",
      "projects": ["Configuration", "Tests"],
      "dryRun": true
    }
  }
}
```

### Предпросмотр YaXUnit all

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "test",
      "testRunner": "yaxunit",
      "testScope": "all",
      "fullOutput": true,
      "dryRun": true
    }
  }
}
```

### Предпросмотр YaXUnit module

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "test",
      "testRunner": "yaxunit",
      "testScope": "module",
      "module": "CommonModule.МоиТесты",
      "dryRun": true
    }
  }
}
```

### Предпросмотр Vanessa Automation

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "test",
      "testRunner": "va",
      "features": ["features/smoke.feature"],
      "filterTags": ["@smoke"],
      "ignoreTags": ["@wip"],
      "scenarioFilters": ["Open form"],
      "dryRun": true
    }
  }
}
```

### Предпросмотр extension properties

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "extensions",
      "sourceSet": "MyExtension",
      "dryRun": true
    }
  }
}
```

### Предпросмотр нескольких extension source-set

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "extensions",
      "sourceSets": ["Sales", "Warehouse"],
      "dryRun": true
    }
  }
}
```

## Tools

### Download Vanessa Automation

Если Vanessa Automation ещё не подготовлена в workspace, можно предпросмотреть
загрузку управляемого v8-runner артефакта. Применённый `tools-download`
исполняется и несёт `runtime_risk_publication_without_bounded_recovery`: до
появления прерываемой атомарной публикации оборванный запуск может оставить
частичный артефакт.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "tools-download",
      "tool": "vanessa",
      "dryRun": true
    }
  }
}
```

Для любого preview запуска Vanessa EPF по effective `tools.va.epf_path` должна
уже существовать. Предпросмотр `tools-download` с `dryRun: true` только
проверяет типизированные аргументы и не создаёт и не сохраняет артефакт.
Применённая загрузка со стандартной конфигурацией сохраняет EPF как
`build/tools/vanessa-automation-single.epf`; если project config переопределяет
путь, в `execute` для запуска можно использовать только уже существующий файл по
этому пути.

### Download client MCP extension

По умолчанию runner берёт готовый артефакт релиза и кладёт его в
`build/tools/client_mcp.cfe`. Это тот путь, которого ждут
`tools.client_mcp.extension.artifact.path` и preflight `build`, поэтому для
подготовки клиентского MCP вызов без `sources` артефакт не собирает:
готовый артефакт должен уже существовать по настроенному пути:

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "tools-download",
      "tool": "client-mcp",
      "dryRun": true
    }
  }
}
```

Исходники нужны только когда расширение правится. `sources: true` не добавляет
их к артефакту, а заменяет его: runner переключается в режим `sources`, кладёт
дерево EDT в `build/tools/onec-client-mcp-devkit/exts/client-mcp`, `.cfe` при
этом не создаётся, и собрать дерево можно только установленным `1cedtcli`. Если
`1cedtcli` в системе нет, этот маршрут тупиковый — используй уже существующий
готовый артефакт.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "tools-download",
      "tool": "client-mcp",
      "sources": true,
      "force": true,
      "dryRun": true
    }
  }
}
```

## Launch

Все режимы launch исполняются применённо и несут названный риск:
`runtime_risk_unproven_process_ownership` с `waitForExit=true` и
`runtime_risk_detached_child` без него. Даже `waitForExit=true` не
доказывает владение отдельно сгруппированным процессом 1С на каждом аварийном
пути закреплённого runner-а.

### Предпросмотр Designer

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "launch",
      "clientMode": "designer",
      "dryRun": true
    }
  }
}
```

### Предпросмотр thin client

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "launch",
      "clientMode": "thin",
      "dryRun": true
    }
  }
}
```

### Дождаться завершения внешней EPF, передав команду в `/C`

Для preview bounded-запуска локальной внешней обработки выбери
`clientMode=thin` и явно задай разные файлы: `output` — платформенный `/Out`, а
`stderrOutput` — stderr клиентского процесса 1С. Если обработке нужна команда
запуска, передавай содержимое платформенного `/C` через типизированное поле `c`,
не через `rawKeys`.

Ниже показан preview bounded-запуска Vanessa Automation с профилем
`VAParams.json`. Если задан
`tools.va.epf_path`, подставь его значение вместо пути из примера:

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "launch",
      "clientMode": "thin",
      "execute": "build/tools/vanessa-automation-single.epf",
      "c": "StartFeaturePlayer;VAParams=tools/VAParams.json",
      "rawKeys": ["/TESTMANAGER"],
      "output": "build/va.platform-out.log",
      "stderrOutput": "build/va.client.stderr.log",
      "waitForExit": true,
      "waitTimeoutMs": 30000,
      "dryRun": true
    }
  }
}
```

Любой применённый launch исполняется и несёт названный риск. Поля `waitForExit`,
`waitTimeoutMs`, `output` и `stderrOutput` можно проверить в preview, но
terminal receipt реального EPF не обещается до появления доказанного
ownership-контракта runner-а.
Не обходи названный риск через `unica.runtime.job.start`: долговременное
задание — отдельная поверхность для работы, которую вызов ждать не должен.
Поле `c` runner преобразует в единственный ключ `/C`.
Дополнительные нерезервированные ключи, например `/TESTMANAGER`, можно передать
через `rawKeys`; не дублируй там `/C`, `/Execute` или `/Out`.

### Предпросмотр Client MCP без VA

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "launch",
      "clientMode": "mcp",
      "mode": "thin",
      "mcpPort": 1550,
      "dryRun": true
    }
  }
}
```

### Предпросмотр Client MCP с Vanessa Automation

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "launch",
      "clientMode": "mcp-va",
      "mode": "thin",
      "mcpConfig": "tools/client-mcp.json",
      "dryRun": true
    }
  }
}
```

## References

- `references/command-selection.md` — карта intent -> MCP arguments.
- `references/project-workflows.md` — workspace, build, syntax, extensions, launch.
- `references/config-and-backends.md` — `v8project.yaml`, `v8project.local.yaml`, source-set и backend constraints.
- `references/file-and-artifact-workflows.md` — dump/convert/load/make.
- `references/testing.md` — YaXUnit, Vanessa Automation, syntax validation.
- `references/troubleshooting.md` — безопасная диагностика без обхода лицензий и auth.
