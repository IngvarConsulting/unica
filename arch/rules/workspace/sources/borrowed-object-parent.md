---
id: INV.SOURCE.BORROWED-OBJECT-PARENT-ADDRESS
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::borrowing_view_resolves_registered_parent_and_preserves_unresolved_extension_facts
  - crates/unica-coder/src/infrastructure/daemon/server.rs::borrowing_view_rejects_parent_changes_before_final_publication
  - crates/unica-coder/src/infrastructure/daemon/server.rs::borrowing_view_bounds_override_props_for_metadata_and_specialized_readers
---

# Заимствованный объект указывает проверенный адрес родителя

При чтении заимствованного объекта Unica показывает адрес родительского
объекта вместе с набором исходников. По этому адресу можно прочитать именно
родительский объект; одного UUID или совпадения имён для такой ссылки мало.

Если исходники родителя недоступны или соответствие неоднозначно, Unica
сообщает причину и не угадывает адрес. Недоказанная связь не скрывает
остальные сведения об исправном объекте расширения.
