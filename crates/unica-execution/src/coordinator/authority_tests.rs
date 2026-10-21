//! Physical faults invalidate later rights, but never discard factual outcomes.
use super::hydration::ReadPublication;
use super::test_support::Fixture;
use super::*;
use unica_execution_contract::invocation::DomainResult;
use unica_execution_contract::Tool;

fn key() -> Key {
    Key::new(
        InvocationId::new(),
        TaskId::new(),
        [1; 32],
        Tool::Apply,
        [2; 32],
        [3; 32],
    )
}
fn owner() -> OwnerCapability {
    OwnerCapability::new([4; 32])
}
fn only(mut effects: Vec<Effect>) -> CommitRequest {
    assert_eq!(effects.len(), 1);
    match effects.remove(0) {
        Effect::Commit(request) => request,
        other => panic!("expected commit, got {other:?}"),
    }
}
fn start(c: &mut Fixture, key: &Key) -> RunPermit {
    let request = only(
        c.submit(
            key.clone(),
            &owner(),
            ObservationWindow::new(10, 7_000).unwrap(),
        )
        .unwrap(),
    );
    let accepted = match c.commit(request).unwrap().remove(0) {
        Effect::Accepted(permit) => permit,
        other => panic!("{other:?}"),
    };
    let request = only(c.begin(accepted, 20, None).unwrap());
    match c.commit(request).unwrap().remove(0) {
        Effect::Run(permit) => permit,
        other => panic!("{other:?}"),
    }
}
fn finish(c: &mut Fixture, key: &Key) -> CanonicalOutcome {
    let run = start(c, key);
    let outcome = CanonicalOutcome::completed(DomainResult::success("physical result"));
    let request = only(c.report(run.finish(outcome.clone())).unwrap());
    assert!(c.commit(request).unwrap().is_empty());
    outcome
}

#[test]
fn authority_fault_blocks_delayed_rights_and_keeps_pending_queued_late_facts() {
    let mut c = Fixture::new(AuthorityEpoch::new([1; 32]));
    let terminal = key();
    let completed = finish(&mut c, &terminal);
    let direct = c.direct(&terminal, &owner()).unwrap();
    let delayed = c
        .prepare_task(&terminal)
        .unwrap()
        .loaded(c.store(&terminal))
        .unwrap();
    let failed_read = c.prepare_task(&terminal).unwrap();
    let revision = failed_read.revision();

    let pending_key = key();
    let pending_run = start(&mut c, &pending_key);
    let pending_body = CanonicalOutcome::completed(DomainResult::success("pending fact"));
    let pending_weak = pending_body.weak_bytes();
    let pending = only(c.report(pending_run.finish(pending_body)).unwrap());

    let queued_key = key();
    let queued_run = start(&mut c, &queued_key);
    let expose = only(c.expose_task(&queued_key).unwrap());
    let queued_body = CanonicalOutcome::completed(DomainResult::success("queued fact"));
    let queued_weak = queued_body.weak_bytes();
    assert!(c.report(queued_run.finish(queued_body)).unwrap().is_empty());

    let late_key = key();
    let late_run = start(&mut c, &late_key);
    let confirmed = c.snapshot();
    let effects = c
        .apply_authority_failure(
            failed_read.authority_failed(AuthorityFailureCause::InvalidAggregate),
        )
        .unwrap();
    assert!(matches!(effects.as_slice(), [Effect::FailStop]));
    let first = c.first_authority_failure().unwrap();
    assert_eq!(first.cause(), AuthorityFailureCause::InvalidAggregate);
    assert_eq!(first.read_context(), Some((&terminal, revision)));
    assert_eq!(c.snapshot(), confirmed);
    assert!(matches!(
        c.complete_read(delayed),
        Err(Error::StoreUnavailable)
    ));
    assert!(matches!(
        c.complete(pending.committed()),
        Err(Error::StoreUnavailable)
    ));
    drop(expose);

    let late_body = CanonicalOutcome::completed(DomainResult::success("late fact"));
    let late_weak = late_body.weak_bytes();
    assert!(matches!(
        c.report(late_run.finish(late_body)).unwrap().as_slice(),
        [Effect::FailStop]
    ));
    assert!(matches!(c.cancel(&late_key), Err(Error::StoreUnavailable)));
    let repeat = c
        .store_authority()
        .failed(AuthorityFailureCause::IdentityLost);
    assert!(c.apply_authority_failure(repeat).unwrap().is_empty());
    assert_eq!(
        c.first_authority_failure().unwrap().cause(),
        AuthorityFailureCause::InvalidAggregate
    );
    assert!(c.first_known_write_failure().is_none());
    assert_eq!(direct.outcome.bytes(), completed.bytes());
    for weak in [&pending_weak, &queued_weak, &late_weak] {
        assert!(weak.upgrade().is_some(), "fail-stop dropped factual bytes");
    }
    drop(c);
    for weak in [&pending_weak, &queued_weak, &late_weak] {
        assert!(
            weak.upgrade().is_none(),
            "control: owner was the final holder"
        );
    }
    assert_eq!(direct.outcome.bytes(), completed.bytes());
}

#[test]
fn foreign_authority_fault_cannot_poison_current_read_or_commit() {
    let mut c = Fixture::new(AuthorityEpoch::new([1; 32]));
    let terminal = key();
    let completed = finish(&mut c, &terminal);
    let delayed = c
        .prepare_task(&terminal)
        .unwrap()
        .loaded(c.store(&terminal))
        .unwrap();
    let foreign = ExecutionCoordinator::new(AuthorityEpoch::new([2; 32]));
    let binding = foreign.store_authority();
    let result = c.apply_authority_failure(binding.failed(AuthorityFailureCause::IdentityLost));
    assert!(
        !c.unavailable(),
        "foreign fault poisoned this owner: {result:?}"
    );
    assert!(matches!(result, Err(Error::StaleCompletion)));
    assert!(c.first_authority_failure().is_none());
    let ReadPublication::Task(publication) = c.complete_read(delayed).unwrap() else {
        panic!("Task read changed role")
    };
    assert_eq!(publication.outcome().bytes(), completed.bytes());
    let expose = only(c.expose_task(&terminal).unwrap());
    assert!(matches!(
        c.commit(expose).unwrap().as_slice(),
        [Effect::Task(_)]
    ));
}

#[test]
fn stale_read_revision_does_not_hide_current_authority_loss() {
    let mut c = Fixture::new(AuthorityEpoch::new([1; 32]));
    let terminal = key();
    finish(&mut c, &terminal);
    let old_read = c.prepare_task(&terminal).unwrap();
    let failed_read = c.prepare_task(&terminal).unwrap();
    let old_revision = failed_read.revision();
    let expose = only(c.expose_task(&terminal).unwrap());
    c.commit(expose).unwrap();
    assert!(c.inspect(&terminal).unwrap().revision() > old_revision);
    assert!(matches!(
        old_read.loaded(c.store(&terminal)),
        Err(Error::StaleCompletion)
    ));
    assert!(!c.unavailable(), "a normal obsolete read is not corruption");
    let effects = c
        .apply_authority_failure(failed_read.authority_failed(AuthorityFailureCause::ReadFailed))
        .unwrap();
    assert!(matches!(effects.as_slice(), [Effect::FailStop]));
    assert_eq!(
        c.first_authority_failure().unwrap().read_context(),
        Some((&terminal, old_revision))
    );
}

#[test]
fn authority_fault_never_replaces_first_known_write_failure() {
    let mut c = Fixture::new(AuthorityEpoch::new([1; 32]));
    let k = key();
    let run = start(&mut c, &k);
    let request = only(
        c.report(run.finish(CanonicalOutcome::completed(DomainResult::success("fact"))))
            .unwrap(),
    );
    c.complete(request.failed_before_publication(WriteFailureCause::NoSpace))
        .unwrap();
    let first = c.first_known_write_failure().unwrap().clone();
    let fault = c
        .store_authority()
        .failed(AuthorityFailureCause::WorkerFailed);
    assert!(c.apply_authority_failure(fault).unwrap().is_empty());
    assert_eq!(c.first_known_write_failure(), Some(&first));
    assert!(c.first_authority_failure().is_none());
}

#[test]
fn explicit_close_requires_finished_work_and_no_pending_storage_fact() {
    let mut c = Fixture::new(AuthorityEpoch::new([1; 32]));
    assert!(c.storage_is_quiescent());
    let k = key();
    let run = start(&mut c, &k);
    assert!(
        !c.storage_is_quiescent(),
        "live work still owns this process"
    );
    let request = only(
        c.report(run.finish(CanonicalOutcome::completed(DomainResult::success("done"))))
            .unwrap(),
    );
    assert!(
        !c.storage_is_quiescent(),
        "unconfirmed outcome can still publish"
    );
    c.commit(request).unwrap();
    assert!(c.storage_is_quiescent());
    let expose = only(c.expose_task(&k).unwrap());
    assert!(!c.storage_is_quiescent());
    c.commit(expose).unwrap();
    assert!(c.storage_is_quiescent());
    let failure = c
        .store_authority()
        .failed(AuthorityFailureCause::IdentityLost);
    c.apply_authority_failure(failure).unwrap();
    assert!(
        !c.storage_is_quiescent(),
        "frozen owners retain ownership until process death"
    );
}
