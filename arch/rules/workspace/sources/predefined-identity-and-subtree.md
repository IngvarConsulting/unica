---
id: INV.SOURCE.PREDEFINED-IDENTITY-AND-SUBTREE
check:
  - crates/unica-coder/src/infrastructure/daemon/metadata_can_tests/predefined_nested_identity.rs::canonical_predefined_apply_adds_at_root_and_updates_and_removes_nested_identity
  - crates/unica-coder/src/infrastructure/daemon/metadata_can_tests/predefined_nested_identity.rs::canonical_predefined_apply_refuses_duplicate_source_uuids_without_mutation
  - crates/unica-coder/src/infrastructure/native_operations/meta/predefined.rs::planner_creates_bom_file_and_second_equivalent_add_is_noop
  - crates/unica-coder/src/infrastructure/native_operations/meta/predefined.rs::add_equivalence_does_not_ignore_an_existing_child_subtree
  - crates/unica-coder/src/infrastructure/native_operations/meta/predefined.rs::remove_of_a_parent_and_its_descendant_is_order_independent
  - crates/unica-coder/src/infrastructure/native_operations/meta/predefined.rs::remove_effect_reports_the_entire_subtree_in_document_order
---

# Изменения предопределённых элементов учитывают UUID и поддерево

Добавление создаёт только корневой элемент. Изменение и удаление выбирают
элемент по UUID на любой глубине. Повтор UUID в исходном документе
отклоняется до изменения.

Повторное добавление по UUID не меняет эквивалентный элемент и отказывает
при ином содержимом. Наличие дочерних элементов учитывается при сравнении.
Удаление родителя удаляет его поддерево; описание эффекта перечисляет
родителя и потомков в документном порядке. Соседние элементы сохраняются.

Проверки проходят писатель и планировщик `Predefined.xml`, а также текущий
канонический путь предпросмотра и исполнения по токену. Именованный адрес
выбирает вложенный элемент, результат сохраняет его UUID и соседей.
