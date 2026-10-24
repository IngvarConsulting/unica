use super::test_support::*;
use super::*;
use crate::platform::testing;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

#[test]
fn retained_displacement_preserves_provider_and_identity_failure_kinds() {
    for kind in [
        ErrorKind::PermissionDenied,
        ErrorKind::ReadOnlyFilesystem,
        ErrorKind::StorageFull,
        ErrorKind::QuotaExceeded,
    ] {
        assert_eq!(
            retained_apply_displacement_error_kind(&std::io::Error::new(kind, "opaque")),
            RetainedApplyPublishErrorKind::Provider,
        );
    }
    for kind in [ErrorKind::NotFound, ErrorKind::InvalidData] {
        assert_eq!(
            retained_apply_displacement_error_kind(&std::io::Error::new(kind, "opaque")),
            RetainedApplyPublishErrorKind::ContainmentIdentity,
        );
    }
    for (error, sharing_violation) in testing::sharing_violation_error_cases_for_test() {
        assert_eq!(
            retained_apply_displacement_error_kind(&error),
            if sharing_violation {
                RetainedApplyPublishErrorKind::Provider
            } else {
                RetainedApplyPublishErrorKind::ContainmentIdentity
            },
            "{error:?}",
        );
    }
}

#[test]
fn commit_failure_kind_does_not_depend_on_message_wording() {
    let provider = CommitFailure::provider(
        "changed preimage and rollback encountered are only words in this provider error",
    );
    assert_eq!(provider.kind(), CommitFailureKind::ProviderUnavailable);

    let concurrent = CommitFailure::concurrent("opaque state mismatch");
    assert_eq!(concurrent.kind(), CommitFailureKind::ConcurrentModification);

    let cleanup = with_cleanup_diagnostics(concurrent, vec!["temporary file remains".into()]);
    assert_eq!(cleanup.kind(), CommitFailureKind::ConcurrentModification);
    let cleanup = with_cleanup_diagnostics(
        CommitFailure::provider("source unavailable"),
        vec!["temporary file remains".into()],
    );
    assert_eq!(cleanup.kind(), CommitFailureKind::ProviderUnavailable);

    let rollback = with_rollback_diagnostics(provider, vec!["cleanup failed".into()]);
    assert_eq!(rollback.kind(), CommitFailureKind::RollbackFailed);
}

#[test]
fn retained_apply_rollback_releases_published_capability_before_preimage_restore() {
    let source = include_str!("compile_transaction.rs");
    let rollback = source
        .split_once("fn rollback_retained_apply(")
        .expect("retained apply rollback must exist")
        .1
        .split_once("fn finalize_retained_apply_recovery(")
        .expect("retained apply rollback must precede recovery finalization")
        .0;
    let take = rollback
        .find("change.published.take()")
        .expect("rollback must take ownership of the published capability");
    let release = rollback
        .find("drop(file)")
        .expect("rollback must explicitly release the published capability");
    let restore = rollback
        .find("restore_displaced_regular_child_no_replace")
        .expect("rollback must restore the displaced preimage");

    assert!(
            take < release && release < restore,
            "a delete-pending Windows publication handle must close before same-name preimage restoration"
        );
}

#[test]
fn binary_replacement_uses_the_same_transaction_without_xml_validation() {
    let root = temp_root("binary-replacement");
    let target = root.join("Ext/ParentConfigurations.bin");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    let original = vec![0, 0xff, 0x01, 0x80];
    let replacement = vec![0x7f, 0, 0xfe, 0x02, 0x03];
    fs::write(&target, &original).unwrap();
    let mut transaction = CompileTransaction::new();
    transaction
        .replace_bytes(&target, &original, replacement.clone())
        .unwrap();

    let report = transaction.commit().expect("binary update must commit");

    assert_eq!(report.updated, vec![target.clone()]);
    assert_eq!(fs::read(&target).unwrap(), replacement);
    assert!(transaction_debris(&root).is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn replacement_rejects_parent_symlink_swap_after_planning_before_locking() {
    let root = temp_root("replacement-parent-symlink-swap");
    let role_dir = root.join("safe/Role");
    let saved_role_dir = root.join("saved-role");
    let outside_role_dir = root.join("outside-role");
    let target = role_dir.join("Ext/Rights.xml");
    let saved_target = saved_role_dir.join("Ext/Rights.xml");
    let outside_target = outside_role_dir.join("Ext/Rights.xml");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::create_dir_all(outside_target.parent().unwrap()).unwrap();
    let before = b"<Rights><value>true</value></Rights>\n".to_vec();
    let after = b"<Rights><value>false</value></Rights>\n".to_vec();
    fs::write(&target, &before).unwrap();
    fs::write(&outside_target, &before).unwrap();

    let mut transaction = CompileTransaction::new();
    transaction
        .replace_bytes(&target, &before, after)
        .expect("safe replacement must plan");

    fs::rename(&role_dir, &saved_role_dir).unwrap();
    // Only an explicit privilege refusal skips; any other link error fails.
    match crate::platform::testing::create_directory_link_fixture_for_test(
        &outside_role_dir,
        &role_dir,
    )
    .expect("create a directory link fixture")
    {
        crate::platform::testing::FileLinkFixtureOutcome::Created => {}
        crate::platform::testing::FileLinkFixtureOutcome::Unsupported
        | crate::platform::testing::FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
            fs::rename(&saved_role_dir, &role_dir).unwrap();
            fs::remove_dir_all(root).unwrap();
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::Symlink,
                "creating a directory link needs the symlink privilege",
            );
            return;
        }
    }

    let error = transaction
        .commit()
        .expect_err("a changed canonical route must reject publication");
    assert!(
        error.contains("publication target identity changed after planning"),
        "{error}"
    );
    assert_eq!(fs::read(&saved_target).unwrap(), before);
    assert_eq!(fs::read(&outside_target).unwrap(), before);

    testing::remove_dir_symlink_for_test(&role_dir).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn transaction_rejects_cache_create_parent_symlink_swap_before_touching_either_target() {
    let root = temp_root("cache-create-parent-symlink-swap");
    let rights = root.join("src/Roles/Demo/Ext/Rights.xml");
    let build_dir = root.join(".build");
    let saved_build_dir = root.join("saved-build");
    let outside_build_dir = root.join("outside-build");
    let cache = build_dir.join("unica/state.json");
    let saved_cache = saved_build_dir.join("unica/state.json");
    let outside_cache = outside_build_dir.join("unica/state.json");
    fs::create_dir_all(rights.parent().unwrap()).unwrap();
    fs::create_dir_all(cache.parent().unwrap()).unwrap();
    fs::create_dir_all(outside_cache.parent().unwrap()).unwrap();
    let before = b"<Rights><value>true</value></Rights>\n".to_vec();
    let after = b"<Rights><value>false</value></Rights>\n".to_vec();
    fs::write(&rights, &before).unwrap();

    let mut transaction = CompileTransaction::new();
    transaction
        .replace_bytes(&rights, &before, after)
        .expect("rights replacement must plan");
    transaction
        .create_bytes(&cache, br#"{"workspaceEpoch":1}"#.to_vec())
        .expect("cache creation must plan");

    fs::rename(&build_dir, &saved_build_dir).unwrap();
    // Only an explicit privilege refusal skips; any other link error fails.
    match crate::platform::testing::create_directory_link_fixture_for_test(
        &outside_build_dir,
        &build_dir,
    )
    .expect("create a directory link fixture")
    {
        crate::platform::testing::FileLinkFixtureOutcome::Created => {}
        crate::platform::testing::FileLinkFixtureOutcome::Unsupported
        | crate::platform::testing::FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
            fs::rename(&saved_build_dir, &build_dir).unwrap();
            fs::remove_dir_all(root).unwrap();
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::Symlink,
                "creating a directory link needs the symlink privilege",
            );
            return;
        }
    }

    let error = transaction
        .commit()
        .expect_err("a changed cache route must reject the complete transaction");
    assert!(
        error.contains("publication target identity changed after planning"),
        "{error}"
    );
    assert_eq!(fs::read(&rights).unwrap(), before);
    assert!(!saved_cache.exists());
    assert!(!outside_cache.exists());
    assert!(transaction_debris(&root).is_empty());

    testing::remove_dir_symlink_for_test(&build_dir).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn nested_absence_guard_with_missing_parent_acquires_a_lock_only_identity() {
    let root = temp_root("nested-absence-missing-parent");
    let marker = root.join("Configuration/Configuration.mdo");
    let output = root.join("Generated.xml");
    let mut transaction = CompileTransaction::new();
    transaction
        .guard_path_absent(&marker)
        .expect("nested marker absence must plan");
    transaction
        .create_bytes(&output, b"<Generated/>\n".to_vec())
        .expect("output creation must plan");

    let report = transaction
        .commit()
        .expect("missing marker parent must not prevent guard locking");

    assert_eq!(report.created, vec![output.clone()]);
    assert!(!marker.exists());
    assert_eq!(fs::read(&output).unwrap(), b"<Generated/>\n");
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

#[test]
fn direct_child_directory_guard_lists_directories_and_skips_everything_else() {
    let root = temp_root("membership-child-directories-skips");
    fs::create_dir_all(root.join("extension")).unwrap();
    fs::write(root.join(".gitkeep"), b"").unwrap();
    let external = temp_root("membership-child-directories-external");
    let has_link = matches!(
        testing::create_directory_link_fixture_for_test(&external, root.join("linked")).unwrap(),
        testing::FileLinkFixtureOutcome::Created
    );

    // A container guarded for its child directories legitimately holds other
    // things. They are not members: the snapshot lists the directories and
    // refuses nothing, because a guard that cannot be taken beside a `.gitkeep`
    // is a guarantee that is silently absent.
    let snapshot =
        snapshot_directory_membership(&root, DirectoryMembershipSelector::DirectChildDirectories)
            .expect("a file or link beside the directories must not fail the snapshot");

    assert_eq!(
        snapshot,
        DirectoryMembershipSnapshot::Present(vec![DirectoryTopologyEntry {
            name: OsString::from("extension"),
            kind: DirectoryTopologyEntryKind::Directory,
        }]),
        "link fixture created: {has_link}"
    );
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(external).unwrap();
}

#[test]
fn direct_child_directory_guard_detects_a_child_directory_that_appeared() {
    let root = temp_root("membership-child-directories-appeared");
    fs::create_dir_all(root.join("extension")).unwrap();
    let expected =
        snapshot_directory_membership(&root, DirectoryMembershipSelector::DirectChildDirectories)
            .unwrap();
    let mut transaction = CompileTransaction::new();
    transaction
        .guard_or_verify_directory_membership(
            &root,
            DirectoryMembershipSelector::DirectChildDirectories,
            expected,
        )
        .unwrap();
    fs::create_dir_all(root.join("late")).unwrap();

    let error = transaction
        .commit()
        .expect_err("a child directory that appeared after planning must be caught");

    assert!(error.contains("late"), "{error}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn direct_child_directory_guard_ignores_a_planned_file_in_the_guarded_directory() {
    let root = temp_root("membership-child-directories-planned-file");
    fs::create_dir_all(root.join("extension")).unwrap();
    let expected =
        snapshot_directory_membership(&root, DirectoryMembershipSelector::DirectChildDirectories)
            .unwrap();
    let created = root.join("Created.xml");
    let mut transaction = CompileTransaction::new();
    transaction
        .guard_or_verify_directory_membership(
            &root,
            DirectoryMembershipSelector::DirectChildDirectories,
            expected,
        )
        .unwrap();

    // Writing a file into the guarded container does not change which of its
    // children are directories, so it is not a membership delta at all.
    transaction
        .create_bytes(&created, b"<Created/>\n".to_vec())
        .unwrap();
    let report = transaction
        .commit()
        .expect("a planned file is not a member of a child-directory guard");

    assert_eq!(report.created, vec![created]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn direct_child_directory_guard_rejects_a_file_shaped_expected_entry() {
    let root = temp_root("membership-child-directories-file-entry");
    let mut transaction = CompileTransaction::new();

    let error = transaction
        .guard_or_verify_directory_membership(
            &root,
            DirectoryMembershipSelector::DirectChildDirectories,
            DirectoryMembershipSnapshot::Present(vec![DirectoryTopologyEntry {
                name: OsString::from("Configuration.xml"),
                kind: DirectoryTopologyEntryKind::File,
            }]),
        )
        .expect_err("a file cannot be a member of a child-directory guard");

    assert!(error.contains("Configuration.xml"), "{error}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn directory_membership_guard_accepts_its_own_top_level_xml_create() {
    let root = temp_root("membership-own-create");
    let expected =
        snapshot_directory_membership(&root, DirectoryMembershipSelector::XmlFiles).unwrap();
    let created = root.join("Created.xml");
    let mut transaction = CompileTransaction::new();
    transaction
        .guard_or_verify_directory_membership(
            &root,
            DirectoryMembershipSelector::XmlFiles,
            expected,
        )
        .unwrap();
    transaction
        .create_bytes(&created, b"<Created/>\n".to_vec())
        .unwrap();

    let report = transaction
        .commit()
        .expect("planned membership delta must be accepted");

    assert_eq!(report.created, vec![created.clone()]);
    assert_eq!(
        snapshot_directory_membership(&root, DirectoryMembershipSelector::XmlFiles).unwrap(),
        DirectoryMembershipSnapshot::Present(vec![DirectoryTopologyEntry {
            name: OsString::from("Created.xml"),
            kind: DirectoryTopologyEntryKind::File,
        }])
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn directory_membership_guard_accepts_own_create_in_an_absent_root() {
    let root = temp_root("membership-absent-root");
    let external_root = root.join("external");
    let expected =
        snapshot_directory_membership(&external_root, DirectoryMembershipSelector::XmlFiles)
            .unwrap();
    let created = external_root.join("Created.xml");
    let mut transaction = CompileTransaction::new();
    transaction
        .guard_or_verify_directory_membership(
            &external_root,
            DirectoryMembershipSelector::XmlFiles,
            expected,
        )
        .unwrap();
    transaction
        .create_bytes(&created, b"<Created/>\n".to_vec())
        .unwrap();

    transaction
        .commit()
        .expect("absent guarded root must allow its planned descriptor");

    assert_eq!(fs::read(&created).unwrap(), b"<Created/>\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn directory_membership_snapshot_distinguishes_absent_from_present_empty() {
    let root = temp_root("membership-existence-snapshot");
    let guarded = root.join("guarded");

    let absent = snapshot_directory_membership_entries(
        &guarded,
        DirectoryMembershipSelector::AllDirectEntries,
    )
    .unwrap();
    fs::create_dir(&guarded).unwrap();
    let present_empty = snapshot_directory_membership_entries(
        &guarded,
        DirectoryMembershipSelector::AllDirectEntries,
    )
    .unwrap();

    assert_ne!(absent, present_empty);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn directory_membership_guard_rejects_external_empty_root_after_absent_snapshot() {
    let root = temp_root("membership-external-empty-root");
    let guarded = root.join("guarded");
    let expected = snapshot_directory_membership_entries(
        &guarded,
        DirectoryMembershipSelector::AllDirectEntries,
    )
    .unwrap();
    fs::create_dir(&guarded).unwrap();
    let mut transaction = CompileTransaction::new();

    let error = transaction
        .guard_or_verify_directory_topology(&guarded, expected)
        .expect_err("an externally-created empty root must not satisfy an absent snapshot");

    assert!(error.contains("directory membership"), "{error}");
    assert_eq!(fs::read_dir(&guarded).unwrap().count(), 0);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn directory_membership_guard_accepts_own_nested_create_in_an_absent_root() {
    let root = temp_root("membership-absent-nested-root");
    let external_root = root.join("external");
    let expected = snapshot_directory_membership_entries(
        &external_root,
        DirectoryMembershipSelector::AllDirectEntries,
    )
    .unwrap();
    let created = external_root.join("Ext/Module.bsl");
    let mut transaction = CompileTransaction::new();
    transaction
        .guard_or_verify_directory_topology(&external_root, expected)
        .unwrap();
    transaction
        .create_bytes(&created, b"Procedure Run()\nEndProcedure\n".to_vec())
        .unwrap();

    transaction
        .commit()
        .expect("guarded root must accept its planned transient parent directories");

    assert_eq!(
        fs::read(&created).unwrap(),
        b"Procedure Run()\nEndProcedure\n"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn nested_directory_membership_guard_rolls_back_on_a_post_write_entry() {
    let root = temp_root("membership-nested-concurrent-create");
    let resource_root = root.join("Resource");
    let ext = resource_root.join("Ext");
    let planned = ext.join("Module.bsl");
    let concurrent = ext.join("Help.xml");
    let mut transaction = CompileTransaction::new();
    transaction
        .guard_or_verify_directory_topology(&resource_root, DirectoryMembershipSnapshot::Absent)
        .unwrap();
    transaction
        .guard_or_verify_directory_topology(&ext, DirectoryMembershipSnapshot::Absent)
        .unwrap();
    transaction
        .create_bytes(&planned, b"Procedure Run()\nEndProcedure\n".to_vec())
        .unwrap();
    let concurrent_for_validation = concurrent.clone();

    let error = transaction
        .commit_with_post_validation(move || {
            fs::write(&concurrent_for_validation, b"<Help/>\n").map_err(|error| error.to_string())
        })
        .expect_err("unplanned nested entry must abort publication");

    assert!(error.contains("directory membership guard"), "{error}");
    assert!(!planned.exists());
    assert_eq!(fs::read(&concurrent).unwrap(), b"<Help/>\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn directory_membership_guard_rolls_back_on_an_unplanned_top_level_xml_entry() {
    let root = temp_root("membership-concurrent-create");
    let existing = root.join("Existing.xml");
    let planned = root.join("Planned.xml");
    let concurrent = root.join("Concurrent.xml");
    fs::write(&existing, b"<Existing/>\n").unwrap();
    let expected =
        snapshot_directory_membership(&root, DirectoryMembershipSelector::XmlFiles).unwrap();
    let mut transaction = CompileTransaction::new();
    transaction
        .guard_or_verify_directory_membership(
            &root,
            DirectoryMembershipSelector::XmlFiles,
            expected,
        )
        .unwrap();
    transaction
        .create_bytes(&planned, b"<Planned/>\n".to_vec())
        .unwrap();
    let concurrent_for_hook = concurrent.clone();

    let error = with_before_commit_hook(
        move |_| fs::write(&concurrent_for_hook, b"<Concurrent/>\n").unwrap(),
        || transaction.commit(),
    )
    .expect_err("unplanned direct XML entry must abort publication");

    assert!(error.contains("directory membership guard"), "{error}");
    assert!(!planned.exists());
    assert_eq!(fs::read(&existing).unwrap(), b"<Existing/>\n");
    assert_eq!(fs::read(&concurrent).unwrap(), b"<Concurrent/>\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn directory_membership_guard_exclusively_serializes_a_cooperating_create() {
    let root = temp_root("membership-exclusive-tree-lock");
    let expected =
        snapshot_directory_membership(&root, DirectoryMembershipSelector::XmlFiles).unwrap();
    let mut guarded = CompileTransaction::new();
    guarded
        .guard_or_verify_directory_membership(
            &root,
            DirectoryMembershipSelector::XmlFiles,
            expected,
        )
        .unwrap();
    let created = root.join("Concurrent.xml");
    let mut creator = CompileTransaction::new();
    creator
        .create_bytes(&created, b"<Concurrent/>\n".to_vec())
        .unwrap();

    let acquired = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let acquired_by_guard = Arc::clone(&acquired);
    let release_guard = Arc::clone(&release);
    let guarded_thread = thread::spawn(move || {
        with_publication_lock_pause(acquired_by_guard, release_guard, || guarded.commit())
    });
    acquired.wait();

    let (contended_sender, contended_receiver) = mpsc::channel();
    let creator_thread = thread::spawn(move || {
        with_publication_lock_contention_signal(contended_sender, || creator.commit())
    });
    let contention = contended_receiver.recv_timeout(Duration::from_secs(2));
    release.wait();

    guarded_thread
        .join()
        .expect("guarded transaction thread must not panic")
        .expect("stable membership guard must commit");
    creator_thread
        .join()
        .expect("creator transaction thread must not panic")
        .expect("creator must commit after the membership guard releases");

    contention.expect("cooperating create must wait on the exclusive tree gate");
    assert_eq!(fs::read(&created).unwrap(), b"<Concurrent/>\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
pub(crate) fn identical_replacement_commits_as_a_byte_for_byte_noop() {
    let root = temp_root("replacement-noop");
    let target = root.join("Configuration.xml");
    let original = b"<MetaDataObject><Configuration/></MetaDataObject>".to_vec();
    fs::write(&target, &original).unwrap();
    let mut transaction = CompileTransaction::new();
    transaction
        .replace_bytes(&target, &original, original.clone())
        .expect("an identical replacement remains a bound transaction input");

    let report = transaction.commit().expect("no-op replacement must commit");

    assert!(report.created.is_empty());
    assert!(report.updated.is_empty());
    assert_eq!(fs::read(&target).unwrap(), original);
    assert!(transaction_debris(&root).is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn publication_cleanup_retry_preserves_a_same_name_file_replacement() {
    use super::super::single_file_publisher::{
        publish, with_publish_failpoints, PublishCheckpoint,
    };

    fn assert_hard_link_count_if_supported(path: &Path, expected: u64, message: &str) {
        let file = fs::File::open(path).expect("published target must remain readable");
        match hard_link_count(&file) {
            Ok(actual) => assert_eq!(actual, expected, "{message}"),
            Err(error) if error.kind() == ErrorKind::Unsupported => {}
            Err(error) => panic!("failed to inspect published hard links: {error}"),
        }
    }

    let root = temp_root("publication-cleanup-retry-identity-swap");
    let target = root.join("created.bin");
    let report = with_publish_failpoints(&[PublishCheckpoint::Cleanup], || {
        publish(PublishRequest {
            target: &target,
            replacement: b"published bytes",
            mode: PublishMode::CreateOnly,
        })
    })
    .expect("committed create must surface cleanup as a warning");
    let warning = report
        .cleanup_warnings
        .into_iter()
        .next()
        .expect("cleanup failpoint must retain one identity-bound warning");
    let warned_path = warning.artifact.path().to_path_buf();
    let displaced = root.join("displaced-stage.bin");
    fs::rename(&warned_path, &displaced).expect("warned artifact must be displaced");
    fs::write(&warned_path, b"same-name retry decoy").expect("retry decoy must be written");
    let mut state = PublishState::default();
    record_cleanup_warnings(&mut state, [warning]);

    let retry_errors = retry_warned_artifacts(&mut state);

    assert!(
        retry_errors
            .iter()
            .any(|error| error.contains("file identity changed")),
        "{retry_errors:?}"
    );
    assert_eq!(
        state.pending_artifact_cleanups.len(),
        1,
        "a failed retry must retain its identity-bound token"
    );
    assert_eq!(fs::read(&warned_path).unwrap(), b"same-name retry decoy");
    assert_eq!(fs::read(&displaced).unwrap(), b"published bytes");
    assert_eq!(fs::read(&target).unwrap(), b"published bytes");
    assert_hard_link_count_if_supported(
        &target,
        2,
        "the committed target and displaced owned stage must both remain",
    );
    fs::remove_file(&warned_path).expect("retry decoy must be removed");
    fs::rename(&displaced, &warned_path).expect("owned artifact route must be restored");

    let completed_retry = retry_warned_artifacts(&mut state);

    assert!(completed_retry.is_empty(), "{completed_retry:?}");
    assert!(state.pending_artifact_cleanups.is_empty());
    assert!(!warned_path.exists());
    assert_eq!(fs::read(&target).unwrap(), b"published bytes");
    assert_hard_link_count_if_supported(
        &target,
        1,
        "successful retry must remove only the owned stage link",
    );
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

#[test]
fn publication_cleanup_retry_does_not_treat_a_missing_route_as_cleaned() {
    if !testing::can_rename_parent_with_retained_cleanup_child_for_test() {
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::RenameUnderOpenHandle,
            "the platform keeps an open handle from being renamed or replaced",
        );
        return;
    }
    use super::super::single_file_publisher::{
        publish, with_publish_failpoints, PublishCheckpoint,
    };

    let root = temp_root("publication-cleanup-retry-missing-route");
    let active = root.join("active");
    let displaced = root.join("displaced");
    fs::create_dir(&active).expect("publication parent must be created");
    let target = active.join("created.bin");
    let report = with_publish_failpoints(&[PublishCheckpoint::Cleanup], || {
        publish(PublishRequest {
            target: &target,
            replacement: b"published bytes",
            mode: PublishMode::CreateOnly,
        })
    })
    .expect("committed create must surface cleanup as a warning");
    let warning = report
        .cleanup_warnings
        .into_iter()
        .next()
        .expect("cleanup failpoint must retain one identity-bound warning");
    let stage_name = warning
        .artifact
        .path()
        .file_name()
        .expect("warned stage must have a child name")
        .to_os_string();
    fs::rename(&active, &displaced).expect("publication parent must be displaced");
    let mut state = PublishState::default();
    record_cleanup_warnings(&mut state, [warning]);

    let retry_errors = retry_warned_artifacts(&mut state);

    assert!(
        retry_errors
            .iter()
            .any(|error| error.contains("route could not be resolved")),
        "{retry_errors:?}"
    );
    assert_eq!(
        state.pending_artifact_cleanups.len(),
        1,
        "an absent lexical route must retain the identity-bound cleanup token"
    );
    assert_eq!(
        fs::read(displaced.join(stage_name)).unwrap(),
        b"published bytes",
        "the owned artifact must remain available through its displaced parent"
    );
    drop(state);
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

#[test]
fn partial_recovery_cleanup_advances_the_file_guard_before_directory_retry() {
    let root = temp_root("partial-recovery-cleanup-state");
    let target = root.join("Configuration.xml");
    fs::write(&target, configuration_bytes()).expect("fixture must be written");
    let recovery_directory = root.join("reserved-recovery");
    let mut recovery = reserve_recovery_with(&target, || recovery_directory.clone())
        .expect("recovery directory must be reserved");
    let recovery_path = recovery.path();
    let permissions = crate::platform::filesystem::portable_permissions(
        &fs::metadata(&target).expect("fixture metadata must be readable"),
    );
    let artifact = write_exact_new_file(&recovery_path, b"original", &permissions)
        .expect("recovery artifact must be written");
    recovery
        .bind_artifact(artifact)
        .expect("artifact must bind to its recovery directory");
    let blocker = recovery_directory.join("blocker");
    fs::write(&blocker, b"keep directory non-empty").expect("blocker must be written");

    let first_cleanup = recovery.cleanup();

    assert_eq!(first_cleanup.diagnostics.len(), 1, "{first_cleanup:?}");
    assert!(first_cleanup.diagnostics[0].contains("recovery directory"));
    assert!(first_cleanup.artifact_warnings.is_empty());
    assert!(!recovery_path.exists());
    assert!(
        recovery.cleanup.artifact.is_none(),
        "a removed file identity must not be retried through its old route"
    );
    fs::remove_file(blocker).expect("blocker must be removed");

    assert!(recovery.cleanup().is_empty());
    assert!(!recovery_directory.exists());
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

#[test]
fn retried_recovery_file_cleanup_advances_the_directory_guard() {
    let root = temp_root("retried-recovery-cleanup-state");
    let target = root.join("Configuration.xml");
    fs::write(&target, configuration_bytes()).expect("fixture must be written");
    let recovery_directory = root.join("reserved-recovery");
    let mut recovery = reserve_recovery_with(&target, || recovery_directory.clone())
        .expect("recovery directory must be reserved");
    let recovery_path = recovery.path();
    let permissions = crate::platform::filesystem::portable_permissions(
        &fs::metadata(&target).expect("fixture metadata must be readable"),
    );
    let artifact = write_exact_new_file(&recovery_path, b"original", &permissions)
        .expect("recovery artifact must be written");
    recovery
        .bind_artifact(artifact)
        .expect("artifact must bind to its recovery directory");
    let displaced = recovery_directory.join("displaced-original");
    fs::rename(&recovery_path, &displaced).expect("owned recovery must be displaced");
    fs::write(&recovery_path, b"same-name decoy").expect("recovery decoy must be written");

    let report = recovery.cleanup();
    let mut state = PublishState::default();
    record_cleanup_warnings(&mut state, report.artifact_warnings);

    assert_eq!(state.pending_artifact_cleanups.len(), 1);
    assert_eq!(fs::read(&recovery_path).unwrap(), b"same-name decoy");
    assert_eq!(fs::read(&displaced).unwrap(), b"original");
    fs::remove_file(&recovery_path).expect("recovery decoy must be removed");
    fs::rename(&displaced, &recovery_path).expect("owned recovery route must be restored");

    assert!(retry_warned_artifacts(&mut state).is_empty());
    assert!(state.pending_artifact_cleanups.is_empty());
    assert!(
        recovery
            .cleanup
            .advance_completed_file_cleanup()
            .expect("retry state must remain readable"),
        "the owner guard must observe cleanup completed through its cloned token"
    );
    assert!(recovery.cleanup.artifact().is_none());
    assert!(recovery.cleanup.cleanup_directory().is_ok());
    assert!(recovery.cleanup().is_empty());
    assert!(!recovery_directory.exists());
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

#[test]
fn removal_plan_rejects_hard_linked_file_in_tree() {
    let root = temp_root("hard-linked-removal-plan");
    let tree = root.join("Template");
    let payload = tree.join("Ext/Template.bin");
    let alias = root.join("Template.alias.bin");
    fs::create_dir_all(payload.parent().unwrap()).unwrap();
    fs::write(&payload, b"payload-before").unwrap();
    fs::hard_link(&payload, &alias).unwrap();
    let mut transaction = CompileTransaction::new();

    let error = transaction
        .remove_path(&tree)
        .expect_err("hard-linked removal payload must be rejected");

    assert!(error.contains("multiple hard links"), "{error}");
    assert_eq!(fs::read(&payload).unwrap(), b"payload-before");
    assert_eq!(fs::read(&alias).unwrap(), b"payload-before");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn conditional_collection_removal_preserves_an_unrelated_direct_child() {
    let root = temp_root("conditional-collection-preserves-sibling");
    let collection = root.join("Forms");
    fs::create_dir_all(&collection).unwrap();
    let target = collection.join("Main.xml");
    let sibling = collection.join("Other.xml");
    fs::write(&target, b"target").unwrap();
    fs::write(&sibling, b"sibling").unwrap();
    let mut transaction = CompileTransaction::new();

    assert!(!transaction
        .remove_directory_if_only_direct_entries(&collection, vec![OsString::from("Main.xml")],)
        .unwrap());
    transaction.remove_path(&target).unwrap();
    transaction.commit().unwrap();

    assert!(collection.is_dir());
    assert!(!target.exists());
    assert_eq!(fs::read(&sibling).unwrap(), b"sibling");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn conditional_collection_removal_rejects_a_late_direct_child() {
    let root = temp_root("conditional-collection-late-child");
    let collection = root.join("Forms");
    fs::create_dir_all(&collection).unwrap();
    let target = collection.join("Main.xml");
    let late = collection.join("Late.xml");
    let owner = root.join("Owner.xml");
    let owner_before = b"<Owner>before</Owner>".to_vec();
    let owner_after = b"<Owner>after</Owner>".to_vec();
    fs::write(&target, b"target").unwrap();
    fs::write(&owner, &owner_before).unwrap();
    let mut transaction = CompileTransaction::new();
    transaction
        .replace_bytes(&owner, &owner_before, owner_after)
        .unwrap();

    assert!(transaction
        .remove_directory_if_only_direct_entries(&collection, vec![OsString::from("Main.xml")],)
        .unwrap());
    let late_for_hook = late.clone();
    let error = with_before_commit_hook(
        move |_| fs::write(&late_for_hook, b"late").unwrap(),
        || transaction.commit(),
    )
    .expect_err("late collection member must invalidate the removal snapshot");

    assert!(error.contains("removal target changed"), "{error}");
    assert_eq!(fs::read(&owner).unwrap(), owner_before);
    assert_eq!(fs::read(&target).unwrap(), b"target");
    assert_eq!(fs::read(&late).unwrap(), b"late");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn removal_recheck_rejects_late_hard_link_and_rolls_back_replacement() {
    let root = temp_root("late-hard-linked-removal");
    let owner = root.join("Owner.xml");
    let owner_before = b"<Owner><State>before</State></Owner>".to_vec();
    let owner_after = b"<Owner><State>after</State></Owner>".to_vec();
    fs::write(&owner, &owner_before).unwrap();
    let tree = root.join("Template");
    let payload = tree.join("Ext/Template.bin");
    let payload_before = b"payload-before".to_vec();
    fs::create_dir_all(payload.parent().unwrap()).unwrap();
    fs::write(&payload, &payload_before).unwrap();
    let alias = root.join("late.alias.bin");
    let payload_for_hook = payload.clone();
    let alias_for_hook = alias.clone();
    let mut transaction = CompileTransaction::new();
    transaction
        .replace_bytes(&owner, &owner_before, owner_after)
        .unwrap();
    transaction.remove_path(&tree).unwrap();

    let result = with_before_commit_hook(
        move |_| fs::hard_link(&payload_for_hook, &alias_for_hook).unwrap(),
        || transaction.commit(),
    );
    let error = result.expect_err("late hard link must abort the whole transaction");

    assert!(error.contains("multiple hard links"), "{error}");
    assert_eq!(fs::read(&owner).unwrap(), owner_before);
    assert_eq!(fs::read(&payload).unwrap(), payload_before);
    assert_eq!(fs::read(&alias).unwrap(), payload_before);
    assert!(transaction_debris(&root).is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rollback_preserves_concurrent_edit_of_create_only_target() {
    let root = temp_root("rollback-create-concurrent-replacement");
    let target = root.join("Generated.xml");
    let concurrent = b"<ConcurrentCreate/>\n".to_vec();
    let guard = root.join("Decision.bsl");
    fs::write(&guard, b"stable").unwrap();
    let mut transaction = CompileTransaction::new();
    transaction
        .create_bytes(&target, b"<Generated/>\n".to_vec())
        .unwrap();
    transaction.guard_exact_preimage(&guard, b"stable").unwrap();
    let target_for_hook = target.clone();
    let concurrent_for_hook = concurrent.clone();

    let error = with_before_rollback_mutation_hook(
        move |path| {
            assert_eq!(path, target_for_hook);
            fs::write(&target_for_hook, &concurrent_for_hook).unwrap();
        },
        || {
            transaction.commit_with_post_validation(|| {
                fs::write(&guard, b"changed").unwrap();
                Ok(())
            })
        },
    )
    .expect_err("late guard failure must not delete a concurrent replacement");

    assert!(error.contains("read guard"), "{error}");
    assert!(error.contains("rollback conflict"), "{error}");
    assert_eq!(fs::read(&target).unwrap(), concurrent);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn removal_rollback_preserves_concurrent_file_and_recovery_artifact() {
    let root = temp_root("rollback-removal-concurrent-file");
    let target = root.join("Removed.bin");
    let original = b"removed-before".to_vec();
    let concurrent = b"concurrent-file".to_vec();
    fs::write(&target, &original).unwrap();
    let mut transaction = CompileTransaction::new();
    transaction.remove_path(&target).unwrap();
    let target_for_hook = target.clone();
    let concurrent_for_hook = concurrent.clone();

    let error = with_before_removal_rollback_restore_hook(
        move |recovery, restore_target| {
            assert!(recovery.is_file(), "recovery file must be ready");
            assert_eq!(restore_target, target_for_hook);
            assert!(
                !restore_target.exists(),
                "restore target must still be absent at the race point"
            );
            fs::write(restore_target, &concurrent_for_hook).unwrap();
        },
        || {
            with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
                transaction.commit()
            })
        },
    )
    .expect_err("late validation failure must preserve a concurrent restore target");

    assert!(error.contains("rollback conflict"), "{error}");
    assert!(error.contains("recovery is preserved"), "{error}");
    assert_eq!(fs::read(&target).unwrap(), concurrent);
    let recovery_directory = transaction_debris(&root)
        .into_iter()
        .find(|path| path.is_dir())
        .expect("removed file recovery directory must be preserved");
    let recovered_original = recovery_directory.join("original");
    assert_eq!(fs::read(&recovered_original).unwrap(), original);
    assert_ne!(recovered_original, target);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn removal_rollback_preserves_concurrent_empty_directory_and_recovery_tree() {
    let root = temp_root("rollback-removal-concurrent-directory");
    let target = root.join("RemovedTree");
    let original_payload = target.join("Ext/original.bin");
    fs::create_dir_all(original_payload.parent().unwrap()).unwrap();
    fs::write(&original_payload, b"removed-tree-before").unwrap();
    let mut transaction = CompileTransaction::new();
    transaction.remove_path(&target).unwrap();
    let target_for_hook = target.clone();

    let error = with_before_removal_rollback_restore_hook(
        move |recovery, restore_target| {
            assert!(recovery.is_dir(), "recovery tree must be ready");
            assert_eq!(restore_target, target_for_hook);
            assert!(
                !restore_target.exists(),
                "restore target must still be absent at the race point"
            );
            fs::create_dir(restore_target).unwrap();
        },
        || {
            with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
                transaction.commit()
            })
        },
    )
    .expect_err("late validation failure must preserve a concurrent restore directory");

    assert!(error.contains("rollback conflict"), "{error}");
    assert!(error.contains("recovery is preserved"), "{error}");
    assert!(
        fs::read_dir(&target).unwrap().next().is_none(),
        "concurrent empty directory must not be replaced by the removed tree"
    );
    let recovery_directory = transaction_debris(&root)
        .into_iter()
        .find(|path| path.is_dir())
        .expect("removed tree recovery directory must be preserved");
    let recovered_original = recovery_directory.join("original/Ext/original.bin");
    assert_eq!(
        fs::read(recovered_original).unwrap(),
        b"removed-tree-before"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn recovery_reservation_retries_without_clobbering_an_occupied_candidate() {
    let root = temp_root("recovery-reservation-collision");
    let target = root.join("Configuration.xml");
    let occupied = root.join("occupied-recovery");
    let available = root.join("available-recovery");
    fs::write(&occupied, b"must remain exact").expect("collision fixture must be written");
    let mut candidates = vec![occupied.clone(), available.clone()].into_iter();

    let mut reservation = reserve_recovery_with(&target, || {
        candidates
            .next()
            .expect("reservation should need only one retry")
    })
    .expect("second candidate must reserve successfully");

    assert_eq!(fs::read(&occupied).unwrap(), b"must remain exact");
    assert_eq!(reservation.cleanup.directory.path, available);
    assert!(reservation.cleanup.directory.path.is_dir());
    assert!(!reservation.path().exists());
    assert!(reservation.cleanup().is_empty());
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

#[test]
fn path_identity_alias_is_protected_as_same_transaction_target() {
    let root = temp_root("path-identity-alias");
    let detour = root.join("detour");
    fs::create_dir(&detour).expect("detour directory must be created");
    let owner = root.join("Owner.xml");
    let owner_alias = detour.join("..").join("Owner.xml");
    let before = b"<Owner><State>before</State></Owner>\n";
    fs::write(&owner, before).expect("owner fixture must be written");

    let mut transaction = CompileTransaction::new();
    transaction
        .replace_bytes(
            &owner_alias,
            before,
            b"<Owner><State>after</State></Owner>\n".to_vec(),
        )
        .expect("owner alias replacement must plan");

    assert!(
        transaction
            .protects_path(&owner)
            .expect("path identities must normalize"),
        "lexical and filesystem aliases must resolve to one protected target"
    );
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

#[test]
fn exact_input_guard_reuses_replacement_preimage_through_path_alias() {
    let root = temp_root("exact-input-path-alias");
    let detour = root.join("detour");
    fs::create_dir(&detour).expect("detour directory must be created");
    let owner = root.join("Owner.xml");
    let owner_alias = detour.join("..").join("Owner.xml");
    let before = b"<Owner><State>before</State></Owner>\n";
    let after = b"<Owner><State>after</State></Owner>\n";
    fs::write(&owner, before).expect("owner fixture must be written");

    let mut transaction = CompileTransaction::new();
    transaction
        .replace_bytes(&owner_alias, before, after.to_vec())
        .expect("replacement through alias must plan");
    transaction
        .guard_or_verify_exact_preimage(&owner, before)
        .expect("same normalized preimage must reuse the replacement guard");
    transaction.commit().expect("transaction must commit");

    assert_eq!(fs::read(&owner).unwrap(), after);
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

#[test]
fn exact_input_guard_rejects_a_path_that_contains_a_planned_removal() {
    let root = temp_root("exact-input-contains-removal");
    let removed = root.join("Removed");
    let input = removed.join("Input.xml");
    let original = b"<Input><State>original</State></Input>\n";
    fs::create_dir(&removed).unwrap();
    fs::write(&input, original).unwrap();

    let mut transaction = CompileTransaction::new();
    transaction
        .remove_path(&input)
        .expect("the file must be snapshotted for removal");
    // The overlap runs the other way here: the planned removal lives below
    // the guarded path, so no snapshot entry can describe it. That is an
    // overlapping plan, not evidence, and it must be reported as one.
    let error = transaction
        .guard_or_verify_exact_preimage(&removed, original)
        .expect_err("a guard containing a planned removal must not be accepted");

    assert!(error.contains("duplicate or overlapping path"), "{error}");
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

#[test]
fn exact_input_guard_verifies_a_file_inside_a_removal_snapshot() {
    let root = temp_root("exact-input-removal-snapshot");
    let removed = root.join("Removed");
    let input = removed.join("Input.xml");
    let original = b"<Input><State>original</State></Input>\n";
    fs::create_dir(&removed).unwrap();
    fs::write(&input, original).unwrap();

    let mut transaction = CompileTransaction::new();
    transaction
        .remove_path(&removed)
        .expect("the complete subtree must be snapshotted");
    transaction
        .guard_or_verify_exact_preimage(&input, original)
        .expect("matching semantic evidence must reuse the removal snapshot");
    let error = transaction
        .guard_or_verify_exact_preimage(&input, b"<Input><State>concurrent</State></Input>\n")
        .expect_err("different semantic evidence must not reuse the removal snapshot");

    assert!(error.contains("changed while planning"), "{error}");
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

#[test]
fn binding_many_exact_read_guards_normalizes_each_path_a_constant_number_of_times() {
    let root = temp_root("exact-read-guard-normalization-complexity");
    let file_count = 64usize;
    let paths = (0..file_count)
        .map(|index| {
            let path = root.join(format!("Input{index:03}.bsl"));
            fs::write(&path, b"stable").unwrap();
            path
        })
        .collect::<Vec<_>>();
    TEST_PATH_IDENTITY_NORMALIZATIONS.with(|count| count.set(0));
    let mut transaction = CompileTransaction::new();

    for path in &paths {
        transaction
            .guard_or_verify_exact_preimage(path, b"stable")
            .unwrap();
    }

    let normalization_count = TEST_PATH_IDENTITY_NORMALIZATIONS.with(Cell::get);
    assert!(
        normalization_count <= file_count * 4,
        "binding {file_count} guards performed {normalization_count} path normalizations"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
pub(crate) fn exact_read_guard_serializes_with_owner_writer_and_rejects_stale_plan() {
    let root = temp_root("read-guard-concurrent-owner-writer");
    let owner = root.join("Owner.xml");
    let owner_before = b"<Owner><State>before</State></Owner>\n".to_vec();
    let owner_concurrent = b"<Owner><State>concurrent</State></Owner>\n".to_vec();
    let target = root.join("Generated.xml");
    fs::write(&owner, &owner_before).unwrap();

    let mut guarded_transaction = CompileTransaction::new();
    guarded_transaction
        .guard_exact_preimage(&owner, &owner_before)
        .expect("owner read guard must plan");
    guarded_transaction
        .create_bytes(&target, b"<Generated/>\n".to_vec())
        .expect("target create must plan");

    let mut owner_writer = CompileTransaction::new();
    owner_writer
        .replace_bytes(&owner, &owner_before, owner_concurrent.clone())
        .expect("owner replacement must plan");

    let acquired = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let acquired_by_writer = Arc::clone(&acquired);
    let release_writer = Arc::clone(&release);
    let writer_thread = thread::spawn(move || {
        with_publication_lock_pause(acquired_by_writer, release_writer, || owner_writer.commit())
    });
    acquired.wait();

    let (contended_sender, contended_receiver) = mpsc::channel();
    let guarded_thread = thread::spawn(move || {
        with_publication_lock_contention_signal(contended_sender, || guarded_transaction.commit())
    });
    let contention = contended_receiver.recv_timeout(Duration::from_secs(2));
    release.wait();

    writer_thread
        .join()
        .expect("owner writer thread must not panic")
        .expect("owner writer must commit");
    let error = guarded_thread
        .join()
        .expect("guarded transaction thread must not panic")
        .expect_err("stale guarded owner must reject the transaction");

    contention.expect("read guard must use the same publication lock as the owner writer");
    assert!(error.contains("read guard"), "{error}");
    assert!(error.contains("changed after planning"), "{error}");
    assert_eq!(fs::read(&owner).unwrap(), owner_concurrent);
    assert!(!target.exists());
    assert!(transaction_debris(&root).is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn exact_read_guard_rechecks_after_writes_and_rolls_back_created_target() {
    let root = temp_root("read-guard-late-owner-change");
    let owner = root.join("Owner.xml");
    let owner_before = b"<Owner><State>before</State></Owner>\n".to_vec();
    let owner_concurrent = b"<Owner><State>concurrent</State></Owner>\n".to_vec();
    let target = root.join("Generated.xml");
    fs::write(&owner, &owner_before).unwrap();

    let mut transaction = CompileTransaction::new();
    transaction
        .guard_exact_preimage(&owner, &owner_before)
        .expect("owner read guard must plan");
    transaction
        .create_bytes(&target, b"<Generated/>\n".to_vec())
        .expect("target create must plan");
    let owner_for_hook = owner.clone();
    let target_for_hook = target.clone();
    let concurrent_for_hook = owner_concurrent.clone();

    let result = with_before_commit_hook(
        move |path| {
            assert_eq!(path, target_for_hook);
            fs::write(&owner_for_hook, &concurrent_for_hook).unwrap();
        },
        || transaction.commit(),
    );
    let error = result.expect_err("late guarded owner change must roll the transaction back");

    assert!(error.contains("read guard"), "{error}");
    assert!(error.contains("changed after planning"), "{error}");
    assert_eq!(fs::read(&owner).unwrap(), owner_concurrent);
    assert!(!target.exists());
    assert!(transaction_debris(&root).is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn subtree_removal_excludes_a_stale_unchanged_descendant_transaction() {
    let root = temp_root("subtree-removal-excludes-descendant");
    let tree = root.join("tree");
    let child = tree.join("child.xml");
    let outside = root.join("outside.xml");
    fs::create_dir_all(&tree).unwrap();
    let original = b"<Child><State>original</State></Child>\n";
    fs::write(&child, original).unwrap();

    let mut descendant_transaction = CompileTransaction::new();
    descendant_transaction
        .replace_bytes(&child, original, original.to_vec())
        .unwrap();
    descendant_transaction
        .create_bytes(&outside, b"<Outside/>\n".to_vec())
        .unwrap();

    let mut removal_transaction = CompileTransaction::new();
    removal_transaction.remove_path(&tree).unwrap();

    let acquired = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let acquired_by_removal = Arc::clone(&acquired);
    let release_removal = Arc::clone(&release);
    let removal_thread = thread::spawn(move || {
        with_publication_lock_pause(acquired_by_removal, release_removal, || {
            removal_transaction.commit()
        })
    });
    acquired.wait();

    let (contended_sender, contended_receiver) = mpsc::channel();
    let descendant_thread = thread::spawn(move || {
        with_publication_lock_contention_signal(contended_sender, || {
            descendant_transaction.commit()
        })
    });
    let contention = contended_receiver.recv_timeout(Duration::from_secs(2));
    release.wait();

    let removal_result = removal_thread
        .join()
        .expect("removal transaction thread must not panic");
    let descendant_result = descendant_thread
        .join()
        .expect("descendant transaction thread must not panic");

    contention.expect("descendant publication must contend on the subtree-removal gate");
    removal_result.expect("subtree removal must commit");
    let error =
        descendant_result.expect_err("stale descendant preimage must fail after subtree removal");
    assert!(
        error.contains("invalid publication target")
            || error.contains("failed to inspect")
            || error.contains("No such file"),
        "{error}"
    );
    assert!(!tree.exists());
    assert!(!outside.exists());
    assert!(transaction_debris(&root).is_empty());
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

#[test]
fn two_create_only_transactions_contend_on_the_shared_target_lock() {
    let root = temp_root("concurrent-create-only");
    let target = root.join("shared.bin");
    let mut transaction_a = CompileTransaction::new();
    transaction_a
        .create_bytes(&target, b"from transaction A".to_vec())
        .expect("first create must plan");
    let mut transaction_b = CompileTransaction::new();
    transaction_b
        .create_bytes(&target, b"from transaction B".to_vec())
        .expect("second create must plan from the same absent preimage");

    let acquired = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let acquired_by_a = Arc::clone(&acquired);
    let release_a = Arc::clone(&release);
    let thread_a = thread::spawn(move || {
        with_publication_lock_pause(acquired_by_a, release_a, || transaction_a.commit())
    });
    acquired.wait();

    let (contended_sender, contended_receiver) = mpsc::channel();
    let thread_b = thread::spawn(move || {
        with_publication_lock_contention_signal(contended_sender, || transaction_b.commit())
    });
    let contention = contended_receiver.recv_timeout(Duration::from_secs(2));
    release.wait();

    let report_a = thread_a
        .join()
        .expect("first commit thread must not panic")
        .expect("first create transaction must commit");
    let error_b = thread_b
        .join()
        .expect("second commit thread must not panic")
        .expect_err("second create transaction must observe the committed target");
    contention.expect("second create transaction must contend on the publisher lock");
    assert_eq!(report_a.created, vec![target.clone()]);
    assert!(error_b.contains("already exists"), "{error_b}");
    assert_eq!(fs::read(&target).unwrap(), b"from transaction A");
    assert!(transaction_debris(&root).is_empty());
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

#[test]
fn create_only_collision_is_rejected_before_publication() {
    let root = temp_root("collision");
    let target = root.join("existing.txt");
    fs::write(&target, b"original").expect("fixture must be written");
    let mut transaction = CompileTransaction::new();

    let error = transaction
        .create_text(&target, "replacement")
        .expect_err("existing target must be rejected");

    assert!(error.contains("create-only"), "{error}");
    assert_eq!(fs::read(&target).unwrap(), b"original");
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

#[test]
fn prepublication_gate_failure_rolls_back_earlier_files() {
    let root = temp_root("prepublication-refusal");
    let first = root.join("first.bin");
    let second = root.join("second.bin");
    let mut transaction = CompileTransaction::new();
    transaction.create_bytes(&first, b"first".to_vec()).unwrap();
    transaction
        .create_bytes(&second, b"second".to_vec())
        .unwrap();
    let mut gates = 0;
    let result = transaction.commit_with_classified_pre_and_post_validation(
        || {
            gates += 1;
            if gates == 2 {
                Err(CommitFailure::provider("caller refused publication"))
            } else {
                Ok(())
            }
        },
        || Ok(()),
    );
    assert!(result.is_err());
    assert_eq!(gates, 2);
    assert!(!first.exists());
    assert!(!second.exists());
    assert_eq!(
        fs::read_dir(&root).unwrap().count(),
        0,
        "staging and recovery must be cleaned up"
    );
    fs::remove_dir_all(root).unwrap();
}
