---
id: INV.SOURCE.ATOMIC-PUBLISH
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs::precommit_failpoints_preserve_target_and_remove_stage
scope: [source]
---

# Ошибка до commit не меняет цель публикации

Сбой на любой проверяемой фазе до commit сохраняет исходные байты цели и
удаляет staging-файл публикации.
