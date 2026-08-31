---
id: INV.APP.EXACT-SHARED-WORK
status: active
governs: product
decision: DEC.2026-08-24.EXACT-SHARED-DELIVERY-SLICE
check: crates/unica-coder/src/application/shared_work.rs::exact_shared_work_keys_fanout_cancellation_and_retirement_are_one_contract
scope: [app]
---

# SharedWork линеаризует exact-key producer и владельцев

В пределах одного владеющего `SharedWork` registry один точный ключ имеет не
более одного живого producer. Followers получают тот же result или failure,
другой ключ не объединяется, уход follower не отменяет живого владельца, а
последний owner-bound владелец отменяет producer. Join не ждёт результата;
attach, cancellation и terminal retirement не открывают окно для
перекрывающегося producer.
