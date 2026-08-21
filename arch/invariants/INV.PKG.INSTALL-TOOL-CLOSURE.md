---
id: INV.PKG.INSTALL-TOOL-CLOSURE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-bootstrap/tests/runtime_install.rs::ready_marker_waits_for_the_complete_runtime_file_closure
scope: [host, pkg, product]
---

# Неполный runtime не помечается готовым

Bootstrap публикует маркер готовности только после проверки полного набора
файлов установленного runtime.
