---
id: INV.PKG.INSTALL-TOOL-CLOSURE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-bootstrap/tests/runtime_install.rs::install_closure_rejects_unsafe_or_drifted_archives_without_ready
scope: [host, pkg, product]
---

# Неполный runtime не помечается готовым

Bootstrap публикует маркер готовности только после проверки полного набора
файлов установленного runtime.
