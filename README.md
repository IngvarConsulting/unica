<p align="center">
  <a href="https://ingvar.pro/products/unica">
    <img src="docs/visual-kit/logos/unica-logo-letter-transparent-blue.svg" alt="Unica" width="420">
  </a>
</p>

# Unica

Unica (Ю&#x301;ника) — публичный плагин [Codex](https://openai.com/codex/) и
[Claude Code](https://code.claude.com/docs/en/overview) для разработки на
1С:Предприятии. Он добавляет навыки и один MCP-сервер `unica`, через который
агент создаёт и проверяет метаданные, формы, роли, СКД, внешние обработки и
отчёты, запускает 1С и ищет BSL-код.

Оба хоста получают один и тот же каталог плагина: манифесты лежат рядом, а
`.mcp.json` определяет корень плагина по той переменной, которую подставляет
конкретный хост.

## Требования

- один из агентов:
  - [Codex CLI](https://learn.chatgpt.com/docs/codex/cli);
  - [Claude Code](https://code.claude.com/docs/en/overview);
- стандартный Git, включая Git for Windows на Windows;
- платформа 1С только для операций, которым реально требуется запуск 1С.

### Поддерживаемые версии платформы 1С

| Версия платформы | Статус | Что это означает |
| --- | --- | --- |
| `8.5.1.x`, `8.5.4.x` | Планируется | Хотим добавить в ближайшее время. |
| `8.3.27.x` | Поддерживается | Unica поддерживает все актуальные релизы ветки 8.3.27. |
| `8.3.26.x` и ниже | Не планируется | Помогаем мигрировать на 8.3.27. Если вам действительно нужна более старая версия, [создайте issue](https://github.com/IngvarConsulting/unica/issues/new) и опишите причину — нам важно понимать такой сценарий. |

## Установка

### Codex

```sh
codex plugin marketplace add IngvarConsulting/unica-marketplace --ref main
codex plugin add unica@unica
```

После установки откройте new Codex task: список навыков и MCP-конфигурация
фиксируются на границе новой задачи, а не подменяются в уже работающей сессии.

### Claude Code

```sh
claude plugin marketplace add IngvarConsulting/unica-marketplace
claude plugin install unica@unica
```

Затем выполните `/reload-plugins` либо начните новую сессию. Навыки становятся
доступны с префиксом плагина, например `/unica:meta-info`.

### Загрузка runtime

При первом MCP-вызове `unica` скачивает из релиза `IngvarConsulting/unica`
исполнительные файлы для текущей ОС и архитектуры. Архив и каждый файл
проверяются по SHA-256. Неполная или повреждённая загрузка не получает маркер
готовности.

Готовый runtime атомарно публикуется в кэше хоста:

| Хост | Каталог кэша |
| --- | --- |
| Codex | `$CODEX_HOME/unica/runtimes/<version>/<target>`, при стандартном `CODEX_HOME` — `~/.codex/unica/runtimes/...` |
| Claude Code | `${CLAUDE_PLUGIN_DATA}/runtimes/<version>/<target>`, по умолчанию — `~/.claude/plugins/data/unica-unica/...`; этот каталог переживает обновление плагина |

## Обновление

### Codex

```sh
codex plugin marketplace upgrade unica
codex plugin remove unica@unica
codex plugin add unica@unica
```

Затем откройте new Codex task: уже работающая сессия не загрузит обновлённые
навыки и MCP-конфигурацию.

Отдельной команды `codex plugin upgrade` в поддерживаемом CLI нет, поэтому
переустановка плагина после обновления каталога является намеренным шагом.

### Claude Code

```sh
claude plugin marketplace update unica
claude plugin update unica@unica
```

Затем выполните `/reload-plugins`.

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

В `0.12.0` публичная группа `unica.meta.*` перешла на один просмотр и три
типизированные мутации без совместимости со старыми маршрутами. Таблица замен и
новые контракты собраны в [руководствах по миграции](docs/migrations/README.md).

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

Codex:

```sh
codex plugin remove unica@unica
codex plugin marketplace remove unica
```

Claude Code:

```sh
claude plugin uninstall unica@unica
claude plugin marketplace remove unica
```

Проверенные исполняемые-кэши можно оставить для повторной установки. Их ручное
удаление не является частью обычного процесса удаления.

## Разработка

Для разработки под Codex используется отдельный marketplace `unica-dev`:

```sh
git clone https://github.com/IngvarConsulting/unica.git
cd unica
scripts/dev/install-local-unica.sh
```

Под Claude Code каталог плагина подключается напрямую, без маркетплейса:

```sh
claude --plugin-dir ./plugins/unica
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
- `plugins/unica/third-party/tools.lock.json` — версии внутренних инструментов.

[Авторы, источники и лицензии](plugins/unica/ATTRIBUTIONS.md).
Лицензия Unica: LGPL-3.0-or-later.
