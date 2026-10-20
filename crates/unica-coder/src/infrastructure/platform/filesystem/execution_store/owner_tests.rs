use crate::application::execution::hydration::ReadPublication;
use crate::application::execution::{
    AuthorityEpoch, AuthorityFailureCause, CommitRequest, DeliveryChoice, Effect,
    ExecutionCoordinator, OwnerCapability, RunPermit,
};
use crate::domain::execution::{digest, CanonicalOutcome, Error, Key, ObservationWindow, Tool};
use crate::domain::invocation::{DomainResult, InvocationId, TaskId};
use crate::infrastructure::execution_store::file::OpeningStore;
use crate::infrastructure::execution_store::namespace::{
    hex, AuthorityRoot, NamespaceSpec, ResolvedNamespace,
};
use crate::infrastructure::execution_store::owner::{
    test_support, OwnerError, PrototypeStoreOwner,
};
use crate::infrastructure::execution_store::{OpenError, Rejection};
use crate::infrastructure::platform::filesystem::execution_store::PublicationPoint;
use std::path::PathBuf;

struct Fixture {
    _temp: tempfile::TempDir,
    root: AuthorityRoot,
    records: PathBuf,
    spec: NamespaceSpec,
}
impl Fixture {
    fn create() -> (Self, PrototypeStoreOwner) {
        let temp = tempfile::tempdir().unwrap();
        let parent = std::fs::canonicalize(temp.path()).unwrap();
        let identity = digest(uuid::Uuid::new_v4().as_bytes());
        let path = parent.join("authority");
        let root = AuthorityRoot::provision(&path, identity).unwrap();
        let spec = NamespaceSpec::new([1; 32], [2; 32]);
        let owner = Self::open(&root, spec.clone(), AuthorityEpoch::new([3; 32]));
        let child = std::fs::read_dir(path.join("namespaces"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        (
            Self {
                _temp: temp,
                root,
                records: child.join("records"),
                spec,
            },
            owner,
        )
    }
    fn open(
        root: &AuthorityRoot,
        spec: NamespaceSpec,
        epoch: AuthorityEpoch,
    ) -> PrototypeStoreOwner {
        let opening = match root.resolve(spec).unwrap() {
            ResolvedNamespace::Fresh(ticket) => OpeningStore::create_new(ticket, epoch).unwrap(),
            ResolvedNamespace::Existing(registration) => {
                OpeningStore::open_existing(registration, epoch).unwrap()
            }
        };
        PrototypeStoreOwner::finish_opening(opening).unwrap()
    }
    fn row(&self, key: &Key) -> PathBuf {
        self.records
            .join(format!("{}.aggregate", hex(&key.digest())))
    }
}
fn key() -> Key {
    Key::new(
        InvocationId::new(),
        TaskId::new(),
        [4; 32],
        Tool::Apply,
        [5; 32],
        [6; 32],
    )
}
fn capability() -> OwnerCapability {
    OwnerCapability::new([7; 32])
}
fn only(mut effects: Vec<Effect>) -> CommitRequest {
    assert_eq!(effects.len(), 1);
    match effects.remove(0) {
        Effect::Commit(request) => request,
        other => panic!("{other:?}"),
    }
}
fn start(owner: &mut PrototypeStoreOwner, key: &Key) -> RunPermit {
    let request = only(
        owner
            .submit(
                key.clone(),
                &capability(),
                ObservationWindow::new(10, 7_000).unwrap(),
            )
            .unwrap(),
    );
    let accepted = match owner.commit(request).unwrap().remove(0) {
        Effect::Accepted(permit) => permit,
        other => panic!("{other:?}"),
    };
    let request = only(owner.begin(accepted, 20, None).unwrap());
    match owner.commit(request).unwrap().remove(0) {
        Effect::Run(permit) => permit,
        other => panic!("{other:?}"),
    }
}
fn finish(owner: &mut PrototypeStoreOwner, key: &Key) -> CanonicalOutcome {
    let run = start(owner, key);
    let outcome = CanonicalOutcome::completed(DomainResult::success("actual worker result"));
    let request = only(owner.report(run.finish(outcome.clone())).unwrap());
    assert!(owner.commit(request).unwrap().is_empty());
    outcome
}

#[test]
fn physical_read_fault_precedes_delayed_publication_and_preserves_built_direct() {
    let (fixture, mut owner) = Fixture::create();
    let k = key();
    let factual = finish(&mut owner, &k);
    let request = owner.model().prepare_direct(&k, &capability()).unwrap();
    let ReadPublication::Direct(permit) = owner.read(request).unwrap() else {
        panic!("Direct role")
    };
    let mut delivery = DeliveryChoice::new(k.clone());
    delivery.build_direct(permit).unwrap();
    let delayed_request = owner.model().prepare_task(&k).unwrap();
    let delayed = test_support::read_loaded(&mut owner, delayed_request).unwrap();
    let failing_request = owner.model().prepare_task(&k).unwrap();
    // External corruption is the fixture. The actual worker detects it through
    // the physical adapter; the test never fabricates an AuthorityFailure.
    std::fs::write(fixture.row(&k), [b'!'; 128]).unwrap();
    assert!(matches!(
        owner.read(failing_request),
        Err(OwnerError::Model(Error::StoreUnavailable))
    ));
    assert!(owner.model().unavailable());
    assert_eq!(
        owner.model().first_authority_failure().unwrap().cause(),
        AuthorityFailureCause::InvalidAggregate
    );
    assert!(matches!(
        test_support::publish_loaded(&mut owner, delayed),
        Err(OwnerError::Model(Error::StoreUnavailable))
    ));
    assert!(matches!(owner.cancel(&k), Err(Error::StoreUnavailable)));
    assert_eq!(delivery.direct().unwrap().bytes(), factual.bytes());
}

#[test]
fn same_owner_successor_makes_read_obsolete_without_poisoning_or_new_execution() {
    let (fixture, mut owner) = Fixture::create();
    let k = key();
    let factual = finish(&mut owner, &k);
    let old = owner.model().prepare_task(&k).unwrap();
    let request = only(owner.expose_task(&k).unwrap());
    assert!(matches!(
        owner.commit(request).unwrap().as_slice(),
        [Effect::Task(_)]
    ));
    assert!(matches!(
        owner.read(old),
        Err(OwnerError::ObsoleteRead { .. })
    ));
    assert!(!owner.model().unavailable());
    let request = owner.model().prepare_task(&k).unwrap();
    let ReadPublication::Task(result) = owner.read(request).unwrap() else {
        panic!("Task role")
    };
    assert_eq!(result.outcome().bytes(), factual.bytes());
    owner.close_quiescent().unwrap();
    let mut reopened = Fixture::open(
        &fixture.root,
        fixture.spec.clone(),
        AuthorityEpoch::new([8; 32]),
    );
    let request = reopened.model().prepare_task(&k).unwrap();
    let ReadPublication::Task(result) = reopened.read(request).unwrap() else {
        panic!("Task role")
    };
    assert_eq!(result.outcome().bytes(), factual.bytes());
    reopened.close_quiescent().unwrap();
}

#[test]
fn foreign_read_is_rejected_before_io_and_healthy_owner_keeps_serving() {
    let (fixture, mut owner) = Fixture::create();
    let k = key();
    finish(&mut owner, &k);
    let foreign_key = key();
    let mut other = Fixture::open(
        &fixture.root,
        NamespaceSpec::new([9; 32], [2; 32]),
        AuthorityEpoch::new([10; 32]),
    );
    finish(&mut other, &foreign_key);
    let request = other.model().prepare_task(&foreign_key).unwrap();
    assert!(matches!(
        owner.read(request),
        Err(OwnerError::RejectedRead {
            reason: Rejection::ForeignEpoch,
            ..
        })
    ));
    assert!(!owner.model().unavailable());
    let request = owner.model().prepare_task(&k).unwrap();
    assert!(matches!(
        owner.read(request).unwrap(),
        ReadPublication::Task(_)
    ));
    other.close_quiescent().unwrap();
    owner.close_quiescent().unwrap();
}

#[test]
fn post_rename_panic_freezes_real_owner_and_preserves_pending_fact() {
    let (fixture, mut owner) = Fixture::create();
    let k = key();
    let run = start(&mut owner, &k);
    let factual = CanonicalOutcome::completed(DomainResult::success("already performed mutation"));
    let weak = factual.weak_bytes();
    let expected_bytes = factual.bytes().to_vec();
    let request = only(owner.report(run.finish(factual)).unwrap());
    let prior = std::fs::read(fixture.row(&k)).unwrap();
    let prior_identity = crate::infrastructure::platform::filesystem::file_identity(
        &std::fs::File::open(fixture.row(&k)).unwrap(),
    )
    .unwrap();
    let observation = std::sync::Arc::new(std::sync::Mutex::new(None));
    let observed_by_hook = observation.clone();
    let path = fixture.row(&k);
    let effects = owner
        .commit_with_publication_hook(request, move |point| {
            if point == PublicationPoint::AfterRename {
                let bytes = std::fs::read(&path).unwrap();
                let identity = crate::infrastructure::platform::filesystem::file_identity(
                    &std::fs::File::open(&path).unwrap(),
                )
                .unwrap();
                *observed_by_hook.lock().unwrap() = Some((bytes, identity));
                panic!("injected after actual rename");
            }
            Ok(())
        })
        .unwrap();
    assert!(matches!(effects.as_slice(), [Effect::FailStop]));
    assert!(owner.model().unavailable());
    assert!(owner.model().first_known_write_failure().is_none());
    assert!(weak.upgrade().is_some());
    let (observed, identity) = observation
        .lock()
        .unwrap()
        .take()
        .expect("candidate observed before injected panic");
    assert_ne!(
        observed, prior,
        "hook must observe the new physical aggregate"
    );
    assert_ne!(
        identity, prior_identity,
        "actual rename must publish the new inode"
    );
    let nonce = observed[16..32].try_into().unwrap();
    let stored = crate::infrastructure::execution_store::codec::read_frame(
        &mut std::io::Cursor::new(&observed),
        observed.len() as u64,
        nonce,
    )
    .unwrap();
    assert_eq!(stored.record().key(), &k);
    assert_eq!(stored.producer().unwrap().bytes(), expected_bytes);
    assert_eq!(std::fs::read(fixture.row(&k)).unwrap(), observed);
    assert_eq!(
        crate::infrastructure::platform::filesystem::file_identity(
            &std::fs::File::open(fixture.row(&k)).unwrap()
        )
        .unwrap(),
        identity
    );
    drop(owner);
    assert!(
        matches!(
            fixture.root.resolve(fixture.spec.clone()),
            Err(OpenError::Busy)
        ),
        "same-process fencing must refuse before another OS lease attempt"
    );
}

#[test]
fn held_read_prevents_explicit_close_instead_of_silently_releasing_lease() {
    let (fixture, mut owner) = Fixture::create();
    let k = key();
    finish(&mut owner, &k);
    let request = owner.model().prepare_task(&k).unwrap();
    let held = test_support::read_loaded(&mut owner, request).unwrap();
    assert!(matches!(
        owner.close_quiescent(),
        Err(OwnerError::NotQuiescent)
    ));
    drop(held);
    assert!(
        matches!(
            fixture.root.resolve(fixture.spec.clone()),
            Err(OpenError::Busy)
        ),
        "implicit drop after refused close must keep the owner fenced"
    );
}

#[test]
fn same_epoch_foreign_commit_is_rejected_before_io_without_poisoning_owner() {
    let (fixture, mut owner) = Fixture::create();
    let epoch = owner.model().store_authority().authority_epoch();
    let mut foreign = ExecutionCoordinator::new(epoch);
    let foreign_key = key();
    let request = only(
        foreign
            .submit(
                foreign_key.clone(),
                &capability(),
                ObservationWindow::new(10, 7_000).unwrap(),
            )
            .unwrap(),
    );
    let result = owner.commit(request);
    assert!(
        !fixture.row(&foreign_key).exists(),
        "an unowned request reached durable storage"
    );
    assert!(
        !owner.model().unavailable(),
        "routing error poisoned the physical owner"
    );
    assert!(matches!(result, Err(OwnerError::RejectedCommit { .. })));
    let legitimate_key = key();
    let outcome = finish(&mut owner, &legitimate_key);
    let request = owner.model().prepare_task(&legitimate_key).unwrap();
    let ReadPublication::Task(permit) = owner.read(request).unwrap() else {
        panic!("Task")
    };
    assert_eq!(permit.outcome(), &outcome);
    owner.close_quiescent().unwrap();
}

#[test]
fn same_epoch_foreign_read_is_rejected_without_physical_authority_failure() {
    let (_fixture, mut owner) = Fixture::create();
    let epoch = owner.model().store_authority().authority_epoch();
    let mut foreign = ExecutionCoordinator::new(epoch);
    let foreign_key = key();
    // The other model's acknowledged facts only construct a misrouted request;
    // the checked owner's operations below use its actual physical worker.
    let request = only(
        foreign
            .submit(
                foreign_key.clone(),
                &capability(),
                ObservationWindow::new(10, 7_000).unwrap(),
            )
            .unwrap(),
    );
    let Effect::Accepted(accepted) = foreign.complete(request.committed()).unwrap().remove(0)
    else {
        panic!("Accepted")
    };
    let request = only(foreign.begin(accepted, 20, None).unwrap());
    let Effect::Run(run) = foreign.complete(request.committed()).unwrap().remove(0) else {
        panic!("Run")
    };
    let request = only(
        foreign
            .report(
                run.finish(CanonicalOutcome::completed(DomainResult::success(
                    "foreign fixture",
                ))),
            )
            .unwrap(),
    );
    foreign.complete(request.committed()).unwrap();
    let request = foreign.prepare_task(&foreign_key).unwrap();
    let result = owner.read(request);
    assert!(
        !owner.model().unavailable(),
        "unowned read poisoned physical authority"
    );
    assert!(matches!(result, Err(OwnerError::RejectedRead { .. })));
    let legitimate_key = key();
    finish(&mut owner, &legitimate_key);
    owner.close_quiescent().unwrap();
}
