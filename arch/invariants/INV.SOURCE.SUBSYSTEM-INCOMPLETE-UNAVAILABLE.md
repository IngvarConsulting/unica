---
id: INV.SOURCE.SUBSYSTEM-INCOMPLETE-UNAVAILABLE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/meta_info_surface_tests.rs::subsystem_membership_evidence_contract_is_complete
scope: [source]
---

# Неполное свидетельство подсистемы не становится пустой проекцией

Недопустимый зарегистрированный элемент, ошибка, отмена или неполное чтение не
публикуются как пустая доказанная проекция членства; незарегистрированный файл
также не создаёт членство.
