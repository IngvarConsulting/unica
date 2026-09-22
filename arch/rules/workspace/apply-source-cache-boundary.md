---
id: INV.APP.RETAINED-APPLY-CLOSED-PARTICIPANTS
check:
  - crates/unica-coder/src/infrastructure/native_operations/apply.rs::retained_transaction_roles_require_explicit_roots_and_cache_authority
  - crates/unica-coder/src/infrastructure/native_operations/apply.rs::arbitrary_second_transaction_cannot_masquerade_as_actor_cache_authority
  - crates/unica-coder/src/infrastructure/native_operations/apply.rs::closed_transaction_rejects_physical_alias_and_second_cache_participant
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_admission_rejects_source_inside_cache
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::workspace_root_source_allows_exact_generated_cache_descendant
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::workspace_root_source_and_missing_cache_publish_through_disjoint_shared_anchor
---

# Применение связывает исходники и кеш с одним правом записи

Подготовленный `apply` содержит ровно два участника: исходники (`Source`)
и кеш проекта (`WorkspaceCache`). Оба получают право записи при одном допуске
актора к операции. Их удерживаемые корни указываются явно даже для пакета
без изменений. Чужое право записи, третий участник или произвольный каталог
вместо назначенного актором кеша отклоняются.

Исходники не могут совпадать с кешем или находиться внутри него. Участники
не записывают одни и те же пути, а `Source` не может изменять путь с компонентом
`.build` на любом уровне.

Когда исходники занимают корень проекта, кеш допускается в его точном
подкаталоге `.build/unica`. Отсутствующий каталог кеша можно создать через
удерживаемого общего предка.
