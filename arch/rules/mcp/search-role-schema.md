---
id: INV.WIRE.SEARCH-PROVIDER-ROLES
check:
  - crates/unica-coder/src/application/v13/tool_catalog.rs::search_publishes_two_provider_roles_and_stays_literal_without_one
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::retired_lexical_role_is_refused_with_the_search_without_a_role
---

# Схема поиска сохраняет необязательную роль без умолчания

Аргумент `role` у `unica.search` принимает только `symbol` и `semantic`.
У него нет значения по умолчанию: без роли поиск буквальный. Прежнее
значение `lexical` отклоняется как `bad_value` до чтения исходников, с
продолжением тем же запросом без роли; пустого ответа оно не получает.
