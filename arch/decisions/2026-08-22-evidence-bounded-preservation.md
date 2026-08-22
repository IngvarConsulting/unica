---
id: DEC.2026-08-22.EVIDENCE-BOUNDED-PRESERVATION
status: active
governs: product
realized: tests/arch/test_fate_coverage.py::test_narrowed_v1_claims_are_explicit_and_product_owned
supersedes: []
superseded-by: null
establishes: [INV.PKG.PACKAGED-PUBLIC-SURFACE, INV.TOKEN.CACHE-IMPACT-IN-RESULT]
---

# Перенесённое обещание не шире исполняемого доказательства

**Решение.** От package parity сохраняется только доказанный bootstrap-контур:
legacy `initialize` и direct-first `server/discover`, их `tools/list` и наличие
`unica.project.status`, `unica.standards.search`, `unica.standards.explain`.
Точный набор, схемы и предметные вызовы этим не доказаны. Cache impact в том же
результате доказан для публичного `meta.remove`; замкнутые каталоги событий и
влияния не делают пример универсальным для всех результатов мутаций.

**Почему.** Архив v1 формулировал универсальные требования шире названных им
проверок. Реестр v2 сохраняет проверяемое поведение и отдельно называет снятую
часть вместо выдачи частичного smoke за полное доказательство.

**Цена.** Универсальные гарантии потребуют отдельных исполняемых матриц; до их
появления потребитель опирается только на явно названные контуры.
