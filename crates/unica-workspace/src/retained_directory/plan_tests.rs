use super::*;
use crate::actor::apply_writer_authority_for_test;
use crate::apply::ApplyStagedState;
use std::time::Duration;

fn fixture() -> (tempfile::TempDir, PathBuf, DirectoryRemoval) {
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    std::fs::create_dir_all(root.join("Payload/Ext/Empty")).unwrap();
    std::fs::write(root.join("Payload/Ext/Module.bsl"), b"original").unwrap();
    let retained = Arc::new(RetainedDirectoryCapability::open(&root).unwrap());
    let file = retained
        .read_relative_regular_bounded(Path::new("Payload/Ext/Module.bsl"), 16)
        .unwrap();
    let identity = retained
        .retain_directory_child(OsStr::new("Payload"))
        .unwrap()
        .retain_directory_child(OsStr::new("Ext"))
        .unwrap()
        .retain_regular_child(OsStr::new("Module.bsl"))
        .unwrap()
        .identity();
    let files = BTreeMap::from([(
        identity,
        RemovedFileImage {
            len: file.len(),
            sha256: Sha256::digest(file).into(),
        },
    )]);
    let mut plans = DirectoryRemoval::capture(
        retained,
        Path::new("Payload"),
        Path::new("Payload.xml"),
        &files,
        ProviderDeadline::no_deadline(),
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(plans.len(), 1);
    (temp, root, plans.remove(0))
}

#[test]
fn saved_directory_intent_obeys_rebound_stop_budget_without_replanning() {
    for cancelled in [false, true] {
        let (_temp, root, mut plan) = fixture();
        let identity = RetainedDirectoryCapability::open(&root.join("Payload"))
            .unwrap()
            .identity();
        let before = crate::test_support::tree_snapshot(&root);
        let cancellation = CancellationToken::new();
        if cancelled {
            cancellation.cancel();
        }
        plan.rebind(
            if cancelled {
                ProviderDeadline::no_deadline()
            } else {
                ProviderDeadline::from_budget(Duration::ZERO)
            },
            cancellation,
        );
        let failure = match plan.publish() {
            Ok(_) => panic!("stopped publication succeeded"),
            Err(error) => error,
        };
        assert_eq!(
            failure.error.kind(),
            if cancelled {
                ApplyStagingErrorKind::Cancelled
            } else {
                ApplyStagingErrorKind::Deadline
            }
        );
        assert!(failure.artifact.is_none());
        assert_eq!(crate::test_support::tree_snapshot(&root), before);
        assert_eq!(
            RetainedDirectoryCapability::open(&root.join("Payload"))
                .unwrap()
                .identity(),
            identity
        );
        assert!(root.join("Payload/Ext/Empty").is_dir());
    }
}

#[test]
fn cancelled_directory_execution_still_restores_the_retained_original() {
    let (_temp, root, plan) = fixture();
    let identity = RetainedDirectoryCapability::open(&root.join("Payload"))
        .unwrap()
        .identity();
    let before = crate::test_support::tree_snapshot(&root);
    let published = plan
        .publish()
        .unwrap_or_else(|failure| panic!("{}", failure.error));
    assert!(!root.join("Payload").exists());
    plan.cancellation.cancel();
    assert_eq!(
        published.validate().unwrap_err().kind(),
        ApplyStagingErrorKind::Cancelled
    );
    published.rollback().unwrap();
    assert_eq!(crate::test_support::tree_snapshot(&root), before);
    assert_eq!(
        RetainedDirectoryCapability::open(&root.join("Payload"))
            .unwrap()
            .identity(),
        identity
    );
    assert!(root.join("Payload/Ext/Empty").is_dir());
}

#[test]
fn cache_staging_cannot_supply_source_directory_removal_intent() {
    let (_temp, root, _plan) = fixture();
    let mut cache = ApplyStagedState::from_retained_root(
        Arc::new(RetainedDirectoryCapability::open(&root).unwrap()),
        ProviderDeadline::no_deadline(),
        CancellationToken::new(),
        apply_writer_authority_for_test(),
    );
    let before = crate::test_support::tree_snapshot(&root);
    let error = cache
        .remove_owned_directories(
            Path::new("Payload/Ext"),
            Path::new("Payload/Ext.xml"),
            Path::new("Payload"),
        )
        .unwrap_err();
    assert_eq!(error.kind(), ApplyStagingErrorKind::Invariant);
    assert_eq!(crate::test_support::tree_snapshot(&root), before);
    assert!(root.join("Payload/Ext/Empty").is_dir());
}

#[test]
fn collection_promotion_keeps_the_existing_payload_depth_budget() {
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let payload = Path::new("Forms/Main");
    std::fs::create_dir_all(root.join(payload)).unwrap();
    let mut leaf = root.join(payload);
    for _ in 0..MAX_APPLY_NAMESPACE_DEPTH {
        leaf.push("d");
        std::fs::create_dir(&leaf).unwrap();
    }
    std::fs::write(root.join("Forms/Main.xml"), b"descriptor").unwrap();
    let retained = Arc::new(RetainedDirectoryCapability::open(&root).unwrap());
    let mut state = ApplyStagedState::from_retained_root(
        Arc::clone(&retained),
        ProviderDeadline::no_deadline(),
        CancellationToken::new(),
        apply_writer_authority_for_test(),
    )
    .forbid_generated_subtree();
    assert!(
        state.enumerate_tree(payload).unwrap().is_empty(),
        "the pre-existing namespace budget admits this payload"
    );
    let before = crate::test_support::tree_snapshot(&root);
    state
        .remove(Path::new("Forms/Main.xml"), b"descriptor")
        .unwrap();
    state
        .remove_owned_directories(payload, Path::new("Forms/Main.xml"), Path::new("Forms"))
        .unwrap();
    let mut hash = Sha256::new();
    state.fingerprint_inputs(&mut hash).unwrap();
    state.finalize().unwrap();
    assert_eq!(crate::test_support::tree_snapshot(&root), before);
    assert!(leaf.is_dir());
}

#[test]
fn directory_cleanup_does_not_delete_a_foreign_successor_at_its_last_boundary() {
    use crate::platform::filesystem::set_before_identity_bound_directory_cleanup_mutation_hook;
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    std::fs::create_dir(root.join("Payload")).unwrap();
    let plan = DirectoryRemoval::capture(
        Arc::new(RetainedDirectoryCapability::open(&root).unwrap()),
        Path::new("Payload"),
        Path::new("Payload.xml"),
        &BTreeMap::new(),
        ProviderDeadline::no_deadline(),
        CancellationToken::new(),
    )
    .unwrap()
    .remove(0);
    let published = plan
        .publish()
        .unwrap_or_else(|failure| panic!("{}", failure.error));
    let original_identity = published.recovery.identity();
    let recovery = published.recovery.path().to_path_buf();
    let displaced = root.join("original-recovery-relocated");
    let replacement = recovery.clone();
    let moved = displaced.clone();
    let hook_observation = std::rc::Rc::new(std::cell::Cell::new((false, None)));
    let observed_by_hook = std::rc::Rc::clone(&hook_observation);
    set_before_identity_bound_directory_cleanup_mutation_hook(move || {
        observed_by_hook.set((true, None));
        std::fs::rename(&replacement, &moved).unwrap();
        std::fs::create_dir(&replacement).unwrap();
        observed_by_hook.set((
            true,
            Some(
                RetainedDirectoryCapability::open(&replacement)
                    .unwrap()
                    .identity(),
            ),
        ));
    });
    let result = published.cleanup();
    let (hook_fired, foreign_identity) = hook_observation.get();
    let recovery_identity = RetainedDirectoryCapability::open(&recovery).map(|dir| dir.identity());
    let displaced_identity =
        RetainedDirectoryCapability::open(&displaced).map(|dir| dir.identity());
    eprintln!(
        "directory cleanup observation: hook_fired={hook_fired}, result={result:?}, original={original_identity:?}, foreign={foreign_identity:?}, recovery={recovery_identity:?}, displaced={displaced_identity:?}"
    );
    assert!(hook_fired, "the competing directory replacement must run");
    assert!(result.is_err(), "{result:?}");
    assert!(
        recovery.is_dir(),
        "foreign empty directory must not be cleaned"
    );
    let foreign_identity = foreign_identity.expect("the hook must capture its successor identity");
    assert_ne!(foreign_identity, original_identity);
    assert_eq!(recovery_identity.unwrap(), foreign_identity);
    assert_eq!(displaced_identity.unwrap(), original_identity);
}

#[test]
fn displaced_file_growth_is_a_revision_conflict_and_is_never_cleaned() {
    let (_temp, _root, plan) = fixture();
    let published = plan
        .publish()
        .unwrap_or_else(|failure| panic!("{}", failure.error));
    let changed = published.recovery.path().join("Ext/Module.bsl");
    std::fs::write(&changed, b"foreign changed and longer bytes").unwrap();
    assert_eq!(
        published.validate().unwrap_err().kind(),
        ApplyStagingErrorKind::ConcurrentRevision
    );
    assert!(published.rollback().is_err());
    assert!(published.cleanup().is_err());
    assert_eq!(
        std::fs::read(changed).unwrap(),
        b"foreign changed and longer bytes"
    );
    assert!(published.recovery.path().join("Ext/Empty").is_dir());
}
