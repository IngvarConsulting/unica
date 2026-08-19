---
id: INV.SOURCE.ATOMIC-PUBLISH
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs::precommit_failpoints_preserve_target_and_remove_stage
scope: [source]
---

# Мутация источника публикуется атомарно или не публикуется

Изменение источника доходит до диска целиком и после проверки, повторная идентичная мутация
ничего не пишет, а неудавшийся откат виден как ошибка целостности, а не как тишина.
