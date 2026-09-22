---
id: INV.SOURCE.ROLE-RIGHT-DEFAULT-STORAGE
check:
  - crates/unica-coder/src/infrastructure/native_operations/role.rs::default_valued_right_is_stored_as_absence_like_the_platform_export
  - crates/unica-coder/src/infrastructure/native_operations/role.rs::removing_the_last_stored_right_drops_the_emptied_object_block
  - crates/unica-coder/src/infrastructure/native_operations/role.rs::emptied_block_with_unknown_children_keeps_the_block_and_sheds_the_right
  - crates/unica-coder/src/infrastructure/native_operations/role.rs::set_for_new_objects_true_mirrors_the_stored_value_rule
---

# Возврат права к умолчанию удаляет его отдельную запись

Для права объекта верхнего уровня писатель учитывает `setForNewObjects`.
При переходе к этому умолчанию он удаляет запись права вместе с её
ограничениями доступа к записям. Опустевший блок объекта удаляется только
при отсутствии неизвестных дочерних узлов.

Если явно записанное значение уже равно запрошенному, писатель сохраняет
исходные байты, даже когда эта запись избыточна. Проверки проходят писатель
`Rights.xml` для профиля 8.3.27/2.20; они не проверяют добавление зависимых
прав, для которого действует отдельное правило.
