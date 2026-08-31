---
id: CTR.WIRE.TOOL-SURFACE
status: active
governs: product
version: 2
decision: DEC.2026-08-31.V0-13-SURFACE-FIRST-CUTOVER
producer: scripts/ci/generate-tool-surface.py
consumers: [review, docs]
check: crates/unica-coder/src/interfaces/mcp.rs::production_mcp_surface_exposes_only_canonical_v13_tools_and_task_compatibility
scope: [wire]
---

# Package-selected поверхность содержит восемь или одиннадцать инструментов

Ведомость публичной поверхности порождается из `tools/list` собранного бинаря и
руками не пишется: имена, описания и аргументы принадлежат реестру инструментов,
а ведомость лишь показывает их рядом. Native Tasks профиль содержит ровно
восемь предметных `unica.*`; compatibility-профиль добавляет ровно три
`unica.task.*`. Имена v0.12 в обоих профилях отсутствуют. Ручной правке подлежит
только контракт результата и сценарии.
