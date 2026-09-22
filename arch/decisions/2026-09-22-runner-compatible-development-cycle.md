---
id: DEC.2026-09-22.RUNNER-COMPATIBLE-DEVELOPMENT-CYCLE
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/daemon/server.rs::v5_development_cycle_prepares_before_source_admission_and_keeps_revision_gate
supersedes: []
superseded-by: null
establishes: [INV.RUNTIME.RUNNER-ONE-CAPABILITIES, INV.RUNTIME.COMPATIBLE-DEVELOPMENT-CYCLE, CTR.WIRE.TOOL-SURFACE]
changes: [CTR.WIRE.TOOL-SURFACE]
design: docs/design/2026-09-22-runner-compatible-development-cycle-design.md
---

# Переходный раннер обеспечивает цикл разработки

**Решение.** Владелец выбрал совместимое обновление 0.11, чтобы все шесть
операций разработки были исполнимы до готовности 1.0. Ограниченные режимы
публикуют фактические эффекты и недостающие гарантии. Отсутствие контроля
поколений требует явного force там, где перезаписывается работа.

Разделение upload/apply/reset обеспечивает раннер. Unica сохраняет целевые
имена, типизированные аргументы, preview/ifRev и единственную публичную
MCP-границу. Ограниченная реализация не объявляется полной реализацией 1.0.
