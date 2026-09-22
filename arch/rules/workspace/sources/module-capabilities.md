---
id: INV.SOURCE.PLATFORM-CAPABILITY-EXISTENCE
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::module_capability_parents_expose_canonical_module_collections
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::production_authorities_reach_all_profile_module_capabilities_from_real_parent_inventories
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::websocket_client_source_view_is_an_explicit_provider_gap
  - crates/unica-coder/src/infrastructure/logical_tree.rs::platform_capability_controls_logical_existence_without_filesystem_evidence
  - crates/unica-coder/src/domain/platform_profile.rs::service_bot_websocket_and_absent_grpc_capabilities_are_not_conflated
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::missing_owner_module_branch_is_not_invented_but_registered_owner_without_bsl_is_kept
---

# Состав модулей определяется профилем платформы

Для платформы 8.3.27 допустимые модули определяются видом владельца
и ролью модуля. У зарегистрированного владельца допустимый модуль
остаётся в логическом дереве, даже если его BSL-файл ещё не выгружен.
Несуществующий владелец, неизвестная роль или недопустимая пара вида
и роли дают `not_found`.

HTTP, SOAP и сервис интеграции имеют разные роли. Профиль включает `Bot`
и `WebSocketClient`, но не включает `RecordManager`, универсальную роль
`Service` или gRPC. Для внешних обработок и отчётов поддерживаются роли
модулей объекта, формы и команды.

Владелец объявляет одну ветвь `Module`; её счёт соответствует уникальным
допустимым ролям. Через родительскую навигацию достижимы положительные
возможности профиля конфигурации, внешней обработки и внешнего отчёта.
Проверка проходит 25 предусмотренных фикстурой адресов через реальные
полномочия чтения, а не только таблицу допустимых пар.

Логические узлы `WebSocketClient` видимы, но чтение исходников его модуля
возвращает `provider_unavailable`, пока раскладка не доказана. Видимость
узла не обещает реализованное чтение его содержимого.
