---
id: INV.CI.TAG-ONLY-PUBLISH
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_unica_workflow.py::test_only_tag_pushes_enable_release_behavior
scope: [ci]
---

# Релиз начинается человеческим тегом

Публикация включается только пушем тега. Ни контур упаковки, ни дымовой прогон
pull request релизных ассетов не публикуют.
