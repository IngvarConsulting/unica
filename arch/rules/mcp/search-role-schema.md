---
id: INV.WIRE.SEARCH-PROVIDER-ROLES
check:
  - crates/unica-coder/src/application/v13/tool_catalog.rs::search_publishes_three_provider_roles_and_stays_literal_without_one
---

# Схема поиска сохраняет необязательную роль без умолчания

Аргумент `role` у `unica.search` принимает только `lexical`, `symbol`
и `semantic`. У него нет значения по умолчанию: отсутствие роли не
подменяется значением `lexical` в опубликованной схеме.

Проверка относится к схеме запроса. Выбор и запуск провайдера она не
исполняет.
