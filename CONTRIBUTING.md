# Разработка Unica

Спасибо за интерес к проекту. Перед изменениями прочитайте [правила для
агентов](AGENTS.md) и относящиеся к задаче записи из [архитектурного
реестра](arch/index.md). Этот документ описывает рекомендуемое локальное
окружение; команды сборки и проверки приведены в разделе
[«Разработка»](README.md#разработка) и в [README плагина](plugins/unica/README.md#verification).

## Рекомендуемое окружение

### Python 3.12

Минимальная поддерживаемая версия для локальной разработки и сборки —
[Python 3.12](https://www.python.org/downloads/). Используйте ту же ветку
интерпретатора, на которой выполняются команды проверки проекта.

После установки убедитесь, что интерпретатор доступен из shell, в котором
работает агент:

```sh
python3.12 --version
python3.12 -m pip --version
```

В Windows PowerShell используйте `py -3.12`; в Git Bash допустима команда
`python`, если она выводит версию 3.12. Локальный installer на Windows
запускается из Git Bash, как описано в [основном README](README.md#разработка).

### Rust

Установите стабильный Rust через [`rustup`](https://rust-lang.org/tools/install/)
и добавьте компоненты, используемые при разработке:

```sh
rustup toolchain install stable --profile default
rustup default stable
rustup component add rustfmt clippy rust-analyzer rust-src
```

На Windows дополнительно нужны Microsoft C++ Build Tools с MSVC toolchain и
Windows SDK. После установки проверьте окружение:

```sh
rustc --version
cargo --version
rustfmt --version
cargo clippy --version
rust-analyzer --version
```

## `rust-analyzer` для кодового агента

Установите [`rust-analyzer`](https://rust-analyzer.github.io/book/rust_analyzer_binary.html)
как компонент текущего Rust toolchain. Одной установки бинарника недостаточно:
процесс агента должен видеть
`rust-analyzer` в своём `PATH`. `rustup` обычно устанавливает инструменты в
`~/.cargo/bin` на macOS и Linux и в `%USERPROFILE%\.cargo\bin` на Windows.
После изменения `PATH` перезапустите агент и проверьте `rust-analyzer --version`
из его shell.

### Codex

Запускайте Codex из окружения, в котором команда `rust-analyzer` уже доступна,
и после установки или изменения `PATH` начните новую задачу. Проверка версии
доказывает доступность бинарника агенту, но сама по себе не доказывает, что
клиент установил LSP-сессию. Поэтому обязательными проверками Rust-кода остаются
`cargo fmt`, `cargo clippy` и `cargo test`.

### Claude Code

Официальный LSP-плагин настраивает Claude Code для подключения к
`rust-analyzer`. Установите бинарник командой
`rustup component add rust-analyzer`, затем выполните в Claude Code:

```text
/plugin install rust-analyzer-lsp@claude-plugins-official
/reload-plugins
```

Откройте `/plugin` и убедитесь, что у `rust-analyzer-lsp` нет ошибки
`Executable not found in $PATH`. Плагин настраивает LSP-подключение, но не
поставляет сам бинарник `rust-analyzer`.

## Навыки MCP Server Dev

До проектирования или реализации MCP-сервера установите все три навыка из
официального комплекта Anthropic
[`mcp-server-dev`](https://github.com/anthropics/claude-plugins-official/tree/main/plugins/mcp-server-dev):

- `build-mcp-server`;
- `build-mcp-app`;
- `build-mcpb`.

### Codex

Сначала проверьте, какие навыки уже установлены. Ожидаемый путь каждого навыка —
`$CODEX_HOME/skills/<имя>/SKILL.md`; если `CODEX_HOME` не задан —
`~/.codex/skills/<имя>/SKILL.md`. Вызовите `$skill-installer` и попросите
установить из repository `anthropics/claude-plugins-official`, ref `main`, только
отсутствующие пути из списка:

```text
plugins/mcp-server-dev/skills/build-mcp-server
plugins/mcp-server-dev/skills/build-mcp-app
plugins/mcp-server-dev/skills/build-mcpb
```

Не передавайте установщику путь уже установленного навыка: существующий каталог
он не перезаписывает. Если все три навыка установлены, повторно запускать
установщик не нужно.

После установки завершите текущий ход. На следующем ходе проверьте, что доступны
все три навыка для явного вызова и каждый `SKILL.md` читается. Если навык не
появился, перезапустите Codex и повторите проверку. Если хотя бы один обязательный
навык отсутствует, MCP-разработку не начинайте.

### Claude Code

Официальный marketplace обычно уже доступен в Claude Code. Установите из него
плагин и перезагрузите плагины:

```text
/plugin install mcp-server-dev@claude-plugins-official
/reload-plugins
```

Если Claude Code не находит плагин, сначала обновите marketplace командой
`/plugin marketplace update claude-plugins-official`; если marketplace ещё не
подключён — добавьте его командой
`/plugin marketplace add anthropics/claude-plugins-official`.

Основная точка входа — `build-mcp-server`. `build-mcp-app` используется для
MCP Apps и интерактивных виджетов, `build-mcpb` — для локальной упаковки и
поставки. Если нужны оба контура, применяйте навыки в порядке
`build-mcp-server` → `build-mcp-app` → `build-mcpb`.

## Самопроверка

Перед началом работы проверьте Python командой для своей оболочки:

```sh
# macOS или Linux
python3.12 --version

# Windows PowerShell
py -3.12 --version

# Windows Git Bash; вывод должен начинаться с Python 3.12
python --version
```

Затем агент должен получить успешный результат общих команд Rust:

```sh
rustc --version
cargo --version
rustfmt --version
cargo clippy --version
rust-analyzer --version
```

Перед pull request выполните полный набор проверок из
[шаблона PR](.github/PULL_REQUEST_TEMPLATE.md).
