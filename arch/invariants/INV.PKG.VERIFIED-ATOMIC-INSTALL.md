---
id: INV.PKG.VERIFIED-ATOMIC-INSTALL
status: active
governs: product
decision: DEC.2026-08-19.ARTIFACT-VERSIONED-CACHE
check: crates/unica-bootstrap/tests/runtime_install.rs::verified_install_publishes_exact_closure_atomically
scope: [host, pkg, product]
---

# Runtime становится готовым после проверки ассета и файлового замыкания

Bootstrap сверяет SHA-256 ассета, точный состав путей и SHA-256 каждого файла,
отклоняет ссылки, обход staging, пропущенную, лишнюю или дрейфующую раскладку и
публикует маркер готовности только после полного набора. Конкурирующие
установщики получают одну атомарно опубликованную поставку.
