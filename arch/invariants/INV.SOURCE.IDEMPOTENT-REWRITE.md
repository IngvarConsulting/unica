---
id: INV.SOURCE.IDEMPOTENT-REWRITE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/native_operations/code.rs::applied_patch_returns_typed_data_and_repeated_apply_is_noop
scope: [source]
---

# Повторная идентичная мутация ничего не пишет

Повторный идентичный `unica.code.patch` распознаётся до записи как семантически
пустой: хеш до совпадает с хешем после, diff и диапазоны пусты, а байты файла
остаются образом первого применения.
