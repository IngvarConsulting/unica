---
id: INV.PKG.VERIFIED-ATOMIC-INSTALL
status: active
governs: product
decision: DEC.2026-08-19.ARTIFACT-VERSIONED-CACHE
check: crates/unica-bootstrap/tests/runtime_install.rs::verified_install_publishes_exact_closure_atomically
scope: [host, pkg, product]
---

# Runtime становится готовым только после полной проверенной установки

Bootstrap сверяет архив, состав, суммы и режимы файлов, отклоняет небезопасную
или дрейфующую раскладку и публикует маркер готовности только после полного
набора. Конкурирующие установщики получают одну атомарно опубликованную
поставку.
