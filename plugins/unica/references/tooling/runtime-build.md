# Пакетный режим конфигуратора 1С

## Общие сведения

Конфигуратор 1С:Предприятия 8.3 поддерживает пакетный (безоконный) режим для
автоматизации операций с конфигурациями, информационными базами и внешними
обработками. Команды `1cv8.exe` ниже документируют синтаксис платформы, но не
являются разрешённым пользовательским workflow Unica и не должны запускаться
напрямую как обход публичной границы.

По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `unica.runtime.execute` с `dryRun: true`
показывает запланированную команду без побочных эффектов, а с `dryRun: false`
исполняет классифицированную операцию и отвечает её терминальным результатом в
том же вызове, приложив названную причину риска (`runtime_risk_*`)
предупреждением; неклассифицированная операция по-прежнему отказывает
`runtime_operation_unbounded` до обнаружения рабочего пространства. Preview
исполнением не является. Работу, которую вызов ждать не должен, запускай через
`unica.runtime.job.start`. Не обходи контракт прямым runner-ом или через
`unica.build.*`.

**Два режима запуска:**

| Режим | Назначение |
|-------|-----------|
| `DESIGNER` | Конфигуратор — работа с конфигурацией, сборка EPF, обновление БД |
| `ENTERPRISE` | Предприятие — запуск обработок, навигация по ссылкам |
| `CREATEINFOBASE` | Создание новой информационной базы |

**Путь к 1cv8.exe** зависит от версии платформы: `C:\Program Files\1cv8\8.3.27.1859\bin\1cv8.exe`.

## Подключение к информационной базе

| Параметр | Описание |
|----------|----------|
| `/F <каталог>` | Файловая база — каталог с файлом `1Cv8.1CD` |
| `/S <адрес>` | Серверная база — формат `server/ibname` |
| `/IBName <имя>` | По имени из списка баз (в кавычках если содержит пробелы) |
| `/IBConnectionString` | Полная строка соединения |

Примеры:
```
1cv8.exe DESIGNER /F "C:\Bases\MyBase" ...
1cv8.exe DESIGNER /S server-pc/accounting ...
1cv8.exe DESIGNER /IBName "Бухгалтерия предприятия" ...
```

### Аутентификация

| Параметр | Описание |
|----------|----------|
| `/N<имя>` | Имя пользователя (**без пробела** после `/N`) |
| `/P<пароль>` | Пароль (**без пробела** после `/P`). Можно опустить если пароля нет |
| `/WA-` | Запретить аутентификацию ОС |
| `/WA+` | Обязательная аутентификация ОС (по умолчанию) |

> **Важно**: между `/N` и именем, а также между `/P` и паролем пробела нет: `/NАдмин /PSecret123`.

## Общие параметры пакетного режима

| Параметр | Описание |
|----------|----------|
| `/DisableStartupDialogs` | Подавляет интерактивные диалоги. **Обязательно** для пакетного режима — без него конфигуратор может зависнуть в ожидании ввода |
| `/DisableStartupMessages` | Подавляет стартовые предупреждения (несоответствие конфигурации БД и т.п.) |
| `/Out <файл> [-NoTruncate]` | Файл для вывода служебных сообщений (UTF-8). `-NoTruncate` — не очищать файл перед записью |
| `/DumpResult <файл>` | Записать числовой код результата в файл (0 — успех, 1 — ошибка, 101 — ошибки проверки) |
| `/Visible` | Показать окно конфигуратора (по умолчанию скрыто в пакетном режиме) |

## Создание информационной базы

```
1cv8.exe CREATEINFOBASE <строка_соединения> [/AddToList [<имя>]] [/UseTemplate <файл>] [/DumpResult <файл>]
```

### Файловая база

```
1cv8.exe CREATEINFOBASE File="C:\Bases\EmptyDB"
```

### Серверная база

```
1cv8.exe CREATEINFOBASE Srvr="server-pc";Ref="new_db"
```

### Параметры

| Параметр | Описание |
|----------|----------|
| `File="<путь>"` | Строка соединения для файловой базы |
| `Srvr="<сервер>";Ref="<имя>"` | Строка соединения для серверной базы |
| `/AddToList [<имя>]` | Добавить в список баз. Имя — необязательно |
| `/UseTemplate <файл>` | Создать по шаблону (.cf или .dt) |
| `/DumpResult <файл>` | Записать результат (0 — успех) |

## Работа с конфигурацией — бинарные файлы (CF)

### Выгрузка конфигурации в CF-файл

```
1cv8.exe DESIGNER /F <база> /DisableStartupDialogs /DumpCfg config.cf /Out log.txt
```

**`/DumpCfg <файл> [-Extension <имя>]`** — сохранить конфигурацию в .cf-файл.

### Загрузка конфигурации из CF-файла

```
1cv8.exe DESIGNER /F <база> /DisableStartupDialogs /LoadCfg config.cf /Out log.txt
```

**`/LoadCfg <файл> [-Extension <имя>] [-AllExtensions]`** — загрузить конфигурацию из .cf-файла.

| Параметр | Описание |
|----------|----------|
| `-Extension <имя>` | Работа с расширением (указать имя) |
| `-AllExtensions` | Работа со всеми расширениями (файл — архив расширений) |

> После `/LoadCfg` конфигурация загружается в «основную» конфигурацию конфигуратора. Для применения к БД необходим `/UpdateDBCfg`.

## Работа с конфигурацией — XML-исходники

### Выгрузка `/DumpConfigToFiles`

```
1cv8.exe DESIGNER /F <база> /DisableStartupDialogs /DumpConfigToFiles <каталог> [параметры] /Out log.txt
```

Полная сигнатура:
```
/DumpConfigToFiles <каталог> [-Extension <имя>] [-AllExtensions]
    [-update] [-force] [-getChanges <файл>]
    [-configDumpInfoForChanges <файл>] [-listFile <файл>]
    [-configDumpInfoOnly] [-Server] [-Format <формат>]
    [-Archive <файл>] [-ignoreUnresolvedReferences]
```

#### Режимы выгрузки

> Ниже приведён низкоуровневый синтаксис платформы, а не рекомендуемый путь
> Unica. Не направляй incremental/partial/CDFI-only команды прямо в
> Git-visible source root: они допустимы только во временный private staging,
> принадлежащий runtime-слою. Синхронный full dump (`mode=full`) для DESIGNER
> `CONFIGURATION`/`EXTENSION` исполняется с названным риском записи без ограниченного восстановления: его applied path проходит через внешний private stage Unica,
> платформа независимо фиксируется на exact 8.3.27.x, XML проверяется на raw
> `version="2.20"` до целой публикации. На Windows, macOS и Linux verified
> transactional publication определяет этот synchronous full dump, но
> постпроцессинг не имеет доказанного terminal-receipt bound. Владельцем контракта публикации остаётся ADR-0016;
> `INV-SOURCE-BOUND-PREIMAGES` и `INV-SOURCE-ROLLBACK-VISIBLE` описывают
> проверяемую транзакцию, а OS-зависимая реализация остаётся за
> `INV-PLATFORM-OS-BEHIND-FACADE`.
>
> Async full и applied external source-set несут тот же риск публикации. Неполные режимы
> дополнительно не имеют безопасного merge receipt. На Windows Unica через
> no-follow handles проверяет локальную системную установку: trusted owner и DACL
> защищают install tree от изменения запускающим non-elevated пользователем, а
> ancestry — от удаления, замены и перенаправления компонентов пути. На macOS и
> Linux проверяются физические DESIGNER-маркеры, exact sibling
> `ibcmd --version` и root-owned, link-free install tree без group/world write и
> ACL. Secret-bearing effective config отделён от сохраняемого recovery.
> Пользовательская или изменяемая установка отклоняется до `ibcmd` и
> `v8-runner`; прочие Unix fail-closed.

**Полная выгрузка** — все объекты конфигурации:
```
1cv8.exe DESIGNER /F <база> /DisableStartupDialogs /DumpConfigToFiles "C:\src\config" /Out log.txt
```

**Инкрементальная выгрузка** — только изменённые объекты:
```
1cv8.exe DESIGNER /F <база> /DisableStartupDialogs /DumpConfigToFiles "C:\runtime\private-staging\config" -update -force /Out log.txt
```

Инкрементальная выгрузка с отслеживанием изменений:
```
1cv8.exe DESIGNER /F <база> /DisableStartupDialogs /DumpConfigToFiles "C:\runtime\private-staging\config" -update -getChanges "changes.txt" -configDumpInfoForChanges "C:\runtime\ib-state\ConfigDumpInfo.xml" /Out log.txt
```

**Частичная выгрузка** — выбранные объекты по списку:
```
1cv8.exe DESIGNER /F <база> /DisableStartupDialogs /DumpConfigToFiles "C:\runtime\private-staging\config" -listFile "dump_objects.txt" /Out log.txt
```

**Обновление ConfigDumpInfo.xml** — без выгрузки файлов:
```
1cv8.exe DESIGNER /F <база> /DisableStartupDialogs /DumpConfigToFiles "C:\runtime\ib-state" -configDumpInfoOnly /Out log.txt
```

#### Параметры выгрузки

| Параметр | Описание |
|----------|----------|
| `-update` | Обновляющая (инкрементальная) выгрузка — только изменённые объекты |
| `-force` | Принудительная полная выгрузка. Используется с `-update` при несовпадении версий |
| `-getChanges <файл>` | Записать список изменённых файлов |
| `-configDumpInfoForChanges <файл>` | Файл ConfigDumpInfo.xml для определения изменений |
| `-listFile <файл>` | Файл со списком выгружаемых объектов (по одному на строку) |
| `-configDumpInfoOnly` | Выгрузить только ConfigDumpInfo.xml |
| `-Extension <имя>` | Выгрузить расширение |
| `-AllExtensions` | Выгрузить все расширения |
| `-Server` | Выгрузка на стороне сервера |
| `-Format <формат>` | Формат файлов (Hierarchical / Plain) |
| `-Archive <файл>` | Выгрузка в архивный файл |
| `-ignoreUnresolvedReferences` | Игнорировать неразрешённые ссылки |

#### Формат listFile для выгрузки

Файл содержит **имена объектов метаданных** (одно на строку):
```
Справочник.Номенклатура
Справочник.Валюты
Документ.РеализацияТоваровУслуг
Отчет.АнализПродаж
```

Кодировка: UTF-8 с BOM.

### Загрузка `/LoadConfigFromFiles`

```
1cv8.exe DESIGNER /F <база> /DisableStartupDialogs /LoadConfigFromFiles <каталог> [параметры] /Out log.txt
```

Полная сигнатура:
```
/LoadConfigFromFiles <каталог> [-Extension <имя>] [-AllExtensions]
    [-updateConfigDumpInfo] [-listFile <файл>]
    [-Server] [-Archive <файл>] [-Format <формат>]
```

#### Режимы загрузки

**Полная загрузка** — замена всей конфигурации:
```
1cv8.exe DESIGNER /F <база> /DisableStartupDialogs /LoadConfigFromFiles "C:\src\config" /Out log.txt
```

**Частичная загрузка** — выбранные файлы по списку:
```
1cv8.exe DESIGNER /F <база> /DisableStartupDialogs /LoadConfigFromFiles "C:\src\config" -listFile "load_list.txt" -Format Hierarchical -partial -updateConfigDumpInfo /Out log.txt
```

#### Параметры загрузки

| Параметр | Описание |
|----------|----------|
| `-listFile <файл>` | Файл со списком загружаемых файлов (по одному на строку) |
| `-partial` | Частичная загрузка — **не заменять** всю конфигурацию, а внести точечные изменения. Недокументированный, но рабочий параметр |
| `-updateConfigDumpInfo` | Обновить ConfigDumpInfo.xml после загрузки |
| `-Extension <имя>` | Загрузить в расширение |
| `-AllExtensions` | Загрузить все расширения |
| `-Server` | Загрузка на стороне сервера |
| `-Archive <файл>` | Загрузка из архивного файла |
| `-Format <формат>` | Формат файлов (Hierarchical / Plain) |

#### Формат listFile для загрузки

Файл содержит **относительные пути к файлам** в каталоге выгрузки (один на строку):
```
Catalogs/Валюты.xml
Catalogs/Валюты/Ext/ObjectModule.bsl
Documents/РеализацияТоваровУслуг.xml
Documents/РеализацияТоваровУслуг/Forms/ФормаДокумента.xml
```

Кодировка: UTF-8 с BOM.

> **Важно: различие форматов listFile для dump и load:**
> - **Выгрузка** (`/DumpConfigToFiles -listFile`): имена объектов метаданных — `Справочник.Номенклатура`
> - **Загрузка** (`/LoadConfigFromFiles -listFile`): относительные пути файлов — `Catalogs/Валюты.xml`

## Обновление конфигурации БД

```
1cv8.exe DESIGNER /F <база> /DisableStartupDialogs /UpdateDBCfg /Out log.txt
```

Полная сигнатура:
```
/UpdateDBCfg [-Dynamic<режим>] [-Server]
    [-WarningsAsErrors]
    [-BackgroundStart] [-BackgroundFinish]
    [-BackgroundCancel] [-BackgroundSuspend] [-BackgroundResume]
    [-Extension <имя>] [-AllExtensions]
```

| Параметр | Описание |
|----------|----------|
| `-Dynamic+` | Использовать динамическое обновление |
| `-Dynamic-` | Не использовать динамическое обновление |
| `-Server` | Обновление на стороне сервера |
| `-WarningsAsErrors` | Предупреждения считать ошибками |
| `-Extension <имя>` | Обновить расширение |
| `-AllExtensions` | Обновить все расширения |

### Фоновое обновление

| Параметр | Описание |
|----------|----------|
| `-BackgroundStart` | Начать фоновое обновление |
| `-BackgroundFinish` | Дождаться окончания и завершить |
| `-BackgroundCancel` | Отменить фоновое обновление |
| `-BackgroundSuspend` | Приостановить |
| `-BackgroundResume` | Возобновить |

> После `/LoadCfg` или `/LoadConfigFromFiles` необходимо выполнить `/UpdateDBCfg` чтобы изменения применились к базе данных.

## Сборка и разборка внешних обработок (EPF/ERF)

EPF/ERF runtime-аргументы в packaged Unica plugin предпросматриваются через
`v8-runner` и MCP `unica.runtime.execute`. Отдельные EPF/ERF build/dump skills
не являются пользовательским workflow, а текущий preview не собирает и не
выгружает артефакты.

### Preview публикации внешних обработок и отчетов

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "operation": "make",
      "cwd": "<workspace>",
      "sourceSet": "external-processors",
      "output": "build/external",
      "dryRun": true
    }
  }
}
```

Для preview внешних отчетов используй `sourceSet: "external-reports"`.
`output` задаёт предполагаемый каталог будущей публикации; текущий вызов не
создаёт `.epf` или `.erf`.

### Выгрузка внешних исходников

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "operation": "dump",
      "cwd": "<workspace>",
      "sourceSet": "external-processors",
      "mode": "full",
      "dryRun": true
    }
  }
}
```

Для выгрузки внешних отчетов используй `sourceSet: "external-reports"`.
Сейчас это только preview: applied external dump блокируется до появления
такой же проверяемой private-stage публикации, как для configuration/extension.

### Preview загрузки XML-исходников в базу

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "operation": "build",
      "cwd": "<workspace>",
      "sourceSet": "external-processors",
      "mode": "full",
      "dryRun": true
    }
  }
}
```

### Примечания

- `operation=load` моделирует `.cf` и `.cfe`; `.epf` и `.erf` моделируются
  через preview `build` и `make` external source-set. Все эти текущие операции,
  включая `dump`, вызываются только с `dryRun=true`.
- Внешние source-set должны быть объявлены в `v8project.yaml` с типами `EXTERNAL_DATA_PROCESSORS` или `EXTERNAL_REPORTS`.
- Dump требует базу с конфигурацией, содержащей используемые типы. Dump в пустой базе может потерять ссылочные типы (`CatalogRef.XXX` превращается в `xs:string`).
- Категории колонок регистров (Dimension/Resource/Attribute) зависят от Form.xml и конфигурации базы; при round-trip через неподходящую базу привязки полей формы могут не сохраниться.

## Запуск в режиме предприятия

```
1cv8.exe ENTERPRISE /F <база> [/N<имя> /P<пароль>] /DisableStartupDialogs [параметры]
```

| Параметр | Описание |
|----------|----------|
| `/Execute <файл.epf>` | Запуск внешней обработки сразу после старта. При указании `/Execute` параметр `/URL` игнорируется |
| `/URL <ссылка>` | Навигационная ссылка (формат `e1cib/...`) |
| `/C <строка>` | Передача параметра в прикладное решение |

Примеры:
```
1cv8.exe ENTERPRISE /F "C:\Bases\MyBase" /NАдмин /PSecret /DisableStartupDialogs /Execute "C:\scripts\process.epf"
```

```
1cv8.exe ENTERPRISE /IBName "Бухгалтерия" /NАдмин /DisableStartupDialogs /URL "e1cib/data/Справочник.Номенклатура"
```

## Коды возврата

| Код | Значение |
|-----|----------|
| `0` | Успешно |
| `1` | Ошибка |
| `101` | Ошибки при проверке конфигурации |

Числовой код можно записать в файл через `/DumpResult <файл>`.

При работе с расширениями (`-Extension`, `-AllExtensions`): 0 — успех, 1 — ошибка.

## ConfigDumpInfo.xml

`ConfigDumpInfo.xml` — служебный файл, создаваемый при выгрузке конфигурации в файлы (`/DumpConfigToFiles`). Содержит информацию о составе и версиях объектов конфигурации на момент выгрузки.

Platform-generated CDFI sidecar с корнем `<ConfigDumpInfo>` — локальное
runtime-состояние конкретной ИБ, а не коллективный XML-исходник. Не добавляй
этот sidecar в Git и не передавай его в `unica.cf.*` или `unica.meta.*`. Чистый
checkout без него является нормальным состоянием. Unica не считает его
признаком формата source-set и не включает в mutation targets/receipts.
Legitimate metadata descriptor (включая external EPF/ERF) объекта с именем
`ConfigDumpInfo` остаётся исходником и хранится в Git.

**Назначение:**
- Определение изменений при инкрементальной выгрузке (`-update`, `-configDumpInfoForChanges`)
- Синхронизация состояния выгрузки с конфигурацией ИБ

**Использование:**
- `-configDumpInfoForChanges <файл>` — передать предыдущий ConfigDumpInfo.xml для определения изменений
- `-configDumpInfoOnly` — обновить только этот файл без выгрузки объектов
- `-updateConfigDumpInfo` — обновить файл после частичной загрузки (`/LoadConfigFromFiles`)

Платформа предоставляет параметры для использования вспомогательного CDFI при
сравнении, но управление приватным CDFI для пары `source-set + ИБ` относится к
 runtime-слою. На Windows, macOS и Linux синхронный full dump (`mode=full`) для
DESIGNER `CONFIGURATION`/`EXTENSION` исполняется с названным риском; его verified transactional
publication Unica перенаправляет выбранный source-set во внешний private
stage, платформа проверяется как exact 8.3.27.x, а version-bearing XML roots —
как raw `2.20`; только затем целое дерево публикуется с проверкой preimage и
rollback (ADR-0016, `INV-PLATFORM-OS-BEHIND-FACADE`). До реализации private
state и shadow publication в `alkoleft/v8-runner-rust#30`
`mode=incremental|partial` исполняется, но доверять его результату вслепую
нельзя: закреплённый runner не возвращает точные processed paths/hashes и не
выполняет divergence-safe merge, поэтому расхождение обнаруживается только
сравнением исходников после прогона.
Будущий applied-маршрут сможет принять только системную установку платформы,
неизменяемую для вызывающего пользователя; сейчас любой applied-вызов
останавливается ещё до проверки или исполнения установки.

## Выбор платформы 1С

Закреплённый `v8-runner` выбирает платформу по `tools.platform.version` и
`tools.platform.path` в `v8project.yaml` или локальном
`v8project.local.yaml`. Без явного ограничения он использует максимальную
найденную версию. Например, проект может ограничить семейство платформы:

```yaml
tools:
  platform:
    version: "8.3.27"
```

Четырёхкомпонентная версия требует точного совпадения сборки. Путь к установке
зависит от машины, поэтому его следует хранить в `v8project.local.yaml`:

```yaml
tools:
  platform:
    version: "8.3.27.1859"
    path: "C:\\Program Files\\1cv8\\8.3.27.1859\\bin"
```

Полный контракт и правила выбора описаны в `v8project.md`.
