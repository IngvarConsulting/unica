---
id: INV.APP.META-OBSERVATION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/meta_info_surface_tests.rs::info_preserves_local_structure_when_child_resource_evidence_is_unavailable
scope: [app]
---

# Ошибка дочернего ресурса не стирает исправное наблюдение

Чтение метаданных сохраняет доказанные имя и коллекцию владельца, когда
неожиданный файл делает доказательство дочернего ресурса недоступным.
