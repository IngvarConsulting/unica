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

## Workspace init

Для пустого репозитория сначала создай `src/`, затем `v8project.yaml`, затем реши источник правды.

Если исходники отсутствуют или `src/` пустой, считай существующую базу источником правды и сделай полный `dump`. Если исходники уже есть, не выполняй `build` автоматически: спроси, база или Git является источником правды.

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

## Build/load/artifacts

### Обычный build

После успешного `unica.meta.edit` или `unica.code.patch` Unica хранит вне
исходников raw-byte manifest затронутых XML/BSL-файлов. Обычный build остаётся
обычным: Unica не подменяет его скрытым `fullRebuild`, а сверяет terminal JSON
runner-а и помечает синхронизированными только неизменившиеся файлы из реально
успешных, не пропущенных source-set. В `details` смотри массивы `requested`,
`processed`, `skipped` и `conflicted`; skipped/conflicted target остаётся dirty.
Dry-run читает durable state без lock/state/shadow writes и возвращает те же
terminal-массивы; `dryRunRequiresShadow` означает, что версию ИБ можно узнать
только реальным shadow dump.

Единый durable authority находится в каноническом workspace по пути
`.build/unica/source-sync/<workspace-id>` и не зависит от `UNICA_CACHE_DIR`.
Повреждённый state, symlink вместо state/storage или несовпадающая identity
блокируют mutation/build/dump; такое состояние не считается пустым.

При активном source-sync state Unix runtime наследует lifecycle lease в
запущенный runner/Designer: внезапная смерть клиента Unica не разрешает второму
процессу начать конкурентную mutation/build/dump, пока потомок не завершится.
На платформах без такой наследуемой lease runtime build/dump fail-closed; не
обходи это ручным удалением `.build/unica/source-sync`.

Tracked mutation разрешена только для уникального `platform_xml` configuration
entry из непустого списка `source-set` основного `v8project.yaml`. Каждый entry
обязан иметь безопасное уникальное `name`, `type`, `path` и уникальный canonical
root. Autodetect, mapping-form source-set, symlink root и дубликаты не считаются
runtime topology. При active source-sync state build с другим `config` или после
переноса/удаления source-set блокируется как `sourceTopologyChanged` до запуска
runner-а.

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
      "dryRun": false
    }
  }
}
```

### Partial dump объекта

Для tracked target в platform-XML configuration source-set partial dump
выполняется сначала в изолированный shadow source. Unica сравнивает результат
базы с последним успешно загруженным/выгруженным raw-byte manifest и текущими
исходниками до любой публикации. При divergence обычный вызов завершается
конфликтом и не меняет ни одного байта рабочего source.

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
      "dryRun": false
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
      "dryRun": false
    }
  }
}
```

`object` и `objects` объединяются, приводятся к форме `TYPE:NAME` и
дедуплицируются с сохранением первого порядка. Один конфликт блокирует
публикацию всей группы. Публичный контракт всегда использует `TYPE:NAME`;
только изолированный вызов Designer получает внутренний селектор `TYPE.NAME`,
который не возвращается вызывающему коду и не меняет identity target-а.

### Явная публикация версии из базы

`force` — политика оболочки Unica только для `dump/partial`; этот флаг никогда
не передаётся в v8-runner. Используй его лишь после проверки `conflicted`, когда
версия объекта из базы действительно должна заменить локальный tracked target:

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
      "force": true,
      "dryRun": false
    }
  }
}
```

Защита первого среза относится к tracked metadata/module target в
platform-XML configuration source-set. EDT, extensions, external source-set и
full/incremental dump не выдают ложную гарантию object-level round-trip. При
наличии любого source-sync record Unica блокирует full/incremental dump: сначала заверши
tracked partial round-trip либо явно разреши нужную object-level публикацию
через `force`. Не обходи state через legacy `unica.build.load/update/dump`:
используй typed `unica.runtime.execute`.

Успешный forced partial dump публикует проверенные object/module bytes и
созданный платформой `ConfigDumpInfo.xml` одной rollback-транзакцией. CDFI не
входит в dirty manifests и Unica не синтезирует его вручную. В terminal details
сверяй `observedShadow` и, когда bytes реально опубликованы,
`publishedManifest`; `force` не попадает в argv runner-а.

Если операция сообщает retained publication journal/recovery required, не
правь source и state вручную: следующий locked mutation/build/dump сначала
выполнит rollback recovery. При `sourceTopologyChanged` восстанови исходный
уникальный source-set/root в основном `v8project.yaml`, затем заверши обычный
typed build или target-level partial dump. Не удаляй `.build/unica/source-sync`
как способ «сброса»: это уничтожит synchronized baseline и снимет защиту.

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
      "sourceSet": "main",
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
      "dryRun": false
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
      "dryRun": false
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

### Download client MCP sources

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
