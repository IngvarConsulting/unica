//! Public-API sequences for recovery, competing delivery transitions and join
//! rejection. Confirming a synthetic commit proves the P1 model, not storage.
use super::test_support::{Archive as DurableCatalog, Fixture as ExecutionCoordinator};
use super::*;
use crate::domain::execution::{FailureReason, HolderObservation, Tool};
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

fn coordinator(epoch: u8) -> ExecutionCoordinator {
    ExecutionCoordinator::new(AuthorityEpoch::new([epoch; 32]))
}

fn authority() -> ReadAuthority {
    ReadAuthority::new([7; 32], 1, 1)
}

fn window() -> ObservationWindow {
    ObservationWindow::new(10, 7_000).unwrap()
}

fn confirm(owner: &mut ExecutionCoordinator, effects: Vec<Effect>) -> Vec<Effect> {
    let mut pending: VecDeque<_> = effects.into();
    let mut observed = Vec::new();
    while let Some(effect) = pending.pop_front() {
        match effect {
            Effect::Commit(request) => pending.extend(owner.commit(request).unwrap()),
            other => observed.push(other),
        }
    }
    observed
}

fn only_commit(mut effects: Vec<Effect>) -> CommitRequest {
    assert_eq!(effects.len(), 1);
    match effects.remove(0) {
        Effect::Commit(request) => request,
        other => panic!("expected one pending commit, got {other:?}"),
    }
}

fn accept(
    owner: &mut ExecutionCoordinator,
    key: &Key,
    capability: &OwnerCapability,
    window: ObservationWindow,
) -> AcceptedPermit {
    let pending = owner.submit(key.clone(), capability, window).unwrap();
    let mut effects = confirm(owner, pending);
    assert_eq!(effects.len(), 1);
    match effects.remove(0) {
        Effect::Accepted(permit) => permit,
        other => panic!("expected one AcceptedPermit, got {other:?}"),
    }
}

fn start(
    owner: &mut ExecutionCoordinator,
    key: &Key,
    capability: &OwnerCapability,
    window: ObservationWindow,
) -> RunPermit {
    let accepted = accept(owner, key, capability, window);
    let pending = owner.begin(accepted, 20, Some(authority())).unwrap();
    let mut effects = confirm(owner, pending);
    assert_eq!(effects.len(), 1);
    match effects.remove(0) {
        Effect::Run(permit) => permit,
        other => panic!("expected one RunPermit, got {other:?}"),
    }
}

fn attach(owner: &mut ExecutionCoordinator, primary: &Key, alias: Key) -> Membership {
    let pending = owner
        .attach(primary, JoinEvidence::new(alias, authority(), 7_010))
        .unwrap();
    let mut effects = confirm(owner, pending);
    assert_eq!(effects.len(), 1);
    match effects.remove(0) {
        Effect::Attached(member) => member,
        other => panic!("expected one Membership, got {other:?}"),
    }
}

fn assert_rejected(effects: &[Effect], key: &Key, expected: Error) {
    assert!(
        matches!(
            effects,
            [Effect::Rejected { key: rejected_key, error }]
                if rejected_key == key && error == &expected
        ),
        "expected {expected:?}, got {effects:?}"
    );
}

#[test]
fn acceptance_ineligible_joins_preserve_state_and_release_both_alias_ids() {
    #[derive(Clone, Copy, Debug)]
    enum Mismatch {
        Core,
        Tool,
        Arguments,
        Scope,
        Source,
        ActorGeneration,
        MutationGeneration,
        BeforeWindow,
        BeforeStart,
        Expired,
    }
    for mismatch in [
        Mismatch::Core,
        Mismatch::Tool,
        Mismatch::Arguments,
        Mismatch::Scope,
        Mismatch::Source,
        Mismatch::ActorGeneration,
        Mismatch::MutationGeneration,
        Mismatch::BeforeWindow,
        Mismatch::BeforeStart,
        Mismatch::Expired,
    ] {
        let mut owner = coordinator(1);
        let capability = OwnerCapability::new([4; 32]);
        let primary = key();
        let observation_window = if matches!(mismatch, Mismatch::BeforeStart) {
            ObservationWindow::new(10, 0).unwrap()
        } else {
            window()
        };
        let _run = start(&mut owner, &primary, &capability, observation_window);
        let alias = key();
        let mut core = [1; 32];
        let mut tool = Tool::View;
        let mut arguments = [2; 32];
        let mut scope = [3; 32];
        let mut read_authority = authority();
        let mut observed = 7_010;
        match mismatch {
            Mismatch::Core => core = [9; 32],
            Mismatch::Tool => tool = Tool::Search,
            Mismatch::Arguments => arguments = [9; 32],
            Mismatch::Scope => scope = [9; 32],
            Mismatch::Source => read_authority = ReadAuthority::new([8; 32], 1, 1),
            Mismatch::ActorGeneration => read_authority = ReadAuthority::new([7; 32], 2, 1),
            Mismatch::MutationGeneration => read_authority = ReadAuthority::new([7; 32], 1, 2),
            Mismatch::BeforeWindow => observed = 7_009,
            Mismatch::BeforeStart => observed = 19,
            Mismatch::Expired => observed = 30_021,
        }
        let ineligible_alias = Key::new(
            alias.invocation_id(),
            alias.task_id(),
            core,
            tool,
            arguments,
            scope,
        );
        let before = owner.catalog().unwrap().encode();
        let effects = owner
            .attach(
                &primary,
                JoinEvidence::new(ineligible_alias, read_authority, observed),
            )
            .unwrap();
        assert_rejected(&effects, &primary, Error::NotJoinable);
        assert_eq!(owner.catalog().unwrap().encode(), before, "{mismatch:?}");
        assert_eq!(owner.holder_count(&primary), 0, "{mismatch:?}");

        // Reusing BOTH identifiers with eligible evidence would collide if the
        // rejected tentative claim leaked from either index.
        let pending = only_commit(
            owner
                .attach(
                    &primary,
                    JoinEvidence::new(alias.clone(), authority(), 7_010),
                )
                .unwrap(),
        );
        assert_eq!(owner.holder_count(&primary), 0);
        assert_eq!(owner.catalog().unwrap().encode(), before);
        let effects = owner.commit(pending).unwrap();
        let member = match effects.as_slice() {
            [Effect::Attached(member)] => *member,
            other => panic!("eligible retry after {mismatch:?}: {other:?}"),
        };
        assert_eq!(member.alias_id(), alias.task_id());
        assert_eq!(owner.holder_count(&primary), 1);
        assert!(matches!(
            owner.observe_holder(&primary, member),
            Ok(HolderObservation::Working)
        ));
    }
}

#[test]
fn acceptance_pending_ack_and_expose_follow_commit_order_without_resurrection() {
    for expose_first in [true, false] {
        let mut owner = coordinator(1);
        let capability = OwnerCapability::new([4; 32]);
        let primary = key();
        let run = start(&mut owner, &primary, &capability, window());
        let factual = CanonicalOutcome::completed(DomainResult::success("retained bytes"));
        let pending = owner.report(run.finish(factual.clone())).unwrap();
        assert!(confirm(&mut owner, pending).is_empty());
        let before = owner.catalog().unwrap().encode();
        let first = if expose_first {
            let pending = only_commit(owner.expose_task(&primary).unwrap());
            assert!(owner
                .acknowledge(&primary, &capability, factual.digest(), 9_000)
                .unwrap()
                .is_empty());
            pending
        } else {
            let pending = only_commit(
                owner
                    .acknowledge(&primary, &capability, factual.digest(), 9_000)
                    .unwrap(),
            );
            assert!(owner.expose_task(&primary).unwrap().is_empty());
            pending
        };
        assert_eq!(owner.catalog().unwrap().encode(), before);
        assert!(!owner.inspect(&primary).unwrap().public_task());
        assert!(owner.inspect(&primary).unwrap().acknowledgement().is_none());
        let effects = owner.commit(first).unwrap();
        let observed = confirm(&mut owner, effects);
        let record = owner.lookup_task(primary.task_id()).unwrap();
        if expose_first {
            assert!(
                matches!(observed.as_slice(), [Effect::Task(permit)] if permit.key() == &primary)
            );
            assert!(record.public_task());
            assert!(!record.compacted());
            assert_eq!(record.producer(), Some(&factual.descriptor()));
            assert_eq!(record.primary(), Some(&factual.descriptor()));
        } else {
            assert_rejected(&observed, &primary, Error::Compacted);
            assert!(!record.public_task());
            assert!(record.compacted());
            assert!(owner.store(&primary).producer().is_none());
            assert!(owner.store(&primary).primary().is_none());
        }
        let ack = record.acknowledgement().unwrap();
        assert_eq!(ack.digest(), factual.digest());
        assert_eq!(ack.first_acknowledged_epoch_ms(), 9_000);
        let after = owner.catalog().unwrap().encode();
        let duplicate = owner
            .acknowledge(&primary, &capability, factual.digest(), 10_000)
            .unwrap();
        assert!(duplicate.is_empty());
        assert_eq!(owner.catalog().unwrap().encode(), after);
    }
}

#[test]
fn acceptance_restore_unbegun_requires_recovery_commit_and_never_regrants_run() {
    let mut old = coordinator(1);
    let capability = OwnerCapability::new([4; 32]);
    let primary = key();
    let stale_accepted = accept(&mut old, &primary, &capability, window());
    let before = old.catalog().unwrap().encode();
    let (mut reopened, effects) = ExecutionCoordinator::restore(
        DurableCatalog::decode(&before).unwrap(),
        AuthorityEpoch::new([2; 32]),
    )
    .unwrap();
    let recovery = only_commit(effects);
    assert!(reopened.recovering());
    assert_eq!(reopened.catalog().unwrap().encode(), before);
    assert!(reopened.inspect(&primary).unwrap().primary().is_none());
    assert!(matches!(
        reopened.lookup_task(primary.task_id()),
        Err(Error::RecoveryRequired)
    ));
    assert!(matches!(
        reopened.submit(primary.clone(), &capability, window()),
        Err(Error::RecoveryRequired)
    ));
    assert!(reopened.commit(recovery).unwrap().is_empty());
    assert!(!reopened.recovering());
    let interrupted = CanonicalOutcome::failed(FailureReason::Interrupted);
    let record = reopened.lookup_task(primary.task_id()).unwrap();
    assert_eq!(record.primary(), Some(&interrupted.descriptor()));
    assert_eq!(record.producer(), Some(&interrupted.descriptor()));
    assert!(record.run().is_none());
    assert!(!record.public_task());
    let recovered = reopened.catalog().unwrap().encode();
    assert!(matches!(
        reopened.begin(stale_accepted, 20, Some(authority())),
        Err(Error::ForeignOutcome)
    ));
    assert!(reopened
        .submit(primary.clone(), &capability, window())
        .unwrap()
        .is_empty());
    assert_eq!(reopened.catalog().unwrap().encode(), recovered);
    let (again, effects) = ExecutionCoordinator::restore(
        DurableCatalog::decode(&recovered).unwrap(),
        AuthorityEpoch::new([3; 32]),
    )
    .unwrap();
    assert!(effects.is_empty());
    assert!(!again.recovering());
    assert_eq!(again.catalog().unwrap().encode(), recovered);
}

#[test]
fn acceptance_restore_cancel_only_preserves_refusal_without_a_task_promise() {
    let mut old = coordinator(1);
    let capability = OwnerCapability::new([4; 32]);
    let primary = key();
    let pending = old.cancel(&primary).unwrap();
    assert!(confirm(&mut old, pending).is_empty());
    let before = old.catalog().unwrap().encode();
    let (mut reopened, effects) = ExecutionCoordinator::restore(
        DurableCatalog::decode(&before).unwrap(),
        AuthorityEpoch::new([2; 32]),
    )
    .unwrap();
    assert!(effects.is_empty());
    assert!(!reopened.recovering());
    assert!(!reopened.inspect(&primary).unwrap().accepted());
    assert!(!reopened.inspect(&primary).unwrap().public_task());
    assert!(matches!(
        reopened.lookup_task(primary.task_id()),
        Err(Error::NotAccepted)
    ));
    assert_rejected(
        &reopened.expose_task(&primary).unwrap(),
        &primary,
        Error::NotAccepted,
    );
    assert_eq!(reopened.catalog().unwrap().encode(), before);
    let submission = only_commit(
        reopened
            .submit(primary.clone(), &capability, window())
            .unwrap(),
    );
    assert!(matches!(
        reopened.lookup_task(primary.task_id()),
        Err(Error::NotAccepted)
    ));
    assert!(reopened.commit(submission).unwrap().is_empty());
    let record = reopened.lookup_task(primary.task_id()).unwrap();
    assert_eq!(
        record.primary(),
        Some(&CanonicalOutcome::cancelled().descriptor())
    );
    assert!(record.run().is_none());
    assert!(!record.public_task());
    let cancelled = reopened.catalog().unwrap().encode();
    assert!(reopened
        .submit(primary, &capability, window())
        .unwrap()
        .is_empty());
    assert_eq!(reopened.catalog().unwrap().encode(), cancelled);
}

#[test]
fn acceptance_restore_compacted_ack_keeps_dedup_ids_and_first_ack_time() {
    let mut old = coordinator(1);
    let capability = OwnerCapability::new([4; 32]);
    let primary = key();
    let run = start(&mut old, &primary, &capability, window());
    let factual = CanonicalOutcome::completed(DomainResult::success("direct result"));
    let pending = old.report(run.finish(factual.clone())).unwrap();
    assert!(confirm(&mut old, pending).is_empty());
    let pending = old
        .acknowledge(&primary, &capability, factual.digest(), 9_000)
        .unwrap();
    assert!(confirm(&mut old, pending).is_empty());
    assert!(old.inspect(&primary).unwrap().compacted());
    let before = old.catalog().unwrap().encode();
    let (mut reopened, effects) = ExecutionCoordinator::restore(
        DurableCatalog::decode(&before).unwrap(),
        AuthorityEpoch::new([2; 32]),
    )
    .unwrap();
    assert!(effects.is_empty());
    assert!(!reopened.recovering());
    assert!(reopened
        .submit(primary.clone(), &capability, window())
        .unwrap()
        .is_empty());
    assert_rejected(
        &reopened.expose_task(&primary).unwrap(),
        &primary,
        Error::Compacted,
    );
    let foreign_owner = OwnerCapability::new([5; 32]);
    assert_rejected(
        &reopened
            .submit(primary.clone(), &foreign_owner, window())
            .unwrap(),
        &primary,
        Error::OwnerMismatch,
    );
    for collision in [
        Key::new(
            primary.invocation_id(),
            TaskId::new(),
            [1; 32],
            Tool::View,
            [2; 32],
            [3; 32],
        ),
        Key::new(
            InvocationId::new(),
            primary.task_id(),
            [1; 32],
            Tool::View,
            [2; 32],
            [3; 32],
        ),
    ] {
        assert!(matches!(
            reopened.submit(collision, &capability, window()),
            Err(Error::IdentityCollision)
        ));
    }
    assert!(reopened
        .acknowledge(&primary, &capability, factual.digest(), 10_000)
        .unwrap()
        .is_empty());
    let record = reopened.lookup_task(primary.task_id()).unwrap();
    assert!(record.compacted());
    assert!(!record.public_task());
    assert!(reopened.store(&primary).producer().is_none());
    assert!(reopened.store(&primary).primary().is_none());
    assert_eq!(record.acknowledgement().unwrap().digest(), factual.digest());
    assert_eq!(
        record
            .acknowledgement()
            .unwrap()
            .first_acknowledged_epoch_ms(),
        9_000
    );
    assert_eq!(reopened.catalog().unwrap().encode(), before);
}

#[test]
fn acceptance_valid_readback_cannot_replace_confirmed_owner_or_window() {
    for (different_owner, different_window) in [(true, false), (false, true), (false, false)] {
        let mut owner = coordinator(1);
        let capability = OwnerCapability::new([4; 32]);
        let primary = key();
        let _accepted = accept(&mut owner, &primary, &capability, window());
        let before = owner.catalog().unwrap().encode();
        let pending = only_commit(owner.expose_task(&primary).unwrap());

        // Produce a structurally valid competing record through the same public
        // API, with a strictly later revision than the confirmed local record.
        let mut other = coordinator(2);
        let other_capability = OwnerCapability::new([if different_owner { 5 } else { 4 }; 32]);
        let other_window = if different_window {
            ObservationWindow::new(11, 7_000).unwrap()
        } else {
            window()
        };
        let _other_accepted = accept(&mut other, &primary, &other_capability, other_window);
        let publication = other.expose_task(&primary).unwrap();
        assert!(matches!(
            confirm(&mut other, publication).as_slice(),
            [Effect::Task(_)]
        ));
        let actual = StoredRecord::decode(&other.store(&primary).encode()).unwrap();
        assert!(actual.record().revision() > owner.inspect(&primary).unwrap().revision());
        assert_eq!(actual.record().revision(), pending.candidate().revision());
        let effects = owner.readback(pending, actual).unwrap();
        if different_owner || different_window {
            assert!(matches!(effects.as_slice(), [Effect::FailStop]));
            assert!(owner.unavailable());
            assert_eq!(owner.catalog().unwrap().encode(), before);
            assert!(matches!(
                owner.lookup_task(primary.task_id()),
                Err(Error::StoreUnavailable)
            ));
            assert!(matches!(
                owner.submit(primary.clone(), &capability, window()),
                Err(Error::StoreUnavailable)
            ));
        } else {
            assert!(
                matches!(effects.as_slice(), [Effect::Task(permit)] if permit.key() == &primary)
            );
            assert!(!owner.unavailable());
            assert!(owner.lookup_task(primary.task_id()).unwrap().public_task());
            assert_eq!(
                owner.catalog().unwrap().encode(),
                other.catalog().unwrap().encode()
            );
        }
    }
}

#[test]
fn acceptance_membership_from_another_producer_cannot_cancel_or_retire_same_generation() {
    let mut owner = coordinator(1);
    let capability = OwnerCapability::new([4; 32]);
    let first = key();
    let second = key();
    let first_run = start(&mut owner, &first, &capability, window());
    let second_run = start(&mut owner, &second, &capability, window());
    assert_ne!(
        first_run.run_identity().attempt(),
        second_run.run_identity().attempt()
    );
    let alias = key();
    let stale = attach(&mut owner, &first, alias.clone());
    let pending = owner.retire_holder(&first, stale).unwrap();
    assert!(confirm(&mut owner, pending).is_empty());
    assert_eq!(owner.holder_count(&first), 0);
    let current = attach(&mut owner, &second, alias);
    assert_eq!(stale.alias_id(), current.alias_id());
    assert_eq!(stale.generation(), current.generation());
    let before = owner.catalog().unwrap().encode();
    assert!(matches!(
        owner.observe_holder(&second, stale),
        Err(Error::StaleMembership)
    ));
    assert_rejected(
        &owner.cancel_holder(&second, stale).unwrap(),
        &second,
        Error::StaleMembership,
    );
    assert_rejected(
        &owner.retire_holder(&second, stale).unwrap(),
        &second,
        Error::StaleMembership,
    );
    assert_eq!(owner.catalog().unwrap().encode(), before);
    assert_eq!(owner.holder_count(&second), 1);
    assert!(matches!(
        owner.observe_holder(&second, current),
        Ok(HolderObservation::Working)
    ));
    let pending = owner.cancel_holder(&second, current).unwrap();
    assert!(confirm(&mut owner, pending).is_empty());
    assert!(matches!(
        owner.observe_holder(&second, current),
        Ok(HolderObservation::Cancelled)
    ));
    let pending = owner.retire_holder(&second, current).unwrap();
    assert!(confirm(&mut owner, pending).is_empty());
    assert_eq!(owner.holder_count(&second), 0);
}
