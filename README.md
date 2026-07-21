# Unica

Unica — публичный плагин Codex для разработки на 1С:Предприятии. Он добавляет
навыки и один MCP-сервер `unica`, через который Codex создаёт и проверяет
метаданные, формы, роли, СКД, внешние обработки и отчёты, запускает 1С и ищет
BSL-код.

## Требования

- актуальный Codex CLI с командами `codex plugin`;
- стандартный Git, включая Git for Windows на Windows;
- платформа 1С только для операций, которым реально требуется запуск 1С.

Node.js, Python, `curl`, `wget`, `jq` и архиваторы для обычной установки и
запуска не нужны. Git является частью runtime-контракта: Codex запускает
command-scoped Git shell alias, а тот выбирает небольшой нативный bootstrap для
Windows x64, macOS arm64 или Linux x64.

## Установка

```sh
codex plugin marketplace add IngvarConsulting/unica-marketplace --ref main
codex plugin add unica@unica
```

После установки откройте new Codex task: список навыков и MCP-конфигурация
фиксируются на границе новой задачи, а не подменяются в уже работающей сессии.

При первом MCP-вызове bootstrap скачивает только runtime текущей платформы из
релиза `IngvarConsulting/unica`. Архив и каждый файл проверяются по SHA-256.
Готовый runtime атомарно публикуется в
`$CODEX_HOME/unica/runtimes/<version>/<target>`; при стандартном `CODEX_HOME`
это `~/.codex/unica/runtimes/...`. Неполная или повреждённая загрузка не получает
маркер готовности.

## Обновление

```sh
codex plugin marketplace upgrade unica
codex plugin remove unica@unica
codex plugin add unica@unica
```

Затем откройте new Codex task. Отдельной команды `codex plugin upgrade` в
поддерживаемом CLI нет, поэтому переустановка плагина после обновления каталога
является намеренным шагом.

## Переход со старой установки и откат

Узнайте свою версию:

```sh
codex plugin list
```

В строке `unica@...` смотрите столбец `VERSION`.

| Ваша версия | Что делать |
| --- | --- |
| `0.3.0`–`0.7.4` | Запустите скрипт миграции ниже. |
| `0.7.5` и новее | Выполните обычное обновление ниже. |

Для версий `0.3.0`–`0.7.4` на macOS и Linux:

```sh
curl -fLO https://github.com/IngvarConsulting/unica/releases/download/v0.7.8/install-unica.sh
sh install-unica.sh --ref v0.7.8
```

Для версий `0.3.0`–`0.7.4` в Windows PowerShell:

```powershell
Invoke-WebRequest https://github.com/IngvarConsulting/unica/releases/download/v0.7.8/install-unica.ps1 -OutFile install-unica.ps1
.\install-unica.ps1 -Ref v0.7.8
```

Для версий `0.7.5` и новее:

```sh
codex plugin marketplace upgrade unica
codex plugin remove unica@unica
codex plugin add unica@unica
```

Если скрипт завершился ошибкой, предыдущая установка уже восстановлена.

Начиная с `v0.8.0`, текущий пакет не содержит исполняемого кода legacy-миграции:
для старых установок поддерживается только замороженный bridge `v0.7.8` выше.

## Удаление

```sh
codex plugin remove unica@unica
codex plugin marketplace remove unica
```

Проверенные runtime-кэши можно оставить для повторной установки. Их ручное
удаление не является частью обычного uninstall.

## Разработка

Source checkout не является пользовательским пакетом: в нём нет готовых
runtime-бинарников. Для разработки используется отдельный marketplace
`unica-dev`:

```sh
git clone https://github.com/IngvarConsulting/unica.git
cd unica
scripts/dev/install-local-unica.sh
```

На Windows x64 запускайте этот скрипт из **Git Bash**, входящего в 64-битный
Git for Windows. Для локальной сборки нужны Python 3.10 или новее, стабильный
Rust с нативным toolchain MSVC, а также Microsoft C++ Build Tools и Windows SDK.
Для установки и проверки видимости плагина нужен актуальный Codex CLI.

WSL сохраняет Linux-семантику и собирает `linux-x64`. MSYS2 и Cygwin не входят
в поддерживаемые shell для этого installer; используйте Git Bash.

Исходный `.mcp.json` запускает `cargo run`; локальный скрипт собирает инструменты
только для текущей машины. Официальный пакет остаётся тонким: skills, assets,
три bootstrap-бинарника и `runtime-manifest.json`, без полного runtime.

## Репозиторий

- `plugins/unica/skills/` — прикладные навыки 1С;
- `crates/unica-coder/` — единый MCP runtime `unica`;
- `crates/unica-bootstrap/` — загрузка, проверка и запуск runtime;
- `plugins/unica/third-party/tools.lock.json` — версии внутренних инструментов;
- `.github/workflows/unica-plugin-release.yml` — runtime-релиз;
- `.github/workflows/publish-unica-marketplace.yml` — staging и promotion
  публичного каталога.

[Авторы, источники и лицензии](plugins/unica/ATTRIBUTIONS.md).
Лицензия Unica: LGPL-3.0-or-later.
