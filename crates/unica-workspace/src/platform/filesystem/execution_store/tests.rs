use super::*;
use std::collections::BTreeSet;
use std::fs;

fn directory(path: &std::path::Path) -> RetainedDirectoryCapability {
    RetainedDirectoryCapability::open(&fs::canonicalize(path).unwrap()).unwrap()
}

#[cfg(unix)]
fn publish(
    parent: &RetainedDirectoryCapability,
    stage_name: &str,
    destination: &str,
    bytes: &[u8],
    mode: PublishMode,
) -> RetainedRegularFileCapability {
    let mut stage = Stage::create(parent, OsStr::new(stage_name)).unwrap();
    stage.write_all(bytes).unwrap();
    stage
        .seal()
        .unwrap()
        .publish(OsStr::new(destination), mode)
        .unwrap()
}

#[test]
fn worker_values_are_send_without_a_shared_writer() {
    fn assert_send<T: Send>() {}
    assert_send::<Stage>();
    assert_send::<SealedStage>();
    assert_send::<PublicationFailure>();
    assert_send::<DirectoryCursor>();
    assert_send::<StableLock>();
}

#[cfg(unix)]
#[test]
fn sealed_create_and_replace_confirm_exact_bytes_and_keep_old_descriptor() {
    let temp = tempfile::tempdir().unwrap();
    let parent = directory(temp.path());
    let old = publish(
        &parent,
        ".stage.first",
        "row",
        b"old",
        PublishMode::CreateNew,
    );
    let old_id = old.identity();
    let mut collision = Stage::create(&parent, OsStr::new(".stage.collision")).unwrap();
    collision.write_all(b"must not overwrite").unwrap();
    let failure = collision
        .seal()
        .unwrap()
        .publish(OsStr::new("row"), PublishMode::CreateNew)
        .unwrap_err();
    // The attempted no-replace syscall is conservatively unknown, even when
    // the observed old row happens to prove unchanged in this test process.
    assert_eq!(failure.certainty(), PublicationCertainty::MayHavePublished);
    assert_eq!(fs::read(temp.path().join("row")).unwrap(), b"old");
    let new = publish(
        &parent,
        ".stage.second",
        "row",
        b"new",
        PublishMode::Replace { expected: old_id },
    );
    assert_ne!(new.identity(), old_id);
    assert_eq!(old.read_bounded(3).unwrap(), b"old");
    assert_eq!(new.read_bounded(3).unwrap(), b"new");
    assert_eq!(fs::read(temp.path().join("row")).unwrap(), b"new");
    assert!(!temp.path().join(".stage.first").exists());
    assert!(!temp.path().join(".stage.second").exists());
    new.validate_named_identity().unwrap();
}

#[cfg(unix)]
#[test]
fn post_rename_destination_swap_preserves_foreign_file_without_restore() {
    let temp = tempfile::tempdir().unwrap();
    let parent = directory(temp.path());
    let old = publish(
        &parent,
        ".stage.old",
        "row",
        b"prior",
        PublishMode::CreateNew,
    );
    let mut stage = Stage::create(&parent, OsStr::new(".stage.candidate")).unwrap();
    stage.write_all(b"candidate A").unwrap();
    let expected_candidate = stage.identity;
    let sealed = stage.seal().unwrap();
    let path = temp.path().to_path_buf();
    let _hook = install_publication_hook(move |point| {
        if point == PublicationPoint::AfterRename {
            let visible = File::open(path.join("row"))?;
            assert_eq!(super::super::file_identity(&visible)?, expected_candidate);
            assert_eq!(fs::read(path.join("row"))?, b"candidate A");
            fs::rename(path.join("row"), path.join("witness"))?;
            fs::write(path.join("row"), b"foreign B")?;
        }
        Ok(())
    });
    let failure = sealed
        .publish(
            OsStr::new("row"),
            PublishMode::Replace {
                expected: old.identity(),
            },
        )
        .unwrap_err();
    assert_eq!(failure.certainty(), PublicationCertainty::MayHavePublished);
    assert_eq!(fs::read(temp.path().join("row")).unwrap(), b"foreign B");
    assert_eq!(
        fs::read(temp.path().join("witness")).unwrap(),
        b"candidate A"
    );
    assert_eq!(
        super::super::file_identity(&failure._candidate).unwrap(),
        expected_candidate
    );
    assert!(!temp.path().join(".stage.candidate").exists());
    assert_eq!(old.read_bounded(5).unwrap(), b"prior");
}

#[cfg(unix)]
#[test]
fn short_stage_write_cannot_be_sealed_or_publish_a_partial_candidate() {
    let temp = tempfile::tempdir().unwrap();
    let parent = directory(temp.path());
    let _prior = publish(
        &parent,
        ".stage.old",
        "row",
        b"prior",
        PublishMode::CreateNew,
    );
    let mut stage = Stage::create(&parent, OsStr::new(".stage.partial")).unwrap();
    let limit = install_stage_write_limit(3);
    assert_eq!(
        stage.write_all(b"candidate").unwrap_err().kind(),
        io::ErrorKind::StorageFull
    );
    drop(limit);
    // Removing the injected failure must not resurrect a partially failed
    // writer: both a further write and consuming seal still refuse it.
    assert_eq!(
        stage.write_all(b"rest").unwrap_err().kind(),
        io::ErrorKind::StorageFull
    );
    assert_eq!(stage.seal().unwrap_err().kind(), io::ErrorKind::StorageFull);
    assert_eq!(
        fs::read(temp.path().join(".stage.partial")).unwrap(),
        b"can"
    );
    assert_eq!(fs::read(temp.path().join("row")).unwrap(), b"prior");
}

#[cfg(unix)]
#[test]
fn publication_boundary_distinguishes_preflight_from_post_rename_flush_failure() {
    for point in [
        PublicationPoint::BeforeRename,
        PublicationPoint::BeforeFileSync,
        PublicationPoint::BeforeDirectorySync,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let parent = directory(temp.path());
        let old = publish(
            &parent,
            ".stage.old",
            "row",
            b"prior",
            PublishMode::CreateNew,
        );
        let mut stage = Stage::create(&parent, OsStr::new(".stage.new")).unwrap();
        stage.write_all(b"new").unwrap();
        let stage = stage.seal().unwrap();
        let hook = install_publication_hook(move |actual| {
            if actual == point {
                Err(io::Error::other("injected publication boundary failure"))
            } else {
                Ok(())
            }
        });
        let failure = stage
            .publish(
                OsStr::new("row"),
                PublishMode::Replace {
                    expected: old.identity(),
                },
            )
            .unwrap_err();
        let before = point == PublicationPoint::BeforeRename;
        assert_eq!(
            failure.certainty(),
            if before {
                PublicationCertainty::NeverPublished
            } else {
                PublicationCertainty::MayHavePublished
            }
        );
        assert_eq!(
            fs::read(temp.path().join("row")).unwrap(),
            if before {
                b"prior".as_slice()
            } else {
                b"new".as_slice()
            }
        );
        assert_eq!(temp.path().join(".stage.new").exists(), before);
        drop(hook);
    }
}

#[cfg(unix)]
#[test]
fn stale_expected_identity_refuses_without_overwriting_current_row() {
    let temp = tempfile::tempdir().unwrap();
    let parent = directory(temp.path());
    let old = publish(
        &parent,
        ".stage.old",
        "row",
        b"prior",
        PublishMode::CreateNew,
    );
    let current = publish(
        &parent,
        ".stage.current",
        "row",
        b"current",
        PublishMode::Replace {
            expected: old.identity(),
        },
    );
    let mut candidate = Stage::create(&parent, OsStr::new(".stage.stale")).unwrap();
    candidate.write_all(b"stale").unwrap();
    let failure = candidate
        .seal()
        .unwrap()
        .publish(
            OsStr::new("row"),
            PublishMode::Replace {
                expected: old.identity(),
            },
        )
        .unwrap_err();
    assert_eq!(failure.certainty(), PublicationCertainty::NeverPublished);
    assert_eq!(
        parent
            .retain_regular_child(OsStr::new("row"))
            .unwrap()
            .identity(),
        current.identity()
    );
    assert_eq!(fs::read(temp.path().join("row")).unwrap(), b"current");
}

#[cfg(unix)]
#[test]
fn replaced_parent_route_never_writes_the_replacement_namespace() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let retained_path = temp.path().join("retained");
    fs::create_dir(&root).unwrap();
    let parent = directory(&root);
    let old = publish(
        &parent,
        ".stage.old",
        "row",
        b"prior",
        PublishMode::CreateNew,
    );
    let mut candidate = Stage::create(&parent, OsStr::new(".stage.new")).unwrap();
    candidate.write_all(b"new").unwrap();
    let sealed = candidate.seal().unwrap();
    let hook_root = root.clone();
    let hook_retained = retained_path.clone();
    let _hook = install_publication_hook(move |point| {
        if point == PublicationPoint::BeforeRename {
            fs::rename(&hook_root, &hook_retained)?;
            fs::create_dir(&hook_root)?;
            fs::write(hook_root.join("row"), b"foreign row")?;
            fs::write(hook_root.join(".stage.new"), b"foreign stage")?;
        }
        Ok(())
    });
    let failure = sealed
        .publish(
            OsStr::new("row"),
            PublishMode::Replace {
                expected: old.identity(),
            },
        )
        .unwrap_err();
    assert_eq!(failure.certainty(), PublicationCertainty::MayHavePublished);
    assert_eq!(fs::read(root.join("row")).unwrap(), b"foreign row");
    assert_eq!(fs::read(root.join(".stage.new")).unwrap(), b"foreign stage");
    assert_eq!(fs::read(retained_path.join("row")).unwrap(), b"new");
}

#[cfg(unix)]
#[test]
fn independent_lock_contends_then_releases_while_retained_directory_stays_live() {
    let temp = tempfile::tempdir().unwrap();
    let parent = directory(temp.path());
    let _file = publish(
        &parent,
        ".stage.lock",
        "owner.lock",
        b"",
        PublishMode::CreateNew,
    );
    let first = StableLock::acquire_existing(&parent, OsStr::new("owner.lock")).unwrap();
    let contention = StableLock::acquire_existing(&parent, OsStr::new("owner.lock")).unwrap_err();
    assert!(
        super::super::is_file_lock_contended(&contention),
        "{contention:?}"
    );
    // Closing an independently opened reader neither drops nor duplicates the
    // lease. This is flock/OFD behavior, not POSIX process-record locking.
    drop(
        parent
            .retain_regular_child(OsStr::new("owner.lock"))
            .unwrap(),
    );
    assert!(StableLock::acquire_existing(&parent, OsStr::new("owner.lock")).is_err());
    let first_id = first.identity();
    first.validate().unwrap();
    drop(first);
    let second = StableLock::acquire_existing(&parent, OsStr::new("owner.lock")).unwrap();
    assert_eq!(second.identity(), first_id);
    assert!(StableLock::acquire_existing(&parent, OsStr::new("missing.lock")).is_err());
    assert!(!temp.path().join("missing.lock").exists());
}

#[test]
fn resumable_cursors_return_all_names_and_have_independent_positions() {
    let temp = tempfile::tempdir().unwrap();
    let root = directory(temp.path());
    fs::create_dir(temp.path().join("records")).unwrap();
    let records = root.retain_directory_child(OsStr::new("records")).unwrap();
    // Exceeds one Windows 64 KiB information batch while keeping the fixture
    // small. Expected names are independent of enumeration order and code.
    let expected = (0..512)
        .map(|i| {
            let name = OsString::from(format!("{i:04}-{}", "r".repeat(128)));
            fs::write(temp.path().join("records").join(&name), b"").unwrap();
            name
        })
        .collect::<BTreeSet<_>>();
    let mut left = DirectoryCursor::open(&records).unwrap();
    let mut left_names = BTreeSet::new();
    left_names.insert(left.next_name().unwrap().unwrap());
    let mut right = DirectoryCursor::open(&records).unwrap();
    let mut right_names = BTreeSet::new();
    while let Some(name) = right.next_name().unwrap() {
        assert!(right_names.insert(name));
    }
    while let Some(name) = left.next_name().unwrap() {
        assert!(left_names.insert(name));
    }
    assert_eq!(left_names, expected);
    assert_eq!(right_names, expected);
    assert!(left.next_name().unwrap().is_none());
}

#[cfg(unix)]
#[test]
fn cursor_authority_failure_never_becomes_successful_eof() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("owned"), b"").unwrap();
    let cap = directory(&root);
    let mut cursor = DirectoryCursor::open(&cap).unwrap();
    fs::rename(&root, temp.path().join("old")).unwrap();
    fs::create_dir(&root).unwrap();
    fs::write(root.join("foreign"), b"").unwrap();
    assert!(cursor.next_name().is_err());
    assert!(cursor.next_name().is_err());
    assert_eq!(fs::read(root.join("foreign")).unwrap(), b"");
}
