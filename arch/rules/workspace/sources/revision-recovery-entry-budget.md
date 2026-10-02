---
id: INV.SOURCE.RETAINED-APPLY-TRANSIENT-ENTRY-AUTHORITY
check:
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::actor_revision_recovery_identity_swap_is_rejected_before_revision_install
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::actor_revision_recovery_hard_link_alias_is_never_discounted_or_restored
---

# Применение сохраняет подлинные копии для отката

После записи исходников `apply` проверяет собственные живые копии прежних
файлов по журналу текущей операции. Принадлежность подтверждается точным
каталогом, именем и физической идентичностью; у файла должна быть одна
жёсткая ссылка. Проверка не обходит остальное дерево исходников.

Подмена копии для отката или появление второй жёсткой ссылки запрещают
успешное завершение и восстановление из подменённой копии. Сохранённые
исходные байты и чужие файлы не удаляются ради завершения операции.
Похожее имя само по себе не даёт права использовать или удалять файл.
