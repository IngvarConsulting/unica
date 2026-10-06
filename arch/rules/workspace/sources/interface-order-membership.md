---
id: INV.SOURCE.INTERFACE-ORDER-MEMBERSHIP
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_interface_order_permutes_stored_composition_and_refuses_any_other_without_writing
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_interface_order_plan_is_refused_after_the_composition_changes
  - crates/unica-coder/src/infrastructure/native_operations/interface.rs::order_membership_accepts_a_permutation_or_a_first_order_only
---

# Перестановка интерфейса сохраняет состав уже заданного порядка

`commandOrder.set`, `groupOrder.set` и `subsystemOrder.set` переставляют
элементы уже заданного порядка без изменения его состава. Если сохранён
порядок A, B, C, список B, A отклоняется без записи: пропуск C не означает
удаление его настройки. Повтор элемента или подмена состава также отклоняются.

Для команд сравнение относится к выбранной группе. Первоначальное задание
порядка, когда его ещё нет, — отдельный случай; это правило его не запрещает.
