---
id: INV.SOURCE.ROOT-POLICIES
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/platform_xml_roots.rs::every_root_has_independently_recorded_publication_and_owner_policies
scope: [source]
---

# Публикация и владение форматом задаются независимо

Один каталог точных QName platform XML явно и независимо задаёт для каждого
зарегистрированного корня политику допустимой версии при публикации и роль
документа в разрешении владельца формата.
