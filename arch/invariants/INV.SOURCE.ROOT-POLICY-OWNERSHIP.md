---
id: INV.SOURCE.ROOT-POLICY-OWNERSHIP
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/platform_xml_owner.rs::container_scoped_versioned_roots_never_become_standalone_owners
scope: [source]
---

# Версионный подчинённый корень не становится владельцем

Версионированный при публикации подчинённый документ не становится
самостоятельным владельцем формата только из-за собственного атрибута
`version`.
