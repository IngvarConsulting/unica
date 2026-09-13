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

## Навыки контекстной инженерии

Рекомендуем установить набор навыков
[Agent Skills for Context Engineering](https://github.com/muratcankoylan/agent-skills-for-context-engineering)
Муратджана Кёйлана. Набор не обязателен для сборки и проверок, но полезен при
работе над поверхностью инструментов Unica: описаниями `unica.*`, схемами,
текстами ошибок, `SKILL.md` плагина и бюджетом токенов `tools/list`. Ключевые
навыки для проекта:

- `tool-design` — критерии оценки описаний инструментов, чек-лист и границы
  консолидации поверхности;
- `context-optimization` — стабильность префикса кеша и экономия токенов;
- `evaluation` и `advanced-evaluation` — петля улучшения описаний по
  наблюдаемым провалам маршрутизации.

Остальные навыки набора (`context-fundamentals`, `context-degradation`,
`context-compression`, `multi-agent-patterns`, `memory-systems` и другие)
подключаются по контексту задачи и не мешают основной работе.

### Claude Code

Подключите репозиторий как marketplace и установите единый плагин со всеми
навыками:

```text
/plugin marketplace add muratcankoylan/Agent-Skills-for-Context-Engineering
/plugin install context-engineering@context-engineering-marketplace
/reload-plugins
```

Навыки вызываются с префиксом плагина, например `/context-engineering:tool-design`.

### Codex

Навыки лежат в каталоге `skills/` репозитория, каждый — отдельный каталог с
`SKILL.md` и `references/`. Установите нужные через `$skill-installer` из
repository `muratcankoylan/Agent-Skills-for-Context-Engineering`, ref `main`,
передав пути вида `skills/tool-design`, либо скопируйте каталоги целиком в
`$CODEX_HOME/skills/` (по умолчанию `~/.codex/skills/`). Не сводите навык к
одному файлу `SKILL.md`: относительные ссылки на `references/` перестанут
работать. После установки завершите ход и на следующем проверьте, что навыки
доступны для явного вызова.

## Приёмочные сценарии поверхности

Поверхность v0.13 принимается корпусом задач разработчика:
`tests/fixtures/acceptance/scenario-corpus.json` держит сценарии и провода
вызовов `unica.*`, `tests/ci/test_acceptance_scenarios.py` исполняет их против
собранного бинаря. Человекочитаемый реестр со списком всех сценариев,
покрытием закрытого реестра операций `apply` и профилей `check` лежит в
[`docs/acceptance-scenarios.md`](docs/acceptance-scenarios.md); он
генерируется из корпуса и проверяется на расхождение тем же тестом.

Идеи новых задач и операций, которых поверхность ещё не решает, — самый
полезный вклад: откройте issue по форме
[«Новая операция»](https://github.com/IngvarConsulting/unica/issues/new?template=new_operation.yml)
или добавьте сценарий в корпус по инструкции из реестра и обновите документ
командой `python scripts/ci/render-acceptance-registry.py --write`.

## Отчёт Allure локально

Конвейер публикует отчёт линии на [сайте
проекта](https://ingvarconsulting.github.io/unica/), и собирают его те же
скрипты, что лежат в репозитории: прогон пишет каталог `allure-results`,
Allure CLI превращает его в HTML. Локальный отчёт отвечает на то, о чём
текстовый вывод молчит: какие тесты отключены и почему, что прошло со второй
попытки, куда ушло время набора.

### Allure CLI

Нужны Allure CLI и JRE (проверено на Java 17). На macOS:

```sh
brew install allure
allure --version
```

Версия сайта закреплена по sha256 в
[`.github/workflows/unica-pages.yml`](.github/workflows/unica-pages.yml):
контракт истории у разных версий Allure разный, и меняться под опубликованным
отчётом он не должен. Сейчас там **2.46.1** — та же, что ставит Homebrew, так
что локальный отчёт и отчёт сайта собирает одна версия. Если версии
разойдутся, всё про историю и тренды сайта проверяйте версией сайта: скачайте
архив релиза по ссылке и хешу из workflow и зовите `allure` из распакованного
каталога. Разница не косметическая: 2.35.1 ключевала историю нашим
`historyId`, 2.46.1 считает ключ сама, и переклейку старых ключей делает
сборщик сайта — `migrate_history_keys` в
[`scripts/ci/build-site.py`](scripts/ci/build-site.py).

### Прогон с результатами

Точка входа одна у конвейера и у разработчика:

```sh
python3.12 scripts/ci/run-tests.py --profile all --results .build/allure-results
allure generate --clean --output .build/allure-report .build/allure-results
allure open .build/allure-report
```

`allure serve .build/allure-results` собирает и открывает отчёт одной командой,
не оставляя каталога.

Про сам прогон стоит знать вот что:

- `--profile all` — локальное «гони всё». Профили `pr`, `queue`, `main`,
  `release` и `large` повторяют ворота конвейера тем же отбором, что и там:
  берите их, когда PR покраснел на конкретных воротах.
- `--ecosystem rust|python|all` и `--suite tests/dev` сужают прогон, а
  `--dry-run` печатает команды, ничего не запуская.
- `--runner` по умолчанию `local`. История теста ведётся на раннер, так что
  локальные прогоны не смешиваются с историей конвейера.
- Интерпретатор обязан быть 3.12: скрипт читает `.config/nextest.toml` через
  `tomllib`, и на 3.9 падает на импорте. Наборам `tests/ci` и `tests/arch`
  нужны зависимости из
  [`tests/ci/requirements.txt`](tests/ci/requirements.txt).

Дерево отчёта строится по меткам: `parentSuite` — экосистема (`rust` или
`python`), `suite` — двоичный файл или модуль, рядом `size`, `profile` и
`host`; раннер идёт ещё и параметром теста. Отключённые тесты приходят
`skipped` с причиной из `#[ignore = "..."]` — молча вынести тест из гейта
нельзя. Неудачные попытки видны на графике повторов: nextest делает один
повтор, и зелёный итог не прячет перемежающийся тест.

### Часть тестов

Фильтра у `run-tests.py` нет — он описывает ворота целиком. Когда нужен один
модуль, отбирайте его самим nextest, а JUnit переводите тем же швом, каким
это делает прогон:

```sh
cargo nextest run -p unica-coder --lib -E 'test(/^infrastructure::source_roots::/)' --profile default
```

```sh
python3.12 - <<'PY'
import sys
from pathlib import Path

sys.path.insert(0, "scripts/ci")
import allure_results as ar

out = Path(".build/allure-results")
ar.write_run(out, profile="all", runner="local", ecosystem="rust")
for entry in ar.junit_records(
    Path("target/nextest/default/junit.xml"),
    runner="local",
    profile="all",
    reasons=ar.ignore_reasons(Path(".")),
):
    ar.write(out, entry)
PY
```

### Тренды

У одиночного отчёта истории нет. Чтобы увидеть тренд и метку `flaky`,
перенесите историю прошлого отчёта в свежие результаты перед сборкой:

```sh
cp -r .build/allure-report/history .build/allure-results/history
```

Сайт делает ровно это, только историю берёт с опубликованной страницы линии
(`carry_history` в [`scripts/ci/build-site.py`](scripts/ci/build-site.py)).
План прогона (`--plan-only`) и досбор недошедших тестов
(`scripts/ci/collect-results.py`) локально не нужны: они восстанавливают
исходы упавших джоб по списку задач прогона GitHub Actions.

## MCP Inspector локально

[MCP Inspector](https://github.com/modelcontextprotocol/inspector) показывает
Unica такой, какой её видит хост: весь `tools/list` со схемами, вызов
инструмента с произвольными аргументами и сырой JSON ответа. Ни один хост
этого не показывает, а расхождение описания и провода видно только здесь.

Нужен Node (проверено на 22) и собранный бинарь:

```sh
cargo build --bin unica
```

Собирайте заранее: инспектор запускает готовый файл, а `cargo run` из
[`plugins/unica/.mcp.json`](plugins/unica/.mcp.json) на холодной сборке молчит
в stdio дольше, чем клиент ждёт рукопожатия.

### Рабочий каталог решает всё

Рабочую область `unica` берёт из текущего каталога процесса и ниоткуда больше
(`std::env::current_dir()` в
[`crates/unica-coder/src/interfaces/mcp.rs`](crates/unica-coder/src/interfaces/mcp.rs));
переменной окружения для неё нет. Сервер наследует каталог инспектора,
поэтому запускайте инспектор **из каталога 1С-проекта**, а путь к бинарю
указывайте абсолютный. Из чужого каталога `unica.view` честно ответит
`workspace is uninitialized`, а инструменты, которым нужна рабочая область, —
отказом `workspace actor admission failed`.

Своего проекта под рукой может не быть — тогда берите приёмочную рабочую
область `tests/fixtures/acceptance/workspace`: на ней стоит корпус
приёмочных сценариев, и любой из них воспроизводится вручную.

### Один вызов из терминала

Режим `--cli` не поднимает браузер: одна команда — один сеанс MCP, ответ
печатается в stdout. Путь к бинарю и свой каталог состояния демона удобно
запомнить до перехода в проект:

```sh
UNICA="$PWD/target/debug/unica"
STATE="$PWD/.build/unica-state"
cd tests/fixtures/acceptance/workspace

npx @modelcontextprotocol/inspector --cli "$UNICA" -e UNICA_PROVIDER_STATE_DIR="$STATE" \
  --method tools/list
npx @modelcontextprotocol/inspector --cli "$UNICA" -e UNICA_PROVIDER_STATE_DIR="$STATE" \
  --method tools/call --tool-name unica.run
npx @modelcontextprotocol/inspector --cli "$UNICA" -e UNICA_PROVIDER_STATE_DIR="$STATE" \
  --method tools/call --tool-name unica.docs \
  --tool-arg query="права доступа к регистру сведений" --tool-arg source=platform-help
```

Аргументы инструмента идут по одному `--tool-arg имя=значение`. Переменные
окружения — только флагом `-e ИМЯ=значение`: окружение оболочки инспектор
серверу целиком не передаёт, `export` до вызова ничего не даст. Зачем здесь
`UNICA_PROVIDER_STATE_DIR` — ниже, в «Демон переживает пересборку»; каталог
`.build/` git не отслеживает. Ответ с `isError: true` инспектор дублирует
строкой ошибки и ненулевым кодом возврата, так что режим годится и для
скриптов.

### Веб-интерфейс

```sh
npx @modelcontextprotocol/inspector -e UNICA_PROVIDER_STATE_DIR="$STATE" "$UNICA"
```

Команда печатает адрес вида
`http://127.0.0.1:6274?MCP_INSPECTOR_API_TOKEN=…` и открывает браузер;
`MCP_AUTO_OPEN_ENABLED=false` открытие отключает. Токен обязателен —
`DANGEROUSLY_OMIT_AUTH` не используйте: инспектор запускает произвольные
команды, и открытый порт отдаёт эту возможность кому угодно.

### Чего ожидать

- Поверхность — только инструменты (сегодня одиннадцать `unica.*`);
  `resources/list` и `prompts/list` отвечают пустыми списками, и это не
  поломка.
- Долгая операция возвращает `taskId` и `status: "working"`, а ответ забирают
  `unica.task.result`. Задачи держит фоновый демон (`unica --daemon
  --state-root ~/.unica/provider-state`), поэтому `taskId` переживает выход
  процесса CLI.

### Демон переживает пересборку

Демон опознаётся по ABI ядра и версии протокола, а не по файлу бинаря: живой
демон под `~/.unica/provider-state` переиспользует любой пришедший туда
`unica`, и после последнего вызова он ждёт ещё четверть часа. То есть сразу
после `cargo build` вызов обслужит прежняя сборка, и проверка «починилось ли»
покажет вчерашний ответ. Поэтому в примерах выше стоит свой каталог состояния:
он поднимает отдельного демона от названного бинаря. Добавьте
`-e UNICA_DAEMON_IDLE_GRACE_MS=5000`, чтобы он уходил через пять секунд после
последнего вызова, а не держал каталог четверть часа.

Путь состояния должен быть настоящим: `mktemp -d` на macOS отдаёт путь через
симлинк `/var`, и демон отвергает его с `Not a directory`. Грубая замена
разведению — `pkill -f "unica --daemon"`, но она снимает и демонов, которыми
пользуются запущенные хосты. В веб-интерфейсе сервер поднимается один раз на
сеанс, поэтому после пересборки перезапускайте инспектор, а не только
переподключайтесь.

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
