---
id: INV.APP.SUPPORT-STATE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/application_ports.rs::native_typed_readers_receive_logical_support_targets
scope: [app]
---

# Типизированные читатели запрашивают поддержку по логической цели

Нативные предметные читатели передают в порт состояния поддержки разрешённый
логический объект или подсистему, а не физический путь marker-файла.
