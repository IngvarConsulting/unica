//! Known no-publication failures stop this owner; clean rollback still retries.
use super::test_support::Fixture;
use super::*;
use crate::domain::execution::Tool;
use crate::domain::invocation::DomainResult;

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
fn window() -> ObservationWindow {
    ObservationWindow::new(10, 7_000).unwrap()
}
fn outcome(text: &str) -> CanonicalOutcome {
    CanonicalOutcome::completed(DomainResult::success(text))
}
fn only(mut effects: Vec<Effect>) -> CommitRequest {
    assert_eq!(effects.len(), 1);
    match effects.remove(0) {
        Effect::Commit(r) => r,
        other => panic!("expected one commit, got {other:?}"),
    }
}
fn accept(c: &mut Fixture, k: &Key) -> AcceptedPermit {
    let request = only(c.submit(k.clone(), &owner(), window()).unwrap());
    match c.commit(request).unwrap().remove(0) {
        Effect::Accepted(p) => p,
        other => panic!("{other:?}"),
    }
}
fn begin_request(c: &mut Fixture, k: &Key) -> CommitRequest {
    let accepted = accept(c, k);
    only(c.begin(accepted, 20, None).unwrap())
}
fn start(c: &mut Fixture, k: &Key) -> RunPermit {
    let request = begin_request(c, k);
    match c.commit(request).unwrap().remove(0) {
        Effect::Run(p) => p,
        other => panic!("{other:?}"),
    }
}

#[derive(Clone, Copy, Debug)]
enum Step {
    Submit,
    Begin,
    Outcome,
    Ack,
    Expose,
    Cancel,
}

fn fail(c: &mut ExecutionCoordinator, request: CommitRequest, cause: WriteFailureCause) {
    let id = request.id();
    let key = request.key().clone();
    let epoch = request.authority_epoch();
    let revision = request.expected_revision();
    let fingerprint = request.candidate_fingerprint();
    let effects = c
        .complete(request.failed_before_publication(cause))
        .unwrap();
    assert!(
        matches!(effects.as_slice(), [Effect::FailStop]),
        "known write failure must not retry or grant rights: {effects:?}"
    );
    let first = c.first_known_write_failure().unwrap();
    assert_eq!(first.cause(), cause);
    assert_eq!(first.key(), &key);
    assert_eq!(first.authority_epoch(), epoch);
    assert_eq!(first.commit_id(), id);
    assert_eq!(first.expected_revision(), revision);
    assert_eq!(first.candidate_fingerprint(), fingerprint);
    assert!(c.unavailable());
}

#[test]
fn known_failure_never_confirms_retries_or_grants_pending_transition_rights() {
    for step in [
        Step::Submit,
        Step::Begin,
        Step::Outcome,
        Step::Ack,
        Step::Expose,
        Step::Cancel,
    ] {
        let mut c = Fixture::new(epoch(1));
        let k = key();
        let request = match step {
            Step::Submit => only(c.submit(k.clone(), &owner(), window()).unwrap()),
            Step::Begin => begin_request(&mut c, &k),
            Step::Outcome => {
                let run = start(&mut c, &k);
                only(c.report(run.finish(outcome("completed mutation"))).unwrap())
            }
            Step::Ack => {
                let run = start(&mut c, &k);
                let factual = outcome("primary retained until confirmed acknowledgement");
                let terminal = only(c.report(run.finish(factual.clone())).unwrap());
                assert!(c.commit(terminal).unwrap().is_empty());
                let ack = only(c.acknowledge(&k, &owner(), factual.digest(), 100).unwrap());
                assert!(ack.candidate().compacted());
                ack
            }
            Step::Expose => {
                let _run = start(&mut c, &k);
                let expose = only(c.expose_task(&k).unwrap());
                assert!(expose.candidate().public_task());
                expose
            }
            Step::Cancel => {
                let _run = start(&mut c, &k);
                let cancel = only(c.cancel(&k).unwrap());
                assert!(cancel.candidate().cancel_requested());
                cancel
            }
        };
        let confirmed = c.snapshot();
        let archive = c.catalog().unwrap().encode();
        fail(&mut c, request, WriteFailureCause::NoSpace);
        assert_eq!(c.snapshot(), confirmed, "{step:?}");
        assert_eq!(c.catalog().unwrap().encode(), archive, "{step:?}");
        assert_eq!(c.lookup_task(k.task_id()), Err(Error::StoreUnavailable));
        assert!(matches!(c.prepare_task(&k), Err(Error::StoreUnavailable)));
        assert!(matches!(c.cancel(&k), Err(Error::StoreUnavailable)));
        assert!(matches!(c.expose_task(&k), Err(Error::StoreUnavailable)));
        assert!(matches!(
            c.submit(key(), &owner(), window()),
            Err(Error::StoreUnavailable)
        ));
    }
}

#[test]
fn failed_ack_keeps_confirmed_retention_and_already_built_direct() {
    let mut c = Fixture::new(epoch(1));
    let k = key();
    let run = start(&mut c, &k);
    let factual = outcome("deliver once even if acknowledgement cannot be written");
    let request = only(c.report(run.finish(factual.clone())).unwrap());
    c.commit(request).unwrap();
    let mut delivery = DeliveryChoice::new(k.clone());
    delivery
        .build_direct(c.direct(&k, &owner()).unwrap())
        .unwrap();
    let delayed_read = c.prepare_task(&k).unwrap().loaded(c.store(&k)).unwrap();
    let request = only(c.acknowledge(&k, &owner(), factual.digest(), 100).unwrap());
    assert!(request.candidate().compacted());
    fail(&mut c, request, WriteFailureCause::AccessDenied);
    assert!(c.inspect(&k).unwrap().acknowledgement().is_none());
    assert!(!c.inspect(&k).unwrap().compacted());
    assert_eq!(c.store(&k).primary(), Some(&factual));
    assert_eq!(delivery.direct(), Some(&factual));
    assert!(matches!(
        c.complete_read(delayed_read),
        Err(Error::StoreUnavailable)
    ));
    assert!(matches!(
        c.direct(&k, &owner()),
        Err(Error::StoreUnavailable)
    ));

    // The known failed ACK was never written. A fresh owner sees the retained
    // terminal and can confirm its first ACK; no worker is run again.
    let (mut reopened, effects) = Fixture::restore(c.catalog().unwrap(), epoch(2)).unwrap();
    assert!(effects.is_empty());
    assert!(reopened.first_known_write_failure().is_none());
    assert_eq!(reopened.direct(&k, &owner()).unwrap().outcome, factual);
    let ack = only(
        reopened
            .acknowledge(&k, &owner(), factual.digest(), 200)
            .unwrap(),
    );
    assert!(reopened.commit(ack).unwrap().is_empty());
    assert!(reopened.inspect(&k).unwrap().compacted());
    let first = *reopened.inspect(&k).unwrap().acknowledgement().unwrap();
    assert!(reopened
        .acknowledge(&k, &owner(), factual.digest(), 300)
        .unwrap()
        .is_empty());
    assert_eq!(
        reopened.inspect(&k).unwrap().acknowledgement(),
        Some(&first)
    );
}

#[test]
fn confirmed_exposure_needs_no_write() {
    let mut c = Fixture::new(epoch(1));
    let k = key();
    let _run = start(&mut c, &k);
    let request = only(c.expose_task(&k).unwrap());
    assert!(matches!(
        c.commit(request).unwrap().as_slice(),
        [Effect::Task(_)]
    ));
    let before = c.snapshot();
    assert!(matches!(
        c.expose_task(&k).unwrap().as_slice(),
        [Effect::Task(_)]
    ));
    assert_eq!(c.snapshot(), before);
    assert!(c.first_known_write_failure().is_none());
}

#[test]
fn pending_queued_and_late_factual_bytes_survive_known_failure_until_owner_drop() {
    // Every result is moved into production. Neither the test store nor a
    // publication/read request owns an extra strong reference to these bytes.
    for placement in [
        "pending",
        "queued-cancel",
        "queued-expose",
        "late-cancel",
        "late-expose",
    ] {
        let mut c = Fixture::new(epoch(1));
        let k = key();
        let run = start(&mut c, &k);
        let factual = outcome(placement);
        let descriptor = factual.descriptor();
        let weak = factual.weak_bytes();
        let evidence = run.finish(factual);
        if placement == "pending" {
            let request = only(c.report(evidence).unwrap());
            fail(&mut c, request, WriteFailureCause::IoFailure);
        } else {
            let request = if placement.ends_with("cancel") {
                only(c.cancel(&k).unwrap())
            } else {
                only(c.expose_task(&k).unwrap())
            };
            if placement.starts_with("queued") {
                assert!(c.report(evidence).unwrap().is_empty());
                fail(&mut c, request, WriteFailureCause::IoFailure);
            } else {
                fail(&mut c, request, WriteFailureCause::IoFailure);
                let first = c.first_known_write_failure().cloned();
                assert!(matches!(
                    c.report(evidence).unwrap().as_slice(),
                    [Effect::FailStop]
                ));
                assert_eq!(c.first_known_write_failure(), first.as_ref());
            }
        }
        assert!(
            weak.upgrade().is_some(),
            "lost {placement} factual ownership"
        );
        assert_ne!(c.inspect(&k).unwrap().producer(), Some(&descriptor));
        assert!(c.inspect(&k).unwrap().producer().is_none());
        assert!(c.store(&k).producer().is_none());
        assert!(matches!(c.prepare_task(&k), Err(Error::StoreUnavailable)));
        assert!(matches!(c.expose_task(&k), Err(Error::StoreUnavailable)));
        drop(c);
        assert!(weak.upgrade().is_none(), "external owner masks {placement}");
    }
}

#[test]
fn foreign_and_stale_known_failures_cannot_poison_a_healthy_pending_begin() {
    for difference in ["epoch", "fingerprint", "commit-id", "key"] {
        let mut healthy = Fixture::new(epoch(1));
        let k = key();
        let pending = begin_request(&mut healthy, &k);
        let mut foreign = Fixture::new(if difference == "epoch" {
            epoch(2)
        } else {
            epoch(1)
        });
        if difference == "commit-id" {
            // A genuine unrelated acceptance advances only the commit sequence.
            let _accepted = accept(&mut foreign, &key());
        }
        let foreign_key = if difference == "key" {
            key()
        } else {
            k.clone()
        };
        let foreign_owner = if difference == "fingerprint" {
            OwnerCapability::new([9; 32])
        } else {
            owner()
        };
        let submitted = only(
            foreign
                .submit(foreign_key.clone(), &foreign_owner, window())
                .unwrap(),
        );
        let accepted = match foreign.commit(submitted).unwrap().remove(0) {
            Effect::Accepted(p) => p,
            other => panic!("{other:?}"),
        };
        let stale = only(foreign.begin(accepted, 20, None).unwrap());
        if difference == "epoch" {
            assert_eq!(stale.key(), pending.key());
            assert_eq!(stale.id(), pending.id());
            assert_eq!(stale.expected_revision(), pending.expected_revision());
            assert_eq!(
                stale.candidate_fingerprint(),
                pending.candidate_fingerprint()
            );
            assert_ne!(stale.authority_epoch(), pending.authority_epoch());
        }
        let before = healthy.snapshot();
        let result = healthy.complete(stale.failed_before_publication(WriteFailureCause::NoSpace));
        assert!(
            !healthy.unavailable(),
            "{difference} foreign callback stopped healthy owner: {result:?}"
        );
        assert!(
            matches!(result, Err(Error::StaleCompletion)),
            "{difference}: {result:?}"
        );
        assert!(healthy.first_known_write_failure().is_none());
        assert_eq!(healthy.snapshot(), before);
        // The healthy request still produces exactly one writable Run right.
        assert!(matches!(
            healthy.commit(pending).unwrap().as_slice(),
            [Effect::Run(_)]
        ));
    }
}

#[test]
fn cross_group_late_callbacks_never_grant_rights_or_replace_the_first_failure() {
    for completion in ["committed", "unknown", "known"] {
        let mut c = Fixture::new(epoch(1));
        let failed_key = key();
        let _run = start(&mut c, &failed_key);
        let other_key = key();
        let other_begin = begin_request(&mut c, &other_key);
        let failed = only(c.cancel(&failed_key).unwrap());
        let before = c.snapshot();
        fail(&mut c, failed, WriteFailureCause::AccessDenied);
        let first = c.first_known_write_failure().cloned();
        let late = match completion {
            "committed" => other_begin.committed(),
            "unknown" => other_begin.unknown(),
            "known" => other_begin.failed_before_publication(WriteFailureCause::NoSpace),
            _ => unreachable!(),
        };
        assert!(matches!(c.complete(late), Err(Error::StoreUnavailable)));
        assert_eq!(c.first_known_write_failure(), first.as_ref());
        assert_eq!(c.snapshot(), before);
        assert!(c.inspect(&other_key).unwrap().run().is_none());
        assert!(matches!(
            c.expose_task(&other_key),
            Err(Error::StoreUnavailable)
        ));
    }
}

#[test]
fn unknown_first_failure_stays_unknown_when_a_late_known_failure_arrives() {
    let mut c = Fixture::new(epoch(1));
    let a = key();
    let b = key();
    let unknown = only(c.submit(a, &owner(), window()).unwrap());
    let later = only(c.submit(b, &owner(), window()).unwrap());
    assert!(matches!(
        c.complete(unknown.unknown()).unwrap().as_slice(),
        [Effect::FailStop]
    ));
    assert!(matches!(
        c.complete(later.failed_before_publication(WriteFailureCause::NoSpace)),
        Err(Error::StoreUnavailable)
    ));
    assert!(c.unavailable());
    assert!(c.first_known_write_failure().is_none());
}

// This adapter fixture writes the aggregate selected by production transitions.
// It proves real read/decode/recovery behavior, not fsync, crash atomicity or an
// OS lease. Disk may be ahead of the old coordinator's confirmed metadata.
fn disk_path(root: &std::path::Path, key: &Key) -> std::path::PathBuf {
    root.join(format!("{}.json", key.task_id()))
}
fn write_candidate(root: &std::path::Path, request: &CommitRequest) {
    let path = disk_path(root, request.key());
    let prior = std::fs::read(&path)
        .ok()
        .map(|b| StoredRecord::decode(&b).unwrap());
    let stored = request.assemble(prior.as_ref()).unwrap();
    std::fs::write(path, stored.encode()).unwrap();
}
fn restore_disk(root: &std::path::Path) -> (ExecutionCoordinator, Vec<Effect>) {
    let mut builder = crate::domain::execution::storage::RecoveryBuilder::default();
    for entry in std::fs::read_dir(root).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        builder.push_encoded(&bytes).unwrap();
    }
    ExecutionCoordinator::restore(builder.finish().unwrap(), epoch(2)).unwrap()
}

#[test]
fn restart_uses_real_terminal_catalog_or_uncertain_begun_not_the_frozen_candidate() {
    use crate::domain::execution::FailureReason;
    // Unknown is compatible with either physical outcome. Recovery must read
    // the aggregate, independently of the old callback's classification.
    for (terminal_was_written, callback_is_committed) in
        [(true, true), (true, false), (false, false)]
    {
        let directory = tempfile::tempdir().unwrap();
        let mut c = Fixture::new(epoch(1));
        let a = key();
        let b = key();
        let _run_a = start(&mut c, &a);
        let run_b = start(&mut c, &b);
        for stored in c.catalog().unwrap().records() {
            std::fs::write(
                disk_path(directory.path(), stored.record().key()),
                stored.encode(),
            )
            .unwrap();
        }
        let old_a = std::fs::read(disk_path(directory.path(), &a)).unwrap();
        let factual = outcome("physical terminal written before its callback");
        let terminal = only(c.report(run_b.finish(factual.clone())).unwrap());
        if terminal_was_written {
            write_candidate(directory.path(), &terminal);
        }
        let failed = only(c.expose_task(&a).unwrap());
        let old_confirmed = c.snapshot();
        fail(&mut c, failed, WriteFailureCause::NoSpace);
        let first = c.first_known_write_failure().cloned();
        let late = if callback_is_committed {
            terminal.committed()
        } else {
            terminal.unknown()
        };
        assert!(matches!(c.complete(late), Err(Error::StoreUnavailable)));
        assert_eq!(c.first_known_write_failure(), first.as_ref());
        assert_eq!(c.snapshot(), old_confirmed);
        assert_eq!(
            std::fs::read(disk_path(directory.path(), &a)).unwrap(),
            old_a
        );
        drop(c);

        let (mut reopened, effects) = restore_disk(directory.path());
        assert!(reopened.first_known_write_failure().is_none());
        assert!(reopened.recovering());
        assert!(matches!(
            reopened.lookup_task(b.task_id()),
            Err(Error::RecoveryRequired)
        ));
        let mut queue: VecDeque<_> = effects.into();
        let mut recovery_keys = Vec::new();
        while let Some(effect) = queue.pop_front() {
            // Recovery may persist uncertainty; it must never replay a worker.
            let Effect::Commit(request) = effect else {
                panic!("unexpected recovery right: {effect:?}")
            };
            recovery_keys.push(request.key().clone());
            write_candidate(directory.path(), &request);
            queue.extend(reopened.complete(request.committed()).unwrap());
        }
        assert!(recovery_keys.contains(&a));
        assert_eq!(recovery_keys.contains(&b), !terminal_was_written);
        assert!(!reopened.recovering());
        assert!(!reopened.unavailable());
        assert!(!reopened.inspect(&a).unwrap().public_task());
        let expected = if terminal_was_written {
            factual
        } else {
            CanonicalOutcome::failed(FailureReason::OutcomeUncertain)
        };
        let actual =
            StoredRecord::decode(&std::fs::read(disk_path(directory.path(), &b)).unwrap()).unwrap();
        let loaded = reopened.prepare_task(&b).unwrap().loaded(&actual).unwrap();
        match reopened.complete_read(loaded).unwrap() {
            super::hydration::ReadPublication::Task(p) => assert_eq!(p.outcome(), &expected),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            reopened.lookup_task(b.task_id()).unwrap().producer(),
            Some(&expected.descriptor())
        );
    }
}
