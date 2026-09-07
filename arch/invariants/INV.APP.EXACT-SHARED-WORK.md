---
id: INV.APP.EXACT-SHARED-WORK
status: active
governs: product
decision: DEC.2026-08-24.EXACT-SHARED-DELIVERY-SLICE
check:
  - crates/unica-coder/src/application/shared_work.rs::exact_key_vocabulary_covers_delivery_index_provider_and_runtime
  - crates/unica-coder/src/application/shared_work.rs::typed_provider_and_runtime_keys_reject_weak_identity_and_remain_exact
  - crates/unica-coder/src/application/shared_work.rs::one_producer_serves_many_exact_key_followers_and_fans_out_the_result
  - crates/unica-coder/src/application/shared_work.rs::different_exact_keys_do_not_share_and_failure_is_fanned_out
  - crates/unica-coder/src/application/shared_work.rs::producer_spawn_failure_is_terminal_for_the_leader_and_attached_follower
  - crates/unica-coder/src/application/shared_work.rs::follower_cancellation_does_not_cancel_a_live_owner
  - crates/unica-coder/src/application/shared_work.rs::owner_bound_work_is_cancelled_when_the_last_owner_leaves
  - crates/unica-coder/src/application/shared_work.rs::owner_attach_racing_last_owner_drop_observes_one_retiring_producer
  - crates/unica-coder/src/application/shared_work.rs::cancelled_attempt_retires_before_a_replacement_producer_starts
  - crates/unica-coder/src/application/shared_work.rs::owner_cancellation_cannot_be_lost_between_predicate_check_and_wait
  - crates/unica-coder/src/application/shared_work.rs::terminal_retirement_cannot_remove_an_entry_while_a_new_owner_attaches
  - crates/unica-coder/src/application/shared_work.rs::joining_shared_work_never_waits_with_an_admission_permit
scope: [app]
---

# SharedWork линеаризует exact-key producer и владельцев

В пределах одного владеющего `SharedWork` registry один точный ключ имеет не
более одного живого producer. Followers получают тот же result или failure,
другой ключ не объединяется, уход follower не отменяет живого владельца, а
последний owner-bound владелец отменяет producer. Join не ждёт результата;
attach, cancellation и terminal retirement не открывают окно для
перекрывающегося producer.
