---
id: DEC.2026-09-13.EXPLICIT-WORKSPACE-CWD
status: active
governs: product
realized: tests/ci/test_unica_mcp_smoke.py::test_workspace_hint_survives_deleted_frontend_and_daemon_cwd
supersedes: []
superseded-by: null
establishes: [INV.APP.EXPLICIT-WORKSPACE-CWD]
design: docs/design/2026-09-13-explicit-workspace-cwd-design.md
---

# Абсолютный путь рабочего пространства не зависит от каталога процесса

**Решение.** Общий resolver рабочего пространства читает каталог процесса
только для относительного пути или отсутствующего выбора. Абсолютный путь
принимается без `current_dir`, включая сохранённый frontend-ом путь,
переданный daemon. Если каталог процесса недоступен, отказ различает
относительный путь и отсутствие `cwd`.

**Почему.** Уже запущенный процесс переживает удаление своего каталога
запуска. Безусловное вычисление fallback ломает даже независимый абсолютный
путь рабочего пространства.

**Цена.** Отдельные диагностики двух отказов и процессный smoke-тест корня
проекта, типизированных метаданных и поиска BSL после удаления каталогов запуска.
Публичная поверхность и маршрутизация остаются у
`CTR.WIRE.TOOL-SURFACE` и `INV.WIRE.SURFACE-RELEASE-ROUTING`.
