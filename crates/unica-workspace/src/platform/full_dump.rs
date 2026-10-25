//! Private full-dump filesystem authority, retained staging and publication.
#![cfg_attr(windows, allow(dead_code))]
#[cfg(any(unix, windows))]
use super::filesystem::hard_link_count;
#[cfg(windows)]
use super::filesystem::{
    capture_windows_immutable_entry_evidence, create_owner_only_directory_child,
    create_owner_only_file_child, delete_open_child, discard_created_child,
    open_any_child_for_delete, open_any_child_nofollow, open_directory_child_for_rename,
    open_directory_child_nofollow, open_directory_nofollow, open_regular_child_nofollow,
    opened_child_kind, rename_directory_handle_child_no_replace, verify_owner_only_acl,
    verify_unprivileged_windows_platform_caller, verify_windows_local_fixed_volume,
    OpenedChildKind, WindowsImmutableAclProfile,
};
use super::filesystem::{
    file_identity, metadata_is_link_or_reparse_point, restrict_stage_to_owner, FileIdentity,
};
use crate::paths::{normalize_leaf_path, normalize_path_identity};
use crate::single_file_publisher::{with_publication_locks_mode, PublicationTreeLockMode};
use sha2::{Digest, Sha256};
#[cfg(any(test, feature = "test-support"))]
use std::cell::RefCell;
#[cfg(any(unix, windows))]
use std::io::Read;
use std::{
    collections::BTreeMap,
    env,
    ffi::{OsStr, OsString},
    fs::{self, File},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    time::Duration,
};
#[cfg(unix)]
use std::{
    ffi::CString,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::ffi::{OsStrExt, OsStringExt},
    },
};
use unica_source_contract::{
    read::full_dump::{
        DumpSemanticValidator, DumpTreeObservation, DumpTreeReadPort, DumpValidationFailure,
    },
    workspace::WorkspaceContext,
    SourceSetKind,
};
use unica_work_control::cancellation::CancellationToken;
use uuid::Uuid;
const EFFECTIVE_CONFIG_NAME: &str = "v8project.yaml";
const LOCAL_CONFIG_NAME: &str = "v8project.local.yaml";
#[cfg(all(test, windows))]
mod windows_anchor_tests {
    use super::{
        capture_directory_snapshot, capture_tree_child_nofollow,
        capture_windows_immutable_platform_children, parse_windows_local_platform_path,
        remove_bound_directory_child, rename_child_no_replace, secure_path_is_absent,
        secure_read_regular_file_snapshot, unlink_bound_regular_child,
        with_private_cleanup_failpoint, with_private_creation_failpoint, with_secure_read_hook,
        with_tree_open_hook, DirectoryAnchor, PrivateCleanupCheckpoint, PrivateCreationCheckpoint,
        PrivateDumpStage, TreeEntryKind, TreeSnapshot,
    };
    use crate::platform::filesystem::{
        create_owner_only_file_child, file_identity, open_directory_nofollow,
        open_regular_child_nofollow, read_directory_names, verify_owner_only_acl,
    };
    use std::cell::Cell;
    use std::ffi::{OsStr, OsString};
    use std::fs;
    use std::os::windows::io::AsRawHandle;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::Storage::FileSystem::SetFileTime;

    fn enable_directory_case_sensitivity(path: &Path) {
        use std::mem::size_of;
        use std::os::windows::ffi::OsStrExt;
        use std::os::windows::io::FromRawHandle;
        use std::ptr;
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, SetFileInformationByHandle, FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY,
            FILE_DELETE_CHILD, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES,
            OPEN_EXISTING, SYNCHRONIZE,
        };

        #[repr(C)]
        struct FileCaseSensitiveInfo {
            flags: u32,
        }

        const FILE_CASE_SENSITIVE_INFO_CLASS: i32 = 23;
        const FILE_CS_FLAG_CASE_SENSITIVE_DIR: u32 = 1;

        let mut path = path.as_os_str().encode_wide().collect::<Vec<_>>();
        path.push(0);
        // SAFETY: path is NUL-terminated and the requested rights are the documented rights for
        // changing an empty directory's case-sensitivity flag.
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                FILE_ADD_FILE
                    | FILE_ADD_SUBDIRECTORY
                    | FILE_DELETE_CHILD
                    | FILE_WRITE_ATTRIBUTES
                    | SYNCHRONIZE,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                ptr::null_mut(),
            )
        };
        assert_ne!(
            handle,
            INVALID_HANDLE_VALUE,
            "{}",
            std::io::Error::last_os_error()
        );
        // SAFETY: CreateFileW returned an owned handle.
        let directory = unsafe { fs::File::from_raw_handle(handle) };
        let information = FileCaseSensitiveInfo {
            flags: FILE_CS_FLAG_CASE_SENSITIVE_DIR,
        };
        // SAFETY: directory is a live empty-directory handle and information has the exact
        // FileCaseSensitiveInfo layout and size.
        assert_ne!(
            unsafe {
                SetFileInformationByHandle(
                    directory.as_raw_handle(),
                    FILE_CASE_SENSITIVE_INFO_CLASS,
                    (&raw const information).cast(),
                    size_of::<FileCaseSensitiveInfo>() as u32,
                )
            },
            0,
            "{}",
            std::io::Error::last_os_error()
        );
    }

    fn create_case_sensitive_file(path: &Path, contents: &[u8]) -> fs::File {
        use std::io::Write;
        use std::os::windows::ffi::OsStrExt;
        use std::os::windows::io::FromRawHandle;
        use std::ptr;
        use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_POSIX_SEMANTICS,
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        };

        let mut path = path.as_os_str().encode_wide().collect::<Vec<_>>();
        path.push(0);
        // SAFETY: path is NUL-terminated. POSIX semantics make the independently prepared fixture
        // honor the already-enabled per-directory case-sensitive state.
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                ptr::null(),
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_POSIX_SEMANTICS,
                ptr::null_mut(),
            )
        };
        assert_ne!(
            handle,
            INVALID_HANDLE_VALUE,
            "{}",
            std::io::Error::last_os_error()
        );
        // SAFETY: CreateFileW returned an owned handle.
        let mut file = unsafe { fs::File::from_raw_handle(handle) };
        file.write_all(contents).unwrap();
        file.sync_all().unwrap();
        file
    }

    #[test]
    fn windows_immutable_platform_path_accepts_only_normal_or_verbatim_local_disks() {
        for path in [
            PathBuf::from(r"C:\Program Files\1cv8\8.3.27.1859"),
            PathBuf::from(r"\\?\C:\Program Files\1cv8\8.3.27.1859"),
        ] {
            let (root, names) = parse_windows_local_platform_path(&path).unwrap();
            assert!(
                root.as_path() == std::path::Path::new(r"C:\")
                    || root.as_path() == std::path::Path::new(r"\\?\C:\"),
                "{}",
                root.display()
            );
            assert_eq!(
                names.last().unwrap(),
                &std::ffi::OsString::from("8.3.27.1859")
            );
        }

        for path in [
            PathBuf::from(r"\\server\share\1cv8"),
            PathBuf::from(r"\\?\UNC\server\share\1cv8"),
            PathBuf::from(r"\\.\C:\Program Files\1cv8"),
            PathBuf::from(r"\\?\Volume{01234567-89ab-cdef-0123-456789abcdef}\1cv8"),
        ] {
            assert!(
                parse_windows_local_platform_path(&path).is_err(),
                "{}",
                path.display()
            );
        }
    }

    #[test]
    fn windows_anchor_detects_parent_name_replacement() {
        let root = unique_temp_root("anchor-replacement");
        let parent = root.join("parent");
        fs::create_dir_all(&parent).unwrap();
        let anchor = DirectoryAnchor::capture_exact(&parent).unwrap();
        let moved = root.join("moved");
        fs::rename(&parent, &moved).unwrap();
        fs::create_dir(&parent).unwrap();

        let error = anchor.verify_path_binding().unwrap_err();

        assert!(error.contains("identity"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_anchor_creates_a_private_bound_child() {
        let root = unique_temp_root("anchor-child");
        fs::create_dir_all(&root).unwrap();
        let anchor = DirectoryAnchor::capture_exact(&root).unwrap();
        let child_path = root.join("child");

        let child = anchor
            .create_child(OsStr::new("child"), &child_path)
            .unwrap();

        child.verify_path_binding().unwrap();
        verify_owner_only_acl(&child.directory).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_immutable_platform_inventory_rejects_a_multiply_linked_file() {
        let root = unique_temp_root("immutable-hard-link");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("1cv8.exe"), b"platform").unwrap();
        fs::hard_link(root.join("1cv8.exe"), root.join("alias.exe")).unwrap();
        let directory = open_directory_nofollow(&root).unwrap();
        let mut entries = Vec::new();

        let error = capture_windows_immutable_platform_children(&directory, &root, &mut entries)
            .unwrap_err();

        assert!(error.contains("exactly one hard link"), "{error}");
        drop(directory);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_immutable_platform_inventory_rejects_a_reparse_point() {
        let root = unique_temp_root("immutable-reparse");
        let outside = unique_temp_root("immutable-reparse-outside");
        fs::create_dir_all(&root).unwrap();
        fs::write(&outside, b"outside").unwrap();
        std::os::windows::fs::symlink_file(&outside, root.join("1cv8.exe")).unwrap();
        let directory = open_directory_nofollow(&root).unwrap();
        let mut entries = Vec::new();

        let error = capture_windows_immutable_platform_children(&directory, &root, &mut entries)
            .unwrap_err();

        assert!(
            error.contains("reparse") || error.contains("unsupported"),
            "{error}"
        );
        drop(directory);
        fs::remove_dir_all(root).unwrap();
        fs::remove_file(outside).unwrap();
    }

    #[test]
    fn windows_anchor_rejects_a_display_path_outside_the_parent() {
        let root = unique_temp_root("anchor-containment");
        let parent = root.join("parent");
        let outside = root.join("outside");
        fs::create_dir_all(&parent).unwrap();
        let anchor = DirectoryAnchor::capture_exact(&parent).unwrap();

        let error = anchor
            .create_child(OsStr::new("child"), &outside)
            .unwrap_err();

        assert!(error.contains("does not match anchored child"), "{error}");
        assert!(!outside.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_descendant_anchor_walks_multiple_components_from_the_retained_root() {
        let root = unique_temp_root("descendant-walk");
        let descendant = root.join("sources").join("configuration");
        fs::create_dir_all(&descendant).unwrap();
        let anchor = DirectoryAnchor::capture_exact(&root).unwrap();

        let captured = anchor
            .capture_descendant(
                PathBuf::from("sources/configuration").as_path(),
                &descendant,
            )
            .unwrap();

        assert_eq!(captured.path, descendant);
        captured.verify_path_binding().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_descendant_anchor_rejects_a_replaced_intermediate_component() {
        let root = unique_temp_root("descendant-replacement");
        let first = root.join("sources");
        let descendant = first.join("configuration");
        fs::create_dir_all(&descendant).unwrap();
        let anchor = DirectoryAnchor::capture_exact(&root).unwrap();
        let captured = anchor
            .capture_descendant(
                PathBuf::from("sources/configuration").as_path(),
                &descendant,
            )
            .unwrap();
        let captured_identity = captured.identity;
        drop(captured);
        let displaced = root.join("sources-displaced");
        fs::rename(&first, &displaced).unwrap();
        fs::create_dir_all(&descendant).unwrap();

        let error = anchor
            .verify_descendant_identity(
                PathBuf::from("sources/configuration").as_path(),
                captured_identity,
                &descendant,
            )
            .unwrap_err();

        assert!(
            error.contains("identity") || error.contains("changed"),
            "{error}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_descendant_anchor_captures_child_root_identity_and_absence() {
        let root = unique_temp_root("descendant-child-root");
        let child = root.join("source");
        fs::create_dir_all(&child).unwrap();
        let anchor = DirectoryAnchor::capture_exact(&root).unwrap();

        let identity = anchor
            .capture_child_root_identity(OsStr::new("source"), &child)
            .unwrap();
        let missing = anchor
            .capture_child_root_identity(OsStr::new("missing"), &root.join("missing"))
            .unwrap();

        assert!(identity.is_some());
        assert_eq!(missing, None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn no_clobber_directory_move_preserves_an_existing_destination() {
        let root = unique_temp_root("no-clobber");
        let source_parent_path = root.join("source-parent");
        let destination_parent_path = root.join("destination-parent");
        let source = source_parent_path.join("payload");
        let destination = destination_parent_path.join("payload");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&destination).unwrap();
        fs::write(source.join("new.txt"), b"new").unwrap();
        fs::write(destination.join("old.txt"), b"old").unwrap();

        let source_parent = DirectoryAnchor::capture_exact(&source_parent_path).unwrap();
        let destination_parent = DirectoryAnchor::capture_exact(&destination_parent_path).unwrap();
        let error = rename_child_no_replace(
            &source_parent,
            OsStr::new("payload"),
            &destination_parent,
            OsStr::new("payload"),
        )
        .unwrap_err();

        assert!(destination.join("old.txt").is_file());
        assert!(!destination.join("new.txt").exists());
        assert!(source.join("new.txt").is_file());
        assert!(!error.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn directory_move_installs_when_destination_is_absent() {
        let root = unique_temp_root("rename-success");
        let source_parent_path = root.join("source-parent");
        let destination_parent_path = root.join("destination-parent");
        let source = source_parent_path.join("payload");
        let destination = destination_parent_path.join("payload");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&destination_parent_path).unwrap();
        fs::write(source.join("new.txt"), b"new").unwrap();

        let source_parent = DirectoryAnchor::capture_exact(&source_parent_path).unwrap();
        let destination_parent = DirectoryAnchor::capture_exact(&destination_parent_path).unwrap();
        rename_child_no_replace(
            &source_parent,
            OsStr::new("payload"),
            &destination_parent,
            OsStr::new("payload"),
        )
        .unwrap();

        assert!(!source.exists());
        assert_eq!(fs::read(destination.join("new.txt")).unwrap(), b"new");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_secure_read_rejects_a_final_reparse_point() {
        let root = unique_temp_root("secure-read-reparse");
        fs::create_dir_all(&root).unwrap();
        let real = root.join("real.yaml");
        let link = root.join("v8project.yaml");
        fs::write(&real, b"project: real").unwrap();
        std::os::windows::fs::symlink_file(&real, &link).unwrap();

        let error = secure_read_regular_file_snapshot(&link, "project config").unwrap_err();

        assert!(!error.contains("unavailable"), "{error}");
        assert!(
            error.contains("reparse") || error.contains("link"),
            "{error}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_secure_read_detects_child_name_identity_replacement() {
        let root = unique_temp_root("secure-read-identity");
        fs::create_dir_all(&root).unwrap();
        let config = root.join("v8project.yaml");
        let displaced = root.join("v8project.displaced.yaml");
        let replacement = root.join("v8project.replacement.yaml");
        fs::write(&config, b"project: original").unwrap();
        fs::write(&replacement, b"project: replacement").unwrap();
        let hook_config = config.clone();
        let hook_displaced = displaced.clone();
        let hook_replacement = replacement.clone();
        let hook_ran = Rc::new(Cell::new(false));
        let hook_ran_inside = Rc::clone(&hook_ran);

        let result = with_secure_read_hook(
            move |path| {
                assert_eq!(path, hook_config);
                hook_ran_inside.set(true);
                fs::rename(&hook_config, &hook_displaced).unwrap();
                fs::rename(&hook_replacement, &hook_config).unwrap();
            },
            || secure_read_regular_file_snapshot(&config, "project config"),
        );

        let error = result.expect_err("the retained child identity must reject replacement");
        assert!(hook_ran.get(), "the secure-read hook must run");
        assert!(!error.contains("unavailable"), "{error}");
        assert!(
            error.contains("identity") || error.contains("changed"),
            "{error}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_secure_read_absence_treats_reparse_points_as_present() {
        let root = unique_temp_root("secure-absence-reparse");
        fs::create_dir_all(&root).unwrap();
        let real = root.join("real.yaml");
        let link = root.join("v8project.local.yaml");
        fs::write(&real, b"project: local").unwrap();
        std::os::windows::fs::symlink_file(&real, &link).unwrap();

        assert!(!secure_path_is_absent(&link).unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_secure_read_absence_accepts_only_a_missing_child() {
        let root = unique_temp_root("secure-absence-missing");
        fs::create_dir_all(&root).unwrap();
        let missing = root.join("v8project.local.yaml");

        assert!(secure_path_is_absent(&missing).unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_tree_snapshot_captures_nested_identities_sizes_and_digests() {
        let root = unique_temp_root("tree-snapshot-stable");
        let nested = root.join("nested");
        let payload = nested.join("payload.txt");
        fs::create_dir_all(&nested).unwrap();
        fs::write(&payload, b"abc").unwrap();
        let expected_file_identity = file_identity(&fs::File::open(&payload).unwrap()).unwrap();
        let expected_directory_identity =
            file_identity(&open_directory_nofollow(&nested).unwrap()).unwrap();

        let first = capture_directory_snapshot(&root).unwrap();
        let second = capture_directory_snapshot(&root).unwrap();

        assert_eq!(
            first, second,
            "a stable tree must produce a stable snapshot"
        );
        assert_eq!(first.len(), 2);
        assert!(first.iter().any(|entry| {
            entry.relative_path.as_path() == Path::new("nested")
                && entry.kind
                    == TreeEntryKind::Directory {
                        identity: expected_directory_identity,
                    }
        }));
        assert!(first.iter().any(|entry| {
            entry.relative_path.as_path() == Path::new("nested/payload.txt")
                && entry.kind
                    == TreeEntryKind::File {
                        identity: expected_file_identity,
                        size: 3,
                        sha256: [
                            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde,
                            0x5d, 0xae, 0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c,
                            0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
                        ],
                    }
        }));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_relative_lookup_preserves_ordinary_case_insensitive_ntfs_semantics() {
        let root = unique_temp_root("ordinary-case-insensitive-lookup");
        fs::create_dir_all(&root).unwrap();
        let exact = root.join("Entry.txt");
        fs::write(&exact, b"ordinary").unwrap();
        let expected_identity = file_identity(&fs::File::open(&exact).unwrap()).unwrap();
        let parent = open_directory_nofollow(&root).unwrap();

        let collision = create_owner_only_file_child(&parent, OsStr::new("eNTRY.TxT")).unwrap_err();
        assert_eq!(collision.kind(), std::io::ErrorKind::AlreadyExists);
        let differently_cased =
            open_regular_child_nofollow(&parent, OsStr::new("eNTRY.TxT")).unwrap();

        assert_eq!(
            file_identity(&differently_cased).unwrap(),
            expected_identity
        );
        drop(differently_cased);
        drop(parent);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_case_sensitive_siblings_bind_snapshot_inventory_and_cleanup_to_exact_names() {
        use std::io::{Read, Write};

        let root = unique_temp_root("case-sensitive-siblings");
        let target = root.join("target");
        fs::create_dir_all(&target).unwrap();
        enable_directory_case_sensitivity(&target);
        let lower_path = target.join("entry.txt");
        let lower = create_case_sensitive_file(&lower_path, b"lower-content");
        let parent = open_directory_nofollow(&target).unwrap();
        let mut upper = create_owner_only_file_child(&parent, OsStr::new("ENTRY.txt")).unwrap();
        upper.write_all(b"UPPER-CONTENT").unwrap();
        upper.sync_all().unwrap();
        let lower_identity = file_identity(&lower).unwrap();
        let upper_identity = file_identity(&upper).unwrap();
        assert_ne!(lower_identity, upper_identity);
        drop(lower);
        drop(upper);

        let ambiguous = open_regular_child_nofollow(&parent, OsStr::new("Entry.txt"))
            .expect_err("a case-sensitive parent must reject a non-exact child spelling");
        assert_eq!(ambiguous.kind(), std::io::ErrorKind::NotFound);

        let names = read_directory_names(&parent).unwrap();
        assert_eq!(
            names,
            vec![OsString::from("ENTRY.txt"), OsString::from("entry.txt")]
        );
        for (name, expected_identity, expected_contents) in [
            (
                OsStr::new("entry.txt"),
                lower_identity,
                b"lower-content".as_slice(),
            ),
            (
                OsStr::new("ENTRY.txt"),
                upper_identity,
                b"UPPER-CONTENT".as_slice(),
            ),
        ] {
            let mut child = open_regular_child_nofollow(&parent, name).unwrap();
            let mut contents = Vec::new();
            child.read_to_end(&mut contents).unwrap();
            assert_eq!(
                file_identity(&child).unwrap(),
                expected_identity,
                "{name:?}"
            );
            assert_eq!(contents, expected_contents, "{name:?}");
        }

        let snapshot = capture_directory_snapshot(&target).unwrap();
        assert_eq!(snapshot.len(), 2);
        assert!(snapshot.iter().any(|entry| {
            entry.relative_path.as_path() == Path::new("entry.txt")
                && entry.kind
                    == TreeEntryKind::File {
                        identity: lower_identity,
                        size: 13,
                        sha256: [
                            0xd4, 0x82, 0x15, 0xe7, 0x47, 0xb9, 0xf0, 0x6a, 0xdc, 0x06, 0xee, 0x08,
                            0x67, 0xdb, 0xfb, 0xf7, 0x94, 0x00, 0x67, 0x47, 0xe0, 0xf5, 0xff, 0x23,
                            0x30, 0xaf, 0x82, 0x20, 0x9f, 0x21, 0xca, 0xa3,
                        ],
                    }
        }));
        assert!(snapshot.iter().any(|entry| {
            entry.relative_path.as_path() == Path::new("ENTRY.txt")
                && entry.kind
                    == TreeEntryKind::File {
                        identity: upper_identity,
                        size: 13,
                        sha256: [
                            0xdc, 0xd4, 0x17, 0x49, 0xde, 0x8a, 0x50, 0x74, 0x7e, 0x82, 0x18, 0x46,
                            0x4a, 0x98, 0xa8, 0xbb, 0xfc, 0xe0, 0xec, 0x85, 0xb1, 0x76, 0xae, 0x0a,
                            0x77, 0xb8, 0x5e, 0xd5, 0x30, 0xc9, 0xe7, 0xd9,
                        ],
                    }
        }));

        unlink_bound_regular_child(&parent, OsStr::new("entry.txt"), lower_identity).unwrap();
        let mut survivor = open_regular_child_nofollow(&parent, OsStr::new("ENTRY.txt")).unwrap();
        let mut survivor_contents = Vec::new();
        survivor.read_to_end(&mut survivor_contents).unwrap();
        assert_eq!(file_identity(&survivor).unwrap(), upper_identity);
        assert_eq!(survivor_contents, b"UPPER-CONTENT");
        drop(survivor);
        drop(parent);
        assert!(!lower_path.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_tree_snapshot_rejects_a_reparse_target_without_traversal() {
        let root = unique_temp_root("tree-snapshot-root-reparse");
        let outside = root.join("outside");
        let target = root.join("target");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("sentinel.txt"), b"outside").unwrap();
        std::os::windows::fs::symlink_dir(&outside, &target).unwrap();

        let error = TreeSnapshot::capture_target(&target).unwrap_err();

        assert!(!error.contains("unavailable"), "{error}");
        assert!(
            error.contains("reparse") || error.contains("link") || error.contains("real directory"),
            "{error}"
        );
        assert_eq!(fs::read(outside.join("sentinel.txt")).unwrap(), b"outside");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_tree_snapshot_reports_a_missing_target_as_absent() {
        let root = unique_temp_root("tree-snapshot-absent");
        let target = root.join("target");
        fs::create_dir_all(&root).unwrap();

        assert_eq!(
            TreeSnapshot::capture_target(&target).unwrap(),
            TreeSnapshot::Absent
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_tree_snapshot_rejects_a_nested_reparse_entry_without_traversal() {
        let root = unique_temp_root("tree-snapshot-nested-reparse");
        let target = root.join("target");
        let outside = root.join("outside");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("sentinel.txt"), b"outside").unwrap();
        std::os::windows::fs::symlink_dir(&outside, target.join("link")).unwrap();

        let error = capture_directory_snapshot(&target).unwrap_err();

        assert!(!error.contains("unavailable"), "{error}");
        assert!(
            error.contains("reparse") || error.contains("link") || error.contains("unsupported"),
            "{error}"
        );
        assert_eq!(fs::read(outside.join("sentinel.txt")).unwrap(), b"outside");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_tree_snapshot_detects_identical_byte_name_replacement() {
        let root = unique_temp_root("tree-snapshot-name-replacement");
        let target = root.join("target");
        let file = target.join("payload.txt");
        let displaced = root.join("payload.displaced.txt");
        let replacement = root.join("payload.replacement.txt");
        fs::create_dir_all(&target).unwrap();
        fs::write(&file, b"identical").unwrap();
        fs::write(&replacement, b"identical").unwrap();
        let parent = open_directory_nofollow(&root).unwrap();
        let hook_file = file.clone();
        let hook_displaced = displaced.clone();
        let hook_replacement = replacement.clone();

        let result = with_tree_open_hook(
            move |path| {
                assert_eq!(path, hook_file);
                fs::rename(&hook_file, &hook_displaced).unwrap();
                fs::rename(&hook_replacement, &hook_file).unwrap();
            },
            || capture_tree_child_nofollow(&parent, OsStr::new("target"), target.as_path()),
        );

        let error = result.expect_err("snapshotting must bind each child name to its identity");
        assert!(!error.contains("unavailable"), "{error}");
        assert!(
            error.contains("identity") || error.contains("changed"),
            "{error}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_tree_snapshot_detects_a_sibling_inserted_after_enumeration() {
        let root = unique_temp_root("tree-snapshot-sibling-insertion");
        let target = root.join("target");
        let original = target.join("original.txt");
        let inserted = target.join("inserted.txt");
        fs::create_dir_all(&target).unwrap();
        fs::write(&original, b"original").unwrap();
        let parent = open_directory_nofollow(&root).unwrap();
        let hook_original = original.clone();
        let hook_inserted = inserted.clone();

        let result = with_tree_open_hook(
            move |path| {
                assert_eq!(path, hook_original);
                fs::write(&hook_inserted, b"inserted").unwrap();
            },
            || capture_tree_child_nofollow(&parent, OsStr::new("target"), target.as_path()),
        );

        let error = result.expect_err("snapshotting must reject a changed directory name set");
        assert!(
            error.contains("name") || error.contains("changed"),
            "{error}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_tree_snapshot_rejects_multiply_linked_files() {
        let root = unique_temp_root("tree-snapshot-hard-link");
        let target = root.join("target");
        let file = target.join("payload.txt");
        fs::create_dir_all(&target).unwrap();
        fs::write(&file, b"payload").unwrap();
        fs::hard_link(&file, target.join("alias.txt")).unwrap();

        let error = capture_directory_snapshot(&target).unwrap_err();

        assert!(!error.contains("unavailable"), "{error}");
        assert!(error.contains("exactly one hard link"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_tree_snapshot_detects_last_write_metadata_mutation() {
        let root = unique_temp_root("tree-snapshot-metadata");
        let target = root.join("target");
        let file = target.join("payload.txt");
        fs::create_dir_all(&target).unwrap();
        fs::write(&file, b"payload").unwrap();
        set_last_write_time(&file, 0x01d9_0000, 0x1111_1111);
        let parent = open_directory_nofollow(&root).unwrap();
        let hook_file = file.clone();

        let result = with_tree_open_hook(
            move |path| {
                assert_eq!(path, hook_file);
                set_last_write_time(&hook_file, 0x01da_0000, 0x2222_2222);
            },
            || capture_tree_child_nofollow(&parent, OsStr::new("target"), target.as_path()),
        );

        let error = result.expect_err("snapshotting must reject last-write metadata mutation");
        assert!(!error.contains("unavailable"), "{error}");
        assert!(error.contains("changed"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_handle_cleanup_removes_reparse_entries_without_traversal() {
        let root = unique_temp_root("handle-cleanup-reparse");
        let parent_path = root.join("parent");
        let tree = parent_path.join("tree");
        let nested = tree.join("nested");
        let outside = root.join("outside");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(nested.join("inside.txt"), b"inside").unwrap();
        fs::write(outside.join("sentinel.txt"), b"outside").unwrap();
        std::os::windows::fs::symlink_dir(&outside, tree.join("unsupported-link")).unwrap();
        let parent = open_directory_nofollow(&parent_path).unwrap();
        let expected_identity = file_identity(&open_directory_nofollow(&tree).unwrap()).unwrap();

        remove_bound_directory_child(&parent, OsStr::new("tree"), expected_identity, &tree)
            .unwrap();

        assert!(!tree.exists());
        assert_eq!(fs::read(outside.join("sentinel.txt")).unwrap(), b"outside");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_private_stage_cleanup_closes_retained_anchors_before_deleting_root() {
        let root = unique_temp_root("private-stage-cleanup-lifecycle");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let target_parent = DirectoryAnchor::capture_exact(&workspace).unwrap();
        let mut stage = PrivateDumpStage::create(&target_parent).unwrap();
        let private_root = stage.root.clone();

        stage.cleanup_now().unwrap();

        assert!(!private_root.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_private_stage_cleanup_retries_after_compartment_anchor_release() {
        let root = unique_temp_root("private-stage-released-compartment-retry");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let parent = DirectoryAnchor::capture_exact(&workspace).unwrap();
        let mut stage = PrivateDumpStage::create(&parent).unwrap();
        fs::create_dir(&stage.staged_tree).unwrap();
        let snapshot = super::capture_tree_target_nofollow(&stage.staged_tree).unwrap();
        stage.remember_staged_tree(&snapshot);
        let first = with_private_cleanup_failpoint(
            PrivateCleanupCheckpoint::BeforeCompartmentRemove,
            || stage.cleanup_now(),
        );
        assert!(first.unwrap_err().contains("injected"));
        let retry = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| stage.cleanup_now()));
        let dropped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(stage)));
        assert!(
            retry.is_ok(),
            "retry must return a Result after an I/O failure"
        );
        assert!(
            retry.unwrap().is_ok(),
            "retry must finish cleanup after the fault is removed"
        );
        assert!(dropped.is_ok(), "Drop must not panic after an I/O failure");
        assert!(workspace.read_dir().unwrap().next().is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_effective_config_scrub_failure_keeps_cleanup_retry_and_drop_safe() {
        let root = unique_temp_root("private-stage-scrub-cleanup-retry");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let target_parent = DirectoryAnchor::capture_exact(&workspace).unwrap();
        let mut stage = PrivateDumpStage::create(&target_parent).unwrap();
        stage.write_effective_config(b"secret\n").unwrap();
        let private_root = stage.root.clone();

        let drop_result = with_private_cleanup_failpoint(
            PrivateCleanupCheckpoint::BeforeEffectiveConfigScrub,
            || {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    for _ in 0..2 {
                        let error = stage.cleanup_now().expect_err(
                            "faulted effective-config scrubbing must fail cleanup without advancing anchors",
                        );
                        assert!(error.contains("injected"), "{error}");
                        assert!(stage.effective_config_handle.is_some());
                        assert!(stage.execution_anchor.is_some());
                    }
                    drop(stage);
                }))
            },
        );

        assert!(
            drop_result.is_ok(),
            "Drop must retry cleanup without panicking after a scrub failure"
        );
        fs::remove_dir_all(private_root).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_effective_config_scrub_failure_keeps_recovery_retry_and_drop_safe() {
        let root = unique_temp_root("private-stage-scrub-recovery-retry");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let target_parent = DirectoryAnchor::capture_exact(&workspace).unwrap();
        let mut stage = PrivateDumpStage::create(&target_parent).unwrap();
        stage.write_effective_config(b"secret\n").unwrap();
        let private_root = stage.root.clone();

        let drop_result = with_private_cleanup_failpoint(
            PrivateCleanupCheckpoint::BeforeEffectiveConfigScrub,
            || {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    for _ in 0..2 {
                        let error = stage.preserve_for_recovery().expect_err(
                            "faulted effective-config scrubbing must fail recovery preservation without advancing anchors",
                        );
                        assert!(error.contains("injected"), "{error}");
                        assert!(stage.effective_config_handle.is_some());
                        assert!(stage.execution_anchor.is_some());
                    }
                    drop(stage);
                }))
            },
        );

        assert!(
            drop_result.is_ok(),
            "Drop must retry cleanup without panicking after recovery preservation failed"
        );
        fs::remove_dir_all(private_root).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_effective_config_cleanup_without_execution_anchor_returns_an_error() {
        let root = unique_temp_root("private-stage-missing-execution-anchor");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let target_parent = DirectoryAnchor::capture_exact(&workspace).unwrap();
        let mut stage = PrivateDumpStage::create(&target_parent).unwrap();
        stage.write_effective_config(b"secret\n").unwrap();
        let execution_anchor = stage.execution_anchor.take().unwrap();

        let cleanup =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| stage.cleanup_now()));
        let error = cleanup
            .expect("cleanup must return an error instead of panicking")
            .expect_err("cleanup without the execution anchor must fail");

        assert!(error.contains("execution anchor"), "{error}");
        assert!(!stage.effective_config_secret_present);
        assert!(stage.effective_config_handle.is_some());
        stage.execution_anchor = Some(execution_anchor);
        stage.cleanup_now().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_private_stage_cleanup_unlinks_root_while_another_handle_is_open() {
        let root = unique_temp_root("private-stage-root-absence");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let target_parent = DirectoryAnchor::capture_exact(&workspace).unwrap();
        let mut stage = PrivateDumpStage::create(&target_parent).unwrap();
        let private_root = stage.root.clone();
        let extra_root_handle = open_directory_nofollow(&private_root).unwrap();

        stage.cleanup_now().unwrap();

        assert!(stage.root_removed);
        assert!(!stage.cleanup_on_drop);
        assert!(!private_root.exists());
        assert!(
            read_directory_names(&target_parent.directory)
                .unwrap()
                .into_iter()
                .all(|name| name != private_root.file_name().unwrap()),
            "POSIX-удаление должно убрать имя приватного корня до закрытия чужого дескриптора"
        );
        drop(extra_root_handle);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_failure_after_execution_creation_leaves_no_private_root() {
        let root = unique_temp_root("private-stage-partial-execution");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let target_parent = DirectoryAnchor::capture_exact(&workspace).unwrap();

        let result = with_private_creation_failpoint(
            PrivateCreationCheckpoint::AfterExecutionCreation,
            || PrivateDumpStage::create(&target_parent),
        );

        let error = match result {
            Ok(stage) => {
                drop(stage);
                panic!("construction unexpectedly passed its injected execution checkpoint")
            }
            Err(error) => error,
        };
        assert!(error.contains("injected"), "{error}");
        assert!(fs::read_dir(&workspace).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".unica-dump-guard-")));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_failure_after_recovery_creation_leaves_no_private_root() {
        let root = unique_temp_root("private-stage-partial-recovery");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let target_parent = DirectoryAnchor::capture_exact(&workspace).unwrap();

        let result = with_private_creation_failpoint(
            PrivateCreationCheckpoint::AfterRecoveryCreation,
            || PrivateDumpStage::create(&target_parent),
        );

        let error = match result {
            Ok(stage) => {
                drop(stage);
                panic!("construction unexpectedly passed its injected recovery checkpoint")
            }
            Err(error) => error,
        };
        assert!(error.contains("injected"), "{error}");
        assert!(fs::read_dir(&workspace).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".unica-dump-guard-")));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_handle_cleanup_leaves_an_identity_replacement_untouched() {
        let root = unique_temp_root("handle-cleanup-replacement");
        let parent_path = root.join("parent");
        let tree = parent_path.join("tree");
        let displaced = parent_path.join("tree-displaced");
        fs::create_dir_all(&tree).unwrap();
        fs::write(tree.join("owned.txt"), b"owned").unwrap();
        let parent = open_directory_nofollow(&parent_path).unwrap();
        let expected_identity = file_identity(&open_directory_nofollow(&tree).unwrap()).unwrap();
        fs::rename(&tree, &displaced).unwrap();
        fs::create_dir(&tree).unwrap();
        fs::write(tree.join("replacement.txt"), b"replacement").unwrap();

        let error =
            remove_bound_directory_child(&parent, OsStr::new("tree"), expected_identity, &tree)
                .unwrap_err();

        assert!(
            error.contains("identity") || error.contains("changed"),
            "{error}"
        );
        assert_eq!(
            fs::read(tree.join("replacement.txt")).unwrap(),
            b"replacement"
        );
        assert_eq!(fs::read(displaced.join("owned.txt")).unwrap(), b"owned");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_handle_cleanup_removes_an_expected_regular_file() {
        let root = unique_temp_root("handle-cleanup-file");
        let parent_path = root.join("parent");
        let file = parent_path.join("effective.yaml");
        fs::create_dir_all(&parent_path).unwrap();
        fs::write(&file, b"private").unwrap();
        let parent = open_directory_nofollow(&parent_path).unwrap();
        let expected_identity = file_identity(&fs::File::open(&file).unwrap()).unwrap();

        unlink_bound_regular_child(&parent, OsStr::new("effective.yaml"), expected_identity)
            .unwrap();

        assert!(!file.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_handle_cleanup_leaves_a_regular_file_replacement_untouched() {
        let root = unique_temp_root("handle-cleanup-file-replacement");
        let parent_path = root.join("parent");
        let file = parent_path.join("effective.yaml");
        let displaced = parent_path.join("effective.displaced.yaml");
        fs::create_dir_all(&parent_path).unwrap();
        fs::write(&file, b"owned").unwrap();
        let parent = open_directory_nofollow(&parent_path).unwrap();
        let expected_identity = file_identity(&fs::File::open(&file).unwrap()).unwrap();
        fs::rename(&file, &displaced).unwrap();
        fs::write(&file, b"replacement").unwrap();

        let error =
            unlink_bound_regular_child(&parent, OsStr::new("effective.yaml"), expected_identity)
                .unwrap_err();

        assert!(error.contains("identity"), "{error}");
        assert_eq!(fs::read(&file).unwrap(), b"replacement");
        assert_eq!(fs::read(&displaced).unwrap(), b"owned");
        fs::remove_dir_all(root).unwrap();
    }

    fn set_last_write_time(path: &std::path::Path, high: u32, low: u32) {
        let file = fs::OpenOptions::new().write(true).open(path).unwrap();
        let last_write = FILETIME {
            dwLowDateTime: low,
            dwHighDateTime: high,
        };
        // SAFETY: file owns a valid writable handle and last_write remains live for the call.
        assert_ne!(
            unsafe {
                SetFileTime(
                    file.as_raw_handle(),
                    std::ptr::null(),
                    std::ptr::null(),
                    &last_write,
                )
            },
            0,
            "{}",
            std::io::Error::last_os_error()
        );
    }

    fn unique_temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("unica-full-dump-{name}-{}", uuid::Uuid::new_v4()))
    }
}

/// A stable working-copy config, serialized across every runner invocation.
/// A retained sibling lock directory keeps the ownership namespace alive when
/// the projection is removed, independently for each working copy.
pub struct RunnerProjectStore {
    parent: DirectoryAnchor,
    directory: Option<DirectoryAnchor>,
    path: PathBuf,
    config_path: PathBuf,
    lock_anchor: DirectoryAnchor,
    _ownership: File,
    inputs: Vec<ConfigInputSnapshot>,
}

#[derive(Debug)]
pub enum RunnerProjectLockFailure {
    Cancelled,
    InvalidState(String),
}

impl From<String> for RunnerProjectLockFailure {
    fn from(message: String) -> Self {
        Self::InvalidState(message)
    }
}
impl From<&str> for RunnerProjectLockFailure {
    fn from(message: &str) -> Self {
        Self::InvalidState(message.into())
    }
}
impl std::fmt::Display for RunnerProjectLockFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => formatter
                .write_str("runner project preparation cancelled while waiting for ownership"),
            Self::InvalidState(message) => formatter.write_str(message),
        }
    }
}

impl RunnerProjectStore {
    pub fn open_and_lock(
        directory: &Path,
        cancellation: &CancellationToken,
    ) -> Result<Self, RunnerProjectLockFailure> {
        use fs2::FileExt;
        let directory = if directory.is_absolute() {
            directory.to_path_buf()
        } else {
            env::current_dir()
                .map_err(|error| error.to_string())?
                .join(directory)
        };
        let parent = ensure_private_directory_chain(
            directory.parent().ok_or("runner project has no parent")?,
        )?;
        let path = parent
            .path
            .join(directory.file_name().ok_or("runner project has no name")?);
        let lock_name = OsString::from(format!(
            ".runner-project-lock-{:x}",
            Sha256::digest(
                path.file_name()
                    .expect("runner project name was validated")
                    .as_encoded_bytes()
            )
        ));
        let lock_path = parent.path.join(&lock_name);
        let lock_anchor = ensure_private_directory_chain(&lock_path)?;
        verify_private_owner(&lock_anchor.directory).map_err(|error| error.to_string())?;
        let ownership = crate::platform::filesystem::open_directory_ownership_lock(
            &lock_anchor.directory,
            OsStr::new(".runner-project.lock"),
        )
        .map_err(|error| format!("failed to open runner project ownership: {error}"))?;
        loop {
            if cancellation.is_cancelled() {
                return Err(RunnerProjectLockFailure::Cancelled);
            }
            match ownership.try_lock_exclusive() {
                Ok(()) => break,
                Err(error) if crate::platform::filesystem::is_file_lock_contended(&error) => {
                    #[cfg(any(test, feature = "test-support"))]
                    TEST_PROJECT_LOCK_CONTENDED.with(|slot| {
                        if let Some(hook) = slot.borrow_mut().take() {
                            hook();
                        }
                    });
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(format!("failed to lock runner project: {error}").into()),
            }
        }
        parent.verify_path_binding()?;
        let mut store = Self {
            parent,
            directory: None,
            config_path: path.join(EFFECTIVE_CONFIG_NAME),
            lock_anchor,
            path,
            _ownership: ownership,
            inputs: Vec::new(),
        };
        if !secure_path_is_absent(&store.path)? {
            store.capture_directory()?;
        }
        Ok(store)
    }

    pub fn directory(&self) -> &Path {
        &self.path
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    fn capture_directory(&mut self) -> Result<(), String> {
        let name = self.path.file_name().ok_or("runner project has no name")?;
        let anchor = self
            .parent
            .capture_descendant(Path::new(name), &self.path)?;
        verify_private_owner(&anchor.directory)
            .map_err(|error| format!("runner project is not owner-only: {error}"))?;
        self.inputs = [EFFECTIVE_CONFIG_NAME, LOCAL_CONFIG_NAME]
            .into_iter()
            .map(|name| ConfigInputSnapshot::optional(self.path.join(name)))
            .collect::<Result<Vec<_>, _>>()?;
        self.directory = Some(anchor);
        Ok(())
    }

    fn ensure_directory(&mut self) -> Result<(), String> {
        if self.directory.is_some() {
            return self.verify();
        }
        self.parent.verify_path_binding()?;
        let name = self.path.file_name().ok_or("runner project has no name")?;
        let anchor = self.parent.create_child(name, &self.path)?;
        self.directory = Some(anchor);
        self.inputs = [EFFECTIVE_CONFIG_NAME, LOCAL_CONFIG_NAME]
            .into_iter()
            .map(|name| ConfigInputSnapshot::optional(self.path.join(name)))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(())
    }

    pub fn verify(&self) -> Result<(), String> {
        self.parent.verify_path_binding()?;
        self.lock_anchor.verify_path_binding()?;
        if let Some(anchor) = &self.directory {
            anchor.verify_path_binding()?;
            verify_private_owner(&anchor.directory)
                .map_err(|error| format!("runner project privacy changed: {error}"))?;
            for input in &self.inputs {
                input.recheck()?;
                if matches!(input.state, ConfigInputState::Exact(_)) {
                    let file = open_regular_child_nofollow(
                        &anchor.directory,
                        input.path.file_name().expect("tracked config has a name"),
                    )
                    .map_err(|error| error.to_string())?;
                    verify_private_owner(&file).map_err(|error| error.to_string())?;
                }
            }
        } else if !secure_path_is_absent(&self.path)? {
            return Err("runner project appeared while its ownership was held".into());
        }
        Ok(())
    }

    /// Publishes both layers as one owned projection. During a local-layer
    /// replacement/removal the primary is deliberately unreadable to the
    /// runner's conservative owner probe. Its reader also checks that the
    /// primary did not change while it inspected the local layer.
    pub fn write_serialized_projection(
        &mut self,
        projection: &SerializedDumpProjection,
    ) -> Result<(), String> {
        let primary = projection.primary.as_slice();
        let local = projection.local.as_deref();
        let local_bytes = local;
        self.ensure_directory()?;
        let previous_local = self
            .inputs
            .iter()
            .find(|input| input.path.file_name() == Some(OsStr::new(LOCAL_CONFIG_NAME)))
            .expect("both runner config names are tracked");
        let local_changed = previous_local.raw() != local_bytes;
        let guard_needed =
            local_changed && matches!(previous_local.state, ConfigInputState::Exact(_));
        if guard_needed {
            self.write_config(EFFECTIVE_CONFIG_NAME, &projection.unreadable_guard)?;
            run_runner_projection_checkpoint(
                RunnerProjectionCheckpoint::GuardInstalled,
                &self.path,
            )?;
        }
        if local_changed {
            match local {
                Some(local) => self.write_config(LOCAL_CONFIG_NAME, local)?,
                None => {
                    self.remove_config(LOCAL_CONFIG_NAME)?;
                    run_runner_projection_checkpoint(
                        RunnerProjectionCheckpoint::LocalGap,
                        &self.path,
                    )?;
                }
            }
        }
        // Any refusal above leaves the owned Null guard in place. In
        // particular, never restore a valid primary over an unconfirmed local.
        self.verify()?;
        run_runner_projection_checkpoint(RunnerProjectionCheckpoint::LocalConfirmed, &self.path)?;
        self.write_config(EFFECTIVE_CONFIG_NAME, primary)?;
        self.verify()
    }

    fn write_config(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        if ![EFFECTIVE_CONFIG_NAME, LOCAL_CONFIG_NAME].contains(&name) {
            return Err("runner project only stores its primary and local config".into());
        }
        self.ensure_directory()?;
        let anchor = self.directory.as_ref().expect("runner project was ensured");
        let stage_name = OsString::from(format!(".config-{}", Uuid::new_v4()));
        let stage_path = self.path.join(&stage_name);
        let mut file = create_regular_child_owner_only(&anchor.directory, &stage_name, &stage_path)
            .map_err(|error| format!("failed to create private runner config: {error}"))?;
        let identity = file_identity(&file).map_err(|error| error.to_string())?;
        let result = (|| {
            verify_private_owner(&file).map_err(|error| error.to_string())?;
            file.write_all(bytes).map_err(|error| error.to_string())?;
            file.sync_all().map_err(|error| error.to_string())?;
            self.verify()?;
            let expected = self
                .inputs
                .iter()
                .find(|input| input.path.file_name() == Some(OsStr::new(name)))
                .expect("both config names are tracked");
            publish_private_runner_config(
                anchor,
                &stage_name,
                identity,
                OsStr::new(name),
                expected,
            )?;
            Ok(())
        })();
        if let Err(error) = result {
            let cleanup = if secure_path_is_absent(&stage_path)? {
                Ok(())
            } else {
                unlink_bound_regular_child(&anchor.directory, &stage_name, identity)
            };
            return Err(match cleanup {
                Ok(()) => error,
                Err(cleanup) => format!("{error}; private runner config cleanup failed: {cleanup}"),
            });
        }
        let snapshot = ConfigInputSnapshot::required(self.path.join(name))?;
        *self
            .inputs
            .iter_mut()
            .find(|input| input.path.file_name() == Some(OsStr::new(name)))
            .expect("both runner config names are tracked") = snapshot;
        self.verify()
    }

    fn remove_config(&mut self, name: &str) -> Result<(), String> {
        if ![EFFECTIVE_CONFIG_NAME, LOCAL_CONFIG_NAME].contains(&name) {
            return Err("runner project only stores its primary and local config".into());
        }
        self.verify()?;
        let Some(anchor) = &self.directory else {
            return Ok(());
        };
        let input = self
            .inputs
            .iter_mut()
            .find(|input| input.path.file_name() == Some(OsStr::new(name)))
            .expect("both runner config names are tracked");
        if let ConfigInputState::Exact(snapshot) = &input.state {
            let directory =
                crate::platform::filesystem::RetainedDirectoryCapability::open(&anchor.path)
                    .map_err(|error| error.to_string())?;
            if directory.identity() != anchor.identity {
                return Err("runner project directory changed before config removal".into());
            }
            let file = directory
                .retain_regular_child(OsStr::new(name))
                .map_err(|error| error.to_string())?;
            if file.identity() != snapshot.identity {
                return Err(
                    "runner config changed before removal; replacement was left untouched".into(),
                );
            }
            file.remove_named_identity_relative_for_retained_apply(OsStr::new(&format!(
                ".config-cleanup-{}",
                Uuid::new_v4()
            )))
            .map_err(|error| format!("failed to remove retained runner config: {error}"))?;
            input.state = ConfigInputState::Absent;
        }
        self.verify()
    }

    pub fn remove_projection(&mut self) -> Result<(), String> {
        self.remove_config(EFFECTIVE_CONFIG_NAME)?;
        self.remove_config(LOCAL_CONFIG_NAME)?;
        let Some(anchor) = self.directory.as_ref() else {
            return Ok(());
        };
        if !matches!(TreeSnapshot::capture_target(&self.path)?, TreeSnapshot::Directory { entries, .. } if entries.is_empty())
        {
            return Err(
                "runner project contains an unexpected child; it was left untouched".into(),
            );
        }
        let parent =
            crate::platform::filesystem::RetainedDirectoryCapability::open(&self.parent.path)
                .map_err(|error| error.to_string())?;
        if parent.identity() != self.parent.identity {
            return Err("runner project parent changed before removal".into());
        }
        let directory = parent
            .retain_directory_child(self.path.file_name().ok_or("runner project has no name")?)
            .map_err(|error| error.to_string())?;
        if directory.identity() != anchor.identity {
            return Err(
                "runner project changed before removal; replacement was left untouched".into(),
            );
        }
        directory
            .remove_empty_named_identity_relative_for_retained_apply(OsStr::new(&format!(
                ".runner-project-cleanup-{}",
                Uuid::new_v4()
            )))
            .map_err(|error| format!("failed to remove retained runner project: {error}"))?;
        self.directory.take();
        self.inputs.clear();
        self.verify()
    }
}

fn publish_private_runner_config(
    anchor: &DirectoryAnchor,
    stage_name: &OsStr,
    stage_identity: FileIdentity,
    destination_name: &OsStr,
    expected: &ConfigInputSnapshot,
) -> Result<(), String> {
    use crate::platform::filesystem::RetainedDirectoryCapability;
    let directory =
        RetainedDirectoryCapability::open(&anchor.path).map_err(|error| error.to_string())?;
    if directory.identity() != anchor.identity {
        return Err("runner project directory changed before config publication".into());
    }
    let staged = directory
        .retain_regular_child(stage_name)
        .map_err(|error| error.to_string())?;
    if staged.identity() != stage_identity {
        return Err("private staged config identity changed before publication".into());
    }
    let recovery_name = OsString::from(format!(".config-recovery-{}", Uuid::new_v4()));
    let previous = if let ConfigInputState::Exact(snapshot) = &expected.state {
        let previous = directory
            .retain_regular_child(destination_name)
            .map_err(|error| error.to_string())?;
        if previous.identity() != snapshot.identity {
            return Err("runner config destination changed before publication; replacement was left untouched".into());
        }
        Some(
            directory
                .displace_regular_child_no_replace(destination_name, &previous, &recovery_name)
                .map_err(|error| {
                    format!(
                        "runner config destination could not be retained for replacement: {error}"
                    )
                })?,
        )
    } else {
        None
    };
    let installed = (|| {
        if destination_name == OsStr::new(LOCAL_CONFIG_NAME) && previous.is_some() {
            run_runner_projection_checkpoint(RunnerProjectionCheckpoint::LocalGap, &anchor.path)?;
        }
        directory
            .displace_regular_child_no_replace(stage_name, &staged, destination_name)
            .map_err(|error| error.to_string())
    })();
    if let Err(error) = installed {
        return Err(match previous {
            Some(previous) => match directory.restore_displaced_regular_child_no_replace(&previous, destination_name) {
                Ok(()) => format!("private runner config publication failed; previous config restored: {error}"),
                Err(restore) => format!("private runner config publication failed: {error}; original config retained at {} because restoration was refused: {restore}", anchor.path.join(&recovery_name).display()),
            },
            None => format!("private runner config publication failed: {error}"),
        });
    }
    if let Some(previous) = previous {
        previous.remove_named_identity_relative().map_err(|error| {
            format!(
                "private runner config replaced, but original config cleanup failed at {}: {error}",
                anchor.path.join(recovery_name).display()
            )
        })?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerProjectionCheckpoint {
    GuardInstalled,
    LocalGap,
    LocalConfirmed,
}

#[cfg(any(test, feature = "test-support"))]
type RunnerProjectionHook = (RunnerProjectionCheckpoint, Box<dyn FnOnce(&Path)>);

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static TEST_RUNNER_PROJECTION_HOOK: RefCell<Option<RunnerProjectionHook>> = const { RefCell::new(None) };
    static TEST_RUNNER_PROJECTION_FAILPOINT: RefCell<Option<RunnerProjectionCheckpoint>> = const { RefCell::new(None) };
}

fn run_runner_projection_checkpoint(
    checkpoint: RunnerProjectionCheckpoint,
    path: &Path,
) -> Result<(), String> {
    #[cfg(any(test, feature = "test-support"))]
    {
        let hook = TEST_RUNNER_PROJECTION_HOOK.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot
                .as_ref()
                .is_some_and(|(expected, _)| *expected == checkpoint)
            {
                slot.take().map(|(_, hook)| hook)
            } else {
                None
            }
        });
        if let Some(hook) = hook {
            hook(path);
        }
        if TEST_RUNNER_PROJECTION_FAILPOINT.with(|slot| slot.borrow().as_ref() == Some(&checkpoint))
        {
            return Err(format!(
                "injected runner projection failure at {checkpoint:?}"
            ));
        }
    }
    let _ = (checkpoint, path);
    Ok(())
}

#[cfg(any(test, feature = "test-support"))]
pub fn with_runner_projection_hook<T>(
    checkpoint: RunnerProjectionCheckpoint,
    hook: impl FnOnce(&Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<RunnerProjectionHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_RUNNER_PROJECTION_HOOK.with(|slot| slot.replace(self.0.take()));
        }
    }
    let old =
        TEST_RUNNER_PROJECTION_HOOK.with(|slot| slot.replace(Some((checkpoint, Box::new(hook)))));
    let _reset = Reset(old);
    action()
}

#[cfg(any(test, feature = "test-support"))]
pub fn with_runner_projection_failpoint<T>(
    checkpoint: RunnerProjectionCheckpoint,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<RunnerProjectionCheckpoint>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_RUNNER_PROJECTION_FAILPOINT.with(|slot| slot.replace(self.0.take()));
        }
    }
    let old = TEST_RUNNER_PROJECTION_FAILPOINT.with(|slot| slot.replace(Some(checkpoint)));
    let _reset = Reset(old);
    action()
}

fn ensure_private_directory_chain(path: &Path) -> Result<DirectoryAnchor, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata_is_link_or_reparse_point(&metadata) => {
            return Err(format!(
                "private directory resolves to a link or reparse point: {}",
                path.display()
            ));
        }
        Ok(_) => return DirectoryAnchor::capture_exact(&normalize_path_identity(path)?),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "failed to inspect private directory ancestry: {error}"
            ))
        }
    }
    let parent =
        ensure_private_directory_chain(path.parent().ok_or("private directory has no parent")?)?;
    let path = parent
        .path
        .join(path.file_name().ok_or("private directory has no name")?);
    match parent.create_child(
        path.file_name().ok_or("private directory has no name")?,
        &path,
    ) {
        Ok(anchor) => Ok(anchor),
        Err(error) => match DirectoryAnchor::capture_exact(&path) {
            Ok(anchor) => Ok(anchor),
            Err(_) => Err(error),
        },
    }
}

fn verify_private_owner(file: &File) -> std::io::Result<()> {
    crate::platform::filesystem::verify_owner_only_acl(file)
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static TEST_PROJECT_LOCK_CONTENDED: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
}

#[cfg(any(test, feature = "test-support"))]
pub fn with_test_project_lock_contention<T>(
    hook: impl FnOnce() + 'static,
    run: impl FnOnce() -> T,
) -> T {
    struct Restore(Option<Box<dyn FnOnce()>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_PROJECT_LOCK_CONTENDED.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let _restore =
        Restore(TEST_PROJECT_LOCK_CONTENDED.with(|slot| slot.replace(Some(Box::new(hook)))));
    run()
}

/// Holds the filesystem objects and platform attestation across a canonical
/// runner invocation. The caller validates the provider result before commit.
pub struct VerifiedFullDumpPublication {
    prepared: PreparedPublication,
    ready_staged_snapshot: Option<TreeSnapshot>,
    published: Option<PublishedFullDump>,
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FullDumpCleanupReason {
    VerifyPrivateStage,
    CleanupPrivateStage,
    CleanupCreatedParents,
    WriterCompletionUnconfirmed,
    RecoveryRetained,
    RecoveryLocationUnavailable,
    PublicationCleanup,
}

#[derive(Debug, Clone)]
pub struct PublishedFullDump {
    pub replaced: bool,
    pub files: u64,
    pub warnings: Vec<String>,
}

pub struct FullDumpFinish {
    pub recovery_origin: Option<PathBuf>,
    pub cleanup_reasons: Vec<FullDumpCleanupReason>,
    pub warnings: Vec<String>,
    pub retained: Result<Option<PathBuf>, String>,
}

#[derive(Debug, Clone, Copy)]
pub enum FullDumpPhase {
    Cancelled,
    Selection,
    Preparation,
    Integrity,
    Validation,
    Publication,
    Cleanup,
}

#[derive(Default, Debug)]
pub struct FullDumpFailureReport {
    pub validation: Option<DumpValidationFailure>,
    pub cleanup_reasons: Vec<FullDumpCleanupReason>,
    pub recovery_origin: Option<PathBuf>,
}

pub struct FullDumpFailure {
    pub phase: FullDumpPhase,
    pub recovery: Result<Option<PathBuf>, String>,
    pub publication_uncertain: bool,
    detail: String,
    pub report: Box<FullDumpFailureReport>,
}

impl std::fmt::Debug for FullDumpFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FullDumpFailure")
            .field("phase", &self.phase)
            .field("recovery", &self.recovery)
            .field("publication_uncertain", &self.publication_uncertain)
            .field("detail", &self.detail)
            .finish()
    }
}

impl FullDumpFailure {
    pub fn new(phase: FullDumpPhase, detail: String) -> Self {
        Self {
            phase,
            recovery: Ok(None),
            publication_uncertain: false,
            detail,
            report: Box::default(),
        }
    }
    fn validation(error: DumpValidationFailure) -> Self {
        let mut failure = Self::new(FullDumpPhase::Validation, error.to_string());
        failure.report.validation = Some(error);
        failure
    }
}

impl std::fmt::Display for FullDumpFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self.phase {
            FullDumpPhase::Cancelled => "full export cancelled before its first publication effect",
            FullDumpPhase::Selection => "pull requires one Configuration source set or an Extension source set with an explicit extension argument",
            FullDumpPhase::Preparation => {
                "full export could not prepare private staging and a trusted platform"
            }
            FullDumpPhase::Integrity => {
                "full export inputs, private config, target or platform changed before publication"
            }
            FullDumpPhase::Validation => {
                "full export produced invalid or empty source files; sources were not published"
            }
            FullDumpPhase::Publication => "full export could not publish its validated sources",
            FullDumpPhase::Cleanup => "full export private cleanup failed",
        })
    }
}

impl VerifiedFullDumpPublication {
    pub fn from_prepared(prepared: PreparedPublication) -> Self {
        Self {
            prepared,
            ready_staged_snapshot: None,
            published: None,
        }
    }

    pub fn effective_config(&self) -> &Path {
        self.prepared
            .config_store
            .as_ref()
            .expect("canonical dump owns its config store")
            .config_path()
    }

    pub fn target(&self) -> &Path {
        &self.prepared.target
    }

    pub fn source_set(&self) -> &str {
        &self.prepared.selected_source_set
    }

    pub fn selected_extension(&self) -> Option<&str> {
        self.prepared.extension.as_deref()
    }

    pub fn staged_target(&self) -> &Path {
        &self.prepared.private.staged_tree
    }

    pub fn recheck_before_dispatch(&self) -> Result<(), FullDumpFailure> {
        self.prepared
            .recheck_inputs()
            .map_err(|error| FullDumpFailure::new(FullDumpPhase::Integrity, error))
    }

    /// Attests exactly the provider selected by the runner. This never resolves
    /// another executable or rewrites provider selection in the configuration.
    pub fn bind_selected_attestation(
        &mut self,
        selected_executable: &Path,
        attestation: PlatformAttestation,
        cancellation: &CancellationToken,
    ) -> Result<(), FullDumpFailure> {
        if cancellation.is_cancelled() {
            return Err(FullDumpFailure::new(
                FullDumpPhase::Cancelled,
                "full export cancelled before selected local provider attestation".into(),
            ));
        }
        if normalize_leaf_path(selected_executable)
            .map_err(|error| FullDumpFailure::new(FullDumpPhase::Integrity, error))?
            != attestation.executable_path
        {
            return Err(FullDumpFailure::new(
                FullDumpPhase::Integrity,
                "selected local executable differs from its retained attestation".into(),
            ));
        }
        self.prepared.platform_attestation = Some(attestation);
        self.recheck_before_dispatch()
    }

    /// Creates the retained missing ancestry only after Selected is attested,
    /// before Continue lets the runner acquire its normal shared target lock.
    /// These empty directories are reversible preparation, not publication.
    pub fn prepare_dispatch_parent(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<(), FullDumpFailure> {
        if self.prepared.platform_attestation.is_none() {
            return Err(FullDumpFailure::new(
                FullDumpPhase::Integrity,
                "actual local provider was not attested before dispatch parent preparation".into(),
            ));
        }
        self.prepared.prepare_dispatch_parent(cancellation)
    }

    pub fn begin_execution(&mut self) {
        self.prepared.private.begin_execution();
    }

    pub fn confirm_execution_finished(&mut self) {
        self.prepared.private.confirm_execution_finished();
    }

    pub fn is_published(&self) -> bool {
        self.published.is_some()
    }

    /// Authenticated Ready authorizes one snapshot of the held, settled stage.
    /// It does not prove runner or descendant completion and never clears the
    /// pending-writer guard. Repeated publication uses this same snapshot.
    pub fn capture_ready_staged_tree(&mut self) -> Result<(), FullDumpFailure> {
        if self.published.is_some() || self.ready_staged_snapshot.is_some() {
            return Ok(());
        }
        if self.prepared.platform_attestation.is_none() {
            return Err(FullDumpFailure::new(
                FullDumpPhase::Integrity,
                "actual local provider was not attested before Ready capture".into(),
            ));
        }
        self.prepared
            .private
            .verify_execution_location()
            .and_then(|()| self.prepared.recheck_canonical_stage())
            .map_err(|error| FullDumpFailure::new(FullDumpPhase::Integrity, error))?;
        let stage = self
            .prepared
            .canonical_stage_anchor
            .as_ref()
            .ok_or_else(|| {
                FullDumpFailure::new(
                    FullDumpPhase::Integrity,
                    "canonical Ready capture has no held stage".into(),
                )
            })?;
        let mut entries = Vec::new();
        capture_directory_snapshot_recursive(
            &stage.directory,
            Path::new(""),
            &self.prepared.private.staged_tree,
            &mut entries,
        )
        .map_err(|error| FullDumpFailure::new(FullDumpPhase::Validation, error))?;
        entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        let snapshot = TreeSnapshot::Directory {
            identity: stage.identity,
            entries,
        };
        self.prepared
            .private
            .verify_execution_location()
            .and_then(|()| self.prepared.recheck_canonical_stage())
            .map_err(|error| FullDumpFailure::new(FullDumpPhase::Integrity, error))?;
        if snapshot.root_identity() != self.prepared.canonical_stage_identity {
            return Err(FullDumpFailure::new(
                FullDumpPhase::Integrity,
                "canonical staged dump root changed during Ready capture".into(),
            ));
        }
        self.prepared.private.remember_staged_tree(&snapshot);
        self.ready_staged_snapshot = Some(snapshot);
        run_publication_hook(PublicationCheckpoint::AfterStagedCapture);
        Ok(())
    }

    /// Publishes once but retains all project-store ownership through the
    /// runner's Published acknowledgement and terminal baseline finish.
    pub fn publish(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<PublishedFullDump, FullDumpFailure> {
        if let Some(published) = &self.published {
            return Ok(published.clone());
        }
        if self.prepared.platform_attestation.is_none() {
            return Err(FullDumpFailure::new(
                FullDumpPhase::Integrity,
                "actual local provider was not attested before publication".into(),
            ));
        }
        self.capture_ready_staged_tree()?;
        self.prepared
            .recheck_platform()
            .and_then(|()| self.prepared.recheck_canonical_stage())
            .map_err(|error| FullDumpFailure::new(FullDumpPhase::Integrity, error))?;
        let staged = self
            .ready_staged_snapshot
            .as_ref()
            .expect("Ready capture precedes publication validation");
        validate_staged_dump_snapshot(
            &self.prepared.private.staged_tree,
            self.prepared.source_kind,
            staged,
            self.prepared.semantic_validator.as_ref(),
        )
        .map_err(FullDumpFailure::validation)?;
        if self.prepared.canonical_stage_identity != staged.root_identity() {
            return Err(FullDumpFailure::new(
                FullDumpPhase::Integrity,
                "canonical staged dump root changed during validation".into(),
            ));
        }
        let files = match staged {
            TreeSnapshot::Directory { entries, .. } => entries
                .iter()
                .filter(|entry| matches!(entry.kind, TreeEntryKind::File { .. }))
                .count() as u64,
            TreeSnapshot::Absent => unreachable!("validated dump is a directory"),
        };
        let warnings = self
            .prepared
            .publish_canonical(staged, cancellation)
            .map_err(|mut failure| {
                failure.recovery = self.prepared.private.retained_location();
                failure.report.recovery_origin = Some(self.prepared.private.root.clone());
                failure.publication_uncertain = self.prepared.private.publication_uncertain;
                failure
            })?;
        let published = PublishedFullDump {
            replaced: !matches!(self.prepared.target_snapshot, TreeSnapshot::Absent),
            files,
            warnings,
        };
        self.published = Some(published.clone());
        Ok(published)
    }

    /// Completes local cleanup after the caller classifies the runner outcome.
    /// Pending writers keep private data and prepared public ancestry. Cleanup
    /// after verified publication is reported without undoing its changed result.
    pub fn finish(mut self) -> Result<FullDumpFinish, FullDumpFailure> {
        let published = self.is_published();
        let pending_writer = self.prepared.private.writer_completion_pending;
        let mut errors = Vec::new();
        let mut cleanup_reasons = Vec::new();
        if pending_writer {
            cleanup_reasons.push(FullDumpCleanupReason::WriterCompletionUnconfirmed);
        }
        let stage_cleanup = self.prepared.release_canonical_stage_for_cleanup();
        let stage_integrity_failed = stage_cleanup.is_err();
        if let Err(error) = stage_cleanup {
            cleanup_reasons.push(FullDumpCleanupReason::VerifyPrivateStage);
            errors.push(error);
        }
        if let Err(error) = self.prepared.private.cleanup_now() {
            cleanup_reasons.push(FullDumpCleanupReason::CleanupPrivateStage);
            // A failed cleanup still owns private debris. Keep its location
            // stable while Drop may retry scrubbing the held config file.
            self.prepared.private.retention = PrivateRetention::EntireStage;
            errors.push(error);
        }
        if !published && !pending_writer {
            if let Err(error) = self.prepared.cleanup_created_target_parents() {
                cleanup_reasons.push(FullDumpCleanupReason::CleanupCreatedParents);
                errors.push(error);
            }
        }
        // Cleanup can itself discover data that must be retained. Derive the
        // location after cleanup; a retained directory does not mean that the
        // public publication state is unknown.
        let retained = self.prepared.private.retained_location();
        match &retained {
            Ok(Some(_)) => cleanup_reasons.push(FullDumpCleanupReason::RecoveryRetained),
            Err(_) => cleanup_reasons.push(FullDumpCleanupReason::RecoveryLocationUnavailable),
            Ok(None) => {}
        }
        if published {
            // Stage and ledger binding checks can both describe the same
            // failed cleanup attempt. Report it once and preserve every detail.
            let mut warnings = if errors.is_empty() {
                Vec::new()
            } else {
                vec![format!(
                    "validated full dump was published, but private staging cleanup failed: {}",
                    errors.join("; ")
                )]
            };
            if let Ok(Some(path)) = &retained {
                let note = format!(
                    "private execution data retained for recovery at {}",
                    path.display()
                );
                if let Some(warning) = warnings.first_mut() {
                    warning.push_str(&format!("; {note}"));
                } else {
                    warnings.push(note);
                }
            }
            return Ok(FullDumpFinish {
                recovery_origin: Some(self.prepared.private.root.clone()),
                warnings,
                retained,
                cleanup_reasons,
            });
        }
        if !errors.is_empty() || !matches!(&retained, Ok(None)) {
            let mut failure = FullDumpFailure::new(
                if stage_integrity_failed {
                    FullDumpPhase::Integrity
                } else {
                    FullDumpPhase::Cleanup
                },
                if errors.is_empty() {
                    "private execution data was retained for recovery".into()
                } else {
                    errors.join("; ")
                },
            );
            failure.publication_uncertain = self.prepared.private.publication_uncertain;
            failure.report.cleanup_reasons = cleanup_reasons;
            failure.report.recovery_origin = Some(self.prepared.private.root.clone());
            failure.recovery = match retained {
                Ok(None) => Ok(self
                    .prepared
                    .created_target_parents
                    .first()
                    .map(|parent| parent.path.clone())),
                retained => retained,
            };
            return Err(failure);
        }
        Ok(FullDumpFinish {
            recovery_origin: None,
            cleanup_reasons,
            warnings: Vec::new(),
            retained: Ok(None),
        })
    }

    pub fn abort(self) -> Result<(), FullDumpFailure> {
        self.finish().map(|_| ())
    }
}

impl Drop for VerifiedFullDumpPublication {
    fn drop(&mut self) {
        // Release only the canonical handle. The shared ledger owns deletion
        // and retains pending writers even during unwind.
        let _ = self.prepared.release_canonical_stage_for_cleanup();
    }
}

enum DumpPreparationFailure {
    BeforeStage(String),
    PrivateSetup {
        detail: String,
        private: Box<PrivateDumpStage>,
    },
}

impl From<String> for DumpPreparationFailure {
    fn from(detail: String) -> Self {
        Self::BeforeStage(detail)
    }
}

impl From<&str> for DumpPreparationFailure {
    fn from(detail: &str) -> Self {
        Self::BeforeStage(detail.to_string())
    }
}

impl std::fmt::Debug for DumpPreparationFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BeforeStage(detail) => {
                formatter.debug_tuple("BeforeStage").field(detail).finish()
            }
            Self::PrivateSetup { detail, .. } => formatter
                .debug_struct("PrivateSetup")
                .field("detail", detail)
                .finish_non_exhaustive(),
        }
    }
}

impl DumpPreparationFailure {
    fn into_canonical_failure(self, phase: FullDumpPhase) -> FullDumpFailure {
        match self {
            Self::BeforeStage(detail) => FullDumpFailure::new(phase, detail),
            Self::PrivateSetup {
                detail,
                mut private,
            } => {
                let mut cleanup_reasons = Vec::new();
                let detail = match private.cleanup_now() {
                    Ok(()) => detail,
                    Err(cleanup) => {
                        // Match canonical finish: a reported recovery location
                        // must survive a later Drop retry of private cleanup.
                        private.retention = PrivateRetention::EntireStage;
                        cleanup_reasons.push(FullDumpCleanupReason::CleanupPrivateStage);
                        format!("{detail}; {cleanup}")
                    }
                };
                let mut failure = FullDumpFailure::new(phase, detail);
                failure.recovery = private.retained_location();
                match &failure.recovery {
                    Ok(Some(_)) => cleanup_reasons.push(FullDumpCleanupReason::RecoveryRetained),
                    Err(_) => {
                        cleanup_reasons.push(FullDumpCleanupReason::RecoveryLocationUnavailable)
                    }
                    Ok(None) => {}
                }
                failure.report.cleanup_reasons = cleanup_reasons;
                failure.report.recovery_origin = Some(private.root.clone());
                failure.publication_uncertain = private.publication_uncertain;
                failure
            }
        }
    }
}

pub struct PreparedPublication {
    semantic_validator: std::sync::Arc<dyn DumpSemanticValidator>,
    canonical_stage_anchor: Option<DirectoryAnchor>,
    canonical_stage_identity: Option<FileIdentity>,
    private: PrivateDumpStage,
    target: PathBuf,
    workspace_anchor: DirectoryAnchor,
    target_parent_relative: PathBuf,
    target_parent: DirectoryAnchor,
    target_snapshot: TreeSnapshot,
    platform_attestation: Option<PlatformAttestation>,
    target_missing_parents: Vec<OsString>,
    opened_target_parent: Option<DirectoryAnchor>,
    created_target_parents: Vec<DirectoryAnchor>,
    config_inputs: Vec<ConfigInputSnapshot>,
    config_store: Option<RunnerProjectStore>,
    selected_source_set: String,
    source_kind: SourceSetKind,
    source_set_name: Option<String>,
    extension: Option<String>,
}

impl PreparedPublication {
    fn recheck_platform(&self) -> Result<(), String> {
        self.platform_attestation
            .as_ref()
            .map_or(Ok(()), PlatformAttestation::recheck)
    }

    fn prepare_dispatch_parent(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<(), FullDumpFailure> {
        self.recheck_inputs()
            .map_err(|error| FullDumpFailure::new(FullDumpPhase::Integrity, error))?;
        for index in self.created_target_parents.len()..self.target_missing_parents.len() {
            if cancellation.is_cancelled() {
                return Err(FullDumpFailure::new(
                    FullDumpPhase::Cancelled,
                    "full export cancelled while preparing source ancestry before Continue".into(),
                ));
            }
            let parent = self
                .created_target_parents
                .last()
                .unwrap_or(&self.target_parent);
            let name = &self.target_missing_parents[index];
            let child = parent
                .create_child(name, &parent.path.join(name))
                .map_err(|error| FullDumpFailure::new(FullDumpPhase::Preparation, error))?;
            self.created_target_parents.push(child);
        }
        if cancellation.is_cancelled() {
            return Err(FullDumpFailure::new(
                FullDumpPhase::Cancelled,
                "full export cancelled before authorizing its prepared source parent".into(),
            ));
        }
        if self.opened_target_parent.is_none() {
            self.opened_target_parent = Some(
                self.created_target_parents
                    .last()
                    .unwrap_or(&self.target_parent)
                    .try_clone()
                    .map_err(|error| FullDumpFailure::new(FullDumpPhase::Preparation, error))?,
            );
        }
        self.recheck_inputs()
            .map_err(|error| FullDumpFailure::new(FullDumpPhase::Integrity, error))
    }

    fn cleanup_created_target_parents(&mut self) -> Result<(), String> {
        // Release the duplicate final-parent handle before Windows deletion.
        self.opened_target_parent.take();
        while let Some(child) = self.created_target_parents.last() {
            let parent = self
                .created_target_parents
                .iter()
                .rev()
                .nth(1)
                .unwrap_or(&self.target_parent);
            remove_created_source_parent(parent, child).map_err(|error| format!(
                "prepared source parent was retained because its empty identity-bound cleanup was refused: {error}"))?;
            self.created_target_parents.pop();
        }
        Ok(())
    }

    fn recheck_canonical_stage(&self) -> Result<(), String> {
        let Some(stage) = self.canonical_stage_anchor.as_ref() else {
            return Ok(());
        };
        self.private.execution_anchor().verify_descendant_identity(
            Path::new("staged-source"),
            stage.identity,
            &self.private.staged_tree,
        )?;
        verify_private_owner(&stage.directory)
            .map_err(|error| format!("canonical staged dump privacy changed: {error}"))
    }

    /// Closing the Windows handle does not transfer cleanup authority. Keep
    /// the expected identity until the original child name is confirmed absent
    /// or the existing identity-bound cleanup owner removes that exact child.
    fn release_canonical_stage_for_cleanup(&mut self) -> Result<(), String> {
        let Some(expected) = self.canonical_stage_identity else {
            self.canonical_stage_anchor.take();
            return Ok(());
        };
        if self.private.writer_completion_pending {
            self.canonical_stage_anchor.take();
            self.canonical_stage_identity = None;
            return Ok(());
        }
        let observed = self.private.execution_anchor.as_ref().map_or_else(
            || {
                if self.private.retention != PrivateRetention::None {
                    Ok(None)
                } else {
                    Err("canonical stage cleanup lost its retained execution authority".into())
                }
            },
            |execution| {
                execution.capture_child_root_identity(
                    OsStr::new("staged-source"),
                    &self.private.staged_tree,
                )
            },
        );
        self.canonical_stage_anchor.take();
        self.canonical_stage_identity = None;
        match observed {
            Ok(None) => Ok(()),
            Ok(Some(identity)) if identity == expected => Ok(()),
            Ok(Some(_)) => {
                self.private.retention = PrivateRetention::EntireStage;
                Err("canonical stage identity changed before cleanup; foreign stage was left untouched".into())
            }
            Err(error) => {
                self.private.retention = PrivateRetention::EntireStage;
                Err(error)
            }
        }
    }

    fn recheck_inputs(&self) -> Result<(), String> {
        self.recheck_canonical_stage()?;
        if let Some(store) = &self.config_store {
            store.verify()?;
        }
        for snapshot in &self.config_inputs {
            snapshot.recheck()?;
        }
        self.workspace_anchor.verify_descendant_identity(
            &self.target_parent_relative,
            self.target_parent.identity,
            &self.target_parent.path,
        )?;
        let mut target_parent = &self.target_parent;
        for child in &self.created_target_parents {
            target_parent.verify_descendant_identity(
                Path::new(
                    child
                        .path
                        .file_name()
                        .ok_or("created source parent has no name")?,
                ),
                child.identity,
                &child.path,
            )?;
            target_parent = child;
        }
        if let Some(next_missing) = self
            .target_missing_parents
            .get(self.created_target_parents.len())
        {
            if !matches!(
                target_parent
                    .capture_child(next_missing, &target_parent.path.join(next_missing))?,
                TreeSnapshot::Absent
            ) {
                return Err("missing dump target ancestry appeared after preparation; replacement was left untouched".into());
            }
            return Ok(());
        }
        if let Some(opened) = &self.opened_target_parent {
            if opened.identity != target_parent.identity {
                return Err("prepared dispatch parent identity changed".into());
            }
            opened.verify_path_binding()?;
        }
        let current = target_parent.capture_child(
            self.target.file_name().ok_or("dump target has no name")?,
            &self.target,
        )?;
        if current != self.target_snapshot {
            return Err(format!(
                "dump target changed after it was inspected; staged output was not published: {}",
                self.target.display()
            ));
        }
        Ok(())
    }

    fn publish_canonical(
        &mut self,
        staged_snapshot: &TreeSnapshot,
        cancellation: &CancellationToken,
    ) -> Result<Vec<String>, FullDumpFailure> {
        let target = self.target.clone();
        let mut cancelled_before_effect = false;
        let publication = with_publication_locks_mode(
            std::slice::from_ref(&target),
            PublicationTreeLockMode::Exclusive,
            |_| {
                self.recheck_inputs()?;
                self.recheck_platform()?;
                let current = validate_staged_dump_with(
                    &self.private.staged_tree,
                    self.source_kind,
                    self.semantic_validator.as_ref(),
                )?;
                if &current != staged_snapshot {
                    return Err("private staged dump changed after validation".into());
                }
                let parent = self
                    .opened_target_parent
                    .as_ref()
                    .or_else(|| {
                        self.target_missing_parents
                            .is_empty()
                            .then_some(&self.target_parent)
                    })
                    .ok_or("source target parent was not prepared before Continue")?;
                // Windows needs the created handle closed for publication.
                // canonical_stage_identity remains until terminal cleanup can
                // confirm that the original child moved or still belongs to us.
                self.canonical_stage_anchor.take();
                publish_staged_tree(
                    &mut self.private,
                    &self.target,
                    parent,
                    &self.target_snapshot,
                    staged_snapshot,
                    Some(PublicationCancellationGate {
                        cancellation,
                        cancelled_before_effect: &mut cancelled_before_effect,
                    }),
                )
            },
        )
        .map_err(|error| {
            FullDumpFailure::new(
                FullDumpPhase::Publication,
                format!("failed to acquire verified dump publication locks: {error}"),
            )
        })?;
        publication.map_err(|error| {
            FullDumpFailure::new(
                if cancelled_before_effect {
                    FullDumpPhase::Cancelled
                } else {
                    FullDumpPhase::Publication
                },
                error,
            )
        })
    }

    fn publish(
        &mut self,
        staged_snapshot: &TreeSnapshot,
        cancellation: &CancellationToken,
    ) -> Result<Vec<String>, String> {
        let target = self.target.clone();
        with_publication_locks_mode(
            std::slice::from_ref(&target),
            PublicationTreeLockMode::Exclusive,
            |_| {
                self.recheck_inputs()?;
                self.recheck_platform()?;
                let current = validate_staged_dump_with(
                    &self.private.staged_tree,
                    self.source_kind,
                    self.semantic_validator.as_ref(),
                )?;
                if &current != staged_snapshot {
                    return Err("private staged dump changed after validation".to_string());
                }
                if cancellation.is_cancelled() {
                    return Err("verified dump cancelled before publication".to_string());
                }
                publish_staged_tree(
                    &mut self.private,
                    &self.target,
                    &self.target_parent,
                    &self.target_snapshot,
                    staged_snapshot,
                    None,
                )
            },
        )
        .map_err(|error| format!("failed to acquire verified dump publication locks: {error}"))?
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConfigInputState {
    Exact(SecureFileSnapshot),
    Absent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigInputSnapshot {
    path: PathBuf,
    state: ConfigInputState,
}

impl ConfigInputSnapshot {
    pub fn required(path: PathBuf) -> Result<Self, String> {
        let snapshot = secure_read_regular_file_snapshot(&path, "project config")?;
        Ok(Self {
            path,
            state: ConfigInputState::Exact(snapshot),
        })
    }

    pub fn optional(path: PathBuf) -> Result<Self, String> {
        match secure_read_regular_file_snapshot(&path, "local config overlay") {
            Ok(snapshot) => Ok(Self {
                state: ConfigInputState::Exact(snapshot),
                path,
            }),
            Err(read_error) => {
                if secure_path_is_absent(&path)? {
                    Ok(Self {
                        path: normalize_leaf_path(&path)?,
                        state: ConfigInputState::Absent,
                    })
                } else {
                    Err(read_error)
                }
            }
        }
    }

    pub fn raw(&self) -> Option<&[u8]> {
        match &self.state {
            ConfigInputState::Exact(snapshot) => Some(&snapshot.raw),
            ConfigInputState::Absent => None,
        }
    }

    pub fn recheck(&self) -> Result<(), String> {
        match &self.state {
            ConfigInputState::Exact(expected) => {
                let actual =
                    secure_read_regular_file_snapshot(&self.path, "project config preimage")?;
                if &actual != expected {
                    return Err(format!(
                        "project config changed during full dump; staged output was not published: {}",
                        self.path.display()
                    ));
                }
            }
            ConfigInputState::Absent => match secure_path_is_absent(&self.path) {
                Ok(true) => {}
                Ok(false) => {
                    return Err(format!(
                        "local config overlay appeared during full dump; staged output was not published: {}",
                        self.path.display()
                    ));
                }
                Err(error) => {
                    return Err(format!(
                        "failed to recheck absent local config overlay {}: {error}",
                        self.path.display()
                    ));
                }
            },
        }
        Ok(())
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn secure_read_regular_file(path: &Path, role: &str) -> Result<Vec<u8>, String> {
    secure_read_regular_file_snapshot(path, role).map(|snapshot| snapshot.raw)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SecureFileSnapshot {
    identity: FileIdentity,
    raw: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SecureFileDigest {
    identity: FileIdentity,
    size: u64,
    sha256: [u8; 32],
}

impl SecureFileDigest {
    fn capture(path: &Path, role: &str) -> Result<Self, String> {
        let snapshot = secure_read_regular_file_snapshot(path, role)?;
        let size = snapshot.raw.len() as u64;
        let sha256 = Sha256::digest(&snapshot.raw).into();
        Ok(Self {
            identity: snapshot.identity,
            size,
            sha256,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformAttestation {
    executable_path: PathBuf,
    executable: SecureFileDigest,
    probe_path: PathBuf,
    probe: SecureFileDigest,
    install_path: PathBuf,
    install_inventory: TreeSnapshot,
    trust: PlatformTrustAttestation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlatformTrustPolicy {
    Immutable,
    #[cfg(any(test, feature = "test-support"))]
    TestFixture,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PlatformTrustAttestation {
    Immutable(ImmutablePlatformTrustSnapshot),
    #[cfg(any(test, feature = "test-support"))]
    TestFixture,
}

impl PlatformTrustAttestation {
    fn policy(&self) -> PlatformTrustPolicy {
        match self {
            Self::Immutable(_) => PlatformTrustPolicy::Immutable,
            #[cfg(any(test, feature = "test-support"))]
            Self::TestFixture => PlatformTrustPolicy::TestFixture,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImmutablePlatformTrustSnapshot {
    entries: Vec<ImmutablePlatformEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImmutablePlatformEntryKind {
    Directory,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImmutablePlatformEntry {
    path: PathBuf,
    kind: ImmutablePlatformEntryKind,
    identity: FileIdentity,
    #[cfg(unix)]
    owner: u32,
    #[cfg(unix)]
    mode: u32,
    #[cfg(windows)]
    security_descriptor_sha256: [u8; 32],
}

impl PlatformAttestation {
    pub fn capture_immutable(executable: &Path) -> Result<Self, String> {
        Self::capture_with_policy(executable, PlatformTrustPolicy::Immutable)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn capture_test_fixture(executable: &Path) -> Result<Self, String> {
        Self::capture_with_policy(executable, PlatformTrustPolicy::TestFixture)
    }

    fn capture_with_policy(
        executable: &Path,
        trust_policy: PlatformTrustPolicy,
    ) -> Result<Self, String> {
        let executable_path = normalize_leaf_path(executable)?;
        let install_path = executable_path
            .parent()
            .ok_or_else(|| {
                format!(
                    "platform executable has no installation directory: {}",
                    executable_path.display()
                )
            })?
            .to_path_buf();
        let probe_name = if cfg!(windows) { "ibcmd.exe" } else { "ibcmd" };
        let probe_path = if executable_path.file_name() == Some(OsStr::new(probe_name)) {
            executable_path.clone()
        } else {
            install_path.join(probe_name)
        };
        let trust_before = match trust_policy {
            PlatformTrustPolicy::Immutable => {
                PlatformTrustAttestation::Immutable(ImmutablePlatformTrustSnapshot::capture(
                    &install_path,
                    &executable_path,
                    &probe_path,
                )?)
            }
            #[cfg(any(test, feature = "test-support"))]
            PlatformTrustPolicy::TestFixture => PlatformTrustAttestation::TestFixture,
        };
        let executable_digest =
            SecureFileDigest::capture(&executable_path, "platform executable attestation")?;
        let probe_digest =
            SecureFileDigest::capture(&probe_path, "platform version probe attestation")?;
        let install_inventory = TreeSnapshot::capture_target(&install_path)?;
        if !matches!(install_inventory, TreeSnapshot::Directory { .. }) {
            return Err(format!(
                "platform installation disappeared during attestation: {}",
                install_path.display()
            ));
        }
        let trust = match trust_policy {
            PlatformTrustPolicy::Immutable => {
                let after =
                    PlatformTrustAttestation::Immutable(ImmutablePlatformTrustSnapshot::capture(
                        &install_path,
                        &executable_path,
                        &probe_path,
                    )?);
                if after != trust_before {
                    return Err(format!(
                        "immutable platform trust metadata changed during attestation: {}",
                        install_path.display()
                    ));
                }
                after
            }
            #[cfg(any(test, feature = "test-support"))]
            PlatformTrustPolicy::TestFixture => PlatformTrustAttestation::TestFixture,
        };
        Ok(Self {
            executable_path,
            executable: executable_digest,
            probe_path,
            probe: probe_digest,
            install_path,
            install_inventory,
            trust,
        })
    }

    pub fn recheck(&self) -> Result<(), String> {
        let current = Self::capture_with_policy(&self.executable_path, self.trust.policy())?;
        if &current == self {
            Ok(())
        } else {
            Err(format!(
                "platform attestation changed during full dump; staged output was not published: {}",
                self.install_path.display()
            ))
        }
    }
}

impl ImmutablePlatformTrustSnapshot {
    #[cfg(unix)]
    fn capture(install: &Path, executable: &Path, probe: &Path) -> Result<Self, String> {
        verify_unprivileged_platform_caller()?;
        let mut entries = capture_immutable_platform_ancestry(install)?;
        let install_directory = open_directory_nofollow(install).map_err(|error| {
            format!(
                "failed to securely open immutable platform installation {}: {error}",
                install.display()
            )
        })?;
        capture_immutable_platform_children(&install_directory, install, &mut entries)?;
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        entries.dedup();
        for (path, role) in [
            (executable, "platform executable"),
            (probe, "platform version probe"),
        ] {
            let Some(entry) = entries.iter().find(|entry| entry.path == path) else {
                return Err(format!(
                    "{role} is outside the immutable platform inventory: {}",
                    path.display()
                ));
            };
            if entry.kind != ImmutablePlatformEntryKind::File || entry.mode & 0o111 == 0 {
                return Err(format!(
                    "{role} is not an executable immutable regular file: {}",
                    path.display()
                ));
            }
        }
        Ok(Self { entries })
    }

    #[cfg(windows)]
    fn capture(install: &Path, executable: &Path, probe: &Path) -> Result<Self, String> {
        verify_unprivileged_windows_platform_caller().map_err(|error| {
            format!("failed to prove an unprivileged Windows platform caller: {error}")
        })?;
        let (mut entries, install_directory) =
            capture_windows_immutable_platform_ancestry(install)?;
        capture_windows_immutable_platform_children(&install_directory, install, &mut entries)?;
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        entries.dedup();
        for (path, role) in [
            (executable, "platform executable"),
            (probe, "platform version probe"),
        ] {
            let Some(entry) = entries.iter().find(|entry| entry.path == path) else {
                return Err(format!(
                    "{role} is outside the immutable platform inventory: {}",
                    path.display()
                ));
            };
            if entry.kind != ImmutablePlatformEntryKind::File {
                return Err(format!(
                    "{role} is not an immutable regular file: {}",
                    path.display()
                ));
            }
        }
        Ok(Self { entries })
    }

    #[cfg(not(any(unix, windows)))]
    fn capture(_install: &Path, _executable: &Path, _probe: &Path) -> Result<Self, String> {
        Err("immutable platform trust verification is unavailable on this host".to_string())
    }
}

#[cfg(windows)]
fn parse_windows_local_platform_path(install: &Path) -> Result<(PathBuf, Vec<OsString>), String> {
    use std::path::{Component, Prefix};

    if !install.is_absolute() {
        return Err(format!(
            "immutable platform installation path must be absolute: {}",
            install.display()
        ));
    }
    let mut components = install.components();
    let prefix = match components.next() {
        Some(Component::Prefix(prefix)) => prefix,
        _ => {
            return Err(format!(
                "Windows immutable platform path has no volume prefix: {}",
                install.display()
            ))
        }
    };
    if !matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)) {
        return Err(format!(
            "Windows immutable platform path must use a local disk prefix: {}",
            install.display()
        ));
    }
    if !matches!(components.next(), Some(Component::RootDir)) {
        return Err(format!(
            "Windows immutable platform path has no volume root: {}",
            install.display()
        ));
    }
    let mut names = Vec::new();
    for component in components {
        match component {
            Component::Normal(name) => names.push(name.to_os_string()),
            Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) | Component::RootDir => {
                return Err(format!(
                    "immutable platform installation path contains a non-normal component: {}",
                    install.display()
                ))
            }
        }
    }

    let mut root_path = PathBuf::from(prefix.as_os_str());
    root_path.push(Path::new(r"\"));
    Ok((root_path, names))
}

#[cfg(windows)]
fn capture_windows_immutable_platform_ancestry(
    install: &Path,
) -> Result<(Vec<ImmutablePlatformEntry>, File), String> {
    let (root_path, names) = parse_windows_local_platform_path(install)?;
    let root = open_directory_nofollow(&root_path).map_err(|error| {
        format!(
            "failed to securely open immutable platform volume root {}: {error}",
            root_path.display()
        )
    })?;
    verify_windows_local_fixed_volume(&root).map_err(|error| {
        format!(
            "failed to prove a local fixed immutable platform volume from retained handle {}: {error}",
            root_path.display()
        )
    })?;
    let root_profile = if names.is_empty() {
        WindowsImmutableAclProfile::Installation
    } else {
        WindowsImmutableAclProfile::Ancestry
    };
    let mut entries = vec![capture_windows_immutable_platform_entry(
        &root,
        &root_path,
        ImmutablePlatformEntryKind::Directory,
        root_profile,
    )?];
    let mut current = root;
    let mut current_path = root_path;
    let final_index = names.len().saturating_sub(1);
    for (index, name) in names.into_iter().enumerate() {
        current = open_directory_child_nofollow(&current, &name).map_err(|error| {
            format!(
                "platform ancestry contains a reparse point, non-directory, or inaccessible component {}: {error}",
                current_path.join(&name).display()
            )
        })?;
        current_path.push(&name);
        let profile = if index == final_index {
            WindowsImmutableAclProfile::Installation
        } else {
            WindowsImmutableAclProfile::Ancestry
        };
        entries.push(capture_windows_immutable_platform_entry(
            &current,
            &current_path,
            ImmutablePlatformEntryKind::Directory,
            profile,
        )?);
    }
    verify_windows_local_fixed_volume(&current).map_err(|error| {
        format!(
            "failed to prove a local fixed immutable platform installation from retained handle {}: {error}",
            current_path.display()
        )
    })?;
    Ok((entries, current))
}

#[cfg(windows)]
fn capture_windows_immutable_platform_children(
    directory: &File,
    display_path: &Path,
    entries: &mut Vec<ImmutablePlatformEntry>,
) -> Result<(), String> {
    let initial_names =
        crate::platform::filesystem::read_directory_names(directory).map_err(|error| {
            format!(
                "failed to enumerate immutable platform installation {}: {error}",
                display_path.display()
            )
        })?;
    for name in &initial_names {
        let child_path = display_path.join(name);
        match open_directory_child_nofollow(directory, name) {
            Ok(child) => {
                let entry = capture_windows_immutable_platform_entry(
                    &child,
                    &child_path,
                    ImmutablePlatformEntryKind::Directory,
                    WindowsImmutableAclProfile::Installation,
                )?;
                let expected_identity = entry.identity;
                entries.push(entry);
                capture_windows_immutable_platform_children(&child, &child_path, entries)?;
                let rebound = open_directory_child_nofollow(directory, name).map_err(|error| {
                    format!(
                        "failed to rebind immutable platform directory {}: {error}",
                        child_path.display()
                    )
                })?;
                if file_identity(&rebound).map_err(|error| {
                    format!(
                        "failed to recheck immutable platform directory {}: {error}",
                        child_path.display()
                    )
                })? != expected_identity
                {
                    return Err(format!(
                        "immutable platform directory identity changed while inspecting: {}",
                        child_path.display()
                    ));
                }
            }
            Err(directory_error) => {
                let file = open_regular_child_nofollow(directory, name).map_err(|file_error| {
                    format!(
                        "immutable platform inventory contains an unsupported or reparse entry {}: directory open failed ({directory_error}); regular-file open failed ({file_error})",
                        child_path.display()
                    )
                })?;
                if hard_link_count(&file).map_err(|error| {
                    format!(
                        "failed to inspect hard links for immutable platform file {}: {error}",
                        child_path.display()
                    )
                })? != 1
                {
                    return Err(format!(
                        "immutable platform file must have exactly one hard link: {}",
                        child_path.display()
                    ));
                }
                let entry = capture_windows_immutable_platform_entry(
                    &file,
                    &child_path,
                    ImmutablePlatformEntryKind::File,
                    WindowsImmutableAclProfile::Installation,
                )?;
                let expected_identity = entry.identity;
                entries.push(entry);
                let rebound = open_regular_child_nofollow(directory, name).map_err(|error| {
                    format!(
                        "failed to rebind immutable platform file {}: {error}",
                        child_path.display()
                    )
                })?;
                if file_identity(&rebound).map_err(|error| {
                    format!(
                        "failed to recheck immutable platform file {}: {error}",
                        child_path.display()
                    )
                })? != expected_identity
                {
                    return Err(format!(
                        "immutable platform file identity changed while inspecting: {}",
                        child_path.display()
                    ));
                }
            }
        }
    }
    let final_names =
        crate::platform::filesystem::read_directory_names(directory).map_err(|error| {
            format!(
                "failed to re-enumerate immutable platform installation {}: {error}",
                display_path.display()
            )
        })?;
    if final_names != initial_names {
        return Err(format!(
            "immutable platform directory name set changed while inspecting: {}",
            display_path.display()
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn capture_windows_immutable_platform_entry(
    file: &File,
    path: &Path,
    kind: ImmutablePlatformEntryKind,
    profile: WindowsImmutableAclProfile,
) -> Result<ImmutablePlatformEntry, String> {
    let evidence = capture_windows_immutable_entry_evidence(file).map_err(|error| {
        format!(
            "failed to capture immutable Windows platform evidence {}: {error}",
            path.display()
        )
    })?;
    evidence.verify(profile).map_err(|error| {
        format!(
            "immutable Windows platform ACL is not trusted {}: {error}",
            path.display()
        )
    })?;
    Ok(ImmutablePlatformEntry {
        path: path.to_path_buf(),
        kind,
        identity: evidence.identity,
        security_descriptor_sha256: evidence.security_descriptor_sha256,
    })
}

#[cfg(unix)]
fn verify_unprivileged_platform_caller() -> Result<(), String> {
    // SAFETY: geteuid has no preconditions and does not mutate process state.
    if unsafe { libc::geteuid() } == 0 {
        return Err(
            "immutable platform execution is refused for an effective root caller".to_string(),
        );
    }
    #[cfg(target_os = "linux")]
    {
        let status = fs::read_to_string("/proc/self/status").map_err(|error| {
            format!("failed to verify Linux effective capabilities from /proc/self/status: {error}")
        })?;
        let capabilities = status
            .lines()
            .find_map(|line| line.strip_prefix("CapEff:"))
            .map(str::trim)
            .ok_or_else(|| {
                "Linux effective capabilities are absent from /proc/self/status".to_string()
            })?;
        if capabilities.is_empty()
            || !capabilities
                .bytes()
                .all(|byte| byte == b'0' || byte.is_ascii_hexdigit())
        {
            return Err("Linux effective capabilities have an invalid encoding".to_string());
        }
        if capabilities.bytes().any(|byte| byte != b'0') {
            return Err(
                "immutable platform execution is refused for a caller with effective Linux capabilities"
                    .to_string(),
            );
        }
    }
    Ok(())
}

#[cfg(unix)]
fn capture_immutable_platform_ancestry(
    install: &Path,
) -> Result<Vec<ImmutablePlatformEntry>, String> {
    use std::path::Component;

    if !install.is_absolute() {
        return Err(format!(
            "immutable platform installation path must be absolute: {}",
            install.display()
        ));
    }
    let root_path = PathBuf::from("/");
    let root = open_directory_nofollow(&root_path)
        .map_err(|error| format!("failed to securely open platform ancestry root: {error}"))?;
    let mut entries = vec![capture_immutable_platform_entry(
        &root,
        &root_path,
        ImmutablePlatformEntryKind::Directory,
    )?];
    let mut current = root;
    let mut current_path = root_path;
    for component in install.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => {
                current = open_directory_child_nofollow(&current, name).map_err(|error| {
                    format!(
                        "platform ancestry contains a link, non-directory, or inaccessible component {}: {error}",
                        current_path.join(name).display()
                    )
                })?;
                current_path.push(name);
                entries.push(capture_immutable_platform_entry(
                    &current,
                    &current_path,
                    ImmutablePlatformEntryKind::Directory,
                )?);
            }
            Component::ParentDir | Component::Prefix(_) => {
                return Err(format!(
                    "immutable platform installation path contains a non-normal component: {}",
                    install.display()
                ));
            }
        }
    }
    Ok(entries)
}

#[cfg(unix)]
fn capture_immutable_platform_children(
    directory: &File,
    display_path: &Path,
    entries: &mut Vec<ImmutablePlatformEntry>,
) -> Result<(), String> {
    for name in read_directory_names(directory).map_err(|error| {
        format!(
            "failed to enumerate immutable platform installation {}: {error}",
            display_path.display()
        )
    })? {
        let child_path = display_path.join(&name);
        match open_directory_child_nofollow(directory, &name) {
            Ok(child) => {
                let entry = capture_immutable_platform_entry(
                    &child,
                    &child_path,
                    ImmutablePlatformEntryKind::Directory,
                )?;
                let expected_identity = entry.identity;
                entries.push(entry);
                capture_immutable_platform_children(&child, &child_path, entries)?;
                let rebound = open_directory_child_nofollow(directory, &name).map_err(|error| {
                    format!(
                        "failed to rebind immutable platform directory {}: {error}",
                        child_path.display()
                    )
                })?;
                if file_identity(&rebound).map_err(|error| {
                    format!(
                        "failed to recheck immutable platform directory {}: {error}",
                        child_path.display()
                    )
                })? != expected_identity
                {
                    return Err(format!(
                        "immutable platform directory identity changed while inspecting: {}",
                        child_path.display()
                    ));
                }
            }
            Err(error)
                if matches!(
                    error.raw_os_error(),
                    Some(libc::ENOTDIR) | Some(libc::EINVAL)
                ) =>
            {
                let file = open_regular_child_nofollow(directory, &name).map_err(|file_error| {
                    let detail = if file_error.raw_os_error() == Some(libc::ELOOP) {
                        "symbolic link".to_string()
                    } else {
                        file_error.to_string()
                    };
                    format!(
                        "immutable platform inventory contains an unsupported entry {}: {detail}",
                        child_path.display()
                    )
                })?;
                if hard_link_count(&file).map_err(|link_error| {
                    format!(
                        "failed to inspect hard links for immutable platform file {}: {link_error}",
                        child_path.display()
                    )
                })? != 1
                {
                    return Err(format!(
                        "immutable platform file must have exactly one hard link: {}",
                        child_path.display()
                    ));
                }
                entries.push(capture_immutable_platform_entry(
                    &file,
                    &child_path,
                    ImmutablePlatformEntryKind::File,
                )?);
            }
            Err(error) => {
                let detail = if error.raw_os_error() == Some(libc::ELOOP) {
                    "symbolic link".to_string()
                } else {
                    error.to_string()
                };
                return Err(format!(
                    "immutable platform inventory contains an unsupported entry {}: {detail}",
                    child_path.display()
                ));
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn capture_immutable_platform_entry(
    file: &File,
    path: &Path,
    kind: ImmutablePlatformEntryKind,
) -> Result<ImmutablePlatformEntry, String> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file.metadata().map_err(|error| {
        format!(
            "failed to inspect immutable platform entry {}: {error}",
            path.display()
        )
    })?;
    if metadata.uid() != 0 {
        return Err(format!(
            "platform installation must be owned by root; user-owned entry refused: {}",
            path.display()
        ));
    }
    let mode = metadata.mode() & 0o7777;
    if mode & 0o022 != 0 {
        return Err(format!(
            "platform installation entry is group/world writable and not immutable: {}",
            path.display()
        ));
    }
    if kind == ImmutablePlatformEntryKind::File && mode & 0o6000 != 0 {
        return Err(format!(
            "setuid/setgid platform file is not accepted: {}",
            path.display()
        ));
    }
    verify_platform_entry_has_no_acl(file, path)?;
    Ok(ImmutablePlatformEntry {
        path: path.to_path_buf(),
        kind,
        identity: file_identity(file).map_err(|error| {
            format!(
                "failed to capture immutable platform identity {}: {error}",
                path.display()
            )
        })?,
        owner: metadata.uid(),
        mode,
    })
}

#[cfg(target_os = "macos")]
fn verify_platform_entry_has_no_acl(file: &File, path: &Path) -> Result<(), String> {
    type DarwinAcl = *mut libc::c_void;
    const ACL_TYPE_EXTENDED: libc::c_int = 0x0000_0100;
    unsafe extern "C" {
        fn acl_get_fd_np(fd: libc::c_int, acl_type: libc::c_int) -> DarwinAcl;
        fn acl_free(object: *mut libc::c_void) -> libc::c_int;
    }

    // SAFETY: file owns a valid descriptor; acl_get_fd_np returns an owned ACL
    // object that is released below when present.
    let acl = unsafe { acl_get_fd_np(file.as_raw_fd(), ACL_TYPE_EXTENDED) };
    if !acl.is_null() {
        // SAFETY: acl is a live allocation returned by acl_get_fd_np.
        let release_status = unsafe { acl_free(acl) };
        if release_status != 0 {
            return Err(format!(
                "failed to release ACL metadata for immutable platform entry {}: {}",
                path.display(),
                std::io::Error::last_os_error()
            ));
        }
        return Err(format!(
            "platform installation entry has an extended ACL and is not immutable: {}",
            path.display()
        ));
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == ErrorKind::NotFound {
        Ok(())
    } else {
        Err(format!(
            "could not prove that platform entry has no extended ACL {}: {error}",
            path.display()
        ))
    }
}

#[cfg(target_os = "linux")]
fn verify_platform_entry_has_no_acl(file: &File, path: &Path) -> Result<(), String> {
    for name in [
        &b"system.posix_acl_access\0"[..],
        &b"system.posix_acl_default\0"[..],
    ] {
        // SAFETY: file owns a valid descriptor, name is NUL-terminated, and a
        // null value with zero size performs a size probe only.
        let status = unsafe {
            libc::fgetxattr(
                file.as_raw_fd(),
                name.as_ptr().cast(),
                std::ptr::null_mut(),
                0,
            )
        };
        if status >= 0 {
            return Err(format!(
                "platform installation entry has a POSIX ACL and is not immutable: {}",
                path.display()
            ));
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ENODATA) {
            return Err(format!(
                "could not prove that platform entry has no POSIX ACL {}: {error}",
                path.display()
            ));
        }
    }
    Ok(())
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn verify_platform_entry_has_no_acl(_file: &File, path: &Path) -> Result<(), String> {
    Err(format!(
        "immutable platform ACL verification is unavailable on this Unix host: {}",
        path.display()
    ))
}

#[cfg(unix)]
fn secure_read_regular_file_snapshot(
    path: &Path,
    role: &str,
) -> Result<SecureFileSnapshot, String> {
    let normalized = normalize_leaf_path(path)?;
    let (parent_path, name) = split_parent_and_name(&normalized)?;
    let parent = open_directory_nofollow(&parent_path).map_err(|error| {
        format!(
            "failed to securely open parent for {role} {}: {error}",
            normalized.display()
        )
    })?;
    run_secure_read_hook(path);
    let mut file = open_regular_child_nofollow(&parent, &name).map_err(|error| {
        let detail = if error.raw_os_error() == Some(libc::ELOOP) {
            "symbolic link or reparse point".to_string()
        } else {
            error.to_string()
        };
        format!(
            "failed to securely open {role} {}: {detail}",
            path.display()
        )
    })?;
    let identity = file_identity(&file).map_err(|error| {
        format!(
            "failed to inspect opened {role} identity {}: {error}",
            path.display()
        )
    })?;
    if hard_link_count(&file).map_err(|error| {
        format!(
            "failed to inspect hard links for {role} {}: {error}",
            path.display()
        )
    })? != 1
    {
        return Err(format!(
            "{role} must have exactly one hard link: {}",
            path.display()
        ));
    }
    let opened = file.metadata().map_err(|error| {
        format!(
            "failed to inspect opened {role} {}: {error}",
            path.display()
        )
    })?;
    let mut raw = Vec::new();
    file.read_to_end(&mut raw)
        .map_err(|error| format!("failed to read {role} {}: {error}", path.display()))?;
    let after = file.metadata().map_err(|error| {
        format!(
            "failed to recheck opened {role} {}: {error}",
            path.display()
        )
    })?;
    if opened.len() != after.len() || opened.modified().ok() != after.modified().ok() {
        return Err(format!("{role} changed while reading: {}", path.display()));
    }
    let rebound = open_regular_child_nofollow(&parent, &name).map_err(|error| {
        format!(
            "failed to rebind {role} name after reading {}: {error}",
            path.display()
        )
    })?;
    let rebound_identity = file_identity(&rebound).map_err(|error| {
        format!(
            "failed to recheck {role} identity {}: {error}",
            path.display()
        )
    })?;
    if rebound_identity != identity {
        return Err(format!(
            "{role} identity changed while reading: {}",
            path.display()
        ));
    }
    run_secure_read_after_hook(path);
    Ok(SecureFileSnapshot { identity, raw })
}

#[cfg(windows)]
fn secure_read_regular_file_snapshot(
    path: &Path,
    role: &str,
) -> Result<SecureFileSnapshot, String> {
    let normalized = normalize_leaf_path(path)?;
    let (parent_path, name) = split_parent_and_name(&normalized)?;
    let parent = open_directory_nofollow(&parent_path).map_err(|error| {
        format!(
            "failed to securely open parent for {role} {}: {error}",
            normalized.display()
        )
    })?;
    let mut file = open_regular_child_nofollow(&parent, &name)
        .map_err(|error| format!("failed to securely open {role} {}: {error}", path.display()))?;
    let identity = file_identity(&file).map_err(|error| {
        format!(
            "failed to inspect opened {role} identity {}: {error}",
            path.display()
        )
    })?;
    if hard_link_count(&file).map_err(|error| {
        format!(
            "failed to inspect hard links for {role} {}: {error}",
            path.display()
        )
    })? != 1
    {
        return Err(format!(
            "{role} must have exactly one hard link: {}",
            path.display()
        ));
    }
    let opened = file.metadata().map_err(|error| {
        format!(
            "failed to inspect opened {role} {}: {error}",
            path.display()
        )
    })?;

    run_secure_read_hook(path);
    let rebound = open_regular_child_nofollow(&parent, &name).map_err(|error| {
        format!(
            "failed to rebind {role} name before reading {}: {error}",
            path.display()
        )
    })?;
    if file_identity(&rebound).map_err(|error| {
        format!(
            "failed to recheck {role} identity {}: {error}",
            path.display()
        )
    })? != identity
    {
        return Err(format!(
            "{role} identity changed before reading: {}",
            path.display()
        ));
    }

    let mut raw = Vec::new();
    file.read_to_end(&mut raw)
        .map_err(|error| format!("failed to read {role} {}: {error}", path.display()))?;
    run_secure_read_after_hook(path);
    let after = file.metadata().map_err(|error| {
        format!(
            "failed to recheck opened {role} {}: {error}",
            path.display()
        )
    })?;
    if opened.len() != after.len() || opened.modified().ok() != after.modified().ok() {
        return Err(format!("{role} changed while reading: {}", path.display()));
    }
    let rebound = open_regular_child_nofollow(&parent, &name).map_err(|error| {
        format!(
            "failed to rebind {role} name after reading {}: {error}",
            path.display()
        )
    })?;
    if file_identity(&rebound).map_err(|error| {
        format!(
            "failed to recheck {role} identity {}: {error}",
            path.display()
        )
    })? != identity
    {
        return Err(format!(
            "{role} identity changed while reading: {}",
            path.display()
        ));
    }
    Ok(SecureFileSnapshot { identity, raw })
}

#[cfg(not(any(unix, windows)))]
fn secure_read_regular_file_snapshot(
    path: &Path,
    role: &str,
) -> Result<SecureFileSnapshot, String> {
    Err(format!(
        "{role} secure no-follow reads are unavailable on this host: {}",
        path.display()
    ))
}

fn split_parent_and_name(path: &Path) -> Result<(PathBuf, OsString), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("path has no parent: {}", path.display()))?;
    let name = path
        .file_name()
        .ok_or_else(|| format!("path has no final component: {}", path.display()))?;
    Ok((parent.to_path_buf(), name.to_os_string()))
}

#[cfg(unix)]
fn secure_path_is_absent(path: &Path) -> Result<bool, String> {
    let normalized = normalize_leaf_path(path)?;
    let (parent_path, name) = split_parent_and_name(&normalized)?;
    let parent = open_directory_nofollow(&parent_path).map_err(|error| {
        format!(
            "failed to securely open parent while checking {}: {error}",
            normalized.display()
        )
    })?;
    let name = CString::new(name.as_bytes()).map_err(|error| {
        format!(
            "path contains an embedded NUL byte {}: {error}",
            normalized.display()
        )
    })?;
    // SAFETY: parent remains open for the call, name is NUL-terminated, and
    // metadata points to writable storage for the duration of fstatat.
    let mut metadata = unsafe { std::mem::zeroed::<libc::stat>() };
    let status = unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            name.as_ptr(),
            &mut metadata,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if status == 0 {
        return Ok(false);
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == ErrorKind::NotFound {
        Ok(true)
    } else {
        Err(format!(
            "failed to securely inspect {}: {error}",
            normalized.display()
        ))
    }
}

#[cfg(windows)]
fn secure_path_is_absent(path: &Path) -> Result<bool, String> {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND};

    let normalized = normalize_leaf_path(path)?;
    let (parent_path, name) = split_parent_and_name(&normalized)?;
    let parent = open_directory_nofollow(&parent_path).map_err(|error| {
        format!(
            "failed to securely open parent while checking {}: {error}",
            normalized.display()
        )
    })?;
    match open_any_child_nofollow(&parent, &name) {
        Ok((_child, OpenedChildKind::ReparsePoint)) => Ok(false),
        Ok((_child, _kind)) => Ok(false),
        Err(error)
            if matches!(
                error.raw_os_error(),
                Some(code)
                    if code == ERROR_FILE_NOT_FOUND as i32
                        || code == ERROR_PATH_NOT_FOUND as i32
            ) =>
        {
            Ok(true)
        }
        Err(error) => Err(format!(
            "failed to securely inspect {}: {error}",
            normalized.display()
        )),
    }
}

#[cfg(not(any(unix, windows)))]
fn secure_path_is_absent(path: &Path) -> Result<bool, String> {
    Err(format!(
        "secure no-follow absence checks are unavailable on this host: {}",
        path.display()
    ))
}

#[cfg(unix)]
fn open_directory_nofollow(path: &Path) -> std::io::Result<File> {
    use std::path::Component;

    if !path.is_absolute() {
        return Err(std::io::Error::new(
            ErrorKind::InvalidInput,
            "secure directory path must be absolute",
        ));
    }
    let root = CString::new("/")?;
    // SAFETY: the static root path is NUL-terminated and the returned descriptor is owned below.
    let root_fd = unsafe {
        libc::open(
            root.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if root_fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: root_fd is a newly owned descriptor from open.
    let mut current = unsafe { File::from_raw_fd(root_fd) };
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                current = open_directory_child_nofollow(&current, name)?;
            }
            Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) => {
                return Err(std::io::Error::new(
                    ErrorKind::InvalidInput,
                    "secure directory path contains a non-normal component",
                ));
            }
        }
    }
    Ok(current)
}

#[cfg(any(unix, windows))]
fn open_directory_relative_nofollow(root: &File, relative: &Path) -> std::io::Result<File> {
    use std::path::Component;

    if relative.is_absolute() {
        return Err(std::io::Error::new(
            ErrorKind::InvalidInput,
            "secure descendant path must be relative",
        ));
    }
    let mut current = root.try_clone()?;
    for component in relative.components() {
        match component {
            Component::Normal(name) => {
                current = open_directory_child_nofollow(&current, name)?;
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(std::io::Error::new(
                    ErrorKind::InvalidInput,
                    "secure descendant path contains a non-normal component",
                ));
            }
        }
    }
    Ok(current)
}

#[cfg(unix)]
fn open_directory_child_nofollow(parent: &File, name: &OsStr) -> std::io::Result<File> {
    let name = CString::new(name.as_bytes())?;
    // SAFETY: parent remains open for the call and name is a live NUL-terminated string.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        // SAFETY: fd is a newly owned descriptor from openat.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

#[cfg(unix)]
fn open_regular_child_nofollow(parent: &File, name: &OsStr) -> std::io::Result<File> {
    let name = CString::new(name.as_bytes())?;
    // SAFETY: parent remains open for the call and name is a live NUL-terminated string.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: fd is a newly owned descriptor from openat.
    let file = unsafe { File::from_raw_fd(fd) };
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "entry is not a regular file",
        ));
    }
    Ok(file)
}

#[cfg(unix)]
fn create_regular_child_owner_only(
    parent: &File,
    name: &OsStr,
    _display_path: &Path,
) -> std::io::Result<File> {
    crate::platform::filesystem::create_owner_only_file_child(parent, name)
}

#[cfg(windows)]
fn create_regular_child_owner_only(
    parent: &File,
    name: &OsStr,
    display_path: &Path,
) -> std::io::Result<File> {
    let created = create_owner_only_file_child(parent, name)?;
    let cleanup_error = |primary: std::io::Error| {
        match discard_created_child(&created) {
        Ok(()) => primary,
        Err(cleanup) => std::io::Error::new(
            primary.kind(),
            format!(
                "{primary}; failed to remove unverified effective config through its created handle: {cleanup}"
            ),
        ),
    }
    };
    let expected_identity = file_identity(&created).map_err(cleanup_error)?;
    run_effective_config_create_hook(display_path);
    verify_owner_only_acl(&created).map_err(cleanup_error)?;
    let rebound = open_regular_child_nofollow(parent, name).map_err(cleanup_error)?;
    let rebound_identity = file_identity(&rebound).map_err(cleanup_error)?;
    if rebound_identity != expected_identity {
        return Err(cleanup_error(std::io::Error::new(
            ErrorKind::InvalidData,
            "private effective config identity changed before verified reopen",
        )));
    }
    Ok(created)
}

#[cfg(not(any(unix, windows)))]
fn create_regular_child_owner_only(
    _parent: &File,
    _name: &OsStr,
    _display_path: &Path,
) -> std::io::Result<File> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "secure openat config creation is unavailable on this host",
    ))
}

#[cfg(unix)]
fn unlink_child_at(parent: &File, name: &OsStr, flags: libc::c_int) -> std::io::Result<()> {
    let name = CString::new(name.as_bytes())?;
    // SAFETY: parent remains open and name is a live NUL-terminated string.
    let status = unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if status == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn unlink_directory_child_at(parent: &File, name: &OsStr) -> std::io::Result<()> {
    unlink_child_at(parent, name, libc::AT_REMOVEDIR)
}

#[cfg(not(unix))]
fn unlink_directory_child_at(_parent: &File, _name: &OsStr) -> std::io::Result<()> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "secure unlinkat directory cleanup is unavailable on this host",
    ))
}

#[cfg(unix)]
fn unlink_non_directory_child_at(parent: &File, name: &OsStr) -> std::io::Result<()> {
    unlink_child_at(parent, name, 0)
}

#[cfg(not(unix))]
fn unlink_non_directory_child_at(_parent: &File, _name: &OsStr) -> std::io::Result<()> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "secure unlinkat file cleanup is unavailable on this host",
    ))
}

/// Retains the existing prefix without creating user source directories.
fn capture_existing_target_parent(
    workspace: &DirectoryAnchor,
    relative: &Path,
) -> Result<(DirectoryAnchor, Vec<OsString>), String> {
    let components = relative
        .components()
        .map(|component| match component {
            std::path::Component::Normal(name) => Ok(name.to_os_string()),
            _ => Err("dump target parent has a non-normal component".to_string()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut current = workspace.try_clone()?;
    for (index, name) in components.iter().enumerate() {
        match open_directory_child_nofollow(&current.directory, name) {
            Ok(directory) => {
                let path = current.path.join(name);
                let identity = file_identity(&directory).map_err(|error| error.to_string())?;
                current = DirectoryAnchor {
                    directory,
                    identity,
                    path,
                };
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                current.verify_path_binding()?;
                return Ok((current, components[index..].to_vec()));
            }
            Err(error) => {
                return Err(format!(
                    "failed to retain dump target parent without following links: {error}"
                ))
            }
        }
    }
    current.verify_path_binding()?;
    Ok((current, Vec::new()))
}

#[derive(Debug)]
struct DirectoryAnchor {
    path: PathBuf,
    identity: FileIdentity,
    directory: File,
}

impl DirectoryAnchor {
    #[cfg(any(unix, windows))]
    fn capture_exact(path: &Path) -> Result<Self, String> {
        if !path.is_absolute() {
            return Err(format!(
                "directory anchor path must already be absolute: {}",
                path.display()
            ));
        }
        let path = path.to_path_buf();
        let directory = open_directory_nofollow(&path).map_err(|error| {
            format!(
                "failed to securely open directory anchor {}: {error}",
                path.display()
            )
        })?;
        let identity = file_identity(&directory).map_err(|error| {
            format!(
                "failed to inspect directory anchor {}: {error}",
                path.display()
            )
        })?;
        Ok(Self {
            path,
            identity,
            directory,
        })
    }

    #[cfg(any(unix, windows))]
    fn capture_descendant(&self, relative: &Path, display_path: &Path) -> Result<Self, String> {
        self.verify_path_binding()?;
        let directory =
            open_directory_relative_nofollow(&self.directory, relative).map_err(|error| {
                format!(
                    "failed to bind dump target parent below retained workspace anchor {}: {error}",
                    display_path.display()
                )
            })?;
        let identity = file_identity(&directory).map_err(|error| {
            format!(
                "failed to inspect workspace-contained directory {}: {error}",
                display_path.display()
            )
        })?;
        self.verify_descendant_identity(relative, identity, display_path)?;
        Ok(Self {
            path: display_path.to_path_buf(),
            identity,
            directory,
        })
    }

    #[cfg(any(unix, windows))]
    fn verify_descendant_identity(
        &self,
        relative: &Path,
        expected_identity: FileIdentity,
        display_path: &Path,
    ) -> Result<(), String> {
        self.verify_path_binding()?;
        let rebound =
            open_directory_relative_nofollow(&self.directory, relative).map_err(|error| {
                format!(
                    "workspace-contained directory could not be rebound without following links {}: {error}",
                    display_path.display()
                )
            })?;
        let actual_identity = file_identity(&rebound).map_err(|error| {
            format!(
                "failed to recheck workspace-contained directory {}: {error}",
                display_path.display()
            )
        })?;
        if actual_identity != expected_identity {
            return Err(format!(
                "workspace-contained directory identity changed during full dump: {}",
                display_path.display()
            ));
        }
        self.verify_path_binding()
    }

    #[cfg(not(any(unix, windows)))]
    fn capture_descendant(&self, _relative: &Path, display_path: &Path) -> Result<Self, String> {
        Err(format!(
            "secure workspace-relative directory anchors are unavailable on this host: {}",
            display_path.display()
        ))
    }

    #[cfg(not(any(unix, windows)))]
    fn verify_descendant_identity(
        &self,
        _relative: &Path,
        _expected_identity: FileIdentity,
        display_path: &Path,
    ) -> Result<(), String> {
        Err(format!(
            "secure workspace-relative containment checks are unavailable on this host: {}",
            display_path.display()
        ))
    }

    #[cfg(any(unix, windows))]
    fn try_clone(&self) -> Result<Self, String> {
        let directory = self.directory.try_clone().map_err(|error| {
            format!(
                "failed to duplicate directory anchor {}: {error}",
                self.path.display()
            )
        })?;
        Ok(Self {
            path: self.path.clone(),
            identity: self.identity,
            directory,
        })
    }

    #[cfg(not(any(unix, windows)))]
    fn try_clone(&self) -> Result<Self, String> {
        Err(format!(
            "secure directory anchors are unavailable on this host: {}",
            self.path.display()
        ))
    }

    #[cfg(unix)]
    fn create_child(&self, name: &OsStr, display_path: &Path) -> Result<Self, String> {
        self.verify_path_binding()?;
        let directory =
            crate::platform::filesystem::create_owner_only_directory_child(&self.directory, name)
                .map_err(|error| {
                format!(
                    "failed to create private directory {}: {error}",
                    display_path.display()
                )
            })?;
        let identity = file_identity(&directory).map_err(|error| {
            format!(
                "failed to inspect private directory identity {}: {error}",
                display_path.display()
            )
        })?;
        if let Err(error) = self.verify_path_binding() {
            let cleanup = unlink_directory_child_at(&self.directory, name);
            return Err(match cleanup {
                Ok(()) => error,
                Err(cleanup) => format!("{error}; private directory cleanup failed: {cleanup}"),
            });
        }
        Ok(Self {
            path: display_path.to_path_buf(),
            identity,
            directory,
        })
    }

    #[cfg(windows)]
    fn create_child(&self, name: &OsStr, display_path: &Path) -> Result<Self, String> {
        let expected_path = self.path.join(name);
        if display_path != expected_path {
            return Err(format!(
                "private directory display path does not match anchored child {}: {}",
                expected_path.display(),
                display_path.display()
            ));
        }
        self.verify_path_binding()?;
        let directory =
            create_owner_only_directory_child(&self.directory, name).map_err(|error| {
                format!(
                    "failed to create private directory {}: {error}",
                    display_path.display()
                )
            })?;
        let identity = file_identity(&directory).map_err(|error| {
            format!(
                "failed to inspect private directory identity {}; the unverified directory was left untouched: {error}",
                display_path.display()
            )
        })?;
        let cleanup_error = |error: String| {
            match remove_bound_directory_child(&self.directory, name, identity, display_path) {
                Ok(()) => error,
                Err(cleanup) => format!(
                    "{error}; failed to remove private directory created through the retained parent anchor {}: {cleanup}",
                    display_path.display()
                ),
            }
        };
        if let Err(error) = verify_owner_only_acl(&directory) {
            return Err(cleanup_error(format!(
                "failed to verify private directory ACL {}: {error}",
                display_path.display()
            )));
        }
        if let Err(error) = self.verify_path_binding() {
            return Err(cleanup_error(error));
        }
        let rebound = open_directory_child_nofollow(&self.directory, name).map_err(|error| {
            cleanup_error(format!(
                "failed to bind newly created private directory {}: {error}",
                display_path.display()
            ))
        })?;
        let rebound_identity = file_identity(&rebound).map_err(|error| {
            cleanup_error(format!(
                "failed to recheck private directory identity {}: {error}",
                display_path.display()
            ))
        })?;
        if rebound_identity != identity {
            return Err(cleanup_error(format!(
                "private directory identity changed during creation: {}",
                display_path.display()
            )));
        }
        Ok(Self {
            path: display_path.to_path_buf(),
            identity,
            directory,
        })
    }

    #[cfg(not(any(unix, windows)))]
    fn create_child(&self, _name: &OsStr, display_path: &Path) -> Result<Self, String> {
        Err(format!(
            "secure mkdirat private directories are unavailable on this host: {}",
            display_path.display()
        ))
    }

    #[cfg(not(any(unix, windows)))]
    fn capture_exact(path: &Path) -> Result<Self, String> {
        Err(format!(
            "secure directory anchors are unavailable on this host: {}",
            path.display()
        ))
    }

    #[cfg(any(unix, windows))]
    fn verify_path_binding(&self) -> Result<(), String> {
        let directory = open_directory_nofollow(&self.path).map_err(|error| {
            format!(
                "failed to securely reopen directory anchor {}: {error}",
                self.path.display()
            )
        })?;
        let identity = file_identity(&directory).map_err(|error| {
            format!(
                "failed to recheck directory anchor {}: {error}",
                self.path.display()
            )
        })?;
        if identity != self.identity {
            return Err(format!(
                "directory anchor identity changed during full dump: {}",
                self.path.display()
            ));
        }
        Ok(())
    }

    #[cfg(not(any(unix, windows)))]
    fn verify_path_binding(&self) -> Result<(), String> {
        Err(format!(
            "secure directory anchors are unavailable on this host: {}",
            self.path.display()
        ))
    }

    fn capture_child(&self, name: &OsStr, display_path: &Path) -> Result<TreeSnapshot, String> {
        self.verify_path_binding()?;
        let snapshot = capture_tree_child_nofollow(&self.directory, name, display_path)?;
        self.verify_path_binding()?;
        Ok(snapshot)
    }

    #[cfg(any(unix, windows))]
    fn capture_child_root_identity(
        &self,
        name: &OsStr,
        display_path: &Path,
    ) -> Result<Option<FileIdentity>, String> {
        self.verify_path_binding()?;
        let directory = match open_directory_child_nofollow(&self.directory, name) {
            Ok(directory) => directory,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                self.verify_path_binding()?;
                return Ok(None);
            }
            Err(error) => {
                return Err(format!(
                    "failed to bind dump target root identity {}: {error}",
                    display_path.display()
                ));
            }
        };
        let identity = file_identity(&directory).map_err(|error| {
            format!(
                "failed to inspect dump target root identity {}: {error}",
                display_path.display()
            )
        })?;
        self.verify_path_binding()?;
        Ok(Some(identity))
    }

    #[cfg(not(any(unix, windows)))]
    fn capture_child_root_identity(
        &self,
        _name: &OsStr,
        display_path: &Path,
    ) -> Result<Option<FileIdentity>, String> {
        Err(format!(
            "secure dump target root identity checks are unavailable on this host: {}",
            display_path.display()
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum PrivateRetention {
    None,
    Recovery,
    EntireStage,
}

struct PrivateDumpStage {
    parent_anchor: DirectoryAnchor,
    root_anchor: Option<DirectoryAnchor>,
    root_name: OsString,
    root: PathBuf,
    execution: PathBuf,
    recovery: PathBuf,
    execution_anchor: Option<DirectoryAnchor>,
    recovery_anchor: Option<DirectoryAnchor>,
    root_identity: FileIdentity,
    execution_identity: FileIdentity,
    recovery_identity: FileIdentity,
    #[cfg(windows)]
    root_removed: bool,
    #[cfg(windows)]
    root_disposition_requested: bool,
    #[cfg(windows)]
    execution_removed: bool,
    #[cfg(windows)]
    recovery_removed: bool,
    staged_tree: PathBuf,
    effective_config: PathBuf,
    effective_config_handle: Option<File>,
    effective_config_secret_present: bool,
    retention: PrivateRetention,
    staged_snapshot: Option<TreeSnapshot>,
    recovery_children: BTreeMap<OsString, TreeSnapshot>,
    publication_uncertain: bool,
    writer_completion_pending: bool,
    cleanup_on_drop: bool,
}

impl PrivateDumpStage {
    fn create(target_parent: &DirectoryAnchor) -> Result<Self, String> {
        let parent_anchor = target_parent.try_clone()?;
        let root_name = OsString::from(format!(".unica-dump-guard-{}", Uuid::new_v4()));
        let root = target_parent.path.join(&root_name);
        run_private_create_hook(&root);
        let root_anchor = parent_anchor.create_child(&root_name, &root)?;
        let execution = root.join("execution");
        let recovery = root.join("recovery");
        let execution_anchor = match root_anchor.create_child(OsStr::new("execution"), &execution) {
            Ok(anchor) => anchor,
            Err(error) => {
                #[cfg(windows)]
                return Err(windows_private_creation_error(
                    WindowsPrivateCreationRollback {
                        parent_anchor: &parent_anchor,
                        root_anchor,
                        root_name: &root_name,
                        root: &root,
                        execution_anchor: None,
                        execution: &execution,
                        recovery_anchor: None,
                        recovery: &recovery,
                        primary: error,
                    },
                ));
                #[cfg(not(windows))]
                return Err(private_creation_error(
                    &parent_anchor,
                    &root_anchor,
                    &root_name,
                    &root,
                    error,
                ));
            }
        };
        #[cfg(windows)]
        if let Err(error) =
            run_private_creation_failpoint(PrivateCreationCheckpoint::AfterExecutionCreation)
        {
            return Err(windows_private_creation_error(
                WindowsPrivateCreationRollback {
                    parent_anchor: &parent_anchor,
                    root_anchor,
                    root_name: &root_name,
                    root: &root,
                    execution_anchor: Some(execution_anchor),
                    execution: &execution,
                    recovery_anchor: None,
                    recovery: &recovery,
                    primary: error,
                },
            ));
        }
        let recovery_anchor = match root_anchor.create_child(OsStr::new("recovery"), &recovery) {
            Ok(anchor) => anchor,
            Err(error) => {
                #[cfg(windows)]
                return Err(windows_private_creation_error(
                    WindowsPrivateCreationRollback {
                        parent_anchor: &parent_anchor,
                        root_anchor,
                        root_name: &root_name,
                        root: &root,
                        execution_anchor: Some(execution_anchor),
                        execution: &execution,
                        recovery_anchor: None,
                        recovery: &recovery,
                        primary: error,
                    },
                ));
                #[cfg(not(windows))]
                return Err(private_creation_error(
                    &parent_anchor,
                    &root_anchor,
                    &root_name,
                    &root,
                    error,
                ));
            }
        };
        #[cfg(windows)]
        if let Err(error) =
            run_private_creation_failpoint(PrivateCreationCheckpoint::AfterRecoveryCreation)
        {
            return Err(windows_private_creation_error(
                WindowsPrivateCreationRollback {
                    parent_anchor: &parent_anchor,
                    root_anchor,
                    root_name: &root_name,
                    root: &root,
                    execution_anchor: Some(execution_anchor),
                    execution: &execution,
                    recovery_anchor: Some(recovery_anchor),
                    recovery: &recovery,
                    primary: error,
                },
            ));
        }
        if let Err(error) = parent_anchor.verify_path_binding() {
            #[cfg(windows)]
            return Err(windows_private_creation_error(
                WindowsPrivateCreationRollback {
                    parent_anchor: &parent_anchor,
                    root_anchor,
                    root_name: &root_name,
                    root: &root,
                    execution_anchor: Some(execution_anchor),
                    execution: &execution,
                    recovery_anchor: Some(recovery_anchor),
                    recovery: &recovery,
                    primary: error,
                },
            ));
            #[cfg(not(windows))]
            return Err(private_creation_error(
                &parent_anchor,
                &root_anchor,
                &root_name,
                &root,
                error,
            ));
        };
        let root_identity = root_anchor.identity;
        let execution_identity = execution_anchor.identity;
        let recovery_identity = recovery_anchor.identity;
        Ok(Self {
            staged_tree: execution.join("staged-source"),
            effective_config: execution.join(EFFECTIVE_CONFIG_NAME),
            parent_anchor,
            root_anchor: Some(root_anchor),
            root_name,
            root,
            execution,
            recovery,
            execution_anchor: Some(execution_anchor),
            recovery_anchor: Some(recovery_anchor),
            root_identity,
            execution_identity,
            recovery_identity,
            #[cfg(windows)]
            root_removed: false,
            #[cfg(windows)]
            root_disposition_requested: false,
            #[cfg(windows)]
            execution_removed: false,
            #[cfg(windows)]
            recovery_removed: false,
            effective_config_handle: None,
            effective_config_secret_present: false,
            retention: PrivateRetention::None,
            staged_snapshot: None,
            recovery_children: BTreeMap::new(),
            publication_uncertain: false,
            writer_completion_pending: false,
            cleanup_on_drop: true,
        })
    }

    fn root_anchor(&self) -> &DirectoryAnchor {
        self.root_anchor
            .as_ref()
            .expect("private root anchor remains open before cleanup")
    }

    fn execution_anchor(&self) -> &DirectoryAnchor {
        self.execution_anchor
            .as_ref()
            .expect("private execution anchor remains open before cleanup")
    }

    fn recovery_anchor(&self) -> &DirectoryAnchor {
        self.recovery_anchor
            .as_ref()
            .expect("private recovery anchor remains open before cleanup")
    }

    fn bound_root_for_reporting(&self) -> Result<DirectoryAnchor, String> {
        let root = self
            .parent_anchor
            .capture_descendant(Path::new(&self.root_name), &self.root)?;
        if root.identity != self.root_identity {
            return Err(format!(
                "private dump root identity changed at its original name: {}",
                self.root.display()
            ));
        }
        Ok(root)
    }

    fn verify_recovery_location(&self) -> Result<(), String> {
        let root = self.bound_root_for_reporting()?;
        let identity = root.capture_child_root_identity(OsStr::new("recovery"), &self.recovery)?;
        if identity != Some(self.recovery_identity) {
            return Err(format!(
                "private recovery directory identity changed at its original name: {}",
                self.recovery.display()
            ));
        }
        Ok(())
    }

    fn retained_location(&self) -> Result<Option<PathBuf>, String> {
        let location = match self.retention {
            PrivateRetention::None => return Ok(None),
            PrivateRetention::Recovery => &self.recovery,
            PrivateRetention::EntireStage => &self.root,
        };
        let proof = if self.retention == PrivateRetention::EntireStage {
            self.bound_root_for_reporting().map(drop)
        } else {
            self.verify_recovery_location()
        };
        proof.map(|()| Some(location.clone())).map_err(|error| {
            format!(
                "Private execution data retained, but its current location is unconfirmed; original name: {} ({error})",
                location.display()
            )
        })
    }

    fn begin_execution(&mut self) {
        self.writer_completion_pending = true;
    }

    fn confirm_execution_finished(&mut self) {
        self.writer_completion_pending = false;
    }

    fn retain_unconfirmed_execution(&mut self) -> Result<(), String> {
        self.retention = PrivateRetention::EntireStage;
        let _ = self.scrub_effective_config()?;
        self.effective_config_handle.take();
        self.cleanup_on_drop = false;
        Ok(())
    }

    fn capture_finished_staged_tree(&mut self) -> Result<TreeSnapshot, String> {
        self.verify_execution_location()?;
        let snapshot = self
            .execution_anchor()
            .capture_child(OsStr::new("staged-source"), &self.staged_tree)?;
        self.verify_execution_location()?;
        self.remember_staged_tree(&snapshot);
        Ok(snapshot)
    }

    fn verify_execution_location(&self) -> Result<(), String> {
        let root = self.bound_root_for_reporting()?;
        let identity =
            root.capture_child_root_identity(OsStr::new("execution"), &self.execution)?;
        if identity != Some(self.execution_identity) {
            return Err(format!(
                "private execution directory identity changed at its original name: {}",
                self.execution.display()
            ));
        }
        Ok(())
    }

    fn remember_staged_tree(&mut self, snapshot: &TreeSnapshot) {
        self.staged_snapshot = Some(snapshot.clone());
    }

    fn execution_children(&self) -> BTreeMap<OsString, TreeSnapshot> {
        self.staged_snapshot
            .as_ref()
            .map(|snapshot| (OsString::from("staged-source"), snapshot.clone()))
            .into_iter()
            .collect()
    }

    fn check_cleanup_bindings(&mut self) -> Result<(), String> {
        let Some(root) = self.root_anchor.as_ref() else {
            #[cfg(windows)]
            if self.execution_removed && self.recovery_removed {
                return Ok(());
            }
            self.retention = PrivateRetention::EntireStage;
            return Err("private root closed before cleanup ownership was confirmed".into());
        };
        let roots = BTreeMap::from([
            (OsString::from("execution"), self.execution_identity),
            (OsString::from("recovery"), self.recovery_identity),
        ]);
        if let Err(error) = Self::check_compartment(root, &roots) {
            self.retention = PrivateRetention::EntireStage;
            return Err(error);
        }
        let mut errors = Vec::new();
        if let Err(error) = self.check_known_compartment(
            OsStr::new("execution"),
            self.execution_identity,
            &self.execution_children(),
        ) {
            self.retention = PrivateRetention::EntireStage;
            errors.push(error);
        }
        if let Err(error) = self.check_known_compartment(
            OsStr::new("recovery"),
            self.recovery_identity,
            &self.recovery_children,
        ) {
            self.retention = self.retention.max(PrivateRetention::Recovery);
            errors.push(error);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    fn check_known_compartment(
        &self,
        name: &OsStr,
        expected_identity: FileIdentity,
        children: &BTreeMap<OsString, TreeSnapshot>,
    ) -> Result<(), String> {
        let path = self.root.join(name);
        let Some(anchor) = open_bound_cleanup_directory(
            &self.root_anchor().directory,
            name,
            expected_identity,
            &path,
        )?
        else {
            return Ok(());
        };
        let expected = children
            .iter()
            .filter_map(|(name, snapshot)| snapshot.root_identity().map(|id| (name.clone(), id)))
            .collect();
        Self::check_compartment(&anchor, &expected)
    }

    fn cleanup_execution(&mut self) -> Result<(), String> {
        #[cfg(windows)]
        if self.execution_removed {
            return Ok(());
        }
        let children = self.execution_children();
        remove_known_compartment(
            self.root_anchor.as_ref(),
            &mut self.execution_anchor,
            OsStr::new("execution"),
            self.execution_identity,
            &self.execution,
            &children,
        )?;
        #[cfg(windows)]
        {
            self.execution_removed = true;
        }
        Ok(())
    }

    fn cleanup_recovery(&mut self) -> Result<(), String> {
        #[cfg(windows)]
        if self.recovery_removed {
            return Ok(());
        }
        remove_known_compartment(
            self.root_anchor.as_ref(),
            &mut self.recovery_anchor,
            OsStr::new("recovery"),
            self.recovery_identity,
            &self.recovery,
            &self.recovery_children,
        )?;
        #[cfg(windows)]
        {
            self.recovery_removed = true;
        }
        Ok(())
    }

    #[cfg(any(unix, windows))]
    fn check_compartment(
        anchor: &DirectoryAnchor,
        expected: &BTreeMap<OsString, FileIdentity>,
    ) -> Result<(), String> {
        anchor.verify_path_binding()?;
        let names = crate::platform::filesystem::read_directory_names_with_limit(
            &anchor.directory,
            None,
            || Ok(()),
        )
        .map_err(|error| format!("could not inspect private cleanup bindings: {error}"))?;
        for name in names {
            let identity = anchor.capture_child_root_identity(&name, &anchor.path.join(&name))?;
            if identity.is_some() && identity != expected.get(&name).copied() {
                return Err(format!(
                    "private cleanup retained an unrecognized object at {}",
                    anchor.path.join(name).display(),
                ));
            }
        }
        anchor.verify_path_binding()
    }

    #[cfg(not(any(unix, windows)))]
    fn check_compartment(
        _anchor: &DirectoryAnchor,
        _expected: &BTreeMap<OsString, FileIdentity>,
    ) -> Result<(), String> {
        Err("retained private cleanup bindings are unavailable on this host".into())
    }

    fn write_effective_config(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.execution_anchor().verify_path_binding()?;
        run_effective_config_prepare_hook(&self.effective_config);
        let file = create_regular_child_owner_only(
            &self.execution_anchor().directory,
            OsStr::new(EFFECTIVE_CONFIG_NAME),
            &self.effective_config,
        )
        .map_err(|error| {
            format!(
                "failed to securely create private effective config {}: {error}",
                self.effective_config.display()
            )
        })?;
        restrict_stage_to_owner(&file).map_err(|error| {
            format!(
                "failed to restrict private effective config {}: {error}",
                self.effective_config.display()
            )
        })?;
        self.effective_config_handle = Some(file);
        self.effective_config_secret_present = true;
        self.effective_config_handle
            .as_mut()
            .expect("effective config handle was just installed")
            .write_all(bytes)
            .map_err(|error| {
                format!(
                    "failed to write private effective config {}: {error}",
                    self.effective_config.display()
                )
            })?;
        self.effective_config_handle
            .as_ref()
            .expect("effective config handle remains installed")
            .sync_all()
            .map_err(|error| {
                format!(
                    "failed to sync private effective config {}: {error}",
                    self.effective_config.display()
                )
            })?;
        self.execution_anchor()
            .verify_path_binding()
            .map_err(|error| {
                format!(
                    "private effective config directory changed after secure creation {}: {error}",
                    self.effective_config.display()
                )
            })?;
        Ok(())
    }

    fn scrub_effective_config(&mut self) -> Result<Option<FileIdentity>, String> {
        let Some(file) = self.effective_config_handle.as_ref() else {
            return Ok(None);
        };
        #[cfg(windows)]
        run_private_cleanup_failpoint(PrivateCleanupCheckpoint::BeforeEffectiveConfigScrub)?;
        let identity = file_identity(file).map_err(|error| {
            format!(
                "private effective config identity check failed for {}: {error}; secret-bearing private data may remain",
                self.effective_config.display()
            )
        })?;
        file.set_len(0).map_err(|error| {
            format!(
                "private effective config scrubbing failed for {}: {error}; secret-bearing private data may remain",
                self.effective_config.display()
            )
        })?;
        file.sync_all().map_err(|error| {
            format!(
                "private effective config scrub sync failed for {}: {error}; secret-bearing private data may remain",
                self.effective_config.display()
            )
        })?;
        self.effective_config_secret_present = false;
        Ok(Some(identity))
    }

    fn remove_effective_config(&mut self) -> Result<(), String> {
        if self.writer_completion_pending {
            return self.retain_unconfirmed_execution();
        }
        let Some(identity) = self.scrub_effective_config()? else {
            return Ok(());
        };
        let execution_anchor = self.execution_anchor.as_ref().ok_or_else(|| {
            format!(
                "private effective config cleanup cannot unlink {} because the execution anchor is closed; the scrubbed private file may remain",
                self.effective_config.display()
            )
        })?;
        let unlink = unlink_bound_regular_child(
            &execution_anchor.directory,
            OsStr::new(EFFECTIVE_CONFIG_NAME),
            identity,
        )
        .map_err(|error| {
            format!(
                "private effective config cleanup failed for {}: {error}",
                self.effective_config.display()
            )
        });
        self.effective_config_handle.take();
        unlink
    }

    fn preserve_for_recovery(&mut self) -> Result<(), String> {
        if self.writer_completion_pending {
            return self.retain_unconfirmed_execution();
        }
        self.retention = self.retention.max(PrivateRetention::Recovery);
        self.remove_effective_config()?;
        let mut errors = Vec::new();
        if let Err(error) = self.check_cleanup_bindings() {
            errors.push(error);
        }
        if self.retention != PrivateRetention::EntireStage {
            if let Err(error) = self.cleanup_execution() {
                self.retention = PrivateRetention::EntireStage;
                errors.push(format!(
                    "private execution cleanup failed before retaining recovery: {error}"
                ));
            }
        }
        if !self.effective_config_secret_present {
            self.cleanup_on_drop = false;
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    fn cleanup_now(&mut self) -> Result<(), String> {
        if !self.cleanup_on_drop {
            return Ok(());
        }
        if self.writer_completion_pending {
            return self.retain_unconfirmed_execution();
        }
        if self.publication_uncertain {
            self.retention = self.retention.max(PrivateRetention::Recovery);
        }
        if self.retention != PrivateRetention::None {
            return self.preserve_for_recovery();
        }
        self.remove_effective_config()?;
        if let Err(error) = self.check_cleanup_bindings() {
            let preservation = self.preserve_for_recovery();
            return Err(match preservation {
                Ok(()) => error,
                Err(cleanup) => format!("{error}; {cleanup}"),
            });
        }
        if let Err(error) = self.cleanup_execution() {
            if self.execution_anchor.is_none() {
                // An empty compartment may fail to unlink after its anchor was
                // released. The next attempt reopens and checks its identity.
                return Err(format!("private execution cleanup failed: {error}"));
            }
            // Retain both compartments when execution ownership is uncertain.
            self.retention = PrivateRetention::EntireStage;
            self.cleanup_on_drop = false;
            return Err(format!("private execution cleanup failed: {error}"));
        }
        if let Err(error) = self.cleanup_recovery() {
            if self.recovery_anchor.is_none() {
                return Err(format!("private recovery cleanup failed: {error}"));
            }
            self.retention = PrivateRetention::Recovery;
            self.cleanup_on_drop = false;
            return Err(format!("private recovery cleanup failed: {error}"));
        }
        self.cleanup_empty_root()?;
        self.cleanup_on_drop = false;
        Ok(())
    }

    #[cfg(not(windows))]
    fn cleanup_empty_root(&mut self) -> Result<(), String> {
        remove_empty_bound_directory_child(
            &self.parent_anchor.directory,
            &self.root_name,
            self.root_identity,
            &self.root,
        )
        .map_err(|error| {
            self.retention = PrivateRetention::EntireStage;
            format!("private dump root cleanup failed: {error}")
        })
    }

    #[cfg(windows)]
    fn cleanup_empty_root(&mut self) -> Result<(), String> {
        if !self.root_removed {
            if !self.root_disposition_requested {
                self.root_anchor.take();
                remove_empty_bound_directory_child(
                    &self.parent_anchor.directory,
                    &self.root_name,
                    self.root_identity,
                    &self.root,
                )
                .map_err(|error| format!("private dump root cleanup failed: {error}"))?;
                self.root_disposition_requested = true;
            }
            prove_windows_child_absent(&self.parent_anchor.directory, &self.root_name, &self.root)?;
            self.root_removed = true;
        }
        Ok(())
    }
}

#[cfg(windows)]
fn prove_windows_child_absent(
    parent: &File,
    name: &OsStr,
    display_path: &Path,
) -> Result<(), String> {
    match open_any_child_nofollow(parent, name) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Ok((child, _kind)) => {
            drop(child);
            Err(format!(
                "private path is still present after cleanup; absence was not proven: {}",
                display_path.display()
            ))
        }
        Err(error) => Err(format!(
            "failed to prove private path absent through its retained parent anchor {}: {error}",
            display_path.display()
        )),
    }
}

#[cfg(windows)]
fn release_anchor_and_remove_private_child(
    parent: &File,
    anchor: &mut Option<DirectoryAnchor>,
    removed: &mut bool,
    name: &OsStr,
    expected_identity: FileIdentity,
    display_path: &Path,
) -> Result<(), String> {
    if *removed {
        return Ok(());
    }
    if let Some(retained) = anchor.as_ref() {
        remove_directory_contents_nofollow(&retained.directory, display_path)?;
    }
    anchor.take();
    run_private_cleanup_failpoint(PrivateCleanupCheckpoint::BeforeCompartmentRemove)?;
    remove_bound_directory_child(parent, name, expected_identity, display_path)?;
    *removed = true;
    Ok(())
}

#[cfg(windows)]
struct WindowsPrivateCreationRollback<'a> {
    parent_anchor: &'a DirectoryAnchor,
    root_anchor: DirectoryAnchor,
    root_name: &'a OsStr,
    root: &'a Path,
    execution_anchor: Option<DirectoryAnchor>,
    execution: &'a Path,
    recovery_anchor: Option<DirectoryAnchor>,
    recovery: &'a Path,
    primary: String,
}

#[cfg(windows)]
fn windows_private_creation_error(rollback: WindowsPrivateCreationRollback<'_>) -> String {
    let WindowsPrivateCreationRollback {
        parent_anchor,
        root_anchor,
        root_name,
        root,
        mut execution_anchor,
        execution,
        mut recovery_anchor,
        recovery,
        primary,
    } = rollback;
    let root_identity = root_anchor.identity;
    let mut cleanup_errors = Vec::new();

    if let Some(execution_identity) = execution_anchor.as_ref().map(|anchor| anchor.identity) {
        let mut removed = false;
        if let Err(error) = release_anchor_and_remove_private_child(
            &root_anchor.directory,
            &mut execution_anchor,
            &mut removed,
            OsStr::new("execution"),
            execution_identity,
            execution,
        ) {
            cleanup_errors.push(format!(
                "private execution cleanup failed during construction rollback {}: {error}",
                execution.display()
            ));
        }
    }
    execution_anchor.take();

    if let Some(recovery_identity) = recovery_anchor.as_ref().map(|anchor| anchor.identity) {
        let mut removed = false;
        if let Err(error) = release_anchor_and_remove_private_child(
            &root_anchor.directory,
            &mut recovery_anchor,
            &mut removed,
            OsStr::new("recovery"),
            recovery_identity,
            recovery,
        ) {
            cleanup_errors.push(format!(
                "private recovery cleanup failed during construction rollback {}: {error}",
                recovery.display()
            ));
        }
    }
    recovery_anchor.take();

    if let Err(error) = remove_directory_contents_nofollow(&root_anchor.directory, root) {
        cleanup_errors.push(format!(
            "private dump contents cleanup failed for {}: {error}",
            root.display()
        ));
    }
    drop(root_anchor);

    let root_disposition_requested = match remove_bound_directory_child(
        &parent_anchor.directory,
        root_name,
        root_identity,
        root,
    ) {
        Ok(()) => true,
        Err(error) => {
            cleanup_errors.push(format!(
                "private dump root cleanup failed for {}: {error}",
                root.display()
            ));
            false
        }
    };
    if let Err(error) = prove_windows_child_absent(&parent_anchor.directory, root_name, root) {
        cleanup_errors.push(error);
    }
    if root_disposition_requested && cleanup_errors.is_empty() {
        primary
    } else {
        format!("{primary}; {}", cleanup_errors.join("; "))
    }
}

#[cfg(not(windows))]
fn private_creation_error(
    parent_anchor: &DirectoryAnchor,
    root_anchor: &DirectoryAnchor,
    root_name: &OsStr,
    root: &Path,
    primary: String,
) -> String {
    let contents_cleanup = remove_directory_contents_nofollow(&root_anchor.directory, root);
    let root_cleanup = remove_bound_directory_child(
        &parent_anchor.directory,
        root_name,
        root_anchor.identity,
        root,
    );
    let mut errors = Vec::new();
    if let Err(error) = contents_cleanup {
        errors.push(format!(
            "private dump contents cleanup failed for {}: {error}",
            root.display()
        ));
    }
    if let Err(error) = root_cleanup {
        errors.push(format!(
            "private dump root cleanup failed for {}: {error}",
            root.display()
        ));
    }
    if errors.is_empty() {
        primary
    } else {
        format!("{primary}; {}", errors.join("; "))
    }
}

impl Drop for PrivateDumpStage {
    fn drop(&mut self) {
        if self.cleanup_on_drop {
            let _ = self.cleanup_now();
        }
    }
}

#[cfg(any(unix, windows))]
fn open_bound_cleanup_directory(
    parent: &File,
    name: &OsStr,
    expected_identity: FileIdentity,
    path: &Path,
) -> Result<Option<DirectoryAnchor>, String> {
    let directory = match open_directory_child_nofollow(parent, name) {
        Ok(directory) => directory,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "could not bind private cleanup directory {}: {error}",
                path.display()
            ))
        }
    };
    if file_identity(&directory).map_err(|error| error.to_string())? != expected_identity {
        return Err(format!(
            "private cleanup directory identity changed; replacement retained: {}",
            path.display()
        ));
    }
    Ok(Some(DirectoryAnchor {
        path: path.to_path_buf(),
        identity: expected_identity,
        directory,
    }))
}

#[cfg(not(any(unix, windows)))]
fn open_bound_cleanup_directory(
    _parent: &File,
    _name: &OsStr,
    _expected_identity: FileIdentity,
    _path: &Path,
) -> Result<Option<DirectoryAnchor>, String> {
    Err("retained cleanup is unavailable on this host".into())
}

#[cfg(any(unix, windows))]
fn remove_known_compartment(
    parent: Option<&DirectoryAnchor>,
    retained: &mut Option<DirectoryAnchor>,
    name: &OsStr,
    identity: FileIdentity,
    path: &Path,
    children: &BTreeMap<OsString, TreeSnapshot>,
) -> Result<(), String> {
    let parent = parent.ok_or_else(|| "private cleanup parent anchor is closed".to_string())?;
    parent.verify_path_binding()?;
    let Some(anchor) = open_bound_cleanup_directory(&parent.directory, name, identity, path)?
    else {
        retained.take();
        return Ok(());
    };
    let names = crate::platform::filesystem::read_directory_names_with_limit(
        &anchor.directory,
        None,
        || Ok(()),
    )
    .map_err(|error| {
        format!(
            "could not enumerate private cleanup directory {}: {error}",
            path.display()
        )
    })?;
    if let Some(unknown) = names.iter().find(|name| !children.contains_key(*name)) {
        return Err(format!(
            "private cleanup retained an unrecognized object at {}",
            path.join(unknown).display()
        ));
    }
    for name in names {
        remove_owned_tree_child(
            &anchor.directory,
            &name,
            &path.join(&name),
            &children[&name],
        )?;
    }
    drop(anchor);
    retained.take();
    #[cfg(windows)]
    run_private_cleanup_failpoint(PrivateCleanupCheckpoint::BeforeCompartmentRemove)?;
    remove_empty_bound_directory_child(&parent.directory, name, identity, path)
}

#[cfg(not(any(unix, windows)))]
fn remove_known_compartment(
    _parent: Option<&DirectoryAnchor>,
    _retained: &mut Option<DirectoryAnchor>,
    _name: &OsStr,
    _identity: FileIdentity,
    _path: &Path,
    _children: &BTreeMap<OsString, TreeSnapshot>,
) -> Result<(), String> {
    Err("retained cleanup is unavailable on this host".into())
}

#[cfg(any(unix, windows))]
fn remove_owned_tree_child(
    parent: &File,
    name: &OsStr,
    path: &Path,
    expected: &TreeSnapshot,
) -> Result<(), String> {
    let TreeSnapshot::Directory { identity, entries } = expected else {
        return Err(format!(
            "cleanup has no owned directory snapshot for {}",
            path.display()
        ));
    };
    run_directory_cleanup_open_hook(path);
    let Some(anchor) = open_bound_cleanup_directory(parent, name, *identity, path)? else {
        return Ok(());
    };
    remove_owned_directory_contents(&anchor.directory, path, entries, Path::new(""))?;
    drop(anchor);
    remove_empty_bound_directory_child(parent, name, *identity, path)
}

#[cfg(any(unix, windows))]
fn remove_owned_directory_contents(
    directory: &File,
    path: &Path,
    entries: &[TreeEntrySnapshot],
    relative: &Path,
) -> Result<(), String> {
    let expected: BTreeMap<_, _> = entries
        .iter()
        .filter(|entry| entry.relative_path.parent() == Some(relative))
        .map(|entry| {
            (
                entry
                    .relative_path
                    .file_name()
                    .expect("snapshot entries have names"),
                &entry.kind,
            )
        })
        .collect();
    let names =
        crate::platform::filesystem::read_directory_names_with_limit(directory, None, || Ok(()))
            .map_err(|error| {
                format!(
                    "could not enumerate owned cleanup tree {}: {error}",
                    path.display()
                )
            })?;
    if let Some(unknown) = names
        .iter()
        .find(|name| !expected.contains_key(name.as_os_str()))
    {
        return Err(format!(
            "private cleanup retained an unrecognized descendant at {}",
            path.join(unknown).display()
        ));
    }
    for name in names {
        let child_path = path.join(&name);
        run_directory_cleanup_open_hook(&child_path);
        match expected[name.as_os_str()] {
            TreeEntryKind::Directory { identity } => {
                let Some(child) =
                    open_bound_cleanup_directory(directory, &name, *identity, &child_path)?
                else {
                    continue;
                };
                remove_owned_directory_contents(
                    &child.directory,
                    &child_path,
                    entries,
                    &relative.join(&name),
                )?;
                drop(child);
                remove_empty_bound_directory_child(directory, &name, *identity, &child_path)?;
            }
            TreeEntryKind::File { identity, .. } => {
                unlink_bound_regular_child(directory, &name, *identity)?;
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn remove_empty_bound_directory_child(
    parent: &File,
    name: &OsStr,
    expected_identity: FileIdentity,
    path: &Path,
) -> Result<(), String> {
    let Some(_anchor) = open_bound_cleanup_directory(parent, name, expected_identity, path)? else {
        return Ok(());
    };
    unlink_directory_child_at(parent, name).map_err(|error| {
        format!(
            "could not remove empty owned private directory {}: {error}",
            path.display()
        )
    })
}

#[cfg(windows)]
fn remove_empty_bound_directory_child(
    parent: &File,
    name: &OsStr,
    expected_identity: FileIdentity,
    path: &Path,
) -> Result<(), String> {
    let child = match open_any_child_for_delete(parent, name) {
        Ok(child) => child,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "could not bind empty private directory {}: {error}",
                path.display()
            ))
        }
    };
    if opened_child_kind(&child).map_err(|error| error.to_string())? != OpenedChildKind::Directory
        || file_identity(&child).map_err(|error| error.to_string())? != expected_identity
    {
        return Err(format!(
            "empty private directory identity changed; replacement retained: {}",
            path.display()
        ));
    }
    delete_open_child(&child).map_err(|error| {
        format!(
            "could not remove empty owned private directory {}: {error}",
            path.display()
        )
    })
}

#[cfg(not(any(unix, windows)))]
fn remove_empty_bound_directory_child(
    _parent: &File,
    _name: &OsStr,
    _expected_identity: FileIdentity,
    _path: &Path,
) -> Result<(), String> {
    Err("retained cleanup is unavailable on this host".into())
}

#[cfg(unix)]
fn unlink_bound_regular_child(
    parent: &File,
    name: &OsStr,
    expected_identity: FileIdentity,
) -> Result<(), String> {
    let rebound = open_regular_child_nofollow(parent, name)
        .map_err(|error| format!("failed to rebind the private file before cleanup: {error}"))?;
    let actual_identity = file_identity(&rebound)
        .map_err(|error| format!("failed to inspect the private file before cleanup: {error}"))?;
    if actual_identity != expected_identity {
        return Err(
            "private file identity changed before cleanup; the replacement was left untouched"
                .to_string(),
        );
    }
    unlink_non_directory_child_at(parent, name)
        .map_err(|error| format!("secure unlinkat failed: {error}"))
}

#[cfg(windows)]
fn unlink_bound_regular_child(
    parent: &File,
    name: &OsStr,
    expected_identity: FileIdentity,
) -> Result<(), String> {
    let child = open_any_child_for_delete(parent, name)
        .map_err(|error| format!("failed to rebind the private file before cleanup: {error}"))?;
    if opened_child_kind(&child)
        .map_err(|error| format!("failed to classify the private file before cleanup: {error}"))?
        != OpenedChildKind::RegularFile
    {
        return Err(
            "private file changed kind before cleanup; the replacement was left untouched"
                .to_string(),
        );
    }
    let actual_identity = file_identity(&child)
        .map_err(|error| format!("failed to inspect the private file before cleanup: {error}"))?;
    if actual_identity != expected_identity {
        return Err(
            "private file identity changed before cleanup; the replacement was left untouched"
                .to_string(),
        );
    }
    delete_open_child(&child)
        .map_err(|error| format!("secure handle-relative file cleanup failed: {error}"))
}

#[cfg(not(any(unix, windows)))]
fn unlink_bound_regular_child(
    _parent: &File,
    _name: &OsStr,
    _expected_identity: FileIdentity,
) -> Result<(), String> {
    Err("secure descriptor-relative file cleanup is unavailable on this host".to_string())
}

#[cfg(unix)]
fn remove_directory_contents_nofollow(directory: &File, display_path: &Path) -> Result<(), String> {
    for name in read_directory_names(directory).map_err(|error| {
        format!(
            "failed to enumerate the retained private directory anchor {}: {error}",
            display_path.display()
        )
    })? {
        let child_path = display_path.join(&name);
        run_directory_cleanup_open_hook(&child_path);
        match open_directory_child_nofollow(directory, &name) {
            Ok(child) => {
                let expected_identity = file_identity(&child).map_err(|error| {
                    format!(
                        "failed to inspect private directory {} before cleanup: {error}",
                        child_path.display()
                    )
                })?;
                remove_directory_contents_nofollow(&child, &child_path)?;
                let rebound = open_directory_child_nofollow(directory, &name).map_err(|error| {
                    format!(
                        "failed to rebind private directory {} before cleanup: {error}",
                        child_path.display()
                    )
                })?;
                let actual_identity = file_identity(&rebound).map_err(|error| {
                    format!(
                        "failed to recheck private directory {} before cleanup: {error}",
                        child_path.display()
                    )
                })?;
                if actual_identity != expected_identity {
                    return Err(format!(
                        "private directory identity changed before cleanup; replacement left untouched: {}",
                        child_path.display()
                    ));
                }
                unlink_directory_child_at(directory, &name).map_err(|error| {
                    format!(
                        "failed to remove private directory {} through its retained parent anchor: {error}",
                        child_path.display()
                    )
                })?;
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error)
                if matches!(
                    error.raw_os_error(),
                    Some(libc::ENOTDIR) | Some(libc::EINVAL) | Some(libc::ELOOP)
                ) =>
            {
                unlink_non_directory_child_at(directory, &name).map_err(|unlink_error| {
                    format!(
                        "failed to remove private non-directory entry {} through its retained parent anchor: {unlink_error}",
                        child_path.display()
                    )
                })?;
            }
            Err(error) => {
                return Err(format!(
                    "failed to inspect private entry {} before cleanup: {error}",
                    child_path.display()
                ));
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn remove_directory_contents_nofollow(directory: &File, display_path: &Path) -> Result<(), String> {
    for name in crate::platform::filesystem::read_directory_names(directory).map_err(|error| {
        format!(
            "failed to enumerate the retained private directory anchor {}: {error}",
            display_path.display()
        )
    })? {
        let child_path = display_path.join(&name);
        run_directory_cleanup_open_hook(&child_path);
        let child = match open_any_child_for_delete(directory, &name) {
            Ok(child) => child,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "failed to inspect private entry {} before cleanup: {error}",
                    child_path.display()
                ));
            }
        };
        let identity = file_identity(&child).map_err(|error| {
            format!(
                "failed to inspect private entry identity {} before cleanup: {error}",
                child_path.display()
            )
        })?;
        let kind = opened_child_kind(&child).map_err(|error| {
            format!(
                "failed to classify private entry {} before cleanup: {error}",
                child_path.display()
            )
        })?;

        if kind == OpenedChildKind::Directory {
            let directory_child =
                open_directory_child_nofollow(directory, &name).map_err(|error| {
                    format!(
                        "failed to bind private directory {} before cleanup: {error}",
                        child_path.display()
                    )
                })?;
            if file_identity(&directory_child).map_err(|error| {
                format!(
                    "failed to inspect private directory {} before cleanup: {error}",
                    child_path.display()
                )
            })? != identity
            {
                return Err(format!(
                    "private directory identity changed before cleanup; replacement left untouched: {}",
                    child_path.display()
                ));
            }
            remove_directory_contents_nofollow(&directory_child, &child_path)?;
        } else if kind == OpenedChildKind::Unsupported {
            return Err(format!(
                "private tree contains an unsupported entry that cannot be securely removed: {}",
                child_path.display()
            ));
        }

        let rebound = open_any_child_for_delete(directory, &name).map_err(|error| {
            format!(
                "failed to rebind private entry {} before cleanup: {error}",
                child_path.display()
            )
        })?;
        let rebound_identity = file_identity(&rebound).map_err(|error| {
            format!(
                "failed to recheck private entry {} before cleanup: {error}",
                child_path.display()
            )
        })?;
        let rebound_kind = opened_child_kind(&rebound).map_err(|error| {
            format!(
                "failed to reclassify private entry {} before cleanup: {error}",
                child_path.display()
            )
        })?;
        if rebound_identity != identity || rebound_kind != kind {
            return Err(format!(
                "private entry identity or kind changed before cleanup; replacement left untouched: {}",
                child_path.display()
            ));
        }
        delete_open_child(&rebound).map_err(|error| {
            format!(
                "failed to remove private entry {} through its retained parent anchor: {error}",
                child_path.display()
            )
        })?;
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn remove_directory_contents_nofollow(
    _directory: &File,
    display_path: &Path,
) -> Result<(), String> {
    Err(format!(
        "secure descriptor-relative private cleanup is unavailable on this host: {}",
        display_path.display()
    ))
}

#[cfg(unix)]
fn remove_bound_directory_child(
    parent: &File,
    name: &OsStr,
    expected_identity: FileIdentity,
    display_path: &Path,
) -> Result<(), String> {
    let child = open_directory_child_nofollow(parent, name).map_err(|error| {
        format!(
            "failed to rebind expected private directory {} before cleanup: {error}",
            display_path.display()
        )
    })?;
    let actual_identity = file_identity(&child).map_err(|error| {
        format!(
            "failed to inspect expected private directory {} before cleanup: {error}",
            display_path.display()
        )
    })?;
    if actual_identity != expected_identity {
        return Err(format!(
            "private directory identity changed before cleanup; replacement left untouched: {}",
            display_path.display()
        ));
    }
    remove_directory_contents_nofollow(&child, display_path)?;
    let rebound = open_directory_child_nofollow(parent, name).map_err(|error| {
        format!(
            "failed to rebind expected private directory {} after cleanup: {error}",
            display_path.display()
        )
    })?;
    if file_identity(&rebound).map_err(|error| {
        format!(
            "failed to recheck expected private directory {} after cleanup: {error}",
            display_path.display()
        )
    })? != expected_identity
    {
        return Err(format!(
            "private directory identity changed after cleanup; replacement left untouched: {}",
            display_path.display()
        ));
    }
    unlink_directory_child_at(parent, name).map_err(|error| {
        format!(
            "failed to remove expected private directory {} through its retained parent anchor: {error}",
            display_path.display()
        )
    })
}

#[cfg(windows)]
fn remove_bound_directory_child(
    parent: &File,
    name: &OsStr,
    expected_identity: FileIdentity,
    display_path: &Path,
) -> Result<(), String> {
    let delete_handle = open_any_child_for_delete(parent, name).map_err(|error| {
        format!(
            "failed to rebind expected private directory {} before cleanup: {error}",
            display_path.display()
        )
    })?;
    if opened_child_kind(&delete_handle).map_err(|error| {
        format!(
            "failed to classify expected private directory {} before cleanup: {error}",
            display_path.display()
        )
    })? != OpenedChildKind::Directory
    {
        return Err(format!(
            "private directory changed kind before cleanup; replacement left untouched: {}",
            display_path.display()
        ));
    }
    if file_identity(&delete_handle).map_err(|error| {
        format!(
            "failed to inspect expected private directory {} before cleanup: {error}",
            display_path.display()
        )
    })? != expected_identity
    {
        return Err(format!(
            "private directory identity changed before cleanup; replacement left untouched: {}",
            display_path.display()
        ));
    }

    let directory = open_directory_child_nofollow(parent, name).map_err(|error| {
        format!(
            "failed to retain expected private directory {} before cleanup: {error}",
            display_path.display()
        )
    })?;
    if file_identity(&directory).map_err(|error| {
        format!(
            "failed to inspect retained private directory {} before cleanup: {error}",
            display_path.display()
        )
    })? != expected_identity
    {
        return Err(format!(
            "private directory identity changed before cleanup; replacement left untouched: {}",
            display_path.display()
        ));
    }
    remove_directory_contents_nofollow(&directory, display_path)?;

    let rebound = open_any_child_for_delete(parent, name).map_err(|error| {
        format!(
            "failed to rebind expected private directory {} after cleanup: {error}",
            display_path.display()
        )
    })?;
    if file_identity(&rebound).map_err(|error| {
        format!(
            "failed to recheck expected private directory {} after cleanup: {error}",
            display_path.display()
        )
    })? != expected_identity
        || opened_child_kind(&rebound).map_err(|error| {
            format!(
                "failed to reclassify expected private directory {} after cleanup: {error}",
                display_path.display()
            )
        })? != OpenedChildKind::Directory
    {
        return Err(format!(
            "private directory identity or kind changed after cleanup; replacement left untouched: {}",
            display_path.display()
        ));
    }
    delete_open_child(&rebound).map_err(|error| {
        format!(
            "failed to remove expected private directory {} through its retained parent anchor: {error}",
            display_path.display()
        )
    })
}

#[cfg(not(any(unix, windows)))]
fn remove_bound_directory_child(
    _parent: &File,
    _name: &OsStr,
    _expected_identity: FileIdentity,
    display_path: &Path,
) -> Result<(), String> {
    Err(format!(
        "secure descriptor-relative private cleanup is unavailable on this host: {}",
        display_path.display()
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeSnapshot {
    Absent,
    Directory {
        identity: FileIdentity,
        entries: Vec<TreeEntrySnapshot>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntrySnapshot {
    pub relative_path: PathBuf,
    pub kind: TreeEntryKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeEntryKind {
    Directory {
        identity: FileIdentity,
    },
    File {
        identity: FileIdentity,
        size: u64,
        sha256: [u8; 32],
    },
}

impl TreeSnapshot {
    pub fn capture_target(path: &Path) -> Result<Self, String> {
        capture_tree_target_nofollow(path)
    }

    fn root_identity(&self) -> Option<FileIdentity> {
        match self {
            Self::Absent => None,
            Self::Directory { identity, .. } => Some(*identity),
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn capture_directory_snapshot(root: &Path) -> Result<Vec<TreeEntrySnapshot>, String> {
    let normalized = normalize_leaf_path(root)?;
    let (parent_path, name) = split_parent_and_name(&normalized)?;
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (parent_path, name);
        Err(format!(
            "secure no-follow directory snapshots are unavailable on this host: {}",
            root.display()
        ))
    }
    #[cfg(any(unix, windows))]
    {
        let directory = {
            let parent = open_directory_nofollow(&parent_path).map_err(|error| {
                format!(
                    "failed to securely open directory parent {}: {error}",
                    parent_path.display()
                )
            })?;
            open_directory_child_nofollow(&parent, &name).map_err(|error| {
                format!(
                    "failed to securely open directory {}: {error}",
                    root.display()
                )
            })?
        };
        let mut entries = Vec::new();
        capture_directory_snapshot_recursive(&directory, Path::new(""), root, &mut entries)?;
        entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        Ok(entries)
    }
}

#[cfg(unix)]
pub fn capture_tree_target_nofollow(path: &Path) -> Result<TreeSnapshot, String> {
    let normalized = normalize_leaf_path(path)?;
    let (parent_path, name) = split_parent_and_name(&normalized)?;
    let parent = open_directory_nofollow(&parent_path).map_err(|error| {
        format!(
            "failed to securely open dump target parent {}: {error}",
            parent_path.display()
        )
    })?;
    run_tree_open_hook(path);
    capture_tree_child_nofollow(&parent, &name, path)
}

#[cfg(unix)]
fn capture_tree_child_nofollow(
    parent: &File,
    name: &OsStr,
    display_path: &Path,
) -> Result<TreeSnapshot, String> {
    let directory = match open_directory_child_nofollow(parent, name) {
        Ok(directory) => directory,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(TreeSnapshot::Absent),
        Err(error) => {
            let detail = if matches!(
                error.raw_os_error(),
                Some(libc::ELOOP) | Some(libc::ENOTDIR)
            ) {
                "symbolic link, reparse point, or non-directory entry".to_string()
            } else {
                error.to_string()
            };
            return Err(format!(
                "dump target must be a real directory or absent {}: {detail}",
                display_path.display()
            ));
        }
    };
    let identity = file_identity(&directory).map_err(|error| {
        format!(
            "failed to inspect dump target directory identity {}: {error}",
            display_path.display()
        )
    })?;
    let mut entries = Vec::new();
    capture_directory_snapshot_recursive(&directory, Path::new(""), display_path, &mut entries)?;
    let rebound = open_directory_child_nofollow(parent, name).map_err(|error| {
        format!(
            "failed to rebind dump target directory {}: {error}",
            display_path.display()
        )
    })?;
    if file_identity(&rebound).map_err(|error| {
        format!(
            "failed to recheck dump target identity {}: {error}",
            display_path.display()
        )
    })? != identity
    {
        return Err(format!(
            "dump target directory identity changed while snapshotting: {}",
            display_path.display()
        ));
    }
    entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(TreeSnapshot::Directory { identity, entries })
}

#[cfg(windows)]
pub fn capture_tree_target_nofollow(path: &Path) -> Result<TreeSnapshot, String> {
    let normalized = normalize_leaf_path(path)?;
    let (parent_path, name) = split_parent_and_name(&normalized)?;
    let parent = open_directory_nofollow(&parent_path).map_err(|error| {
        format!(
            "failed to securely open dump target parent {}: {error}",
            parent_path.display()
        )
    })?;
    run_tree_open_hook(path);
    capture_tree_child_nofollow(&parent, &name, path)
}

#[cfg(windows)]
fn capture_tree_child_nofollow(
    parent: &File,
    name: &OsStr,
    display_path: &Path,
) -> Result<TreeSnapshot, String> {
    let directory = match open_directory_child_nofollow(parent, name) {
        Ok(directory) => directory,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(TreeSnapshot::Absent),
        Err(error) => {
            return Err(format!(
                "dump target must be a real directory or absent {}: {error}",
                display_path.display()
            ));
        }
    };
    let identity = file_identity(&directory).map_err(|error| {
        format!(
            "failed to inspect dump target directory identity {}: {error}",
            display_path.display()
        )
    })?;
    let mut entries = Vec::new();
    capture_directory_snapshot_recursive(&directory, Path::new(""), display_path, &mut entries)?;
    let rebound = open_directory_child_nofollow(parent, name).map_err(|error| {
        format!(
            "failed to rebind dump target directory {}: {error}",
            display_path.display()
        )
    })?;
    if file_identity(&rebound).map_err(|error| {
        format!(
            "failed to recheck dump target identity {}: {error}",
            display_path.display()
        )
    })? != identity
    {
        return Err(format!(
            "dump target directory identity changed while snapshotting: {}",
            display_path.display()
        ));
    }
    entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(TreeSnapshot::Directory { identity, entries })
}

#[cfg(not(any(unix, windows)))]
pub fn capture_tree_target_nofollow(path: &Path) -> Result<TreeSnapshot, String> {
    Err(format!(
        "secure no-follow directory snapshots are unavailable on this host: {}",
        path.display()
    ))
}

#[cfg(not(any(unix, windows)))]
fn capture_tree_child_nofollow(
    _parent: &File,
    _name: &OsStr,
    display_path: &Path,
) -> Result<TreeSnapshot, String> {
    Err(format!(
        "secure no-follow directory snapshots are unavailable on this host: {}",
        display_path.display()
    ))
}

#[cfg(windows)]
fn capture_directory_snapshot_recursive(
    directory: &File,
    relative_root: &Path,
    display_root: &Path,
    entries: &mut Vec<TreeEntrySnapshot>,
) -> Result<(), String> {
    let initial_names =
        crate::platform::filesystem::read_directory_names(directory).map_err(|error| {
            format!(
                "failed to securely enumerate {}: {error}",
                display_root.join(relative_root).display()
            )
        })?;
    for name in &initial_names {
        let relative = relative_root.join(name);
        let display_path = display_root.join(&relative);
        let (opened, opened_kind) = open_any_child_nofollow(directory, name).map_err(|error| {
            format!(
                "dump tree contains an unsupported entry {}: {error}",
                display_path.display()
            )
        })?;
        let identity = file_identity(&opened).map_err(|error| {
            format!(
                "failed to inspect entry identity {}: {error}",
                display_path.display()
            )
        })?;
        if opened_kind == OpenedChildKind::Directory {
            let child = open_directory_child_nofollow(directory, name).map_err(|error| {
                format!(
                    "failed to securely bind directory {}: {error}",
                    display_path.display()
                )
            })?;
            if file_identity(&child).map_err(|error| {
                format!(
                    "failed to inspect directory identity {}: {error}",
                    display_path.display()
                )
            })? != identity
            {
                return Err(format!(
                    "dump tree directory identity changed while opening: {}",
                    display_path.display()
                ));
            }
            entries.push(TreeEntrySnapshot {
                relative_path: relative.clone(),
                kind: TreeEntryKind::Directory { identity },
            });
            capture_directory_snapshot_recursive(&child, &relative, display_root, entries)?;
            let rebound = open_directory_child_nofollow(directory, name).map_err(|error| {
                format!(
                    "failed to rebind directory {}: {error}",
                    display_path.display()
                )
            })?;
            if file_identity(&rebound).map_err(|error| {
                format!(
                    "failed to recheck directory identity {}: {error}",
                    display_path.display()
                )
            })? != identity
            {
                return Err(format!(
                    "dump tree directory identity changed while snapshotting: {}",
                    display_path.display()
                ));
            }
            continue;
        }

        if opened_kind != OpenedChildKind::RegularFile {
            return Err(format!(
                "dump tree contains an unsupported entry kind: {}",
                display_path.display()
            ));
        }
        let mut file = open_regular_child_nofollow(directory, name).map_err(|error| {
            format!(
                "dump tree contains an unsupported entry {}: {error}",
                display_path.display()
            )
        })?;
        if file_identity(&file).map_err(|error| {
            format!(
                "failed to inspect file identity {}: {error}",
                display_path.display()
            )
        })? != identity
        {
            return Err(format!(
                "dump tree file identity changed while opening: {}",
                display_path.display()
            ));
        }
        if hard_link_count(&file).map_err(|error| {
            format!(
                "failed to inspect hard links for {}: {error}",
                display_path.display()
            )
        })? != 1
        {
            return Err(format!(
                "dump tree file must have exactly one hard link: {}",
                display_path.display()
            ));
        }
        let before = file.metadata().map_err(|error| {
            format!(
                "failed to inspect opened file {}: {error}",
                display_path.display()
            )
        })?;
        let mut hasher = Sha256::new();
        let mut size = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file
                .read(&mut buffer)
                .map_err(|error| format!("failed to read {}: {error}", display_path.display()))?;
            if read == 0 {
                break;
            }
            size += read as u64;
            hasher.update(&buffer[..read]);
        }
        run_tree_open_hook(&display_path);
        let after = file.metadata().map_err(|error| {
            format!(
                "failed to recheck opened file {}: {error}",
                display_path.display()
            )
        })?;
        if before.len() != after.len()
            || before.modified().ok() != after.modified().ok()
            || after.len() != size
        {
            return Err(format!(
                "dump tree file changed while snapshotting: {}",
                display_path.display()
            ));
        }
        if hard_link_count(&file).map_err(|error| {
            format!(
                "failed to recheck hard links for {}: {error}",
                display_path.display()
            )
        })? != 1
        {
            return Err(format!(
                "dump tree file hard-link count changed while snapshotting: {}",
                display_path.display()
            ));
        }
        let rebound = open_regular_child_nofollow(directory, name).map_err(|error| {
            format!("failed to rebind file {}: {error}", display_path.display())
        })?;
        if file_identity(&rebound).map_err(|error| {
            format!(
                "failed to recheck file identity {}: {error}",
                display_path.display()
            )
        })? != identity
        {
            return Err(format!(
                "dump tree file identity changed while snapshotting: {}",
                display_path.display()
            ));
        }
        entries.push(TreeEntrySnapshot {
            relative_path: relative,
            kind: TreeEntryKind::File {
                identity,
                size,
                sha256: hasher.finalize().into(),
            },
        });
    }
    let final_names =
        crate::platform::filesystem::read_directory_names(directory).map_err(|error| {
            format!(
                "failed to securely re-enumerate {}: {error}",
                display_root.join(relative_root).display()
            )
        })?;
    if final_names != initial_names {
        return Err(format!(
            "dump tree directory name set changed while snapshotting: {}",
            display_root.join(relative_root).display()
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn capture_directory_snapshot_recursive(
    directory: &File,
    relative_root: &Path,
    display_root: &Path,
    entries: &mut Vec<TreeEntrySnapshot>,
) -> Result<(), String> {
    for name in read_directory_names(directory).map_err(|error| {
        format!(
            "failed to securely enumerate {}: {error}",
            display_root.join(relative_root).display()
        )
    })? {
        let relative = relative_root.join(&name);
        let display_path = display_root.join(&relative);
        match open_directory_child_nofollow(directory, &name) {
            Ok(child) => {
                let identity = file_identity(&child).map_err(|error| {
                    format!(
                        "failed to inspect directory identity {}: {error}",
                        display_path.display()
                    )
                })?;
                entries.push(TreeEntrySnapshot {
                    relative_path: relative.clone(),
                    kind: TreeEntryKind::Directory { identity },
                });
                capture_directory_snapshot_recursive(&child, &relative, display_root, entries)?;
                let rebound = open_directory_child_nofollow(directory, &name).map_err(|error| {
                    format!(
                        "failed to rebind directory {}: {error}",
                        display_path.display()
                    )
                })?;
                if file_identity(&rebound).map_err(|error| {
                    format!(
                        "failed to recheck directory identity {}: {error}",
                        display_path.display()
                    )
                })? != identity
                {
                    return Err(format!(
                        "dump tree directory identity changed while snapshotting: {}",
                        display_path.display()
                    ));
                }
            }
            Err(directory_error)
                if matches!(
                    directory_error.raw_os_error(),
                    Some(libc::ENOTDIR) | Some(libc::EINVAL)
                ) =>
            {
                let mut file = open_regular_child_nofollow(directory, &name).map_err(|error| {
                    let detail = if error.raw_os_error() == Some(libc::ELOOP) {
                        "symbolic link or reparse point".to_string()
                    } else {
                        error.to_string()
                    };
                    format!(
                        "dump tree contains an unsupported entry {}: {detail}",
                        display_path.display()
                    )
                })?;
                let identity = file_identity(&file).map_err(|error| {
                    format!(
                        "failed to inspect file identity {}: {error}",
                        display_path.display()
                    )
                })?;
                let links = hard_link_count(&file).map_err(|error| {
                    format!(
                        "failed to inspect hard links for {}: {error}",
                        display_path.display()
                    )
                })?;
                if links != 1 {
                    return Err(format!(
                        "dump tree file must have exactly one hard link: {}",
                        display_path.display()
                    ));
                }
                let before = file.metadata().map_err(|error| {
                    format!(
                        "failed to inspect opened file {}: {error}",
                        display_path.display()
                    )
                })?;
                let mut hasher = Sha256::new();
                let mut size = 0_u64;
                let mut buffer = [0_u8; 64 * 1024];
                loop {
                    let read = file.read(&mut buffer).map_err(|error| {
                        format!("failed to read {}: {error}", display_path.display())
                    })?;
                    if read == 0 {
                        break;
                    }
                    size += read as u64;
                    hasher.update(&buffer[..read]);
                }
                let after = file.metadata().map_err(|error| {
                    format!(
                        "failed to recheck opened file {}: {error}",
                        display_path.display()
                    )
                })?;
                if before.len() != after.len()
                    || before.modified().ok() != after.modified().ok()
                    || after.len() != size
                {
                    return Err(format!(
                        "dump tree file changed while snapshotting: {}",
                        display_path.display()
                    ));
                }
                let rebound = open_regular_child_nofollow(directory, &name).map_err(|error| {
                    format!("failed to rebind file {}: {error}", display_path.display())
                })?;
                if file_identity(&rebound).map_err(|error| {
                    format!(
                        "failed to recheck file identity {}: {error}",
                        display_path.display()
                    )
                })? != identity
                {
                    return Err(format!(
                        "dump tree file identity changed while snapshotting: {}",
                        display_path.display()
                    ));
                }
                entries.push(TreeEntrySnapshot {
                    relative_path: relative,
                    kind: TreeEntryKind::File {
                        identity,
                        size,
                        sha256: hasher.finalize().into(),
                    },
                });
            }
            Err(error) => {
                let detail = if error.raw_os_error() == Some(libc::ELOOP) {
                    "symbolic link or reparse point".to_string()
                } else {
                    error.to_string()
                };
                return Err(format!(
                    "dump tree contains an unsupported entry {}: {detail}",
                    display_path.display()
                ));
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn read_directory_names(directory: &File) -> std::io::Result<Vec<OsString>> {
    // SAFETY: fcntl duplicates the live descriptor; fdopendir assumes ownership of that duplicate.
    let duplicate = unsafe { libc::fcntl(directory.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
    if duplicate < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: duplicate is a valid owned directory descriptor.
    let stream = unsafe { libc::fdopendir(duplicate) };
    if stream.is_null() {
        let error = std::io::Error::last_os_error();
        // SAFETY: fdopendir failed and did not take ownership of duplicate.
        unsafe {
            libc::close(duplicate);
        }
        return Err(error);
    }
    let mut names = Vec::new();
    loop {
        // SAFETY: stream remains valid until closed below.
        let entry = unsafe { libc::readdir(stream) };
        if entry.is_null() {
            break;
        }
        // SAFETY: d_name is a NUL-terminated array owned by the live dirent.
        let bytes = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        names.push(OsString::from_vec(bytes.to_vec()));
    }
    // SAFETY: stream is live and owns duplicate.
    if unsafe { libc::closedir(stream) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    names.sort();
    Ok(names)
}

#[cfg(not(any(unix, windows)))]
fn capture_directory_snapshot_recursive(
    _directory: &File,
    _relative_root: &Path,
    display_root: &Path,
    _entries: &mut Vec<TreeEntrySnapshot>,
) -> Result<(), String> {
    Err(format!(
        "secure no-follow directory snapshots are unavailable on this host: {}",
        display_root.display()
    ))
}

fn validate_staged_dump_with(
    root: &Path,
    kind: SourceSetKind,
    validator: &dyn DumpSemanticValidator,
) -> Result<TreeSnapshot, String> {
    let snapshot = TreeSnapshot::capture_target(root)?;
    validate_staged_dump_snapshot(root, kind, &snapshot, validator)
        .map_err(|error| error.to_string())?;
    Ok(snapshot)
}
fn validate_staged_dump_snapshot(
    root: &Path,
    kind: SourceSetKind,
    snapshot: &TreeSnapshot,
    validator: &dyn DumpSemanticValidator,
) -> Result<(), DumpValidationFailure> {
    let read = BoundDumpTree { root, snapshot };
    validator.validate_stage(&DumpTreeObservation::from_read_port(&read), kind)
}

struct BoundDumpTree<'a> {
    root: &'a Path,
    snapshot: &'a TreeSnapshot,
}
impl DumpTreeReadPort for BoundDumpTree<'_> {
    fn display_root(&self) -> &Path {
        self.root
    }
    fn is_directory(&self) -> bool {
        matches!(self.snapshot, TreeSnapshot::Directory { .. })
    }
    fn entry_count(&self) -> usize {
        match self.snapshot {
            TreeSnapshot::Directory { entries, .. } => entries.len(),
            TreeSnapshot::Absent => 0,
        }
    }
    fn entry_path(&self, index: usize) -> Option<&Path> {
        match self.snapshot {
            TreeSnapshot::Directory { entries, .. } => entries
                .get(index)
                .map(|entry| entry.relative_path.as_path()),
            TreeSnapshot::Absent => None,
        }
    }
    fn entry_is_file(&self, index: usize) -> bool {
        match self.snapshot {
            TreeSnapshot::Directory { entries, .. } => entries
                .get(index)
                .is_some_and(|entry| matches!(entry.kind, TreeEntryKind::File { .. })),
            TreeSnapshot::Absent => false,
        }
    }
    fn read_entry_exact(&self, index: usize, role: &str) -> Result<Vec<u8>, String> {
        let entry = match self.snapshot {
            TreeSnapshot::Directory { entries, .. } => entries.get(index),
            TreeSnapshot::Absent => None,
        }
        .ok_or_else(|| "full dump observation entry is unavailable".to_string())?;
        read_tree_bound_file(&self.root.join(&entry.relative_path), role, &entry.kind)
    }
}

fn read_tree_bound_file(
    path: &Path,
    role: &str,
    expected: &TreeEntryKind,
) -> Result<Vec<u8>, String> {
    let TreeEntryKind::File {
        identity,
        size,
        sha256,
    } = expected
    else {
        return Err(format!(
            "{role} is not a file in the captured tree snapshot: {}",
            path.display()
        ));
    };
    let actual = secure_read_regular_file_snapshot(path, role)?;
    let actual_size = actual.raw.len() as u64;
    let actual_sha256: [u8; 32] = Sha256::digest(&actual.raw).into();
    if actual.identity != *identity || actual_size != *size || actual_sha256 != *sha256 {
        return Err(format!(
            "{role} bytes or identity changed from the captured tree snapshot: {}",
            path.display()
        ));
    }
    Ok(actual.raw)
}

fn remove_created_source_parent(
    parent: &DirectoryAnchor,
    child: &DirectoryAnchor,
) -> Result<(), String> {
    use super::filesystem::RetainedDirectoryCapability;
    let retained_parent =
        RetainedDirectoryCapability::open(&parent.path).map_err(|error| error.to_string())?;
    if retained_parent.identity() != parent.identity {
        return Err(
            "created source parent ancestry changed; replacement was left untouched".into(),
        );
    }
    let retained_child = retained_parent
        .retain_directory_child(
            child
                .path
                .file_name()
                .ok_or("created source parent has no name")?,
        )
        .map_err(|error| error.to_string())?;
    if retained_child.identity() != child.identity {
        return Err("created source parent changed; replacement was left untouched".into());
    }
    retained_child
        .remove_empty_named_identity_relative_for_retained_apply(OsStr::new(&format!(
            ".source-parent-cleanup-{}",
            Uuid::new_v4()
        )))
        .map_err(|error| error.to_string())
}

/// Records the gate's refusal by whether it invoked the namespace operation,
/// rather than inferring cancellation from a later token value or error text.
struct PublicationCancellationGate<'a> {
    cancellation: &'a CancellationToken,
    cancelled_before_effect: &'a mut bool,
}

impl PublicationCancellationGate<'_> {
    fn first_effect(&mut self, effect: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        let mut attempted = false;
        let result = self.cancellation.handoff_with_gate(|| {
            attempted = true;
            effect()
        });
        if result.is_err() && !attempted {
            *self.cancelled_before_effect = true;
        }
        result
    }
}

fn publish_staged_tree(
    private: &mut PrivateDumpStage,
    target: &Path,
    target_parent: &DirectoryAnchor,
    original: &TreeSnapshot,
    staged: &TreeSnapshot,
    mut cancellation: Option<PublicationCancellationGate<'_>>,
) -> Result<Vec<String>, String> {
    let target_name = target
        .file_name()
        .ok_or_else(|| format!("dump target has no name: {}", target.display()))?;
    let sealed_name = OsString::from(format!("sealed-stage-{}", Uuid::new_v4()));
    let backup_name = OsString::from(format!("target-backup-{}", Uuid::new_v4()));
    let failed_name = OsString::from(format!("failed-publication-{}", Uuid::new_v4()));
    let sealed = private.recovery.join(&sealed_name);
    let backup = private.recovery.join(&backup_name);
    let failed = private.recovery.join(&failed_name);
    let had_target = matches!(original, TreeSnapshot::Directory { .. });
    let staged_identity = staged.root_identity().ok_or_else(|| {
        format!(
            "validated staged dump has no directory identity: {}",
            private.staged_tree.display()
        )
    })?;
    private.remember_staged_tree(staged);
    private
        .recovery_children
        .insert(sealed_name.clone(), staged.clone());

    run_publication_failpoint(PublicationCheckpoint::BeforeStageInstall)?;
    let previous_retention = private.retention;
    private.retention = PrivateRetention::EntireStage;
    rename_prechecked_directory_child_no_replace(
        private.execution_anchor(),
        OsStr::new("staged-source"),
        staged_identity,
        private.recovery_anchor(),
        &sealed_name,
    )
    .map_err(|error| {
        format!(
            "failed to atomically seal the validated staged dump {} in {}: {error}",
            private.staged_tree.display(),
            sealed.display()
        )
    })?;
    let sealed_snapshot = private
        .recovery_anchor()
        .capture_child(&sealed_name, &sealed)?;
    if &sealed_snapshot != staged {
        return Err(format!(
            "private staged dump changed while it was being sealed; Git-visible sources were not touched: {}",
            private.staged_tree.display()
        ));
    }
    private.retention = previous_retention;

    if had_target {
        private
            .recovery_children
            .insert(backup_name.clone(), original.clone());
        let mut first_effect = || {
            private.publication_uncertain = true;
            rename_child_no_replace(
                target_parent,
                target_name,
                private.recovery_anchor(),
                &backup_name,
            )
        };
        match cancellation.as_mut() {
            Some(cancellation) => cancellation.first_effect(first_effect),
            None => first_effect(),
        }
        .map_err(|error| {
            format!(
                "failed to atomically move dump target {} to private recovery {} without clobbering: {error}",
                target.display(),
                backup.display()
            )
        })?;
        if let Err(error) = target_parent.verify_path_binding() {
            return Err(retain_recovery_error(
                private,
                format!("dump target parent changed after the atomic backup move: {error}"),
            ));
        }
        let moved = match private
            .recovery_anchor()
            .capture_child(&backup_name, &backup)
        {
            Ok(moved) => moved,
            Err(error) => {
                return Err(retain_recovery_error(
                    private,
                    format!("could not verify the atomic target backup: {error}"),
                ));
            }
        };
        if &moved != original {
            let primary = format!(
                "dump target changed during the atomic backup move; staged output was not published: {}",
                target.display()
            );
            return rollback_before_stage_install(
                private,
                target,
                target_parent,
                target_name,
                &backup_name,
                &moved,
                false,
                primary,
            );
        }
        if let Err(error) = run_publication_failpoint(PublicationCheckpoint::AfterBackup) {
            return rollback_before_stage_install(
                private,
                target,
                target_parent,
                target_name,
                &backup_name,
                original,
                true,
                error,
            );
        }
    }
    if let Err(error) = run_publication_failpoint(PublicationCheckpoint::BeforeSealedPublishRename)
    {
        if had_target {
            return rollback_before_stage_install(
                private,
                target,
                target_parent,
                target_name,
                &backup_name,
                original,
                true,
                error,
            );
        }
        return Err(error);
    }
    // Refuse a source-name replacement before installation. After the move,
    // recovery is permitted only for the root identity of this validated stage.
    let mut install = || {
        private.publication_uncertain = true;
        rename_prechecked_directory_child_no_replace(
            private.recovery_anchor(),
            &sealed_name,
            staged_identity,
            target_parent,
            target_name,
        )
    };
    let installation = if had_target {
        install()
    } else {
        match cancellation.as_mut() {
            Some(cancellation) => cancellation.first_effect(install),
            None => install(),
        }
    };
    if let Err(error) = installation {
        let primary = format!(
            "failed to atomically publish validated staged dump {} to {} without clobbering: {error}",
            sealed.display(),
            target.display()
        );
        if had_target {
            return rollback_before_stage_install(
                private,
                target,
                target_parent,
                target_name,
                &backup_name,
                original,
                true,
                primary,
            );
        }
        return Err(primary);
    }

    let reject_publication = |private: &mut PrivateDumpStage,
                              installed: Option<&TreeSnapshot>,
                              error: String| {
        let identity = match installed {
            Some(snapshot) => Ok(snapshot.root_identity()),
            None => target_parent.capture_child_root_identity(target_name, target),
        };
        match identity {
            Ok(Some(identity)) if identity == staged_identity => {
                quarantine_unverified_and_rollback(
                    private,
                    target,
                    target_parent,
                    target_name,
                    &failed_name,
                    &failed,
                    staged,
                    installed,
                    had_target,
                    &backup_name,
                    original,
                    error,
                )
            }
            Ok(None) if had_target => rollback_before_stage_install(
                private,
                target,
                target_parent,
                target_name,
                &backup_name,
                original,
                true,
                error,
            ),
            Ok(None) => {
                private.publication_uncertain = false;
                Err(error)
            }
            Ok(Some(_)) => {
                let error = format!(
                    "{error}; a concurrent target with a different root identity now occupies {} and was left untouched",
                    target.display()
                );
                Err(retain_recovery_error(private, error))
            }
            Err(identity_error) => {
                let error = format!(
                    "{error}; published target ownership could not be classified: {identity_error}"
                );
                Err(retain_recovery_error(private, error))
            }
        }
    };

    run_publication_hook(PublicationCheckpoint::BeforeInstalledSnapshot);
    let installed = match target_parent.capture_child(target_name, target) {
        Ok(installed) => installed,
        Err(error) => {
            return reject_publication(
                private,
                None,
                format!("could not verify the published target before commit: {error}"),
            );
        }
    };
    if &installed != staged {
        return reject_publication(
            private,
            Some(&installed),
            format!(
                "published dump differs from the validated staged snapshot before commit: {}",
                target.display()
            ),
        );
    }

    let checkpoint_error =
        run_publication_failpoint(PublicationCheckpoint::AfterStageInstall).err();
    let published = match target_parent.capture_child(target_name, target) {
        Ok(published) => published,
        Err(capture_error) => {
            let error = match checkpoint_error.as_deref() {
                Some(checkpoint_error) => format!(
                    "{checkpoint_error}; could not verify the published target: {capture_error}"
                ),
                None => format!("could not verify the published target: {capture_error}"),
            };
            return reject_publication(private, None, error);
        }
    };
    let validation_error = (&published != staged).then(|| {
        format!(
            "published dump differs from the validated staged snapshot: {}",
            target.display()
        )
    });
    if let Some(error) = checkpoint_error.or(validation_error) {
        return reject_publication(private, Some(&published), error);
    }

    if let Err(error) = target_parent.verify_path_binding() {
        return Err(retain_recovery_error(
            private,
            format!("dump target parent changed after publication verification: {error}"),
        ));
    }
    private.publication_uncertain = false;
    Ok(Vec::new())
}

#[allow(clippy::too_many_arguments)]
fn quarantine_unverified_install(
    private: &mut PrivateDumpStage,
    target: &Path,
    target_parent: &DirectoryAnchor,
    target_name: &OsStr,
    quarantine_name: &OsStr,
    quarantine_path: &Path,
    owned_tree: &TreeSnapshot,
    installed: Option<&TreeSnapshot>,
) -> Result<(), String> {
    let expected_identity = owned_tree
        .root_identity()
        .ok_or_else(|| "unverified publication has no owned root identity".to_string())?;
    // Protect recovery before a rename whose outcome may be uncertain. Cleanup
    // may delete it again only after the moved root is proved to be our tree.
    let previous_retention = private.retention;
    private.retention = private.retention.max(PrivateRetention::Recovery);
    private
        .recovery_children
        .entry(quarantine_name.to_os_string())
        .or_insert_with(|| owned_tree.clone());
    rename_prechecked_directory_child_no_replace(
        target_parent,
        target_name,
        expected_identity,
        private.recovery_anchor(),
        quarantine_name,
    )
    .map_err(|error| {
        format!(
            "could not atomically quarantine unverified publication {} at {}: {error}",
            target.display(),
            quarantine_path.display()
        )
    })?;
    match private
        .recovery_anchor()
        .capture_child_root_identity(quarantine_name, quarantine_path)
    {
        Ok(Some(identity)) if identity == expected_identity => {}
        Ok(Some(identity)) => {
            let return_result = rename_prechecked_directory_child_no_replace(
                private.recovery_anchor(),
                quarantine_name,
                identity,
                target_parent,
                target_name,
            )
            .and_then(|()| {
                if target_parent.capture_child_root_identity(target_name, target)? == Some(identity)
                {
                    Ok(())
                } else {
                    Err("returned target identity could not be confirmed".to_string())
                }
            });
            return Err(match return_result {
                Ok(()) => format!(
                    "quarantine captured a different root identity; the concurrent target was returned without clobbering: {}",
                    target.display()
                ),
                Err(error) => format!(
                    "quarantine captured a different root identity; recovery remains protected at {}: {error}",
                    quarantine_path.display()
                ),
            });
        }
        Ok(None) => {
            return Err(format!(
                "quarantined root is absent; recovery remains protected at {}",
                quarantine_path.display()
            ));
        }
        Err(error) => {
            return Err(format!(
                "could not verify quarantined root identity; recovery remains protected at {}: {error}",
                quarantine_path.display()
            ));
        }
    }
    if let Some(installed) = installed {
        let quarantined = private
            .recovery_anchor()
            .capture_child(quarantine_name, quarantine_path)
            .map_err(|error| {
                format!(
                    "could not verify quarantined unverified publication {}: {error}",
                    quarantine_path.display()
                )
            })?;
        if &quarantined != installed {
            return Err(format!(
                "quarantined tree does not match the unverified publication snapshot: {}",
                quarantine_path.display()
            ));
        }
        let current = target_parent.capture_child(target_name, target)?;
        if &current == installed {
            return Err(format!(
                "unverified publication still occupies the Git-visible target after quarantine: {}",
                target.display()
            ));
        }
    }
    private.retention = previous_retention;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn quarantine_unverified_and_rollback(
    private: &mut PrivateDumpStage,
    target: &Path,
    target_parent: &DirectoryAnchor,
    target_name: &OsStr,
    quarantine_name: &OsStr,
    quarantine_path: &Path,
    owned_tree: &TreeSnapshot,
    installed: Option<&TreeSnapshot>,
    had_target: bool,
    backup_name: &OsStr,
    expected_backup: &TreeSnapshot,
    primary: String,
) -> Result<Vec<String>, String> {
    if let Err(error) = quarantine_unverified_install(
        private,
        target,
        target_parent,
        target_name,
        quarantine_name,
        quarantine_path,
        owned_tree,
        installed,
    ) {
        return Err(retain_recovery_error(
            private,
            format!("{primary}; {error}"),
        ));
    }
    if had_target {
        rollback_before_stage_install(
            private,
            target,
            target_parent,
            target_name,
            backup_name,
            expected_backup,
            true,
            primary,
        )
    } else {
        match target_parent.capture_child_root_identity(target_name, target) {
            Ok(None) => {
                private.publication_uncertain = false;
                Err(primary)
            }
            Ok(Some(_)) => Err(retain_recovery_error(private, primary)),
            Err(error) => Err(retain_recovery_error(
                private,
                format!("{primary}; {error}"),
            )),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn rollback_before_stage_install(
    private: &mut PrivateDumpStage,
    target: &Path,
    target_parent: &DirectoryAnchor,
    target_name: &OsStr,
    backup_name: &OsStr,
    expected_backup: &TreeSnapshot,
    restores_original: bool,
    primary: String,
) -> Result<Vec<String>, String> {
    let backup = private.recovery.join(backup_name);
    let expected_identity = match expected_backup.root_identity() {
        Some(identity) => identity,
        None => {
            return Err(retain_recovery_error(
                private,
                format!(
                    "{primary}; rollback backup has no expected directory identity: {}",
                    backup.display()
                ),
            ))
        }
    };
    if let Err(error) = rename_prechecked_directory_child_no_replace(
        private.recovery_anchor(),
        backup_name,
        expected_identity,
        target_parent,
        target_name,
    ) {
        return Err(retain_recovery_error(
            private,
            format!(
                "{primary}; rollback could not atomically restore original target {} from {} without clobbering: {error}",
                target.display(),
                backup.display()
            ),
        ));
    }

    let restored = match target_parent.capture_child(target_name, target) {
        Ok(restored) => restored,
        Err(snapshot_error) => {
            let detail = format!(
                "{primary}; rollback source could not be validated after tentative restore: {snapshot_error}"
            );
            if let Err(quarantine_error) = quarantine_unverified_install(
                private,
                target,
                target_parent,
                target_name,
                backup_name,
                &backup,
                expected_backup,
                None,
            ) {
                return Err(retain_recovery_error(
                    private,
                    format!("{detail}; {quarantine_error}"),
                ));
            }
            return Err(retain_recovery_error(private, detail));
        }
    };
    if &restored != expected_backup {
        let detail = format!(
            "{primary}; rollback source changed before tentative restore and was not accepted: {}",
            target.display()
        );
        if matches!(restored, TreeSnapshot::Absent) {
            return Err(retain_recovery_error(private, detail));
        }
        if let Err(quarantine_error) = quarantine_unverified_install(
            private,
            target,
            target_parent,
            target_name,
            backup_name,
            &backup,
            expected_backup,
            Some(&restored),
        ) {
            return Err(retain_recovery_error(
                private,
                format!("{detail}; {quarantine_error}"),
            ));
        }
        return Err(retain_recovery_error(private, detail));
    }

    match target_parent.verify_path_binding() {
        Ok(()) => {
            if restores_original {
                private.publication_uncertain = false;
            }
            Err(primary)
        }
        Err(binding_error) => {
            let quarantine = quarantine_unverified_install(
                private,
                target,
                target_parent,
                target_name,
                backup_name,
                &backup,
                expected_backup,
                Some(expected_backup),
            );
            let detail = match quarantine {
                Ok(()) => format!(
                    "{primary}; target parent changed during rollback ({binding_error}); original target was returned to private recovery"
                ),
                Err(quarantine_error) => format!(
                    "{primary}; target parent changed during rollback ({binding_error}); original target could not be returned to private recovery: {quarantine_error}"
                ),
            };
            Err(retain_recovery_error(private, detail))
        }
    }
}

fn retain_recovery_error(private: &mut PrivateDumpStage, primary: String) -> String {
    match private.preserve_for_recovery() {
        Ok(()) => format!(
            "{primary}; data retained for recovery at {}",
            if private.retention == PrivateRetention::EntireStage { &private.root } else { &private.recovery }.display()
        ),
        Err(cleanup_error) => format!(
            "{primary}; recovery remains protected; private execution cleanup will be retried after {cleanup_error}: {}",
            private.root.display()
        ),
    }
}

#[cfg(unix)]
fn rename_prechecked_directory_child_no_replace(
    source_parent: &DirectoryAnchor,
    source_name: &OsStr,
    expected_identity: FileIdentity,
    destination_parent: &DirectoryAnchor,
    destination_name: &OsStr,
) -> Result<(), String> {
    source_parent.verify_path_binding()?;
    let source =
        open_directory_child_nofollow(&source_parent.directory, source_name).map_err(|error| {
            format!(
                "failed to bind source directory {}: {error}",
                source_parent.path.join(source_name).display()
            )
        })?;
    let actual_identity = file_identity(&source).map_err(|error| {
        format!(
            "failed to inspect source directory identity {}: {error}",
            source_parent.path.join(source_name).display()
        )
    })?;
    if actual_identity != expected_identity {
        return Err(format!(
            "source directory identity changed before atomic rename: {}",
            source_parent.path.join(source_name).display()
        ));
    }
    rename_child_no_replace(
        source_parent,
        source_name,
        destination_parent,
        destination_name,
    )
}

#[cfg(windows)]
fn rename_prechecked_directory_child_no_replace(
    source_parent: &DirectoryAnchor,
    source_name: &OsStr,
    expected_identity: FileIdentity,
    destination_parent: &DirectoryAnchor,
    destination_name: &OsStr,
) -> Result<(), String> {
    let source = open_directory_child_for_rename(&source_parent.directory, source_name).map_err(
        |error| {
            format!(
                "failed to open staged child {}: {error}",
                source_parent.path.join(source_name).display()
            )
        },
    )?;
    if file_identity(&source).map_err(|error| error.to_string())? != expected_identity {
        return Err("staged directory identity changed".to_string());
    }
    rename_open_directory_child_no_replace(source, destination_parent, destination_name)
}

#[cfg(not(any(unix, windows)))]
fn rename_prechecked_directory_child_no_replace(
    _source_parent: &DirectoryAnchor,
    _source_name: &OsStr,
    _expected_identity: FileIdentity,
    _destination_parent: &DirectoryAnchor,
    _destination_name: &OsStr,
) -> Result<(), String> {
    Err("prechecked atomic no-clobber directory rename is unavailable on this host".to_string())
}

#[cfg(unix)]
fn rename_child_no_replace(
    source_parent: &DirectoryAnchor,
    source_name: &OsStr,
    destination_parent: &DirectoryAnchor,
    destination_name: &OsStr,
) -> Result<(), String> {
    source_parent.verify_path_binding()?;
    destination_parent.verify_path_binding()?;
    run_directory_rename_hook(&source_parent.path.join(source_name));
    let source_name = CString::new(source_name.as_bytes()).map_err(|error| {
        format!(
            "source name contains an embedded NUL byte in {}: {error}",
            source_parent.path.display()
        )
    })?;
    let destination_name = CString::new(destination_name.as_bytes()).map_err(|error| {
        format!(
            "destination name contains an embedded NUL byte in {}: {error}",
            destination_parent.path.display()
        )
    })?;
    #[cfg(target_os = "linux")]
    let no_replace_flag = libc::RENAME_NOREPLACE;
    #[cfg(target_os = "android")]
    let no_replace_flag = libc::RENAME_NOREPLACE as libc::c_uint;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    // SAFETY: both directory descriptors and both NUL-terminated names remain
    // live for the syscall. Calling SYS_renameat2 directly avoids a glibc-only
    // symbol on musl; RENAME_NOREPLACE atomically protects destination absence.
    let status = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            source_parent.directory.as_raw_fd(),
            source_name.as_ptr(),
            destination_parent.directory.as_raw_fd(),
            destination_name.as_ptr(),
            no_replace_flag,
        )
    };
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    // SAFETY: both directory descriptors and both NUL-terminated names remain
    // live for the syscall. RENAME_EXCL atomically protects destination absence.
    let status = unsafe {
        libc::renameatx_np(
            source_parent.directory.as_raw_fd(),
            source_name.as_ptr(),
            destination_parent.directory.as_raw_fd(),
            destination_name.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    )))]
    let status = {
        return Err("atomic no-clobber directory rename is unavailable on this host".to_string());
    };
    if status == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

#[cfg(windows)]
fn rename_child_no_replace(
    source_parent: &DirectoryAnchor,
    source_name: &OsStr,
    destination_parent: &DirectoryAnchor,
    destination_name: &OsStr,
) -> Result<(), String> {
    let source = open_directory_child_for_rename(&source_parent.directory, source_name).map_err(
        |error| {
            format!(
                "failed to bind source directory {}: {error}",
                source_parent.path.join(source_name).display()
            )
        },
    )?;
    rename_open_directory_child_no_replace(source, destination_parent, destination_name)
}

#[cfg(windows)]
fn rename_open_directory_child_no_replace(
    source: File,
    destination_parent: &DirectoryAnchor,
    destination_name: &OsStr,
) -> Result<(), String> {
    let expected_identity = file_identity(&source)
        .map_err(|error| format!("failed to inspect source directory identity: {error}"))?;
    rename_directory_handle_child_no_replace(
        &source,
        &destination_parent.directory,
        destination_name,
    )
    .map_err(|error| error.to_string())?;
    let destination =
        open_directory_child_nofollow(&destination_parent.directory, destination_name).map_err(
            |error| {
                format!(
                    "failed to reopen renamed destination {}: {error}",
                    destination_parent.path.join(destination_name).display()
                )
            },
        )?;
    let actual_identity = file_identity(&destination).map_err(|error| {
        format!(
            "failed to inspect renamed destination identity {}: {error}",
            destination_parent.path.join(destination_name).display()
        )
    })?;
    if actual_identity != expected_identity {
        return Err(format!(
            "renamed destination identity does not match source: {}",
            destination_parent.path.join(destination_name).display()
        ));
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn rename_child_no_replace(
    _source_parent: &DirectoryAnchor,
    _source_name: &OsStr,
    _destination_parent: &DirectoryAnchor,
    _destination_name: &OsStr,
) -> Result<(), String> {
    Err("atomic no-clobber directory rename is unavailable on this host".to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationCheckpoint {
    AfterStagedCapture,
    AfterBackup,
    BeforeStageInstall,
    BeforeSealedPublishRename,
    BeforeInstalledSnapshot,
    AfterStageInstall,
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrivateCleanupCheckpoint {
    BeforeEffectiveConfigScrub,
    BeforeCompartmentRemove,
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrivateCreationCheckpoint {
    AfterExecutionCreation,
    AfterRecoveryCreation,
}

#[cfg(any(test, feature = "test-support"))]
type PublicationHook = (PublicationCheckpoint, Box<dyn FnOnce()>);
#[cfg(any(test, feature = "test-support"))]
type PathHook = Box<dyn FnOnce(&Path)>;
#[cfg(any(test, feature = "test-support"))]
type CleanupOpenHook = Box<dyn FnMut(&Path)>;
#[cfg(all(any(test, feature = "test-support"), unix))]
type DirectoryRenameHook = Box<dyn FnMut(&Path)>;
#[cfg(any(test, feature = "test-support"))]
type TargetedReadHooks = (
    PathBuf,
    Option<Box<dyn FnOnce()>>,
    Option<Box<dyn FnOnce()>>,
);

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static TEST_PUBLICATION_FAILPOINT: RefCell<Option<PublicationCheckpoint>> = const { RefCell::new(None) };
    static TEST_PUBLICATION_HOOK: RefCell<Option<PublicationHook>> = const { RefCell::new(None) };
    #[cfg(unix)]
    static TEST_DIRECTORY_RENAME_HOOK: RefCell<Option<DirectoryRenameHook>> = const { RefCell::new(None) };
    static TEST_SECURE_READ_HOOK: RefCell<Option<PathHook>> = const { RefCell::new(None) };
    static TEST_TARGETED_READ_HOOKS: RefCell<Option<TargetedReadHooks>> = const { RefCell::new(None) };
    static TEST_TREE_OPEN_HOOK: RefCell<Option<PathHook>> = const { RefCell::new(None) };
    static TEST_CLEANUP_OPEN_HOOK: RefCell<Option<CleanupOpenHook>> = const { RefCell::new(None) };
    static TEST_PRIVATE_CREATE_HOOK: RefCell<Option<PathHook>> = const { RefCell::new(None) };
    static TEST_EFFECTIVE_CONFIG_PREPARE_HOOK: RefCell<Option<PathHook>> = const { RefCell::new(None) };
    #[cfg(windows)]
    static TEST_EFFECTIVE_CONFIG_CREATE_HOOK: RefCell<Option<PathHook>> = const { RefCell::new(None) };
    static TEST_TARGET_PARENT_CAPTURE_HOOK: RefCell<Option<PathHook>> = const { RefCell::new(None) };
    #[cfg(windows)]
    static TEST_PRIVATE_CLEANUP_FAILPOINT: RefCell<Option<PrivateCleanupCheckpoint>> = const { RefCell::new(None) };
    #[cfg(windows)]
    static TEST_PRIVATE_CREATION_FAILPOINT: RefCell<Option<PrivateCreationCheckpoint>> = const { RefCell::new(None) };
}

#[cfg(any(test, feature = "test-support"))]
pub fn with_publication_failpoint<T>(
    checkpoint: PublicationCheckpoint,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<PublicationCheckpoint>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_PUBLICATION_FAILPOINT.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let previous = TEST_PUBLICATION_FAILPOINT.with(|slot| slot.replace(Some(checkpoint)));
    let _reset = Reset(previous);
    action()
}

#[cfg(any(test, feature = "test-support"))]
pub fn with_publication_hook<T>(
    checkpoint: PublicationCheckpoint,
    hook: impl FnOnce() + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<(PublicationCheckpoint, Box<dyn FnOnce()>)>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_PUBLICATION_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let previous =
        TEST_PUBLICATION_HOOK.with(|slot| slot.replace(Some((checkpoint, Box::new(hook)))));
    let _reset = Reset(previous);
    action()
}

#[cfg(any(test, feature = "test-support"))]
pub fn with_secure_read_hook<T>(
    hook: impl FnOnce(&Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<PathHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_SECURE_READ_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let previous = TEST_SECURE_READ_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(all(any(test, feature = "test-support"), unix))]
pub fn with_directory_rename_hook<T>(
    hook: impl FnMut(&Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<DirectoryRenameHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_DIRECTORY_RENAME_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let previous = TEST_DIRECTORY_RENAME_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(any(test, feature = "test-support"))]
pub fn with_targeted_read_hooks<T>(
    target: PathBuf,
    before: impl FnOnce() + 'static,
    after: impl FnOnce() + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<TargetedReadHooks>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_TARGETED_READ_HOOKS.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let hooks = (
        target,
        Some(Box::new(before) as Box<dyn FnOnce()>),
        Some(Box::new(after) as Box<dyn FnOnce()>),
    );
    let previous = TEST_TARGETED_READ_HOOKS.with(|slot| slot.replace(Some(hooks)));
    let _reset = Reset(previous);
    action()
}

#[cfg(any(test, feature = "test-support"))]
pub fn with_tree_open_hook<T>(hook: impl FnOnce(&Path) + 'static, action: impl FnOnce() -> T) -> T {
    struct Reset(Option<PathHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_TREE_OPEN_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let previous = TEST_TREE_OPEN_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(any(test, feature = "test-support"))]
pub fn with_directory_cleanup_open_hook<T>(
    hook: impl FnMut(&Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<CleanupOpenHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_CLEANUP_OPEN_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let previous = TEST_CLEANUP_OPEN_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(any(test, feature = "test-support"))]
pub fn with_private_create_hook<T>(
    hook: impl FnOnce(&Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<PathHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_PRIVATE_CREATE_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let previous = TEST_PRIVATE_CREATE_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(all(any(test, feature = "test-support"), windows))]
pub fn with_effective_config_create_hook<T>(
    hook: impl FnOnce(&Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<PathHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_EFFECTIVE_CONFIG_CREATE_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let previous =
        TEST_EFFECTIVE_CONFIG_CREATE_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(any(test, feature = "test-support"))]
pub fn with_effective_config_prepare_hook<T>(
    hook: impl FnOnce(&Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<PathHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_EFFECTIVE_CONFIG_PREPARE_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let previous =
        TEST_EFFECTIVE_CONFIG_PREPARE_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(any(test, feature = "test-support"))]
pub fn with_target_parent_capture_hook<T>(
    hook: impl FnOnce(&Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<PathHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_TARGET_PARENT_CAPTURE_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let previous = TEST_TARGET_PARENT_CAPTURE_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(all(any(test, feature = "test-support"), windows))]
fn with_private_cleanup_failpoint<T>(
    checkpoint: PrivateCleanupCheckpoint,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<PrivateCleanupCheckpoint>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_PRIVATE_CLEANUP_FAILPOINT.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let previous = TEST_PRIVATE_CLEANUP_FAILPOINT.with(|slot| slot.replace(Some(checkpoint)));
    let _reset = Reset(previous);
    action()
}

#[cfg(all(any(test, feature = "test-support"), windows))]
fn with_private_creation_failpoint<T>(
    checkpoint: PrivateCreationCheckpoint,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<PrivateCreationCheckpoint>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_PRIVATE_CREATION_FAILPOINT.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let previous = TEST_PRIVATE_CREATION_FAILPOINT.with(|slot| slot.replace(Some(checkpoint)));
    let _reset = Reset(previous);
    action()
}

#[cfg(windows)]
fn run_private_cleanup_failpoint(checkpoint: PrivateCleanupCheckpoint) -> Result<(), String> {
    #[cfg(any(test, feature = "test-support"))]
    if TEST_PRIVATE_CLEANUP_FAILPOINT.with(|slot| slot.borrow().as_ref() == Some(&checkpoint)) {
        return Err(format!(
            "injected private dump cleanup failure at {checkpoint:?}"
        ));
    }
    let _ = checkpoint;
    Ok(())
}

#[cfg(windows)]
fn run_private_creation_failpoint(checkpoint: PrivateCreationCheckpoint) -> Result<(), String> {
    #[cfg(any(test, feature = "test-support"))]
    if TEST_PRIVATE_CREATION_FAILPOINT.with(|slot| slot.borrow().as_ref() == Some(&checkpoint)) {
        return Err(format!(
            "injected private dump creation failure at {checkpoint:?}"
        ));
    }
    let _ = checkpoint;
    Ok(())
}

fn run_private_create_hook(_path: &Path) {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = TEST_PRIVATE_CREATE_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook(_path);
    }
}

fn run_effective_config_prepare_hook(_path: &Path) {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = TEST_EFFECTIVE_CONFIG_PREPARE_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook(_path);
    }
}

#[cfg(windows)]
fn run_effective_config_create_hook(_path: &Path) {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = TEST_EFFECTIVE_CONFIG_CREATE_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook(_path);
    }
}

fn run_target_parent_capture_hook(_path: &Path) {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = TEST_TARGET_PARENT_CAPTURE_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook(_path);
    }
}

fn run_secure_read_hook(_path: &Path) {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = TEST_SECURE_READ_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook(_path);
    }
    #[cfg(any(test, feature = "test-support"))]
    let hook = TEST_TARGETED_READ_HOOKS.with(|slot| {
        let mut slot = slot.borrow_mut();
        let (target, before, _) = slot.as_mut()?;
        if target == _path {
            before.take()
        } else {
            None
        }
    });
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = hook {
        hook();
    }
}

fn run_secure_read_after_hook(_path: &Path) {
    #[cfg(any(test, feature = "test-support"))]
    let hook = TEST_TARGETED_READ_HOOKS.with(|slot| {
        let mut slot = slot.borrow_mut();
        let (target, _, after) = slot.as_mut()?;
        if target == _path {
            after.take()
        } else {
            None
        }
    });
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = hook {
        hook();
    }
}

fn run_tree_open_hook(_path: &Path) {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = TEST_TREE_OPEN_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook(_path);
    }
}

fn run_directory_cleanup_open_hook(_path: &Path) {
    #[cfg(any(test, feature = "test-support"))]
    TEST_CLEANUP_OPEN_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().as_mut() {
            hook(_path);
        }
    });
}

fn run_publication_hook(checkpoint: PublicationCheckpoint) {
    #[cfg(any(test, feature = "test-support"))]
    {
        let hook = TEST_PUBLICATION_HOOK.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot
                .as_ref()
                .is_some_and(|(expected, _)| *expected == checkpoint)
            {
                slot.take().map(|(_, hook)| hook)
            } else {
                None
            }
        });
        if let Some(hook) = hook {
            hook();
        }
    }
    let _ = checkpoint;
}

#[cfg(unix)]
fn run_directory_rename_hook(_source: &Path) {
    #[cfg(any(test, feature = "test-support"))]
    TEST_DIRECTORY_RENAME_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().as_mut() {
            hook(_source);
        }
    });
}

fn run_publication_failpoint(checkpoint: PublicationCheckpoint) -> Result<(), String> {
    run_publication_hook(checkpoint);
    #[cfg(any(test, feature = "test-support"))]
    if TEST_PUBLICATION_FAILPOINT.with(|slot| slot.borrow().as_ref() == Some(&checkpoint)) {
        return Err(format!(
            "injected verified dump publication failure at {checkpoint:?}"
        ));
    }
    Ok(())
}

fn validate_dump_target(
    target: &Path,
    workspace_root: &Path,
    base_path: &Path,
    work_path: &Path,
) -> Result<(), String> {
    if !target.starts_with(workspace_root) {
        return Err(format!(
            "dump target must stay inside workspace root {}: {}",
            workspace_root.display(),
            target.display()
        ));
    }
    if target == workspace_root {
        return Err(format!(
            "dump target must not equal workspace root: {}",
            target.display()
        ));
    }
    if target == base_path {
        return Err("dump target must not equal project basePath".to_string());
    }
    if target == work_path {
        return Err("dump target must not equal workPath".to_string());
    }
    if target.parent().is_none() || target == Path::new("/") {
        return Err(format!(
            "dump target must not be a filesystem root: {}",
            target.display()
        ));
    }
    Ok(())
}

fn validate_physical_dump_target(
    target: &Path,
    snapshot: &TreeSnapshot,
    kind: SourceSetKind,
    validator: &dyn DumpSemanticValidator,
) -> Result<(), String> {
    let read = BoundDumpTree {
        root: target,
        snapshot,
    };
    validator.validate_existing_target(&DumpTreeObservation::from_read_port(&read), kind)
}

pub struct SerializedDumpProjection {
    primary: Vec<u8>,
    local: Option<Vec<u8>>,
    unreadable_guard: Vec<u8>,
}
impl SerializedDumpProjection {
    pub fn from_serialized(
        primary: Vec<u8>,
        local: Option<Vec<u8>>,
        unreadable_guard: Vec<u8>,
    ) -> Self {
        Self {
            primary,
            local,
            unreadable_guard,
        }
    }
}

pub struct CapturedDumpTree(TreeSnapshot);

pub struct PreparationFailure(DumpPreparationFailure);
impl PreparationFailure {
    pub fn detail(&self) -> &str {
        match &self.0 {
            DumpPreparationFailure::BeforeStage(detail)
            | DumpPreparationFailure::PrivateSetup { detail, .. } => detail,
        }
    }
    pub fn into_canonical_failure(self, phase: FullDumpPhase) -> FullDumpFailure {
        self.0.into_canonical_failure(phase)
    }
    pub fn cleanup_observation(&mut self) -> Option<DumpCleanupObservation> {
        match &mut self.0 {
            DumpPreparationFailure::PrivateSetup { private, .. } => {
                Some(private.cleanup_observation())
            }
            DumpPreparationFailure::BeforeStage(_) => None,
        }
    }
}
impl From<String> for PreparationFailure {
    fn from(error: String) -> Self {
        Self(DumpPreparationFailure::BeforeStage(error))
    }
}
impl From<&str> for PreparationFailure {
    fn from(error: &str) -> Self {
        Self::from(error.to_string())
    }
}
impl std::fmt::Debug for PreparationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

pub struct DumpTargetCapture {
    target: PathBuf,
    workspace_anchor: DirectoryAnchor,
    target_parent_relative: PathBuf,
    target_parent: DirectoryAnchor,
    target_snapshot: TreeSnapshot,
    target_missing_parents: Vec<OsString>,
    config_inputs: Vec<ConfigInputSnapshot>,
    config_store: Option<RunnerProjectStore>,
    selected_source_set: String,
    source_kind: SourceSetKind,
    source_set_name: Option<String>,
    extension: Option<String>,
    semantic_validator: std::sync::Arc<dyn DumpSemanticValidator>,
}
impl DumpTargetCapture {
    #[allow(clippy::too_many_arguments)]
    pub fn capture(
        context: &WorkspaceContext,
        target: PathBuf,
        base_path: &Path,
        work_path: &Path,
        expected_target: Option<&Path>,
        source_kind: SourceSetKind,
        selected_source_set: String,
        source_set_name: Option<String>,
        extension: Option<String>,
        config_inputs: Vec<ConfigInputSnapshot>,
        config_store: Option<RunnerProjectStore>,
        semantic_validator: std::sync::Arc<dyn DumpSemanticValidator>,
    ) -> Result<Self, PreparationFailure> {
        let workspace_root = normalize_path_identity(&context.workspace_root)?;
        let workspace_anchor = DirectoryAnchor::capture_exact(&workspace_root)?;
        let target = normalize_leaf_path(&target)?;
        if let Some(expected) = expected_target {
            if target != normalize_leaf_path(expected)? {
                return Err("configured full dump target differs from the approved preview".into());
            }
        }
        validate_dump_target(&target, &workspace_root, base_path, work_path)?;
        let target_parent_path = target
            .parent()
            .ok_or_else(|| format!("dump target has no parent directory: {}", target.display()))?;
        let target_parent_relative = target_parent_path
            .strip_prefix(&workspace_root)
            .map_err(|_| {
                format!(
                    "dump target parent is not physically addressable below workspace root {}: {}",
                    workspace_root.display(),
                    target_parent_path.display()
                )
            })?
            .to_path_buf();
        run_target_parent_capture_hook(target_parent_path);
        let (target_parent, target_missing_parents) = if config_store.is_some() {
            capture_existing_target_parent(&workspace_anchor, &target_parent_relative)?
        } else {
            (
                workspace_anchor.capture_descendant(&target_parent_relative, target_parent_path)?,
                Vec::new(),
            )
        };
        let target_parent_relative = target_parent
            .path
            .strip_prefix(&workspace_root)
            .map_err(|error| error.to_string())?
            .to_path_buf();
        let target_snapshot = if target_missing_parents.is_empty() {
            target_parent.capture_child(
                target
                    .file_name()
                    .ok_or_else(|| format!("dump target has no name: {}", target.display()))?,
                &target,
            )?
        } else {
            TreeSnapshot::Absent
        };
        workspace_anchor.verify_descendant_identity(
            &target_parent_relative,
            target_parent.identity,
            &target_parent.path,
        )?;
        validate_physical_dump_target(
            &target,
            &target_snapshot,
            source_kind,
            semantic_validator.as_ref(),
        )?;

        Ok(Self {
            target,
            workspace_anchor,
            target_parent_relative,
            target_parent,
            target_snapshot,
            target_missing_parents,
            config_inputs,
            config_store,
            selected_source_set,
            source_kind,
            source_set_name,
            extension,
            semantic_validator,
        })
    }
    pub fn create_private(
        self,
        platform_attestation: Option<PlatformAttestation>,
    ) -> Result<UnconfiguredDump, PreparationFailure> {
        let private = PrivateDumpStage::create(&self.target_parent)?;
        Ok(UnconfiguredDump {
            prepared: PreparedPublication {
                semantic_validator: self.semantic_validator,
                canonical_stage_anchor: None,
                canonical_stage_identity: None,
                private,
                target: self.target,
                workspace_anchor: self.workspace_anchor,
                target_parent_relative: self.target_parent_relative,
                target_parent: self.target_parent,
                target_snapshot: self.target_snapshot,
                platform_attestation,
                target_missing_parents: self.target_missing_parents,
                opened_target_parent: None,
                created_target_parents: Vec::new(),
                config_inputs: self.config_inputs,
                config_store: self.config_store,
                selected_source_set: self.selected_source_set,
                source_kind: self.source_kind,
                source_set_name: self.source_set_name,
                extension: self.extension,
            },
        })
    }
}

pub struct UnconfiguredDump {
    prepared: PreparedPublication,
}
impl UnconfiguredDump {
    pub fn refuse_setup(mut self, detail: String) -> PreparationFailure {
        self.prepared.canonical_stage_anchor.take();
        PreparationFailure(DumpPreparationFailure::PrivateSetup {
            detail,
            private: Box::new(self.prepared.private),
        })
    }

    pub fn staged_target(&self) -> &Path {
        &self.prepared.private.staged_tree
    }
    pub fn configure(
        mut self,
        projection: SerializedDumpProjection,
    ) -> Result<PreparedPublication, PreparationFailure> {
        let setup = (|| {
            if let Some(store) = self.prepared.config_store.as_mut() {
                store.write_serialized_projection(&projection)?;
                store.verify()?;
                let created_stage = self.prepared.private.execution_anchor().create_child(
                    OsStr::new("staged-source"),
                    &self.prepared.private.staged_tree,
                )?;
                self.prepared
                    .private
                    .remember_staged_tree(&TreeSnapshot::Directory {
                        identity: created_stage.identity,
                        entries: Vec::new(),
                    });
                self.prepared.canonical_stage_identity = Some(created_stage.identity);
                self.prepared.canonical_stage_anchor = Some(created_stage);
            } else {
                self.prepared
                    .private
                    .write_effective_config(&projection.primary)?;
            }
            Ok(())
        })();
        if let Err(detail) = setup {
            self.prepared.canonical_stage_anchor.take();
            return Err(PreparationFailure(DumpPreparationFailure::PrivateSetup {
                detail,
                private: Box::new(self.prepared.private),
            }));
        }
        Ok(self.prepared)
    }
}

/// Immutable late cleanup facts. The publisher never accepts these as input.
pub struct DumpCleanupObservation {
    pub cleanup_error: Option<String>,
    pub effective_config_secret_present: bool,
    pub publication_uncertain: bool,
    pub retained: bool,
    pub entire_stage: bool,
    pub root: PathBuf,
    pub recovery: PathBuf,
    pub cleanup_error_location_proof: Result<(), String>,
    pub retention_location_proof: Result<(), String>,
}
impl PrivateDumpStage {
    fn cleanup_observation(&mut self) -> DumpCleanupObservation {
        let cleanup_error = self.cleanup_now().err();
        let cleanup_error_location_proof = if cleanup_error.is_some() {
            self.bound_root_for_reporting().map(drop)
        } else {
            Ok(())
        };
        let retained = self.retention != PrivateRetention::None || self.publication_uncertain;
        let retention_location_proof = if retained {
            if self.retention == PrivateRetention::EntireStage {
                self.bound_root_for_reporting().map(drop)
            } else {
                self.verify_recovery_location()
            }
        } else {
            Ok(())
        };
        DumpCleanupObservation {
            cleanup_error,
            effective_config_secret_present: self.effective_config_secret_present,
            publication_uncertain: self.publication_uncertain,
            retained,
            entire_stage: self.retention == PrivateRetention::EntireStage,
            root: self.root.clone(),
            recovery: self.recovery.clone(),
            cleanup_error_location_proof,
            retention_location_proof,
        }
    }
}
impl PreparedPublication {
    pub fn target(&self) -> &Path {
        &self.target
    }
    pub fn staged_target(&self) -> &Path {
        &self.private.staged_tree
    }
    pub fn effective_config(&self) -> &Path {
        &self.private.effective_config
    }
    pub fn explicit_source_set(&self) -> Option<&str> {
        self.source_set_name.as_deref()
    }
    pub fn extension(&self) -> Option<&str> {
        self.extension.as_deref()
    }
    pub fn begin_execution(&mut self) {
        self.private.begin_execution();
    }
    pub fn confirm_execution_finished(&mut self) {
        self.private.confirm_execution_finished();
    }
    pub fn capture_finished_staged_tree(&mut self) -> Result<CapturedDumpTree, String> {
        self.private
            .capture_finished_staged_tree()
            .map(CapturedDumpTree)
    }
    pub fn remove_effective_config(&mut self) -> Result<(), String> {
        self.private.remove_effective_config()
    }
    pub fn validate_captured(&self, captured: &CapturedDumpTree) -> Result<(), String> {
        run_publication_hook(PublicationCheckpoint::AfterStagedCapture);
        validate_staged_dump_snapshot(
            &self.private.staged_tree,
            self.source_kind,
            &captured.0,
            self.semantic_validator.as_ref(),
        )
        .map_err(|error| error.to_string())
    }
    pub fn publish_legacy(
        &mut self,
        captured: &CapturedDumpTree,
        cancellation: &CancellationToken,
    ) -> Result<Vec<String>, String> {
        self.publish(&captured.0, cancellation)
    }
    pub fn cleanup_observation(&mut self) -> DumpCleanupObservation {
        self.private.cleanup_observation()
    }
    pub fn recheck_selected_platform(&self) -> Result<(), String> {
        self.recheck_platform()
    }
}
impl PlatformAttestation {
    pub fn executable_path(&self) -> &Path {
        &self.executable_path
    }
}

#[cfg(any(test, feature = "test-support"))]
pub struct PublicationObservation {
    pub private_root: PathBuf,
    pub execution: PathBuf,
    pub writer_completion_pending: bool,
    pub canonical_stage_identity: Option<FileIdentity>,
    pub execution_identity: FileIdentity,
    pub opened_target_parent_identity: Option<FileIdentity>,
    pub created_target_parents: usize,
}
#[cfg(any(test, feature = "test-support"))]
impl VerifiedFullDumpPublication {
    pub fn test_observation(&self) -> PublicationObservation {
        PublicationObservation {
            private_root: self.prepared.private.root.clone(),
            execution: self.prepared.private.execution.clone(),
            writer_completion_pending: self.prepared.private.writer_completion_pending,
            canonical_stage_identity: self
                .prepared
                .canonical_stage_anchor
                .as_ref()
                .map(|a| a.identity),
            execution_identity: self.prepared.private.execution_anchor().identity,
            opened_target_parent_identity: self
                .prepared
                .opened_target_parent
                .as_ref()
                .map(|a| a.identity),
            created_target_parents: self.prepared.created_target_parents.len(),
        }
    }
    /// Explicit fixture ownership registration. No production path accepts a caller snapshot.
    pub fn test_remember_staged_fixture(&mut self, snapshot: &TreeSnapshot) {
        self.prepared.private.remember_staged_tree(snapshot);
    }
    pub fn test_assert_held_stage_binding(&self) {
        let retained = self
            .prepared
            .canonical_stage_anchor
            .as_ref()
            .expect("retained fixture stage");
        verify_private_owner(&retained.directory).unwrap();
        assert_eq!(
            self.prepared
                .private
                .execution_anchor()
                .capture_child_root_identity(OsStr::new("staged-source"), self.staged_target())
                .unwrap(),
            Some(retained.identity)
        );
    }
}
#[cfg(any(test, feature = "test-support"))]
pub fn fixture_file_identity(file: &File) -> std::io::Result<FileIdentity> {
    file_identity(file)
}
#[cfg(any(test, feature = "test-support"))]
pub fn fixture_verify_private_owner(file: &File) -> std::io::Result<()> {
    verify_private_owner(file)
}
#[cfg(any(test, feature = "test-support"))]
pub fn inspect_and_validate_stage(
    root: &Path,
    kind: SourceSetKind,
    validator: &dyn DumpSemanticValidator,
) -> Result<TreeSnapshot, String> {
    let snapshot = TreeSnapshot::capture_target(root)?;
    validate_staged_dump_snapshot(root, kind, &snapshot, validator)
        .map_err(|error| error.to_string())?;
    Ok(snapshot)
}

#[cfg(any(test, feature = "test-support"))]
pub mod retained_recovery_contract {
    use std::path::PathBuf;
    use unica_source_contract::workspace::WorkspaceContext;
    pub fn run(
        workspace: impl Fn(&str) -> (PathBuf, WorkspaceContext, PathBuf),
        initial_outcome: impl Fn() -> unica_execution_contract::adapter_outcome::AdapterOutcome,
        finalize: impl Fn(
            super::DumpCleanupObservation,
            unica_execution_contract::adapter_outcome::AdapterOutcome,
        ) -> unica_execution_contract::adapter_outcome::AdapterOutcome,
    ) {
        let mut invalid_reports = Vec::new();
        for case in [
            "stable",
            "stable-closed-anchors",
            "root-missing",
            "root-replaced",
            "recovery-missing",
            "recovery-replaced",
        ] {
            let (root, context, _) = workspace("retained-location");
            let parent = super::DirectoryAnchor::capture_exact(
                &std::fs::canonicalize(&context.cwd).unwrap(),
            )
            .unwrap();
            let mut private = super::PrivateDumpStage::create(&parent).unwrap();
            let owned_child = private.recovery.join("owned");
            std::fs::create_dir(&owned_child).unwrap();
            std::fs::write(owned_child.join("owned.txt"), b"owned recovery sentinel").unwrap();
            private.recovery_children.insert(
                std::ffi::OsString::from("owned"),
                super::capture_tree_target_nofollow(&owned_child).unwrap(),
            );
            private.preserve_for_recovery().unwrap();
            let owned_before = super::capture_tree_target_nofollow(&private.recovery).unwrap();
            let owned_identity =
                super::file_identity(&private.recovery_anchor().directory).unwrap();
            let mut actual_recovery = private.recovery.clone();
            let mut foreign = None;
            if case.starts_with("root-") {
                let displaced = context.cwd.join("displaced-root");
                #[cfg(windows)]
                {
                    // Windows blocks an ancestor rename while a descendant directory is open.
                    // Keep the root anchor; release only the recovery descendant after proving it.
                    let error = std::fs::rename(&private.root, &displaced)
                        .expect_err("an open recovery descendant must prevent the root rename");
                    assert_eq!(
                        error.kind(),
                        std::io::ErrorKind::PermissionDenied,
                        "{case}: unexpected error while the recovery descendant is open: {error}"
                    );
                    drop(
                        private
                            .recovery_anchor
                            .take()
                            .expect("the recovery descendant is retained before the rename"),
                    );
                }
                std::fs::rename(&private.root, &displaced).unwrap();
                actual_recovery = displaced.join("recovery");
                if case == "root-replaced" {
                    std::fs::create_dir(&private.root).unwrap();
                    std::fs::create_dir(&private.recovery).unwrap();
                    std::fs::write(
                        private.recovery.join("foreign.txt"),
                        b"foreign root sentinel",
                    )
                    .unwrap();
                    foreign = Some((
                        private.root.clone(),
                        super::capture_tree_target_nofollow(&private.root).unwrap(),
                    ));
                }
            } else if case.starts_with("recovery-") {
                actual_recovery = private.root.join("displaced-recovery");
                std::fs::rename(&private.recovery, &actual_recovery).unwrap();
                if case == "recovery-replaced" {
                    std::fs::create_dir(&private.recovery).unwrap();
                    std::fs::write(
                        private.recovery.join("foreign.txt"),
                        b"foreign recovery sentinel",
                    )
                    .unwrap();
                    foreign = Some((
                        private.recovery.clone(),
                        super::capture_tree_target_nofollow(&private.recovery).unwrap(),
                    ));
                }
            } else if case == "stable-closed-anchors" {
                private.root_anchor.take();
                private.recovery_anchor.take();
            }
            let result = finalize(private.cleanup_observation(), initial_outcome());
            drop(private);
            assert_eq!(
                super::capture_tree_target_nofollow(&actual_recovery).unwrap(),
                owned_before,
                "{case}: reporting and Drop must preserve owned identities and bytes"
            );
            assert_eq!(
                std::fs::read(actual_recovery.join("owned/owned.txt")).unwrap(),
                b"owned recovery sentinel"
            );
            if let Some((path, before)) = foreign {
                assert_eq!(
                    super::capture_tree_target_nofollow(&path).unwrap(),
                    before,
                    "{case}: reporting and Drop must preserve foreign identities and bytes"
                );
            }
            let owned_artifacts = result.artifacts.iter().all(|artifact| {
                std::fs::canonicalize(artifact)
                    .ok()
                    .and_then(|path| super::open_directory_nofollow(&path).ok())
                    .and_then(|directory| super::file_identity(&directory).ok())
                    == Some(owned_identity)
            });
            let location_unconfirmed = result
                .warnings
                .iter()
                .any(|warning| warning.contains("current location is unconfirmed"));
            let stable = case.starts_with("stable");
            if !owned_artifacts
                || (result.artifacts.is_empty() && !location_unconfirmed)
                || (stable && (result.artifacts.len() != 1 || location_unconfirmed))
            {
                invalid_reports.push(case);
            }
            drop(parent);
            std::fs::remove_dir_all(root).unwrap();
        }
        assert!(
            invalid_reports.is_empty(),
            "invalid retained locations: {invalid_reports:?}"
        );
    }
}

/// A failed Windows scrub of a pending writer's effective configuration keeps
/// the whole private stage, reports it through the caller's outcome mapping
/// and stays retryable without unlinking writer data.
#[cfg(all(any(test, feature = "test-support"), windows))]
pub mod pending_writer_scrub_contract {
    use super::{
        capture_tree_target_nofollow, file_identity, open_directory_nofollow,
        with_private_cleanup_failpoint, DirectoryAnchor, DumpCleanupObservation,
        PrivateCleanupCheckpoint, PrivateDumpStage, PrivateRetention,
    };
    use std::fs;
    use unica_execution_contract::adapter_outcome::AdapterOutcome;

    /// `initial_outcome` describes a dump that failed before publication.
    pub fn run(
        initial_outcome: impl Fn() -> AdapterOutcome,
        finalize: impl Fn(DumpCleanupObservation, AdapterOutcome) -> AdapterOutcome,
    ) {
        for cleanup_via_drop in [false, true] {
            let root = std::env::temp_dir().join(format!(
                "unica-full-dump-pending-writer-scrub-retry-{}",
                uuid::Uuid::new_v4()
            ));
            let workspace = root.join("workspace");
            fs::create_dir_all(&workspace).unwrap();
            let parent = DirectoryAnchor::capture_exact(&workspace).unwrap();
            let mut stage = PrivateDumpStage::create(&parent).unwrap();
            stage.write_effective_config(b"secret\n").unwrap();
            fs::create_dir(&stage.staged_tree).unwrap();
            fs::write(
                stage.staged_tree.join("writer-data.txt"),
                b"unfinished writer data",
            )
            .unwrap();
            let staged_before = capture_tree_target_nofollow(&stage.staged_tree).unwrap();
            stage.remember_staged_tree(&staged_before);
            stage.begin_execution();
            let private_root = stage.root.clone();
            let config_path = stage.effective_config.clone();
            let stage_path = stage.staged_tree.clone();
            let root_identity = stage.root_identity;
            let execution_identity = stage.execution_identity;
            let recovery_identity = stage.recovery_identity;
            let config_identity =
                file_identity(stage.effective_config_handle.as_ref().unwrap()).unwrap();
            let before = capture_tree_target_nofollow(&private_root).unwrap();

            let outcome = with_private_cleanup_failpoint(
                PrivateCleanupCheckpoint::BeforeEffectiveConfigScrub,
                || finalize(stage.cleanup_observation(), initial_outcome()),
            );
            assert!(!outcome.ok);
            assert!(outcome
                .errors
                .iter()
                .any(|error| error.contains("injected")));
            assert!(outcome
                .warnings
                .iter()
                .any(|warning| warning.contains("Secret-bearing")));
            assert_eq!(stage.retention, PrivateRetention::EntireStage);
            assert!(stage.cleanup_on_drop, "failed scrub must remain retryable");
            assert!(stage.effective_config_secret_present);
            assert!(stage.effective_config_handle.is_some());
            assert_eq!(capture_tree_target_nofollow(&private_root).unwrap(), before);
            assert!(fs::read(&config_path)
                .unwrap()
                .windows(6)
                .any(|part| part == b"secret"));

            if !cleanup_via_drop {
                let retry =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| stage.cleanup_now()));
                retry
                    .expect("pending cleanup retry must not panic")
                    .unwrap();
                assert!(!stage.effective_config_secret_present);
                assert!(!stage.cleanup_on_drop);
            }
            let dropped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(stage)));
            assert!(
                dropped.is_ok(),
                "Drop must retry the held scrub without panicking"
            );
            assert!(
                fs::read(&config_path).unwrap().is_empty(),
                "held configuration must be empty"
            );
            let config = fs::File::open(&config_path).unwrap();
            assert_eq!(file_identity(&config).unwrap(), config_identity);
            drop(config);
            for (path, expected) in [
                (&private_root, root_identity),
                (&private_root.join("execution"), execution_identity),
                (&private_root.join("recovery"), recovery_identity),
            ] {
                let retained = open_directory_nofollow(path).unwrap();
                assert_eq!(file_identity(&retained).unwrap(), expected);
            }
            assert_eq!(
                capture_tree_target_nofollow(&stage_path).unwrap(),
                staged_before
            );
            drop(parent);
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl RunnerProjectStore {
    pub fn test_write_config_bytes(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        self.write_config(name, bytes)
    }
    pub fn test_remove_config(&mut self, name: &str) -> Result<(), String> {
        self.remove_config(name)
    }
}
#[cfg(any(test, feature = "test-support"))]
pub fn set_before_fixture_config_rename_hook(hook: impl FnOnce() + 'static) {
    crate::platform::filesystem::set_before_identity_bound_no_replace_rename_hook(hook)
}

#[cfg(any(test, feature = "test-support"))]
impl PreparedPublication {
    pub fn test_has_canonical_stage(&self) -> bool {
        self.canonical_stage_anchor.is_some()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl FullDumpFailure {
    pub fn test_detail(&self) -> &str {
        &self.detail
    }
}

/// The existing host branch classes used by v8 filename and installation policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostPlatform {
    Windows,
    Linux,
    Other,
}

pub fn current_host_platform() -> HostPlatform {
    #[cfg(windows)]
    {
        HostPlatform::Windows
    }
    #[cfg(target_os = "linux")]
    {
        HostPlatform::Linux
    }
    #[cfg(all(not(windows), not(target_os = "linux")))]
    {
        HostPlatform::Other
    }
}

/// An observation of POSIX permission bits, not an execution/publication permit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutablePermissionObservation {
    NotApplicable,
    Executable,
    NotExecutable,
}

pub fn observe_executable_permissions(metadata: &fs::Metadata) -> ExecutablePermissionObservation {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            ExecutablePermissionObservation::NotExecutable
        } else {
            ExecutablePermissionObservation::Executable
        }
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        ExecutablePermissionObservation::NotApplicable
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support_os {
    #[cfg(unix)]
    pub use super::with_directory_rename_hook;
    #[cfg(windows)]
    pub use super::{
        pending_writer_scrub_contract::run as pending_writer_scrub_contract,
        with_effective_config_create_hook,
    };
}
