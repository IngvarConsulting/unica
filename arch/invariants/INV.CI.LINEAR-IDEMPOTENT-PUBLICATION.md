---
id: INV.CI.LINEAR-IDEMPOTENT-PUBLICATION
status: active
governs: product
decision: DEC.2026-08-22.LINEAR-PUBLICATION
check: tests/ci/test_unica_workflow.py::test_publication_is_one_linear_pass_ordered_by_needs
scope: [ci, pkg]
---

# Публикация идёт одним возобновляемым проходом

Конвейер выполняет stage, tag, fresh-install и upgrade verification, затем
promotion по явным `needs`. Повтор обнаруживает готовые стадии и тот же тег,
никогда не двигает тег силой и не продвигает каталог до зелёной установки.
