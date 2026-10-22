//! Causal checks for the compact model and explicit aggregate/body boundaries.
use super::hydration::*;
use super::test_support::{Archive, Fixture};
use super::*;
use unica_execution_contract::invocation::DomainResult;
use unica_execution_contract::storage::{PayloadChange, RecoveryBuilder};
use unica_execution_contract::{FailureReason, OutcomeKind, Tool};

fn key() -> Key {
    Key::new(
        InvocationId::new(),
        TaskId::new(),
        [1; 32],
        Tool::View,
        [2; 32],
        [3; 32],
    )
}
fn owner() -> OwnerCapability {
    OwnerCapability::new([4; 32])
}
fn epoch(n: u8) -> AuthorityEpoch {
    AuthorityEpoch::new([n; 32])
}
fn authority() -> ReadAuthority {
    ReadAuthority::new([7; 32], 1, 1)
}
fn window() -> ObservationWindow {
    ObservationWindow::new(10, 7_000).unwrap()
}
fn only(mut effects: Vec<Effect>) -> CommitRequest {
    assert_eq!(effects.len(), 1);
    match effects.remove(0) {
        Effect::Commit(r) => r,
        other => panic!("{other:?}"),
    }
}
fn settle(c: &mut Fixture, effects: Vec<Effect>) -> Vec<Effect> {
    let mut queue: VecDeque<_> = effects.into();
    let mut result = Vec::new();
    while let Some(effect) = queue.pop_front() {
        match effect {
            Effect::Commit(r) => queue.extend(c.commit(r).unwrap()),
            other => result.push(other),
        }
    }
    result
}
fn accept(c: &mut Fixture, k: &Key) -> AcceptedPermit {
    let pending = c.submit(k.clone(), &owner(), window()).unwrap();
    match settle(c, pending).remove(0) {
        Effect::Accepted(p) => p,
        other => panic!("{other:?}"),
    }
}
fn start(c: &mut Fixture, k: &Key) -> RunPermit {
    let accepted = accept(c, k);
    let pending = c.begin(accepted, 20, Some(authority())).unwrap();
    match settle(c, pending).remove(0) {
        Effect::Run(p) => p,
        other => panic!("{other:?}"),
    }
}
fn attach(c: &mut Fixture, k: &Key, alias: Key) -> Membership {
    let pending = c
        .attach(k, JoinEvidence::new(alias, authority(), 7_010))
        .unwrap();
    match settle(c, pending).remove(0) {
        Effect::Attached(m) => m,
        other => panic!("{other:?}"),
    }
}
fn publish(c: &mut Fixture, run: RunPermit, text: &str) -> CanonicalOutcome {
    let outcome = CanonicalOutcome::completed(DomainResult::success(text));
    let pending = c.report(run.finish(outcome.clone())).unwrap();
    settle(c, pending);
    outcome
}
fn load(c: &Fixture, r: ReadRequest) -> ReadPublication {
    let key = r.key().clone();
    let completion = r.loaded(c.store(&key)).unwrap();
    c.complete_read(completion).unwrap()
}

#[test]
fn aggregate_decode_rejects_noncanonical_missing_mismatched_and_impossible_bodies() {
    let mut c = Fixture::new(epoch(1));
    let k = key();
    let run = start(&mut c, &k);
    publish(&mut c, run, "exact body");
    let valid = serde_json::to_value(c.store(&k)).unwrap();
    assert!(StoredRecord::decode(&serde_json::to_vec(&valid).unwrap()).is_ok());
    let mut corruptions = Vec::new();
    let mut v = valid.clone();
    v["record"]["begun"]["run"]["foreign_semantic_field"] = 1.into();
    corruptions.push(v);
    for path in [
        "/record/producer/kind/Completed",
        "/producer/kind/Completed",
    ] {
        let mut v = valid.clone();
        v.pointer_mut(path).unwrap()["foreign_semantic_field"] = 1.into();
        corruptions.push(v);
    }
    let mut v = valid.clone();
    v["producer"] = serde_json::Value::Null;
    corruptions.push(v);
    let mut v = valid.clone();
    v["producer"]["bytes"][0] = 32.into();
    corruptions.push(v);
    let mut v = valid.clone();
    v["record"]["producer"]["length"] = 1.into();
    corruptions.push(v);
    let mut v = valid.clone();
    v["record"]["producer"]["kind"] = serde_json::json!({"Completed":{"ok":false}});
    corruptions.push(v);
    let mut v = valid.clone();
    v["producer"]["digest"] = serde_json::json!(vec![0; 32]);
    corruptions.push(v);
    let mut v = valid.clone();
    v["record"]["begun"] = serde_json::Value::Null;
    corruptions.push(v);
    let mut v = valid.clone();
    v["record"]["primary_override"] = v["record"]["producer"].clone();
    v["primary_override"] = v["producer"].clone();
    corruptions.push(v);
    for (i, value) in corruptions.into_iter().enumerate() {
        assert!(
            StoredRecord::decode(&serde_json::to_vec(&value).unwrap()).is_err(),
            "case {i}"
        );
    }
    // Recovery creates the permitted unbegun Interrupted through production transitions.
    let mut waiting = Fixture::new(epoch(2));
    let interrupted_key = key();
    accept(&mut waiting, &interrupted_key);
    let (mut recovered, pending) = Fixture::restore(waiting.catalog().unwrap(), epoch(3)).unwrap();
    settle(&mut recovered, pending);
    let interrupted = CanonicalOutcome::failed(FailureReason::Interrupted);
    let pending = recovered
        .acknowledge(&interrupted_key, &owner(), interrupted.digest(), 100)
        .unwrap();
    settle(&mut recovered, pending);
    assert!(recovered.store(&interrupted_key).record().compacted());
    let mut compact = serde_json::to_value(recovered.store(&interrupted_key)).unwrap();
    compact["record"]["producer"]["kind"] = serde_json::json!({"Completed":{"ok":true}});
    assert!(StoredRecord::decode(&serde_json::to_vec(&compact).unwrap()).is_err());
}

#[test]
fn primary_and_alias_hydration_keep_distinct_roles_through_ack_and_reopen() {
    let mut c = Fixture::new(epoch(1));
    let k = key();
    let run = start(&mut c, &k);
    let a = attach(&mut c, &k, key());
    let b = attach(&mut c, &k, key());
    let pending = c.cancel(&k).unwrap();
    settle(&mut c, pending);
    let cancelled = CanonicalOutcome::cancelled();
    let pending = c
        .acknowledge(&k, &owner(), cancelled.digest(), 100)
        .unwrap();
    settle(&mut c, pending);
    let factual = publish(&mut c, run, "successful producer, personal cancellation");
    assert!(!c.inspect(&k).unwrap().public_task());
    let ReadPublication::Task(primary) = load(&c, c.prepare_task(&k).unwrap()) else {
        panic!()
    };
    assert_eq!(primary.key(), &k);
    assert_eq!(primary.outcome(), &cancelled);
    for member in [a, b] {
        let ReadPublication::Holder(alias) = load(&c, c.prepare_holder(&k, member).unwrap()) else {
            panic!()
        };
        assert_eq!(alias.membership(), member);
        assert_eq!(alias.key(), &k);
        assert_eq!(alias.outcome(), &factual);
        assert_ne!(alias.membership(), if member == a { b } else { a });
    }
    assert!(matches!(
        c.prepare_direct(&k, &owner()),
        Err(Error::AlreadyAcknowledged)
    ));
    let (mut reopened, pending) = Fixture::restore(
        Archive::decode(&c.catalog().unwrap().encode()).unwrap(),
        epoch(2),
    )
    .unwrap();
    settle(&mut reopened, pending);
    let ReadPublication::Task(primary) = load(&reopened, reopened.prepare_task(&k).unwrap()) else {
        panic!()
    };
    assert_eq!(primary.outcome(), &cancelled);
    assert!(matches!(
        reopened.prepare_holder(&k, a),
        Err(Error::StaleMembership)
    ));
    assert_eq!(reopened.store(&k).producer(), Some(&factual));
}

#[test]
fn delayed_reads_revalidate_ack_compaction_epoch_failure_and_membership() {
    for public in [true, false] {
        let mut c = Fixture::new(epoch(1));
        let k = key();
        let run = start(&mut c, &k);
        let result = publish(&mut c, run, "read race");
        if public {
            let pending = c.expose_task(&k).unwrap();
            settle(&mut c, pending);
        }
        let earlier = c.direct(&k, &owner()).unwrap();
        let delayed = c
            .prepare_direct(&k, &owner())
            .unwrap()
            .loaded(c.store(&k))
            .unwrap();
        let task = c.prepare_task(&k).unwrap().loaded(c.store(&k)).unwrap();
        let pending = c.acknowledge(&k, &owner(), result.digest(), 100).unwrap();
        settle(&mut c, pending);
        let task_result = c.complete_read(task);
        assert!(
            matches!(&task_result, Err(Error::StaleCompletion)),
            "{task_result:?}"
        );
        assert!(matches!(
            c.complete_read(delayed),
            Err(Error::StaleCompletion)
        ));
        if public {
            assert!(c.prepare_task(&k).is_ok());
        } else {
            assert!(matches!(c.prepare_task(&k), Err(Error::Compacted)));
        }
        let mut choice = DeliveryChoice::new(k);
        choice.build_direct(earlier).unwrap();
        assert_eq!(choice.direct(), Some(&result));
    }
    let mut c = Fixture::new(epoch(1));
    let k = key();
    let run = start(&mut c, &k);
    let old_alias = key();
    let old = attach(&mut c, &k, old_alias.clone());
    publish(&mut c, run, "membership result");
    let delayed = c
        .prepare_holder(&k, old)
        .unwrap()
        .loaded(c.store(&k))
        .unwrap();
    let pending = c.retire_holder(&k, old).unwrap();
    settle(&mut c, pending);
    let other = key();
    let _running = start(&mut c, &other);
    let current = attach(&mut c, &other, old_alias);
    assert_ne!(old, current);
    assert!(matches!(
        c.complete_read(delayed),
        Err(Error::StaleCompletion)
    ));
    assert!(matches!(
        c.prepare_holder(&k, current),
        Err(Error::StaleMembership)
    ));
    let old_epoch = c.prepare_task(&k).unwrap().loaded(c.store(&k)).unwrap();
    let (mut reopened, pending) = Fixture::restore(c.catalog().unwrap(), epoch(2)).unwrap();
    settle(&mut reopened, pending);
    assert!(matches!(
        reopened.complete_read(old_epoch),
        Err(Error::StaleCompletion)
    ));
    let delayed = c.prepare_task(&k).unwrap().loaded(c.store(&k)).unwrap();
    let pending = only(c.expose_task(&k).unwrap());
    c.complete(pending.unknown()).unwrap();
    assert!(matches!(
        c.complete_read(delayed),
        Err(Error::StoreUnavailable)
    ));
}

#[test]
fn full_prior_fingerprint_and_keep_set_drop_are_required_for_assembly() {
    let mut c = Fixture::new(epoch(1));
    let k = key();
    let run = start(&mut c, &k);
    let factual = publish(&mut c, run, "keep exact bytes");
    let before = c.store(&k).clone();
    let request = only(c.expose_task(&k).unwrap());
    assert!(matches!(request.payloads().producer(), PayloadChange::Keep));
    let assembled = request.assemble(Some(&before)).unwrap();
    assert_eq!(assembled.producer(), Some(&factual));
    let mut different = serde_json::to_value(&before).unwrap();
    different["record"]["public_task"] = true.into();
    let different = StoredRecord::decode(&serde_json::to_vec(&different).unwrap()).unwrap();
    assert_eq!(different.record().revision(), before.record().revision());
    assert_ne!(
        different.record().fingerprint(),
        before.record().fingerprint()
    );
    assert!(matches!(
        request.assemble(Some(&different)),
        Err(Error::InvalidRecord)
    ));
    c.commit(request).unwrap();
    let mut compact = Fixture::new(epoch(2));
    let k = key();
    let run = start(&mut compact, &k);
    let result = CanonicalOutcome::completed(DomainResult::success("install then release"));
    let request = only(compact.report(run.finish(result.clone())).unwrap());
    assert!(matches!(request.payloads().producer(), PayloadChange::Set(body) if body == &result));
    compact.commit(request).unwrap();
    let request = only(
        compact
            .acknowledge(&k, &owner(), result.digest(), 99)
            .unwrap(),
    );
    assert!(matches!(request.payloads().producer(), PayloadChange::Drop));
    let assembled = compact.assemble(&request);
    assert!(assembled.producer().is_none());
    assert_eq!(
        assembled.record().producer().unwrap().kind(),
        OutcomeKind::Completed { ok: true }
    );
    compact.commit(request).unwrap();
}

#[test]
fn streamed_recovery_rejects_last_row_corruption_and_both_global_id_collisions() {
    let mut c = Fixture::new(epoch(1));
    let k = key();
    accept(&mut c, &k);
    let bytes = c.store(&k).encode();
    let mut valid = RecoveryBuilder::new();
    valid.push_encoded(&bytes).unwrap();
    let (mut restored, pending) =
        ExecutionCoordinator::restore(valid.finish().unwrap(), epoch(2)).unwrap();
    assert!(matches!(
        restored.lookup_task(k.task_id()),
        Err(Error::RecoveryRequired)
    ));
    let request = only(pending);
    let stored = request.assemble(Some(c.store(&k))).unwrap();
    restored.complete(request.committed()).unwrap();
    assert!(restored.lookup_task(k.task_id()).is_ok());
    assert!(stored.primary().is_some());
    let mut corrupt = RecoveryBuilder::new();
    corrupt.push_encoded(&bytes).unwrap();
    assert!(matches!(
        corrupt.push_encoded(b"{bad last row"),
        Err(Error::InvalidRecord)
    ));
    assert!(corrupt.push_encoded(&bytes).is_err());
    assert!(corrupt.finish().is_err());
    for invocation_collision in [false, true] {
        let other_key = Key::new(
            if invocation_collision {
                k.invocation_id()
            } else {
                InvocationId::new()
            },
            if invocation_collision {
                TaskId::new()
            } else {
                k.task_id()
            },
            [1; 32],
            Tool::View,
            [8; 32],
            [3; 32],
        );
        let mut other = Fixture::new(epoch(3));
        accept(&mut other, &other_key);
        let mut builder = RecoveryBuilder::new();
        builder.push(c.store(&k).clone()).unwrap();
        assert!(matches!(
            builder.push(other.store(&other_key).clone()),
            Err(Error::IdentityCollision)
        ));
        assert!(matches!(builder.finish(), Err(Error::IdentityCollision)));
    }
}

#[test]
fn confirmed_metadata_releases_distinct_bodies_while_active_rights_pin_them() {
    let mut c = ExecutionCoordinator::new(epoch(1));
    let mut encoded = Vec::new();
    for i in 0..32 {
        let k = key();
        let request = only(c.submit(k.clone(), &owner(), window()).unwrap());
        let accepted_store = request.assemble(None).unwrap();
        let Effect::Accepted(accepted) = c.complete(request.committed()).unwrap().remove(0) else {
            panic!()
        };
        let request = only(c.begin(accepted, 20, Some(authority())).unwrap());
        let begun_store = request.assemble(Some(&accepted_store)).unwrap();
        let Effect::Run(run) = c.complete(request.committed()).unwrap().remove(0) else {
            panic!()
        };
        let body = CanonicalOutcome::completed(DomainResult::success(format!(
            "{i}:{}",
            "x".repeat(16_384)
        )));
        let weak = body.weak_bytes();
        let request = only(c.report(run.finish(body)).unwrap());
        assert!(
            weak.upgrade().is_some(),
            "pending factual outcome pins its bytes"
        );
        let aggregate = request.assemble(Some(&begun_store)).unwrap();
        encoded.push(aggregate.encode());
        drop(aggregate);
        c.complete(request.committed()).unwrap();
        assert!(
            weak.upgrade().is_none(),
            "confirmed coordinator retained payload {i}"
        );
    }
    assert_eq!(c.snapshot().len(), 32);
    let stored = StoredRecord::decode(&encoded[0]).unwrap();
    let k = stored.record().key().clone();
    let weak = stored.primary().unwrap().weak_bytes();
    let completion = c
        .prepare_direct(&k, &owner())
        .unwrap()
        .loaded(&stored)
        .unwrap();
    drop(stored);
    assert!(weak.upgrade().is_some(), "loaded read pins bytes");
    let ReadPublication::Direct(permit) = c.complete_read(completion).unwrap() else {
        panic!()
    };
    assert!(weak.upgrade().is_some(), "publication pins bytes");
    drop(permit);
    assert!(weak.upgrade().is_none());
    let mut builder = RecoveryBuilder::new();
    for bytes in encoded {
        let stored = StoredRecord::decode(&bytes).unwrap();
        let weak = stored.primary().unwrap().weak_bytes();
        builder.push(stored).unwrap();
        assert!(
            weak.upgrade().is_none(),
            "builder retained a historical body"
        );
    }
    let (restored, effects) =
        ExecutionCoordinator::restore(builder.finish().unwrap(), epoch(2)).unwrap();
    assert!(effects.is_empty());
    assert_eq!(restored.snapshot().len(), 32);
}

#[test]
fn pending_and_frozen_outcomes_keep_factual_bytes_without_reexecution() {
    let mut c = Fixture::new(epoch(1));
    let k = key();
    let run = start(&mut c, &k);
    let cancel = only(c.cancel(&k).unwrap());
    let body = CanonicalOutcome::completed(DomainResult::success(
        "factual bytes behind unknown metadata write",
    ));
    let weak = body.weak_bytes();
    assert!(c.report(run.finish(body)).unwrap().is_empty());
    assert!(weak.upgrade().is_some());
    assert!(matches!(
        c.complete(cancel.unknown()).unwrap().as_slice(),
        [Effect::FailStop]
    ));
    assert!(
        weak.upgrade().is_some(),
        "unknown write must not discard queued factual bytes"
    );
    assert!(matches!(c.prepare_task(&k), Err(Error::StoreUnavailable)));
    assert!(matches!(
        c.submit(key(), &owner(), window()),
        Err(Error::StoreUnavailable)
    ));
    drop(c);
    assert!(
        weak.upgrade().is_none(),
        "evidence is volatile owner data, not a durable promise"
    );

    // The Outcome's own request is consumed by unknown(); Pending must still pin it.
    let mut c = Fixture::new(epoch(2));
    let k = key();
    let run = start(&mut c, &k);
    let body = CanonicalOutcome::completed(DomainResult::success("own pending outcome"));
    let weak = body.weak_bytes();
    let request = only(c.report(run.finish(body)).unwrap());
    c.complete(request.unknown()).unwrap();
    assert!(
        weak.upgrade().is_some(),
        "dropping unknown CommitRequest lost its fact"
    );
    assert!(matches!(c.prepare_task(&k), Err(Error::StoreUnavailable)));
    drop(c);
    assert!(weak.upgrade().is_none());
}

#[test]
fn metadata_fingerprint_covers_retained_semantics_and_is_hydration_independent() {
    let mut c = Fixture::new(epoch(1));
    let k = key();
    let run = start(&mut c, &k);
    let alias = attach(&mut c, &k, key());
    let pending = c.cancel(&k).unwrap();
    settle(&mut c, pending);
    let pending = c
        .acknowledge(&k, &owner(), CanonicalOutcome::cancelled().digest(), 100)
        .unwrap();
    settle(&mut c, pending);
    publish(&mut c, run, "retained metadata");
    let original = c.store(&k);
    let fingerprint = original.record().fingerprint();
    let loaded = c
        .prepare_holder(&k, alias)
        .unwrap()
        .loaded(original)
        .unwrap();
    assert_eq!(fingerprint, c.inspect(&k).unwrap().fingerprint());
    let _publication = c.complete_read(loaded).unwrap();
    assert_eq!(fingerprint, c.inspect(&k).unwrap().fingerprint());
    let original_json = serde_json::to_value(original).unwrap();
    // Every changed field remains structurally valid; failure is not masked by a rejected decode.
    let changes = [
        (
            "/record/revision",
            serde_json::json!(original.record().revision() + 1),
        ),
        (
            "/record/accepted/window/budget_ms",
            serde_json::json!(6_000),
        ),
        (
            "/record/accepted/owner_hash",
            serde_json::json!(vec![9; 32]),
        ),
        ("/record/begun/started_epoch_ms", serde_json::json!(21)),
        ("/record/cancel_requested", serde_json::json!(true)),
        (
            "/record/acknowledgement/first_acknowledged_epoch_ms",
            serde_json::json!(101),
        ),
        ("/record/public_task", serde_json::json!(true)),
    ];
    for (path, changed) in changes {
        let mut json = original_json.clone();
        *json.pointer_mut(path).unwrap() = changed;
        let aggregate = StoredRecord::decode(&serde_json::to_vec(&json).unwrap()).unwrap();
        assert_ne!(aggregate.record().fingerprint(), fingerprint, "{path}");
    }
    let replacement = CanonicalOutcome::completed(DomainResult::success("different producer body"));
    let mut json = original_json.clone();
    json["producer"] = serde_json::to_value(&replacement).unwrap();
    json["record"]["producer"] = serde_json::to_value(replacement.descriptor()).unwrap();
    json["record"]["producer_digest"] = serde_json::to_value(replacement.digest()).unwrap();
    let aggregate = StoredRecord::decode(&serde_json::to_vec(&json).unwrap()).unwrap();
    assert_ne!(aggregate.record().fingerprint(), fingerprint);
    let mut compacted = original_json;
    compacted["record"]["compacted"] = true.into();
    compacted["producer"] = serde_json::Value::Null;
    compacted["primary_override"] = serde_json::Value::Null;
    let aggregate = StoredRecord::decode(&serde_json::to_vec(&compacted).unwrap()).unwrap();
    assert_ne!(aggregate.record().fingerprint(), fingerprint);
}
