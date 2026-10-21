# Где искать код Unica

Открывайте эту карту, когда нужно выбрать исходники и проверки по области задачи.

Пути в обоих столбцах даны от корня репозитория. Применимые правила ищите
в `arch/rules/` по предмету задачи, исходникам и ссылкам `check`; прочитайте
их до выбора решения. Вместо `<имя>` подставьте имя навыка.

| Задача | Что читать сначала | Где менять код |
| --- | --- | --- |
| Публичный инструмент `unica.*` | `docs/tool-surface.md`, `crates/unica-coder/src/interfaces/mcp.rs`, `tests/fixtures/acceptance/` | Каталог `crates/unica-coder/src/application/v13/tool_catalog.rs`, связывание `crates/unica-coder/src/infrastructure/daemon/v13_service.rs`, затронутый навык `plugins/unica/skills/<имя>/SKILL.md` |
| Грамматика и возможности семейства операций | Дескрипторы и тесты своего семейства: `crates/unica-metadata/`, `unica-forms/`, `unica-roles/`, `unica-dcs/`, `unica-xdto/`, `unica-templates/`, `unica-bsl/` | Семейство владеет типизированным контрактом; общая регистрация — `crates/unica-operations/`, проверка схемы — `crates/unica-operation-contract/`, контекст адреса — `crates/unica-source-contract/src/context.rs` |
| XML 1С, чтение или изменение представления | Нужная спецификация в `plugins/unica/references/specs/`, фикстуры и пакет своего семейства | Кодек `crates/unica-<семейство>-designer/`; общие layout и регистрация — `crates/unica-source-designer/`, XML-примитивы — `crates/unica-xml-format/` |
| Обнаружение и выбор набора исходников | `crates/unica-source-contract/src/discovery.rs`, `crates/unica-source-integration-tests/tests/` | Грамматика — `crates/unica-source-designer/src/project_sources.rs`, физические наблюдения — `crates/unica-workspace/src/discovery.rs`, сохранённые свидетельства — `crates/unica-workspace/src/selection_evidence.rs` |
| Подготовка, защита и публикация изменения исходников | `crates/unica-source-contract/src/proposal.rs`, правила `arch/rules/workspace/` | Владение и допуск — `crates/unica-workspace/src/actor.rs`, staging — `apply.rs`, транзакция — `compile_transaction.rs`; семейные предложения создаёт кодек, связывает продуктовая композиция |
| Кеш и события рабочего пространства | `crates/unica-source-contract/src/events.rs`, ближайшие тесты владельца | `crates/unica-workspace/src/cache.rs`, `state.rs`, `actor.rs`; семантику событий задаёт их фактический производитель |
| Поиск кода, навигация и диагностика | `crates/unica-code-intelligence/`, `crates/unica-navigation/`, `crates/unica-diagnostics/` | Координатор своей возможности; реализации — `crates/unica-adapter-rlm/`, `unica-adapter-bsl-analyzer/`, `unica-adapter-lexical/`; связывание — `crates/unica-coder/src/infrastructure/analysis_composition.rs` |
| Синтаксис BSL и версии parser/analyzer | `plugins/unica/third-party/tools.lock.json`, корневые `Cargo.toml` и `Cargo.lock` | `crates/unica-bsl-parser/` скрывает апстрим-типы; `crates/unica-bsl/` использует нормализованные факты |
| Связи методов, событий и деклараций | `crates/unica-bindings/src/projection.rs`, правила `arch/rules/workspace/sources/module-*.md` | Проекция сопоставляет факты BSL, Forms и Metadata; форматные фикстуры проверяет `crates/unica-bindings-integration-tests/tests/projection.rs` |
| Скрытый сервис анализа | `crates/unica-analysis-services/src/backend.rs`, `protocol.rs`, тесты изоляции | `crates/unica-analysis-services/` владеет общим жизненным циклом, конкретный адаптер — своим протоколом и сессией движка |
| Задание runtime, Task и Receipt | `crates/unica-execution-contract/`, правила `arch/rules/runtime/` | `crates/unica-execution/` — полномочия, хранилища и исходы; `crates/unica-runner/` — политика запуска; `crates/unica-adapter-v8-runner/` — существующий движок |
| Документация и её поставщики | `crates/unica-documentation/`, правила `arch/rules/analysis/documentation/` | `crates/unica-documentation-sources/`; продукт связывает выбранных поставщиков с координатором |
| Доставка артефакта или сетевой выход | Правила `arch/rules/distribution/` и тесты своего владельца | `crates/unica-artifact-delivery/`, `crates/unica-network/`; выбор установленного движка остаётся продуктовой композицией |
| Процессы, отмена и общая работа | Порты и тесты фактического владельца | `crates/unica-process/` — процессы и потоки; `crates/unica-work-control/` — сроки, отмена и механизм single-flight; точные ключи и политика — у потребителя |
| Упаковка или релиз | Контрактные метаданные пакета, `docs/release-runbook.md` | `scripts/ci/package-unica-plugin.py`, `crates/unica-bootstrap/`, `.github/workflows/unica-plugin-release.yml` |
| Поведение, зависящее от ОС | `scripts/ci/check-rust-platform-boundary.py`, `tests/ci/test_rust_platform_boundary.py` | Приватный `src/platform/` фактического владельца: `unica-bootstrap`, `unica-process`, `unica-workspace`, `unica-artifact-delivery`; вход продукта — `crates/unica-coder/src/infrastructure/platform/`, native-тесты — `crates/*/tests/platform/` |
| Сборка, отбор и исполнение тестов | [unica-testing](../../unica-testing/SKILL.md), `CONTRIBUTING.md` | `scripts/ci/run-tests.py`, `ci_rust_lanes.py`, `prepared_binary.py`, `ci_shadow_selector.py`; размер — `.config/nextest.toml` и `scripts/ci/size-filters.py` |
| Архитектурная граница | [unica-rules](../../unica-rules/SKILL.md), применимая запись и тело её `check` | `scripts/ci/check-component-boundaries.py` проверяет направления зависимостей, код и тест меняются у фактического владельца |

Читайте объединение подходящих строк: задача может затрагивать несколько
областей.

Маршрут вызова прослеживайте от MCP-входа через сервис к обработчику.
Реализацию режима подтверждают его тело и содержательные проверки,
включая приёмочный корпус `tests/fixtures/acceptance/`. Типизированный ответ
сам по себе не доказывает, что операция реализована.
