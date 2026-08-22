---
id: INV.SOURCE.ROOT-POLICY-PUBLICATION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::staged_tree_uses_closed_root_registry_and_exact_raw_versions
scope: [source]
---

# Публикация неизвестного корня закрыта

Публикация platform XML с QName вне закрытого каталога отказывает независимо
от наличия у неизвестного корня атрибута `version`; зарегистрированный
версионно независимый корень остаётся допустим только без версии.
