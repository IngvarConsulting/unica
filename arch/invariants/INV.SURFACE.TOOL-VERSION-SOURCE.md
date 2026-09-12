---
id: INV.SURFACE.TOOL-VERSION-SOURCE
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_skill_provenance.py::test_v8_runner_tool_lock_ref_resolves_to_locked_baseline
scope: [pkg]
---

# Ссылка происхождения v8-runner разрешается в lock-файл

Запись происхождения `v8-runner-rust` содержит `toolLockRef = v8-runner`, а
соответствующая запись `tools.lock.json` несёт полную привязку сборки: тег,
совпадающий с версией, коммит и хеш на каждую цель.

**Номер версии не фиксируется ни здесь, ни в продуктовом слое.** Откуда забирать
сборку — продуктовое решение (`DEC.2026-09-02.MAINTAINED-ENGINES-PUBLISH-AT-SOURCE`
и `INV.PKG.ENGINE-RELEASE-SOURCES`); какая именно версия закреплена сейчас —
регулярная работа, и живёт она в одном месте, в lock-файле. Поэтому проверка
смотрит на форму привязки и на её внутреннюю согласованность, а не на значения:
прибитая версия означала бы, что каждая новая сборка раннера правит правило.
