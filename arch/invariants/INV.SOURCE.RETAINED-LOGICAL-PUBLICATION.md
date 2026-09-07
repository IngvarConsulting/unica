---
id: INV.SOURCE.RETAINED-LOGICAL-PUBLICATION
status: active
governs: product
decision: DEC.2026-08-25.LOGICAL-READ-CORE-SLICE
check:
  - crates/unica-coder/src/infrastructure/source_revision.rs::review_final_confirmation_rejects_root_replacement_during_retained_scan
  - crates/unica-coder/src/infrastructure/source_revision.rs::review_final_confirmation_rejects_nested_directory_replacement_after_retention
  - crates/unica-coder/src/infrastructure/source_revision.rs::review_final_confirmation_rejects_file_replacement_after_retention
  - crates/unica-coder/src/infrastructure/source_revision.rs::review_final_confirmation_rejects_membership_added_after_enumeration
  - crates/unica-coder/src/infrastructure/source_revision.rs::review_final_confirmation_rejects_in_place_change_after_hash
  - crates/unica-coder/src/infrastructure/source_revision.rs::unsupported_fence_stable_operation_lease_scans_at_admission_and_confirmation
  - crates/unica-coder/src/infrastructure/source_revision.rs::unsupported_fence_reconcile_is_bounded_to_six_passes_when_corpus_never_stabilizes
  - crates/unica-coder/src/infrastructure/daemon/server.rs::hidden_v13_logical_lease_survives_the_handoff_window_and_confirms_once
  - crates/unica-coder/src/infrastructure/daemon/server.rs::review_invalid_logical_address_reaches_typed_bad_value_result
  - crates/unica-coder/src/infrastructure/daemon/server.rs::valid_unknown_source_reaches_typed_provider_unavailable_without_scanning
  - crates/unica-coder/src/infrastructure/daemon/server.rs::zero_fence_view_rejection_accepts_only_the_exact_canonical_envelope
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::logical_read_publication_lane_wait_honors_existing_cancellation_and_deadline
scope: [app, platform, product, source]
---

# Retained logical read публикуется после стабилизации под actor mutation lane

При unsupported platform fence initial capture и final confirmation каждого
выбранного source set выполняют по два равных descriptor-relative passes.
Каждый pass post-order повторно проверяет named identities и membership,
ограничивает entries, file/aggregate bytes и проверяет одну operation deadline
и cancellation. Semantic manifest и отдельная physical identity evidence
должны совпасть; physical identity не меняет byte-compatible semantic digest.
Stable operation выполняет четыре passes на source set. Три bounded attempts
ограничивают capture шестью, operation двенадцатью passes независимо от числа
logical nodes.

Final publication один раз захватывает mutation lane `WorkspaceActor`, под ней
проверяет все actor-issued source fences и выполняет все final retained
confirmations, после чего source I/O нет. Lane wait использует ту же абсолютную
120-секундную logical-read deadline и cancellation, не создавая новый budget.
Malformed View address и valid unknown source не захватывают revision и
возвращают соответственно typed `bad_value` и `provider_unavailable`; zero-fence
publication принимает только такой закрытый typed rejection, а успешный result
без admitted fence fail closed.
