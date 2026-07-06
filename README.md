# Unica

Unica - это плагин для Codex, который помогает работать с проектами 1С:Предприятие.

Обычным языком: репозиторий содержит набор инструкций, сценариев и подключаемых инструментов, чтобы Codex мог выполнять типовые задачи 1С-разработчика: создавать объекты конфигурации, собирать внешние обработки и отчеты, обновлять базы, запускать проверки и искать код в больших 1С-проектах.

## Что в этой репе

- `plugins/unica/skills/` - прикладные навыки Codex: формы, метаданные, EPF/ERF, базы, роли, СКД, веб-публикация и другие задачи 1С.
- `plugins/unica/.mcp.json` - MCP-подключения для поиска кода, работы с инструментами 1С и справочными материалами.
- `plugins/unica/scripts/legacy/` - временные legacy-реализации операций, которые еще мигрируют в Rust MCP.
- `plugins/unica/third-party/tools.lock.json` - единый список версий внешних инструментов.
- `.github/workflows/unica-plugin-release.yml` - сборка готового пакета плагина для установки.

Исходники в репозитории не хранят готовые бинарные утилиты. Они собираются в GitHub Actions и попадают в готовый marketplace-пакет.

## Для кого

- Для 1С-разработчиков, которые хотят использовать Codex как помощника по реальным задачам разработки.
- Для тех, кто поддерживает или расширяет сам плагин Unica.
- Для команд, которым нужен воспроизводимый набор 1С-инструментов внутри Codex.

## Установка

На macOS и Linux одна команда скачивает POSIX installer из последнего GitHub
Release, определяет платформу, скачивает нужный пакет Unica и устанавливает его
в Codex:

```sh
curl -fsSL https://github.com/IngvarConsulting/unica/releases/latest/download/install-unica.sh | sh
```

На Windows используйте Windows PowerShell 5.1. `install-unica.sh` не
поддерживает Windows, включая Git Bash, MSYS и Cygwin:

```powershell
iwr https://github.com/IngvarConsulting/unica/releases/latest/download/install-unica.ps1 -OutFile install-unica.ps1
powershell -ExecutionPolicy Bypass -File .\install-unica.ps1
```

Для установки конкретного релиза на macOS или Linux:

```sh
curl -fsSL https://github.com/IngvarConsulting/unica/releases/latest/download/install-unica.sh | sh -s -- --version v0.5.1
```

Для установки конкретного релиза на Windows:

```powershell
powershell -ExecutionPolicy Bypass -File .\install-unica.ps1 -Version v0.5.1
```

Release assets собираются отдельно под платформы:

- `unica-codex-marketplace-darwin-arm64.tar.gz`
- `unica-codex-marketplace-linux-x64.tar.gz`
- `unica-codex-marketplace-win-x64.zip`

Installer выбирает нужный архив для своей платформы, регистрирует marketplace
`unica-local`, обновляет cache Codex и включает `unica@unica-local`.

Проверка:

```sh
codex debug prompt-input 'test'
```

В выводе должны быть видны marketplace `unica-local` и навыки вида `unica:meta-compile`, `unica:v8-runner`, `unica:epf-bsp-init`.

## Установка из исходников для разработки

Этот режим нужен, если вы меняете сам плагин. Git/source marketplace install
из этого репозитория не является runtime-установкой: skills и metadata могут
быть видны, но готовых `bin/<target>/` бинарников и generated
`third-party/manifest.json` в source tree нет. Для рабочего MCP используйте
release installer или generated marketplace archive.

```sh
git clone https://github.com/IngvarConsulting/unica.git
cd unica
scripts/dev/install-local-unica.sh
```

Скрипт соберет пакет под текущую машину из локальных исходников, установит его
в Codex как `unica-local` и проверит свежую сессию через
`codex debug prompt-input`.

## Что нужно для работы

- Установленный Codex CLI.
- Для реальных операций с базами и конфигурациями - установленная платформа 1С.
- Для Windows-сценариев 1С - PowerShell, пока соответствующие legacy-операции не мигрированы.

## Где смотреть детали

- Техническое описание плагина: `plugins/unica/README.md`.
- Внутренняя схема инструментов и сборки: `plugins/unica/references/tooling/internal-package.md`.
- Список pinned-инструментов: `plugins/unica/third-party/tools.lock.json`.

Официальная публикация в публичный каталог Codex будет отдельным шагом, когда OpenAI откроет self-serve публикацию плагинов. Сейчас репозиторий готовит воспроизводимый marketplace-пакет; рабочий runtime-путь идет через release artifacts или локально сгенерированный marketplace-пакет, а не через сырой Git/source checkout.
