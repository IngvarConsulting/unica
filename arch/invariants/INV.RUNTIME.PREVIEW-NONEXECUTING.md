---
id: INV.RUNTIME.PREVIEW-NONEXECUTING
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::a_preview_does_not_fetch_an_engine_it_will_not_run
scope: [app, product]
---

# Предпросмотр runtime не запускает и не доставляет движок

`unica.runtime.execute` в режиме preview проходит к планирующему обработчику,
не запрашивая доставку отсутствующего движка, который этот вызов не будет
исполнять.
