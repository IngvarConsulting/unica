---
id: INV.SOURCE.FORM-ELEMENT-MOVE-PRESERVATION
check:
  - crates/unica-coder/src/infrastructure/native_operations/form_move_tests.rs::compact_subtree_moves_without_losing_adjacent_siblings_or_references
  - crates/unica-coder/src/infrastructure/native_operations/form_move_tests.rs::crlf_comments_and_meaningful_text_survive_platform_reindentation
  - crates/unica-coder/src/infrastructure/native_operations/form_move_tests.rs::platform_indentation_follows_destination_without_changing_leaf_text
  - crates/unica-coder/src/infrastructure/native_operations/form_move_tests.rs::inherited_qname_namespaces_survive_conflicting_destination_bindings
  - crates/unica-coder/src/infrastructure/native_operations/form_move_tests.rs::prefixed_form_preserves_inherited_default_namespace_for_qname_values
  - crates/unica-coder/src/infrastructure/native_operations/form_move_tests.rs::moving_from_no_default_namespace_does_not_rebind_unqualified_extensions
  - crates/unica-coder/src/infrastructure/native_operations/form_move_tests.rs::exact_placement_is_a_byte_preserving_noop
  - crates/unica-coder/src/infrastructure/native_operations/form_move_tests.rs::moving_the_last_child_removes_empty_child_items_and_its_whitespace
  - crates/unica-coder/src/infrastructure/native_operations/form_move_tests.rs::parent_moved_after_its_last_child_gets_destination_indentation
  - crates/unica-coder/src/infrastructure/native_operations/form_move_tests.rs::base_form_and_companions_cannot_be_independently_moved
  - crates/unica-coder/src/infrastructure/native_operations/form_move_tests.rs::placement_matrix_matches_observed_platform_kinds_bindings_and_parents
  - crates/unica-coder/src/infrastructure/daemon/form_move_tests.rs::canonical_form_move_composes_tabs_events_and_preserves_atomic_noops
---

# Перенос сохраняет содержимое элемента формы

`item.move` перемещает существующее XML-поддерево целиком, сохраняя
идентификаторы, свойства, события, привязки, вложенные элементы и companions.
Соседние элементы и сохранённая базовая форма `BaseForm` не переписываются.
Companions нельзя переносить отдельно от владельца.

Перенос сохраняет текстовые значения и смысл пространств имён, включая
QName в значениях атрибутов и текста. Для этого на переносимом узле могут
появиться необходимые объявления пространств имён. Повтор текущего положения
сохраняет исходные байты формы.

Операция не пересоздаёт поддерево из описания элемента и не преобразует
его тип или привязки для нового родителя. Эти гарантии сохранности не
означают, что любой контейнер допускает любой элемент.
