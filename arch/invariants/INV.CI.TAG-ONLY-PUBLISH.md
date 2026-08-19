---
id: INV.CI.TAG-ONLY-PUBLISH
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_unica_workflow.py
scope: [ci]
---

# Публикация происходит только по тегу и через один шлюз

Релиз начинается единственным человеческим тегом, каждая платформа проверяет то, что собрала,
и каждый pull request закрывает один агрегирующий шлюз.
