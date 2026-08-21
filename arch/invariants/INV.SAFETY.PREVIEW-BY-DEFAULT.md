---
id: INV.SAFETY.PREVIEW-BY-DEFAULT
status: active
governs: product
decision: DEC.2026-08-22.EVIDENCE-BOUNDED-SAFETY
check: crates/unica-coder/src/infrastructure/application_ports.rs::public_preview_strategies_are_real_and_recursively_write_free
scope: [app, product]
---

# Preview-стратегии имеют реальные write-free представители

Реестр классифицирует весь набор мутаторов как post-image или planned-command.
Реальный представитель каждого класса — `unica.cf.init` и
`unica.build.load` — без `dryRun: false` не меняет рекурсивный снимок workspace,
состояние, индекс и записи скрытых сервисов. Это доказательство не обобщает две
операции на все комбинации аргументов остальных мутаторов.
