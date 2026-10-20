use crate::application::execution::hydration::ReadPublication;
use crate::application::execution::{
    AuthorityEpoch, CommitRequest, Effect, ExecutionCoordinator, OwnerCapability, RunPermit,
    WriteFailureCause,
};
use crate::application::invocation_store::MAX_CANONICAL_RESULT_BYTES;
use crate::domain::execution::storage::RecoveryCatalog;
use crate::domain::execution::{CanonicalOutcome, Key, ObservationWindow, Tool};
use crate::domain::invocation::{DomainResult, InvocationId, TaskId};
use crate::infrastructure::execution_store::codec::{self, EncodedFrame};
use crate::infrastructure::execution_store::file::{test_support, OpeningStore, ScanProgress};
use crate::infrastructure::execution_store::namespace::{
    hex, AuthorityRoot, NamespaceSpec, ResolvedNamespace,
};
use crate::infrastructure::execution_store::{
    CommitStep, FileAggregateStore, OpenError, ReadStep, Rejection,
};
use crate::infrastructure::platform::filesystem::execution_store::{
    install_publication_hook, install_stage_write_limit, PublicationPoint, Stage,
};
use std::ffi::OsStr;
use std::io::{self, Cursor};
use std::sync::Arc;

fn key() -> Key {
    Key::new(
        InvocationId::new(),
        TaskId::new(),
        [1; 32],
        Tool::Run,
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
fn only(mut effects: Vec<Effect>) -> CommitRequest {
    assert_eq!(effects.len(), 1, "{effects:?}");
    match effects.remove(0) {
        Effect::Commit(request) => request,
        other => panic!("{other:?}"),
    }
}
fn complete(
    store: &mut FileAggregateStore,
    model: &mut ExecutionCoordinator,
    request: CommitRequest,
) -> Vec<Effect> {
    match store.commit(request) {
        CommitStep::Completed(completion) => model.complete(completion).unwrap(),
        other => panic!("{other:?}"),
    }
}
fn opening(root: &AuthorityRoot, spec: NamespaceSpec, epoch: AuthorityEpoch) -> OpeningStore {
    match root.resolve(spec).unwrap() {
        ResolvedNamespace::Fresh(ticket) => OpeningStore::create_new(*ticket, epoch).unwrap(),
        ResolvedNamespace::Existing(registration) => {
            OpeningStore::open_existing(*registration, epoch).unwrap()
        }
    }
}
fn scanned(mut opening: OpeningStore) -> (FileAggregateStore, RecoveryCatalog) {
    while opening.scan_next().unwrap() != ScanProgress::Complete {}
    opening.finish().unwrap()
}
fn fixture() -> (
    tempfile::TempDir,
    AuthorityRoot,
    NamespaceSpec,
    FileAggregateStore,
    ExecutionCoordinator,
) {
    let temp = tempfile::tempdir().unwrap();
    let identity = crate::domain::execution::digest(uuid::Uuid::new_v4().as_bytes());
    let root = AuthorityRoot::provision(
        &temp.path().canonicalize().unwrap().join("authority"),
        identity,
    )
    .unwrap();
    let spec = NamespaceSpec::new([9; 32], [10; 32]);
    let (store, catalog) = scanned(opening(&root, spec.clone(), epoch(1)));
    let (model, effects) = ExecutionCoordinator::restore(catalog, epoch(1)).unwrap();
    assert!(effects.is_empty());
    (temp, root, spec, store, model)
}
fn start(store: &mut FileAggregateStore, model: &mut ExecutionCoordinator, key: &Key) -> RunPermit {
    let request = only(
        model
            .submit(
                key.clone(),
                &owner(),
                ObservationWindow::new(1, 7_000).unwrap(),
            )
            .unwrap(),
    );
    let accepted = match complete(store, model, request).remove(0) {
        Effect::Accepted(p) => p,
        other => panic!("{other:?}"),
    };
    let request = only(model.begin(accepted, 2, None).unwrap());
    match complete(store, model, request).remove(0) {
        Effect::Run(p) => p,
        other => panic!("{other:?}"),
    }
}
fn terminal(
    store: &mut FileAggregateStore,
    model: &mut ExecutionCoordinator,
    key: &Key,
    body: CanonicalOutcome,
) {
    let run = start(store, model, key);
    let request = only(model.report(run.finish(body)).unwrap());
    complete(store, model, request);
}

#[test]
fn raw_frame_exact_result_bound_reopens_and_rejects_corruption() {
    let (_temp, root, spec, mut store, mut model) = fixture();
    let key = key();
    let overhead = serde_json::to_vec(&DomainResult::success(""))
        .unwrap()
        .len();
    let result = DomainResult::success("x".repeat(MAX_CANONICAL_RESULT_BYTES - overhead));
    assert_eq!(
        serde_json::to_vec(&result).unwrap().len(),
        MAX_CANONICAL_RESULT_BYTES
    );
    let body = CanonicalOutcome::completed(result);
    assert!(body.bytes().len() > MAX_CANONICAL_RESULT_BYTES);
    terminal(&mut store, &mut model, &key, body.clone());
    let stored = test_support::load(&store, &key).unwrap();
    let nonce = test_support::nonce(&store);
    let mut bytes = Vec::new();
    EncodedFrame::prepare(&stored, nonce)
        .unwrap()
        .write_to(&mut bytes)
        .unwrap();
    let decoded = codec::read_frame(&mut Cursor::new(&bytes), bytes.len() as u64, nonce).unwrap();
    assert_eq!(decoded, stored);
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    assert!(codec::read_frame(&mut Cursor::new(&bytes), bytes.len() as u64, nonce).is_err());
    bytes[last] ^= 1;
    bytes.push(0);
    assert!(codec::read_frame(&mut Cursor::new(&bytes), bytes.len() as u64, nonce).is_err());
    drop(decoded);
    drop(stored);
    drop(model);
    store.close_quiescent().unwrap();
    let (mut reopened, catalog) = scanned(opening(&root, spec, epoch(2)));
    let (mut model, effects) = ExecutionCoordinator::restore(catalog, epoch(2)).unwrap();
    assert!(!effects.iter().any(|e| matches!(e, Effect::Run(_))));
    for effect in effects {
        if let Effect::Commit(request) = effect {
            complete(&mut reopened, &mut model, request);
        }
    }
    let request = model.prepare_task(&key).unwrap();
    let ReadStep::Loaded(completion) = reopened.read(request) else {
        panic!("exact read");
    };
    let ReadPublication::Task(permit) = model.complete_read(completion).unwrap() else {
        panic!("Task");
    };
    assert_eq!(permit.outcome(), &body);
    reopened.close_quiescent().unwrap();
}

#[test]
fn oversized_factual_candidate_is_known_and_keeps_its_bytes() {
    let (_temp, _root, _spec, mut store, mut model) = fixture();
    let key = key();
    let run = start(&mut store, &mut model, &key);
    let body = CanonicalOutcome::completed(DomainResult::success(
        "x".repeat(MAX_CANONICAL_RESULT_BYTES),
    ));
    let weak = body.weak_bytes();
    let request = only(model.report(run.finish(body)).unwrap());
    let effects = complete(&mut store, &mut model, request);
    assert!(
        matches!(effects.as_slice(), [Effect::FailStop]),
        "{effects:?}"
    );
    assert_eq!(
        model.first_known_write_failure().unwrap().cause(),
        WriteFailureCause::CandidateTooLarge
    );
    assert!(weak.upgrade().is_some());
    assert!(model.inspect(&key).unwrap().producer().is_none());
    drop(model);
    assert!(weak.upgrade().is_none());
}

#[test]
fn short_write_is_known_but_after_rename_failure_is_unknown() {
    for post_rename in [false, true] {
        let (_temp, _root, _spec, mut store, mut model) = fixture();
        let key = key();
        let request = only(
            model
                .submit(key, &owner(), ObservationWindow::new(1, 7_000).unwrap())
                .unwrap(),
        );
        let stage_limit = (!post_rename).then(|| install_stage_write_limit(8));
        let publication = post_rename.then(|| {
            install_publication_hook(|point| {
                if point == PublicationPoint::BeforeFileSync {
                    return Err(io::Error::other("after rename"));
                }
                Ok(())
            })
        });
        let effects = complete(&mut store, &mut model, request);
        assert!(
            matches!(effects.as_slice(), [Effect::FailStop]),
            "{effects:?}"
        );
        assert_eq!(model.first_known_write_failure().is_none(), post_rename);
        assert!(store.is_frozen());
        drop(publication);
        drop(stage_limit);
    }
}

#[test]
fn both_physical_identity_claims_reject_before_another_write() {
    let (_temp, _root, _spec, mut store, mut model) = fixture();
    let first = key();
    terminal(
        &mut store,
        &mut model,
        &first,
        CanonicalOutcome::completed(DomainResult::success("first")),
    );
    for second in [
        Key::new(
            first.invocation_id(),
            TaskId::new(),
            [1; 32],
            Tool::Run,
            [7; 32],
            [3; 32],
        ),
        Key::new(
            InvocationId::new(),
            first.task_id(),
            [1; 32],
            Tool::Run,
            [7; 32],
            [3; 32],
        ),
    ] {
        let mut other = ExecutionCoordinator::new(epoch(1));
        let request = only(
            other
                .submit(second, &owner(), ObservationWindow::new(1, 7_000).unwrap())
                .unwrap(),
        );
        assert!(matches!(
            store.commit(request),
            CommitStep::Rejected {
                reason: Rejection::IdentityCollision,
                ..
            }
        ));
        assert!(!store.is_frozen());
    }
    assert_eq!(test_support::record_count(&store), 1);
    store.close_quiescent().unwrap();
}

#[test]
fn same_owner_new_revision_is_obsolete_without_freezing_reads() {
    let (_temp, _root, _spec, mut store, mut model) = fixture();
    let key = key();
    terminal(
        &mut store,
        &mut model,
        &key,
        CanonicalOutcome::completed(DomainResult::success("result")),
    );
    let old = model.prepare_task(&key).unwrap();
    let request = only(model.expose_task(&key).unwrap());
    complete(&mut store, &mut model, request);
    assert!(matches!(store.read(old), ReadStep::Obsolete { .. }));
    assert!(!store.is_frozen());
    let ReadStep::Loaded(completion) = store.read(model.prepare_task(&key).unwrap()) else {
        panic!("fresh read");
    };
    assert!(matches!(
        model.complete_read(completion),
        Ok(ReadPublication::Task(_))
    ));
    store.close_quiescent().unwrap();
}

#[test]
fn lost_child_never_becomes_a_new_namespace() {
    let (_temp, root, spec, store, model) = fixture();
    drop(model);
    let child = test_support::records(&store)
        .path()
        .parent()
        .unwrap()
        .to_path_buf();
    store.close_quiescent().unwrap();
    std::fs::rename(&child, child.with_extension("retained")).unwrap();
    assert!(matches!(
        root.resolve(spec),
        Err(OpenError::IncompleteBootstrap)
    ));
    assert!(!child.exists());
}

#[test]
fn corrupted_last_record_blocks_recovery_writes_after_valid_begun_rows() {
    use crate::infrastructure::execution_store::owner::PrototypeStoreOwner;
    use std::collections::BTreeMap;
    use std::io::Write;
    use std::os::unix::fs::MetadataExt;
    fn inventory(path: &std::path::Path) -> BTreeMap<std::ffi::OsString, (u64, Vec<u8>)> {
        std::fs::read_dir(path)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.file_name(),
                    (
                        entry.metadata().unwrap().ino(),
                        std::fs::read(entry.path()).unwrap(),
                    ),
                )
            })
            .collect()
    }
    let (_temp, root, spec, mut store, mut model) = fixture();
    let keys: Vec<_> = (0..3).map(|_| key()).collect();
    for key in &keys {
        // Authorize but launch no external worker: these durable Begun records
        // require recovery terminalization before any new admission.
        let _run = start(&mut store, &mut model, key);
    }
    let records = test_support::records(&store).path().to_path_buf();
    let stage_name = format!(
        ".stage.{}.91.{}",
        hex(&test_support::nonce(&store)),
        uuid::Uuid::new_v4()
    );
    let mut stage = Stage::create(test_support::records(&store), OsStr::new(&stage_name)).unwrap();
    stage.write_all(b"partial").unwrap();
    drop(stage);
    drop(model);
    store.close_quiescent().unwrap();
    let mut opening = opening(&root, spec, epoch(2));
    while test_support::scanned_records(&opening).len() < 2 {
        assert_ne!(opening.scan_next().unwrap(), ScanProgress::Complete);
    }
    assert!(test_support::scanned_records(&opening)
        .all(|record| record.run().is_some() && record.producer().is_none()));
    let last = keys
        .iter()
        .find(|key| {
            !test_support::scanned_records(&opening)
                .any(|record| record.key().invocation_id() == key.invocation_id())
        })
        .unwrap();
    // Corrupt the actual remaining row after two valid rows have entered the
    // production recovery builder. No directory iteration order is assumed.
    std::fs::write(
        records.join(format!("{}.aggregate", hex(&last.digest()))),
        [0; codec::HEADER_BYTES],
    )
    .unwrap();
    let before = inventory(&records);
    assert!(PrototypeStoreOwner::finish_opening(opening).is_err());
    assert_eq!(
        inventory(&records),
        before,
        "failed inventory must not publish recovery or clean stages"
    );
    assert_eq!(std::fs::read(records.join(stage_name)).unwrap(), b"partial");
}

#[test]
fn dropped_store_keeps_same_process_admission_closed() {
    let (_temp, root, spec, store, _model) = fixture();
    drop(store);
    assert!(matches!(root.resolve(spec), Err(OpenError::Busy)));
}

#[test]
fn raw_factory_rejects_wrong_role_and_noncanonical_metadata() {
    let (_temp, _root, _spec, mut store, mut model) = fixture();
    let key = key();
    terminal(
        &mut store,
        &mut model,
        &key,
        CanonicalOutcome::completed(DomainResult::success("actual")),
    );
    let stored = test_support::load(&store, &key).unwrap();
    let metadata = stored.record().encode();
    let bytes = Arc::<[u8]>::from(stored.producer().unwrap().bytes());
    let decode = crate::domain::execution::storage::codec::decode_parts;
    assert!(decode(
        &metadata,
        None,
        Some(bytes.clone()),
        MAX_CANONICAL_RESULT_BYTES
    )
    .is_err());
    let mut altered = metadata;
    altered.push(b' ');
    assert!(decode(&altered, Some(bytes), None, MAX_CANONICAL_RESULT_BYTES).is_err());
    store.close_quiescent().unwrap();
}

#[test]
fn copied_namespace_seals_cannot_adopt_replacement_directories() {
    use std::os::unix::fs::PermissionsExt;
    for replacement in [None, Some(false), Some(true)] {
        let (_temp, root, spec, store, model) = fixture();
        let records = test_support::records(&store).path().to_path_buf();
        let child = records.parent().unwrap().to_path_buf();
        drop(model);
        store.close_quiescent().unwrap();
        let Some(replace_records) = replacement else {
            let (store, catalog) = scanned(opening(&root, spec, epoch(2)));
            let (_, effects) = ExecutionCoordinator::restore(catalog, epoch(2)).unwrap();
            assert!(effects.is_empty());
            store.close_quiescent().unwrap();
            continue;
        };
        let replaced = if replace_records { &records } else { &child };
        let retained = replaced.with_extension("retained");
        std::fs::rename(replaced, &retained).unwrap();
        std::fs::create_dir(replaced).unwrap();
        std::fs::set_permissions(replaced, std::fs::Permissions::from_mode(0o700)).unwrap();
        if !replace_records {
            std::fs::copy(retained.join("manifest.v1"), child.join("manifest.v1")).unwrap();
            std::fs::create_dir(&records).unwrap();
            std::fs::set_permissions(&records, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let ResolvedNamespace::Existing(registration) = root.resolve(spec).unwrap() else {
            panic!("registered namespace became fresh");
        };
        assert!(matches!(
            OpeningStore::open_existing(*registration, epoch(2)),
            Err(OpenError::InvalidInventory)
        ));
        assert!(retained.exists());
    }
}

#[test]
fn raw_frame_rejects_foreign_namespace_and_oversized_headers_before_body_reads() {
    let (_temp, _root, _spec, mut store, mut model) = fixture();
    let key = key();
    terminal(
        &mut store,
        &mut model,
        &key,
        CanonicalOutcome::completed(DomainResult::success("bounded")),
    );
    let stored = test_support::load(&store, &key).unwrap();
    let nonce = test_support::nonce(&store);
    let mut bytes = Vec::new();
    EncodedFrame::prepare(&stored, nonce)
        .unwrap()
        .write_to(&mut bytes)
        .unwrap();
    let mut foreign = nonce;
    foreign[0] ^= 1;
    let mut reader = Cursor::new(&bytes);
    assert!(codec::read_frame(&mut reader, bytes.len() as u64, foreign).is_err());
    assert_eq!(reader.position(), codec::HEADER_BYTES as u64);
    for offset in [64, 72, 80] {
        let mut header = bytes[..codec::HEADER_BYTES].to_vec();
        header[offset..offset + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        let mut reader = Cursor::new(&header);
        assert!(codec::read_frame(&mut reader, u64::MAX, nonce).is_err());
        assert_eq!(reader.position(), codec::HEADER_BYTES as u64);
    }
    let mut unsupported = bytes.clone();
    unsupported[8..10].copy_from_slice(&2_u16.to_le_bytes());
    assert!(codec::read_frame(
        &mut Cursor::new(&unsupported),
        unsupported.len() as u64,
        nonce
    )
    .is_err());
    let truncated = &bytes[..bytes.len() - 1];
    assert!(codec::read_frame(&mut Cursor::new(truncated), truncated.len() as u64, nonce).is_err());
    store.close_quiescent().unwrap();
}

#[test]
fn valid_shared_read_metadata_with_both_roles_fits_the_closed_bound() {
    use crate::domain::execution::{FailureReason, JoinEvidence, ReadAuthority};
    let (_temp, _root, _spec, mut store, mut model) = fixture();
    let key = Key::new(
        InvocationId::new(),
        TaskId::new(),
        [255; 32],
        Tool::Resolve,
        [255; 32],
        [255; 32],
    );
    let read_authority = ReadAuthority::new([255; 32], u64::MAX, u64::MAX);
    let request = only(
        model
            .submit(
                key.clone(),
                &owner(),
                ObservationWindow::new(u64::MAX - 7_000, 7_000).unwrap(),
            )
            .unwrap(),
    );
    let Effect::Accepted(accepted) = complete(&mut store, &mut model, request).remove(0) else {
        panic!("Accepted")
    };
    let request = only(
        model
            .begin(accepted, u64::MAX, Some(read_authority))
            .unwrap(),
    );
    let Effect::Run(run) = complete(&mut store, &mut model, request).remove(0) else {
        panic!("Run")
    };
    let alias = Key::new(
        InvocationId::new(),
        TaskId::new(),
        [255; 32],
        Tool::Resolve,
        [255; 32],
        [255; 32],
    );
    let request = only(
        model
            .attach(&key, JoinEvidence::new(alias, read_authority, u64::MAX))
            .unwrap(),
    );
    assert!(matches!(
        complete(&mut store, &mut model, request).as_slice(),
        [Effect::Attached(_)]
    ));
    let request = only(model.cancel(&key).unwrap());
    complete(&mut store, &mut model, request);
    let request = only(
        model
            .report(run.finish(CanonicalOutcome::failed(FailureReason::ResumeUnsupported)))
            .unwrap(),
    );
    complete(&mut store, &mut model, request);
    let request = only(model.expose_task(&key).unwrap());
    complete(&mut store, &mut model, request);
    let request = only(
        model
            .acknowledge(
                &key,
                &owner(),
                CanonicalOutcome::cancelled().digest(),
                u64::MAX,
            )
            .unwrap(),
    );
    complete(&mut store, &mut model, request);
    let stored = test_support::load(&store, &key).unwrap();
    assert!(stored.producer().is_some());
    assert!(stored.primary_override().is_some());
    assert!(stored.record().acknowledgement().is_some());
    assert!(
        stored.record().encode().len()
            <= crate::domain::execution::storage::codec::MAX_METADATA_BYTES
    );
    let mut bytes = Vec::new();
    EncodedFrame::prepare(&stored, test_support::nonce(&store))
        .unwrap()
        .write_to(&mut bytes)
        .unwrap();
    assert_eq!(
        codec::read_frame(
            &mut Cursor::new(&bytes),
            bytes.len() as u64,
            test_support::nonce(&store)
        )
        .unwrap(),
        stored
    );
    store.close_quiescent().unwrap();
}

#[test]
fn physical_cas_readback_redecides_the_same_factual_outcome() {
    let (_temp, _root, _spec, mut store, mut first) = fixture();
    let key = key();
    let run = start(&mut store, &mut first, &key);
    let original_run = first.inspect(&key).unwrap().run();
    let base_revision = first.inspect(&key).unwrap().revision();
    // The physical store has one lease. A second pure view is seeded through
    // actual readback of its confirmed row, without granting another Run.
    // This tests the low-level CAS seam, not a supported owner-facade race.
    let mut second = ExecutionCoordinator::new(epoch(1));
    let request = only(
        second
            .submit(
                key.clone(),
                &owner(),
                ObservationWindow::new(1, 7_000).unwrap(),
            )
            .unwrap(),
    );
    assert!(complete(&mut store, &mut second, request).is_empty());
    assert_eq!(first.inspect(&key).unwrap(), second.inspect(&key).unwrap());
    let outcome = CanonicalOutcome::completed(DomainResult::success("exact pending factual bytes"));
    let weak = outcome.weak_bytes();
    let expected_bytes = outcome.bytes().to_vec();
    let pending = only(first.report(run.finish(outcome)).unwrap());
    assert_eq!(pending.expected_revision(), base_revision);
    let expose = only(second.expose_task(&key).unwrap());
    assert!(matches!(
        complete(&mut store, &mut second, expose).as_slice(),
        [Effect::Task(_)]
    ));
    let path = test_support::records(&store)
        .path()
        .join(format!("{}.aggregate", hex(&key.digest())));
    let after_expose = std::fs::read(&path).unwrap();
    let effects = complete(&mut store, &mut first, pending);
    assert!(!effects
        .iter()
        .any(|effect| matches!(effect, Effect::Run(_))));
    let retried = only(effects);
    assert_eq!(retried.expected_revision(), base_revision + 1);
    assert_eq!(retried.candidate().revision(), base_revision + 2);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        after_expose,
        "readback must not publish the stale candidate"
    );
    assert!(weak.upgrade().is_some(), "redecision lost the factual body");
    assert!(complete(&mut store, &mut first, retried).is_empty());
    assert!(
        weak.upgrade().is_none(),
        "confirmed metadata retained the original body"
    );
    let stored = test_support::load(&store, &key).unwrap();
    assert_eq!(stored.record().revision(), base_revision + 2);
    assert_eq!(stored.record().run(), original_run);
    assert!(stored.record().public_task());
    assert_eq!(stored.producer().unwrap().bytes(), expected_bytes);
    store.close_quiescent().unwrap();
}
