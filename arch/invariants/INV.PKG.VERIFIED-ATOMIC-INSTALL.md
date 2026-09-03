---
id: INV.PKG.VERIFIED-ATOMIC-INSTALL
status: active
governs: product
decision: DEC.2026-08-19.ARTIFACT-VERSIONED-CACHE
check:
  - crates/unica-bootstrap/tests/runtime_install.rs::valid_archive_is_published_with_a_ready_marker
  - crates/unica-bootstrap/tests/runtime_install.rs::ready_marker_waits_for_the_complete_runtime_file_closure
  - crates/unica-bootstrap/tests/runtime_install.rs::corrupt_archive_never_publishes_a_ready_runtime
  - crates/unica-bootstrap/tests/runtime_install.rs::install_closure_rejects_unsafe_or_drifted_archives_without_ready
  - crates/unica-bootstrap/tests/runtime_install.rs::concurrent_installers_download_and_publish_once
scope: [host, pkg, product]
---

# Runtime становится готовым после проверки ассета и файлового замыкания

Bootstrap сверяет SHA-256 ассета, точный состав путей и SHA-256 каждого файла,
отклоняет ссылки, обход staging, пропущенную, лишнюю или дрейфующую раскладку и
публикует маркер готовности только после полного набора. Конкурирующие
установщики получают одну атомарно опубликованную поставку.
