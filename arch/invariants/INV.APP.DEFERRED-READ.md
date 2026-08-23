---
id: INV.APP.DEFERRED-READ
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::continuation_slices_byte_stably_without_rereading_the_source
scope: [app]
---

# Продолжение читает неизменяемый сохранённый снимок

Повтор одной селекции по `resultRef` возвращает побайтно стабильный срез, не
вызывая предметный читатель и обнаружение рабочего пространства повторно.
