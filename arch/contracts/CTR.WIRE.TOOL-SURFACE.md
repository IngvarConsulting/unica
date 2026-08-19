---
id: CTR.WIRE.TOOL-SURFACE
status: active
governs: product
version: 1
decision: DEC.2026-08-19.SURFACE-BY-QUESTION
producer: scripts/ci/generate-tool-surface.py
consumers: [review, docs]
check: tests/ci/test_tool_surface_ledger.py::test_ledger_matches_the_live_registry
---

# Ведомость поверхности порождается из бинаря

Ведомость публичной поверхности порождается из `tools/list` собранного бинаря и
руками не пишется: имена, описания и аргументы принадлежат реестру инструментов,
а ведомость лишь показывает их рядом. Ручной правке подлежит только контракт
результата и сценарии.
