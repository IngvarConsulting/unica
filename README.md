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

Переходные скрипты `scripts/install-unica.sh` и `scripts/install-unica.ps1`
нужны только для миграции прежней локальной схемы. Они клонируют стабильный
Git-каталог, запускают `migrate-preflight`, затем общий нативный transactional
bootstrap. Bootstrap сохраняет резервную копию в
`$CODEX_HOME/unica/migration-backups/`, применяет только команды Codex CLI и при
ошибке восстанавливает точный `config.toml` и прежние регистрации в обратном
порядке. Путь резервной копии печатается в отчёте.

Если миграция завершилась ошибкой, автоматический откат уже выполнен; не
удаляйте cache вручную. Для возврата после успешной миграции сначала удалите
публичную установку командами ниже, затем используйте инструкции и активы того
предыдущего релиза, к которому возвращаетесь.

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

Исходный `.mcp.json` запускает `cargo run`; локальный скрипт собирает инструменты
только для текущей машины. Официальный пакет остаётся тонким: skills, assets,
три bootstrap-бинарника и `runtime-manifest.json`, без полного runtime.

## Репозиторий

- `plugins/unica/skills/` — прикладные навыки 1С;
- `crates/unica-coder/` — единый MCP runtime `unica`;
- `crates/unica-bootstrap/` — загрузка, проверка, запуск и миграция;
- `plugins/unica/third-party/tools.lock.json` — версии внутренних инструментов;
- `.github/workflows/unica-plugin-release.yml` — runtime-релиз;
- `.github/workflows/publish-unica-marketplace.yml` — staging и promotion
  публичного каталога.

Лицензия: LGPL-3.0-or-later.
