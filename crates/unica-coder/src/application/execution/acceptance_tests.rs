//! Independent acceptance sequences exercise the application API. The driver
//! acknowledges synthetic commits; it is not evidence of filesystem durability.
use super::*;
use crate::domain::execution::{FailureReason, Tool};
use crate::domain::invocation::DomainResult;

fn key(tool: Tool) -> Key {
    Key::new(
        InvocationId::new(),
        TaskId::new(),
        [1; 32],
        tool,
        [2; 32],
        [3; 32],
    )
}

fn owner(epoch: u8) -> ExecutionCoordinator {
    ExecutionCoordinator::new(AuthorityEpoch::new([epoch; 32]))
}

fn confirm(owner: &mut ExecutionCoordinator, effects: Vec<Effect>) -> Vec<Effect> {
    let mut pending: VecDeque<_> = effects.into();
    let mut observed = Vec::new();
    while let Some(effect) = pending.pop_front() {
        match effect {
            Effect::Commit(request) => pending.extend(owner.complete(request.committed()).unwrap()),
            other => observed.push(other),
        }
    }
    observed
}

fn only_commit(mut effects: Vec<Effect>) -> CommitRequest {
    assert_eq!(effects.len(), 1);
    match effects.remove(0) {
        Effect::Commit(request) => request,
        other => panic!("expected a pending commit, got {other:?}"),
    }
}

fn start(owner: &mut ExecutionCoordinator, key: &Key, capability: &OwnerCapability) -> RunPermit {
    let pending = owner
        .submit(
            key.clone(),
            capability,
            ObservationWindow::new(10, 7_000).unwrap(),
        )
        .unwrap();
    let accepted = take_accepted(confirm(owner, pending));
    let authority = Some(ReadAuthority::new([7; 32], 1, 1));
    let pending = owner.begin(accepted, 20, authority).unwrap();
    let mut effects = confirm(owner, pending);
    match effects.remove(0) {
        Effect::Run(permit) => permit,
        other => panic!("expected RunPermit, got {other:?}"),
    }
}

#[test]
fn acceptance_pending_ids_are_unique_across_groups_before_any_commit() {
    let mut coordinator = owner(1);
    let capability = OwnerCapability::new([4; 32]);
    let first = key(Tool::View);
    let same_task = Key::new(
        InvocationId::new(),
        first.task_id(),
        [1; 32],
        Tool::View,
        [2; 32],
        [3; 32],
    );
    let same_invocation = Key::new(
        first.invocation_id(),
        TaskId::new(),
        [1; 32],
        Tool::View,
        [9; 32],
        [3; 32],
    );
    let pending = coordinator
        .submit(
            first.clone(),
            &capability,
            ObservationWindow::new(10, 0).unwrap(),
        )
        .unwrap();
    assert!(coordinator.lookup_task(first.task_id()).is_err());
    for collision in [same_task, same_invocation] {
        assert!(matches!(
            coordinator.submit(
                collision,
                &capability,
                ObservationWindow::new(10, 0).unwrap()
            ),
            Err(Error::IdentityCollision)
        ));
    }
    let effects = confirm(&mut coordinator, pending);
    assert!(matches!(effects.as_slice(), [Effect::Accepted(_)]));
    assert_eq!(coordinator.catalog().unwrap().records().len(), 1);
}

#[test]
fn acceptance_primary_ack_does_not_drop_an_active_alias_or_factual_result() {
    let mut coordinator = owner(1);
    let capability = OwnerCapability::new([4; 32]);
    let primary = key(Tool::View);
    let run = start(&mut coordinator, &primary, &capability);
    let alias = key(Tool::View);
    let pending = coordinator
        .attach(
            &primary,
            JoinEvidence::new(alias.clone(), ReadAuthority::new([7; 32], 1, 1), 7_010),
        )
        .unwrap();
    let member = match confirm(&mut coordinator, pending).remove(0) {
        Effect::Attached(member) => member,
        other => panic!("{other:?}"),
    };
    let pending = coordinator.cancel(&primary).unwrap();
    assert!(confirm(&mut coordinator, pending).is_empty());
    let cancelled = CanonicalOutcome::cancelled();
    assert_eq!(
        coordinator.inspect(&primary).unwrap().primary(),
        Some(&cancelled)
    );
    let pending = coordinator
        .acknowledge(&primary, &capability, cancelled.digest(), 40)
        .unwrap();
    confirm(&mut coordinator, pending);
    assert!(!coordinator.inspect(&primary).unwrap().compacted());
    assert_eq!(coordinator.holder_count(&primary), 1);
    let factual = CanonicalOutcome::completed(DomainResult::success("producer published"));
    let pending = coordinator.report(run.finish(factual.clone())).unwrap();
    confirm(&mut coordinator, pending);
    let record = coordinator.inspect(&primary).unwrap();
    assert_eq!(record.primary(), Some(&cancelled));
    assert_eq!(record.producer(), Some(&factual));
    assert!(
        matches!(coordinator.observe_holder(&primary, member), Ok(model::HolderObservation::Terminal(outcome)) if outcome == &factual)
    );
    let bytes = coordinator.catalog().unwrap().encode();
    let (mut reopened, effects) = ExecutionCoordinator::restore(
        DurableCatalog::decode(&bytes).unwrap(),
        AuthorityEpoch::new([2; 32]),
    )
    .unwrap();
    confirm(&mut reopened, effects);
    assert_eq!(
        reopened.lookup_task(primary.task_id()).unwrap().primary(),
        Some(&cancelled)
    );
    assert!(reopened.lookup_task(alias.task_id()).is_err());
    assert_eq!(reopened.holder_count(&primary), 0);
}

#[test]
fn acceptance_both_ack_expose_orders_obey_confirmed_public_promises() {
    for expose_first in [false, true] {
        let mut coordinator = owner(1);
        let capability = OwnerCapability::new([4; 32]);
        let invocation = key(Tool::View);
        let run = start(&mut coordinator, &invocation, &capability);
        let factual = CanonicalOutcome::completed(DomainResult::success("stable payload"));
        let pending = coordinator.report(run.finish(factual.clone())).unwrap();
        confirm(&mut coordinator, pending);
        if expose_first {
            let pending = coordinator.expose_task(&invocation).unwrap();
            assert!(pending
                .iter()
                .all(|effect| !matches!(effect, Effect::Task(_))));
            assert!(!coordinator.inspect(&invocation).unwrap().public_task());
            assert!(matches!(
                confirm(&mut coordinator, pending).as_slice(),
                [Effect::Task(_)]
            ));
        }
        let pending = coordinator
            .acknowledge(&invocation, &capability, factual.digest(), 40)
            .unwrap();
        confirm(&mut coordinator, pending);
        if expose_first {
            assert_eq!(
                coordinator
                    .lookup_task(invocation.task_id())
                    .unwrap()
                    .primary(),
                Some(&factual)
            );
        } else {
            assert!(coordinator.inspect(&invocation).unwrap().compacted());
            let pending = coordinator.expose_task(&invocation).unwrap();
            assert!(matches!(
                confirm(&mut coordinator, pending).as_slice(),
                [Effect::Rejected {
                    error: Error::Compacted,
                    ..
                }]
            ));
            assert!(coordinator
                .inspect(&invocation)
                .unwrap()
                .producer()
                .is_none());
        }
        let before = coordinator.catalog().unwrap().encode();
        let pending = coordinator
            .acknowledge(&invocation, &capability, factual.digest(), 40)
            .unwrap();
        assert!(confirm(&mut coordinator, pending).is_empty());
        assert_eq!(coordinator.catalog().unwrap().encode(), before);
    }
}

#[test]
fn acceptance_cancel_and_expose_pending_commits_do_not_discard_factual_mutation() {
    let mut coordinator = owner(1);
    let capability = OwnerCapability::new([4; 32]);
    let invocation = key(Tool::Apply);
    let pending = coordinator
        .submit(
            invocation.clone(),
            &capability,
            ObservationWindow::new(10, 7_000).unwrap(),
        )
        .unwrap();
    let accepted = take_accepted(confirm(&mut coordinator, pending));
    let pending = coordinator.begin(accepted, 20, None).unwrap();
    let run = match confirm(&mut coordinator, pending).remove(0) {
        Effect::Run(run) => run,
        other => panic!("{other:?}"),
    };
    let cancellation = only_commit(coordinator.cancel(&invocation).unwrap());
    assert!(coordinator.expose_task(&invocation).unwrap().is_empty());
    let factual = CanonicalOutcome::completed(DomainResult::success("source already published"));
    assert!(coordinator
        .report(run.finish(factual.clone()))
        .unwrap()
        .is_empty());
    let effects = coordinator.complete(cancellation.committed()).unwrap();
    let observed = confirm(&mut coordinator, effects);
    assert!(observed
        .iter()
        .all(|effect| !matches!(effect, Effect::Run(_))));
    assert_eq!(
        coordinator
            .lookup_task(invocation.task_id())
            .unwrap()
            .primary(),
        Some(&factual)
    );
}

#[test]
fn acceptance_unknown_begin_never_grants_run_and_recovery_never_replays() {
    let mut coordinator = owner(1);
    let capability = OwnerCapability::new([4; 32]);
    let invocation = key(Tool::Apply);
    let pending = coordinator
        .submit(
            invocation.clone(),
            &capability,
            ObservationWindow::new(10, 0).unwrap(),
        )
        .unwrap();
    let accepted = take_accepted(confirm(&mut coordinator, pending));
    let begin = only_commit(coordinator.begin(accepted, 20, None).unwrap());
    // The storage fault fixture says the uncertain write actually reached disk.
    let durable = DurableCatalog::new(vec![begin.candidate().clone()]).unwrap();
    assert!(matches!(
        coordinator.complete(begin.unknown()).unwrap().as_slice(),
        [Effect::FailStop]
    ));
    assert!(coordinator.unavailable());
    let (mut reopened, pending) =
        ExecutionCoordinator::restore(durable, AuthorityEpoch::new([2; 32])).unwrap();
    assert!(reopened.recovering());
    assert!(reopened.lookup_task(invocation.task_id()).is_err());
    let observed = confirm(&mut reopened, pending);
    assert!(observed
        .iter()
        .all(|effect| !matches!(effect, Effect::Run(_) | Effect::Accepted(_))));
    assert_eq!(
        reopened
            .lookup_task(invocation.task_id())
            .unwrap()
            .primary(),
        Some(&CanonicalOutcome::failed(FailureReason::OutcomeUncertain))
    );
}

#[test]
fn acceptance_wrong_epoch_completion_does_not_accept_the_current_pending() {
    let invocation = key(Tool::View);
    let capability = OwnerCapability::new([4; 32]);
    let mut old = owner(1);
    let stale = only_commit(
        old.submit(
            invocation.clone(),
            &capability,
            ObservationWindow::new(10, 0).unwrap(),
        )
        .unwrap(),
    )
    .committed();
    let mut current = owner(2);
    let pending = current
        .submit(
            invocation.clone(),
            &capability,
            ObservationWindow::new(10, 0).unwrap(),
        )
        .unwrap();
    assert!(matches!(
        current.complete(stale),
        Err(Error::StaleCompletion)
    ));
    assert!(current.lookup_task(invocation.task_id()).is_err());
    assert!(matches!(
        confirm(&mut current, pending).as_slice(),
        [Effect::Accepted(_)]
    ));
}

fn take_accepted(mut effects: Vec<Effect>) -> AcceptedPermit {
    assert_eq!(effects.len(), 1);
    match effects.remove(0) {
        Effect::Accepted(permit) => permit,
        other => panic!("{other:?}"),
    }
}

fn task_permit(owner: &mut ExecutionCoordinator, key: &Key) -> TaskPublicationPermit {
    let pending = owner.expose_task(key).unwrap();
    match confirm(owner, pending).remove(0) {
        Effect::Task(permit) => permit,
        other => panic!("{other:?}"),
    }
}

#[test]
fn acceptance_delivery_choice_does_not_ack_a_late_direct_or_lose_built_direct() {
    let mut coordinator = owner(1);
    let capability = OwnerCapability::new([4; 32]);
    let invocation = key(Tool::View);
    let run = start(&mut coordinator, &invocation, &capability);
    let result = CanonicalOutcome::completed(DomainResult::success("prepared"));
    let pending = coordinator.report(run.finish(result.clone())).unwrap();
    confirm(&mut coordinator, pending);
    let late = coordinator
        .prepare_direct(&invocation, &capability)
        .unwrap();
    let ready = coordinator
        .prepare_direct(&invocation, &capability)
        .unwrap();
    let mut task = DeliveryChoice::new(invocation.clone());
    task.return_task(task_permit(&mut coordinator, &invocation))
        .unwrap();
    assert!(task.build_direct(late).is_err());
    assert_eq!(task.ack_digest(), None);
    let mut direct = DeliveryChoice::new(invocation.clone());
    direct.build_direct(ready).unwrap();
    assert_eq!(direct.ack_digest(), Some(result.digest()));
    // A failed ACK does not mutate the observation choice or prepared bytes.
    assert_eq!(direct.direct(), Some(&result));
    assert!(direct
        .return_task(task_permit(&mut coordinator, &invocation))
        .is_err());
    assert_eq!(direct.direct(), Some(&result));
}

#[test]
fn acceptance_writable_capabilities_cannot_be_cloned() {
    trait AmbiguousIfClone<A> {
        fn check() {}
    }
    impl<T: ?Sized> AmbiguousIfClone<()> for T {}
    impl<T: Clone> AmbiguousIfClone<u8> for T {}
    let _ = <RunPermit as AmbiguousIfClone<_>>::check;
    let _ = <DirectPublicationPermit as AmbiguousIfClone<_>>::check;
    let _ = <OutcomeEvidence as AmbiguousIfClone<_>>::check;
    let _ = <AcceptedPermit as AmbiguousIfClone<_>>::check;
    let _ = <TaskPublicationPermit as AmbiguousIfClone<_>>::check;
    let _ = <AdmissionRejection as AmbiguousIfClone<_>>::check;
    let _ = <CancellationPermit as AmbiguousIfClone<_>>::check;
    trait AmbiguousIfDeserializable<A> {
        fn check() {}
    }
    impl<T: ?Sized> AmbiguousIfDeserializable<()> for T {}
    impl<T: serde::de::DeserializeOwned> AmbiguousIfDeserializable<u8> for T {}
    let _ = <RunPermit as AmbiguousIfDeserializable<_>>::check;
    let _ = <DirectPublicationPermit as AmbiguousIfDeserializable<_>>::check;
    let _ = <OutcomeEvidence as AmbiguousIfDeserializable<_>>::check;
    let _ = <AcceptedPermit as AmbiguousIfDeserializable<_>>::check;
    let _ = <TaskPublicationPermit as AmbiguousIfDeserializable<_>>::check;
    let _ = <AdmissionRejection as AmbiguousIfDeserializable<_>>::check;
    let _ = <CancellationPermit as AmbiguousIfDeserializable<_>>::check;
}

fn attach(owner: &mut ExecutionCoordinator, primary: &Key, alias: Key) -> Membership {
    let pending = owner
        .attach(
            primary,
            JoinEvidence::new(alias, ReadAuthority::new([7; 32], 1, 1), 7_010),
        )
        .unwrap();
    match confirm(owner, pending).remove(0) {
        Effect::Attached(member) => member,
        other => panic!("expected membership, got {other:?}"),
    }
}

#[test]
fn acceptance_last_cancel_and_attach_are_ordered_by_confirmed_membership() {
    for attach_first in [false, true] {
        let mut coordinator = owner(1);
        let capability = OwnerCapability::new([4; 32]);
        let primary = key(Tool::View);
        let _run = start(&mut coordinator, &primary, &capability);
        let first = attach(&mut coordinator, &primary, key(Tool::View));
        let pending = coordinator.cancel(&primary).unwrap();
        assert!(confirm(&mut coordinator, pending).is_empty());
        let joining = JoinEvidence::new(key(Tool::View), ReadAuthority::new([7; 32], 1, 1), 7_010);
        let pending = if attach_first {
            let pending = coordinator.attach(&primary, joining).unwrap();
            assert!(coordinator
                .cancel_holder(&primary, first)
                .unwrap()
                .is_empty());
            pending
        } else {
            let pending = coordinator.cancel_holder(&primary, first).unwrap();
            assert!(coordinator.attach(&primary, joining).unwrap().is_empty());
            pending
        };
        let effects = confirm(&mut coordinator, pending);
        assert_eq!(
            effects
                .iter()
                .filter(|e| matches!(e, Effect::CancelRequested(_)))
                .count(),
            usize::from(!attach_first)
        );
        assert_eq!(
            effects
                .iter()
                .filter(|e| matches!(e, Effect::Attached(_)))
                .count(),
            usize::from(attach_first)
        );
        assert_eq!(
            effects
                .iter()
                .filter(|e| matches!(
                    e,
                    Effect::Rejected {
                        error: Error::NotJoinable,
                        ..
                    }
                ))
                .count(),
            usize::from(!attach_first)
        );
        assert_eq!(
            coordinator.holder_count(&primary),
            if attach_first { 2 } else { 1 }
        );
    }
}

#[test]
fn acceptance_retired_membership_cannot_cancel_its_successor() {
    let mut coordinator = owner(1);
    let capability = OwnerCapability::new([4; 32]);
    let primary = key(Tool::View);
    let run = start(&mut coordinator, &primary, &capability);
    let alias = key(Tool::View);
    let old = attach(&mut coordinator, &primary, alias.clone());
    let pending = coordinator.retire_holder(&primary, old).unwrap();
    confirm(&mut coordinator, pending);
    let current = attach(&mut coordinator, &primary, alias);
    let pending = coordinator.cancel_holder(&primary, old).unwrap();
    assert!(matches!(
        confirm(&mut coordinator, pending).as_slice(),
        [Effect::Rejected {
            error: Error::StaleMembership,
            ..
        }]
    ));
    assert!(matches!(
        coordinator.observe_holder(&primary, old),
        Err(Error::StaleMembership)
    ));
    assert!(matches!(
        coordinator.observe_holder(&primary, current),
        Ok(model::HolderObservation::Working)
    ));
    let factual = CanonicalOutcome::completed(DomainResult::success("still owned"));
    let pending = coordinator.report(run.finish(factual.clone())).unwrap();
    confirm(&mut coordinator, pending);
    assert!(
        matches!(coordinator.observe_holder(&primary, current), Ok(model::HolderObservation::Terminal(outcome)) if outcome == &factual)
    );
}

const ORDERS: [[usize; 3]; 6] = [
    [0, 1, 2],
    [0, 2, 1],
    [1, 0, 2],
    [1, 2, 0],
    [2, 0, 1],
    [2, 1, 0],
];

#[test]
fn acceptance_submit_cancel_begin_permutations_never_replay() {
    // Begin can be requested only after an AcceptedPermit is delivered. In
    // delayed schedules a requested Begin has no capability and cannot enqueue.
    for order in ORDERS {
        for defer in [false, true] {
            let mut coordinator = owner(1);
            let capability = OwnerCapability::new([4; 32]);
            let invocation = key(Tool::Apply);
            let mut effects = Vec::new();
            let mut accepted = None;
            for event in order {
                let next = match event {
                    0 => coordinator.submit(
                        invocation.clone(),
                        &capability,
                        ObservationWindow::new(10, 0).unwrap(),
                    ),
                    1 => coordinator.cancel(&invocation),
                    2 => match accepted.take() {
                        Some(permit) => coordinator.begin(permit, 20, None),
                        None => continue,
                    },
                    _ => unreachable!(),
                }
                .unwrap();
                if defer {
                    effects.extend(next);
                } else {
                    for effect in confirm(&mut coordinator, next) {
                        match effect {
                            Effect::Accepted(permit) => accepted = Some(permit),
                            other => effects.push(other),
                        }
                    }
                }
            }
            let observed = confirm(&mut coordinator, effects);
            assert_eq!(
                observed
                    .iter()
                    .filter(|e| matches!(e, Effect::Run(_)))
                    .count(),
                usize::from(!defer && order == [0, 2, 1]),
                "order={order:?}, defer={defer}"
            );
            let (mut reopened, effects) = ExecutionCoordinator::restore(
                coordinator.catalog().unwrap(),
                AuthorityEpoch::new([2; 32]),
            )
            .unwrap();
            assert!(confirm(&mut reopened, effects)
                .iter()
                .all(|e| !matches!(e, Effect::Run(_))));
        }
    }
}

#[test]
fn acceptance_cancel_outcome_expose_permutations_preserve_factual_mutation() {
    // Once Begin commits, cancellation is intent; it cannot replace the
    // already-produced external mutation's result in any command order.
    for order in ORDERS {
        for defer in [false, true] {
            let mut coordinator = owner(1);
            let capability = OwnerCapability::new([4; 32]);
            let invocation = key(Tool::Apply);
            let pending = coordinator
                .submit(
                    invocation.clone(),
                    &capability,
                    ObservationWindow::new(10, 0).unwrap(),
                )
                .unwrap();
            let accepted = take_accepted(confirm(&mut coordinator, pending));
            let pending = coordinator.begin(accepted, 20, None).unwrap();
            let run = match confirm(&mut coordinator, pending).remove(0) {
                Effect::Run(run) => run,
                other => panic!("{other:?}"),
            };
            let factual =
                CanonicalOutcome::completed(DomainResult::success("external mutation complete"));
            let mut evidence = Some(run.finish(factual.clone()));
            let mut effects = Vec::new();
            for event in order {
                let next = match event {
                    0 => coordinator.cancel(&invocation),
                    1 => coordinator.report(evidence.take().unwrap()),
                    2 => coordinator.expose_task(&invocation),
                    _ => unreachable!(),
                }
                .unwrap();
                if defer {
                    effects.extend(next);
                } else {
                    effects.extend(confirm(&mut coordinator, next));
                }
            }
            let observed = confirm(&mut coordinator, effects);
            assert!(
                observed.iter().all(|e| !matches!(
                    e,
                    Effect::Run(_) | Effect::FailStop | Effect::Rejected { .. }
                )),
                "order={order:?}, defer={defer}"
            );
            let pending = coordinator
                .acknowledge(&invocation, &capability, factual.digest(), 40)
                .unwrap();
            confirm(&mut coordinator, pending);
            assert_eq!(
                coordinator
                    .lookup_task(invocation.task_id())
                    .unwrap()
                    .primary(),
                Some(&factual),
                "order={order:?}, defer={defer}"
            );
            let (mut reopened, effects) = ExecutionCoordinator::restore(
                coordinator.catalog().unwrap(),
                AuthorityEpoch::new([2; 32]),
            )
            .unwrap();
            assert!(confirm(&mut reopened, effects)
                .iter()
                .all(|e| !matches!(e, Effect::Run(_))));
            assert_eq!(
                reopened
                    .lookup_task(invocation.task_id())
                    .unwrap()
                    .primary(),
                Some(&factual)
            );
        }
    }
}

#[test]
fn acceptance_observer_and_wrong_owner_cannot_release_primary_payload() {
    let mut coordinator = owner(1);
    let capability = OwnerCapability::new([4; 32]);
    let invocation = key(Tool::View);
    let run = start(&mut coordinator, &invocation, &capability);
    let factual = CanonicalOutcome::completed(DomainResult::success("owner-only acknowledgement"));
    let pending = coordinator.report(run.finish(factual.clone())).unwrap();
    confirm(&mut coordinator, pending);
    let before = coordinator.catalog().unwrap().encode();
    assert_eq!(
        coordinator
            .lookup_task(invocation.task_id())
            .unwrap()
            .primary(),
        Some(&factual)
    );
    assert_eq!(coordinator.holder_count(&invocation), 0);
    let wrong_owner = OwnerCapability::new([5; 32]);
    for (cap, digest, expected) in [
        (&wrong_owner, factual.digest(), Error::OwnerMismatch),
        (&capability, [0; 32], Error::DigestMismatch),
    ] {
        let pending = coordinator
            .acknowledge(&invocation, cap, digest, 40)
            .unwrap();
        assert!(
            matches!(confirm(&mut coordinator, pending).as_slice(), [Effect::Rejected { error, .. }] if *error == expected)
        );
        assert_eq!(coordinator.catalog().unwrap().encode(), before);
    }
    let pending = coordinator
        .submit(
            invocation.clone(),
            &wrong_owner,
            ObservationWindow::new(10, 0).unwrap(),
        )
        .unwrap();
    assert!(matches!(
        confirm(&mut coordinator, pending).as_slice(),
        [Effect::Rejected {
            error: Error::OwnerMismatch,
            ..
        }]
    ));
    assert_eq!(coordinator.catalog().unwrap().encode(), before);
}
