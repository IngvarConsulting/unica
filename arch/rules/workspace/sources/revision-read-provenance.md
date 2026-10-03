---
id: INV.SOURCE.REVISION-READ-PROVENANCE
check:
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::retained_binding_rejects_a_same_path_directory_replacement
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::retained_binding_rejects_a_root_replaced_by_another_source_set_link
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::prepared_apply_root_and_actor_capabilities_cannot_be_redirected_or_replayed
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::prepared_apply_dry_run_rejects_root_replacement_before_result
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::prepared_apply_root_replacement_during_commit_rolls_back_before_result
---

# Чтение и применение связаны с одним физическим источником

Привязка источника удерживает физический корень, допущенный актором.
Повторное использование того же пути другим каталогом или символической
ссылкой не переносит выданные полномочия на новый источник.

Входы плана, предпросмотр и публикация относятся к этому удерживаемому
корню. Подмена корня до возврата результата даёт отказ; при подмене
во время публикации уже выполненные изменения откатываются. План нельзя
передать другому актору или корню, даже если логические адреса совпадают.

Эта гарантия не требует общей ревизии содержимого дерева. Проверки подмены
открытого каталога выполняются на ОС, которые её допускают.
