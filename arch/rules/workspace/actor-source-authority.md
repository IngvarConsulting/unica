---
id: INV.APP.ACTOR-AUTHENTICATED-SOURCE-CAPABILITIES
check:
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::duplicate_physical_root_names_are_rejected_as_ambiguous
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::duplicate_source_set_names_with_distinct_roots_are_rejected
  - crates/unica-coder/src/infrastructure/daemon/server.rs::actor_read_source_capability_is_sealed_after_binding
  - crates/unica-coder/src/infrastructure/daemon/server.rs::actor_read_authority_builder_rejects_actor_bound_unsupported_profile
  - crates/unica-coder/src/infrastructure/daemon/server.rs::actor_read_authority_builder_preserves_actor_bound_source_kind
  - crates/unica-coder/src/infrastructure/daemon/server.rs::actor_read_authority_builder_preserves_non_replenishing_deadline
  - crates/unica-coder/src/infrastructure/daemon/server.rs::provider_binding_and_actor_bound_invocation_cannot_substitute_kind_or_profile
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::capabilities_do_not_cross_distinct_actor_instances_with_equal_identity
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::workspace_actor_capabilities_enforce_identity_physical_and_bounded_publication
  - crates/unica-coder/src/infrastructure/daemon/server.rs::hidden_v13_logical_lease_survives_the_handoff_window_and_confirms_once
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::logical_read_publication_lane_wait_honors_existing_cancellation_and_deadline
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::actor_owned_reader_never_follows_a_source_set_remap_after_admission
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::actor_owned_configuration_support_and_home_page_sidecars_are_retained
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::actor_owned_typed_form_reader_never_follows_a_source_set_remap
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::actor_owned_module_reader_never_follows_a_source_set_remap
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::every_typed_reader_remains_on_the_admitted_root_after_source_set_remap
---

# Право чтения и публикации принадлежит выдавшему его актору

Актор связывает операцию с набором исходников, удерживаемым физическим
каталогом, видом, форматом и профилем платформы. Читатель получает эти
сведения из выданного права доступа и не может подменить их параметрами
вызова. Другой экземпляр актора не принимает это право, даже если его
логическая идентичность совпадает.

Читатель Platform XML принимает профиль платформы 8.3.27 с сериализацией
2.20. Право, выданное для неподдерживаемого профиля, не превращается
в право чтения поддерживаемого формата.

Одно имя набора нельзя связать с разными корнями; один физический корень
нельзя объявить под разными именами. Чтение относительно удерживаемого
каталога не следует вложенным ссылкам. Подмена именованного корня
другим каталогом отклоняет результат.

Перед выдачей подготовленного результата актор повторно проверяет экземпляр,
физический корень и ревизию исходников. Проверка проходит в той же очереди,
что и изменения исходников. Ожидание очереди и ревизии ограничено исходным
сроком и отменой: после их наступления результат не выдаётся. Передача
операции в фоновое задание не сбрасывает эти условия.
