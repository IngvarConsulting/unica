---
id: INV.SURFACE.DOCS-BEFORE-ADMISSION
status: active
governs: product
decision: DEC.2026-09-09.DOCS-BEFORE-WORKSPACE-ADMISSION
check: crates/unica-coder/src/infrastructure/daemon/server.rs::v5_documentation_answers_before_source_admission_without_an_actor_lease
scope: [app, product, wire]
---

# Справка не требует набора исходников

`unica.docs` связывается и исполняется без допущенного PlatformXml source set:
из каталога без `v8project.yaml` и без корней 1С он отвечает своей
документацией, а закрытый список источников — своим `unsupported_source`.
Общий отказ допуска рабочей области ответом `docs` не бывает.

Класс исполнения остаётся `InlineCandidate`: локальное попадание отвечает
сразу, сетевое уходит в Task на общем cutoff. Актора вызов не занимает —
аренду ревизии на наборы исходников он не берёт ни в каком каталоге.
