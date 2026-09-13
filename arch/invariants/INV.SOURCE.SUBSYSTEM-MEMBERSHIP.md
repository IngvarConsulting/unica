---
id: INV.SOURCE.SUBSYSTEM-MEMBERSHIP
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/meta_info_surface_tests.rs::info_matches_subsystem_memberships_by_address_or_root_descriptor_uuid
scope: [source]
---

# Членство в подсистеме сопоставляет обе логические идентичности

`meta.info` публикует членство текущего объекта только в зарегистрированных
подсистемах и сопоставляет `Content` как по каноническому адресу метаданных, так
и по UUID корневого дескриптора.
