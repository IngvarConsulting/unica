---
id: INV.PKG.RETENTION-BY-ARTIFACT
status: active
governs: product
decision: DEC.2026-08-19.RETENTION-BY-ARTIFACT
check: crates/unica-bootstrap/tests/runtime_install.rs::collecting_keeps_the_newest_versions_of_each_artifact
scope: [pkg]
---

# Сборка мусора считает версии по артефакту

У каждого артефакта в кеше остаются две свежайшие версии, остальные удаляются.
Служебные области кеша — блокировки и незавершённые транзакции — артефактами не
считаются и сборкой не затрагиваются.
