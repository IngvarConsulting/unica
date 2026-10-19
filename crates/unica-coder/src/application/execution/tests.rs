use super::hydration::*;
use super::test_support::{Archive as DurableCatalog, Fixture as ExecutionCoordinator};
use super::*;
use crate::domain::execution::{HolderObservation, Tool};
use crate::domain::invocation::DomainResult;

fn key() -> Key {
    Key::new(
        InvocationId::new(),
        TaskId::new(),
        [1; 32],
        Tool::Search,
        [2; 32],
        [3; 32],
    )
}
fn owner() -> OwnerCapability {
    OwnerCapability::new([7; 32])
}
fn coordinator() -> ExecutionCoordinator {
    ExecutionCoordinator::new(AuthorityEpoch::new([9; 32]))
}
fn window() -> ObservationWindow {
    ObservationWindow::new(1_000, 7_000).unwrap()
}
fn outcome() -> CanonicalOutcome {
    CanonicalOutcome::completed(DomainResult::success("factual result"))
}
fn authority() -> ReadAuthority {
    ReadAuthority::new([4; 32], 1, 1)
}
fn commit_request(mut effects: Vec<Effect>) -> CommitRequest {
    assert_eq!(effects.len(), 1);
    match effects.remove(0) {
        Effect::Commit(r) => r,
        other => panic!("expected commit, got {other:?}"),
    }
}
fn settle(c: &mut ExecutionCoordinator, effects: Vec<Effect>) -> Vec<Effect> {
    let mut queue: VecDeque<_> = effects.into();
    let mut done = Vec::new();
    while let Some(effect) = queue.pop_front() {
        match effect {
            Effect::Commit(r) => queue.extend(c.commit(r).unwrap()),
            other => done.push(other),
        }
    }
    done
}
fn accept(c: &mut ExecutionCoordinator, k: &Key) -> AcceptedPermit {
    let e = c.submit(k.clone(), &owner(), window()).unwrap();
    match settle(c, e).remove(0) {
        Effect::Accepted(p) => p,
        other => panic!("expected acceptance, got {other:?}"),
    }
}
fn begin(c: &mut ExecutionCoordinator, accepted: AcceptedPermit) -> RunPermit {
    let e = c.begin(accepted, 1_000, Some(authority())).unwrap();
    let mut e = settle(c, e);
    match e.remove(0) {
        Effect::Run(p) => p,
        other => panic!("expected run, got {other:?}"),
    }
}

#[test]
fn accepted_and_begin_commits_are_required_before_a_single_run_permit() {
    let mut c = coordinator();
    let k = key();
    let acceptance = commit_request(c.submit(k.clone(), &owner(), window()).unwrap());
    assert_eq!(c.lookup_task(k.task_id()), Err(Error::NotAccepted));
    // No AcceptedPermit exists while the acceptance commit is pending.
    let mut effects = c.commit(acceptance).unwrap();
    assert!(!effects.iter().any(|e| matches!(e, Effect::Run(_))));
    let accepted = match effects.remove(0) {
        Effect::Accepted(p) => p,
        other => panic!("{other:?}"),
    };
    let pending = c.begin(accepted, 1_000, Some(authority())).unwrap();
    assert!(!pending.iter().any(|e| matches!(e, Effect::Run(_))));
    assert!(matches!(&settle(&mut c, pending)[..], [Effect::Run(_)]));
}

#[test]
fn early_cancel_has_no_task_entitlement_and_submit_never_runs() {
    let mut c = coordinator();
    let k = key();
    let e = c.cancel(&k).unwrap();
    settle(&mut c, e);
    assert_eq!(c.lookup_task(k.task_id()), Err(Error::NotAccepted));
    let e = c.submit(k.clone(), &owner(), window()).unwrap();
    assert!(settle(&mut c, e).is_empty());
    assert_eq!(
        c.lookup_task(k.task_id()).unwrap().primary(),
        Some(&CanonicalOutcome::cancelled().descriptor())
    );
    assert!(matches!(
        State::from_record(c.inspect(&k).unwrap().clone())
            .unwrap()
            .decide(Event::Begin {
                started_epoch_ms: 1_000,
                read_authority: None
            }),
        Err(Error::NotRunnable)
    ));
}

#[test]
fn both_ids_are_claimed_before_commit_and_remain_claimed_after_compaction() {
    let mut c = coordinator();
    let k = key();
    let pending = c.submit(k.clone(), &owner(), window()).unwrap();
    let same_i = Key::new(
        k.invocation_id(),
        TaskId::new(),
        [1; 32],
        Tool::Search,
        [5; 32],
        [3; 32],
    );
    let same_t = Key::new(
        InvocationId::new(),
        k.task_id(),
        [1; 32],
        Tool::Search,
        [5; 32],
        [3; 32],
    );
    for collision in [&same_i, &same_t] {
        assert!(matches!(
            c.submit(collision.clone(), &owner(), window()),
            Err(Error::IdentityCollision)
        ));
    }
    let accepted = match settle(&mut c, pending).remove(0) {
        Effect::Accepted(p) => p,
        other => panic!("{other:?}"),
    };
    let run = begin(&mut c, accepted);
    let result = outcome();
    let e = c.report(run.finish(result.clone())).unwrap();
    settle(&mut c, e);
    let e = c.acknowledge(&k, &owner(), result.digest(), 9_000).unwrap();
    settle(&mut c, e);
    assert!(c.inspect(&k).unwrap().compacted());
    for collision in [same_i, same_t] {
        assert!(matches!(
            c.submit(collision, &owner(), window()),
            Err(Error::IdentityCollision)
        ));
    }
}

#[test]
fn queued_factual_outcome_survives_cancel_expose_and_known_rollback() {
    let mut c = coordinator();
    let k = key();
    let accepted = accept(&mut c, &k);
    let run = begin(&mut c, accepted);
    let cancel = commit_request(c.cancel(&k).unwrap());
    let result = outcome();
    assert!(c.report(run.finish(result.clone())).unwrap().is_empty());
    assert!(c.expose_task(&k).unwrap().is_empty());
    let retry = commit_request(c.complete(cancel.rolled_back()).unwrap());
    let effects = c.commit(retry).unwrap();
    settle(&mut c, effects);
    let record = c.inspect(&k).unwrap();
    assert_eq!(record.producer(), Some(&result.descriptor()));
    assert!(record.cancel_requested());
    assert!(record.public_task());
}

#[test]
fn metadata_conflict_redecides_factual_outcome_without_new_execution() {
    let mut c = coordinator();
    let k = key();
    let accepted = accept(&mut c, &k);
    let run = begin(&mut c, accepted);
    let result = outcome();
    let pending = commit_request(c.report(run.finish(result.clone())).unwrap());
    // A competing durable metadata change is produced by the same pure model.
    let old = State::from_record(c.inspect(&k).unwrap().clone()).unwrap();
    let (exposed, _, payloads) = old.decide(Event::ExposeTask).unwrap().unwrap().into_parts();
    let effects = c
        .readback(
            pending,
            payloads
                .assemble(exposed.record().clone(), Some(c.store(&k)))
                .unwrap(),
        )
        .unwrap();
    let effects = settle(&mut c, effects);
    assert!(!effects.iter().any(|e| matches!(e, Effect::Run(_))));
    assert_eq!(
        c.inspect(&k).unwrap().producer(),
        Some(&result.descriptor())
    );
    assert!(c.inspect(&k).unwrap().public_task());
    assert_eq!(c.store(&k).producer(), Some(&result));
    let loaded = c.prepare_task(&k).unwrap().loaded(c.store(&k)).unwrap();
    assert!(matches!(c.complete_read(loaded).unwrap(),
        ReadPublication::Task(permit) if permit.outcome() == &result));
}

#[test]
fn exact_readback_and_duplicate_callback_never_repeat_the_run_permit() {
    let mut c = coordinator();
    let k = key();
    let accepted = accept(&mut c, &k);
    let request = commit_request(c.begin(accepted, 1_000, Some(authority())).unwrap());
    let duplicate = CommitCompletion {
        stamp: request.stamp.clone(),
        resolution: Resolution::Committed,
    };
    let actual = c.assemble(&request);
    let effects = c.readback(request, actual).unwrap();
    assert!(matches!(&effects[..], [Effect::Run(_)]));
    assert!(matches!(c.complete(duplicate), Err(Error::StaleCompletion)));
}

#[test]
fn unknown_commit_closes_admission_without_publishing_pending_outcome() {
    let mut c = coordinator();
    let k = key();
    let accepted = accept(&mut c, &k);
    let run = begin(&mut c, accepted);
    let request = commit_request(c.report(run.finish(outcome())).unwrap());
    assert!(matches!(
        &c.complete(request.unknown()).unwrap()[..],
        [Effect::FailStop]
    ));
    assert!(c.unavailable());
    assert!(c.inspect(&k).unwrap().producer().is_none());
    assert_eq!(c.lookup_task(k.task_id()), Err(Error::StoreUnavailable));
    assert!(matches!(
        c.submit(key(), &owner(), window()),
        Err(Error::StoreUnavailable)
    ));
}

#[test]
fn recovery_preserves_primary_and_drops_aliases_without_replaying() {
    let mut c = coordinator();
    let k = key();
    let accepted = accept(&mut c, &k);
    let run = begin(&mut c, accepted);
    let alias = Key::new(
        InvocationId::new(),
        TaskId::new(),
        [1; 32],
        Tool::Search,
        [2; 32],
        [3; 32],
    );
    let e = c
        .attach(&k, JoinEvidence::new(alias.clone(), authority(), 8_000))
        .unwrap();
    let e = settle(&mut c, e);
    let member = match e.as_slice() {
        [Effect::Attached(m)] => *m,
        _ => panic!("expected alias"),
    };
    let e = c.cancel(&k).unwrap();
    settle(&mut c, e);
    let result = outcome();
    let e = c.report(run.finish(result.clone())).unwrap();
    settle(&mut c, e);
    assert_eq!(
        c.observe_holder(&k, member),
        Ok(HolderObservation::Terminal(&result.descriptor()))
    );
    assert_eq!(
        c.inspect(&k).unwrap().primary(),
        Some(&CanonicalOutcome::cancelled().descriptor())
    );
    let bytes = c.catalog().unwrap().encode();
    assert!(!String::from_utf8_lossy(&bytes).contains(&alias.task_id().to_string()));
    let (restored, effects) = ExecutionCoordinator::restore(
        DurableCatalog::decode(&bytes).unwrap(),
        AuthorityEpoch::new([10; 32]),
    )
    .unwrap();
    assert!(effects.is_empty());
    assert_eq!(restored.holder_count(&k), 0);
    assert_eq!(
        restored.lookup_task(alias.task_id()),
        Err(Error::NotAccepted)
    );
    assert_eq!(
        restored.inspect(&k).unwrap().primary(),
        Some(&CanonicalOutcome::cancelled().descriptor())
    );
    assert_eq!(
        restored.inspect(&k).unwrap().producer(),
        Some(&result.descriptor())
    );
}

#[test]
fn begun_recovery_requires_commit_and_admits_no_old_worker_evidence() {
    let mut c = coordinator();
    let k = key();
    let accepted = accept(&mut c, &k);
    let run = begin(&mut c, accepted);
    let (mut restored, effects) =
        ExecutionCoordinator::restore(c.catalog().unwrap(), AuthorityEpoch::new([10; 32])).unwrap();
    assert!(restored.recovering());
    assert_eq!(
        restored.lookup_task(k.task_id()),
        Err(Error::RecoveryRequired)
    );
    assert!(matches!(
        restored.report(run.finish(outcome())),
        Err(Error::ForeignOutcome)
    ));
    assert!(!settle(&mut restored, effects)
        .iter()
        .any(|e| matches!(e, Effect::Run(_))));
    assert_eq!(
        restored.lookup_task(k.task_id()).unwrap().producer(),
        Some(&CanonicalOutcome::failed(model::FailureReason::OutcomeUncertain).descriptor())
    );
}

#[test]
fn strict_decode_rejects_noncanonical_payload_and_impossible_record() {
    let result = outcome();
    let mut value = serde_json::to_value(&result).unwrap();
    value["digest"] = serde_json::json!(vec![0; 32]);
    assert!(serde_json::from_value::<CanonicalOutcome>(value).is_err());
    let mut c = coordinator();
    let k = key();
    accept(&mut c, &k);
    let mut value = serde_json::to_value(c.store(&k)).unwrap();
    value["record"]["primary_digest"] = serde_json::json!(vec![0; 32]);
    assert!(serde_json::from_value::<StoredRecord>(value).is_err());
}

fn task_permit(c: &mut ExecutionCoordinator, k: &Key) -> TaskPublicationPermit {
    let e = c.expose_task(k).unwrap();
    match settle(c, e).remove(0) {
        Effect::Task(p) => p,
        other => panic!("expected task publication, got {other:?}"),
    }
}

#[test]
fn frontend_choice_never_acks_after_task_and_keeps_built_direct() {
    let mut c = coordinator();
    let k = key();
    let accepted = accept(&mut c, &k);
    let run = begin(&mut c, accepted);
    let result = outcome();
    let e = c.report(run.finish(result.clone())).unwrap();
    settle(&mut c, e);
    let mut task = DeliveryChoice::new(k.clone());
    task.return_task(task_permit(&mut c, &k)).unwrap();
    assert!(task.build_direct(c.direct(&k, &owner()).unwrap()).is_err());
    assert_eq!(task.ack_digest(), None);
    let mut direct = DeliveryChoice::new(k.clone());
    direct
        .build_direct(c.direct(&k, &owner()).unwrap())
        .unwrap();
    assert!(direct.return_task(task_permit(&mut c, &k)).is_err());
    // An ACK transport failure has no transition capable of clearing DirectBuilt.
    assert_eq!(direct.direct(), Some(&result));
    assert_eq!(direct.ack_digest(), Some(result.digest()));
}

#[test]
fn admission_refusal_retains_accepted_identity_without_begin_or_run() {
    let mut c = coordinator();
    let k = key();
    let e = c.submit(k.clone(), &owner(), window()).unwrap();
    let permit = match settle(&mut c, e).remove(0) {
        Effect::Accepted(p) => p,
        other => panic!("{other:?}"),
    };
    let mut refusal = DomainResult::success("validation refused");
    refusal.ok = false;
    let expected = CanonicalOutcome::completed(refusal.clone());
    let e = c.reject_admission(permit.reject(refusal).unwrap()).unwrap();
    assert!(settle(&mut c, e).is_empty());
    assert_eq!(
        c.lookup_task(k.task_id()).unwrap().primary(),
        Some(&expected.descriptor())
    );
    assert_eq!(c.inspect(&k).unwrap().run(), None);
    assert!(matches!(
        State::from_record(c.inspect(&k).unwrap().clone())
            .unwrap()
            .decide(Event::Begin {
                started_epoch_ms: 1_000,
                read_authority: None
            }),
        Err(Error::NotRunnable)
    ));
}

#[test]
fn first_acknowledgement_is_immutable_across_duplicates_and_reopen() {
    let mut c = coordinator();
    let k = key();
    let accepted = accept(&mut c, &k);
    let run = begin(&mut c, accepted);
    let result = outcome();
    let e = c.report(run.finish(result.clone())).unwrap();
    settle(&mut c, e);
    let pending = commit_request(c.acknowledge(&k, &owner(), result.digest(), 9_000).unwrap());
    assert!(c.inspect(&k).unwrap().acknowledgement().is_none());
    let effects = c.commit(pending).unwrap();
    settle(&mut c, effects);
    let ack = *c.inspect(&k).unwrap().acknowledgement().unwrap();
    assert_eq!(ack.digest(), result.digest());
    assert_eq!(ack.first_acknowledged_epoch_ms(), 9_000);
    let bytes = c.catalog().unwrap().encode();
    // Even a backwards caller clock cannot replace the first confirmed time.
    assert!(c
        .acknowledge(&k, &owner(), result.digest(), 1)
        .unwrap()
        .is_empty());
    assert_eq!(c.catalog().unwrap().encode(), bytes);
    let (mut restored, effects) = ExecutionCoordinator::restore(
        DurableCatalog::decode(&bytes).unwrap(),
        AuthorityEpoch::new([10; 32]),
    )
    .unwrap();
    assert!(effects.is_empty());
    assert!(restored
        .acknowledge(&k, &owner(), result.digest(), 99_000)
        .unwrap()
        .is_empty());
    assert_eq!(restored.inspect(&k).unwrap().acknowledgement(), Some(&ack));
    assert_eq!(restored.catalog().unwrap().encode(), bytes);
    assert!(restored.inspect(&k).unwrap().compacted());
}

#[test]
fn publication_requires_committed_outcome_exact_key_and_live_direct_right() {
    let mut c = coordinator();
    let k = key();
    let accepted = accept(&mut c, &k);
    let run = begin(&mut c, accepted);
    assert!(matches!(
        c.prepare_direct(&k, &owner()),
        Err(Error::NotRunnable)
    ));
    let result = outcome();
    let pending = commit_request(c.report(run.finish(result.clone())).unwrap());
    assert!(matches!(
        c.prepare_direct(&k, &owner()),
        Err(Error::NotRunnable)
    ));
    let effects = c.commit(pending).unwrap();
    settle(&mut c, effects);
    assert!(matches!(
        c.prepare_direct(&k, &OwnerCapability::new([99; 32])),
        Err(Error::OwnerMismatch)
    ));
    let other_key = key();
    let mut wrong_task = DeliveryChoice::new(other_key.clone());
    assert_eq!(
        wrong_task.return_task(task_permit(&mut c, &k)),
        Err(Error::IdentityCollision)
    );
    let mut wrong_direct = DeliveryChoice::new(other_key);
    assert_eq!(
        wrong_direct.build_direct(c.direct(&k, &owner()).unwrap()),
        Err(Error::IdentityCollision)
    );
    let prepared_before_ack = c.direct(&k, &owner()).unwrap();
    let e = c.acknowledge(&k, &owner(), result.digest(), 9_000).unwrap();
    settle(&mut c, e);
    assert!(c.inspect(&k).unwrap().public_task());
    assert!(!c.inspect(&k).unwrap().compacted());
    assert!(matches!(
        c.prepare_direct(&k, &owner()),
        Err(Error::AlreadyAcknowledged)
    ));
    let mut direct = DeliveryChoice::new(k);
    direct.build_direct(prepared_before_ack).unwrap();
    assert_eq!(direct.direct(), Some(&result));
}

#[test]
fn alias_capacity_counts_pending_cancelled_and_cross_group_until_retirement() {
    let mut c = coordinator();
    c.alias_capacity = 2;
    let first = key();
    let second = key();
    let p = accept(&mut c, &first);
    let _run1 = begin(&mut c, p);
    let p = accept(&mut c, &second);
    let _run2 = begin(&mut c, p);
    let pending1 = c
        .attach(&first, JoinEvidence::new(key(), authority(), 8_000))
        .unwrap();
    let pending2 = c
        .attach(&second, JoinEvidence::new(key(), authority(), 8_000))
        .unwrap();
    let overflow = key();
    assert!(matches!(
        c.attach(
            &second,
            JoinEvidence::new(overflow.clone(), authority(), 8_000)
        ),
        Err(Error::AliasCapacity)
    ));
    let member = match settle(&mut c, pending1).remove(0) {
        Effect::Attached(m) => m,
        other => panic!("{other:?}"),
    };
    settle(&mut c, pending2);
    let e = c.cancel_holder(&first, member).unwrap();
    settle(&mut c, e);
    assert!(matches!(
        c.attach(
            &second,
            JoinEvidence::new(overflow.clone(), authority(), 8_000)
        ),
        Err(Error::AliasCapacity)
    ));
    let retiring = c.retire_holder(&first, member).unwrap();
    assert!(matches!(
        c.attach(
            &second,
            JoinEvidence::new(overflow.clone(), authority(), 8_000)
        ),
        Err(Error::AliasCapacity)
    ));
    settle(&mut c, retiring);
    let invalid = c
        .attach(
            &second,
            JoinEvidence::new(overflow.clone(), ReadAuthority::new([99; 32], 1, 1), 8_000),
        )
        .unwrap();
    assert!(matches!(
        &invalid[..],
        [Effect::Rejected {
            error: Error::NotJoinable,
            ..
        }]
    ));
    // Both rejected IDs and the capacity reservation have been released.
    let e = c
        .attach(&second, JoinEvidence::new(overflow, authority(), 8_000))
        .unwrap();
    assert!(matches!(&settle(&mut c, e)[..], [Effect::Attached(_)]));
}

#[test]
fn owner_mismatch_readback_cannot_issue_begin_authority_and_old_epoch_cannot_run() {
    let mut c = coordinator();
    let k = key();
    let pending = commit_request(c.submit(k.clone(), &owner(), window()).unwrap());
    let mut other = coordinator();
    let e = other
        .submit(k.clone(), &OwnerCapability::new([99; 32]), window())
        .unwrap();
    settle(&mut other, e);
    let effects = c.readback(pending, other.store(&k).clone()).unwrap();
    assert!(matches!(
        &effects[..],
        [Effect::Rejected {
            error: Error::OwnerMismatch,
            ..
        }]
    ));
    assert_eq!(c.inspect(&k).unwrap().run(), None);
    // Begin has no Key-only ingress and this owner never received AcceptedPermit.
    let mut original = coordinator();
    let k = key();
    let accepted = accept(&mut original, &k);
    let (mut restored, effects) =
        ExecutionCoordinator::restore(original.catalog().unwrap(), AuthorityEpoch::new([10; 32]))
            .unwrap();
    settle(&mut restored, effects);
    assert!(matches!(
        restored.begin(accepted, 1_000, Some(authority())),
        Err(Error::ForeignOutcome)
    ));
    assert_eq!(restored.inspect(&k).unwrap().run(), None);
}

#[test]
fn late_factual_completion_is_retained_after_unknown_metadata_without_reopening() {
    for expose in [false, true] {
        let mut c = coordinator();
        let k = key();
        let accepted = accept(&mut c, &k);
        let run = begin(&mut c, accepted);
        let identity = run.run_identity();
        let pending = commit_request(if expose {
            c.expose_task(&k).unwrap()
        } else {
            c.cancel(&k).unwrap()
        });
        assert!(matches!(
            &c.complete(pending.unknown()).unwrap()[..],
            [Effect::FailStop]
        ));
        let result = outcome();
        assert!(matches!(
            &c.report(run.finish(result.clone())).unwrap()[..],
            [Effect::FailStop]
        ));
        let slot = c.groups.get(&k.invocation_id()).unwrap();
        assert!(slot.pending.is_some());
        assert!(
            matches!(slot.queue.back(), Some(Event::Outcome { run, outcome })
            if *run == identity && outcome == &result)
        );
        assert!(c.inspect(&k).unwrap().producer().is_none());
        assert!(matches!(
            c.prepare_direct(&k, &owner()),
            Err(Error::StoreUnavailable)
        ));
        assert!(matches!(c.expose_task(&k), Err(Error::StoreUnavailable)));
        assert!(matches!(
            c.submit(key(), &owner(), window()),
            Err(Error::StoreUnavailable)
        ));
        assert!(c.unavailable());
    }
}
