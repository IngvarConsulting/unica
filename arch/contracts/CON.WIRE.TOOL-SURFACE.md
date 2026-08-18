---
id: CON.WIRE.TOOL-SURFACE
status: active
version: 1
decision: DEC.2026-08-18.EIGHT-ENTRIES
producer: scripts/ci/generate-tool-surface.py
consumers: [review, docs]
check: tests/ci/test_tool_surface_ledger.py
---

# Ведомость поверхности порождается из бинаря

Ведомость публичной поверхности порождается из `tools/list` собранного бинаря и
руками не пишется: имена, описания и аргументы принадлежат реестру инструментов,
а ведомость лишь показывает их рядом. Ручной правке подлежит только контракт
результата и сценарии.
