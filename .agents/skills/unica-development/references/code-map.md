# Где искать код Unica

Открывайте эту карту, когда нужно выбрать исходники и проверки по области задачи.

Пути в обоих столбцах даны от корня репозитория. Применимые правила ищите
в `arch/rules/` по предмету задачи, исходникам и ссылкам `check`; прочитайте
их до выбора решения. Вместо `<имя>` подставьте имя навыка.
Сохраняйте полные пути от корня, чтобы по ним находился файл.

| Задача | Что читать сначала | Где менять код |
| --- | --- | --- |
| Новый или изменённый публичный инструмент `unica.*` | `docs/tool-surface.md`, `crates/unica-coder/src/interfaces/mcp.rs`, `tests/fixtures/acceptance/scenario-corpus.json` | `crates/unica-coder/src/application/v13/tool_catalog.rs`, `crates/unica-coder/src/infrastructure/daemon/v13_service.rs`, `crates/unica-coder/src/infrastructure/native_operations/apply_families/`, `plugins/unica/skills/<имя>/SKILL.md` |
| Изменение формата XML 1С или DSL | Нужная спецификация в `plugins/unica/references/specs/` и проверяющие её фикстуры | `crates/unica-coder/src/infrastructure/native_operations/` |
| Кеш, состояние рабочего пространства, доменные события | Код владения состоянием и ближайшие тесты в перечисленных модулях | `crates/unica-coder/src/domain/events.rs`, `crates/unica-coder/src/domain/cache.rs`, `crates/unica-coder/src/infrastructure/workspace_state.rs`, `crates/unica-coder/src/infrastructure/workspace.rs`, `crates/unica-coder/src/infrastructure/workspace_actor.rs` |
| Скрытый сервис рабочего пространства или задание runtime | Код жизненного цикла и тесты изоляции в перечисленных модулях | `crates/unica-coder/src/infrastructure/workspace_services.rs`, `crates/unica-coder/src/infrastructure/runtime_jobs.rs` |
| Упаковка или релиз | Контрактные метаданные пакета, `docs/release-runbook.md` | `scripts/ci/package-unica-plugin.py`, `crates/unica-bootstrap/src/`, `.github/workflows/unica-plugin-release.yml` |
| Поведение, зависящее от ОС | `scripts/ci/check-rust-platform-boundary.py`, `tests/ci/test_rust_platform_boundary.py` | `crates/unica-coder/src/infrastructure/platform/`, `crates/unica-bootstrap/src/platform/` |
| Само архитектурное правило | Применимая запись в `arch/rules/` и тело названного в `check` теста | Код, поведение которого проверяет этот тест |

**Строки комбинируются.** Одна задача обычно попадает сразу в несколько:
инструмент, который пишет платформенный XML, — это первая строка вместе со
второй; инструмент, который меняет файлы и обязан сообщить о влиянии на кеш, —
первая вместе с третьей; новый инструмент, запускающий долгую работу, — первая
вместе с четвёртой. Читайте объединение подходящих строк, а не одну самую
похожую.

Публичный каталог и схемы задаёт
`crates/unica-coder/src/application/v13/tool_catalog.rs`. Маршрут вызова
прослеживайте через `crates/unica-coder/src/interfaces/mcp.rs` и
`crates/unica-coder/src/infrastructure/daemon/v13_service.rs` к обработчику.
Семейства `apply` находятся в
`crates/unica-coder/src/infrastructure/native_operations/apply_families/`.
Реализацию режима подтверждают тело обработчика и содержательные проверки,
включая `tests/fixtures/acceptance/scenario-corpus.json`. Типизированный ответ
сам по себе не доказывает, что операция реализована.
