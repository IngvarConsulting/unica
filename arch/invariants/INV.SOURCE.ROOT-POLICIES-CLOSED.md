---
id: INV.SOURCE.ROOT-POLICIES-CLOSED
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/platform_xml_roots.rs::unregistered_roots_have_no_policies
scope: [source]
---

# Неизвестный QName не получает политику корня

QName вне закрытого каталога platform XML не получает ни политику публикации,
ни роль владельца формата и потому не расширяет разрешённый контур.
