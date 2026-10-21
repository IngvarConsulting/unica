# Разработка Unica

Этот документ описывает окружение и инструменты разработки. Рабочий вход
для агента — [AGENTS.md](AGENTS.md); порядок выбора и написания проверок —
[unica-testing](.agents/skills/unica-testing/SKILL.md).
Открывайте нужный раздел при настройке, изменении окружения или сбое.
Команды сборки и проверки приведены в разделе
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

### Claude Code в worktree

Если в worktree нет собственного `.claude/skills`, Claude Code с версии 2.1.277
загружает скиллы из `.claude/skills` основного checkout; `.claude/agents` и
`.claude/commands` подгружаются так же
([Anthropic — worktrees](https://code.claude.com/docs/en/worktrees#what-worktrees-share-with-the-main-checkout)).
В Unica `.claude/` не отслеживается, поэтому в сессию попадает содержимое
`.claude/` основного checkout. Держите его на актуальном `main` и не храните там
скиллы разработки: старая ветка или неотслеживаемый файл добавят устаревший
скилл во все сессии worktree.

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

### Проверка локального плагина в Claude Code

Для проверки исходного дерева запустите `claude --plugin-dir ./plugins/unica`
из корня репозитория. Установка из маркетплейса для этого не нужна.
Один каталог плагина обслуживает три хоста: Claude читает
`plugins/unica/.claude-plugin/plugin.json`, Codex —
`plugins/unica/.codex-plugin/plugin.json`, ZCode —
`plugins/unica/.zcode-plugin/plugin.json`.

Продуктовые скиллы вызываются с префиксом плагина, например `/unica:meta-info`.
Канонические имена MCP-инструментов в тексте — `unica.*`; в Claude имя имеет
вид `mcp__plugin_unica_unica__<tool>`, где символы вне `A-Za-z0-9_-` заменены
на `_`. Например, `unica.check` становится
`mcp__plugin_unica_unica__unica_check`. Реальный каталог берите из подключённой
сборки и профиля; успешный вызов установленного плагина не проверяет checkout.

### Проверка локального плагина в ZCode

Упакуйте собранные для текущей машины инструменты через
`scripts/ci/package-unica-plugin.py` с `--local-debug-host zcode`,
`--local-debug-target <цель>` и отдельным `--marketplace-name unica-zcode-dev`.
Остальные параметры локальной сборки описаны в [README плагина](plugins/unica/README.md#local-development).
Упаковщик создаёт `marketplace/marketplace.json`, указывающий на `./plugins/unica`;
MCP запускает бинарник этого пакета по абсолютному пути после подстановки корня
хостом. Не подменяйте этим сценарием проверку тонкого релизного пакета.

Добавьте абсолютный путь `marketplace/` в **Plugin Marketplace → Add → Add Plugin
Marketplace**, затем установите `unica` из этого marketplace в **Personal**.
Отключите прежнюю установку Юники через **Settings → Plugins**, не удаляя её
кеши. Обновление состоит из refresh существующего marketplace и update плагина;
прямая правка установленного каталога не используется.

В новой задаче с той же моделью и провайдером проверьте:

1. Доступны актуальные `unica.view`, `unica.run` и остальные канонические
   инструменты, а не сохранённый каталог `unica.runtime.execute` версии 0.12.
2. `unica.run {}` возвращает словарь операций без запуска 1С, не требуя
   заполнить `op`, `args`, `ifRev` и другие необязательные поля.
3. Корневой `unica.view {}` относится к выбранному тестовому проекту, не к
   каталогу плагина. Набор навыков и сервер `unica` не дублируются.
4. Предпросмотр одной поддержанной операции из полученного словаря возвращает
   план без изменений базы; нерелевантный аргумент даёт предметный отказ.

Сохраните версии ZCode, плагина, модель/провайдера и ответы без учётных данных.
Прямой stdio-тест проверяет серверный контракт, а эта проверка — путь через
ZCode к модели. Не объявляйте [#1032](https://github.com/IngvarConsulting/unica/issues/1032)
исправленным только по упаковке или `tools/list`. Результаты разных хостов,
ОС и протокольных профилей не заменяют друг друга.

## Навыки MCP Server Dev

Подключайте нужный навык официального комплекта Anthropic
[`mcp-server-dev`](https://github.com/anthropics/claude-plugins-official/tree/main/plugins/mcp-server-dev)
по задаче:

- `build-mcp-server` — выбор MCP-контракта или модели доставки;
- `build-mcp-app` — MCP Apps и интерактивные виджеты;
- `build-mcpb` — упаковка и поставка MCPB.

Обычная правка Unica не требует всего комплекта. Основной проектный навык —
[unica-development](.agents/skills/unica-development/SKILL.md).

### Codex

Если для задачи нужен отсутствующий навык, проверьте его доступность в хосте.
Ожидаемый путь локально установленного навыка —
`$CODEX_HOME/skills/<имя>/SKILL.md`; если `CODEX_HOME` не задан —
`~/.codex/skills/<имя>/SKILL.md`. Вызовите `$skill-installer` и попросите
установить из repository `anthropics/claude-plugins-official`, ref `main`, только
нужные отсутствующие пути из списка:

```text
plugins/mcp-server-dev/skills/build-mcp-server
plugins/mcp-server-dev/skills/build-mcp-app
plugins/mcp-server-dev/skills/build-mcpb
```

Не передавайте установщику путь уже установленного навыка: существующий каталог
он не перезаписывает. Если нужный навык уже доступен, повторно запускать
установщик не нужно.

После установки проверьте в новой сессии, что нужный навык доступен и его
`SKILL.md` читается. Наличие файла на диске не доказывает регистрацию в хосте.

### Claude Code

Если нужен навык из комплекта, установите плагин из официального marketplace
и перезагрузите плагины:

```text
/plugin install mcp-server-dev@claude-plugins-official
/reload-plugins
```

Если Claude Code не находит плагин, сначала обновите marketplace командой
`/plugin marketplace update claude-plugins-official`; если marketplace ещё не
подключён — добавьте его командой
`/plugin marketplace add anthropics/claude-plugins-official`.

Установка плагина не означает, что для каждой задачи нужно читать все его
навыки. Выбирайте применимый по описанию и загружайте нужные ему материалы.

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

Поверхность v0.13 принимается корпусом задач разработчика в
`tests/fixtures/acceptance/`; `tests/ci/test_acceptance_scenarios.py` исполняет
его против собранного бинаря. Сценарии `S001`–`S440` лежат в
`scenario-corpus.json` и сохраняют свои номера; этот блок закрыт. Каждый новый
сценарий — отдельный файл `scenarios/<id>.json` с идентификатором
`S-<issue>-<slug>`, а его шаги заморожены в `scenario-manifest/<id>.steps`
командой `python scripts/ci/acceptance_corpus.py --freeze <id>`. Сквозного
номера и общих счётчиков нет, поэтому изменения с разными сценариями
не конфликтуют и могут стоять в очереди слияния вместе. Пропавший сценарий
или шаг без правки манифеста проваливает тест корпуса.

Новая source-фикстура регистрируется отдельным файлом
`source-profiles/<имя-каталога-workspace>.json` с содержимым
`{"profile": "source"}`. Имя указывает на соседний каталог фикстуры внутри
`tests/fixtures/acceptance/`; общий Python-список менять не требуется.
Декларации лежат отдельно от рабочих пространств и не копируются в запуск
продукта. Неизвестная фикстура, неверная декларация или профиль без своего
драйвера останавливают проверку корпуса, включая невыбранные профили.

Человекочитаемый реестр со списком всех сценариев, покрытием закрытого
реестра операций `apply` и профилей `check` собирается из корпуса вместе
с сайтом и публикуется на странице
[реестра приёмочных сценариев](https://ingvarconsulting.github.io/unica/acceptance-scenarios.html). Локально его печатает
`python scripts/ci/render-acceptance-registry.py`.

Идеи новых задач и операций, которых поверхность ещё не решает, — самый
полезный вклад: откройте issue по форме
[«Новая операция»](https://github.com/IngvarConsulting/unica/issues/new?template=new_operation.yml)
или добавьте сценарий в корпус по инструкции из реестра.

## Ворота pull request

Основной workflow запускается при открытии, повторном открытии и новом push
в pull request. Смена меток сама по себе его не запускает, чтобы новый пустой
прогон не заменял для ruleset последний результат обязательной проверки
`Unica CI` на той же вершине.

PR исполняет профиль `pr`: малые тесты, выбранный Rust-контур — на Ubuntu. Очередь слияния
проверяет будущее дерево `main` профилем `queue`: малые и средние тесты,
Rust на Ubuntu полностью, а на macOS — только тесты платформенных фасадов
(профиль nextest `queue-platform`). Полный macOS, включая Clippy, идёт ночью
и в релизе: предупреждение или ветку `cfg` под macOS вне платформенных
каталогов первой увидит ночь, уже в `main`. Если группа содержит изменения вне разрешённого списка
прозы или состав изменений не удалось достоверно установить, выполняется весь
этот контур.

`small` и `medium` описывают стоимость и зависимости исполнения теста.
В Rust набор `small` включает unit-тесты продукта: сначала Cargo собирает
тестовые бинарники, затем nextest исполняет выбранные тесты. Python-наборы
`tests/ci` и `tests/dev` содержат проверки инфраструктуры и инструментов
репозитория, а также интеграционные проверки продукта; их `small`
не заменяет Rust-тесты.

Если diff состоит только из добавлений или правок обычных текстовых файлов
из узкого списка прозы, PR и очередь выполняют Guards и Python `small`
в обоих наборах — `tests/ci` и `tests/dev`. Python `medium`, Rust-тесты
и Clippy пропускаются; продукт не компилируется. В очереди средние
Python-тесты отражаются в отчёте как `skipped` с причиной `docs_only`.
Guards сохраняют проверку форматирования через `cargo fmt`; отдельные малые
проверки читают описание workspace через `cargo metadata --no-deps`.
Эти команды не собирают продукт.

В список прозы входят, например, `README.md`, `CONTRIBUTING.md` и Markdown
в `arch/`; полный список задают `PROSE_PATHS` и `is_prose_path` в
[`classify-workflow-changes.py`](scripts/ci/classify-workflow-changes.py).
Расширения `.md` самого по себе недостаточно: инструкции плагина, спецификации,
генерируемые реестры и код сайта в исключение не входят. Удаление, переименование,
смена типа или режима файла также не получают этот пропуск.

Прогон занимает одну джобу на ОС: запись очереди — Linux-джоба `Unica CI`
и джоба `Rust (macos-14)`, pull request — только `Unica CI` (метка `ci:full`
добавляет macOS). `Unica CI` подряд исполняет guards, сборки, тесты, Clippy
и Python, в конце ждёт джобу macOS и сверяет общий инвентарь. Красный шаг
не отменяет следующие: ошибка линта или отправки SARIF не лишает отчёт
результатов тестов. Сборка поставки ждёт успешные `Unica CI` и джобу ОС.
В профиле `large` Windows остаётся advisory: его сбой виден, но не блокирует
поставочный контур.

Каждая джоба создаёт nextest archive и полный план для каждой полосы
своей ОС (ordinary, ReceiptLedger с тестовой feature, ночью и на теге —
vendor/ureq) и исполняет его, не передавая другой джобе. Сверка проверяет,
что результаты покрыли полный план ровно один раз для той же ревизии, ОС,
конфигурации и попытки workflow. Потерянная полоса или обязательный
результат останавливают gate. При «Re-run failed jobs» перезапускайте весь
прогон: результат джобы прошлой попытки сверка отвергает.

Python `medium` получает ordinary-сборку той же джобы: продукт собирается
в общий `target` и сразу копируется вне него. `UNICA_PREPARED_BINARY_MANIFEST`
указывает на проверенный манифест этой неизменяемой копии. Проверяются дерево
исходников, SHA, target, toolchain, cfg/features, hash и режим файла.
Receipt-сборка и устаревший артефакт отвергаются. В CI без этой переменной
Python-тест отказывает, а не собирает продукт. Локально без неё
`ensure_binary` один раз на процесс собирает продукт командой `LOCAL_BUILD`
из `scripts/ci/prepared_binary.py` и пишет об этом в stderr. Для повторных
прогонов на неизменном дереве можно один раз подготовить манифест:
`python scripts/ci/prepared_binary.py build --destination DIR --archive FILE`
(отдельная сборка в `.build/ci-product-target`, `DIR` не должен существовать)
и задать `UNICA_PREPARED_BINARY_MANIFEST=DIR/manifest.json`. После правки
исходников, включая неотслеживаемые файлы, манифест отвергается; переменную
тогда снимите.

Компонентный selector сравнивает графы Cargo на base/head и выбирает
изменённые packages с обратными потребителями. Неизвестные входы, fixtures,
locks, общая composition и неполный граф выбирают full. В CI его решение
сужает только medium pull request; обязательный набор очереди остаётся
полным. Повторное исполнение суженного кандидата ради замера (`--shadow`
у `--phase run`) в CI не идёт: в одной джобе оно удлиняло бы путь до гейта.
Это решение не даёт допуска сокращать очередь.

Push в `main` после очереди повторно не запускает Python-тесты, Rust-тесты
и Clippy; джоба macOS на нём не идёт. При изменении CI или toolchain шаг
`warm` в `Unica CI` прогревает зависимости на Ubuntu: компилирует тестовые
варианты с `--no-run`, продукт и выполняет `cargo check`. Неизвестный
диапазон push также включает прогрев. Без прогрева push в `main` кеш
не трогает. Ключ зависимостей macOS пишет ночной прогон. Прогрев не создаёт
тестовых результатов.
Push в релизную ветку использует профиль `main`, тег — `release`. При ручном
запуске выбирают `main` или `large`; ночной workflow запускает `large`.
Все эти события сохраняют свой контур без исключения для прозы.
Имя ночного профиля `large` сохранено для истории линий, но он исполняет
small + medium + large через nextest `nightly-full`, ordinary/Receipt/vendor
и оба Python-набора. Прежний nextest `large` остаётся каталогом тяжёлых тестов
для разметки размеров. Старые отчёты large-only не доказывают полную ночную
проверку. `verification.json` различает обязательный gate и прохождение всей
матрицы: видимый Windows advisory не становится успешной полной проверкой.

Метка `ci:full` действует со следующего запуска основного workflow — push в
pull request (`synchronize`) или повторного открытия (`reopened`): снимает
отбор по изменённым файлам, включая исключение для прозы, и добавляет Rust-тесты
и Clippy на macOS. Профиль остаётся `pr`, поэтому `medium`-тесты метка не включает.
«Re-run jobs» метку не
подхватывает: перезапуск берёт прежний снимок меток. Без нового кода метку
применяет пустой коммит или закрытие и повторное открытие PR. Снятие `ci:full`
действует так же.

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
  `tomllib`, и на 3.9 падает на импорте. Набору `tests/ci` нужны зависимости из
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

### Как выбирается рабочая папка

При запуске через хост рабочую папку передаёт Codex, Claude Code или ZCode.
Приоритет имеют метаданные текущего вызова; затем — первый MCP root, если
клиент объявил `roots` в сессии до протокола 2026-07-28; затем — переменные
`CLAUDE_PROJECT_DIR` или `ZCODE_PROJECT_DIR`, сохранённые при запуске сервера.
Некорректный переданный контекст даёт отказ без подстановки другого проекта.
Упакованный плагин требует контекст хоста: его технический каталог запуска
не становится рабочей папкой. Каналы и их проверки описаны в
[контракте контекста хоста](arch/rules/workspace/host-context.md).

При прямом запуске бинарника без контекста хоста и без требования его передать
используется каталог запуска. Поэтому для примеров Inspector ниже перейдите
**в каталог 1С-проекта**, а путь к бинарю укажите абсолютный. Явный абсолютный
путь проекта, переданный хостом, не зависит от текущего каталога процесса
([контракт выбора cwd](arch/rules/workspace/explicit-workspace-cwd.md)).
Ответ `unica.view {}` показывает найденный `workspaceRoot` и канал его выбора
в `workspaceRootOrigin`; каталог состояния демона рабочую папку не задаёт.

Своего проекта под рукой может не быть — тогда берите приёмочную рабочую
область `tests/fixtures/acceptance/workspace`: на ней стоит корпус
приёмочных сценариев, и любой из них воспроизводится вручную.

### Один вызов из терминала

Режим `--cli` не поднимает браузер: одна команда — один сеанс MCP, ответ
печатается в stdout. Путь к бинарю и свой каталог состояния демона удобно
запомнить до перехода в проект:

```sh
UNICA="$(pwd -P)/target/debug/unica"
STATE="$(pwd -P)/.build/unica-state"
cd tests/fixtures/acceptance/workspace

npx @modelcontextprotocol/inspector --cli "$UNICA" -e UNICA_PROVIDER_STATE_DIR="$STATE" \
  --method tools/list
npx @modelcontextprotocol/inspector --cli "$UNICA" -e UNICA_PROVIDER_STATE_DIR="$STATE" \
  --method tools/call --tool-name unica.view
npx @modelcontextprotocol/inspector --cli "$UNICA" -e UNICA_PROVIDER_STATE_DIR="$STATE" \
  --method tools/call --tool-name unica.run
npx @modelcontextprotocol/inspector --cli "$UNICA" -e UNICA_PROVIDER_STATE_DIR="$STATE" \
  --method tools/call --tool-name unica.docs \
  --tool-arg query="права доступа к регистру сведений" --tool-arg source=platform-help
```

Аргументы инструмента идут по одному `--tool-arg имя=значение`. Переменные
окружения — только флагом `-e ИМЯ=значение`: окружение оболочки инспектор
серверу целиком не передаёт, `export` до вызова ничего не даст. Зачем здесь
`UNICA_PROVIDER_STATE_DIR` — ниже, в «Проверка пересобранного бинаря»; каталог
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

### Проверка пересобранного бинаря

Идентичность демона включает ABI ядра, версию протокола и сборку. При запуске
ядра через bootstrap сборку определяет SHA-256 архива ядра для текущей платформы
из манифеста рантайма; при прямом запуске — SHA-256 исполняемого файла.
Разные сборки получают отдельные каталоги демона внутри корня состояния.
Новый процесс `unica` после `cargo build` вычисляет идентичность текущего
бинарника и не подключается к демону другой сборки.

Уже работающий процесс сервера сохраняет свою идентичность. Если перед
запуском демона повторная проверка исполняемого файла или манифеста показывает
другую сборку, сервер отказывает с требованием перезапустить хост. Пока прежний
демон работает, ранее запущенный сервер может продолжать обращаться к нему.
Поэтому после пересборки перезапустите сервер Inspector; в веб-интерфейсе
перезапустите сам инспектор, а не только переподключайтесь. Эти условия
закреплены в [контракте идентичности демона](arch/rules/runtime/daemon-identity.md).

Отдельный `UNICA_PROVIDER_STATE_DIR` в примерах изолирует отладочное состояние
от состояния установленных плагинов. Для быстрого завершения демона добавьте
`-e UNICA_DAEMON_IDLE_GRACE_MS=5000`, чтобы он уходил через пять секунд после
последнего вызова, а не держал каталог четверть часа.

Путь состояния должен быть без симлинков; поэтому примеры используют `pwd -P`.
`mktemp -d` на macOS может вернуть путь через симлинк `/var`, который демон
отвергнет с `Not a directory`. Для такого каталога сначала получите физический
путь командой `pwd -P`, выполнив её внутри него. Завершать все демоны через
`pkill -f "unica --daemon"` для этой проверки не нужно: команда затронет и
демонов работающих хостов.

## Самопроверка

При настройке, изменении окружения или сбое запуска проверьте Python командой
для своей оболочки:

```sh
# macOS или Linux
python3.12 --version

# Windows PowerShell
py -3.12 --version

# Windows Git Bash; вывод должен начинаться с Python 3.12
python --version
```

Для проверки доступности Rust-инструментов выполните:

```sh
rustc --version
cargo --version
rustfmt --version
cargo clippy --version
rust-analyzer --version
```

Перед pull request выберите локальные тесты по
[unica-testing](.agents/skills/unica-testing/SKILL.md#что-запускать)
и выполните обязательные проверки из
[порядка подготовки PR](.agents/skills/unica-development/references/pull-requests.md#вливание-через-очередь).
В [шаблоне PR](.github/PULL_REQUEST_TEMPLATE.md) укажите выполненные команды,
результаты и ограничения проверки; перед отправкой выполните `git diff --check`.

### Приёмка агентом Codex

Набор `tests/agent_evaluation` запускается только локально и по решению владельца
не входит ни в один профиль `scripts/ci/run-tests.py` или GitHub Actions.
Запускайте его при изменении навыков или поверхности, которые проверяют агентные
сценарии:

```sh
python3.12 -m unittest discover -s tests/agent_evaluation -p 'test_*.py'
```

Для прогона нужны установленный Codex CLI и действующая авторизация.
Команда приёмки проверена с `0.163.0-alpha.10.2`: CLI должен поддерживать
`--ignore-user-config`, `--ignore-rules` и отключение возможностей через
`--disable`, в том числе `--disable view_image`. Устаревшая настройка
`tools.view_image` не используется. Перед запуском сценариев на другой версии
проверьте совместимость общей команды агента и reviewer:

```sh
python3.12 -m unittest tests.agent_evaluation.test_codex_cli
```

Эта проверка выполняет короткий запрос к модели без инструментов и сохраняет
версию CLI, команду и JSONL в `.session-temp/acceptance-agent/CodexCLI-*`.
Ошибки и неизвестные события CLI считаются отказом проверки, в том числе
предупреждения о нераспознанных настройках; они не отфильтровываются.

Если CLI или авторизации
нет, набор завершается ошибкой с причиной и не объявляет сценарии пройденными
или пропущенными. Агент и отдельный reviewer работают с подготовленным пакетом
текущей сборки; протокол и физический результат сохраняются в
`.session-temp/acceptance-agent/`.
