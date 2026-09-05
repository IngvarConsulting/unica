---
id: INV.SOURCE.SNAPSHOT-BINDING
status: superseded
governs: product
decision: DEC.2026-09-05.SOURCE-SNAPSHOT-PROVIDER-RETIRED
check: tests/ci/test_acceptance_scenarios.py::test_every_wire_answers_its_frozen_classes
scope: [source]
---

# Ресурс действует только внутри выдавшего его снимка

Идентификатор ресурса нельзя прочитать с идентификатором другого снимка, даже
если оба снимка получены от одного поставщика для одного источника.

Правило снято вместе с поставщиком снимков: идентификаторов ресурсов больше
нет, а курсор канонического `view` связан с вопросом и ревизией
(`INV.SOURCE.REVISION-BOUND-VIEW-CURSOR`).
