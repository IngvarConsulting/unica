---
id: INV.WIRE.SEARCH-SELECTED-ROLE
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::public_role_search_runs_only_selected_provider_and_cannot_use_a_neighbor_success
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::selected_role_search_does_not_start_a_provider_after_parent_cancellation
  - crates/unica-coder/tests/v13_search_integration.rs::canonical_search_is_source_scoped_and_rejects_legacy_call_shape
---

# Указанная роль запускает только выбранный способ поиска

В `unica.search` роль `lexical`, `symbol` или `semantic` выбирает
соответствующего поставщика. Остальные поставщики не запускаются и не
добавляют время ожидания или свои результаты.

Отказ выбранного поставщика не превращается в успех за счёт ответа другого.
Без роли поиск выполняется самой Unica и не запускает внешних поставщиков.
Свод имён `names` не принимает роль.
