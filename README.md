# Unica

Unica (Ю&#x301;ника) — публичный плагин [Codex](https://openai.com/codex/) для разработки на 1С:Предприятии. Он добавляет
навыки и один MCP-сервер `unica`, через который Codex создаёт и проверяет
метаданные, формы, роли, СКД, внешние обработки и отчёты, запускает 1С и ищет BSL-код.

## Требования

- актуальный [Codex CLI](https://learn.chatgpt.com/docs/codex/cli) с командами `codex plugin`;
- стандартный Git, включая Git for Windows на Windows;
- платформа 1С только для операций, которым реально требуется запуск 1С.

### Поддерживаемые версии платформы 1С

| Версия платформы | Статус | Что это означает |
| --- | --- | --- |
| `8.3.27.x` | Поддерживается | Unica поддерживает все актуальные релизы ветки 8.3.27. |
| `8.5.1.x`, `8.5.4.x` | Планируется | Хотим добавить в ближайшее время. |
| `8.3.26.x` и ниже | Не планируется | Помогаем мигрировать на 8.3.27. Если вам действительно нужна более старая версия, [создайте issue](https://github.com/IngvarConsulting/unica/issues/new) и опишите причину — нам важно понимать такой сценарий. |

## Установка

```sh
codex plugin marketplace add IngvarConsulting/unica-marketplace --ref main
codex plugin add unica@unica
```

При первом MCP-вызове `unica` скачивает только исполнительные файлы текущей платформы из
релиза `IngvarConsulting/unica`. Архив и каждый файл проверяются по SHA-256.

## Обновление

```sh
codex plugin marketplace upgrade unica
codex plugin remove unica@unica
codex plugin add unica@unica
```

Затем откройте Codex. 

Отдельной команды `codex plugin upgrade` в поддерживаемом CLI нет, поэтому переустановка плагина после обновления каталога
является намеренным шагом.

## Переход со старых версий

Узнайте свою версию:

```sh
codex plugin list
```

В строке `unica@...` смотрите столбец `VERSION`.

| Ваша версия | Что делать |
| --- | --- |
| `0.3.0`–`0.7.4` | Запустите скрипт миграции |
| `0.7.5` и новее | Выполните обычное обновление |

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

Если скрипт завершился ошибкой, предыдущая установка уже будет восстановлена.

Начиная с `v0.8.0`, текущий пакет не содержит исполняемого кода старых-миграций:
для старых версий поддерживается только переход через замороженную версию `v0.7.8`.

## Удаление

```sh
codex plugin remove unica@unica
codex plugin marketplace remove unica
```

Проверенные исполняемые-кэши можно оставить для повторной установки. Их ручное
удаление не является частью обычного процесса удаления.

## Разработка

Для разработки используется отдельный marketplace `unica-dev`:

```sh
git clone https://github.com/IngvarConsulting/unica.git
cd unica
scripts/dev/install-local-unica.sh
```

На Windows x64 запускайте этот скрипт из **Git Bash**, входящего в 64-битный
Git for Windows. Для локальной сборки нужны Python 3.10 или новее, стабильный
Rust с нативным toolchain MSVC, а также Microsoft C++ Build Tools и Windows SDK.

WSL сохраняет Linux-семантику и собирает `linux-x64`. MSYS2 и Cygwin не входят
в поддерживаемые shell для этого installer; используйте Git Bash.

Исходный `.mcp.json` запускает `cargo run`; локальный скрипт собирает инструменты
только для текущей машины. Официальный пакет остаётся тонким: skills, assets,
три bootstrap-бинарника и `runtime-manifest.json`, без полного runtime.

## Репозиторий

- `plugins/unica/skills/` — прикладные навыки 1С;
- `crates/unica-coder/` — единый MCP runtime `unica`;
- `crates/unica-bootstrap/` — загрузка, проверка и запуск runtime;
- `plugins/unica/third-party/tools.lock.json` — версии внутренних инструм

[Авторы, источники и лицензии](plugins/unica/ATTRIBUTIONS.md).
Лицензия Unica: LGPL-3.0-or-later.
