use super::{
    portable_permissions, set_after_retained_directory_move_hook, RetainedDirectoryCapability,
};
use std::ffi::OsStr;
use std::{fs, io};

struct ResetMoveHooks;

impl Drop for ResetMoveHooks {
    fn drop(&mut self) {
        super::TEST_AFTER_RETAINED_DIRECTORY_MOVE.with(|slot| slot.borrow_mut().take());
        super::TEST_BEFORE_IDENTITY_BOUND_NO_REPLACE_RENAME.with(|slot| slot.borrow_mut().take());
    }
}

fn fixture() -> (tempfile::TempDir, RetainedDirectoryCapability) {
    let temporary = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temporary.path()).unwrap();
    fs::create_dir_all(root.join("payload/empty/nested")).unwrap();
    fs::write(root.join("payload/module.bsl"), b"original module").unwrap();
    let parent = RetainedDirectoryCapability::open(&root).unwrap();
    (temporary, parent)
}

#[test]
fn populated_directory_round_trip_preserves_original_objects_and_permissions() {
    let (_temporary, parent) = fixture();
    let expected = parent
        .retain_directory_child(OsStr::new("payload"))
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        expected
            .retained
            .directory
            .set_permissions(fs::Permissions::from_mode(0o750))
            .unwrap();
    }
    let permissions = portable_permissions(&expected.retained.directory.metadata().unwrap());
    let directory_identity = expected.identity();
    let nested_identity = expected
        .retain_directory_child(OsStr::new("empty"))
        .unwrap()
        .retain_directory_child(OsStr::new("nested"))
        .unwrap()
        .identity();
    let file_identity = expected
        .retain_regular_child(OsStr::new("module.bsl"))
        .unwrap()
        .identity();
    #[cfg(windows)]
    let acl_before = super::capture_windows_immutable_entry_evidence(&expected.retained.directory)
        .unwrap()
        .security_descriptor_sha256;

    let recovery = parent
        .displace_directory_child_no_replace(
            OsStr::new("payload"),
            &expected,
            OsStr::new("recovery"),
        )
        .unwrap();

    assert!(!parent.path().join("payload").exists());
    assert_eq!(recovery.identity(), directory_identity);
    assert!(std::sync::Arc::ptr_eq(
        &recovery.retained,
        &expected.retained
    ));
    recovery.validate_named_identity().unwrap();
    assert_eq!(
        recovery
            .read_relative_regular_bounded(std::path::Path::new("module.bsl"), 64)
            .unwrap(),
        b"original module"
    );
    assert!(parent.path().join("recovery/empty/nested").is_dir());

    let restored = parent
        .restore_displaced_directory_child_no_replace(&recovery, OsStr::new("payload"))
        .unwrap();

    assert!(!parent.path().join("recovery").exists());
    assert_eq!(restored.identity(), directory_identity);
    assert_eq!(restored.path(), expected.path());
    assert!(permissions.matches(&restored.retained.directory.metadata().unwrap()));
    restored.validate_named_identity().unwrap();
    assert_eq!(
        restored
            .retain_directory_child(OsStr::new("empty"))
            .unwrap()
            .retain_directory_child(OsStr::new("nested"))
            .unwrap()
            .identity(),
        nested_identity
    );
    assert_eq!(
        restored
            .retain_regular_child(OsStr::new("module.bsl"))
            .unwrap()
            .identity(),
        file_identity
    );
    #[cfg(windows)]
    assert_eq!(
        super::capture_windows_immutable_entry_evidence(&restored.retained.directory)
            .unwrap()
            .security_descriptor_sha256,
        acl_before
    );
}

#[test]
fn directory_displacement_does_not_replace_an_occupied_recovery() {
    let (_temporary, parent) = fixture();
    let expected = parent
        .retain_directory_child(OsStr::new("payload"))
        .unwrap();
    fs::create_dir(parent.path().join("recovery")).unwrap();
    fs::write(parent.path().join("recovery/foreign.txt"), b"foreign").unwrap();

    let error = parent
        .displace_directory_child_no_replace(
            OsStr::new("payload"),
            &expected,
            OsStr::new("recovery"),
        )
        .unwrap_err();

    assert!(error.into_parts().1.is_none());
    expected.validate_named_identity().unwrap();
    assert_eq!(
        fs::read(parent.path().join("payload/module.bsl")).unwrap(),
        b"original module"
    );
    assert_eq!(
        fs::read(parent.path().join("recovery/foreign.txt")).unwrap(),
        b"foreign"
    );
}

#[test]
fn directory_restore_preserves_concurrent_public_file_or_directory() {
    for directory in [false, true] {
        let (_temporary, parent) = fixture();
        let expected = parent
            .retain_directory_child(OsStr::new("payload"))
            .unwrap();
        let recovery = parent
            .displace_directory_child_no_replace(
                OsStr::new("payload"),
                &expected,
                OsStr::new("recovery"),
            )
            .unwrap();
        let public = parent.path().join("payload");
        let foreign = if directory {
            fs::create_dir(&public).unwrap();
            public.join("foreign.txt")
        } else {
            public.clone()
        };
        fs::write(&foreign, b"foreign").unwrap();

        let error = parent
            .restore_displaced_directory_child_no_replace(&recovery, OsStr::new("payload"))
            .unwrap_err();

        assert!(error.into_parts().1.is_none());
        recovery.validate_named_identity().unwrap();
        assert_eq!(fs::read(&foreign).unwrap(), b"foreign");
        assert_eq!(
            fs::read(parent.path().join("recovery/module.bsl")).unwrap(),
            b"original module"
        );
    }
}

#[test]
fn directory_move_rejects_root_foreign_parent_and_same_name() {
    let (_temporary, parent) = fixture();
    let expected = parent
        .retain_directory_child(OsStr::new("payload"))
        .unwrap();
    fs::create_dir(parent.path().join("other")).unwrap();
    let other = parent.retain_directory_child(OsStr::new("other")).unwrap();

    for result in [
        parent.displace_directory_child_no_replace(
            OsStr::new("payload"),
            &parent,
            OsStr::new("recovery"),
        ),
        other.displace_directory_child_no_replace(
            OsStr::new("payload"),
            &expected,
            OsStr::new("recovery"),
        ),
        parent.displace_directory_child_no_replace(
            OsStr::new("payload"),
            &expected,
            OsStr::new("payload"),
        ),
        parent.displace_directory_child_no_replace(
            OsStr::new("other"),
            &expected,
            OsStr::new("recovery"),
        ),
    ] {
        let (error, artifact) = result.unwrap_err().into_parts();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(artifact.is_none());
    }
    expected.validate_named_identity().unwrap();
    assert!(!parent.path().join("recovery").exists());
}

#[test]
fn directory_move_rejects_a_successor_observed_before_mutation() {
    let (_temporary, parent) = fixture();
    let expected = parent
        .retain_directory_child(OsStr::new("payload"))
        .unwrap();
    fs::rename(
        parent.path().join("payload"),
        parent.path().join("original-displaced"),
    )
    .unwrap();
    fs::create_dir(parent.path().join("payload")).unwrap();
    fs::write(parent.path().join("payload/foreign.txt"), b"foreign").unwrap();

    let error = parent
        .displace_directory_child_no_replace(
            OsStr::new("payload"),
            &expected,
            OsStr::new("recovery"),
        )
        .unwrap_err();

    assert!(error.into_parts().1.is_none());
    assert!(!parent.path().join("recovery").exists());
    assert_eq!(
        fs::read(parent.path().join("payload/foreign.txt")).unwrap(),
        b"foreign"
    );
    assert_eq!(
        fs::read(parent.path().join("original-displaced/module.bsl")).unwrap(),
        b"original module"
    );
}

#[test]
fn late_displacement_failure_returns_the_original_object_for_rollback() {
    let (_temporary, parent) = fixture();
    let _reset = ResetMoveHooks;
    let expected = parent
        .retain_directory_child(OsStr::new("payload"))
        .unwrap();
    set_after_retained_directory_move_hook(|| {
        Err(io::Error::other("injected observation failure"))
    });

    let error = parent
        .displace_directory_child_no_replace(
            OsStr::new("payload"),
            &expected,
            OsStr::new("recovery"),
        )
        .unwrap_err();
    let (_, artifact) = error.into_parts();
    let (recovery, name, moved) = artifact.expect("successful rename must remain journalable");
    assert!(moved);
    assert_eq!(name, OsStr::new("recovery"));
    assert_eq!(recovery.identity(), expected.identity());
    assert!(!parent.path().join("payload").exists());
    recovery.validate_named_identity().unwrap();

    let restored = parent
        .restore_displaced_directory_child_no_replace(&recovery, OsStr::new("payload"))
        .unwrap();
    assert_eq!(restored.identity(), expected.identity());
    assert_eq!(
        fs::read(parent.path().join("payload/module.bsl")).unwrap(),
        b"original module"
    );
}

#[test]
fn late_restore_failure_reports_the_already_restored_directory() {
    let (_temporary, parent) = fixture();
    let _reset = ResetMoveHooks;
    let expected = parent
        .retain_directory_child(OsStr::new("payload"))
        .unwrap();
    let recovery = parent
        .displace_directory_child_no_replace(
            OsStr::new("payload"),
            &expected,
            OsStr::new("recovery"),
        )
        .unwrap();
    set_after_retained_directory_move_hook(|| {
        Err(io::Error::other("injected observation failure"))
    });

    let error = parent
        .restore_displaced_directory_child_no_replace(&recovery, OsStr::new("payload"))
        .unwrap_err();
    let (_, artifact) = error.into_parts();
    let (restored, name, moved) = artifact.unwrap();

    assert!(moved);
    assert_eq!(name, OsStr::new("payload"));
    assert_eq!(restored.identity(), expected.identity());
    restored.validate_named_identity().unwrap();
    assert!(!parent.path().join("recovery").exists());
    assert_eq!(
        fs::read(parent.path().join("payload/module.bsl")).unwrap(),
        b"original module"
    );
}

#[test]
fn directory_restore_does_not_adopt_a_replaced_recovery() {
    let (_temporary, parent) = fixture();
    let expected = parent
        .retain_directory_child(OsStr::new("payload"))
        .unwrap();
    let recovery = parent
        .displace_directory_child_no_replace(
            OsStr::new("payload"),
            &expected,
            OsStr::new("recovery"),
        )
        .unwrap();
    fs::rename(
        parent.path().join("recovery"),
        parent.path().join("original-displaced"),
    )
    .unwrap();
    fs::create_dir(parent.path().join("recovery")).unwrap();
    fs::write(parent.path().join("recovery/foreign.txt"), b"foreign").unwrap();

    let error = parent
        .restore_displaced_directory_child_no_replace(&recovery, OsStr::new("payload"))
        .unwrap_err();

    assert!(error.into_parts().1.is_none());
    assert!(!parent.path().join("payload").exists());
    assert_eq!(
        fs::read(parent.path().join("recovery/foreign.txt")).unwrap(),
        b"foreign"
    );
    assert_eq!(
        fs::read(parent.path().join("original-displaced/module.bsl")).unwrap(),
        b"original module"
    );
}

#[cfg(unix)]
#[test]
fn directory_displacement_rejects_a_link_without_following_its_target() {
    let (_temporary, parent) = fixture();
    let expected = parent
        .retain_directory_child(OsStr::new("payload"))
        .unwrap();
    fs::rename(
        parent.path().join("payload"),
        parent.path().join("original-displaced"),
    )
    .unwrap();
    fs::create_dir(parent.path().join("foreign-target")).unwrap();
    fs::write(parent.path().join("foreign-target/data"), b"foreign").unwrap();
    std::os::unix::fs::symlink(
        parent.path().join("foreign-target"),
        parent.path().join("payload"),
    )
    .unwrap();

    let error = parent
        .displace_directory_child_no_replace(
            OsStr::new("payload"),
            &expected,
            OsStr::new("recovery"),
        )
        .unwrap_err();

    assert!(error.into_parts().1.is_none());
    assert!(!parent.path().join("recovery").exists());
    assert!(fs::symlink_metadata(parent.path().join("payload"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(
        fs::read(parent.path().join("foreign-target/data")).unwrap(),
        b"foreign"
    );
}

#[cfg(unix)]
#[test]
fn moved_parent_route_never_redirects_displacement_or_rollback() {
    let outer = tempfile::tempdir().unwrap();
    let active = fs::canonicalize(outer.path()).unwrap().join("active");
    let displaced_parent = active.with_file_name("displaced-parent");
    fs::create_dir_all(active.join("payload")).unwrap();
    fs::write(active.join("payload/module.bsl"), b"original").unwrap();
    let parent = RetainedDirectoryCapability::open(&active).unwrap();
    let expected = parent
        .retain_directory_child(OsStr::new("payload"))
        .unwrap();
    let _reset = ResetMoveHooks;
    let replaced = active.clone();
    let displaced = displaced_parent.clone();
    super::set_before_identity_bound_no_replace_rename_hook(move || {
        fs::rename(&replaced, &displaced).unwrap();
        fs::create_dir_all(replaced.join("payload")).unwrap();
        fs::write(replaced.join("payload/foreign.txt"), b"foreign public").unwrap();
        fs::create_dir(replaced.join("recovery")).unwrap();
        fs::write(replaced.join("recovery/foreign.txt"), b"foreign recovery").unwrap();
    });

    let error = parent
        .displace_directory_child_no_replace(
            OsStr::new("payload"),
            &expected,
            OsStr::new("recovery"),
        )
        .unwrap_err();
    let (recovery, _, _) = error.into_parts().1.unwrap();
    assert_eq!(recovery.identity(), expected.identity());
    assert!(!displaced_parent.join("payload").exists());
    assert_eq!(
        fs::read(displaced_parent.join("recovery/module.bsl")).unwrap(),
        b"original"
    );

    // Restoration uses the retained parent. Its post-validation still reports
    // that the public parent route was replaced, even though rollback moved
    // the original object back inside the original physical parent.
    let error = parent
        .restore_displaced_directory_child_no_replace(&recovery, OsStr::new("payload"))
        .unwrap_err();
    let (restored, name, moved) = error.into_parts().1.unwrap();
    assert!(moved);
    assert_eq!(name, OsStr::new("payload"));
    assert_eq!(restored.identity(), expected.identity());
    assert!(!displaced_parent.join("recovery").exists());
    assert_eq!(
        fs::read(displaced_parent.join("payload/module.bsl")).unwrap(),
        b"original"
    );
    assert_eq!(
        fs::read(active.join("payload/foreign.txt")).unwrap(),
        b"foreign public"
    );
    assert_eq!(
        fs::read(active.join("recovery/foreign.txt")).unwrap(),
        b"foreign recovery"
    );
}

#[cfg(unix)]
#[test]
fn directory_move_race_restores_the_foreign_successor_without_adopting_it() {
    let (_temporary, parent) = fixture();
    let _reset = ResetMoveHooks;
    let expected = parent
        .retain_directory_child(OsStr::new("payload"))
        .unwrap();
    let root = parent.path().to_path_buf();
    super::set_before_identity_bound_no_replace_rename_hook(move || {
        fs::rename(root.join("payload"), root.join("original-displaced")).unwrap();
        fs::create_dir(root.join("payload")).unwrap();
        fs::write(root.join("payload/foreign.txt"), b"foreign").unwrap();
    });

    let error = parent
        .displace_directory_child_no_replace(
            OsStr::new("payload"),
            &expected,
            OsStr::new("recovery"),
        )
        .unwrap_err();

    assert!(error.into_parts().1.is_none());
    assert!(!parent.path().join("recovery").exists());
    assert_eq!(
        fs::read(parent.path().join("payload/foreign.txt")).unwrap(),
        b"foreign"
    );
    assert_eq!(
        fs::read(parent.path().join("original-displaced/module.bsl")).unwrap(),
        b"original module"
    );
}

#[cfg(unix)]
#[test]
fn raced_successor_with_blocked_restore_is_reported_with_its_actual_identity() {
    let (_temporary, parent) = fixture();
    let _reset = ResetMoveHooks;
    let expected = parent
        .retain_directory_child(OsStr::new("payload"))
        .unwrap();
    let root = parent.path().to_path_buf();
    super::set_before_identity_bound_no_replace_rename_hook(move || {
        fs::rename(root.join("payload"), root.join("original-displaced")).unwrap();
        fs::create_dir(root.join("payload")).unwrap();
        fs::write(root.join("payload/foreign.txt"), b"first foreign").unwrap();
        super::set_before_identity_bound_no_replace_rename_hook(move || {
            fs::create_dir(root.join("payload")).unwrap();
            fs::write(root.join("payload/foreign.txt"), b"second foreign").unwrap();
        });
    });

    let error = parent
        .displace_directory_child_no_replace(
            OsStr::new("payload"),
            &expected,
            OsStr::new("recovery"),
        )
        .unwrap_err();
    let (_, artifact) = error.into_parts();
    let (foreign, name, moved) = artifact.unwrap();

    assert!(moved);
    assert_eq!(name, OsStr::new("recovery"));
    assert_ne!(foreign.identity(), expected.identity());
    foreign.validate_named_identity().unwrap();
    assert_eq!(
        fs::read(parent.path().join("recovery/foreign.txt")).unwrap(),
        b"first foreign"
    );
    assert_eq!(
        fs::read(parent.path().join("payload/foreign.txt")).unwrap(),
        b"second foreign"
    );
    assert_eq!(
        fs::read(parent.path().join("original-displaced/module.bsl")).unwrap(),
        b"original module"
    );
}

#[cfg(unix)]
#[test]
fn replaced_recovery_observation_keeps_the_original_descriptor_and_foreign_name() {
    let (_temporary, parent) = fixture();
    let _reset = ResetMoveHooks;
    let expected = parent
        .retain_directory_child(OsStr::new("payload"))
        .unwrap();
    let root = parent.path().to_path_buf();
    set_after_retained_directory_move_hook(move || {
        fs::rename(root.join("recovery"), root.join("original-displaced"))?;
        fs::write(root.join("recovery"), b"foreign file")?;
        Ok(())
    });

    let error = parent
        .displace_directory_child_no_replace(
            OsStr::new("payload"),
            &expected,
            OsStr::new("recovery"),
        )
        .unwrap_err();
    let (_, artifact) = error.into_parts();
    let (unbound, name, moved) = artifact.unwrap();

    assert!(moved);
    assert_eq!(name, OsStr::new("recovery"));
    assert_eq!(unbound.identity(), expected.identity());
    assert!(unbound.validate_named_identity().is_err());
    assert_eq!(
        fs::read(parent.path().join("recovery")).unwrap(),
        b"foreign file"
    );
    assert_eq!(
        fs::read(parent.path().join("original-displaced/module.bsl")).unwrap(),
        b"original module"
    );
}

#[cfg(windows)]
#[test]
fn windows_directory_move_refuses_a_descendant_without_delete_sharing() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

    let (_temporary, parent) = fixture();
    let expected = parent
        .retain_directory_child(OsStr::new("payload"))
        .unwrap();
    let external_reader = fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(parent.path().join("payload/module.bsl"))
        .unwrap();

    let error = parent
        .displace_directory_child_no_replace(
            OsStr::new("payload"),
            &expected,
            OsStr::new("recovery"),
        )
        .unwrap_err();

    assert!(error.into_parts().1.is_none());
    expected.validate_named_identity().unwrap();
    assert!(!parent.path().join("recovery").exists());
    assert_eq!(
        fs::read(parent.path().join("payload/module.bsl")).unwrap(),
        b"original module"
    );
    drop(external_reader);
    let recovery = parent
        .displace_directory_child_no_replace(
            OsStr::new("payload"),
            &expected,
            OsStr::new("recovery"),
        )
        .unwrap();
    parent
        .restore_displaced_directory_child_no_replace(&recovery, OsStr::new("payload"))
        .unwrap();
}
