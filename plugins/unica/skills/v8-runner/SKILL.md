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

- Preferred path: use MCP `unica` tool `unica.runtime.execute`; `unica` owns v8-runner execution, workspace events, and cache refresh after successful mutations.
- Do not start internal runner MCP servers or package launchers directly for normal workflows. The runner is an internal adapter behind public MCP `unica`.
- Direct shell runner calls are allowed only for maintainer/debug investigation when MCP itself is broken; do not use them as task examples.
- For mutating operations, pass `dryRun: false` only when the user explicitly requested execution. Default dry run is the safe preview.

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

| Намерение | MCP `operation` | Cache/event после успешного non-dry-run |
|---|---|---|
| Создать `v8project.yaml` | `config-init` | `SourceSetChanged` |
| Инициализировать базу/workspace | `init` | `SourceSetChanged` |
| Загрузить XML/EDT исходники в базу | `build` | `BuildCompleted` |
| Выгрузить базу в исходники | `dump` | `SourceSetChanged` |
| Конвертировать Designer/EDT sources | `convert` | `SourceSetChanged` |
| Собрать CF/CFE/EPF/ERF артефакт | `make` | без invalidation |
| Загрузить CF/CFE артефакт | `load` | `BuildCompleted` |
| Проверить синтаксис | `syntax` | без invalidation |
| Запустить тесты | `test` | `BuildCompleted` |
| Запустить клиент/Designer/MCP-клиент | `launch` | без invalidation |
| Синхронизировать extension properties | `extensions` | `BuildCompleted` |
| Скачать/обновить runner tools | `tools-download` | без invalidation |

## Auth/license stop rules

- Если вывод операции похож на проблему лицензии 1С (`лиценз`, `license`, `HASP`, `nethasp`, `LM`, `No license`, `Лицензия не найдена`), остановись. Не лечи лицензию, не меняй службы, реестр, `nethasp.ini` или программную лицензию.
- Если база без указанного пользователя/пароля, допускается только два предположения: `Администратор` без пароля, затем `Admin` без пароля. Если оба не подходят, спроси пользователя.
- Не сохраняй пароль в `v8project.yaml` молча. Если credentials нужно записать в connection string, предупреди пользователя и не коммить такой файл.
- Если `tools-download` падает на `failed to fetch latest release … 403`, это анонимный лимит GitHub API, а не ошибка проекта и не отказ 1С. Не повторяй вызов по кругу: назови пользователю причину прямо. Аутентифицировать запрос runner не умеет — переменной с токеном он не читает. Выходы: подождать сброса лимита; направить `V8TR_GITHUB_API_BASE_URL` на зеркало или прокси, которое добавит авторизацию (Unica пробрасывает окружение в runner, поэтому переменная доходит); либо положить готовый артефакт по настроенному пути вручную — для client MCP это `tools.client_mcp.extension.artifact.path`.

## Workspace init

Для пустого репозитория сначала создай `src/`, затем `v8project.yaml`, затем реши источник правды.

Если исходники отсутствуют или `src/` пустой, считай существующую базу
источником правды и выполни синхронный полный `dump`. Для source-set типа
`CONFIGURATION` или `EXTENSION` Unica принудительно выбирает проверенную
платформу 8.3.27, направляет runner в private staging и публикует дерево только
после проверки exact 2.20. Асинхронный и external-source-set dump пока
preview-only. Если исходники уже есть, не выполняй `build` автоматически:
спроси, база или Git является источником правды.

### Новый `v8project.yaml`

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
      "dryRun": false
    }
  }
}
```

### Первичная инициализация runtime state

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "init",
      "dryRun": false
    }
  }
}
```

### Первичная выгрузка в `src/`

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
      "dryRun": false
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
      "dryRun": false
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
      "dryRun": false
    }
  }
}
```

### Локальный overlay

Используй `v8project.local.yaml` для локальных `workPath`, `infobase.connection`, credentials, `tools`, `tests` и `mcp`. Не передавай local overlay как `config`. Не добавляй туда `source-set`, `format`, `builder` или `execution_timeout`: эти поля должны жить в основном проектном конфиге.

Для долгих операций меняй `execution_timeout` в `v8project.yaml` (миллисекунды, default `300000`, диапазон `1..=86400000`). Не прокидывай отдельный `timeoutMs` в `unica.runtime.execute`: Unica не владеет таймаутом runner-а.

Если ignored EPF workspace уже содержит основной `v8project.yaml` только с
`EXTERNAL_DATA_PROCESSORS`, привяжи его к личной локальной ИБ через
`config-init` с явными `config`, `sourceSet` и `connection`. Unica проверит
выбранный source-set и создаст рядом только `v8project.local.yaml`; runner не
запускается, а основной конфиг не меняется. В этом режиме не передавай
`format`, `builder` или `force`, и не перезаписывай существующий local overlay.

## Build/load/artifacts

### Обычный build

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "build",
      "dryRun": false
    }
  }
}
```

Для `build` без `fullRebuild: true` Unica сначала запускает обычную сборку и не
читает состояние поддержки Platform XML. Только корректный структурированный
результат v8-runner после внешнего кода `4`, показывающий завершившийся partial load,
вызывает ровно одну полную повторную попытку. Это временный support-independent
fallback для `unica.runtime.execute` и `unica.runtime.job.start`: он фиксирует
факт сбоя partial load, но не определяет причину ошибки и не утверждает, что она
связана с поддержкой поставщика.

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

### Build одного source-set

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
      "dryRun": false
    }
  }
}
```

### Полная пересборка после branch switch/rebase

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
      "dryRun": false
    }
  }
}
```

### Загрузка CF/CFE

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
      "dryRun": false
    }
  }
}
```

### Загрузка с merge settings

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
      "dryRun": false
    }
  }
}
```

### Загрузка расширения

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
      "dryRun": false
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
На Windows, macOS и Linux verified transactional publication поддерживает
синхронный applied full dump (`mode=full`) только для DESIGNER source-set типа
`CONFIGURATION` или `EXTENSION`. Unica независимо проверяет установленную
платформу 8.3.27, подменяет выбранный target на private staging, проверяет
владельца и все XML version-bearing roots на exact raw `2.20`, затем атомарно с
rollback публикует целое дерево. Контракт публикации принадлежит ADR-0016:
привязку preimage и обязательный видимый отказ rollback уточняют
`INV-SOURCE-BOUND-PREIMAGES` и `INV-SOURCE-ROLLBACK-VISIBLE`, а OS-зависимая
реализация остаётся за `INV-PLATFORM-OS-BEHIND-FACADE`.

Асинхронный full dump и applied dump для external source-set пока доступны
только как preview. `incremental` и `partial` также preview-only: до private
CDFI, точного receipt и divergence-safe merge (alkoleft/v8-runner-rust#30) им
нельзя писать в Git-visible root.

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
      "dryRun": false
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

### Экспорт CF/CFE/EPF/ERF

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
      "dryRun": false
    }
  }
}
```

### Публикация внешних обработок EPF

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
      "dryRun": false
    }
  }
}
```

### Публикация внешних отчётов ERF

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
      "dryRun": false
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

### Загрузка external source-set в базу

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
      "dryRun": false
    }
  }
}
```

## Syntax/tests/extensions

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
      "dryRun": false
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
      "dryRun": false
    }
  }
}
```

### YaXUnit all

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
      "dryRun": false
    }
  }
}
```

### YaXUnit module

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
      "dryRun": false
    }
  }
}
```

### Vanessa Automation

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
      "dryRun": false
    }
  }
}
```

### Extension properties

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
      "dryRun": false
    }
  }
}
```

### Несколько extension source-set

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
      "dryRun": false
    }
  }
}
```

## Tools

### Download Vanessa Automation

Если Vanessa Automation ещё не подготовлена в workspace, сначала скачай
управляемый v8-runner артефакт:

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
      "dryRun": false
    }
  }
}
```

При стандартной конфигурации runner сохраняет EPF как
`build/tools/vanessa-automation-single.epf`. Если effective project config
переопределяет `tools.va.epf_path`, используй в `execute` именно это значение.

### Download client MCP extension

По умолчанию runner берёт готовый артефакт релиза и кладёт его в
`build/tools/client_mcp.cfe`. Это тот путь, которого ждут
`tools.client_mcp.extension.artifact.path` и preflight `build`, поэтому для
подготовки клиентского MCP используй вызов без `sources`:

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
      "dryRun": false
    }
  }
}
```

Исходники нужны только когда расширение правится. `sources: true` не добавляет
их к артефакту, а заменяет его: runner переключается в режим `sources`, кладёт
дерево EDT в `build/tools/onec-client-mcp-devkit/exts/client-mcp`, `.cfe` при
этом не создаётся, и собрать дерево можно только установленным `1cedtcli`. Если
`1cedtcli` в системе нет, этот маршрут тупиковый — оставайся на вызове выше.

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
      "dryRun": false
    }
  }
}
```

## Launch

### Designer

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
      "dryRun": false
    }
  }
}
```

### Thin client

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
      "dryRun": false
    }
  }
}
```

### Дождаться завершения внешней EPF, передав команду в `/C`

Для bounded-запуска локальной внешней обработки используй только прямой thin
client и явно задай разные файлы: `output` — платформенный `/Out`, а
`stderrOutput` — stderr клиентского процесса 1С. Если обработке нужна команда
запуска, передавай содержимое платформенного `/C` через типизированное поле `c`,
не через `rawKeys`.

Ниже bounded-запуск Vanessa Automation с профилем `VAParams.json` использует
стандартный managed path после `tools-download`. Если задан
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
      "dryRun": false
    }
  }
}
```

`waitForExit` не меняет обычный асинхронный launch по умолчанию. В bounded-режиме
Unica возвращает код завершения EPF как успех/ошибку и включает оба объявленных
файла в `artifacts`. Наблюдаемый receipt доступен в
`data.external_epf_wait` и `diagnostics.external_epf_wait`: там есть `pid`,
`execute_path`, `exit_code`, `timed_out`, `output_path` и `stderr_path`.
Этот режим доступен только через `unica.runtime.execute`;
`unica.runtime.job.start` не принимает bounded-поля.
Поле `c` runner преобразует в единственный ключ `/C`.
Дополнительные нерезервированные ключи, например `/TESTMANAGER`, можно передать
через `rawKeys`; не дублируй там `/C`, `/Execute` или `/Out`.

### Client MCP без VA

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
      "dryRun": false
    }
  }
}
```

### Client MCP с Vanessa Automation

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
      "dryRun": false
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
