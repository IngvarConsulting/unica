---
id: INV.APP.EXACT-LONG-WORK-OWNERSHIP
check:
  - crates/unica-coder/src/application/shared_work.rs::exact_key_vocabulary_covers_delivery_index_provider_and_runtime
  - crates/unica-coder/src/application/shared_work.rs::typed_provider_and_runtime_keys_reject_weak_identity_and_remain_exact
  - crates/unica-coder/src/infrastructure/daemon/server.rs::daemon_long_work_capabilities_handoff_before_wait_and_preserve_exact_ownership
  - crates/unica-coder/src/infrastructure/daemon/server.rs::daemon_index_work_separates_worktrees_and_build_generations
  - crates/unica-coder/src/infrastructure/daemon/server.rs::daemon_long_work_rejects_replaced_actor_root_before_reuse_or_publication
gap: https://github.com/IngvarConsulting/unica/issues/988
---

# Совместная подготовка сервиса не объединяет данные рабочих пространств

Координатор индекса объединяет работу только для одного актора, набора
исходников, поколения сборки, провайдера и профиля. Другой worktree
или новое поколение запускает отдельную подготовку. Поколение не доказывает
актуальность исходников; она сообщается отдельно. Подмена удержанного корня
отклоняет результат.

Запуск `ProviderHost` можно разделить между рабочими пространствами
при совпадении движка, целевой платформы и набора возможностей.
Чтение исходников и результат каждого потребителя остаются связаны
с его собственным актором.

Ожидание общей подготовки начинается после передачи вызова в фоновое
задание. Проверки исполняют настоящий демон с внедрённым сервисом;
они защищают разделение работы и данных на границе координаторов.
Подключение к уже работающему runtime имеет отдельную
[проверку права на ресурс](active-lease-join.md).

Присоединение к общей подготовке не обходит блокировку построителя индекса.
Сквозная проверка этой связи остаётся в `gap`: проверки координатора
не подтверждают получение файловой блокировки.
