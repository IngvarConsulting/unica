---
id: INV.SOURCE.INSERTED-METHOD-SPACING
check:
  - crates/unica-coder/tests/platform/code_patch_composition.rs::inserted_method_spacing_covers_positions_eol_eof_and_repeat
  - crates/unica-coder/tests/platform/code_patch_composition.rs::inserted_method_spacing_preserves_empty_bom_payload_and_mixed_bytes
  - crates/unica-coder/tests/platform/code_patch_composition.rs::inserted_method_spacing_recognizes_an_existing_method_without_separators
  - crates/unica-coder/tests/platform/code_patch_composition.rs::insertion_around_a_method_keeps_its_head_together_and_respects_its_preprocessor
  - crates/unica-coder/tests/platform/code_patch_composition.rs::a_comment_inserted_before_a_method_joins_its_head_and_repeats_as_noop
---

# Вставленный BSL-метод отделяется от соседних пустой строкой

При добавлении метода Unica оставляет пустую строку между ним и соседними
методами. Вставка должна давать синтаксически корректный BSL и сохранять
принадлежность комментариев, аннотаций и директив своим методам.

Это локальное оформление вставки, а не переформатирование модуля.
[Сохранение нетронутых байтов](code-patch-newlines.md) остаётся обязательным.
Наличие перевода строки в самом конце файла эта запись не определяет.
