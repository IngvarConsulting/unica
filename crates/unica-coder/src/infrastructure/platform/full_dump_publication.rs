//! Guarded publication for synchronous full configuration dumps.
//!
//! The legacy adapter redirects its effective configuration into a private stage.
//! Canonical full pull keeps the original configured source identity and supplies
//! that stage through the runner publication protocol. It attests the actual
//! selected local executable before authorizing it, then validates and publishes
//! the held tree while retaining the project store until the runner is terminal.

#![cfg_attr(windows, allow(dead_code))]

use crate::application::AdapterOutcome;
use crate::domain::cancellation::CancellationToken;
use crate::domain::format_profile::ACTIVE_FORMAT_PROFILE;
use crate::domain::project_sources::{SourceFormat, SourceSetKind};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::bundled_tools::resolve_bundled_tool;
use crate::infrastructure::internal_adapters::{
    system_process_runner, ProcessCommand, ProcessOutput, ProcessRunner, ProcessStreamFailure,
};
use crate::infrastructure::native_operations::single_file_publisher::{
    with_publication_locks_mode, PublicationTreeLockMode,
};
#[cfg(any(unix, windows))]
use crate::infrastructure::platform::filesystem::hard_link_count;
#[cfg(windows)]
use crate::infrastructure::platform::filesystem::{
    capture_windows_immutable_entry_evidence, create_owner_only_directory_child,
    create_owner_only_file_child, delete_open_child, discard_created_child,
    open_any_child_for_delete, open_any_child_nofollow, open_directory_child_for_rename,
    open_directory_child_nofollow, open_directory_nofollow, open_regular_child_nofollow,
    opened_child_kind, rename_directory_handle_child_no_replace, verify_owner_only_acl,
    verify_unprivileged_windows_platform_caller, verify_windows_local_fixed_volume,
    OpenedChildKind, WindowsImmutableAclProfile,
};
use crate::infrastructure::platform::filesystem::{
    file_identity, metadata_is_link_or_reparse_point, restrict_stage_to_owner, FileIdentity,
};
use crate::infrastructure::platform_xml_owner::root_version_literal;
use crate::infrastructure::plugin_runtime::find_plugin_root;
use crate::infrastructure::project_sources::classify_physical_source_inventory;
use crate::infrastructure::redaction::redactor;
use crate::infrastructure::source_roots::normalize_path_identity;
use roxmltree::Document;
use serde_json::{Map, Value};
use serde_yaml::{Mapping as YamlMapping, Value as YamlValue};
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::env;
#[cfg(unix)]
use std::ffi::CString;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
#[cfg(any(unix, windows))]
use std::io::Read;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt, OsStringExt};

const TARGET_PLATFORM_LINE: &str = ACTIVE_FORMAT_PROFILE.platform_line;
const TARGET_EXPORT_FORMAT: &str = ACTIVE_FORMAT_PROFILE.export_format;
const EFFECTIVE_CONFIG_NAME: &str = "v8project.yaml";
const LOCAL_CONFIG_NAME: &str = "v8project.local.yaml";
const MD_CLASSES_NS: &str = "http://v8.1c.ru/8.3/MDClasses";
const BUILD_DUMP_TIMEOUT: Duration = Duration::from_secs(120);
const PLATFORM_PROBE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FullDumpInvocation {
    BuildDump,
    RuntimeExecute,
}

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
    use crate::infrastructure::platform::filesystem::{
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
    fn windows_pending_writer_scrub_failure_preserves_data_and_retries_without_unlink() {
        for cleanup_via_drop in [false, true] {
            let root = unique_temp_root("pending-writer-scrub-retry");
            let workspace = root.join("workspace");
            fs::create_dir_all(&workspace).unwrap();
            let parent = DirectoryAnchor::capture_exact(&workspace).unwrap();
            let mut stage = PrivateDumpStage::create(&parent).unwrap();
            stage
                .write_effective_config(&serde_yaml::Value::String("secret".to_string()))
                .unwrap();
            fs::create_dir(&stage.staged_tree).unwrap();
            fs::write(
                stage.staged_tree.join("writer-data.txt"),
                b"unfinished writer data",
            )
            .unwrap();
            let staged_before = super::capture_tree_target_nofollow(&stage.staged_tree).unwrap();
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
            let before = super::capture_tree_target_nofollow(&private_root).unwrap();

            let outcome = with_private_cleanup_failpoint(
                PrivateCleanupCheckpoint::BeforeEffectiveConfigScrub,
                || {
                    super::finalize_private_outcome(
                        &mut stage,
                        super::dump_failure(
                            "unica.build.dump",
                            "runner completion was not confirmed".into(),
                            None,
                            None,
                            None,
                        ),
                    )
                },
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
            assert_eq!(stage.retention, super::PrivateRetention::EntireStage);
            assert!(stage.cleanup_on_drop, "failed scrub must remain retryable");
            assert!(stage.effective_config_secret_present);
            assert!(stage.effective_config_handle.is_some());
            assert_eq!(
                super::capture_tree_target_nofollow(&private_root).unwrap(),
                before
            );
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
                super::capture_tree_target_nofollow(&stage_path).unwrap(),
                staged_before
            );
            drop(parent);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn windows_effective_config_scrub_failure_keeps_cleanup_retry_and_drop_safe() {
        let root = unique_temp_root("private-stage-scrub-cleanup-retry");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let target_parent = DirectoryAnchor::capture_exact(&workspace).unwrap();
        let mut stage = PrivateDumpStage::create(&target_parent).unwrap();
        stage
            .write_effective_config(&serde_yaml::Value::String("secret".to_string()))
            .unwrap();
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
        stage
            .write_effective_config(&serde_yaml::Value::String("secret".to_string()))
            .unwrap();
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
        stage
            .write_effective_config(&serde_yaml::Value::String("secret".to_string()))
            .unwrap();
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlatformUtility {
    Designer,
    Ibcmd,
}

impl PlatformUtility {
    fn executable_name(self) -> &'static str {
        #[cfg(windows)]
        {
            match self {
                Self::Designer => "1cv8.exe",
                Self::Ibcmd => "ibcmd.exe",
            }
        }
        #[cfg(not(windows))]
        {
            match self {
                Self::Designer => "1cv8",
                Self::Ibcmd => "ibcmd",
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VerifiedPlatform {
    executable: PathBuf,
    exact_version: String,
    attestation: PlatformAttestation,
}

trait PlatformResolver {
    fn resolve(
        &self,
        effective_config: &YamlValue,
        config_dir: &Path,
        utility: PlatformUtility,
        runner: &dyn ProcessRunner,
        cancellation: &CancellationToken,
    ) -> Result<VerifiedPlatform, String>;
}

struct SystemPlatformResolver;
static SYSTEM_PLATFORM_RESOLVER: SystemPlatformResolver = SystemPlatformResolver;

impl PlatformResolver for SystemPlatformResolver {
    fn resolve(
        &self,
        effective_config: &YamlValue,
        config_dir: &Path,
        utility: PlatformUtility,
        runner: &dyn ProcessRunner,
        cancellation: &CancellationToken,
    ) -> Result<VerifiedPlatform, String> {
        #[cfg(test)]
        if let Some((executable, exact_version)) =
            TEST_PLATFORM_OVERRIDE.with(|slot| slot.borrow().clone())
        {
            return Ok(VerifiedPlatform {
                attestation: PlatformAttestation::capture_test_fixture(&executable)?,
                executable,
                exact_version,
            });
        }
        if let Some(configured_version) =
            nested_yaml_string(effective_config, &["tools", "platform", "version"])?
        {
            validate_configured_platform_line(configured_version)?;
        }

        let configured_hint = nested_yaml_string(effective_config, &["tools", "platform", "path"])?
            .map(|path| absolutize(Path::new(path), config_dir));
        let candidates = if let Some(hint) = configured_hint {
            platform_candidates_from_hint(&hint, utility).map_err(|error| {
                format!(
                    "configured tools.platform.path cannot prove platform {TARGET_PLATFORM_LINE}: {error}"
                )
            })?
        } else {
            default_platform_candidates(utility)
        };

        let mut verified = Vec::new();
        let mut last_rejection = None;
        for candidate in candidates {
            match verify_platform_candidate(&candidate, utility, runner, cancellation) {
                Ok(candidate) => verified.push(candidate),
                Err(error) => last_rejection = Some(error),
            }
        }
        verified.sort_by_key(|candidate| {
            parse_exact_platform_version(&candidate.exact_version)
                .expect("verified platform always carries a four-part version")
        });
        verified.pop().ok_or_else(|| {
            format!(
                "could not resolve an immutable trusted executable for platform {TARGET_PLATFORM_LINE} ({}) with an exact four-part version in its installation path{}",
                utility.executable_name(),
                last_rejection
                    .map(|error| format!("; last rejected candidate: {error}"))
                    .unwrap_or_default()
            )
        })
    }
}

impl SystemPlatformResolver {
    fn attest_selected_local(
        executable: &Path,
        runner: &dyn ProcessRunner,
        cancellation: &CancellationToken,
        probe_cancelled: &mut bool,
    ) -> Result<VerifiedPlatform, String> {
        if !executable.is_absolute() {
            return Err("selected local platform executable must be absolute".into());
        }
        #[cfg(test)]
        if let Some((fixture, exact_version)) =
            TEST_PLATFORM_OVERRIDE.with(|slot| slot.borrow().clone())
        {
            if normalize_leaf_path(executable)? != normalize_leaf_path(&fixture)? {
                return Err(
                    "selected local executable differs from the test platform receipt".into(),
                );
            }
            let version = parse_exact_platform_version(&exact_version)
                .ok_or("selected local platform version is not exact")?;
            if version[..3] != [8, 3, 27] {
                return Err("selected local platform must be 8.3.27.x".into());
            }
            return Ok(VerifiedPlatform {
                attestation: PlatformAttestation::capture_test_fixture(executable)?,
                executable: executable.to_path_buf(),
                exact_version,
            });
        }
        let utility = [PlatformUtility::Designer, PlatformUtility::Ibcmd]
            .into_iter()
            .find(|utility| executable.file_name() == Some(OsStr::new(utility.executable_name())))
            .ok_or("selected local provider has an unsupported executable name")?;
        verify_platform_candidate_recording_cancellation(
            executable,
            utility,
            runner,
            cancellation,
            Some(probe_cancelled),
        )
    }
}

pub(crate) struct VerifiedFullDumpAdapter<'a> {
    runner: &'a dyn ProcessRunner,
    platform_resolver: &'a dyn PlatformResolver,
    bundled_program_override: Option<PathBuf>,
}

impl VerifiedFullDumpAdapter<'static> {
    pub(crate) fn new() -> Self {
        Self {
            runner: system_process_runner(),
            platform_resolver: &SYSTEM_PLATFORM_RESOLVER,
            bundled_program_override: None,
        }
    }
}

impl<'a> VerifiedFullDumpAdapter<'a> {
    #[cfg(test)]
    fn with_dependencies(
        runner: &'a dyn ProcessRunner,
        platform_resolver: &'a dyn PlatformResolver,
        bundled_program: PathBuf,
    ) -> Self {
        Self {
            runner,
            platform_resolver,
            bundled_program_override: Some(bundled_program),
        }
    }

    pub(crate) fn invoke(
        &self,
        tool_name: &str,
        invocation: FullDumpInvocation,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<AdapterOutcome, String> {
        if cancellation.is_cancelled() {
            return Ok(AdapterOutcome::cancelled(format!(
                "{tool_name} cancelled before verified dump preparation"
            )));
        }

        let mut prepared = match PreparedDump::prepare(
            args,
            context,
            self.platform_resolver,
            self.runner,
            cancellation,
        ) {
            Ok(prepared) => prepared,
            Err(DumpPreparationFailure::BeforeStage(error)) => {
                return Ok(dump_failure(tool_name, error, None, None, None));
            }
            Err(DumpPreparationFailure::PrivateSetup {
                detail,
                mut private,
            }) => {
                return Ok(finalize_private_outcome(
                    &mut private,
                    dump_failure(tool_name, detail, None, None, None),
                ));
            }
        };
        macro_rules! finish_prepared {
            ($outcome:expr) => {
                return Ok(finalize_private_outcome(&mut prepared.private, $outcome))
            };
        }
        let bundled = if let Some(program) = &self.bundled_program_override {
            (program.clone(), Vec::new())
        } else {
            let Some(plugin_root) = find_plugin_root(&context.cwd) else {
                finish_prepared!(dump_failure(
                    tool_name,
                    "could not locate Unica plugin root for internal adapter lookup".to_string(),
                    None,
                    None,
                    None,
                ));
            };
            let bundled = match resolve_bundled_tool(&plugin_root, "v8-runner", true) {
                Ok(bundled) => bundled,
                Err(error) => {
                    finish_prepared!(dump_failure(tool_name, error.into(), None, None, None));
                }
            };
            (bundled.program, bundled.warnings)
        };

        let execution_args = dump_process_args(
            invocation,
            &prepared.private.effective_config,
            prepared.source_set_name.as_deref(),
            prepared.extension.as_deref(),
        );
        let report_args = reported_dump_process_args(
            invocation,
            prepared.source_set_name.as_deref(),
            prepared.extension.as_deref(),
        );
        let mut reported_command = vec![bundled.0.display().to_string()];
        reported_command.extend(report_args);
        if let Err(error) = prepared.recheck_platform() {
            finish_prepared!(dump_failure(
                tool_name,
                format!("platform trust check immediately before v8-runner failed: {error}"),
                None,
                None,
                Some(reported_command),
            ));
        }
        prepared.private.begin_execution();
        let output_result = self.runner.run_staged(&ProcessCommand {
            program: bundled.0,
            args: execution_args,
            cwd: context.cwd.clone(),
            env: Vec::new(),
            env_remove: Vec::new(),
            capture_limits: None,
            timeout: match invocation {
                FullDumpInvocation::BuildDump => Some(BUILD_DUMP_TIMEOUT),
                FullDumpInvocation::RuntimeExecute => None,
            },
            cancellation: cancellation.clone(),
        });
        if matches!(&output_result, Err(ProcessStreamFailure::Spawn(_)))
            || output_result
                .as_ref()
                .is_ok_and(|output| output.status_success && !output.timed_out && !output.cancelled)
        {
            prepared.private.confirm_execution_finished();
        }
        let staged_capture = if output_result
            .as_ref()
            .is_ok_and(|output| output.status_success && !output.timed_out && !output.cancelled)
        {
            prepared.private.capture_finished_staged_tree().map(Some)
        } else {
            Ok(None)
        };
        let output_result = output_result.map_err(|failure| failure.message().to_string());
        let platform_recheck = prepared.recheck_platform();
        let config_cleanup = prepared.private.remove_effective_config();
        let (output, staged_snapshot) = match (
            output_result,
            staged_capture,
            platform_recheck,
            config_cleanup,
        ) {
            (Ok(output), Ok(snapshot), Ok(()), Ok(())) => (output, snapshot),
            (output_result, staged_capture, platform_recheck, config_cleanup) => {
                let mut errors = Vec::new();
                if let Err(error) = output_result {
                    errors.push(redactor(&error));
                }
                if let Err(error) = staged_capture {
                    errors.push(redactor(&error));
                }
                if let Err(error) = platform_recheck {
                    errors.push(format!(
                        "platform trust check immediately after v8-runner failed: {error}"
                    ));
                }
                if let Err(error) = config_cleanup {
                    errors.push(error);
                }
                finish_prepared!(dump_failure(
                    tool_name,
                    errors.join("; "),
                    None,
                    None,
                    Some(reported_command),
                ));
            }
        };
        if output.cancelled || cancellation.is_cancelled() {
            finish_prepared!(cancelled_dump_outcome(tool_name, &output, reported_command));
        }
        if !output.status_success {
            let error = if output.stderr.trim().is_empty() {
                format!(
                    "internal v8-runner verified dump exited with status {}",
                    output.status
                )
            } else {
                redactor(output.stderr.trim())
            };
            finish_prepared!(dump_failure(
                tool_name,
                error,
                Some(redactor(&output.stdout)),
                Some(redactor(&output.stderr)),
                Some(reported_command),
            ));
        }

        let Some(staged_snapshot) = staged_snapshot else {
            finish_prepared!(dump_failure(
                tool_name,
                "v8-runner did not confirm successful completion of the staged dump".to_string(),
                Some(redactor(&output.stdout)),
                Some(redactor(&output.stderr)),
                Some(reported_command),
            ));
        };
        run_publication_hook(PublicationCheckpoint::AfterStagedCapture);
        if let Err(error) = validate_staged_dump_snapshot(
            &prepared.private.staged_tree,
            prepared.source_kind,
            &staged_snapshot,
        ) {
            finish_prepared!(dump_failure(
                tool_name,
                error,
                Some(redactor(&output.stdout)),
                Some(redactor(&output.stderr)),
                Some(reported_command),
            ));
        }
        if cancellation.is_cancelled() {
            finish_prepared!(cancelled_dump_outcome(tool_name, &output, reported_command));
        }

        let target = prepared.target.clone();
        let publication = prepared.publish(&staged_snapshot, cancellation);
        let mut private = prepared.private;
        macro_rules! finish_private {
            ($outcome:expr) => {
                return Ok(finalize_private_outcome(&mut private, $outcome))
            };
        }
        let cleanup_warnings = match publication {
            Ok(warnings) => warnings,
            Err(error) => {
                finish_private!(dump_failure(
                    tool_name,
                    error,
                    Some(redactor(&output.stdout)),
                    Some(redactor(&output.stderr)),
                    Some(reported_command),
                ));
            }
        };

        let mut warnings = bundled.1;
        warnings.extend(cleanup_warnings);
        Ok(finalize_private_outcome(
            &mut private,
            AdapterOutcome {
            ok: true,
            summary: format!(
                "{tool_name} published a validated platform {TARGET_PLATFORM_LINE} / export format {TARGET_EXPORT_FORMAT} full dump"
            ),
            changes: vec![format!(
                "replaced {} with validated full dump",
                target.display()
            )],
            warnings,
            errors: Vec::new(),
            artifacts: vec![target.display().to_string()],
            stdout: Some(redactor(&output.stdout)),
            stderr: Some(redactor(&output.stderr)),
            command: Some(reported_command),
            },
        ))
    }
}

/// A stable working-copy config, serialized across every runner invocation.
/// A retained sibling lock directory keeps the ownership namespace alive when
/// the projection is removed, independently for each working copy.
pub(crate) struct RunnerProjectStore {
    parent: DirectoryAnchor,
    directory: Option<DirectoryAnchor>,
    path: PathBuf,
    config_path: PathBuf,
    lock_anchor: DirectoryAnchor,
    _ownership: File,
    inputs: Vec<ConfigInputSnapshot>,
}

#[derive(Debug)]
pub(crate) enum RunnerProjectLockFailure {
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
    pub(crate) fn open_and_lock(
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
        let ownership = crate::infrastructure::platform::filesystem::open_directory_ownership_lock(
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
                Err(error)
                    if crate::infrastructure::platform::filesystem::is_file_lock_contended(
                        &error,
                    ) =>
                {
                    #[cfg(test)]
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

    pub(crate) fn directory(&self) -> &Path {
        &self.path
    }

    pub(crate) fn config_path(&self) -> &Path {
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

    pub(crate) fn verify(&self) -> Result<(), String> {
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
    pub(crate) fn write_projection(
        &mut self,
        primary: &YamlValue,
        local: Option<&YamlValue>,
    ) -> Result<(), String> {
        yaml_mapping(primary)?;
        if let Some(local) = local {
            yaml_mapping(local)?;
        }
        let local_bytes = local
            .map(serde_yaml::to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        self.ensure_directory()?;
        let previous_local = self
            .inputs
            .iter()
            .find(|input| input.path.file_name() == Some(OsStr::new(LOCAL_CONFIG_NAME)))
            .expect("both runner config names are tracked");
        let local_changed = previous_local.raw() != local_bytes.as_deref().map(str::as_bytes);
        let guard_needed =
            local_changed && matches!(previous_local.state, ConfigInputState::Exact(_));
        if guard_needed {
            self.write_config(EFFECTIVE_CONFIG_NAME, &YamlValue::Null)?;
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

    fn write_config(&mut self, name: &str, value: &YamlValue) -> Result<(), String> {
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
            let bytes = serde_yaml::to_string(value).map_err(|error| error.to_string())?;
            file.write_all(bytes.as_bytes())
                .map_err(|error| error.to_string())?;
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
                crate::infrastructure::platform::filesystem::RetainedDirectoryCapability::open(
                    &anchor.path,
                )
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

    pub(crate) fn remove_projection(&mut self) -> Result<(), String> {
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
            crate::infrastructure::platform::filesystem::RetainedDirectoryCapability::open(
                &self.parent.path,
            )
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
    use crate::infrastructure::platform::filesystem::RetainedDirectoryCapability;
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
enum RunnerProjectionCheckpoint {
    GuardInstalled,
    LocalGap,
    LocalConfirmed,
}

#[cfg(test)]
type RunnerProjectionHook = (RunnerProjectionCheckpoint, Box<dyn FnOnce(&Path)>);

#[cfg(test)]
thread_local! {
    static TEST_RUNNER_PROJECTION_HOOK: RefCell<Option<RunnerProjectionHook>> = const { RefCell::new(None) };
    static TEST_RUNNER_PROJECTION_FAILPOINT: RefCell<Option<RunnerProjectionCheckpoint>> = const { RefCell::new(None) };
}

fn run_runner_projection_checkpoint(
    checkpoint: RunnerProjectionCheckpoint,
    path: &Path,
) -> Result<(), String> {
    #[cfg(test)]
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

#[cfg(test)]
fn with_runner_projection_hook<T>(
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

#[cfg(test)]
fn with_runner_projection_failpoint<T>(
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
    crate::infrastructure::platform::filesystem::verify_owner_only_acl(file)
}

#[cfg(test)]
thread_local! {
    static TEST_PLATFORM_OVERRIDE: RefCell<Option<(PathBuf, String)>> = const { RefCell::new(None) };
    static TEST_PROJECT_LOCK_CONTENDED: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_test_project_lock_contention<T>(
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

#[cfg(test)]
pub(crate) fn with_test_platform<T>(
    executable: &Path,
    version: &str,
    run: impl FnOnce() -> T,
) -> T {
    struct Restore(Option<(PathBuf, String)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_PLATFORM_OVERRIDE.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let old = TEST_PLATFORM_OVERRIDE.with(|slot| {
        slot.borrow_mut()
            .replace((executable.to_path_buf(), version.to_string()))
    });
    let _restore = Restore(old);
    run()
}

pub(crate) struct FullDumpSelection<'a> {
    pub(crate) config_path: &'a Path,
    pub(crate) source_set: Option<&'a str>,
    pub(crate) extension: Option<&'a str>,
    pub(crate) expected_target: Option<&'a Path>,
}

/// Holds the filesystem objects and platform attestation across a canonical
/// runner invocation. The caller validates the provider result before commit.
pub(crate) struct VerifiedFullDumpPublication {
    prepared: PreparedDump,
    ready_staged_snapshot: Option<TreeSnapshot>,
    published: Option<PublishedFullDump>,
}

#[derive(Debug, Clone)]
pub(crate) struct PublishedFullDump {
    pub(crate) replaced: bool,
    pub(crate) files: u64,
    pub(crate) warnings: Vec<String>,
}

pub(crate) struct FullDumpFinish {
    pub(crate) warnings: Vec<String>,
    pub(crate) retained: Result<Option<PathBuf>, String>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum FullDumpPhase {
    Cancelled,
    Selection,
    Preparation,
    Integrity,
    Validation,
    Publication,
    Cleanup,
}

pub(crate) struct FullDumpFailure {
    pub(crate) phase: FullDumpPhase,
    pub(crate) recovery: Result<Option<PathBuf>, String>,
    pub(crate) publication_uncertain: bool,
    detail: String,
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
    fn new(phase: FullDumpPhase, detail: String) -> Self {
        Self {
            phase,
            recovery: Ok(None),
            publication_uncertain: false,
            detail,
        }
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
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        context: &WorkspaceContext,
        selection: FullDumpSelection<'_>,
        store: RunnerProjectStore,
        runner: &dyn ProcessRunner,
        cancellation: &CancellationToken,
        merge_inputs: impl FnOnce(YamlValue, Option<YamlValue>, &Path) -> Result<YamlValue, String>,
        project: impl FnOnce(YamlValue, &Path) -> Result<(YamlValue, Option<YamlValue>), String>,
    ) -> Result<Self, FullDumpFailure> {
        if cancellation.is_cancelled() {
            return Err(FullDumpFailure::new(
                FullDumpPhase::Cancelled,
                "full export cancelled before private preparation".into(),
            ));
        }
        let mut preparation_phase = FullDumpPhase::Preparation;
        let prepared = PreparedDump::prepare_selected(
            selection.config_path.to_path_buf(),
            selection.source_set,
            selection.extension,
            None,
            selection.expected_target,
            Some(store),
            context,
            &SYSTEM_PLATFORM_RESOLVER,
            runner,
            cancellation,
            merge_inputs,
            project,
            Some(&mut preparation_phase),
        )
        .map_err(|error| error.into_canonical_failure(preparation_phase))?;
        Ok(Self {
            prepared,
            ready_staged_snapshot: None,
            published: None,
        })
    }

    pub(crate) fn effective_config(&self) -> &Path {
        self.prepared
            .config_store
            .as_ref()
            .expect("canonical dump owns its config store")
            .config_path()
    }

    pub(crate) fn target(&self) -> &Path {
        &self.prepared.target
    }

    pub(crate) fn source_set(&self) -> &str {
        &self.prepared.selected_source_set
    }

    pub(crate) fn selected_extension(&self) -> Option<&str> {
        self.prepared.extension.as_deref()
    }

    pub(crate) fn staged_target(&self) -> &Path {
        &self.prepared.private.staged_tree
    }

    pub(crate) fn recheck_before_dispatch(&self) -> Result<(), FullDumpFailure> {
        self.prepared
            .recheck_inputs()
            .map_err(|error| FullDumpFailure::new(FullDumpPhase::Integrity, error))
    }

    /// Attests exactly the provider selected by the runner. This never resolves
    /// another executable or rewrites provider selection in the configuration.
    pub(crate) fn attest_selected_local(
        &mut self,
        executable: &Path,
        runner: &dyn ProcessRunner,
        cancellation: &CancellationToken,
    ) -> Result<(), FullDumpFailure> {
        if cancellation.is_cancelled() {
            return Err(FullDumpFailure::new(
                FullDumpPhase::Cancelled,
                "full export cancelled before selected local provider attestation".into(),
            ));
        }
        let mut probe_cancelled = false;
        let verified = SystemPlatformResolver::attest_selected_local(
            executable,
            runner,
            cancellation,
            &mut probe_cancelled,
        )
        .map_err(|error| {
            FullDumpFailure::new(
                if probe_cancelled {
                    FullDumpPhase::Cancelled
                } else {
                    FullDumpPhase::Integrity
                },
                error,
            )
        })?;
        self.prepared.platform_attestation = Some(verified.attestation);
        self.recheck_before_dispatch()
    }

    /// Creates the retained missing ancestry only after Selected is attested,
    /// before Continue lets the runner acquire its normal shared target lock.
    /// These empty directories are reversible preparation, not publication.
    pub(crate) fn prepare_dispatch_parent(
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

    pub(crate) fn begin_execution(&mut self) {
        self.prepared.private.begin_execution();
    }

    pub(crate) fn confirm_execution_finished(&mut self) {
        self.prepared.private.confirm_execution_finished();
    }

    pub(crate) fn is_published(&self) -> bool {
        self.published.is_some()
    }

    /// Authenticated Ready authorizes one snapshot of the held, settled stage.
    /// It does not prove runner or descendant completion and never clears the
    /// pending-writer guard. Repeated publication uses this same snapshot.
    pub(crate) fn capture_ready_staged_tree(&mut self) -> Result<(), FullDumpFailure> {
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
    pub(crate) fn publish(
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
        )
        .map_err(|error| FullDumpFailure::new(FullDumpPhase::Validation, error))?;
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
    pub(crate) fn finish(mut self) -> Result<FullDumpFinish, FullDumpFailure> {
        let published = self.is_published();
        let pending_writer = self.prepared.private.writer_completion_pending;
        let mut errors = Vec::new();
        let stage_cleanup = self.prepared.release_canonical_stage_for_cleanup();
        let stage_integrity_failed = stage_cleanup.is_err();
        if let Err(error) = stage_cleanup {
            errors.push(error);
        }
        if let Err(error) = self.prepared.private.cleanup_now() {
            // A failed cleanup still owns private debris. Keep its location
            // stable while Drop may retry scrubbing the held config file.
            self.prepared.private.retention = PrivateRetention::EntireStage;
            errors.push(error);
        }
        if !published && !pending_writer {
            if let Err(error) = self.prepared.cleanup_created_target_parents() {
                errors.push(error);
            }
        }
        // Cleanup can itself discover data that must be retained. Derive the
        // location after cleanup; a retained directory does not mean that the
        // public publication state is unknown.
        let retained = self.prepared.private.retained_location();
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
            return Ok(FullDumpFinish { warnings, retained });
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
            warnings: Vec::new(),
            retained: Ok(None),
        })
    }

    pub(crate) fn abort(self) -> Result<(), FullDumpFailure> {
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
                let detail = match private.cleanup_now() {
                    Ok(()) => detail,
                    Err(cleanup) => {
                        // Match canonical finish: a reported recovery location
                        // must survive a later Drop retry of private cleanup.
                        private.retention = PrivateRetention::EntireStage;
                        format!("{detail}; {cleanup}")
                    }
                };
                let mut failure = FullDumpFailure::new(phase, detail);
                failure.recovery = private.retained_location();
                failure.publication_uncertain = private.publication_uncertain;
                failure
            }
        }
    }
}

struct PreparedDump {
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

impl PreparedDump {
    fn prepare(
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        platform_resolver: &dyn PlatformResolver,
        runner: &dyn ProcessRunner,
        cancellation: &CancellationToken,
    ) -> Result<Self, DumpPreparationFailure> {
        validate_applied_full_dump_args(args)?;
        let config_path = resolve_config_path(args, context)?;
        Self::prepare_selected(
            config_path,
            args.get("sourceSet").and_then(Value::as_str),
            args.get("extension").and_then(Value::as_str),
            args.get("workdir").and_then(Value::as_str),
            None,
            None,
            context,
            platform_resolver,
            runner,
            cancellation,
            |mut primary, local, config_dir| {
                if let Some(local) = local {
                    validate_local_overlay_keys(&local, &config_dir.join(LOCAL_CONFIG_NAME))?;
                    merge_yaml_values(&mut primary, local);
                }
                Ok(primary)
            },
            |effective, _| Ok((effective, None)),
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_selected(
        config_path: PathBuf,
        source_set: Option<&str>,
        extension: Option<&str>,
        workdir: Option<&str>,
        expected_target: Option<&Path>,
        mut config_store: Option<RunnerProjectStore>,
        context: &WorkspaceContext,
        platform_resolver: &dyn PlatformResolver,
        runner: &dyn ProcessRunner,
        cancellation: &CancellationToken,
        merge_inputs: impl FnOnce(YamlValue, Option<YamlValue>, &Path) -> Result<YamlValue, String>,
        project: impl FnOnce(YamlValue, &Path) -> Result<(YamlValue, Option<YamlValue>), String>,
        mut preparation_phase: Option<&mut FullDumpPhase>,
    ) -> Result<Self, DumpPreparationFailure> {
        let config_dir = config_path.parent().unwrap_or(&context.cwd);
        let primary = ConfigInputSnapshot::required(config_path.clone())?;
        let primary_raw = primary
            .raw()
            .expect("required config snapshot always contains bytes");
        let primary_config = parse_yaml_mapping(primary_raw, &config_path)?;

        let local_path = config_dir.join(LOCAL_CONFIG_NAME);
        let local = ConfigInputSnapshot::optional(local_path.clone())?;
        let overlay = local
            .raw()
            .map(|raw| parse_yaml_mapping(raw, &local_path))
            .transpose()?;
        let mut effective = merge_inputs(primary_config, overlay, config_dir)?;
        if cancellation.is_cancelled() {
            if let Some(phase) = preparation_phase.as_deref_mut() {
                *phase = FullDumpPhase::Cancelled;
            }
            return Err("verified dump cancelled while preparing effective config"
                .to_string()
                .into());
        }

        let format = yaml_string(&effective, "format")?.unwrap_or("DESIGNER");
        if format != "DESIGNER" {
            return Err(format!(
                "applied full dump supports only format=DESIGNER source sets; configured format is {format}"
            ).into());
        }
        let builder = nested_yaml_string(&effective, &["providers", "pull"])?
            .or(yaml_string(&effective, "builder")?)
            .unwrap_or("DESIGNER");
        let utility = match builder {
            "DESIGNER" | "designer" => PlatformUtility::Designer,
            "IBCMD" | "ibcmd" => PlatformUtility::Ibcmd,
            _ if config_store.is_some() => PlatformUtility::Designer,
            other => {
                return Err(format!(
                    "applied full dump supports only builder=DESIGNER or IBCMD; configured builder is {other}"
                ).into());
            }
        };

        if yaml_mapping(&effective)?.contains_key(yaml_key("basePath")) {
            return Err(
                "v8-runner rejects the removed top-level `basePath` key; paths are resolved relative to the primary config directory"
                    .to_string().into(),
            );
        }
        let base_path = normalize_path_identity(config_dir)?;
        let selection = select_source_set(&effective, source_set, extension).inspect_err(|_| {
            if let Some(phase) = preparation_phase {
                *phase = FullDumpPhase::Selection;
            }
        })?;
        if config_store.is_none() {
            validate_selected_source_path(&selection.path)?;
        }
        normalize_source_set_paths(&mut effective, &base_path)?;
        normalize_relocated_config_paths(&mut effective, config_dir)?;

        let configured_work_path = match yaml_string(&effective, "workPath")? {
            Some(path) => path,
            None if config_store.is_some() => ".build",
            None => return Err("v8project.yaml field `workPath` must be a string".into()),
        };
        let work_path = workdir
            .map(|path| absolutize(Path::new(path), config_dir))
            .unwrap_or_else(|| absolutize(Path::new(configured_work_path), config_dir));
        let work_path = normalize_path_identity(&work_path)?;
        set_yaml_string(&mut effective, "workPath", &work_path.display().to_string())?;
        normalize_infobase_connection(&mut effective, config_dir)?;

        let workspace_root = normalize_path_identity(&context.workspace_root)?;
        let workspace_anchor = DirectoryAnchor::capture_exact(&workspace_root)?;
        let configured_target = base_path.join(&selection.path);
        let target = normalize_leaf_path(&configured_target)?;
        if let Some(expected) = expected_target {
            if target != normalize_leaf_path(expected)? {
                return Err("configured full dump target differs from the approved preview".into());
            }
        }
        validate_dump_target(&target, &workspace_root, &base_path, &work_path)?;
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
        validate_physical_dump_target(&target, &target_snapshot, selection.kind)?;

        let platform = if config_store.is_none() {
            let platform =
                platform_resolver.resolve(&effective, config_dir, utility, runner, cancellation)?;
            let version = parse_exact_platform_version(&platform.exact_version)
                .ok_or("resolved platform version is not an exact four-part version")?;
            if version[..3] != [8, 3, 27] {
                return Err(format!(
                    "resolved platform {} is not supported; required {TARGET_PLATFORM_LINE}.x",
                    platform.exact_version
                )
                .into());
            }
            Some(platform)
        } else {
            None
        };
        let mut private = PrivateDumpStage::create(&target_parent)?;
        let mut canonical_stage_anchor = None;
        let private_setup = (|| {
            if let Some(platform) = &platform {
                redirect_selected_source_set(
                    &mut effective,
                    selection.index,
                    &private.staged_tree,
                )?;
                set_nested_yaml_string(
                    &mut effective,
                    &["tools", "platform", "path"],
                    &platform.executable.display().to_string(),
                )?;
                set_nested_yaml_string(
                    &mut effective,
                    &["tools", "platform", "version"],
                    &platform.exact_version,
                )?;
            }
            let (primary, local) = project(effective, config_dir)?;
            if let Some(store) = config_store.as_mut() {
                store.write_projection(&primary, local.as_ref())?;
                store.verify()?;
                let created_stage = private
                    .execution_anchor()
                    .create_child(OsStr::new("staged-source"), &private.staged_tree)?;
                private.remember_staged_tree(&TreeSnapshot::Directory {
                    identity: created_stage.identity,
                    entries: Vec::new(),
                });
                canonical_stage_anchor = Some(created_stage);
            } else {
                private.write_effective_config(&primary)?;
            }
            Ok(())
        })();
        if let Err(error) = private_setup {
            canonical_stage_anchor.take();
            return Err(DumpPreparationFailure::PrivateSetup {
                detail: error,
                private: Box::new(private),
            });
        }

        let canonical_stage_identity = canonical_stage_anchor.as_ref().map(|stage| stage.identity);
        Ok(Self {
            canonical_stage_anchor,
            canonical_stage_identity,
            private,
            target,
            workspace_anchor,
            target_parent_relative,
            target_parent,
            target_snapshot,
            platform_attestation: platform.map(|platform| platform.attestation),
            target_missing_parents,
            opened_target_parent: None,
            created_target_parents: Vec::new(),
            config_inputs: vec![primary, local],
            config_store,
            selected_source_set: selection.name,
            source_kind: selection.kind,
            source_set_name: selection.explicit_source_set,
            extension: selection.extension,
        })
    }

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
                let current = validate_staged_dump(&self.private.staged_tree, self.source_kind)?;
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
                let current = validate_staged_dump(&self.private.staged_tree, self.source_kind)?;
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

#[derive(Debug)]
struct SourceSelection {
    index: usize,
    name: String,
    kind: SourceSetKind,
    path: PathBuf,
    explicit_source_set: Option<String>,
    extension: Option<String>,
}

fn select_source_set(
    effective: &YamlValue,
    source_set: Option<&str>,
    extension: Option<&str>,
) -> Result<SourceSelection, String> {
    let entries = yaml_mapping(effective)?
        .get(yaml_key("source-set"))
        .and_then(YamlValue::as_sequence)
        .ok_or_else(|| "v8project.yaml field `source-set` must be a list".to_string())?;
    let source_set = source_set.filter(|value| !value.trim().is_empty());
    let extension = extension.filter(|value| !value.trim().is_empty());

    let requested_name = match (source_set, extension) {
        (Some(source_set), Some(extension)) if source_set != extension => {
            return Err(format!(
                "sourceSet `{source_set}` does not match extension `{extension}`"
            ));
        }
        (Some(source_set), _) => Some(source_set),
        (None, Some(extension)) => Some(extension),
        (None, None) => None,
    };

    let parsed = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let name = yaml_string(entry, "name")?
                .ok_or_else(|| format!("source-set entry {index} is missing string `name`"))?;
            let source_type = yaml_string(entry, "type")?
                .ok_or_else(|| format!("source-set `{name}` is missing string `type`"))?;
            let kind = match source_type {
                "CONFIGURATION" => SourceSetKind::Configuration,
                "EXTENSION" => SourceSetKind::Extension,
                "EXTERNAL_DATA_PROCESSORS" => SourceSetKind::ExternalProcessor,
                "EXTERNAL_REPORTS" => SourceSetKind::ExternalReport,
                other => {
                    return Err(format!(
                        "source-set `{name}` has unsupported type `{other}`"
                    ));
                }
            };
            let path = yaml_string(entry, "path")?
                .ok_or_else(|| format!("source-set `{name}` is missing string `path`"))?;
            Ok((index, name.to_string(), kind, PathBuf::from(path)))
        })
        .collect::<Result<Vec<_>, String>>()?;

    let selected = if let Some(requested_name) = requested_name {
        parsed
            .iter()
            .find(|(_, name, _, _)| name == requested_name)
            .ok_or_else(|| format!("unknown source-set `{requested_name}`"))?
    } else {
        let configuration = parsed
            .iter()
            .filter(|(_, _, kind, _)| *kind == SourceSetKind::Configuration)
            .collect::<Vec<_>>();
        if configuration.len() != 1 {
            return Err(format!(
                "full dump requires exactly one CONFIGURATION source-set when sourceSet is omitted; found {}",
                configuration.len()
            ));
        }
        configuration[0]
    };

    match (extension, selected.2) {
        (Some(_), SourceSetKind::Extension) => {}
        (Some(extension), _) => {
            return Err(format!(
                "extension `{extension}` does not select an EXTENSION source-set"
            ));
        }
        (None, SourceSetKind::Configuration) => {}
        (None, SourceSetKind::Extension) => {
            return Err(format!(
                "source-set `{}` is an extension and requires `extension`",
                selected.1
            ));
        }
        (None, SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport) => {
            return Err(format!(
                "source-set `{}` is external; applied full dump supports only CONFIGURATION and EXTENSION",
                selected.1
            ));
        }
    }

    Ok(SourceSelection {
        index: selected.0,
        name: selected.1.clone(),
        kind: selected.2,
        path: selected.3.clone(),
        explicit_source_set: source_set.map(str::to_string),
        extension: extension.map(str::to_string),
    })
}

fn validate_applied_full_dump_args(args: &Map<String, Value>) -> Result<(), String> {
    const ALLOWED: &[&str] = &[
        "cwd",
        "dryRun",
        "confirm",
        "operation",
        "config",
        "workdir",
        "mode",
        "sourceSet",
        "extension",
    ];
    if let Some(unsupported) = args.keys().find(|key| !ALLOWED.contains(&key.as_str())) {
        return Err(format!(
            "verified applied full dump does not accept `{unsupported}`"
        ));
    }
    if args.get("mode").and_then(Value::as_str) != Some("full") {
        return Err("verified dump adapter requires explicit mode=full".to_string());
    }
    if args.get("dryRun").and_then(Value::as_bool) != Some(false) {
        return Err("verified dump adapter is only for applied execution".to_string());
    }
    Ok(())
}

fn resolve_config_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    let requested = args
        .get("config")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(EFFECTIVE_CONFIG_NAME));
    if requested.file_name().and_then(|name| name.to_str()) == Some(LOCAL_CONFIG_NAME) {
        return Err(format!(
            "{LOCAL_CONFIG_NAME} is a local overlay and cannot be used as the primary config"
        ));
    }
    normalize_leaf_path(&if requested.is_absolute() {
        requested
    } else {
        context.cwd.join(requested)
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConfigInputState {
    Exact(SecureFileSnapshot),
    Absent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfigInputSnapshot {
    path: PathBuf,
    state: ConfigInputState,
}

impl ConfigInputSnapshot {
    fn required(path: PathBuf) -> Result<Self, String> {
        let snapshot = secure_read_regular_file_snapshot(&path, "project config")?;
        Ok(Self {
            path,
            state: ConfigInputState::Exact(snapshot),
        })
    }

    fn optional(path: PathBuf) -> Result<Self, String> {
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

    fn raw(&self) -> Option<&[u8]> {
        match &self.state {
            ConfigInputState::Exact(snapshot) => Some(&snapshot.raw),
            ConfigInputState::Absent => None,
        }
    }

    fn recheck(&self) -> Result<(), String> {
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

fn secure_read_regular_file(path: &Path, role: &str) -> Result<Vec<u8>, String> {
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
struct PlatformAttestation {
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
    #[cfg(test)]
    TestFixture,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PlatformTrustAttestation {
    Immutable(ImmutablePlatformTrustSnapshot),
    #[cfg(test)]
    TestFixture,
}

impl PlatformTrustAttestation {
    fn policy(&self) -> PlatformTrustPolicy {
        match self {
            Self::Immutable(_) => PlatformTrustPolicy::Immutable,
            #[cfg(test)]
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
    fn capture_immutable(executable: &Path) -> Result<Self, String> {
        Self::capture_with_policy(executable, PlatformTrustPolicy::Immutable)
    }

    #[cfg(test)]
    fn capture_test_fixture(executable: &Path) -> Result<Self, String> {
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
            #[cfg(test)]
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
            #[cfg(test)]
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

    fn recheck(&self) -> Result<(), String> {
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
    let initial_names = crate::infrastructure::platform::filesystem::read_directory_names(
        directory,
    )
    .map_err(|error| {
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
    let final_names = crate::infrastructure::platform::filesystem::read_directory_names(directory)
        .map_err(|error| {
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

fn normalize_leaf_path(path: &Path) -> Result<PathBuf, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .map_err(|error| format!("failed to determine current directory: {error}"))?
            .join(path)
    };
    let parent = absolute
        .parent()
        .ok_or_else(|| format!("path has no parent: {}", absolute.display()))?;
    let name = absolute
        .file_name()
        .ok_or_else(|| format!("path has no final component: {}", absolute.display()))?;
    Ok(normalize_path_identity(parent)?.join(name))
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
    crate::infrastructure::platform::filesystem::create_owner_only_file_child(parent, name)
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
            crate::infrastructure::platform::filesystem::create_owner_only_directory_child(
                &self.directory,
                name,
            )
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
        let names = crate::infrastructure::platform::filesystem::read_directory_names_with_limit(
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

    fn write_effective_config(&mut self, value: &YamlValue) -> Result<(), String> {
        let bytes = serde_yaml::to_string(value)
            .map_err(|error| format!("failed to serialize effective dump config: {error}"))?;
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
            .write_all(bytes.as_bytes())
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
    let names = crate::infrastructure::platform::filesystem::read_directory_names_with_limit(
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
    let names = crate::infrastructure::platform::filesystem::read_directory_names_with_limit(
        directory,
        None,
        || Ok(()),
    )
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
    for name in crate::infrastructure::platform::filesystem::read_directory_names(directory)
        .map_err(|error| {
            format!(
                "failed to enumerate the retained private directory anchor {}: {error}",
                display_path.display()
            )
        })?
    {
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
enum TreeSnapshot {
    Absent,
    Directory {
        identity: FileIdentity,
        entries: Vec<TreeEntrySnapshot>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TreeEntrySnapshot {
    relative_path: PathBuf,
    kind: TreeEntryKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TreeEntryKind {
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
    fn capture_target(path: &Path) -> Result<Self, String> {
        capture_tree_target_nofollow(path)
    }

    fn root_identity(&self) -> Option<FileIdentity> {
        match self {
            Self::Absent => None,
            Self::Directory { identity, .. } => Some(*identity),
        }
    }
}

#[cfg(test)]
fn capture_directory_snapshot(root: &Path) -> Result<Vec<TreeEntrySnapshot>, String> {
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
fn capture_tree_target_nofollow(path: &Path) -> Result<TreeSnapshot, String> {
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
fn capture_tree_target_nofollow(path: &Path) -> Result<TreeSnapshot, String> {
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
fn capture_tree_target_nofollow(path: &Path) -> Result<TreeSnapshot, String> {
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
    let initial_names = crate::infrastructure::platform::filesystem::read_directory_names(
        directory,
    )
    .map_err(|error| {
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
    let final_names = crate::infrastructure::platform::filesystem::read_directory_names(directory)
        .map_err(|error| {
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

fn validate_staged_dump(root: &Path, kind: SourceSetKind) -> Result<TreeSnapshot, String> {
    let snapshot = TreeSnapshot::capture_target(root)?;
    validate_staged_dump_snapshot(root, kind, &snapshot)?;
    Ok(snapshot)
}

fn validate_staged_dump_snapshot(
    root: &Path,
    kind: SourceSetKind,
    snapshot: &TreeSnapshot,
) -> Result<(), String> {
    let TreeSnapshot::Directory { entries, .. } = snapshot else {
        return Err(format!(
            "v8-runner did not create the private staged dump directory: {}",
            root.display()
        ));
    };
    if entries.is_empty() {
        return Err(format!("private staged dump is empty: {}", root.display()));
    }
    let owner_path = root.join("Configuration.xml");
    let owner_entry = entries
        .iter()
        .find(|entry| entry.relative_path == Path::new("Configuration.xml"))
        .ok_or_else(|| {
            format!(
                "private staged dump has no Configuration.xml owner: {}",
                root.display()
            )
        })?;
    validate_staged_required_owner(&owner_path, kind, &owner_entry.kind)?;

    for entry in entries {
        let TreeEntryKind::File { .. } = entry.kind else {
            continue;
        };
        if !entry
            .relative_path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
        {
            continue;
        }
        validate_staged_xml(&root.join(&entry.relative_path), &entry.kind)?;
    }
    Ok(())
}

fn validate_required_owner(path: &Path, kind: SourceSetKind) -> Result<(), String> {
    let raw = secure_read_regular_file(path, "staged source-set owner")?;
    validate_required_owner_raw(path, kind, &raw)
}

fn validate_staged_required_owner(
    path: &Path,
    kind: SourceSetKind,
    expected: &TreeEntryKind,
) -> Result<(), String> {
    let raw = read_tree_bound_file(path, "staged source-set owner", expected)?;
    validate_required_owner_raw(path, kind, &raw)
}

fn validate_required_owner_raw(path: &Path, kind: SourceSetKind, raw: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(raw)
        .map_err(|error| format!("staged source-set owner is not UTF-8: {error}"))?;
    let source = text.trim_start_matches('\u{feff}');
    let document = Document::parse(source)
        .map_err(|error| format!("failed to parse staged owner {}: {error}", path.display()))?;
    let root = document.root_element();
    if root.tag_name().namespace() != Some(MD_CLASSES_NS)
        || root.tag_name().name() != "MetaDataObject"
    {
        return Err(format!(
            "staged owner must be {{{MD_CLASSES_NS}}}MetaDataObject: {}",
            path.display()
        ));
    }
    let version = root_version_literal(source, root);
    if version.as_deref() != Some(TARGET_EXPORT_FORMAT) {
        return Err(format!(
            "staged owner export format must be the exact raw literal {TARGET_EXPORT_FORMAT}; found {} in {}",
            version.as_deref().unwrap_or("<missing>"),
            path.display()
        ));
    }
    let children = root
        .children()
        .filter(|node| node.is_element())
        .collect::<Vec<_>>();
    if children.len() != 1
        || children[0].tag_name().namespace() != Some(MD_CLASSES_NS)
        || children[0].tag_name().name() != "Configuration"
    {
        return Err(format!(
            "staged owner must contain exactly one direct {{{MD_CLASSES_NS}}}Configuration child: {}",
            path.display()
        ));
    }
    let is_extension = is_configuration_extension(children[0]);
    match (kind, is_extension) {
        (SourceSetKind::Configuration, false) | (SourceSetKind::Extension, true) => Ok(()),
        (SourceSetKind::Configuration, true) => Err(format!(
            "CONFIGURATION source-set produced an extension owner: {}",
            path.display()
        )),
        (SourceSetKind::Extension, false) => Err(format!(
            "EXTENSION source-set produced a configuration owner without ConfigurationExtensionPurpose: {}",
            path.display()
        )),
        _ => Err("external source-set kind reached guarded dump validation".to_string()),
    }
}

fn is_configuration_extension(configuration: roxmltree::Node<'_, '_>) -> bool {
    configuration
        .children()
        .find(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(MD_CLASSES_NS)
                && node.tag_name().name() == "Properties"
        })
        .is_some_and(|properties| {
            properties.children().any(|node| {
                node.is_element()
                    && node.tag_name().namespace() == Some(MD_CLASSES_NS)
                    && node.tag_name().name() == "ConfigurationExtensionPurpose"
            })
        })
}

fn validate_staged_xml(path: &Path, expected: &TreeEntryKind) -> Result<(), String> {
    let raw = read_tree_bound_file(path, "staged XML", expected)?;
    let text = std::str::from_utf8(&raw)
        .map_err(|error| format!("staged XML is not UTF-8 {}: {error}", path.display()))?;
    let source = text.trim_start_matches('\u{feff}');
    let document = Document::parse(source)
        .map_err(|error| format!("failed to parse staged XML {}: {error}", path.display()))?;
    let root = document.root_element();
    let namespace = root.tag_name().namespace().unwrap_or("");
    let local_name = root.tag_name().name();
    match staged_root_version_policy(namespace, local_name) {
        Some(StagedRootVersionPolicy::Versionless) => {
            if let Some(version) = root_version_literal(source, root) {
                return Err(format!(
                    "staged XML root {{{namespace}}}{local_name} is a registered versionless family and must not declare version={version} in {}",
                    path.display()
                ));
            }
        }
        Some(StagedRootVersionPolicy::ExactRootVersion) => {
            let version = root_version_literal(source, root);
            if version.as_deref() != Some(TARGET_EXPORT_FORMAT) {
                return Err(format!(
                    "staged XML root {{{namespace}}}{local_name} must use the exact raw version literal {TARGET_EXPORT_FORMAT}; found {} (missing means legacy format 1.0) in {}",
                    version.as_deref().unwrap_or("<missing>"),
                    path.display()
                ));
            }
        }
        None => {
            return Err(format!(
                "staged XML root {{{namespace}}}{local_name} is unsupported by the closed platform {TARGET_PLATFORM_LINE} / export format {TARGET_EXPORT_FORMAT} registry: {}",
                path.display()
            ));
        }
    }
    Ok(())
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

// The closed root registry is shared with platform XML owner resolution so the
// two cannot disagree about a root; see `infrastructure::platform_xml_roots`
// for why it lives there. Fail-closed here means an unlisted root discards the
// staged dump, so a missing entry makes `mode=full` unusable for every
// configuration that owns the artifact. Tests enumerate the table, so a new
// entry that no test case reaches fails the coverage assertion.
#[cfg(test)]
use crate::infrastructure::platform_xml_roots::PLATFORM_XML_ROOTS as STAGED_ROOT_REGISTRY;
use crate::infrastructure::platform_xml_roots::{
    platform_xml_publication_policy as staged_root_version_policy,
    PlatformXmlPublicationPolicy as StagedRootVersionPolicy,
};

fn remove_created_source_parent(
    parent: &DirectoryAnchor,
    child: &DirectoryAnchor,
) -> Result<(), String> {
    use crate::infrastructure::platform::filesystem::RetainedDirectoryCapability;
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
pub(crate) enum PublicationCheckpoint {
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

#[cfg(test)]
type PublicationHook = (PublicationCheckpoint, Box<dyn FnOnce()>);
#[cfg(test)]
type PathHook = Box<dyn FnOnce(&Path)>;
#[cfg(test)]
type CleanupOpenHook = Box<dyn FnMut(&Path)>;
#[cfg(all(test, unix))]
type DirectoryRenameHook = Box<dyn FnMut(&Path)>;
#[cfg(test)]
type TargetedReadHooks = (
    PathBuf,
    Option<Box<dyn FnOnce()>>,
    Option<Box<dyn FnOnce()>>,
);

#[cfg(test)]
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

#[cfg(test)]
fn with_publication_failpoint<T>(
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

#[cfg(test)]
pub(crate) fn with_publication_hook<T>(
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

#[cfg(test)]
fn with_secure_read_hook<T>(hook: impl FnOnce(&Path) + 'static, action: impl FnOnce() -> T) -> T {
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

#[cfg(all(test, unix))]
fn with_directory_rename_hook<T>(
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

#[cfg(test)]
fn with_targeted_read_hooks<T>(
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

#[cfg(test)]
fn with_tree_open_hook<T>(hook: impl FnOnce(&Path) + 'static, action: impl FnOnce() -> T) -> T {
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

#[cfg(test)]
fn with_directory_cleanup_open_hook<T>(
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

#[cfg(test)]
fn with_private_create_hook<T>(
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

#[cfg(all(test, windows))]
fn with_effective_config_create_hook<T>(
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

#[cfg(test)]
fn with_effective_config_prepare_hook<T>(
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

#[cfg(test)]
fn with_target_parent_capture_hook<T>(
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

#[cfg(all(test, windows))]
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

#[cfg(all(test, windows))]
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
    #[cfg(test)]
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
    #[cfg(test)]
    if TEST_PRIVATE_CREATION_FAILPOINT.with(|slot| slot.borrow().as_ref() == Some(&checkpoint)) {
        return Err(format!(
            "injected private dump creation failure at {checkpoint:?}"
        ));
    }
    let _ = checkpoint;
    Ok(())
}

fn run_private_create_hook(_path: &Path) {
    #[cfg(test)]
    if let Some(hook) = TEST_PRIVATE_CREATE_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook(_path);
    }
}

fn run_effective_config_prepare_hook(_path: &Path) {
    #[cfg(test)]
    if let Some(hook) = TEST_EFFECTIVE_CONFIG_PREPARE_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook(_path);
    }
}

#[cfg(windows)]
fn run_effective_config_create_hook(_path: &Path) {
    #[cfg(test)]
    if let Some(hook) = TEST_EFFECTIVE_CONFIG_CREATE_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook(_path);
    }
}

fn run_target_parent_capture_hook(_path: &Path) {
    #[cfg(test)]
    if let Some(hook) = TEST_TARGET_PARENT_CAPTURE_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook(_path);
    }
}

fn run_secure_read_hook(_path: &Path) {
    #[cfg(test)]
    if let Some(hook) = TEST_SECURE_READ_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook(_path);
    }
    #[cfg(test)]
    let hook = TEST_TARGETED_READ_HOOKS.with(|slot| {
        let mut slot = slot.borrow_mut();
        let (target, before, _) = slot.as_mut()?;
        if target == _path {
            before.take()
        } else {
            None
        }
    });
    #[cfg(test)]
    if let Some(hook) = hook {
        hook();
    }
}

fn run_secure_read_after_hook(_path: &Path) {
    #[cfg(test)]
    let hook = TEST_TARGETED_READ_HOOKS.with(|slot| {
        let mut slot = slot.borrow_mut();
        let (target, _, after) = slot.as_mut()?;
        if target == _path {
            after.take()
        } else {
            None
        }
    });
    #[cfg(test)]
    if let Some(hook) = hook {
        hook();
    }
}

fn run_tree_open_hook(_path: &Path) {
    #[cfg(test)]
    if let Some(hook) = TEST_TREE_OPEN_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook(_path);
    }
}

fn run_directory_cleanup_open_hook(_path: &Path) {
    #[cfg(test)]
    TEST_CLEANUP_OPEN_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().as_mut() {
            hook(_path);
        }
    });
}

fn run_publication_hook(checkpoint: PublicationCheckpoint) {
    #[cfg(test)]
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
    #[cfg(test)]
    TEST_DIRECTORY_RENAME_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().as_mut() {
            hook(_source);
        }
    });
}

fn run_publication_failpoint(checkpoint: PublicationCheckpoint) -> Result<(), String> {
    run_publication_hook(checkpoint);
    #[cfg(test)]
    if TEST_PUBLICATION_FAILPOINT.with(|slot| slot.borrow().as_ref() == Some(&checkpoint)) {
        return Err(format!(
            "injected verified dump publication failure at {checkpoint:?}"
        ));
    }
    Ok(())
}

fn dump_process_args(
    invocation: FullDumpInvocation,
    config: &Path,
    source_set: Option<&str>,
    extension: Option<&str>,
) -> Vec<String> {
    let mut args = Vec::new();
    if invocation == FullDumpInvocation::BuildDump {
        args.push("dump".to_string());
    }
    args.extend(["--config".to_string(), config.display().to_string()]);
    if invocation == FullDumpInvocation::RuntimeExecute {
        args.push("dump".to_string());
    }
    args.extend(["--mode".to_string(), "full".to_string()]);
    if let Some(source_set) = source_set {
        args.extend(["--source-set".to_string(), source_set.to_string()]);
    }
    if let Some(extension) = extension {
        args.extend(["--extension".to_string(), extension.to_string()]);
    }
    args
}

fn reported_dump_process_args(
    invocation: FullDumpInvocation,
    source_set: Option<&str>,
    extension: Option<&str>,
) -> Vec<String> {
    dump_process_args(
        invocation,
        Path::new("<private-effective-config>"),
        source_set,
        extension,
    )
}

fn finalize_private_outcome(
    private: &mut PrivateDumpStage,
    mut outcome: AdapterOutcome,
) -> AdapterOutcome {
    if let Err(error) = private.cleanup_now() {
        if outcome.ok {
            outcome.ok = false;
            outcome.summary = format!(
                "{}; validated publication completed but private cleanup failed",
                outcome.summary
            );
        }
        outcome.errors.push(error.clone());
        let status = if private.effective_config_secret_present {
            "Secret-bearing private dump debris may remain"
        } else {
            "Private dump debris may remain"
        };
        match private.bound_root_for_reporting() {
            Ok(_) => {
                outcome
                    .warnings
                    .push(format!("{status} at {}", private.root.display()));
                outcome.artifacts.push(private.root.display().to_string());
            }
            Err(error) => outcome.warnings.push(format!(
                "{status}, but its current location is unconfirmed; original name: {} ({error})",
                private.root.display()
            )),
        }
        outcome.stderr = Some(match outcome.stderr.take() {
            Some(stderr) if !stderr.is_empty() => format!("{stderr}\n{error}\n"),
            _ => format!("{error}\n"),
        });
    }
    if private.publication_uncertain {
        outcome.ok = false;
        outcome.summary =
            "Full dump publication state is unknown; inspect the retained data before retrying"
                .into();
    }
    if private.retention != PrivateRetention::None || private.publication_uncertain {
        outcome.warnings.retain(|warning| {
            warning
                != "Git-visible sources were not published; the private staged dump was discarded"
        });
        let status = if private.effective_config_secret_present {
            "Private data retained; effective configuration cleanup is incomplete"
        } else {
            "Data retained for recovery"
        };
        let location = if private.retention == PrivateRetention::EntireStage {
            &private.root
        } else {
            &private.recovery
        };
        let proof = if private.retention == PrivateRetention::EntireStage {
            private.bound_root_for_reporting().map(drop)
        } else {
            private.verify_recovery_location()
        };
        match proof {
            Ok(()) => {
                outcome
                    .warnings
                    .push(format!("{status} at {}", location.display()));
                outcome.artifacts.push(location.display().to_string());
            }
            Err(error) => outcome.warnings.push(format!(
                "{status}, but its current location is unconfirmed; original name: {} ({error})",
                location.display()
            )),
        }
    }
    outcome
}

fn dump_failure(
    tool_name: &str,
    error: String,
    stdout: Option<String>,
    stderr: Option<String>,
    command: Option<Vec<String>>,
) -> AdapterOutcome {
    AdapterOutcome {
        ok: false,
        summary: format!("{tool_name} failed before verified dump publication"),
        changes: Vec::new(),
        warnings: vec![
            "Git-visible sources were not published; the private staged dump was discarded"
                .to_string(),
        ],
        errors: vec![error.clone()],
        artifacts: Vec::new(),
        stdout,
        stderr: Some(stderr.unwrap_or_else(|| format!("{error}\n"))),
        command,
    }
}

fn cancelled_dump_outcome(
    tool_name: &str,
    output: &ProcessOutput,
    command: Vec<String>,
) -> AdapterOutcome {
    AdapterOutcome {
        ok: false,
        summary: format!("{tool_name} cancelled before verified dump publication"),
        changes: Vec::new(),
        warnings: vec![
            "Git-visible sources were not published; the private staged dump was discarded"
                .to_string(),
        ],
        errors: vec!["verified full dump cancelled".to_string()],
        artifacts: Vec::new(),
        stdout: Some(redactor(&output.stdout)),
        stderr: Some(redactor(&output.stderr)),
        command: Some(command),
    }
}

fn validate_selected_source_path(path: &Path) -> Result<(), String> {
    use std::path::Component;

    if path.is_absolute() {
        return Err(format!(
            "selected source-set target must be relative to the primary config directory: {}",
            path.display()
        ));
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(format!(
            "selected source-set target must be a contained relative path without `..`: {}",
            path.display()
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
) -> Result<(), String> {
    let TreeSnapshot::Directory { entries, .. } = snapshot else {
        return Ok(());
    };
    let files = entries
        .iter()
        .filter_map(|entry| {
            matches!(entry.kind, TreeEntryKind::File { .. })
                .then_some(entry.relative_path.as_path())
        })
        .collect::<Vec<_>>();
    if files.is_empty() {
        return Ok(());
    }
    match classify_physical_source_inventory(kind, files.iter().copied()) {
        SourceFormat::PlatformXml => validate_required_owner(
            &target.join("Configuration.xml"),
            kind,
        )
        .map_err(|error| {
            format!(
                "{error}; the existing DESIGNER target is not an exact export format {TARGET_EXPORT_FORMAT} source set and will not be migrated automatically. Explicitly migrate it with platform {TARGET_PLATFORM_LINE} and retry; formats newer than {TARGET_EXPORT_FORMAT} require future platform 8.5 support"
            )
        }),
        SourceFormat::Edt => Err(format!(
            "physical dump target is EDT and cannot be replaced by a DESIGNER YAML claim: {}",
            target.display()
        )),
        SourceFormat::Invalid => Err(format!(
            "physical dump target has mixed/invalid DESIGNER and EDT markers: {}",
            target.display()
        )),
        SourceFormat::Unknown => Err(format!(
            "physical dump target has no authoritative DESIGNER marker and cannot be replaced: {}",
            target.display()
        )),
    }
}

fn redirect_selected_source_set(
    root: &mut YamlValue,
    selected_index: usize,
    stage: &Path,
) -> Result<(), String> {
    let entries = yaml_mapping_mut(root)?
        .get_mut(yaml_key("source-set"))
        .and_then(YamlValue::as_sequence_mut)
        .ok_or_else(|| "v8project.yaml field `source-set` must be a list".to_string())?;
    let entry = entries
        .get_mut(selected_index)
        .ok_or_else(|| "selected source-set disappeared from effective config".to_string())?;
    set_yaml_string(entry, "path", &stage.display().to_string())
}

fn normalize_source_set_paths(root: &mut YamlValue, config_dir: &Path) -> Result<(), String> {
    let entries = yaml_mapping_mut(root)?
        .get_mut(yaml_key("source-set"))
        .and_then(YamlValue::as_sequence_mut)
        .ok_or_else(|| "v8project.yaml field `source-set` must be a list".to_string())?;
    for (index, entry) in entries.iter_mut().enumerate() {
        let path = yaml_string(entry, "path")?
            .ok_or_else(|| format!("source-set entry {index} is missing string `path`"))?;
        let absolute = normalize_path_identity(&absolutize(Path::new(path), config_dir))?;
        set_yaml_string(entry, "path", &absolute.display().to_string())?;
    }
    Ok(())
}

fn normalize_relocated_config_paths(root: &mut YamlValue, config_dir: &Path) -> Result<(), String> {
    for path in [
        &["tools", "va", "epf_path"][..],
        &["tools", "client_mcp", "extension", "source", "path"][..],
        &["tools", "client_mcp", "extension", "artifact", "path"][..],
        &["tests", "va", "params_path"][..],
    ] {
        normalize_optional_nested_path(root, path, config_dir)?;
    }

    let Some(profiles) = yaml_value_at_path_mut(root, &["tests", "va", "profiles"])? else {
        return Ok(());
    };
    let profiles = profiles
        .as_mapping_mut()
        .ok_or_else(|| "YAML field `tests.va.profiles` must be a mapping".to_string())?;
    for (name, profile) in profiles {
        let name = name
            .as_str()
            .ok_or_else(|| "tests.va.profiles contains a non-string profile name".to_string())?;
        let Some(path) = yaml_string(profile, "feature_path")?.map(str::to_string) else {
            continue;
        };
        let absolute = absolutize(Path::new(&path), config_dir);
        set_yaml_string(profile, "feature_path", &absolute.display().to_string())
            .map_err(|error| format!("invalid tests.va.profiles.{name}: {error}"))?;
    }
    Ok(())
}

fn normalize_optional_nested_path(
    root: &mut YamlValue,
    path: &[&str],
    config_dir: &Path,
) -> Result<(), String> {
    let Some(value) = nested_yaml_string(root, path)?.map(str::to_string) else {
        return Ok(());
    };
    let absolute = absolutize(Path::new(&value), config_dir);
    set_nested_yaml_string(root, path, &absolute.display().to_string())
}

fn yaml_value_at_path_mut<'a>(
    value: &'a mut YamlValue,
    path: &[&str],
) -> Result<Option<&'a mut YamlValue>, String> {
    let mut current = value;
    for (index, key) in path.iter().enumerate() {
        let Some(mapping) = current.as_mapping_mut() else {
            return Err(format!(
                "YAML field `{}` must be a mapping",
                path[..index].join(".")
            ));
        };
        let Some(next) = mapping.get_mut(yaml_key(key)) else {
            return Ok(None);
        };
        if matches!(next, YamlValue::Null) {
            return Ok(None);
        }
        current = next;
    }
    Ok(Some(current))
}

fn parse_yaml_mapping(raw: &[u8], path: &Path) -> Result<YamlValue, String> {
    let text = std::str::from_utf8(raw)
        .map_err(|error| format!("failed to read {} as UTF-8: {error}", path.display()))?;
    let value = serde_yaml::from_str::<YamlValue>(text)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    yaml_mapping(&value)?;
    Ok(value)
}

fn yaml_mapping(value: &YamlValue) -> Result<&YamlMapping, String> {
    value
        .as_mapping()
        .ok_or_else(|| "expected a YAML mapping at the document root".to_string())
}

fn yaml_mapping_mut(value: &mut YamlValue) -> Result<&mut YamlMapping, String> {
    value
        .as_mapping_mut()
        .ok_or_else(|| "expected a YAML mapping at the document root".to_string())
}

fn yaml_key(key: &str) -> YamlValue {
    YamlValue::String(key.to_string())
}

fn yaml_string<'a>(value: &'a YamlValue, key: &str) -> Result<Option<&'a str>, String> {
    match yaml_mapping(value)?.get(yaml_key(key)) {
        None | Some(YamlValue::Null) => Ok(None),
        Some(YamlValue::String(value)) => Ok(Some(value)),
        Some(_) => Err(format!("YAML field `{key}` must be a string")),
    }
}

fn nested_yaml_string<'a>(value: &'a YamlValue, path: &[&str]) -> Result<Option<&'a str>, String> {
    let mut current = value;
    for (index, key) in path.iter().enumerate() {
        let Some(next) = current.as_mapping().and_then(|map| map.get(yaml_key(key))) else {
            return Ok(None);
        };
        if index + 1 == path.len() {
            return match next {
                YamlValue::String(value) => Ok(Some(value)),
                YamlValue::Null => Ok(None),
                _ => Err(format!("YAML field `{}` must be a string", path.join("."))),
            };
        }
        current = next;
    }
    Ok(None)
}

fn set_yaml_string(value: &mut YamlValue, key: &str, new_value: &str) -> Result<(), String> {
    yaml_mapping_mut(value)?.insert(yaml_key(key), YamlValue::String(new_value.to_string()));
    Ok(())
}

fn set_nested_yaml_string(
    value: &mut YamlValue,
    path: &[&str],
    new_value: &str,
) -> Result<(), String> {
    if path.is_empty() {
        return Err("cannot assign an empty YAML path".to_string());
    }
    let mut current = value;
    for key in &path[..path.len() - 1] {
        let mapping = yaml_mapping_mut(current)?;
        current = mapping
            .entry(yaml_key(key))
            .or_insert_with(|| YamlValue::Mapping(YamlMapping::new()));
        if !current.is_mapping() {
            return Err(format!(
                "YAML field `{}` must be a mapping",
                path[..path.len() - 1].join(".")
            ));
        }
    }
    set_yaml_string(current, path[path.len() - 1], new_value)
}

fn validate_local_overlay_keys(value: &YamlValue, path: &Path) -> Result<(), String> {
    for key in yaml_mapping(value)?.keys() {
        let Some(key) = key.as_str() else {
            return Err(format!(
                "local config overlay contains a non-string top-level key: {}",
                path.display()
            ));
        };
        match key {
            "source-set" | "format" | "builder" => {
                return Err(format!(
                    "local config overlay cannot override project identity key `{key}`: {}",
                    path.display()
                ));
            }
            "workPath" | "infobase" | "tools" | "tests" | "mcp" => {}
            other => {
                return Err(format!(
                    "local config overlay does not support top-level key `{other}`: {}",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

fn merge_yaml_values(base: &mut YamlValue, overlay: YamlValue) {
    match (base, overlay) {
        (YamlValue::Mapping(base), YamlValue::Mapping(overlay)) => {
            for (key, overlay_value) in overlay {
                match base.get_mut(&key) {
                    Some(base_value) => merge_yaml_values(base_value, overlay_value),
                    None => {
                        base.insert(key, overlay_value);
                    }
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

fn normalize_infobase_connection(root: &mut YamlValue, config_dir: &Path) -> Result<(), String> {
    let Some(connection) =
        nested_yaml_string(root, &["infobase", "connection"])?.map(str::to_string)
    else {
        return Ok(());
    };
    let trimmed = connection.trim();
    let normalized = if trimmed.starts_with('/') || trimmed.starts_with('-') {
        normalize_raw_connection_args(trimmed, config_dir)
    } else {
        normalize_key_value_connection(&connection, config_dir)?
    };
    set_nested_yaml_string(root, &["infobase", "connection"], &normalized)
}

fn normalize_key_value_connection(connection: &str, config_dir: &Path) -> Result<String, String> {
    split_key_value_connection(connection)?
        .into_iter()
        .map(|part| {
            let trimmed = part.trim();
            if !trimmed.to_ascii_lowercase().starts_with("file=") {
                return Ok(trimmed.to_string());
            }
            let normalized = normalize_quoted_connection_file_path(&trimmed[5..], config_dir)?;
            Ok(format!("{}{}", &trimmed[..5], normalized))
        })
        .collect::<Result<Vec<_>, String>>()
        .map(|parts| parts.join(";"))
}

fn split_key_value_connection(connection: &str) -> Result<Vec<&str>, String> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut quote = None;
    let mut characters = connection.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        if let Some(active_quote) = quote {
            if character == active_quote {
                if characters
                    .peek()
                    .is_some_and(|(_, next)| *next == active_quote)
                {
                    characters.next();
                } else {
                    quote = None;
                }
            }
            continue;
        }
        if character == '"' {
            quote = Some(character);
        } else if character == ';' {
            parts.push(&connection[start..index]);
            start = index + character.len_utf8();
        }
    }
    if let Some(quote) = quote {
        return Err(format!(
            "infobase.connection contains an unterminated {quote} quote"
        ));
    }
    parts.push(&connection[start..]);
    Ok(parts)
}

fn normalize_quoted_connection_file_path(path: &str, config_dir: &Path) -> Result<String, String> {
    let trimmed = path.trim();
    let quote = match (trimmed.chars().next(), trimmed.chars().last()) {
        (Some('"'), Some('"')) => Some('"'),
        (Some('"'), _) => {
            return Err("infobase.connection File value has mismatched quotes".to_string());
        }
        (_, Some('"')) => {
            return Err("infobase.connection File value has mismatched quotes".to_string());
        }
        _ => None,
    };
    let unquoted = quote
        .map(|_| &trimmed[1..trimmed.len() - 1])
        .unwrap_or(trimmed);
    let normalized = normalize_connection_file_path(unquoted, config_dir);
    if let Some(quote) = quote {
        if normalized.contains(quote) {
            return Err(format!(
                "normalized infobase.connection File path contains its {quote} delimiter"
            ));
        }
        return Ok(format!("{quote}{normalized}{quote}"));
    }
    if normalized.contains(';') || normalized.chars().any(char::is_whitespace) {
        return Ok(format!("\"{}\"", normalized.replace('"', "\"\"")));
    }
    Ok(normalized)
}

fn normalize_raw_connection_args(connection: &str, config_dir: &Path) -> String {
    let mut args = split_arg_string(connection);
    let mut index = 0;
    while index + 1 < args.len() {
        if args[index].eq_ignore_ascii_case("/f") || args[index].eq_ignore_ascii_case("-f") {
            args[index + 1] = normalize_connection_file_path(&args[index + 1], config_dir);
            index += 2;
        } else {
            index += 1;
        }
    }
    join_arg_string(&args)
}

fn normalize_connection_file_path(path: &str, config_dir: &Path) -> String {
    let path = strip_matching_quotes(path.trim()).unwrap_or(path.trim());
    absolutize(Path::new(path), config_dir)
        .display()
        .to_string()
}

fn strip_matching_quotes(value: &str) -> Option<&str> {
    if value.len() < 2 {
        return None;
    }
    let quote = value.as_bytes()[0];
    let last = *value.as_bytes().last()?;
    ((quote == b'\'' || quote == b'"') && quote == last).then_some(&value[1..value.len() - 1])
}

fn split_arg_string(raw: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in raw.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ch if ch.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}

fn join_arg_string(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg.is_empty() || arg.chars().any(char::is_whitespace) {
                format!("\"{}\"", arg.replace('"', "\\\""))
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn absolutize(path: &Path, base: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn validate_configured_platform_line(version: &str) -> Result<(), String> {
    let parts = version
        .split('.')
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| format!("invalid tools.platform.version `{version}`"))?;
    if !(2..=4).contains(&parts.len())
        || parts[0] != 8
        || parts[1] != 3
        || parts.get(2).is_some_and(|patch| *patch != 27)
    {
        return Err(format!(
            "configured tools.platform.version `{version}` is incompatible with required {TARGET_PLATFORM_LINE}"
        ));
    }
    Ok(())
}

fn platform_candidates_from_hint(
    hint: &Path,
    utility: PlatformUtility,
) -> Result<Vec<PathBuf>, String> {
    let metadata = fs::symlink_metadata(hint)
        .map_err(|error| format!("failed to inspect {}: {error}", hint.display()))?;
    if metadata_is_link_or_reparse_point(&metadata) {
        return Err(format!(
            "platform hint must not be a symbolic link or reparse point: {}",
            hint.display()
        ));
    }
    if metadata.is_file() {
        let candidate = if hint.file_name() == Some(OsStr::new(utility.executable_name())) {
            hint.to_path_buf()
        } else {
            hint.parent()
                .unwrap_or_else(|| Path::new("."))
                .join(utility.executable_name())
        };
        return Ok(vec![candidate]);
    }
    if !metadata.is_dir() {
        return Err(format!(
            "platform hint is neither a file nor a directory: {}",
            hint.display()
        ));
    }
    let mut candidates = vec![
        hint.join(utility.executable_name()),
        hint.join("bin").join(utility.executable_name()),
    ];
    if let Ok(children) = fs::read_dir(hint) {
        for child in children.flatten() {
            let path = child.path();
            if !path.is_dir() {
                continue;
            }
            candidates.push(path.join(utility.executable_name()));
            candidates.push(path.join("bin").join(utility.executable_name()));
        }
    }
    Ok(candidates)
}

fn default_platform_candidates(utility: PlatformUtility) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for root in default_platform_roots() {
        if let Ok(mut from_root) = platform_candidates_from_hint(&root, utility) {
            candidates.append(&mut from_root);
        }
    }
    if let Some(paths) = env::var_os("PATH") {
        candidates
            .extend(env::split_paths(&paths).map(|path| path.join(utility.executable_name())));
    }
    candidates
}

/// The one enumeration of platform installation roots in the project — reused
/// by `documentation_registry`'s installation resolver in
/// `infrastructure::application_ports` so a second such list never appears.
pub(crate) fn default_platform_roots() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        vec![
            PathBuf::from(r"C:\Program Files\1cv8"),
            PathBuf::from(r"C:\Program Files (x86)\1cv8"),
        ]
    }
    #[cfg(target_os = "linux")]
    {
        vec![
            PathBuf::from("/opt/1cv8/x86_64"),
            PathBuf::from("/opt/1cv8/i386"),
            PathBuf::from("/usr/local/1cv8"),
        ]
    }
    #[cfg(all(not(windows), not(target_os = "linux")))]
    {
        vec![PathBuf::from("/opt/1cv8")]
    }
}

fn verify_platform_candidate(
    candidate: &Path,
    utility: PlatformUtility,
    runner: &dyn ProcessRunner,
    cancellation: &CancellationToken,
) -> Result<VerifiedPlatform, String> {
    verify_platform_candidate_recording_cancellation(candidate, utility, runner, cancellation, None)
}

fn verify_platform_candidate_recording_cancellation(
    candidate: &Path,
    utility: PlatformUtility,
    runner: &dyn ProcessRunner,
    cancellation: &CancellationToken,
    probe_cancelled: Option<&mut bool>,
) -> Result<VerifiedPlatform, String> {
    let metadata = fs::symlink_metadata(candidate)
        .map_err(|error| format!("failed to inspect {}: {error}", candidate.display()))?;
    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
        return Err(format!(
            "platform utility is not a real regular file: {}",
            candidate.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(format!(
                "platform utility is not executable: {}",
                candidate.display()
            ));
        }
    }
    if candidate.file_name() != Some(OsStr::new(utility.executable_name())) {
        return Err(format!(
            "platform utility has an unexpected filename: {}",
            candidate.display()
        ));
    }
    let canonical = fs::canonicalize(candidate)
        .map_err(|error| format!("failed to canonicalize {}: {error}", candidate.display()))?;
    let install_dir = canonical.parent().ok_or_else(|| {
        format!(
            "platform utility has no installation directory: {}",
            canonical.display()
        )
    })?;
    let probe_name = if cfg!(windows) { "ibcmd.exe" } else { "ibcmd" };
    let probe = if utility == PlatformUtility::Ibcmd {
        canonical.clone()
    } else {
        install_dir.join(probe_name)
    };
    verify_regular_executable(&probe, "platform version probe")?;
    if probe.parent() != Some(install_dir) {
        return Err(format!(
            "platform version probe is not a sibling in the exact installation: {}",
            probe.display()
        ));
    }
    let attestation = PlatformAttestation::capture_immutable(&canonical).map_err(|error| {
        format!(
            "platform installation is not immutable and trusted before version probing: {error}"
        )
    })?;
    let output = runner.run(&ProcessCommand {
        program: probe.clone(),
        args: vec!["--version".to_string()],
        cwd: install_dir.to_path_buf(),
        env: Vec::new(),
        env_remove: Vec::new(),
        capture_limits: None,
        timeout: Some(PLATFORM_PROBE_TIMEOUT),
        cancellation: cancellation.clone(),
    })?;
    if output.cancelled || cancellation.is_cancelled() {
        if let Some(probe_cancelled) = probe_cancelled {
            *probe_cancelled = true;
        }
        return Err("platform version probe was cancelled".to_string());
    }
    if !output.status_success {
        return Err(format!(
            "platform version probe failed with status {}: {}",
            output.status,
            redactor(output.stderr.trim())
        ));
    }
    let exact_version = parse_platform_probe_version(&output.stdout, &output.stderr)?;
    let parts = parse_exact_platform_version(&exact_version)
        .expect("platform probe parser returns a four-part version");
    if parts[..3] != [8, 3, 27] {
        return Err(format!(
            "platform version probe for {} reported {exact_version}, required {TARGET_PLATFORM_LINE}.x",
            canonical.display()
        ));
    }
    attestation.recheck().map_err(|error| {
        format!("platform trust changed during version probing and was rejected: {error}")
    })?;
    Ok(VerifiedPlatform {
        executable: canonical,
        exact_version,
        attestation,
    })
}

fn verify_regular_executable(path: &Path, role: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect {role} {}: {error}", path.display()))?;
    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
        return Err(format!(
            "{role} is not a real regular file: {}",
            path.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(format!("{role} is not executable: {}", path.display()));
        }
    }
    Ok(())
}

fn parse_platform_probe_version(stdout: &str, stderr: &str) -> Result<String, String> {
    let mut versions = stdout
        .split(|character: char| !(character.is_ascii_digit() || character == '.'))
        .chain(stderr.split(|character: char| !(character.is_ascii_digit() || character == '.')))
        .filter_map(|token| parse_exact_platform_version(token).map(|_| token.to_string()))
        .collect::<Vec<_>>();
    versions.sort();
    versions.dedup();
    match versions.as_slice() {
        [version] => Ok(version.clone()),
        [] => Err(format!(
            "platform version probe did not report an exact four-part version: {}",
            redactor(stdout.trim())
        )),
        _ => Err(format!(
            "platform version probe reported ambiguous versions: {}",
            versions.join(", ")
        )),
    }
}

fn parse_exact_platform_version(value: &str) -> Option<[u32; 4]> {
    let parts = value
        .split('.')
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    (parts.len() == 4).then(|| [parts[0], parts[1], parts[2], parts[3]])
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::with_effective_config_create_hook;
    #[cfg_attr(windows, allow(unused_imports))]
    use super::{
        normalize_key_value_connection, staged_root_version_policy, validate_staged_dump,
        with_private_create_hook, with_publication_failpoint, with_publication_hook,
        with_secure_read_hook, with_target_parent_capture_hook, with_targeted_read_hooks,
        with_tree_open_hook, Document, FullDumpInvocation, PlatformResolver, PlatformUtility,
        PublicationCheckpoint, StagedRootVersionPolicy, SystemPlatformResolver, TreeSnapshot,
        VerifiedFullDumpAdapter, VerifiedPlatform, STAGED_ROOT_REGISTRY, TARGET_EXPORT_FORMAT,
    };
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::project_sources::SourceSetKind;
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::internal_adapters::{ProcessCommand, ProcessOutput, ProcessRunner};
    use serde_json::{Map, Value};
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn key_value_connection_preserves_quoted_file_path_and_semicolons() {
        let config_dir = Path::new("/workspace/project");
        let connection = r#"File="build/ib;archive";Usr=O'Brien;Pwd="secret;with:semicolon""#;

        let normalized = normalize_key_value_connection(connection, config_dir).unwrap();

        #[cfg(windows)]
        let expected =
            r#"File="/workspace/project\build/ib;archive";Usr=O'Brien;Pwd="secret;with:semicolon""#;
        #[cfg(not(windows))]
        let expected =
            r#"File="/workspace/project/build/ib;archive";Usr=O'Brien;Pwd="secret;with:semicolon""#;
        assert_eq!(normalized, expected);
    }

    struct FixedPlatform {
        executable: PathBuf,
        version: String,
    }

    impl PlatformResolver for FixedPlatform {
        fn resolve(
            &self,
            _effective_config: &serde_yaml::Value,
            _config_dir: &Path,
            _utility: PlatformUtility,
            _runner: &dyn ProcessRunner,
            _cancellation: &CancellationToken,
        ) -> Result<VerifiedPlatform, String> {
            Ok(VerifiedPlatform {
                executable: self.executable.clone(),
                exact_version: self.version.clone(),
                attestation: super::PlatformAttestation::capture_test_fixture(&self.executable)?,
            })
        }
    }

    struct FailingPlatform;

    impl PlatformResolver for FailingPlatform {
        fn resolve(
            &self,
            _effective_config: &serde_yaml::Value,
            _config_dir: &Path,
            _utility: PlatformUtility,
            _runner: &dyn ProcessRunner,
            _cancellation: &CancellationToken,
        ) -> Result<VerifiedPlatform, String> {
            Err("platform 8.3.26 does not match required 8.3.27".to_string())
        }
    }

    enum RunnerMutation {
        None,
        Config(PathBuf),
        Target(PathBuf),
        PlatformFile(PathBuf),
        PlatformInstall(PathBuf),
        #[cfg(unix)]
        RestrictPrivateRoot,
        #[cfg(unix)]
        Symlink,
        #[cfg(unix)]
        SwapPrivateRoot(std::rc::Rc<std::cell::Cell<Option<super::FileIdentity>>>),
    }

    struct DumpRunner {
        owner_xml: String,
        mutation: RunnerMutation,
        calls: AtomicUsize,
    }

    impl DumpRunner {
        fn valid() -> Self {
            Self {
                owner_xml: valid_configuration_owner(),
                mutation: RunnerMutation::None,
                calls: AtomicUsize::new(0),
            }
        }
    }

    struct SwapRestoreProbeRunner {
        install: PathBuf,
        calls: AtomicUsize,
    }

    impl ProcessRunner for SwapRestoreProbeRunner {
        fn run(&self, _command: &ProcessCommand) -> Result<ProcessOutput, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let displaced = self.install.with_extension("swap-restore-displaced");
            std::fs::rename(&self.install, &displaced).unwrap();
            std::fs::create_dir(&self.install).unwrap();
            std::fs::remove_dir(&self.install).unwrap();
            std::fs::rename(displaced, &self.install).unwrap();
            Ok(ProcessOutput {
                status_success: true,
                status: "0".to_string(),
                stdout: "8.3.27.2074".to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            })
        }
    }

    impl ProcessRunner for DumpRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let config_index = command
                .args
                .iter()
                .position(|arg| arg == "--config")
                .expect("effective config flag");
            let config_path = PathBuf::from(&command.args[config_index + 1]);
            let config: serde_yaml::Value =
                serde_yaml::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
            assert!(
                config.get("basePath").is_none(),
                "pinned v8-runner rejects the removed basePath key"
            );
            assert_eq!(config["format"].as_str(), Some("DESIGNER"));
            assert!(Path::new(config["workPath"].as_str().unwrap()).is_absolute());
            let connection = config["infobase"]["connection"].as_str().unwrap();
            if connection.starts_with("/F ") {
                assert!(
                    Path::new(connection.trim_start_matches("/F ")).is_absolute(),
                    "{connection}"
                );
            } else {
                assert!(
                    connection.to_ascii_lowercase().starts_with("file="),
                    "{connection}"
                );
            }
            assert_eq!(
                config["tools"]["platform"]["version"].as_str(),
                Some("8.3.27.2074")
            );
            assert!(config["tools"]["platform"]["path"]
                .as_str()
                .unwrap()
                .contains("8.3.27.2074"));
            let source_sets = config["source-set"].as_sequence().unwrap();
            let target = PathBuf::from(source_sets[0]["path"].as_str().unwrap());
            assert!(
                target
                    .file_name()
                    .is_some_and(|name| name == "staged-source"),
                "{}",
                target.display()
            );
            std::fs::create_dir_all(&target).unwrap();
            std::fs::write(target.join("Configuration.xml"), self.owner_xml.as_bytes()).unwrap();
            std::fs::write(
                target.join("Catalogs.xml"),
                format!(
                    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{TARGET_EXPORT_FORMAT}"><Catalog/></MetaDataObject>"#
                ),
            )
            .unwrap();
            match &self.mutation {
                RunnerMutation::None => {}
                RunnerMutation::Config(path) => {
                    std::fs::write(path, b"changed during child execution").unwrap();
                }
                RunnerMutation::Target(path) => {
                    std::fs::write(path.join("concurrent.txt"), b"concurrent").unwrap();
                }
                RunnerMutation::PlatformFile(path) => {
                    std::fs::write(path, b"replaced platform executable").unwrap();
                }
                RunnerMutation::PlatformInstall(path) => {
                    let displaced = path.with_extension("displaced");
                    std::fs::rename(path, &displaced).unwrap();
                    std::fs::create_dir_all(path).unwrap();
                    make_platform_executable(&path.join("1cv8"));
                    make_platform_executable(&path.join("ibcmd"));
                }
                #[cfg(unix)]
                RunnerMutation::RestrictPrivateRoot => {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(
                        config_path.parent().unwrap(),
                        std::fs::Permissions::from_mode(0o500),
                    )
                    .unwrap();
                }
                #[cfg(unix)]
                RunnerMutation::Symlink => {
                    std::os::unix::fs::symlink(
                        target.join("Configuration.xml"),
                        target.join("unsafe-link.xml"),
                    )
                    .unwrap();
                }
                #[cfg(unix)]
                RunnerMutation::SwapPrivateRoot(original_identity) => {
                    use std::os::unix::fs::PermissionsExt;

                    let private_root = config_path
                        .parent()
                        .and_then(Path::parent)
                        .expect("effective config is under the private root");
                    original_identity.set(Some(
                        super::file_identity(
                            &super::open_directory_nofollow(private_root).unwrap(),
                        )
                        .unwrap(),
                    ));
                    let displaced_name = format!(
                        "{}-displaced",
                        private_root.file_name().unwrap().to_string_lossy()
                    );
                    let displaced = private_root.with_file_name(displaced_name);
                    std::fs::rename(private_root, &displaced).unwrap();
                    std::fs::create_dir(private_root).unwrap();
                    std::fs::set_permissions(private_root, std::fs::Permissions::from_mode(0o700))
                        .unwrap();
                    for child in ["execution", "recovery"] {
                        let path = private_root.join(child);
                        std::fs::create_dir(&path).unwrap();
                        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
                            .unwrap();
                    }
                    std::fs::write(
                        private_root.join("execution/foreign.txt"),
                        b"foreign root sentinel",
                    )
                    .unwrap();
                }
            }
            Ok(ProcessOutput {
                status_success: true,
                status: "0".to_string(),
                stdout: "ok".to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            })
        }
    }

    fn valid_configuration_owner() -> String {
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties/></Configuration></MetaDataObject>"#.to_string()
    }

    fn seed_valid_target(target: &Path) {
        std::fs::create_dir_all(target).unwrap();
        std::fs::write(
            target.join("Configuration.xml"),
            valid_configuration_owner(),
        )
        .unwrap();
        std::fs::write(target.join("before.txt"), b"before").unwrap();
    }

    fn workspace(name: &str) -> (PathBuf, WorkspaceContext, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "unica-verified-full-dump-{name}-{}",
            uuid::Uuid::new_v4()
        ));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let config = workspace.join("v8project.yaml");
        std::fs::write(
            &config,
            "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: '/F base'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace,
            cache_root: root.join("cache"),
            workspace_epoch: 0,
        };
        (root, context, config)
    }

    fn fixed_platform(root: &Path) -> FixedPlatform {
        let install = root.join("8.3.27.2074");
        let executable = install.join(PlatformUtility::Designer.executable_name());
        make_platform_executable(&executable);
        make_platform_executable(&install.join(PlatformUtility::Ibcmd.executable_name()));
        FixedPlatform {
            executable,
            version: "8.3.27.2074".to_string(),
        }
    }

    fn make_platform_executable(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"#!/bin/sh\nprintf '8.3.27.2074\\n'\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(path, permissions).unwrap();
        }
    }

    fn invoke_with_args(
        runner: &DumpRunner,
        platform: &dyn PlatformResolver,
        invocation: FullDumpInvocation,
        context: &WorkspaceContext,
        args: &Map<String, Value>,
    ) -> crate::application::AdapterOutcome {
        VerifiedFullDumpAdapter::with_dependencies(
            runner,
            platform,
            PathBuf::from("fake-v8-runner"),
        )
        .invoke(
            match invocation {
                FullDumpInvocation::BuildDump => "unica.build.dump",
                FullDumpInvocation::RuntimeExecute => "unica.runtime.execute",
            },
            invocation,
            args,
            context,
            &CancellationToken::new(),
        )
        .unwrap()
    }

    fn args() -> Map<String, Value> {
        Map::from_iter([
            ("dryRun".to_string(), Value::Bool(false)),
            ("mode".to_string(), Value::String("full".to_string())),
        ])
    }

    fn invoke(
        runner: &DumpRunner,
        platform: &dyn PlatformResolver,
        invocation: FullDumpInvocation,
        context: &WorkspaceContext,
    ) -> crate::application::AdapterOutcome {
        invoke_with_args(runner, platform, invocation, context, &args())
    }

    fn assert_retained_directory_artifact(
        result: &crate::application::AdapterOutcome,
        retained: &Path,
    ) {
        let expected = std::fs::canonicalize(retained).unwrap();
        let expected_identity =
            super::file_identity(&super::open_directory_nofollow(&expected).unwrap()).unwrap();
        assert!(
            result.artifacts.iter().any(|artifact| {
                std::fs::canonicalize(artifact).is_ok_and(|actual| {
                    actual == expected
                        && super::open_directory_nofollow(&actual).is_ok_and(|directory| {
                            super::file_identity(&directory)
                                .is_ok_and(|identity| identity == expected_identity)
                        })
                })
            }),
            "a recovery artifact must name the retained directory identity"
        );
    }

    #[test]
    fn effective_config_preparation_failure_reports_retained_foreign_data() {
        let (root, context, _) = workspace("effective-config-prepare-retention");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let before = super::capture_tree_target_nofollow(&target).unwrap();
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let retained = std::rc::Rc::new(std::cell::RefCell::new(None));
        let hook_retained = std::rc::Rc::clone(&retained);

        let result = super::with_effective_config_prepare_hook(
            move |path| {
                std::fs::create_dir(path).unwrap();
                std::fs::write(path.join("foreign.txt"), b"foreign sentinel").unwrap();
                let private = path.parent().unwrap().parent().unwrap().to_path_buf();
                let snapshot = super::capture_tree_target_nofollow(&private).unwrap();
                *hook_retained.borrow_mut() = Some((private, snapshot));
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok);
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        assert!(result.changes.is_empty());
        assert_eq!(
            super::capture_tree_target_nofollow(&target).unwrap(),
            before
        );
        let retained = retained.borrow();
        let (private, snapshot) = retained.as_ref().expect("preparation hook must run");
        assert_eq!(
            super::capture_tree_target_nofollow(private).unwrap(),
            *snapshot,
            "preparation cleanup must preserve all retained identities and bytes"
        );
        assert_eq!(
            std::fs::read(
                private
                    .join("execution")
                    .join(super::EFFECTIVE_CONFIG_NAME)
                    .join("foreign.txt")
            )
            .unwrap(),
            b"foreign sentinel"
        );
        assert!(result
            .errors
            .iter()
            .any(|error| error.contains("failed to securely create private effective config")));
        assert!(result
            .summary
            .contains("failed before verified dump publication"));
        let expected = std::fs::canonicalize(private).unwrap();
        let has_recovery_artifact = result
            .artifacts
            .iter()
            .any(|artifact| std::fs::canonicalize(artifact).is_ok_and(|path| path == expected));
        let reports_discarded = result
            .warnings
            .iter()
            .any(|warning| warning.contains("the private staged dump was discarded"));
        assert!(
            has_recovery_artifact && !reports_discarded,
            "retained preparation data needs recovery metadata: artifact={has_recovery_artifact}, discarded={reports_discarded}"
        );
        assert_retained_directory_artifact(&result, private);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_applied_full_dump_reaches_runner_and_commits_validated_tree() {
        let (root, context, _) = workspace("windows-applied-full-dump");
        let target = context.cwd.join("src");
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();

        let result = invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context);

        assert!(result.ok, "{result:?}");
        assert_eq!(runner.calls.load(Ordering::SeqCst), 1);
        assert!(target.join("Configuration.xml").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_applied_full_dump_rejects_effective_config_acl_mutation_before_runner_and_cleans_created_identity(
    ) {
        use std::os::windows::ffi::OsStrExt;
        use std::ptr;
        use windows_sys::Win32::Security::Authorization::{SetNamedSecurityInfoW, SE_FILE_OBJECT};
        use windows_sys::Win32::Security::{
            DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        };

        let (root, context, _) = workspace("windows-effective-config-acl-mutation");
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let displaced = context.cwd.join("created-effective-config.yaml");
        let hook_displaced = displaced.clone();

        let result = with_effective_config_create_hook(
            move |path| {
                std::fs::rename(path, &hook_displaced).unwrap();
                let mut path = hook_displaced.as_os_str().encode_wide().collect::<Vec<_>>();
                path.push(0);
                // SAFETY: path is NUL-terminated and names the newly created file. A null DACL is
                // intentional test corruption that the production verifier must reject.
                let status = unsafe {
                    SetNamedSecurityInfoW(
                        path.as_ptr(),
                        SE_FILE_OBJECT,
                        DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                        ptr::null_mut(),
                        ptr::null_mut(),
                        ptr::null(),
                        ptr::null(),
                    )
                };
                assert_eq!(
                    status,
                    0,
                    "{}",
                    std::io::Error::from_raw_os_error(status as i32)
                );
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        assert!(
            !displaced.exists(),
            "the originally created identity escaped cleanup: {}",
            displaced.display()
        );
        assert!(std::fs::read_dir(&context.cwd).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".unica-dump-guard-")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_applied_full_dump_rejects_effective_config_name_swap_before_runner_and_preserves_replacement_identity(
    ) {
        use std::cell::Cell;

        let (root, context, _) = workspace("windows-effective-config-name-swap");
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let displaced = context.cwd.join("created-effective-config.yaml");
        let replacement = context.cwd.join("replacement-effective-config.yaml");
        let target = context.cwd.join("src");
        let before = super::capture_tree_target_nofollow(&target).unwrap();
        std::fs::write(&replacement, b"replacement sentinel").unwrap();
        let expected_replacement_identity =
            crate::infrastructure::platform::filesystem::file_identity(
                &std::fs::File::open(&replacement).unwrap(),
            )
            .unwrap();
        let created_identity = std::rc::Rc::new(Cell::new(None));
        let retained = std::rc::Rc::new(std::cell::RefCell::new(None));
        let hook_retained = std::rc::Rc::clone(&retained);
        let hook_created_identity = std::rc::Rc::clone(&created_identity);
        let hook_displaced = displaced.clone();
        let hook_replacement = replacement.clone();

        let result = with_effective_config_create_hook(
            move |path| {
                hook_created_identity.set(Some(
                    crate::infrastructure::platform::filesystem::file_identity(
                        &std::fs::File::open(path).unwrap(),
                    )
                    .unwrap(),
                ));
                std::fs::rename(path, &hook_displaced).unwrap();
                std::fs::hard_link(&hook_replacement, path).unwrap();
                let execution = path.parent().unwrap();
                let private = execution.parent().unwrap();
                let directories = [
                    private.to_path_buf(),
                    execution.to_path_buf(),
                    private.join("recovery"),
                ]
                .into_iter()
                .map(|directory| {
                    let identity = super::file_identity(
                        &super::open_directory_nofollow(
                            &std::fs::canonicalize(&directory).unwrap(),
                        )
                        .unwrap(),
                    )
                    .unwrap();
                    (directory, identity)
                })
                .collect::<Vec<_>>();
                *hook_retained.borrow_mut() = Some((path.to_path_buf(), directories));
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok);
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        assert!(
            created_identity.get().is_some(),
            "the post-create hook did not run"
        );
        assert!(
            !displaced.exists(),
            "the originally created identity escaped cleanup: {}",
            displaced.display()
        );
        assert_eq!(
            crate::infrastructure::platform::filesystem::file_identity(
                &std::fs::File::open(&replacement).unwrap(),
            )
            .unwrap(),
            expected_replacement_identity,
            "cleanup followed the replaced effective-config name"
        );
        assert_eq!(
            std::fs::read(&replacement).unwrap(),
            b"replacement sentinel"
        );
        assert!(result.changes.is_empty());
        assert_eq!(
            super::capture_tree_target_nofollow(&target).unwrap(),
            before
        );
        let retained = retained.borrow();
        let (inside, directories) = retained.as_ref().expect("post-create hook must run");
        assert_eq!(
            super::file_identity(&std::fs::File::open(inside).unwrap()).unwrap(),
            expected_replacement_identity,
            "the substituted private name must still refer to the foreign file"
        );
        assert_ne!(Some(expected_replacement_identity), created_identity.get());
        assert_eq!(std::fs::read(inside).unwrap(), b"replacement sentinel");
        for (directory, expected_identity) in directories {
            assert_eq!(
                super::file_identity(
                    &super::open_directory_nofollow(&std::fs::canonicalize(directory).unwrap())
                        .unwrap(),
                )
                .unwrap(),
                *expected_identity,
                "cleanup must retain every original private directory identity"
            );
        }
        let private = &directories[0].0;
        let retained_roots = std::fs::read_dir(&context.cwd)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(".unica-dump-guard-"))
            })
            .map(|path| std::fs::canonicalize(path).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            retained_roots,
            vec![std::fs::canonicalize(private).unwrap()]
        );
        assert_retained_directory_artifact(&result, private);
        assert!(result.errors.iter().any(|error| error
            .contains("private effective config identity changed before verified reopen")));
        assert!(result
            .summary
            .contains("failed before verified dump publication"));
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("Data retained for recovery")));
        assert!(!result
            .warnings
            .iter()
            .any(|warning| warning.contains("the private staged dump was discarded")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn selected_source_set_target_must_be_relative_and_physically_contained_in_workspace() {
        for case in [
            "absolute",
            "parent-escape",
            "symlink-parent",
            "target-symlink",
        ] {
            let (root, context, config) = workspace(case);
            let outside = root.join("outside");
            std::fs::create_dir_all(&outside).unwrap();
            let configured_path = match case {
                "absolute" => outside.join("absolute-src").display().to_string(),
                "parent-escape" => "../../outside/parent-src".to_string(),
                "symlink-parent" => {
                    #[cfg(unix)]
                    std::os::unix::fs::symlink(&outside, context.cwd.join("linked")).unwrap();
                    #[cfg(windows)]
                    std::os::windows::fs::symlink_dir(&outside, context.cwd.join("linked"))
                        .unwrap();
                    #[cfg(not(any(unix, windows)))]
                    std::fs::create_dir_all(context.cwd.join("linked")).unwrap();
                    "linked/symlink-src".to_string()
                }
                "target-symlink" => {
                    let real_target = context.cwd.join("real-target");
                    std::fs::create_dir_all(&real_target).unwrap();
                    #[cfg(unix)]
                    std::os::unix::fs::symlink(&real_target, context.cwd.join("src")).unwrap();
                    #[cfg(windows)]
                    std::os::windows::fs::symlink_dir(&real_target, context.cwd.join("src"))
                        .unwrap();
                    #[cfg(not(any(unix, windows)))]
                    std::fs::create_dir_all(context.cwd.join("src")).unwrap();
                    "src".to_string()
                }
                _ => unreachable!(),
            };
            std::fs::write(
                &config,
                format!(
                    "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: '/F base'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: '{configured_path}'\n"
                ),
            )
            .unwrap();
            let platform = fixed_platform(&root);
            let runner = DumpRunner::valid();

            let result = invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context);

            assert!(!result.ok, "{case}: {result:?}");
            assert_eq!(runner.calls.load(Ordering::SeqCst), 0, "{case}");
            assert!(
                result.errors.join("\n").contains("workspace")
                    || result.errors.join("\n").contains("relative")
                    || result.errors.join("\n").contains("symbolic link")
                    || result.errors.join("\n").contains("real directory"),
                "{case}: {result:?}"
            );
            assert!(!outside.join("absolute-src/Configuration.xml").exists());
            assert!(!outside.join("parent-src/Configuration.xml").exists());
            assert!(!outside.join("symlink-src/Configuration.xml").exists());
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn target_parent_symlink_swap_after_text_normalization_cannot_escape_workspace_anchor() {
        let (root, context, config) = workspace("target-parent-normalization-swap");
        std::fs::write(
            &config,
            "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: '/F base'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: inside/src\n",
        )
        .unwrap();
        let inside = context.cwd.join("inside");
        let displaced = context.cwd.join("inside-displaced");
        let outside = root.join("outside");
        std::fs::create_dir(&inside).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let hook_inside = inside.clone();
        let hook_displaced = displaced.clone();
        let hook_outside = outside.clone();
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();

        let result = with_target_parent_capture_hook(
            move |_normalized_parent| {
                std::fs::rename(&hook_inside, &hook_displaced).unwrap();
                std::os::unix::fs::symlink(&hook_outside, &hook_inside).unwrap();
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        assert!(!outside.join("src").exists(), "{result:?}");
        assert!(
            std::fs::read_dir(&outside).unwrap().all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".unica-dump-guard-")),
            "{result:?}"
        );
        assert!(
            result.errors.join("\n").contains("workspace")
                || result.errors.join("\n").contains("link"),
            "{result:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn physical_edt_or_mixed_target_cannot_be_replaced_by_designer_yaml_claim() {
        for case in ["edt", "mixed"] {
            let (root, context, _) = workspace(case);
            let target = context.cwd.join("src");
            std::fs::create_dir_all(target.join("Configuration")).unwrap();
            std::fs::write(target.join(".project"), b"<projectDescription/>").unwrap();
            std::fs::write(
                target.join("Configuration/Configuration.mdo"),
                b"<mdclass:Configuration/>",
            )
            .unwrap();
            if case == "mixed" {
                std::fs::write(
                    target.join("Configuration.xml"),
                    valid_configuration_owner(),
                )
                .unwrap();
            }
            let platform = fixed_platform(&root);
            let runner = DumpRunner::valid();

            let result = invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context);

            assert!(!result.ok, "{case}: {result:?}");
            assert_eq!(runner.calls.load(Ordering::SeqCst), 0, "{case}");
            assert!(target.join(".project").is_file(), "{case}");
            assert!(
                result.errors.join("\n").contains("EDT")
                    || result.errors.join("\n").contains("mixed")
                    || result.errors.join("\n").contains("invalid"),
                "{case}: {result:?}"
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn existing_legacy_designer_target_requires_explicit_user_migration() {
        let (root, context, _) = workspace("legacy-designer-target");
        let target = context.cwd.join("src");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(
            target.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Configuration><Properties/></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();

        let result = invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context);

        assert!(!result.ok, "{result:?}");
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        assert!(
            result
                .errors
                .join("\n")
                .contains("not be migrated automatically")
                && result.errors.join("\n").contains("8.3.27"),
            "{result:?}"
        );
        assert!(std::fs::read_to_string(target.join("Configuration.xml"))
            .unwrap()
            .contains("2.19"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn both_synchronous_routes_publish_only_validated_private_stage() {
        for invocation in [
            FullDumpInvocation::BuildDump,
            FullDumpInvocation::RuntimeExecute,
        ] {
            let (root, context, _) = workspace(match invocation {
                FullDumpInvocation::BuildDump => "build-success",
                FullDumpInvocation::RuntimeExecute => "runtime-success",
            });
            let platform = fixed_platform(&root);
            let runner = DumpRunner::valid();

            let result = invoke(&runner, &platform, invocation, &context);

            assert!(result.ok, "{invocation:?}: {result:?}");
            assert!(context.cwd.join("src/Configuration.xml").is_file());
            assert_eq!(runner.calls.load(Ordering::SeqCst), 1);
            assert!(
                std::fs::read_dir(&context.cwd).unwrap().all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".unica-dump-guard-")),
                "private stage debris remained"
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[derive(Clone, Copy)]
    enum IncompleteRunnerFinish {
        RunError,
        TimedOut,
        Cancelled,
        FailedExit,
        Panic,
        SpawnFailure,
    }

    struct WaitingDumpWriter {
        release: Option<std::sync::mpsc::Sender<()>>,
        writer: Option<std::thread::JoinHandle<std::io::Result<()>>>,
    }

    impl WaitingDumpWriter {
        fn finish(&mut self) -> std::io::Result<()> {
            if let Some(release) = self.release.take() {
                let _ = release.send(());
            }
            self.writer
                .take()
                .expect("writer is retained")
                .join()
                .unwrap()
        }
    }

    impl Drop for WaitingDumpWriter {
        fn drop(&mut self) {
            if let Some(release) = self.release.take() {
                let _ = release.send(());
            }
            if let Some(writer) = self.writer.take() {
                let _ = writer.join();
            }
        }
    }

    struct IncompleteDumpRunner {
        finish: IncompleteRunnerFinish,
        create_stage: bool,
        private_paths: std::cell::RefCell<Option<(PathBuf, PathBuf, PathBuf)>>,
        pending_writer: std::cell::RefCell<Option<WaitingDumpWriter>>,
        spawn_output_snapshot: std::cell::RefCell<Option<TreeSnapshot>>,
    }

    impl IncompleteDumpRunner {
        fn new(finish: IncompleteRunnerFinish, create_stage: bool) -> Self {
            Self {
                finish,
                create_stage,
                private_paths: std::cell::RefCell::new(None),
                pending_writer: std::cell::RefCell::new(None),
                spawn_output_snapshot: std::cell::RefCell::new(None),
            }
        }

        fn remember_paths(&self, command: &ProcessCommand) -> PathBuf {
            let config_index = command
                .args
                .iter()
                .position(|arg| arg == "--config")
                .unwrap();
            let config_path = PathBuf::from(&command.args[config_index + 1]);
            let config: serde_yaml::Value =
                serde_yaml::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
            let stage = PathBuf::from(config["source-set"][0]["path"].as_str().unwrap());
            let private_root = config_path
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .to_path_buf();
            self.private_paths
                .replace(Some((private_root, config_path, stage.clone())));
            stage
        }
    }

    impl ProcessRunner for IncompleteDumpRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            let stage = self.remember_paths(command);
            if matches!(self.finish, IncompleteRunnerFinish::SpawnFailure) {
                return Err("the operating system refused to launch the runner".into());
            }
            if self.create_stage {
                std::fs::create_dir(&stage).unwrap();
            }
            let (release, wait) = std::sync::mpsc::channel();
            let writer = std::thread::spawn(move || {
                wait.recv().unwrap();
                if !stage.exists() {
                    // A writer may create its own output, but cannot recreate
                    // the enclosing attempt after premature cleanup.
                    std::fs::create_dir(&stage)?;
                }
                std::fs::write(stage.join("late-writer.txt"), b"data after runner returned")
            });
            self.pending_writer.replace(Some(WaitingDumpWriter {
                release: Some(release),
                writer: Some(writer),
            }));
            match self.finish {
                IncompleteRunnerFinish::RunError => {
                    Err("runner wait failed before writer completion was confirmed".into())
                }
                IncompleteRunnerFinish::Panic => panic!("injected runner unwind"),
                _ => Ok(ProcessOutput {
                    status_success: false,
                    status: "1".into(),
                    stdout: String::new(),
                    stderr: "runner completion was not confirmed".into(),
                    timed_out: matches!(self.finish, IncompleteRunnerFinish::TimedOut),
                    cancelled: matches!(self.finish, IncompleteRunnerFinish::Cancelled),
                    stdout_truncated: false,
                    stderr_truncated: false,
                    stdout_had_invalid_utf8: false,
                    stderr_had_invalid_utf8: false,
                }),
            }
        }

        fn run_staged(
            &self,
            command: &ProcessCommand,
        ) -> Result<ProcessOutput, crate::infrastructure::internal_adapters::ProcessStreamFailure>
        {
            if matches!(self.finish, IncompleteRunnerFinish::SpawnFailure) {
                let stage = self.remember_paths(command);
                if self.create_stage {
                    std::fs::create_dir(&stage).unwrap();
                    std::fs::write(stage.join("foreign.txt"), b"unowned spawn-failure sentinel")
                        .unwrap();
                    self.spawn_output_snapshot
                        .replace(Some(super::capture_tree_target_nofollow(&stage).unwrap()));
                }
                return Err(
                    crate::infrastructure::internal_adapters::ProcessStreamFailure::Spawn(
                        "the operating system refused to launch the runner".into(),
                    ),
                );
            }
            self.run(command)
                .map_err(crate::infrastructure::internal_adapters::ProcessStreamFailure::Run)
        }
    }

    #[test]
    fn unconfirmed_runner_retains_private_stage_for_late_writer() {
        for finish in [
            IncompleteRunnerFinish::RunError,
            IncompleteRunnerFinish::TimedOut,
            IncompleteRunnerFinish::Cancelled,
            IncompleteRunnerFinish::FailedExit,
        ] {
            for create_stage in [false, true] {
                let (root, context, _) = workspace("unconfirmed-runner-late-writer");
                let target = context.cwd.join("src");
                seed_valid_target(&target);
                let before = super::capture_tree_target_nofollow(&target).unwrap();
                let platform = fixed_platform(&root);
                let runner = IncompleteDumpRunner::new(finish, create_stage);
                let result = VerifiedFullDumpAdapter::with_dependencies(
                    &runner,
                    &platform,
                    PathBuf::from("fake-v8-runner"),
                )
                .invoke(
                    "unica.build.dump",
                    FullDumpInvocation::BuildDump,
                    &args(),
                    &context,
                    &CancellationToken::new(),
                )
                .unwrap();
                assert!(!result.ok);
                let (private_root, config_path, stage) =
                    runner.private_paths.borrow().clone().unwrap();
                assert!(
                    private_root.is_dir(),
                    "a live writer lost its attempt directory"
                );
                assert!(
                    std::fs::read(&config_path).unwrap().is_empty(),
                    "held configuration must be empty"
                );
                assert!(result
                    .artifacts
                    .contains(&private_root.display().to_string()));
                runner
                    .pending_writer
                    .borrow_mut()
                    .as_mut()
                    .unwrap()
                    .finish()
                    .unwrap();
                assert_eq!(
                    std::fs::read(stage.join("late-writer.txt")).unwrap(),
                    b"data after runner returned"
                );
                assert_eq!(
                    super::capture_tree_target_nofollow(&target).unwrap(),
                    before
                );
                std::fs::remove_dir_all(root).unwrap();
            }
        }
    }

    #[test]
    fn runner_unwind_retains_private_stage_for_late_writer() {
        let (root, context, _) = workspace("runner-unwind-late-writer");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let before = super::capture_tree_target_nofollow(&target).unwrap();
        let platform = fixed_platform(&root);
        let runner = IncompleteDumpRunner::new(IncompleteRunnerFinish::Panic, false);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            VerifiedFullDumpAdapter::with_dependencies(
                &runner,
                &platform,
                PathBuf::from("fake-v8-runner"),
            )
            .invoke(
                "unica.build.dump",
                FullDumpInvocation::BuildDump,
                &args(),
                &context,
                &CancellationToken::new(),
            )
        }));
        assert!(result.is_err());
        let (private_root, config_path, stage) = runner.private_paths.borrow().clone().unwrap();
        assert!(
            private_root.is_dir(),
            "unwinding deleted a live writer's attempt directory"
        );
        assert!(
            std::fs::read(&config_path).unwrap().is_empty(),
            "held configuration must be empty"
        );
        runner
            .pending_writer
            .borrow_mut()
            .as_mut()
            .unwrap()
            .finish()
            .unwrap();
        assert_eq!(
            std::fs::read(stage.join("late-writer.txt")).unwrap(),
            b"data after runner returned"
        );
        assert_eq!(
            super::capture_tree_target_nofollow(&target).unwrap(),
            before
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn confirmed_spawn_failure_preserves_unexpected_staged_output() {
        let (root, context, _) = workspace("spawn-failure-foreign-output");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let before = super::capture_tree_target_nofollow(&target).unwrap();
        let platform = fixed_platform(&root);
        let runner = IncompleteDumpRunner::new(IncompleteRunnerFinish::SpawnFailure, true);

        let result = VerifiedFullDumpAdapter::with_dependencies(
            &runner,
            &platform,
            PathBuf::from("fake-v8-runner"),
        )
        .invoke(
            "unica.build.dump",
            FullDumpInvocation::BuildDump,
            &args(),
            &context,
            &CancellationToken::new(),
        )
        .unwrap();

        assert!(!result.ok, "{result:?}");
        assert!(result.changes.is_empty());
        assert!(result.errors.join("\n").contains("refused to launch"));
        let (private, config, stage) = runner.private_paths.borrow().clone().unwrap();
        assert!(!config.exists());
        assert_eq!(
            super::capture_tree_target_nofollow(&stage).unwrap(),
            *runner.spawn_output_snapshot.borrow().as_ref().unwrap(),
            "a failure to spawn cannot grant ownership of unexpected output"
        );
        assert_eq!(
            std::fs::read(stage.join("foreign.txt")).unwrap(),
            b"unowned spawn-failure sentinel"
        );
        assert_eq!(
            super::capture_tree_target_nofollow(&target).unwrap(),
            before
        );
        assert_retained_directory_artifact(&result, &private);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staged_replacement_after_completion_capture_remains_unowned() {
        let (root, context, _) = workspace("staged-replacement-after-completion");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let before = super::capture_tree_target_nofollow(&target).unwrap();
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let replacement = root.join("foreign-staged-source");
        std::fs::create_dir(&replacement).unwrap();
        std::fs::write(
            replacement.join("Configuration.xml"),
            valid_configuration_owner(),
        )
        .unwrap();
        std::fs::write(
            replacement.join("Catalogs.xml"),
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{TARGET_EXPORT_FORMAT}"><Catalog/></MetaDataObject>"#
            ),
        )
        .unwrap();
        let expected_replacement = super::capture_tree_target_nofollow(&replacement).unwrap();
        let displaced = root.join("original-staged-source");
        let retained = std::rc::Rc::new(std::cell::RefCell::new(None));
        let hook_retained = std::rc::Rc::clone(&retained);
        let hook_workspace = context.cwd.clone();
        let hook_displaced = displaced.clone();

        let result = with_publication_hook(
            PublicationCheckpoint::AfterStagedCapture,
            move || {
                let private = std::fs::read_dir(&hook_workspace)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .find(|path| {
                        path.file_name()
                            .unwrap()
                            .to_string_lossy()
                            .starts_with(".unica-dump-guard-")
                    })
                    .unwrap();
                let stage = private.join("execution/staged-source");
                let original = super::capture_tree_target_nofollow(&stage).unwrap();
                std::fs::rename(&stage, &hook_displaced).unwrap();
                std::fs::rename(&replacement, &stage).unwrap();
                *hook_retained.borrow_mut() = Some((private, stage, original));
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        assert!(result.changes.is_empty());
        assert!(
            result
                .errors
                .join("\n")
                .contains("bytes or identity changed"),
            "validation must use the first captured owner identity: {result:?}"
        );
        assert_eq!(
            super::capture_tree_target_nofollow(&target).unwrap(),
            before
        );
        let retained = retained.borrow();
        let (private, stage, original) = retained.as_ref().expect("capture hook must run");
        assert_eq!(
            super::capture_tree_target_nofollow(stage).unwrap(),
            expected_replacement,
            "cleanup must preserve every identity and byte of the replacement"
        );
        assert_eq!(
            super::capture_tree_target_nofollow(&displaced).unwrap(),
            *original,
            "the first observed tree must not be reclassified as the replacement"
        );
        assert_retained_directory_artifact(&result, private);
        std::fs::remove_dir_all(root).unwrap();
    }

    struct EmptySuccessfulDumpRunner;

    impl ProcessRunner for EmptySuccessfulDumpRunner {
        fn run(&self, _command: &ProcessCommand) -> Result<ProcessOutput, String> {
            Ok(ProcessOutput {
                status_success: true,
                status: "0".into(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            })
        }
    }

    #[test]
    fn output_appearing_after_absent_completion_capture_remains_unowned() {
        let (root, context, _) = workspace("staged-appearance-after-completion");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let before = super::capture_tree_target_nofollow(&target).unwrap();
        let platform = fixed_platform(&root);
        let retained = std::rc::Rc::new(std::cell::RefCell::new(None));
        let hook_retained = std::rc::Rc::clone(&retained);
        let hook_workspace = context.cwd.clone();

        let result = with_publication_hook(
            PublicationCheckpoint::AfterStagedCapture,
            move || {
                let private = std::fs::read_dir(&hook_workspace)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .find(|path| {
                        path.file_name()
                            .unwrap()
                            .to_string_lossy()
                            .starts_with(".unica-dump-guard-")
                    })
                    .unwrap();
                let stage = private.join("execution/staged-source");
                assert!(!stage.exists());
                std::fs::create_dir(&stage).unwrap();
                std::fs::write(stage.join("Configuration.xml"), valid_configuration_owner())
                    .unwrap();
                std::fs::write(stage.join("foreign.txt"), b"appeared after observation").unwrap();
                let snapshot = super::capture_tree_target_nofollow(&stage).unwrap();
                *hook_retained.borrow_mut() = Some((private, stage, snapshot));
            },
            || {
                VerifiedFullDumpAdapter::with_dependencies(
                    &EmptySuccessfulDumpRunner,
                    &platform,
                    PathBuf::from("fake-v8-runner"),
                )
                .invoke(
                    "unica.build.dump",
                    FullDumpInvocation::BuildDump,
                    &args(),
                    &context,
                    &CancellationToken::new(),
                )
                .unwrap()
            },
        );

        assert!(!result.ok, "{result:?}");
        assert!(result.changes.is_empty());
        assert!(result.errors.join("\n").contains("did not create"));
        assert_eq!(
            super::capture_tree_target_nofollow(&target).unwrap(),
            before
        );
        let retained = retained.borrow();
        let (private, stage, snapshot) = retained.as_ref().expect("capture hook must run");
        assert_eq!(
            super::capture_tree_target_nofollow(stage).unwrap(),
            *snapshot
        );
        assert_retained_directory_artifact(&result, private);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn confirmed_spawn_failure_cleans_unused_private_stage() {
        let (root, context, _) = workspace("spawn-failure-cleans-stage");
        let platform = fixed_platform(&root);
        let runner = IncompleteDumpRunner::new(IncompleteRunnerFinish::SpawnFailure, false);
        let result = VerifiedFullDumpAdapter::with_dependencies(
            &runner,
            &platform,
            PathBuf::from("fake-v8-runner"),
        )
        .invoke(
            "unica.build.dump",
            FullDumpInvocation::BuildDump,
            &args(),
            &context,
            &CancellationToken::new(),
        )
        .unwrap();
        assert!(!result.ok);
        let (private_root, config_path, _) = runner.private_paths.borrow().clone().unwrap();
        assert!(!private_root.exists());
        assert!(!config_path.exists());
        assert!(result.artifacts.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn private_root_creation_is_bound_to_the_captured_target_parent() {
        let (root, context, _) = workspace("private-create-parent-swap");
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let visible_workspace = context.cwd.clone();
        let displaced_workspace = root.join("workspace-displaced");
        let hook_visible_workspace = visible_workspace.clone();
        let hook_displaced_workspace = displaced_workspace.clone();

        let result = with_private_create_hook(
            move |_private_root| {
                std::fs::rename(&hook_visible_workspace, &hook_displaced_workspace).unwrap();
                std::fs::create_dir(&hook_visible_workspace).unwrap();
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        for workspace in [&visible_workspace, &displaced_workspace] {
            assert!(
                std::fs::read_dir(workspace).unwrap().all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".unica-dump-guard-")),
                "private root escaped into a replaced parent: {}",
                workspace.display()
            );
        }
        assert!(
            result
                .errors
                .join("\n")
                .contains("directory anchor identity changed"),
            "{result:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unselected_external_source_set_does_not_block_configuration_dump() {
        let (root, context, config) = workspace("configuration-with-external");
        std::fs::write(
            &config,
            "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: '/F base'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: processors\n    type: EXTERNAL_DATA_PROCESSORS\n    path: external\n",
        )
        .unwrap();
        std::fs::create_dir_all(context.cwd.join("external")).unwrap();
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();

        let result = invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context);

        assert!(result.ok, "{result:?}");
        assert_eq!(runner.calls.load(Ordering::SeqCst), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn applied_full_dump_rejects_ignored_route_arguments_before_runner_execution() {
        let (root, context, _) = workspace("ignored-route-argument");
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let mut unsupported = args();
        unsupported.insert(
            "object".to_string(),
            Value::String("Catalog.Items".to_string()),
        );

        let result = VerifiedFullDumpAdapter::with_dependencies(
            &runner,
            &platform,
            PathBuf::from("fake-v8-runner"),
        )
        .invoke(
            "unica.runtime.execute",
            FullDumpInvocation::RuntimeExecute,
            &unsupported,
            &context,
            &CancellationToken::new(),
        )
        .unwrap();

        assert!(!result.ok, "{result:?}");
        assert!(result
            .errors
            .join("\n")
            .contains("does not accept `object`"));
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn selected_external_source_set_is_blocked_before_runner_execution() {
        let (root, context, config) = workspace("selected-external");
        std::fs::write(
            &config,
            "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: '/F base'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: processors\n    type: EXTERNAL_DATA_PROCESSORS\n    path: external\n",
        )
        .unwrap();
        std::fs::create_dir_all(context.cwd.join("external")).unwrap();
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let mut selected = args();
        selected.insert(
            "sourceSet".to_string(),
            Value::String("processors".to_string()),
        );

        let result = VerifiedFullDumpAdapter::with_dependencies(
            &runner,
            &platform,
            PathBuf::from("fake-v8-runner"),
        )
        .invoke(
            "unica.runtime.execute",
            FullDumpInvocation::RuntimeExecute,
            &selected,
            &context,
            &CancellationToken::new(),
        )
        .unwrap();

        assert!(!result.ok, "{result:?}");
        assert!(result.errors.join("\n").contains("is external"));
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    fn assert_failed_dump_has_no_owned_private_debris(
        context: &WorkspaceContext,
        result: &crate::application::AdapterOutcome,
    ) {
        assert!(
            std::fs::read_dir(&context.cwd).unwrap().all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".unica-dump-guard-")),
            "a finished attempt must discard its own rejected output: {result:?}"
        );
        assert!(
            !result
                .errors
                .iter()
                .any(|error| error.contains("private cleanup")),
            "source rejection must not fabricate an ownership/cleanup error: {result:?}"
        );
        assert!(
            !result
                .warnings
                .iter()
                .any(|warning| warning.contains("debris may remain")),
            "source rejection must not claim retained debris after successful cleanup: {result:?}"
        );
        assert!(
            result.artifacts.is_empty(),
            "a completely discarded private attempt has no retained recovery artifact: {result:?}"
        );
    }

    #[test]
    fn invalid_staged_xml_never_changes_visible_target() {
        for (case, owner_xml, expected) in [
            (
                "older",
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Configuration/></MetaDataObject>"#,
                "2.19",
            ),
            (
                "newer",
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Configuration/></MetaDataObject>"#,
                "2.21",
            ),
            (
                "entity",
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.&#50;0"><Configuration/></MetaDataObject>"#,
                "2.&#50;0",
            ),
            (
                "malformed",
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration>"#,
                "parse",
            ),
        ] {
            let (root, context, _) = workspace(case);
            let target = context.cwd.join("src");
            seed_valid_target(&target);
            let platform = fixed_platform(&root);
            let runner = DumpRunner {
                owner_xml: owner_xml.to_string(),
                mutation: RunnerMutation::None,
                calls: AtomicUsize::new(0),
            };

            let result = invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context);

            assert!(!result.ok, "{case}: {result:?}");
            assert!(
                result.errors.join("\n").contains(expected),
                "{case}: {result:?}"
            );
            assert_eq!(std::fs::read(target.join("before.txt")).unwrap(), b"before");
            assert_eq!(
                std::fs::read_to_string(target.join("Configuration.xml")).unwrap(),
                valid_configuration_owner()
            );
            assert_failed_dump_has_no_owned_private_debris(&context, &result);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn platform_mismatch_blocks_before_runner_execution() {
        let (root, context, _) = workspace("platform-mismatch");
        let runner = DumpRunner::valid();

        let result = invoke(
            &runner,
            &FailingPlatform,
            FullDumpInvocation::BuildDump,
            &context,
        );

        assert!(!result.ok, "{result:?}");
        assert!(result.errors.join("\n").contains("8.3.26"));
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        assert!(!context.cwd.join("src").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn platform_resolver_does_not_trust_an_explicit_platform_path_or_broad_claim() {
        let root = std::env::temp_dir().join(format!(
            "unica-platform-resolution-{}",
            uuid::Uuid::new_v4()
        ));
        let wrong = root
            .join("8.3.26.9999")
            .join(if cfg!(windows) { "1cv8.exe" } else { "1cv8" });
        make_platform_executable(&wrong);
        let wrong_config: serde_yaml::Value = serde_yaml::from_str(&format!(
            "tools:\n  platform:\n    path: '{}'\n    version: 8.3.27\n",
            wrong.display()
        ))
        .unwrap();

        let error = SystemPlatformResolver
            .resolve(
                &wrong_config,
                &root,
                PlatformUtility::Designer,
                crate::infrastructure::internal_adapters::system_process_runner(),
                &CancellationToken::new(),
            )
            .unwrap_err();

        assert!(error.contains("8.3.27"), "{error}");

        let exact = root.join("8.3.27.2074").join("1cv8");
        make_platform_executable(&exact);
        make_platform_executable(&exact.parent().unwrap().join("ibcmd"));
        let exact_config: serde_yaml::Value = serde_yaml::from_str(&format!(
            "tools:\n  platform:\n    path: '{}'\n    version: '8.3'\n",
            exact.display()
        ))
        .unwrap();

        let error = SystemPlatformResolver
            .resolve(
                &exact_config,
                &root,
                PlatformUtility::Designer,
                crate::infrastructure::internal_adapters::system_process_runner(),
                &CancellationToken::new(),
            )
            .expect_err("a user-owned platform installation must be refused");
        assert!(
            error.contains("immutable")
                || error.contains("owned by root")
                || error.contains("effective root caller"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mutable_platform_swap_and_restore_is_blocked_before_any_child_process() {
        let (root, context, config) = workspace("platform-swap-restore-before-probe");
        let exact = root.join("8.3.27.2074").join("1cv8");
        make_platform_executable(&exact);
        make_platform_executable(&exact.parent().unwrap().join("ibcmd"));
        std::fs::write(
            &config,
            format!(
                "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: '/F base'\ntools:\n  platform:\n    path: '{}'\n    version: '8.3.27'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
                exact.display()
            ),
        )
        .unwrap();
        let runner = SwapRestoreProbeRunner {
            install: exact.parent().unwrap().to_path_buf(),
            calls: AtomicUsize::new(0),
        };

        let result = VerifiedFullDumpAdapter::with_dependencies(
            &runner,
            &SystemPlatformResolver,
            PathBuf::from("fake-v8-runner"),
        )
        .invoke(
            "unica.build.dump",
            FullDumpInvocation::BuildDump,
            &args(),
            &context,
            &CancellationToken::new(),
        )
        .unwrap();

        assert!(!result.ok, "{result:?}");
        assert_eq!(
            runner.calls.load(Ordering::SeqCst),
            0,
            "a mutable installation must be rejected before its probe or v8-runner can swap and restore it"
        );
        assert!(
            result.errors.join("\n").contains("immutable")
                || result.errors.join("\n").contains("owned by root")
                || result.errors.join("\n").contains("effective root caller"),
            "{result:?}"
        );
        assert!(!context.cwd.join("src").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn platform_resolver_rejects_fake_binary_even_under_exact_version_directory_name() {
        let root = std::env::temp_dir().join(format!(
            "unica-platform-fake-resolution-{}",
            uuid::Uuid::new_v4()
        ));
        let fake = root.join("8.3.27.2074/1cv8");
        std::fs::create_dir_all(fake.parent().unwrap()).unwrap();
        std::fs::write(&fake, b"not a platform executable").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let config: serde_yaml::Value = serde_yaml::from_str(&format!(
            "tools:\n  platform:\n    path: '{}'\n    version: '8.3.27'\n",
            fake.display()
        ))
        .unwrap();

        let error = SystemPlatformResolver
            .resolve(
                &config,
                &root,
                PlatformUtility::Designer,
                crate::infrastructure::internal_adapters::system_process_runner(),
                &CancellationToken::new(),
            )
            .expect_err("a directory name is not platform attestation");

        assert!(
            error.contains("probe") || error.contains("version"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn platform_executable_or_install_swap_after_runner_prevents_publication() {
        for case in ["file", "install"] {
            let (root, context, _) = workspace(&format!("platform-swap-{case}"));
            let target = context.cwd.join("src");
            let platform = fixed_platform(&root);
            let executable = platform.executable.clone();
            let runner = DumpRunner {
                owner_xml: valid_configuration_owner(),
                mutation: match case {
                    "file" => RunnerMutation::PlatformFile(executable),
                    "install" => RunnerMutation::PlatformInstall(
                        platform.executable.parent().unwrap().to_path_buf(),
                    ),
                    _ => unreachable!(),
                },
                calls: AtomicUsize::new(0),
            };

            let result = invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context);

            assert!(!result.ok, "{case}: {result:?}");
            assert!(result.changes.is_empty(), "{case}: {result:?}");
            assert!(!target.join("Configuration.xml").exists(), "{case}");
            assert!(
                result.errors.join("\n").contains("platform")
                    && (result.errors.join("\n").contains("changed")
                        || result.errors.join("\n").contains("attestation")),
                "{case}: {result:?}"
            );
            assert_failed_dump_has_no_owned_private_debris(&context, &result);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn config_and_target_races_block_publication_without_overwriting_concurrent_state() {
        for case in ["config", "target"] {
            let (root, context, config) = workspace(case);
            let target = context.cwd.join("src");
            seed_valid_target(&target);
            let platform = fixed_platform(&root);
            let runner = DumpRunner {
                owner_xml: valid_configuration_owner(),
                mutation: match case {
                    "config" => RunnerMutation::Config(config.clone()),
                    "target" => RunnerMutation::Target(target.clone()),
                    _ => unreachable!(),
                },
                calls: AtomicUsize::new(0),
            };

            let result = invoke(
                &runner,
                &platform,
                FullDumpInvocation::RuntimeExecute,
                &context,
            );

            assert!(!result.ok, "{case}: {result:?}");
            assert!(result.changes.is_empty(), "{case}: {result:?}");
            assert_eq!(std::fs::read(target.join("before.txt")).unwrap(), b"before");
            assert_eq!(
                std::fs::read_to_string(target.join("Configuration.xml")).unwrap(),
                valid_configuration_owner()
            );
            if case == "target" {
                assert_eq!(
                    std::fs::read(target.join("concurrent.txt")).unwrap(),
                    b"concurrent"
                );
            }
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn publication_failure_after_backup_restores_original_tree() {
        let (root, context, _) = workspace("rollback");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();

        let result = with_publication_failpoint(PublicationCheckpoint::AfterBackup, || {
            invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context)
        });

        assert!(!result.ok, "{result:?}");
        assert!(result.errors.join("\n").contains("injected"));
        assert_eq!(std::fs::read(target.join("before.txt")).unwrap(), b"before");
        assert_eq!(
            std::fs::read_to_string(target.join("Configuration.xml")).unwrap(),
            valid_configuration_owner()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_destination_race_after_backup_survives_rollback() {
        let (root, context, _) = workspace("windows-destination-race");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let raced_target = target.clone();
        let hook_workspace = context.cwd.clone();

        let result = with_publication_hook(
            PublicationCheckpoint::AfterBackup,
            move || {
                assert!(
                    !raced_target.exists(),
                    "the original destination must already be moved to backup"
                );
                let private_root = std::fs::read_dir(&hook_workspace)
                    .unwrap()
                    .flatten()
                    .map(|entry| entry.path())
                    .find(|path| {
                        path.file_name().is_some_and(|name| {
                            name.to_string_lossy().starts_with(".unica-dump-guard-")
                        })
                    })
                    .expect("private dump root");
                assert!(
                    std::fs::read_dir(private_root.join("recovery"))
                        .unwrap()
                        .flatten()
                        .any(|entry| entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with("target-backup-")),
                    "the original destination backup must be retained for rollback"
                );
                std::fs::create_dir_all(&raced_target).unwrap();
                std::fs::write(raced_target.join("racer.txt"), b"racer").unwrap();
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(std::fs::read(target.join("racer.txt")).unwrap(), b"racer");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn backup_name_replacement_is_not_accepted_as_rollback_source() {
        for case in ["snapshot-mismatch", "snapshot-error"] {
            let (root, context, _) = workspace(&format!("rollback-backup-name-replacement-{case}"));
            let target = context.cwd.join("src");
            seed_valid_target(&target);
            let displaced_backup = context.cwd.join("validated-backup-displaced");
            let platform = fixed_platform(&root);
            let runner = DumpRunner::valid();
            let hook_cwd = context.cwd.clone();
            let hook_displaced = displaced_backup.clone();

            let result = with_publication_hook(
                PublicationCheckpoint::BeforeSealedPublishRename,
                move || {
                    let private_root = std::fs::read_dir(&hook_cwd)
                        .unwrap()
                        .flatten()
                        .map(|entry| entry.path())
                        .find(|path| {
                            path.file_name().is_some_and(|name| {
                                name.to_string_lossy().starts_with(".unica-dump-guard-")
                            })
                        })
                        .expect("private dump root");
                    let recovery = private_root.join("recovery");
                    let backup = std::fs::read_dir(&recovery)
                        .unwrap()
                        .flatten()
                        .map(|entry| entry.path())
                        .find(|path| {
                            path.file_name().is_some_and(|name| {
                                name.to_string_lossy().starts_with("target-backup-")
                            })
                        })
                        .expect("validated target backup");
                    std::fs::rename(&backup, &hook_displaced).unwrap();
                    std::fs::create_dir(&backup).unwrap();
                    std::fs::write(
                        backup.join("Configuration.xml"),
                        valid_configuration_owner(),
                    )
                    .unwrap();
                    match case {
                        "snapshot-mismatch" => {
                            std::fs::write(backup.join("unvalidated.txt"), b"unvalidated").unwrap();
                        }
                        "snapshot-error" => {
                            std::os::unix::fs::symlink(
                                backup.join("Configuration.xml"),
                                backup.join("unvalidated-link.xml"),
                            )
                            .unwrap();
                        }
                        _ => unreachable!(),
                    }
                },
                || {
                    with_publication_failpoint(
                        PublicationCheckpoint::BeforeSealedPublishRename,
                        || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
                    )
                },
            );

            assert!(!result.ok, "{case}: {result:?}");
            assert!(
                !target.join("unvalidated.txt").exists()
                    && std::fs::symlink_metadata(target.join("unvalidated-link.xml")).is_err(),
                "{case}: a replacement under the backup name must not remain Git-visible"
            );
            assert!(
                displaced_backup.join("before.txt").is_file(),
                "{case}: the independently displaced original remains untouched"
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_target_during_capture_error_quarantine_is_preserved() {
        assert_concurrent_target_survives_quarantine(QuarantineRace::Directory);
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_file_during_capture_error_quarantine_is_preserved() {
        assert_concurrent_target_survives_quarantine(QuarantineRace::File);
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_target_during_capture_error_quarantine_survives_recovery_failure() {
        assert_concurrent_target_survives_quarantine(QuarantineRace::DetachedDirectory);
    }

    #[cfg(unix)]
    enum QuarantineRace {
        Directory,
        File,
        DetachedDirectory,
    }

    #[cfg(unix)]
    fn assert_concurrent_target_survives_quarantine(case: QuarantineRace) {
        let detach_private = matches!(case, QuarantineRace::DetachedDirectory);
        let foreign_file = matches!(case, QuarantineRace::File);
        let (root, context, config) = workspace("quarantine-concurrent-target");
        std::fs::write(&config,
            "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: 'File=base;Usr=Admin;Pwd=quarantine-secret'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        ).unwrap();
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let displaced_stage = context.cwd.join("externally-displaced-stage");
        let detached_private = context.cwd.join("externally-displaced-private");
        let hook_target = std::fs::canonicalize(&target).unwrap();
        let probe_target = target.clone();
        let hook_displaced = displaced_stage.clone();
        let hook_private = detached_private.clone();
        let hook_cwd = context.cwd.clone();
        let armed = std::rc::Rc::new(std::cell::Cell::new(false));
        let probe_armed = armed.clone();
        let identity = std::rc::Rc::new(std::cell::Cell::new(None));
        let hook_identity = identity.clone();
        let result = super::with_directory_rename_hook(
            move |source| {
                if source != hook_target || !armed.replace(false) {
                    return;
                }
                std::fs::rename(&hook_target, &hook_displaced).unwrap();
                let foreign = if foreign_file {
                    std::fs::write(&hook_target, b"concurrent").unwrap();
                    std::fs::File::open(&hook_target).unwrap()
                } else {
                    std::fs::create_dir(&hook_target).unwrap();
                    std::fs::write(hook_target.join("concurrent.txt"), b"concurrent").unwrap();
                    super::open_directory_nofollow(&hook_target).unwrap()
                };
                hook_identity.set(Some(super::file_identity(&foreign).unwrap()));
                if detach_private {
                    let private = std::fs::read_dir(&hook_cwd)
                        .unwrap()
                        .filter_map(Result::ok)
                        .map(|entry| entry.path())
                        .find(|path| {
                            path.file_name().is_some_and(|name| {
                                name.to_string_lossy().starts_with(".unica-dump-guard-")
                            })
                        })
                        .unwrap();
                    std::fs::rename(private, &hook_private).unwrap();
                }
            },
            || {
                with_publication_hook(
                    PublicationCheckpoint::BeforeInstalledSnapshot,
                    move || {
                        std::os::unix::fs::symlink(
                            probe_target.join("Configuration.xml"),
                            probe_target.join("unvalidated-link.xml"),
                        )
                        .unwrap();
                        probe_armed.set(true);
                    },
                    || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
                )
            },
        );
        assert!(!result.ok, "{detach_private}: {result:?}");
        let contents = |path: &Path| {
            std::fs::read(if foreign_file {
                path.to_path_buf()
            } else {
                path.join("concurrent.txt")
            })
        };
        let foreign = if contents(&target).is_ok_and(|bytes| bytes == b"concurrent") {
            target.clone()
        } else {
            let private = if detach_private {
                detached_private.clone()
            } else {
                std::fs::read_dir(&context.cwd)
                    .unwrap()
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .find(|path| {
                        path.file_name().is_some_and(|name| {
                            name.to_string_lossy().starts_with(".unica-dump-guard-")
                        })
                    })
                    .expect("foreign target must survive in retained recovery")
            };
            std::fs::read_dir(private.join("recovery"))
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| contents(path).is_ok_and(|bytes| bytes == b"concurrent"))
                .expect("foreign target must not be deleted during cleanup")
        };
        assert_eq!(contents(&foreign).unwrap(), b"concurrent");
        let foreign_handle = if foreign_file {
            std::fs::File::open(&foreign).unwrap()
        } else {
            super::open_directory_nofollow(&std::fs::canonicalize(&foreign).unwrap()).unwrap()
        };
        assert_eq!(
            Some(super::file_identity(&foreign_handle).unwrap()),
            identity.get()
        );
        assert!(displaced_stage.join("Configuration.xml").is_file());
        let private = if detach_private {
            detached_private
        } else {
            std::fs::read_dir(&context.cwd)
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| {
                    path.file_name().is_some_and(|name| {
                        name.to_string_lossy().starts_with(".unica-dump-guard-")
                    })
                })
                .expect("owned original backup remains recoverable")
        };
        if detach_private {
            let config = private.join("execution").join(super::EFFECTIVE_CONFIG_NAME);
            assert!(
                !config.exists() || std::fs::read(config).unwrap().is_empty(),
                "{result:?}"
            );
        } else {
            assert!(!private.join("execution").exists(), "{result:?}");
        }
        assert!(std::fs::read_dir(private.join("recovery"))
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.path().join("before.txt").is_file()));
        let retained_root_identity = super::file_identity(
            &super::open_directory_nofollow(&std::fs::canonicalize(&private).unwrap()).unwrap(),
        )
        .unwrap();
        let retained_recovery_identity = super::file_identity(
            &super::open_directory_nofollow(
                &std::fs::canonicalize(private.join("recovery")).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        let owned_artifacts = result.artifacts.iter().all(|artifact| {
            std::fs::canonicalize(artifact)
                .ok()
                .and_then(|path| super::open_directory_nofollow(&path).ok())
                .and_then(|directory| super::file_identity(&directory).ok())
                .is_some_and(|identity| {
                    identity == retained_root_identity || identity == retained_recovery_identity
                })
        });
        let location_unconfirmed = result
            .warnings
            .iter()
            .any(|warning| warning.contains("current location is unconfirmed"));
        assert!(
            owned_artifacts && (!result.artifacts.is_empty() || location_unconfirmed),
            "quarantine recovery must report an owned identity or explicit uncertainty"
        );
        if detach_private {
            assert!(
                result.artifacts.is_empty() && location_unconfirmed,
                "a displaced private root must not advertise its missing original name"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_target_directory_before_first_installed_probe_survives_rollback() {
        for had_target in [true, false] {
            assert_foreign_target_survives_first_installed_probe(had_target, "directory");
        }
    }

    #[test]
    fn concurrent_target_file_before_first_installed_probe_survives_rollback() {
        for had_target in [true, false] {
            assert_foreign_target_survives_first_installed_probe(had_target, "file");
        }
    }

    #[test]
    fn absent_target_before_first_installed_probe_restores_the_owned_backup() {
        let (root, context, _) = workspace("first-probe-absent-target");
        let target = context.cwd.join("src");
        let displaced_stage = context.cwd.join("externally-displaced-stage");
        seed_valid_target(&target);
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let hook_target = target.clone();
        let hook_displaced_stage = displaced_stage.clone();

        let result = with_publication_hook(
            PublicationCheckpoint::BeforeInstalledSnapshot,
            move || std::fs::rename(&hook_target, &hook_displaced_stage).unwrap(),
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(std::fs::read(target.join("before.txt")).unwrap(), b"before");
        assert!(displaced_stage.join("Configuration.xml").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    fn assert_foreign_target_survives_first_installed_probe(had_target: bool, kind: &str) {
        let (root, context, _) = workspace(&format!("first-probe-{kind}-{had_target}"));
        let target = context.cwd.join("src");
        let displaced_stage = context.cwd.join("externally-displaced-stage");
        if had_target {
            seed_valid_target(&target);
        }
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let hook_target = target.clone();
        let hook_displaced_stage = displaced_stage.clone();
        let foreign_identity = std::rc::Rc::new(std::cell::Cell::new(None));
        let hook_identity = foreign_identity.clone();
        let is_directory = kind == "directory";

        let result = with_publication_hook(
            PublicationCheckpoint::BeforeInstalledSnapshot,
            move || {
                std::fs::rename(&hook_target, &hook_displaced_stage).unwrap();
                let foreign = if is_directory {
                    std::fs::create_dir(&hook_target).unwrap();
                    std::fs::write(hook_target.join("concurrent.txt"), b"concurrent").unwrap();
                    super::open_directory_nofollow(&std::fs::canonicalize(&hook_target).unwrap())
                        .unwrap()
                } else {
                    std::fs::write(&hook_target, b"concurrent").unwrap();
                    std::fs::File::open(&hook_target).unwrap()
                };
                hook_identity.set(Some(super::file_identity(&foreign).unwrap()));
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{kind}/{had_target}: {result:?}");
        let contents = if is_directory {
            std::fs::read(target.join("concurrent.txt"))
        } else {
            std::fs::read(&target)
        };
        assert_eq!(
            contents.unwrap_or_else(|error| panic!(
                "{kind}/{had_target}: publication removed the concurrent target: {error}; {result:?}"
            )),
            b"concurrent"
        );
        let foreign = if is_directory {
            super::open_directory_nofollow(&std::fs::canonicalize(&target).unwrap()).unwrap()
        } else {
            std::fs::File::open(&target).unwrap()
        };
        assert_eq!(
            Some(super::file_identity(&foreign).unwrap()),
            foreign_identity.get(),
            "{kind}/{had_target}: publication moved or replaced the concurrent target"
        );
        assert!(displaced_stage.join("Configuration.xml").is_file());
        let recoveries = std::fs::read_dir(&context.cwd)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(".unica-dump-guard-"))
            })
            .collect::<Vec<_>>();
        assert_eq!(recoveries.len(), 1, "{result:?}");
        assert!(!recoveries[0].join("execution").exists());
        let recovery = std::fs::canonicalize(recoveries[0].join("recovery")).unwrap();
        assert_retained_directory_artifact(&result, &recovery);
        assert!(
            result.summary.contains("publication state is unknown"),
            "{result:?}"
        );
        if had_target {
            let backup = std::fs::read_dir(&recovery)
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| {
                    path.file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with("target-backup-"))
                })
                .expect("the owned original remains available for recovery");
            assert_eq!(std::fs::read(backup.join("before.txt")).unwrap(), b"before");
        }
        drop(foreign);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_target_replacement_after_stage_install_survives_rollback() {
        let (root, context, _) = workspace("rollback-concurrent-after-install");
        let target = context.cwd.join("src");
        let displaced_stage = context.cwd.join("externally-displaced-stage");
        seed_valid_target(&target);
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let hook_target = target.clone();
        let hook_displaced_stage = displaced_stage.clone();

        let result = with_publication_hook(
            PublicationCheckpoint::AfterStageInstall,
            move || {
                std::fs::rename(&hook_target, &hook_displaced_stage).unwrap();
                std::fs::create_dir_all(&hook_target).unwrap();
                std::fs::write(hook_target.join("concurrent.txt"), b"concurrent").unwrap();
            },
            || {
                with_publication_failpoint(PublicationCheckpoint::AfterStageInstall, || {
                    invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context)
                })
            },
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(
            std::fs::read(target.join("concurrent.txt")).unwrap(),
            b"concurrent",
            "rollback must never move or overwrite a target it no longer owns"
        );
        assert!(displaced_stage.join("Configuration.xml").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn installed_tree_interior_mutation_is_quarantined_and_rolled_back() {
        for checkpoint in [
            PublicationCheckpoint::BeforeInstalledSnapshot,
            PublicationCheckpoint::AfterStageInstall,
        ] {
            let (root, context, _) = workspace("installed-tree-interior-mutation");
            let target = context.cwd.join("src");
            seed_valid_target(&target);
            let platform = fixed_platform(&root);
            let runner = DumpRunner::valid();
            let hook_target = target.clone();

            let result = with_publication_hook(
                checkpoint,
                move || {
                    std::fs::write(hook_target.join("unvalidated.txt"), b"unvalidated").unwrap();
                },
                || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
            );

            assert!(!result.ok, "{result:?}");
            assert_eq!(
                std::fs::read(target.join("before.txt")).unwrap(),
                b"before",
                "the original target must be restored after mutation of our installed directory"
            );
            assert!(
                !target.join("unvalidated.txt").exists(),
                "our installed but mutated directory must be quarantined before lock release"
            );
            assert!(
                result.errors.join("\n").contains("published dump differs")
                    || result.errors.join("\n").contains("quarantine"),
                "{result:?}"
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn installed_tree_capture_error_with_same_root_identity_is_quarantined_and_rolled_back() {
        for checkpoint in [
            PublicationCheckpoint::BeforeInstalledSnapshot,
            PublicationCheckpoint::AfterStageInstall,
        ] {
            let (root, context, _) = workspace("installed-tree-interior-symlink");
            let target = context.cwd.join("src");
            seed_valid_target(&target);
            let platform = fixed_platform(&root);
            let runner = DumpRunner::valid();
            let hook_target = target.clone();

            let result = with_publication_hook(
                checkpoint,
                move || {
                    std::os::unix::fs::symlink(
                        hook_target.join("Configuration.xml"),
                        hook_target.join("unvalidated-link.xml"),
                    )
                    .unwrap();
                },
                || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
            );

            assert!(!result.ok, "{result:?}");
            assert_eq!(
                std::fs::read(target.join("before.txt")).unwrap(),
                b"before",
                "the original target must be restored after capture failure in our installed directory"
            );
            assert!(
                std::fs::symlink_metadata(target.join("unvalidated-link.xml")).is_err(),
                "our installed but invalid directory must be quarantined before lock release"
            );
            assert!(
                result
                    .errors
                    .join("\n")
                    .contains("could not verify the published target")
                    || result.errors.join("\n").contains("quarantine"),
                "{result:?}"
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn stage_name_replacement_before_publish_never_becomes_visible() {
        let (root, context, _) = workspace("stage-name-replacement");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let displaced_stage = context.cwd.join("validated-stage-displaced");
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let hook_cwd = context.cwd.clone();
        let hook_displaced = displaced_stage.clone();

        let result = with_publication_hook(
            PublicationCheckpoint::BeforeStageInstall,
            move || {
                let private_root = std::fs::read_dir(&hook_cwd)
                    .unwrap()
                    .flatten()
                    .map(|entry| entry.path())
                    .find(|path| {
                        path.file_name().is_some_and(|name| {
                            name.to_string_lossy().starts_with(".unica-dump-guard-")
                        })
                    })
                    .expect("private dump root");
                let stage = private_root.join("execution/staged-source");
                std::fs::rename(&stage, &hook_displaced).unwrap();
                std::fs::create_dir_all(&stage).unwrap();
                std::fs::write(stage.join("Configuration.xml"), valid_configuration_owner())
                    .unwrap();
                std::os::unix::fs::symlink(
                    stage.join("Configuration.xml"),
                    stage.join("unvalidated-link.xml"),
                )
                .unwrap();
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(
            std::fs::read(target.join("before.txt")).unwrap(),
            b"before",
            "the original target must be restored without exposing the replacement stage"
        );
        assert!(
            !target.join("unvalidated-link.xml").exists(),
            "the replacement stage must never remain Git-visible"
        );
        assert!(displaced_stage.join("Configuration.xml").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn reappeared_execution_stage_survives_success_and_rollback() {
        assert_reappeared_private_stage_is_preserved(true);
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_open_replacement_preserves_owned_and_foreign_trees() {
        assert_cleanup_open_preserves_foreign_data(false);
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_open_preserves_unknown_descendants_of_owned_tree() {
        assert_cleanup_open_preserves_foreign_data(true);
    }

    #[cfg(unix)]
    fn assert_cleanup_open_preserves_foreign_data(nested: bool) {
        let (root, context, _) = workspace("cleanup-open-replacement");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let original = super::capture_tree_target_nofollow(&target).unwrap();
        let displaced = context.cwd.join("displaced-owned-backup");
        let hook_displaced = displaced.clone();
        let foreign = std::rc::Rc::new(std::cell::RefCell::new(None));
        let hook_foreign = foreign.clone();
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let result = super::with_directory_cleanup_open_hook(
            move |path| {
                if hook_foreign.borrow().is_some()
                    || !path
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with("target-backup-"))
                {
                    return;
                }
                let foreign_path = if nested {
                    let nested_path = path.join("unknown-child");
                    std::fs::create_dir(&nested_path).unwrap();
                    nested_path
                } else {
                    std::fs::rename(path, &hook_displaced).unwrap();
                    std::fs::create_dir(path).unwrap();
                    path.to_path_buf()
                };
                std::fs::write(foreign_path.join("sentinel.txt"), b"foreign cleanup data").unwrap();
                let snapshot = super::capture_tree_target_nofollow(&foreign_path).unwrap();
                hook_foreign.replace(Some((foreign_path, snapshot)));
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );
        let (foreign_path, foreign_snapshot) = foreign.borrow().clone().expect("cleanup hook ran");
        assert_eq!(
            super::capture_tree_target_nofollow(&foreign_path).unwrap(),
            foreign_snapshot,
            "cleanup must preserve a foreign root and all its data"
        );
        if !nested {
            assert_eq!(
                super::capture_tree_target_nofollow(&displaced).unwrap(),
                original
            );
        }
        assert!(std::fs::read_to_string(target.join("Configuration.xml"))
            .unwrap()
            .contains(TARGET_EXPORT_FORMAT));
        assert!(!result.ok, "{result:?}");
        assert!(
            result.summary.contains("private cleanup failed"),
            "{result:?}"
        );
        assert!(
            !result.summary.contains("publication state is unknown"),
            "{result:?}"
        );
        assert!(!result.artifacts.is_empty(), "{result:?}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn reappeared_sealed_stage_survives_success_and_rollback() {
        assert_reappeared_private_stage_is_preserved(false);
    }

    #[cfg(unix)]
    fn assert_reappeared_private_stage_is_preserved(execution_slot: bool) {
        for rollback in [false, true] {
            let (root, context, _) = workspace("reappeared-private-stage");
            let target = context.cwd.join("src");
            seed_valid_target(&target);
            let original = super::capture_tree_target_nofollow(&target).unwrap();
            let staged = std::rc::Rc::new(std::cell::RefCell::new(None));
            let hook_staged = staged.clone();
            let checkpoint_staged = staged.clone();
            let sealed_path = std::rc::Rc::new(std::cell::RefCell::new(None));
            let rename_sealed_path = sealed_path.clone();
            let hook_sealed_path = sealed_path.clone();
            let foreign = std::rc::Rc::new(std::cell::RefCell::new(None));
            let hook_foreign = foreign.clone();
            let checkpoint = if execution_slot {
                PublicationCheckpoint::AfterBackup
            } else {
                PublicationCheckpoint::AfterStageInstall
            };
            let platform = fixed_platform(&root);
            let runner = DumpRunner::valid();
            let hook_cwd = context.cwd.clone();
            let result = super::with_directory_rename_hook(
                move |source| {
                    if source
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with("sealed-stage-"))
                        && hook_staged.borrow().is_none()
                    {
                        hook_staged
                            .replace(Some(super::capture_tree_target_nofollow(source).unwrap()));
                        rename_sealed_path.replace(Some(source.to_path_buf()));
                    }
                },
                || {
                    with_publication_hook(
                        checkpoint,
                        move || {
                            let sealed: PathBuf =
                                hook_sealed_path.borrow().clone().unwrap_or_else(|| {
                                    let private = std::fs::read_dir(&hook_cwd)
                                        .unwrap()
                                        .filter_map(Result::ok)
                                        .map(|entry| entry.path())
                                        .find(|path| {
                                            path.file_name().is_some_and(|name| {
                                                name.to_string_lossy()
                                                    .starts_with(".unica-dump-guard-")
                                            })
                                        })
                                        .unwrap();
                                    std::fs::read_dir(private.join("recovery"))
                                        .unwrap()
                                        .filter_map(Result::ok)
                                        .map(|entry| entry.path())
                                        .find(|path| {
                                            path.file_name().is_some_and(|name| {
                                                name.to_string_lossy().starts_with("sealed-stage-")
                                            })
                                        })
                                        .unwrap()
                                });
                            if checkpoint_staged.borrow().is_none() {
                                checkpoint_staged.replace(Some(
                                    super::capture_tree_target_nofollow(&sealed).unwrap(),
                                ));
                            }
                            hook_sealed_path.replace(Some(sealed.clone()));
                            let path = if execution_slot {
                                sealed
                                    .parent()
                                    .unwrap()
                                    .parent()
                                    .unwrap()
                                    .join("execution/staged-source")
                            } else {
                                sealed
                            };
                            std::fs::create_dir(&path).unwrap();
                            std::fs::write(path.join("foreign.txt"), b"reappeared private stage")
                                .unwrap();
                            hook_foreign.replace(Some((
                                path.clone(),
                                super::capture_tree_target_nofollow(&path).unwrap(),
                            )));
                        },
                        || {
                            if rollback {
                                with_publication_failpoint(checkpoint, || {
                                    invoke(
                                        &runner,
                                        &platform,
                                        FullDumpInvocation::BuildDump,
                                        &context,
                                    )
                                })
                            } else {
                                invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context)
                            }
                        },
                    )
                },
            );
            let (foreign_path, foreign_snapshot) = foreign.borrow().as_ref().unwrap().clone();
            assert_eq!(
                super::capture_tree_target_nofollow(&foreign_path).unwrap(),
                foreign_snapshot,
                "a reused private slot must retain the foreign root identity and bytes"
            );
            let private = sealed_path
                .borrow()
                .as_ref()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .to_path_buf();
            let mut snapshots = vec![super::capture_tree_target_nofollow(&target).unwrap()];
            for directory in [private.join("execution"), private.join("recovery")] {
                if directory.is_dir() {
                    snapshots.extend(
                        std::fs::read_dir(directory)
                            .unwrap()
                            .filter_map(Result::ok)
                            .filter(|entry| entry.file_type().unwrap().is_dir())
                            .map(|entry| {
                                super::capture_tree_target_nofollow(&entry.path()).unwrap()
                            }),
                    );
                }
            }
            assert!(
                snapshots.contains(&original),
                "original source must remain recoverable"
            );
            assert!(
                snapshots.contains(staged.borrow().as_ref().unwrap()),
                "validated stage must remain recoverable"
            );
            assert!(!result.ok, "{execution_slot}/{rollback}: {result:?}");
            if execution_slot {
                let config = private.join("execution").join(super::EFFECTIVE_CONFIG_NAME);
                assert!(!config.exists() || std::fs::read(config).unwrap().is_empty());
            } else {
                assert!(!private.join("execution").exists());
            }
            if rollback {
                assert_eq!(
                    super::capture_tree_target_nofollow(&target).unwrap(),
                    original
                );
                assert!(
                    !result.summary.contains("publication state is unknown"),
                    "{result:?}"
                );
            } else {
                assert!(
                    result
                        .summary
                        .contains("validated publication completed but private cleanup failed"),
                    "{result:?}"
                );
                assert!(
                    !result.summary.contains("publication state is unknown"),
                    "{result:?}"
                );
            }
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn private_stage_swap_before_seal_precheck_preserves_data_and_scrubs_credentials() {
        assert_private_stage_swap_during_seal_is_preserved(true);
    }

    #[cfg(unix)]
    #[test]
    fn private_stage_swap_after_seal_precheck_preserves_data_and_scrubs_credentials() {
        assert_private_stage_swap_during_seal_is_preserved(false);
    }

    #[cfg(unix)]
    fn assert_private_stage_swap_during_seal_is_preserved(before_precheck: bool) {
        for had_target in [true, false] {
            let (root, context, config) = workspace("private-stage-seal-swap");
            std::fs::write(&config,
                "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: 'File=base;Usr=Admin;Pwd=seal-secret'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
            ).unwrap();
            let target = std::fs::canonicalize(&context.cwd).unwrap().join("src");
            if had_target {
                seed_valid_target(&target);
            }
            let original = super::capture_tree_target_nofollow(&target).unwrap();
            let displaced_stage = context.cwd.join("externally-displaced-private-stage");
            let hook_displaced = displaced_stage.clone();
            let stage_before = std::rc::Rc::new(std::cell::RefCell::new(None));
            let hook_stage = stage_before.clone();
            let foreign_before = std::rc::Rc::new(std::cell::RefCell::new(None));
            let hook_foreign = foreign_before.clone();
            let swap = move |source: &Path| {
                if hook_stage.borrow().is_some() {
                    return;
                }
                hook_stage.replace(Some(super::capture_tree_target_nofollow(source).unwrap()));
                std::fs::rename(source, &hook_displaced).unwrap();
                std::fs::create_dir(source).unwrap();
                std::fs::write(
                    source.join("Configuration.xml"),
                    b"foreign private-stage XML",
                )
                .unwrap();
                std::fs::write(source.join("foreign.txt"), b"foreign private-stage data").unwrap();
                hook_foreign.replace(Some(super::capture_tree_target_nofollow(source).unwrap()));
            };
            let platform = fixed_platform(&root);
            let runner = DumpRunner::valid();
            let hook_cwd = context.cwd.clone();
            let result = if before_precheck {
                with_publication_hook(
                    PublicationCheckpoint::BeforeStageInstall,
                    move || {
                        let private = std::fs::read_dir(&hook_cwd)
                            .unwrap()
                            .filter_map(Result::ok)
                            .map(|entry| entry.path())
                            .find(|path| {
                                path.file_name().is_some_and(|name| {
                                    name.to_string_lossy().starts_with(".unica-dump-guard-")
                                })
                            })
                            .unwrap();
                        swap(&private.join("execution/staged-source"));
                    },
                    || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
                )
            } else {
                super::with_directory_rename_hook(
                    move |source| {
                        if source
                            .file_name()
                            .is_some_and(|name| name == "staged-source")
                        {
                            swap(source);
                        }
                    },
                    || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
                )
            };
            assert!(!result.ok, "{before_precheck}/{had_target}: {result:?}");
            assert_eq!(
                super::capture_tree_target_nofollow(&target).unwrap(),
                original
            );
            assert_eq!(
                Some(super::capture_tree_target_nofollow(&displaced_stage).unwrap()),
                *stage_before.borrow()
            );
            let private = std::fs::read_dir(&context.cwd)
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| {
                    path.file_name().is_some_and(|name| {
                        name.to_string_lossy().starts_with(".unica-dump-guard-")
                    })
                })
                .expect("uncertain stage must survive private cleanup");
            let mut snapshots = Vec::new();
            for directory in [private.join("execution"), private.join("recovery")] {
                for entry in std::fs::read_dir(directory).unwrap().filter_map(Result::ok) {
                    if entry.file_type().unwrap().is_dir() {
                        snapshots.push(super::capture_tree_target_nofollow(&entry.path()).unwrap());
                    }
                }
            }
            assert!(
                snapshots.contains(
                    foreign_before
                        .borrow()
                        .as_ref()
                        .expect("source swap occurred")
                ),
                "foreign private stage identity and bytes must be retained"
            );
            let config = private.join("execution").join(super::EFFECTIVE_CONFIG_NAME);
            assert!(!config.exists() || std::fs::read(config).unwrap().is_empty(),
                "the held effective config must be scrubbed without deleting unknown stage contents");
            assert!(
                result
                    .summary
                    .contains("failed before verified dump publication"),
                "{result:?}"
            );
            assert!(
                result.artifacts.contains(
                    &std::fs::canonicalize(&private)
                        .unwrap()
                        .display()
                        .to_string()
                ),
                "{result:?}"
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn sealed_source_swap_after_identity_precheck_preserves_data_and_reports_unknown() {
        assert_sealed_source_swap_after_precheck_reports_unknown(true);
    }

    #[cfg(unix)]
    #[test]
    fn sealed_source_swap_after_identity_precheck_without_original_reports_unknown() {
        assert_sealed_source_swap_after_precheck_reports_unknown(false);
    }

    #[cfg(unix)]
    fn assert_sealed_source_swap_after_precheck_reports_unknown(had_target: bool) {
        let (root, context, _) = workspace("sealed-source-after-precheck");
        let target = std::fs::canonicalize(&context.cwd).unwrap().join("src");
        if had_target {
            seed_valid_target(&target);
        }
        let original = had_target.then(|| super::capture_tree_target_nofollow(&target).unwrap());
        let displaced_stage = context.cwd.join("externally-displaced-validated-stage");
        let hook_displaced = displaced_stage.clone();
        let stage_before = std::rc::Rc::new(std::cell::RefCell::new(None));
        let hook_stage_before = stage_before.clone();
        let foreign_before = std::rc::Rc::new(std::cell::RefCell::new(None));
        let hook_foreign_before = foreign_before.clone();
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();

        let result = super::with_directory_rename_hook(
            move |source| {
                if !source
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("sealed-stage-"))
                    || hook_stage_before.borrow().is_some()
                {
                    return;
                }
                hook_stage_before
                    .replace(Some(super::capture_tree_target_nofollow(source).unwrap()));
                std::fs::rename(source, &hook_displaced).unwrap();
                std::fs::create_dir(source).unwrap();
                std::fs::write(source.join("Configuration.xml"), b"unvalidated XML owner").unwrap();
                std::fs::write(source.join("unvalidated.txt"), b"foreign sealed source").unwrap();
                hook_foreign_before
                    .replace(Some(super::capture_tree_target_nofollow(source).unwrap()));
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(
            Some(
                super::capture_tree_target_nofollow(
                    &std::fs::canonicalize(&displaced_stage).unwrap()
                )
                .unwrap()
            ),
            *stage_before.borrow(),
            "the held validated stage must retain its identity and bytes"
        );
        let private = std::fs::read_dir(&context.cwd)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(".unica-dump-guard-"))
            })
            .expect("owned original and unverified source must remain recoverable");
        let mut snapshots = vec![super::capture_tree_target_nofollow(&target).unwrap()];
        snapshots.extend(
            std::fs::read_dir(private.join("recovery"))
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| super::capture_tree_target_nofollow(&entry.path()).unwrap()),
        );
        if let Some(original) = original {
            assert!(
                snapshots.contains(&original),
                "original target identity and bytes must be retained"
            );
        }
        assert!(
            snapshots.contains(
                foreign_before
                    .borrow()
                    .as_ref()
                    .expect("source swap occurred")
            ),
            "foreign sealed source identity and bytes must be retained"
        );
        assert!(!private.join("execution").exists());
        let recovery = std::fs::canonicalize(private.join("recovery")).unwrap();
        assert!(
            result.artifacts.contains(&recovery.display().to_string()),
            "{result:?}"
        );
        assert!(
            result.summary.contains("publication state is unknown"),
            "{result:?}"
        );
        assert!(
            !result
                .summary
                .contains("failed before verified dump publication"),
            "{result:?}"
        );
        assert!(
            result
                .warnings
                .iter()
                .all(|warning| !warning.contains("discarded")
                    && !warning.contains("sources were not published")),
            "{result:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn sealed_stage_replacement_at_publish_boundary_is_quarantined_before_lock_release() {
        let (root, context, _) = workspace("sealed-stage-publish-replacement");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let displaced_stage = context.cwd.join("sealed-validated-stage-displaced");
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let hook_cwd = context.cwd.clone();
        let hook_displaced = displaced_stage.clone();

        let result = with_publication_hook(
            PublicationCheckpoint::BeforeSealedPublishRename,
            move || {
                let private_root = std::fs::read_dir(&hook_cwd)
                    .unwrap()
                    .flatten()
                    .map(|entry| entry.path())
                    .find(|path| {
                        path.file_name().is_some_and(|name| {
                            name.to_string_lossy().starts_with(".unica-dump-guard-")
                        })
                    })
                    .expect("private dump root");
                let recovery = private_root.join("recovery");
                let sealed = std::fs::read_dir(&recovery)
                    .unwrap()
                    .flatten()
                    .map(|entry| entry.path())
                    .find(|path| {
                        path.file_name()
                            .is_some_and(|name| name.to_string_lossy().starts_with("sealed-stage-"))
                    })
                    .expect("sealed staged dump");
                std::fs::rename(&sealed, &hook_displaced).unwrap();
                std::fs::create_dir(&sealed).unwrap();
                std::fs::write(
                    sealed.join("Configuration.xml"),
                    valid_configuration_owner(),
                )
                .unwrap();
                std::fs::write(sealed.join("unvalidated.txt"), b"unvalidated").unwrap();
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(
            std::fs::read(target.join("before.txt")).unwrap(),
            b"before",
            "the original target must be restored before the publication lock is released"
        );
        assert!(
            !target.join("unvalidated.txt").exists(),
            "the source-name replacement must be removed from the Git-visible target"
        );
        assert!(displaced_stage.join("Configuration.xml").is_file());
        assert!(
            result.errors.join("\n").contains("just-installed")
                || result.errors.join("\n").contains("unverified")
                || result.errors.join("\n").contains("quarantine")
                || result
                    .errors
                    .join("\n")
                    .contains("identity changed before atomic rename"),
            "{result:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preserved_recovery_never_contains_effective_config_or_credentials() {
        let (root, context, config) = workspace("recovery-secrets");
        std::fs::write(
            &config,
            "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: 'File=base;Usr=Admin;Pwd=top-secret'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let displaced_stage = context.cwd.join("secret-test-displaced-stage");
        let platform = fixed_platform(&root);
        let runner = DumpRunner {
            owner_xml: valid_configuration_owner(),
            mutation: RunnerMutation::None,
            calls: AtomicUsize::new(0),
        };
        let hook_target = target.clone();
        let hook_displaced_stage = displaced_stage.clone();

        let result = with_publication_hook(
            PublicationCheckpoint::AfterStageInstall,
            move || {
                std::fs::rename(&hook_target, &hook_displaced_stage).unwrap();
                std::fs::create_dir_all(&hook_target).unwrap();
                std::fs::write(hook_target.join("concurrent.txt"), b"concurrent").unwrap();
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        let recovery_roots = std::fs::read_dir(&context.cwd)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(".unica-dump-guard-"))
            })
            .collect::<Vec<_>>();
        assert!(!recovery_roots.is_empty(), "{result:?}");
        for recovery in recovery_roots {
            let snapshot = super::capture_directory_snapshot(&recovery).unwrap();
            assert!(
                snapshot.iter().all(|entry| {
                    entry
                        .relative_path
                        .file_name()
                        .is_none_or(|name| name != super::EFFECTIVE_CONFIG_NAME)
                }),
                "effective config leaked into recovery: {}",
                recovery.display()
            );
            for entry in snapshot {
                if let super::TreeEntryKind::File { .. } = entry.kind {
                    let raw = std::fs::read(recovery.join(entry.relative_path)).unwrap();
                    assert!(
                        !String::from_utf8_lossy(&raw).contains("top-secret"),
                        "credentials leaked into recovery: {}",
                        recovery.display()
                    );
                }
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn private_root_swap_cannot_escape_secret_scrubbing_or_anchor_cleanup() {
        let (root, context, config) = workspace("private-root-swap-cleanup");
        std::fs::write(
            &config,
            "workPath: .work\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: 'File=base;Usr=Admin;Pwd=top-secret'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let platform = fixed_platform(&root);
        let original_identity = std::rc::Rc::new(std::cell::Cell::new(None));
        let runner = DumpRunner {
            owner_xml: valid_configuration_owner(),
            mutation: RunnerMutation::SwapPrivateRoot(std::rc::Rc::clone(&original_identity)),
            calls: AtomicUsize::new(0),
        };

        let result = invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context);

        assert!(!result.ok, "{result:?}");
        assert_eq!(runner.calls.load(Ordering::SeqCst), 1);
        assert!(!context.cwd.join("src").exists(), "{result:?}");
        let private_roots = std::fs::read_dir(&context.cwd)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(".unica-dump-guard-"))
            })
            .collect::<Vec<_>>();
        assert!(
            private_roots.iter().any(|path| path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with("-displaced")),
            "{private_roots:?}; {result:?}"
        );
        for private_root in &private_roots {
            let snapshot = super::capture_directory_snapshot(private_root).unwrap();
            assert!(
                snapshot.iter().all(|entry| {
                    entry
                        .relative_path
                        .file_name()
                        .is_none_or(|name| name != super::EFFECTIVE_CONFIG_NAME)
                }),
                "effective config escaped cleanup: {}",
                private_root.display()
            );
            for entry in snapshot {
                if let super::TreeEntryKind::File { .. } = entry.kind {
                    let raw = std::fs::read(private_root.join(entry.relative_path)).unwrap();
                    assert!(
                        !String::from_utf8_lossy(&raw).contains("top-secret"),
                        "credentials escaped cleanup: {}",
                        private_root.display()
                    );
                }
            }
        }
        assert!(
            result.errors.join("\n").contains("identity changed")
                || result.warnings.join("\n").contains("debris"),
            "{result:?}"
        );
        let original_identity = original_identity.get().expect("root swap must run");
        let displaced = private_roots
            .iter()
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .ends_with("-displaced")
            })
            .unwrap();
        assert_eq!(
            super::file_identity(
                &super::open_directory_nofollow(&std::fs::canonicalize(displaced).unwrap())
                    .unwrap()
            )
            .unwrap(),
            original_identity
        );
        let replacement = private_roots
            .iter()
            .find(|path| {
                !path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .ends_with("-displaced")
            })
            .unwrap();
        assert_ne!(
            super::file_identity(
                &super::open_directory_nofollow(&std::fs::canonicalize(replacement).unwrap())
                    .unwrap()
            )
            .unwrap(),
            original_identity
        );
        assert_eq!(
            std::fs::read(replacement.join("execution/foreign.txt")).unwrap(),
            b"foreign root sentinel"
        );
        let owned_artifacts = result.artifacts.iter().all(|artifact| {
            std::fs::canonicalize(artifact)
                .ok()
                .and_then(|path| super::open_directory_nofollow(&path).ok())
                .and_then(|directory| super::file_identity(&directory).ok())
                == Some(original_identity)
        });
        let location_unconfirmed = result
            .warnings
            .iter()
            .any(|warning| warning.contains("current location is unconfirmed"));
        assert!(owned_artifacts && (!result.artifacts.is_empty() || location_unconfirmed),
            "recovery must report an owned identity or explicit uncertainty: owned={owned_artifacts}, unconfirmed={location_unconfirmed}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn private_recovery_artifacts_require_the_retained_directory_identity() {
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
            let result = super::finalize_private_outcome(
                &mut private,
                super::dump_failure(
                    "unica.build.dump",
                    "fixture caller refused".into(),
                    None,
                    None,
                    None,
                ),
            );
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

    #[cfg(unix)]
    #[test]
    fn private_stage_cleanup_failures_are_reported() {
        use std::os::unix::fs::PermissionsExt;

        let (root, context, _) = workspace("stage-remove-error");
        let platform = fixed_platform(&root);
        let runner = DumpRunner {
            owner_xml: valid_configuration_owner(),
            mutation: RunnerMutation::RestrictPrivateRoot,
            calls: AtomicUsize::new(0),
        };

        let result = invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context);

        assert!(!result.ok, "{result:?}");
        assert!(
            result.errors.join("\n").contains("cleanup failed")
                || result.warnings.join("\n").contains("cleanup failed"),
            "{result:?}"
        );
        for entry in std::fs::read_dir(&context.cwd).unwrap().flatten() {
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(".unica-dump-guard-")
            {
                std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(0o700))
                    .unwrap();
                let execution = entry.path().join("execution");
                if execution.exists() {
                    std::fs::set_permissions(execution, std::fs::Permissions::from_mode(0o700))
                        .unwrap();
                }
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_in_private_stage_is_rejected_before_publication() {
        let (root, context, _) = workspace("stage-symlink");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let platform = fixed_platform(&root);
        let runner = DumpRunner {
            owner_xml: valid_configuration_owner(),
            mutation: RunnerMutation::Symlink,
            calls: AtomicUsize::new(0),
        };

        let result = invoke(
            &runner,
            &platform,
            FullDumpInvocation::RuntimeExecute,
            &context,
        );

        assert!(!result.ok, "{result:?}");
        assert!(result.errors.join("\n").contains("symbolic link"));
        assert_eq!(std::fs::read(target.join("before.txt")).unwrap(), b"before");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn owner_kind_is_bound_to_selected_source_set() {
        let root =
            std::env::temp_dir().join(format!("unica-dump-owner-kind-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("Configuration.xml"), valid_configuration_owner()).unwrap();

        let error = validate_staged_dump(&root, SourceSetKind::Extension).unwrap_err();

        assert!(error.contains("without ConfigurationExtensionPurpose"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staged_tree_uses_closed_root_registry_and_exact_raw_versions() {
        let cases = [
            (
                "known-missing",
                "Forms/Main/Ext/Form.xml",
                r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"/>"#,
                "missing",
            ),
            (
                "known-numeric-equivalent",
                "Forms/Main/Ext/Form.xml",
                r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.200"/>"#,
                "2.200",
            ),
            (
                "unknown-version-bearing",
                "Unknown.xml",
                r#"<Unknown xmlns="urn:unknown" version="2.20"/>"#,
                "unsupported",
            ),
            (
                "unknown-versionless",
                "Unknown.xml",
                r#"<Unknown xmlns="urn:unknown"/>"#,
                "unsupported",
            ),
            (
                "versionless-family-with-version",
                "Templates/Main/Ext/Template.xml",
                r#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema" version="2.20"/>"#,
                "versionless",
            ),
        ];
        for (case, relative, xml, expected) in cases {
            let root = std::env::temp_dir().join(format!(
                "unica-staged-root-registry-{case}-{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join("Configuration.xml"), valid_configuration_owner()).unwrap();
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, xml).unwrap();

            let error = validate_staged_dump(&root, SourceSetKind::Configuration)
                .expect_err("unregistered or inexact staged XML must fail closed");

            assert!(error.contains(expected), "{case}: {error}");
            std::fs::remove_dir_all(root).unwrap();
        }

        let root = std::env::temp_dir().join(format!(
            "unica-staged-root-registry-versionless-control-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(root.join("Templates/Main/Ext")).unwrap();
        std::fs::write(root.join("Configuration.xml"), valid_configuration_owner()).unwrap();
        std::fs::write(
            root.join("Templates/Main/Ext/Template.xml"),
            r#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema"/>"#,
        )
        .unwrap();
        validate_staged_dump(&root, SourceSetKind::Configuration)
            .expect("registered versionless subordinate family remains allowed");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// One staged file and independently recorded version policy per
    /// `STAGED_ROOT_REGISTRY` entry, at the dump-relative path the platform
    /// writes it to.
    ///
    /// Where a checked-in fixture exists the case uses those bytes, keeping the
    /// platform BOM and CRLF. The remaining cases are the smallest document that
    /// still carries the registered root, because the guard reads nothing below
    /// it. `staged_root_registry_covers_every_registered_root` fails if a
    /// registry entry has no case here.
    const STAGED_ROOT_CASES: &[(&str, &[u8], StagedRootVersionPolicy)] = &[
        (
            "Configuration.xml",
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties/></Configuration></MetaDataObject>"#,
            StagedRootVersionPolicy::ExactRootVersion,
        ),
        (
            "Catalogs/Товары/Forms/Форма/Ext/Form.xml",
            br#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"/>"#,
            StagedRootVersionPolicy::ExactRootVersion,
        ),
        (
            "Subsystems/Продажи/Ext/CommandInterface.xml",
            br#"<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"/>"#,
            StagedRootVersionPolicy::ExactRootVersion,
        ),
        (
            "Catalogs/Товары/Ext/Help.xml",
            br#"<Help xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"/>"#,
            StagedRootVersionPolicy::ExactRootVersion,
        ),
        (
            "ExchangePlans/Обмен/Ext/Content.xml",
            include_bytes!("../../../../../tests/fixtures/platform_8_3_27/exchange_plan/Content.xml"),
            StagedRootVersionPolicy::ExactRootVersion,
        ),
        (
            "Ext/HomePageWorkArea.xml",
            br#"<HomePageWorkArea xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"/>"#,
            StagedRootVersionPolicy::ExactRootVersion,
        ),
        (
            "CommonPictures/Логотип/Ext/Picture.xml",
            include_bytes!(
                "../../../../../tests/fixtures/platform_8_3_27/staged_dump_roots/Picture.xml"
            ),
            StagedRootVersionPolicy::ExactRootVersion,
        ),
        (
            "ScheduledJobs/ОбновлениеКурсов/Ext/Schedule.xml",
            include_bytes!(
                "../../../../../tests/fixtures/platform_8_3_27/staged_dump_roots/Schedule.xml"
            ),
            StagedRootVersionPolicy::ExactRootVersion,
        ),
        (
            "Catalogs/Товары/Ext/Predefined.xml",
            include_bytes!(
                "../../../../../tests/fixtures/platform_8_3_27/staged_dump_roots/Predefined.xml"
            ),
            StagedRootVersionPolicy::ExactRootVersion,
        ),
        (
            "ConfigDumpInfo.xml",
            include_bytes!(
                "../../../../../tests/fixtures/platform_8_3_27/staged_dump_roots/ConfigDumpInfo.xml"
            ),
            StagedRootVersionPolicy::ExactRootVersion,
        ),
        (
            "BusinessProcesses/Согласование/Ext/Flowchart.xml",
            br#"<GraphicalSchema xmlns="http://v8.1c.ru/8.3/xcf/scheme" version="2.20"/>"#,
            StagedRootVersionPolicy::ExactRootVersion,
        ),
        (
            "Roles/Администратор/Ext/Rights.xml",
            br#"<Rights xmlns="http://v8.1c.ru/8.2/roles" version="2.20"/>"#,
            StagedRootVersionPolicy::ExactRootVersion,
        ),
        (
            "Reports/Продажи/Templates/Основной/Ext/Template.xml",
            br#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema"/>"#,
            StagedRootVersionPolicy::Versionless,
        ),
        (
            "CommonTemplates/ОформлениеОтчетов/Ext/Template.xml",
            include_bytes!(
                "../../../../../tests/fixtures/platform_8_3_27/staged_dump_roots/Template.xml"
            ),
            StagedRootVersionPolicy::Versionless,
        ),
        (
            "CommonTemplates/ПечатнаяФорма/Ext/Template.xml",
            include_bytes!("../../../../../tests/fixtures/platform_8_3_27/mxl/Template.xml"),
            StagedRootVersionPolicy::Versionless,
        ),
        (
            "WSReferences/СлужбаОбмена/Ext/WSDefinition.xml",
            include_bytes!(
                "../../../../../tests/fixtures/platform_8_3_27/staged_dump_roots/WSDefinition.xml"
            ),
            StagedRootVersionPolicy::Versionless,
        ),
        (
            "Ext/ClientApplicationInterface.xml",
            br#"<ClientApplicationInterface xmlns="http://v8.1c.ru/8.2/managed-application/core"/>"#,
            // Publication syntax is versionless; the independent owner policy
            // records that this document belongs to its containing source set.
            StagedRootVersionPolicy::Versionless,
        ),
    ];

    fn staged_root_case_root(source: &[u8]) -> (String, String) {
        let text = std::str::from_utf8(source).expect("staged root case is UTF-8");
        let document = Document::parse(text.trim_start_matches('\u{feff}'))
            .expect("staged root case parses as XML");
        let root = document.root_element();
        (
            root.tag_name().namespace().unwrap_or_default().to_string(),
            root.tag_name().name().to_string(),
        )
    }

    fn staged_root_case_tree(case: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "unica-staged-root-case-{case}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("Configuration.xml"), valid_configuration_owner()).unwrap();
        root
    }

    #[test]
    fn staged_root_registry_covers_every_registered_root() {
        let covered = STAGED_ROOT_CASES
            .iter()
            .map(|(_, source, _)| staged_root_case_root(source))
            .collect::<BTreeSet<_>>();
        let uncovered = STAGED_ROOT_REGISTRY
            .iter()
            .filter(|root| {
                !covered.contains(&(root.namespace.to_string(), root.local_name.to_string()))
            })
            .map(|root| format!("{{{}}}{}", root.namespace, root.local_name))
            .collect::<Vec<_>>();

        assert!(
            uncovered.is_empty(),
            "every registered staged root needs a case: {uncovered:?}"
        );
    }

    #[test]
    fn staged_root_registry_publishes_every_platform_8_3_27_dump_root() {
        let root = staged_root_case_tree("accepted");
        for (relative, source, _) in STAGED_ROOT_CASES {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, source).unwrap();
        }

        validate_staged_dump(&root, SourceSetKind::Configuration).unwrap_or_else(|error| {
            panic!("every platform-written dump root must stay publishable: {error}")
        });

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staged_root_registry_binds_each_platform_root_to_its_version_policy() {
        for (index, (relative, source, expected_policy)) in STAGED_ROOT_CASES.iter().enumerate() {
            let (namespace, local_name) = staged_root_case_root(source);
            let policy = staged_root_version_policy(&namespace, &local_name)
                .unwrap_or_else(|| panic!("{relative}: case root must be registered"));
            assert_eq!(
                policy, *expected_policy,
                "{relative}: registry policy must match the independently recorded platform evidence"
            );
            let text = std::str::from_utf8(source).unwrap();
            let source_without_bom = text.trim_start_matches('\u{feff}');
            let document = Document::parse(source_without_bom).unwrap();
            let root_node = document.root_element();
            let root_start = text.len() - source_without_bom.len() + root_node.range().start;
            let root_tag_end = root_start
                + text[root_start..]
                    .find('>')
                    .expect("case root opening tag is well formed");
            let root_tag = &text[root_start..root_tag_end];
            let (mutated, expected) = match expected_policy {
                StagedRootVersionPolicy::ExactRootVersion => {
                    assert_eq!(
                        root_node.attribute("version"),
                        Some("2.20"),
                        "{relative}: the root must carry the exact 2.20 value"
                    );
                    let version_literal = r#"version="2.20""#;
                    let version_start = root_start
                        + root_tag.find(version_literal).unwrap_or_else(|| {
                            panic!("{relative}: root must carry the raw 2.20 literal")
                        });
                    let mut mutated = text.to_string();
                    mutated.replace_range(
                        version_start..version_start + version_literal.len(),
                        r#"version="2.19""#,
                    );
                    (mutated, "2.19")
                }
                StagedRootVersionPolicy::Versionless => {
                    assert!(
                        root_node.attribute("version").is_none(),
                        "{relative}: the root must stay versionless"
                    );
                    let tag_name_end = root_start
                        + text[root_start..]
                            .find(|character: char| {
                                character.is_whitespace() || character == '/' || character == '>'
                            })
                            .expect("case root element is well formed");
                    let mut mutated = text.to_string();
                    mutated.insert_str(tag_name_end, r#" version="2.20""#);
                    (mutated, "versionless")
                }
            };
            let root = staged_root_case_tree(&index.to_string());
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, mutated).unwrap();

            let error = validate_staged_dump(&root, SourceSetKind::Configuration)
                .expect_err("a registered root with the wrong version policy must fail closed");

            assert!(error.contains(expected), "{relative}: {error}");
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn staged_xml_bytes_must_match_the_tree_snapshot_even_if_path_is_restored() {
        let root =
            std::env::temp_dir().join(format!("unica-staged-read-race-{}", uuid::Uuid::new_v4()));
        let form = root.join("Forms/Main/Ext/Form.xml");
        std::fs::create_dir_all(form.parent().unwrap()).unwrap();
        std::fs::write(root.join("Configuration.xml"), valid_configuration_owner()).unwrap();
        let invalid =
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.19"/>"#.to_string();
        let valid = r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"/>"#.to_string();
        std::fs::write(&form, &invalid).unwrap();
        let form_before = form.clone();
        let form_after = form.clone();

        let result = with_targeted_read_hooks(
            form.clone(),
            move || std::fs::write(&form_before, valid).unwrap(),
            move || std::fs::write(&form_after, invalid).unwrap(),
            || validate_staged_dump(&root, SourceSetKind::Configuration),
        );

        let error = result.expect_err(
            "XML bytes parsed during validation must be bound to the captured tree snapshot",
        );
        assert!(
            error.contains("snapshot") || error.contains("changed"),
            "{error}"
        );
        assert!(
            std::fs::read_to_string(&form).unwrap().contains("2.19"),
            "the test must restore the invalid preimage before validation returns"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_swap_between_file_inspection_and_open_is_rejected() {
        let root =
            std::env::temp_dir().join(format!("unica-staged-file-swap-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let owner = root.join("Configuration.xml");
        let original = root.join("Configuration.original.xml");
        let alternate = root.join("Configuration.alternate.xml");
        std::fs::write(&owner, valid_configuration_owner()).unwrap();
        std::fs::write(&alternate, valid_configuration_owner()).unwrap();
        let owner_for_hook = owner.clone();
        let original_for_hook = original.clone();
        let alternate_for_hook = alternate.clone();

        let result = with_secure_read_hook(
            move |path| {
                assert_eq!(path, owner_for_hook);
                std::fs::rename(&owner_for_hook, &original_for_hook).unwrap();
                std::os::unix::fs::symlink(&alternate_for_hook, &owner_for_hook).unwrap();
            },
            || super::secure_read_regular_file(&owner, "swap fixture"),
        );

        let error = result.expect_err("O_NOFOLLOW/identity binding must reject the swap");
        assert!(
            error.contains("symbolic link")
                || error.contains("changed")
                || error.contains("identity"),
            "{error}"
        );
        std::fs::remove_file(&owner).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_swap_between_directory_inspection_and_open_is_rejected() {
        let root = std::env::temp_dir().join(format!(
            "unica-staged-directory-swap-{}",
            uuid::Uuid::new_v4()
        ));
        let target = root.join("target");
        let original = root.join("target-original");
        let outside = root.join("outside");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(target.join("inside"), b"inside").unwrap();
        std::fs::write(outside.join("outside"), b"outside").unwrap();
        let target_for_hook = target.clone();
        let original_for_hook = original.clone();
        let outside_for_hook = outside.clone();

        let result = with_tree_open_hook(
            move |path| {
                assert_eq!(path, target_for_hook);
                std::fs::rename(&target_for_hook, &original_for_hook).unwrap();
                std::os::unix::fs::symlink(&outside_for_hook, &target_for_hook).unwrap();
            },
            || TreeSnapshot::capture_target(&target),
        );

        let error = result.expect_err("directory descriptor open must reject the swap");
        assert!(
            error.contains("symbolic link")
                || error.contains("changed")
                || error.contains("identity"),
            "{error}"
        );
        std::fs::remove_file(&target).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    fn canonical_publication(
        context: &WorkspaceContext,
        config: &Path,
        store_path: &Path,
        runner: &dyn ProcessRunner,
    ) -> super::VerifiedFullDumpPublication {
        let cancellation = CancellationToken::new();
        let store = super::RunnerProjectStore::open_and_lock(store_path, &cancellation).unwrap();
        super::VerifiedFullDumpPublication::prepare(
            context,
            super::FullDumpSelection {
                config_path: config,
                source_set: Some("main"),
                extension: None,
                expected_target: None,
            },
            store,
            runner,
            &cancellation,
            |mut primary, local, _| {
                if let Some(local) = local {
                    super::merge_yaml_values(&mut primary, local);
                }
                Ok(primary)
            },
            |effective, _| Ok((effective, None)),
        )
        .unwrap()
    }

    fn fill_canonical_stage(publication: &mut super::VerifiedFullDumpPublication) {
        use sha2::{Digest, Sha256};
        use std::io::Write;

        assert!(
            publication.staged_target().is_dir(),
            "canonical preparation must create the held stage"
        );
        let stage = publication.staged_target().to_path_buf();
        let owner = valid_configuration_owner();
        let mut entries = Vec::new();
        for (name, bytes) in [
            ("Configuration.xml", owner.as_bytes()),
            ("payload.bsl", b"validated source".as_slice()),
        ] {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(stage.join(name))
                .unwrap();
            file.write_all(bytes).unwrap();
            entries.push(super::TreeEntrySnapshot {
                relative_path: PathBuf::from(name),
                kind: super::TreeEntryKind::File {
                    identity: super::file_identity(&file).unwrap(),
                    size: bytes.len() as u64,
                    sha256: Sha256::digest(bytes).into(),
                },
            });
        }
        publication
            .prepared
            .private
            .remember_staged_tree(&TreeSnapshot::Directory {
                identity: publication.prepared.canonical_stage_identity.unwrap(),
                entries,
            });
    }

    fn attest_canonical(
        publication: &mut super::VerifiedFullDumpPublication,
        platform: &FixedPlatform,
        runner: &dyn ProcessRunner,
        cancellation: &CancellationToken,
    ) {
        super::with_test_platform(&platform.executable, &platform.version, || {
            publication
                .attest_selected_local(&platform.executable, runner, cancellation)
                .unwrap()
        });
    }

    #[test]
    fn canonical_preparation_creates_an_empty_owner_only_held_stage_before_dispatch() {
        let (root, context, config) = workspace("canonical-created-empty-stage");
        let runner = DumpRunner::valid();
        let publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        let stage = publication.staged_target().to_path_buf();
        let private_root = publication.prepared.private.root.clone();
        assert!(stage.is_dir());
        assert_eq!(std::fs::read_dir(&stage).unwrap().count(), 0);
        let retained = publication
            .prepared
            .canonical_stage_anchor
            .as_ref()
            .unwrap();
        super::verify_private_owner(&retained.directory).unwrap();
        assert_eq!(
            publication
                .prepared
                .private
                .execution_anchor()
                .capture_child_root_identity(std::ffi::OsStr::new("staged-source"), &stage)
                .unwrap(),
            Some(retained.identity)
        );
        publication.recheck_before_dispatch().unwrap();
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        assert!(!context.cwd.join("src").exists());
        publication.abort().unwrap();
        assert!(
            !private_root.exists(),
            "terminal cleanup must release the created child handle on Windows too"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    fn verify_canonical_stage_handoff_replacement_cleanup(explicit_abort: bool) {
        let (root, context, config) = workspace("canonical-stage-handoff-replacement");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let original_target = TreeSnapshot::capture_target(&target).unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        fill_canonical_stage(&mut publication);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        let owned_identity = publication
            .prepared
            .canonical_stage_anchor
            .as_ref()
            .unwrap()
            .identity;
        let stage = publication.staged_target().to_path_buf();
        let displaced =
            super::normalize_path_identity(&context.cwd.join("owned-stage-displaced-at-handoff"))
                .unwrap();
        let hook_stage = stage.clone();
        let hook_displaced = displaced.clone();
        let publication_result = with_publication_hook(
            PublicationCheckpoint::BeforeStageInstall,
            move || {
                std::fs::rename(&hook_stage, &hook_displaced).unwrap();
                std::fs::create_dir(&hook_stage).unwrap();
                std::fs::write(
                    hook_stage.join("foreign.txt"),
                    b"foreign bytes after handle release",
                )
                .unwrap();
            },
            || publication.publish(&cancellation),
        );
        assert!(
            publication_result.is_err(),
            "seal must refuse the replaced stage root"
        );
        assert!(!cancellation.protected_process_started());
        let foreign_identity =
            super::file_identity(&super::open_directory_nofollow(&stage).unwrap()).unwrap();
        assert_ne!(foreign_identity, owned_identity);
        let cleanup = if explicit_abort {
            Some(publication.abort())
        } else {
            drop(publication);
            None
        };
        assert_eq!(
            std::fs::read(stage.join("foreign.txt")).unwrap(),
            b"foreign bytes after handle release",
            "terminal cleanup must preserve the foreign stage after a failed handoff"
        );
        assert_eq!(
            super::file_identity(&super::open_directory_nofollow(&stage).unwrap()).unwrap(),
            foreign_identity
        );
        assert_eq!(
            super::file_identity(&super::open_directory_nofollow(&displaced).unwrap()).unwrap(),
            owned_identity
        );
        assert_eq!(
            std::fs::read(displaced.join("payload.bsl")).unwrap(),
            b"validated source"
        );
        assert_eq!(
            TreeSnapshot::capture_target(&target).unwrap(),
            original_target
        );
        if let Some(cleanup) = cleanup {
            let failure = cleanup.unwrap_err();
            assert!(matches!(failure.phase, super::FullDumpPhase::Integrity));
            assert!(matches!(&failure.recovery, Ok(Some(_))));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_stage_handoff_replacement_survives_explicit_abort() {
        verify_canonical_stage_handoff_replacement_cleanup(true);
    }

    #[test]
    fn canonical_stage_handoff_replacement_survives_unexpected_drop() {
        verify_canonical_stage_handoff_replacement_cleanup(false);
    }

    #[cfg(unix)]
    fn verify_canonical_stage_cleanup_ancestor_replacement(explicit_abort: bool) {
        let (root, context, config) = workspace("canonical-stage-cleanup-ancestor-replacement");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let original_target = TreeSnapshot::capture_target(&target).unwrap();
        let runner = DumpRunner::valid();
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        fill_canonical_stage(&mut publication);
        let private_root = publication.prepared.private.root.clone();
        let execution = publication.prepared.private.execution.clone();
        let owned_stage_identity = publication
            .prepared
            .canonical_stage_anchor
            .as_ref()
            .unwrap()
            .identity;
        let owned_execution_identity = publication.prepared.private.execution_anchor().identity;
        let displaced = private_root.join("displaced-execution");
        std::fs::rename(&execution, &displaced).unwrap();
        std::fs::create_dir(&execution).unwrap();
        std::fs::write(
            execution.join("foreign.txt"),
            b"foreign execution replacement sentinel",
        )
        .unwrap();
        let foreign_execution_identity =
            super::file_identity(&super::open_directory_nofollow(&execution).unwrap()).unwrap();
        assert_ne!(foreign_execution_identity, owned_execution_identity);
        let cleanup = if explicit_abort {
            Some(publication.abort())
        } else {
            drop(publication);
            None
        };
        assert_eq!(
            std::fs::read(execution.join("foreign.txt")).unwrap(),
            b"foreign execution replacement sentinel",
            "cleanup must refuse a replaced execution ancestor before reading its held child"
        );
        assert_eq!(
            super::file_identity(&super::open_directory_nofollow(&execution).unwrap()).unwrap(),
            foreign_execution_identity
        );
        assert_eq!(
            super::file_identity(&super::open_directory_nofollow(&displaced).unwrap()).unwrap(),
            owned_execution_identity
        );
        assert_eq!(
            super::file_identity(
                &super::open_directory_nofollow(&displaced.join("staged-source")).unwrap()
            )
            .unwrap(),
            owned_stage_identity
        );
        assert_eq!(
            std::fs::read(displaced.join("staged-source/payload.bsl")).unwrap(),
            b"validated source"
        );
        assert_eq!(
            TreeSnapshot::capture_target(&target).unwrap(),
            original_target
        );
        if let Some(cleanup) = cleanup {
            let failure = cleanup.unwrap_err();
            assert!(matches!(failure.phase, super::FullDumpPhase::Integrity));
            assert_eq!(failure.recovery, Ok(Some(private_root)));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn canonical_stage_cleanup_ancestor_replacement_survives_explicit_abort() {
        verify_canonical_stage_cleanup_ancestor_replacement(true);
    }

    #[cfg(unix)]
    #[test]
    fn canonical_stage_cleanup_ancestor_replacement_survives_unexpected_drop() {
        verify_canonical_stage_cleanup_ancestor_replacement(false);
    }

    #[test]
    fn canonical_stage_replacement_is_refused_and_foreign_tree_survives_abort() {
        let (root, context, config) = workspace("canonical-held-stage-replacement");
        seed_valid_target(&context.cwd.join("src"));
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        fill_canonical_stage(&mut publication);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        let stage = publication.staged_target().to_path_buf();
        let private_root = publication.prepared.private.root.clone();
        let displaced = context.cwd.join("retained-owned-stage");
        std::fs::rename(&stage, &displaced).unwrap();
        std::fs::create_dir(&stage).unwrap();
        std::fs::write(stage.join("foreign.txt"), b"foreign stage replacement").unwrap();
        assert!(matches!(
            publication.recheck_before_dispatch().unwrap_err().phase,
            super::FullDumpPhase::Integrity
        ));
        assert!(matches!(
            publication.publish(&cancellation).unwrap_err().phase,
            super::FullDumpPhase::Integrity
        ));
        assert!(!cancellation.protected_process_started());
        assert_eq!(
            std::fs::read(context.cwd.join("src/before.txt")).unwrap(),
            b"before"
        );
        let cleanup = publication.abort().unwrap_err();
        assert!(matches!(cleanup.phase, super::FullDumpPhase::Integrity));
        assert_eq!(cleanup.recovery, Ok(Some(private_root)));
        assert_eq!(
            std::fs::read(stage.join("foreign.txt")).unwrap(),
            b"foreign stage replacement"
        );
        assert_eq!(
            std::fs::read(displaced.join("payload.bsl")).unwrap(),
            b"validated source"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_stage_replacement_also_survives_an_unexpected_guard_drop() {
        let (root, context, config) = workspace("canonical-held-stage-drop");
        let runner = DumpRunner::valid();
        let publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        let stage = publication.staged_target().to_path_buf();
        let displaced = context.cwd.join("retained-owned-empty-stage");
        std::fs::rename(&stage, &displaced).unwrap();
        std::fs::create_dir(&stage).unwrap();
        std::fs::write(stage.join("foreign.txt"), b"preserved on implicit drop").unwrap();
        drop(publication);
        assert_eq!(
            std::fs::read(stage.join("foreign.txt")).unwrap(),
            b"preserved on implicit drop"
        );
        assert!(displaced.is_dir());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_preparation_keeps_the_runner_owned_stage_creation_behavior() {
        let (root, context, _) = workspace("legacy-stage-remains-runner-owned");
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut prepared = super::PreparedDump::prepare(
            &args(),
            &context,
            &platform,
            &runner,
            &CancellationToken::new(),
        )
        .unwrap();
        assert!(prepared.canonical_stage_anchor.is_none());
        assert!(!prepared.private.staged_tree.exists());
        prepared.private.cleanup_now().unwrap();
        drop(prepared);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_preparation_keeps_logical_target_and_attests_only_the_selected_executable() {
        let (root, context, config) = workspace("canonical-selected-provider");
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        assert_eq!(
            runner.calls.load(Ordering::SeqCst),
            0,
            "preparation must not probe a provider"
        );
        let effective: serde_yaml::Value =
            serde_yaml::from_slice(&std::fs::read(publication.effective_config()).unwrap())
                .unwrap();
        assert_eq!(
            effective["source-set"][0]["path"].as_str(),
            Some(publication.target().to_str().unwrap())
        );
        assert!(
            effective.get("tools").is_none(),
            "selection must not be forced through tools.platform"
        );
        fill_canonical_stage(&mut publication);
        assert!(
            publication.publish(&CancellationToken::new()).is_err(),
            "an unattested provider cannot publish"
        );
        super::with_test_platform(&platform.executable, &platform.version, || {
            assert!(publication
                .attest_selected_local(
                    &root.join("foreign/1cv8"),
                    &runner,
                    &CancellationToken::new()
                )
                .is_err());
        });
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        assert_eq!(publication.selected_extension(), None);
        publication.abort().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_nested_target_prepares_parents_after_selected_and_retains_store_until_finish() {
        let (root, context, config) = workspace("canonical-nested-target");
        std::fs::write(
            &config,
            std::fs::read_to_string(&config)
                .unwrap()
                .replace("path: src", "path: newsrc/nested/Cf"),
        )
        .unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let store_path = root.join("projection");
        let mut publication = canonical_publication(&context, &config, &store_path, &runner);
        let ancestor = super::normalize_path_identity(&context.cwd).unwrap();
        assert!(publication.staged_target().starts_with(&ancestor));
        assert!(!context.cwd.join("newsrc").exists());
        fill_canonical_stage(&mut publication);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        assert!(context.cwd.join("newsrc/nested").is_dir());
        assert!(
            !cancellation.protected_process_started(),
            "dispatch parent preparation is reversible"
        );
        let retained_parent = publication
            .prepared
            .opened_target_parent
            .as_ref()
            .unwrap()
            .identity;
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        assert_eq!(
            publication.prepared.created_target_parents.len(),
            2,
            "repeated preparation must keep one ledger"
        );
        assert_eq!(
            publication
                .prepared
                .opened_target_parent
                .as_ref()
                .unwrap()
                .identity,
            retained_parent
        );
        let result = publication.publish(&cancellation).unwrap();
        assert!(!result.replaced, "the retained initial target was absent");
        assert_eq!(result.files, 2);
        assert!(
            publication.prepared.canonical_stage_anchor.is_none(),
            "the shared publisher takes stage ownership after Ready validation"
        );
        assert!(publication.is_published());
        assert!(cancellation.protected_process_started());
        cancellation.cancel();
        assert_eq!(
            publication.publish(&cancellation).unwrap().files,
            2,
            "publication fact survives cancellation and repeated calls"
        );
        assert_eq!(
            std::fs::read(context.cwd.join("newsrc/nested/Cf/payload.bsl")).unwrap(),
            b"validated source"
        );
        let waiting = CancellationToken::new();
        let thread_cancel = waiting.clone();
        let waiting_path = store_path.clone();
        let (contended_tx, contended_rx) = std::sync::mpsc::channel();
        let contender = std::thread::spawn(move || {
            super::TEST_PROJECT_LOCK_CONTENDED.with(|slot| {
                *slot.borrow_mut() = Some(Box::new(move || contended_tx.send(()).unwrap()))
            });
            super::RunnerProjectStore::open_and_lock(&waiting_path, &thread_cancel).err()
        });
        contended_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        waiting.cancel();
        assert!(contender.join().unwrap().is_some());
        publication.finish().unwrap();
        let acquired =
            super::RunnerProjectStore::open_and_lock(&store_path, &CancellationToken::new())
                .unwrap();
        drop(acquired);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_selected_parent_preparation_aborts_only_its_own_empty_ancestry() {
        let (root, context, config) = workspace("canonical-selected-parent-abort");
        std::fs::create_dir(context.cwd.join("newsrc")).unwrap();
        std::fs::write(
            &config,
            std::fs::read_to_string(&config)
                .unwrap()
                .replace("path: src", "path: newsrc/nested/deeper/Cf"),
        )
        .unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        assert!(
            !context.cwd.join("newsrc/nested").exists(),
            "preparation before Selected stays write-free for source ancestry"
        );
        let cancellation = CancellationToken::new();
        assert!(
            publication.prepare_dispatch_parent(&cancellation).is_err(),
            "unattested Selected cannot create source ancestry"
        );
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        assert!(context.cwd.join("newsrc/nested/deeper").is_dir());
        assert!(!publication.target().exists());
        assert!(!cancellation.protected_process_started());
        publication.abort().unwrap();
        assert!(
            context.cwd.join("newsrc").is_dir(),
            "pre-existing parent must survive"
        );
        assert!(!context.cwd.join("newsrc/nested").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_selected_parent_abort_preserves_a_late_foreign_file_with_typed_recovery() {
        let (root, context, config) = workspace("canonical-selected-parent-foreign-file");
        std::fs::write(
            &config,
            std::fs::read_to_string(&config)
                .unwrap()
                .replace("path: src", "path: newsrc/nested/Cf"),
        )
        .unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        let foreign = context.cwd.join("newsrc/nested/foreign.txt");
        std::fs::write(&foreign, b"created by another owner after Selected").unwrap();
        let failure = publication.abort().unwrap_err();
        assert!(matches!(failure.phase, super::FullDumpPhase::Cleanup));
        assert!(matches!(&failure.recovery, Ok(Some(_))));
        assert_eq!(
            std::fs::read(&foreign).unwrap(),
            b"created by another owner after Selected"
        );
        assert!(!cancellation.protected_process_started());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn canonical_selected_parent_abort_preserves_a_replaced_parent_identity() {
        let (root, context, config) = workspace("canonical-selected-parent-identity-swap");
        std::fs::write(
            &config,
            std::fs::read_to_string(&config)
                .unwrap()
                .replace("path: src", "path: newsrc/nested/Cf"),
        )
        .unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        let parent = context.cwd.join("newsrc/nested");
        let displaced = context.cwd.join("newsrc/displaced-nested");
        std::fs::rename(&parent, &displaced).unwrap();
        std::fs::create_dir(&parent).unwrap();
        std::fs::write(parent.join("foreign.txt"), b"foreign parent replacement").unwrap();
        let failure = publication.abort().unwrap_err();
        assert!(matches!(&failure.recovery, Ok(Some(_))));
        assert_eq!(
            std::fs::read(parent.join("foreign.txt")).unwrap(),
            b"foreign parent replacement"
        );
        assert!(
            displaced.is_dir(),
            "the original retained identity must also survive"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_cancelled_selected_parent_preparation_has_a_causal_typed_refusal() {
        let (root, context, config) = workspace("canonical-selected-parent-cancel");
        std::fs::write(
            &config,
            std::fs::read_to_string(&config)
                .unwrap()
                .replace("path: src", "path: newsrc/nested/Cf"),
        )
        .unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        cancellation.cancel();
        let failure = publication
            .prepare_dispatch_parent(&cancellation)
            .unwrap_err();
        assert!(matches!(failure.phase, super::FullDumpPhase::Cancelled));
        assert!(!context.cwd.join("newsrc").exists());
        assert!(!cancellation.protected_process_started());
        publication.abort().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_publish_never_recreates_unprepared_dispatch_parents() {
        let (root, context, config) = workspace("canonical-unprepared-dispatch-parent");
        std::fs::write(
            &config,
            std::fs::read_to_string(&config)
                .unwrap()
                .replace("path: src", "path: newsrc/nested/Cf"),
        )
        .unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        fill_canonical_stage(&mut publication);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        assert!(publication.publish(&cancellation).is_err());
        assert!(!context.cwd.join("newsrc").exists());
        assert!(!cancellation.protected_process_started());
        publication.abort().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_cancel_before_commit_leaves_missing_source_parents_absent() {
        let (root, context, config) = workspace("canonical-cancel-missing-parent");
        std::fs::write(
            &config,
            std::fs::read_to_string(&config)
                .unwrap()
                .replace("path: src", "path: newsrc/nested/Cf"),
        )
        .unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        fill_canonical_stage(&mut publication);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        let hook_cancellation = cancellation.clone();
        let result = with_publication_hook(
            PublicationCheckpoint::BeforeStageInstall,
            move || hook_cancellation.cancel(),
            || publication.publish(&cancellation),
        );
        assert!(matches!(
            result.unwrap_err().phase,
            super::FullDumpPhase::Cancelled
        ));
        assert!(!publication.is_published());
        assert!(!cancellation.protected_process_started());
        assert!(
            context.cwd.join("newsrc/nested").is_dir(),
            "dispatch ancestry stays held until terminal cleanup"
        );
        publication.abort().unwrap();
        assert!(!context.cwd.join("newsrc").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_late_cancellation_does_not_interrupt_the_shared_publication_transaction() {
        let (root, context, config) = workspace("canonical-late-cancel");
        seed_valid_target(&context.cwd.join("src"));
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        fill_canonical_stage(&mut publication);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        let hook_cancellation = cancellation.clone();
        let result = with_publication_hook(
            PublicationCheckpoint::AfterBackup,
            move || {
                assert!(
                    hook_cancellation.protected_process_started(),
                    "first namespace effect must be sticky before scanning/commit"
                );
                hook_cancellation.cancel();
            },
            || publication.publish(&cancellation),
        );
        assert!(
            result.is_ok(),
            "late cancellation must allow the held transaction to finish"
        );
        assert!(
            result.unwrap().replaced,
            "the retained initial tree was replaced"
        );
        assert!(publication.is_published());
        assert!(cancellation.is_cancelled());
        assert_eq!(
            std::fs::read(context.cwd.join("src/payload.bsl")).unwrap(),
            b"validated source"
        );
        publication.finish().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_shared_publisher_keeps_post_rename_content_checks_and_rolls_back_mutation() {
        let (root, context, config) = workspace("canonical-post-rename-content-change");
        seed_valid_target(&context.cwd.join("src"));
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        fill_canonical_stage(&mut publication);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        let hook_target = context.cwd.join("src/payload.bsl");
        let hook_cancellation = cancellation.clone();
        let result = with_publication_hook(
            PublicationCheckpoint::AfterStageInstall,
            move || {
                std::fs::write(hook_target, b"unvalidated same-inode content").unwrap();
                hook_cancellation.cancel();
            },
            || publication.publish(&cancellation),
        );
        assert!(
            matches!(result.unwrap_err().phase, super::FullDumpPhase::Publication),
            "post-rename mutation stays a publication failure even if cancellation arrives later"
        );
        assert!(!publication.is_published());
        assert!(
            cancellation.protected_process_started(),
            "an attempted public effect stays factual even after rollback"
        );
        assert_eq!(
            std::fs::read(context.cwd.join("src/before.txt")).unwrap(),
            b"before"
        );
        publication.abort().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_missing_parent_refuses_a_foreign_directory_without_adopting_it() {
        let (root, context, config) = workspace("canonical-foreign-parent");
        std::fs::write(
            &config,
            std::fs::read_to_string(&config)
                .unwrap()
                .replace("path: src", "path: newsrc/nested/Cf"),
        )
        .unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        fill_canonical_stage(&mut publication);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        std::fs::create_dir(context.cwd.join("newsrc")).unwrap();
        std::fs::write(context.cwd.join("newsrc/foreign.txt"), b"foreign parent").unwrap();
        assert!(publication.prepare_dispatch_parent(&cancellation).is_err());
        assert!(publication.publish(&cancellation).is_err());
        assert_eq!(
            std::fs::read(context.cwd.join("newsrc/foreign.txt")).unwrap(),
            b"foreign parent"
        );
        assert!(!context.cwd.join("newsrc/nested").exists());
        publication.abort().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_failed_commit_removes_only_its_empty_missing_parents() {
        let (root, context, config) = workspace("canonical-parent-rollback");
        std::fs::write(
            &config,
            std::fs::read_to_string(&config)
                .unwrap()
                .replace("path: src", "path: newsrc/nested/Cf"),
        )
        .unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        fill_canonical_stage(&mut publication);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        let result =
            with_publication_failpoint(PublicationCheckpoint::BeforeSealedPublishRename, || {
                publication.publish(&cancellation)
            });
        assert!(result.is_err());
        assert!(context.cwd.join("newsrc/nested").is_dir());
        publication.abort().unwrap();
        assert!(!context.cwd.join("newsrc").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_parent_rollback_preserves_foreign_children_in_its_created_chain() {
        let (root, context, config) = workspace("canonical-parent-foreign-child");
        std::fs::write(
            &config,
            std::fs::read_to_string(&config)
                .unwrap()
                .replace("path: src", "path: newsrc/nested/Cf"),
        )
        .unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        fill_canonical_stage(&mut publication);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        let foreign = context.cwd.join("newsrc/nested/foreign.txt");
        let hook_foreign = foreign.clone();
        let result = with_publication_hook(
            PublicationCheckpoint::BeforeSealedPublishRename,
            move || {
                std::fs::write(&hook_foreign, b"concurrent source sibling").unwrap();
            },
            || {
                with_publication_failpoint(PublicationCheckpoint::BeforeSealedPublishRename, || {
                    publication.publish(&cancellation)
                })
            },
        );
        assert!(result.is_err());
        assert_eq!(
            std::fs::read(&foreign).unwrap(),
            b"concurrent source sibling"
        );
        assert!(!publication.target().exists());
        let cleanup = publication.abort().unwrap_err();
        assert!(
            matches!(&cleanup.recovery, Ok(Some(_))),
            "foreign sibling must retain a typed recovery boundary"
        );
        assert_eq!(
            std::fs::read(&foreign).unwrap(),
            b"concurrent source sibling"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn canonical_cleanup_failure_after_publication_is_a_warning_with_changed_sources() {
        let (root, context, config) = workspace("canonical-published-cleanup-warning");
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        fill_canonical_stage(&mut publication);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        publication.publish(&cancellation).unwrap();
        let public_before_cleanup =
            super::capture_tree_target_nofollow(&context.cwd.join("src")).unwrap();
        let private = publication.prepared.private.root.clone();
        let owned_before = super::capture_tree_target_nofollow(&private).unwrap();
        let displaced = context.cwd.join("displaced-private");
        std::fs::rename(&private, &displaced).unwrap();
        std::fs::create_dir(&private).unwrap();
        std::fs::write(private.join("foreign.txt"), b"preserve foreign root").unwrap();
        let foreign_before = super::capture_tree_target_nofollow(&private).unwrap();
        let finish = publication
            .finish()
            .expect("published cleanup failure must not become unchanged failure");
        assert_eq!(finish.warnings.len(), 1);
        let location_note = finish
            .retained
            .expect_err("a foreign replacement is not the retained private directory");
        assert!(location_note.contains("current location is unconfirmed"));
        assert!(location_note.contains(&format!("original name: {}", private.display())));
        assert_eq!(
            super::capture_tree_target_nofollow(&displaced).unwrap(),
            owned_before,
            "finalization must preserve every owned private identity and byte"
        );
        assert_eq!(
            super::capture_tree_target_nofollow(&private).unwrap(),
            foreign_before,
            "finalization must preserve every foreign identity and byte"
        );
        assert_eq!(
            super::capture_tree_target_nofollow(&context.cwd.join("src")).unwrap(),
            public_before_cleanup,
            "a private reporting failure must preserve the verified publication"
        );
        assert_eq!(
            std::fs::read(context.cwd.join("src/payload.bsl")).unwrap(),
            b"validated source"
        );
        assert_eq!(
            std::fs::read(private.join("foreign.txt")).unwrap(),
            b"preserve foreign root"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    fn projection_values(password: &str) -> (serde_yaml::Value, serde_yaml::Value) {
        let primary =
            serde_yaml::from_str("workPath: .work\nformat: DESIGNER\nsource-set: []\n").unwrap();
        let local = serde_yaml::from_str(&format!(
            "infobases:\n  origin:\n    connection: 'File=/retained/owned/base'\n    password: '{password}'\n")).unwrap();
        (primary, local)
    }

    fn read_projection_primary(path: &Path) -> serde_yaml::Value {
        serde_yaml::from_slice(&std::fs::read(path.join(super::EFFECTIVE_CONFIG_NAME)).unwrap())
            .unwrap()
    }

    #[test]
    fn runner_project_projection_keeps_a_null_primary_through_the_local_gap() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("projection");
        let mut store =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        let (primary, before) = projection_values("before");
        let (_, after) = projection_values("after");
        store.write_projection(&primary, Some(&before)).unwrap();
        let observed = std::rc::Rc::new(std::cell::Cell::new(false));
        let hook_observed = observed.clone();
        super::with_runner_projection_hook(
            super::RunnerProjectionCheckpoint::LocalGap,
            move |path| {
                assert!(
                    !path.join(super::LOCAL_CONFIG_NAME).exists(),
                    "the hook must observe the real displacement gap"
                );
                assert!(
                    read_projection_primary(path).is_null(),
                    "owner guard must be unreadable while the local layer is absent"
                );
                let guard = std::fs::File::open(path.join(super::EFFECTIVE_CONFIG_NAME)).unwrap();
                super::verify_private_owner(&guard).unwrap();
                hook_observed.set(true);
            },
            || store.write_projection(&primary, Some(&after)),
        )
        .unwrap();
        assert!(observed.get());
        assert_eq!(read_projection_primary(&path), primary);
        let projected: serde_yaml::Value =
            serde_yaml::from_slice(&std::fs::read(path.join(super::LOCAL_CONFIG_NAME)).unwrap())
                .unwrap();
        assert_eq!(projected, after);
        store.verify().unwrap();
    }

    #[test]
    fn runner_project_projection_removal_keeps_the_guard_until_local_absence_is_confirmed() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("projection");
        let mut store =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        let (primary, local) = projection_values("before");
        store.write_projection(&primary, Some(&local)).unwrap();
        let observed = std::rc::Rc::new(std::cell::Cell::new(false));
        let hook_observed = observed.clone();
        super::with_runner_projection_hook(
            super::RunnerProjectionCheckpoint::LocalGap,
            move |path| {
                assert!(!path.join(super::LOCAL_CONFIG_NAME).exists());
                assert!(read_projection_primary(path).is_null());
                hook_observed.set(true);
            },
            || store.write_projection(&primary, None),
        )
        .unwrap();
        assert!(observed.get());
        assert_eq!(
            read_projection_primary(&path),
            primary,
            "an intentional completed removal must restore the valid primary"
        );
        assert!(!path.join(super::LOCAL_CONFIG_NAME).exists());
        store.verify().unwrap();
    }

    #[test]
    fn runner_project_projection_failure_after_guard_stays_unreadable_until_next_owned_repair() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("projection");
        let mut store =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        let (primary, before) = projection_values("before");
        let (_, after) = projection_values("after");
        store.write_projection(&primary, Some(&before)).unwrap();
        let stable_config = store.config_path().to_path_buf();
        let previous_local = std::fs::read(path.join(super::LOCAL_CONFIG_NAME)).unwrap();
        let result = super::with_runner_projection_failpoint(
            super::RunnerProjectionCheckpoint::GuardInstalled,
            || store.write_projection(&primary, Some(&after)),
        );
        assert!(result.is_err());
        assert!(read_projection_primary(&path).is_null());
        assert_eq!(
            std::fs::read(path.join(super::LOCAL_CONFIG_NAME)).unwrap(),
            previous_local,
            "failure must not delete the old local layer"
        );
        store.verify().unwrap();
        store.write_projection(&primary, Some(&after)).unwrap();
        assert_eq!(store.config_path(), stable_config);
        assert_eq!(read_projection_primary(&path), primary);
        let repaired: serde_yaml::Value =
            serde_yaml::from_slice(&std::fs::read(path.join(super::LOCAL_CONFIG_NAME)).unwrap())
                .unwrap();
        assert_eq!(repaired, after);
        assert_eq!(
            repaired["infobases"]["origin"]["connection"],
            before["infobases"]["origin"]["connection"],
            "repair keeps the same source/infobase ownership recipe"
        );
    }

    #[test]
    fn runner_project_projection_error_in_local_gap_retains_guard_and_restores_old_owned_bytes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("projection");
        let mut store =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        let (primary, before) = projection_values("before");
        let (_, after) = projection_values("after");
        store.write_projection(&primary, Some(&before)).unwrap();
        let previous_local = std::fs::read(path.join(super::LOCAL_CONFIG_NAME)).unwrap();
        let result = super::with_runner_projection_failpoint(
            super::RunnerProjectionCheckpoint::LocalGap,
            || store.write_projection(&primary, Some(&after)),
        );
        assert!(result.is_err());
        assert!(read_projection_primary(&path).is_null());
        assert_eq!(
            std::fs::read(path.join(super::LOCAL_CONFIG_NAME)).unwrap(),
            previous_local
        );
        store.verify().unwrap();
        store.write_projection(&primary, Some(&after)).unwrap();
        assert_eq!(read_projection_primary(&path), primary);
    }

    #[test]
    fn runner_project_projection_error_after_local_confirmation_keeps_guard_until_repair() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("projection");
        let mut store =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        let (primary, before) = projection_values("before");
        let (_, after) = projection_values("after");
        store.write_projection(&primary, Some(&before)).unwrap();
        let result = super::with_runner_projection_failpoint(
            super::RunnerProjectionCheckpoint::LocalConfirmed,
            || store.write_projection(&primary, Some(&after)),
        );
        assert!(result.is_err());
        assert!(read_projection_primary(&path).is_null());
        assert_eq!(
            serde_yaml::from_slice::<serde_yaml::Value>(
                &std::fs::read(path.join(super::LOCAL_CONFIG_NAME)).unwrap()
            )
            .unwrap(),
            after
        );
        store.verify().unwrap();
        store.write_projection(&primary, Some(&after)).unwrap();
        assert_eq!(read_projection_primary(&path), primary);
    }

    #[test]
    fn runner_project_projection_preserves_a_foreign_primary_swapped_after_guard_installation() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("projection");
        let mut store =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        let (primary, before) = projection_values("before");
        let (_, after) = projection_values("after");
        store.write_projection(&primary, Some(&before)).unwrap();
        let old_local = std::fs::read(path.join(super::LOCAL_CONFIG_NAME)).unwrap();
        let foreign = b"workPath: foreign-project-work\n";
        let result = super::with_runner_projection_hook(
            super::RunnerProjectionCheckpoint::GuardInstalled,
            move |path| {
                std::fs::rename(
                    path.join(super::EFFECTIVE_CONFIG_NAME),
                    path.join("owned-guard-retained.yaml"),
                )
                .unwrap();
                std::fs::write(path.join(super::EFFECTIVE_CONFIG_NAME), foreign).unwrap();
            },
            || store.write_projection(&primary, Some(&after)),
        );
        assert!(result.is_err());
        assert_eq!(
            std::fs::read(path.join(super::EFFECTIVE_CONFIG_NAME)).unwrap(),
            foreign
        );
        assert!(serde_yaml::from_slice::<serde_yaml::Value>(
            &std::fs::read(path.join("owned-guard-retained.yaml")).unwrap()
        )
        .unwrap()
        .is_null());
        assert_eq!(
            std::fs::read(path.join(super::LOCAL_CONFIG_NAME)).unwrap(),
            old_local
        );
    }

    #[test]
    fn runner_project_projection_foreign_local_in_the_gap_is_preserved_with_the_null_guard() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("projection");
        let mut store =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        let (primary, before) = projection_values("before");
        let (_, after) = projection_values("after");
        store.write_projection(&primary, Some(&before)).unwrap();
        let previous_local = std::fs::read(path.join(super::LOCAL_CONFIG_NAME)).unwrap();
        let foreign = b"infobases: {foreign: {connection: 'File=/foreign/base'}}\n";
        let result = super::with_runner_projection_hook(
            super::RunnerProjectionCheckpoint::LocalGap,
            move |path| {
                assert!(read_projection_primary(path).is_null());
                std::fs::write(path.join(super::LOCAL_CONFIG_NAME), foreign).unwrap();
            },
            || store.write_projection(&primary, Some(&after)),
        );
        assert!(result.is_err());
        assert!(
            read_projection_primary(&path).is_null(),
            "a local refusal must not reinstate a valid primary"
        );
        assert_eq!(
            std::fs::read(path.join(super::LOCAL_CONFIG_NAME)).unwrap(),
            foreign
        );
        assert!(std::fs::read_dir(&path).unwrap().flatten().any(|entry| {
            entry.file_name().to_string_lossy().starts_with(".config-recovery-")
                && std::fs::read(entry.path()).is_ok_and(|bytes| bytes == previous_local)
        }), "the old owned local layer must remain in recovery when a foreign file blocks restoration");
    }

    #[test]
    fn runner_project_projection_interrupted_local_removal_is_repaired_at_the_same_owner_path() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("projection");
        let mut store =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        let (primary, before) = projection_values("before");
        let (_, after) = projection_values("after");
        store.write_projection(&primary, Some(&before)).unwrap();
        let stable_config = store.config_path().to_path_buf();
        let result = super::with_runner_projection_failpoint(
            super::RunnerProjectionCheckpoint::LocalGap,
            || store.write_projection(&primary, None),
        );
        assert!(result.is_err());
        assert!(read_projection_primary(&path).is_null());
        assert!(!path.join(super::LOCAL_CONFIG_NAME).exists());
        drop(store);
        let mut reopened =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        reopened.write_projection(&primary, Some(&after)).unwrap();
        assert_eq!(reopened.config_path(), stable_config);
        assert_eq!(read_projection_primary(&path), primary);
        assert_eq!(
            serde_yaml::from_slice::<serde_yaml::Value>(
                &std::fs::read(path.join(super::LOCAL_CONFIG_NAME)).unwrap()
            )
            .unwrap(),
            after
        );
    }

    #[test]
    fn runner_project_projection_leaves_an_unchanged_local_file_at_its_retained_identity() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("projection");
        let mut store =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        let (primary, local) = projection_values("unchanged");
        store.write_projection(&primary, Some(&local)).unwrap();
        let local_path = path.join(super::LOCAL_CONFIG_NAME);
        let identity = super::file_identity(&std::fs::File::open(&local_path).unwrap()).unwrap();
        super::with_runner_projection_hook(
            super::RunnerProjectionCheckpoint::GuardInstalled,
            |_| panic!("unchanged local must not install an update guard"),
            || store.write_projection(&primary, Some(&local)),
        )
        .unwrap();
        assert_eq!(
            super::file_identity(&std::fs::File::open(&local_path).unwrap()).unwrap(),
            identity
        );
        assert_eq!(read_projection_primary(&path), primary);
    }

    #[test]
    fn runner_project_store_keeps_a_replaced_config_and_directory_untouched() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("runner-project");
        let mut store =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        let value: serde_yaml::Value =
            serde_yaml::from_str("infobases: {origin: {password: secret}}\n").unwrap();
        store
            .write_config(super::LOCAL_CONFIG_NAME, &value)
            .unwrap();
        let local = path.join(super::LOCAL_CONFIG_NAME);
        std::fs::rename(&local, path.join("original-local.yaml")).unwrap();
        std::fs::write(&local, b"foreign replacement").unwrap();
        assert!(store.remove_config(super::LOCAL_CONFIG_NAME).is_err());
        assert_eq!(std::fs::read(&local).unwrap(), b"foreign replacement");
        let displaced = root.path().join("displaced-project");
        std::fs::rename(&path, &displaced).unwrap();
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("foreign.txt"), b"foreign directory").unwrap();
        assert!(store.remove_projection().is_err());
        assert_eq!(
            std::fs::read(path.join("foreign.txt")).unwrap(),
            b"foreign directory"
        );
        assert_eq!(
            std::fs::read(displaced.join("original-local.yaml")).unwrap(),
            serde_yaml::to_string(&value).unwrap().as_bytes()
        );
    }

    #[test]
    fn runner_project_store_does_not_overwrite_a_config_swapped_at_publication() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("runner-project");
        let mut store =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        let before = serde_yaml::Value::String("original secret".into());
        store
            .write_config(super::LOCAL_CONFIG_NAME, &before)
            .unwrap();
        let local = path.join(super::LOCAL_CONFIG_NAME);
        let displaced = path.join("displaced-local.yaml");
        let hook_local = local.clone();
        let hook_displaced = displaced.clone();
        crate::infrastructure::platform::filesystem::set_before_identity_bound_no_replace_rename_hook(move || {
            std::fs::rename(&hook_local, &hook_displaced).unwrap();
            std::fs::write(&hook_local, b"foreign publication boundary").unwrap();
        });
        let result = store.write_config(
            super::LOCAL_CONFIG_NAME,
            &serde_yaml::Value::String("new secret".into()),
        );
        assert!(result.is_err(), "raced config publication must be refused");
        assert_eq!(
            std::fs::read(local).unwrap(),
            b"foreign publication boundary"
        );
        assert_eq!(
            std::fs::read(displaced).unwrap(),
            serde_yaml::to_string(&before).unwrap().as_bytes()
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_runner_project_layers_are_owner_only_before_secret_bytes_are_written() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("runner-project");
        let mut store =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        for name in [super::EFFECTIVE_CONFIG_NAME, super::LOCAL_CONFIG_NAME] {
            let observed = std::rc::Rc::new(std::cell::Cell::new(false));
            let hook_observed = observed.clone();
            super::with_effective_config_create_hook(
                move |path| {
                    let file = std::fs::File::open(path).unwrap();
                    assert_eq!(file.metadata().unwrap().len(), 0);
                    crate::infrastructure::platform::filesystem::verify_owner_only_acl(&file)
                        .unwrap();
                    let directory =
                        crate::infrastructure::platform::filesystem::open_directory_nofollow(
                            path.parent().unwrap(),
                        )
                        .unwrap();
                    crate::infrastructure::platform::filesystem::verify_owner_only_acl(&directory)
                        .unwrap();
                    hook_observed.set(true);
                },
                || {
                    store
                        .write_config(
                            name,
                            &serde_yaml::Value::String("private-test-secret".into()),
                        )
                        .unwrap()
                },
            );
            assert!(observed.get());
            let file = std::fs::File::open(path.join(name)).unwrap();
            crate::infrastructure::platform::filesystem::verify_owner_only_acl(&file).unwrap();
        }
    }

    #[test]
    fn runner_project_store_lock_survives_removal_and_does_not_lock_other_workspaces() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("workspace-a");
        let mut owner =
            super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
        owner
            .write_config(
                super::EFFECTIVE_CONFIG_NAME,
                &serde_yaml::Value::Mapping(Default::default()),
            )
            .unwrap();
        owner.remove_projection().unwrap();
        assert!(!path.exists());

        let cancellation = CancellationToken::new();
        let contender_cancellation = cancellation.clone();
        let contender_path = path.clone();
        let (contended_tx, contended_rx) = std::sync::mpsc::channel();
        let contender = std::thread::spawn(move || {
            super::TEST_PROJECT_LOCK_CONTENDED.with(|slot| {
                *slot.borrow_mut() = Some(Box::new(move || {
                    contended_tx.send(()).unwrap();
                }))
            });
            super::RunnerProjectStore::open_and_lock(&contender_path, &contender_cancellation).err()
        });
        contended_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("same projection must retain the original lock after removal");
        cancellation.cancel();
        assert!(matches!(
            contender.join().unwrap().unwrap(),
            super::RunnerProjectLockFailure::Cancelled
        ));

        let independent_cancellation = CancellationToken::new();
        let child_cancellation = independent_cancellation.clone();
        let sibling = root.path().join("workspace-b");
        let (independent_tx, independent_rx) = std::sync::mpsc::channel();
        let independent = std::thread::spawn(move || {
            let acquired = super::RunnerProjectStore::open_and_lock(&sibling, &child_cancellation);
            independent_tx.send(acquired.is_ok()).unwrap();
        });
        let acquired = independent_rx.recv_timeout(std::time::Duration::from_secs(5));
        independent_cancellation.cancel();
        independent.join().unwrap();
        assert!(
            acquired.unwrap(),
            "another workspace must have independent ownership"
        );
        drop(owner);
        super::RunnerProjectStore::open_and_lock(&path, &CancellationToken::new()).unwrap();
    }

    #[test]
    fn installed_tree_interior_mutation_after_first_probe_is_quarantined_and_rolled_back() {
        let (root, context, _) = workspace("installed-tree-interior-mutation");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let platform = fixed_platform(&root);
        let runner = DumpRunner::valid();
        let hook_target = target.clone();

        let result = with_publication_hook(
            PublicationCheckpoint::AfterStageInstall,
            move || {
                std::fs::write(hook_target.join("unvalidated.txt"), b"unvalidated").unwrap();
            },
            || invoke(&runner, &platform, FullDumpInvocation::BuildDump, &context),
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(
            std::fs::read(target.join("before.txt")).unwrap(),
            b"before",
            "the original target must be restored after mutation of our installed directory"
        );
        assert!(
            !target.join("unvalidated.txt").exists(),
            "our installed but mutated directory must be quarantined before lock release"
        );
        assert!(
            result.errors.join("\n").contains("published dump differs")
                || result.errors.join("\n").contains("quarantine"),
            "{result:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_preparation_failure_reports_retained_projection_and_private_data() {
        let (root, context, config) = workspace("canonical-prepare-retention");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let public_before = super::capture_tree_target_nofollow(&target).unwrap();
        let runner = DumpRunner::valid();
        let cancellation = CancellationToken::new();
        let store =
            super::RunnerProjectStore::open_and_lock(&root.join("projection"), &cancellation)
                .unwrap();
        let retained = std::rc::Rc::new(std::cell::RefCell::new(None));
        let hook_retained = std::rc::Rc::clone(&retained);
        let hook_workspace = context.cwd.clone();
        let result = super::with_runner_projection_hook(
            super::RunnerProjectionCheckpoint::LocalConfirmed,
            move |store_path| {
                let projection = store_path.join(super::EFFECTIVE_CONFIG_NAME);
                std::fs::write(&projection, b"foreign projection sentinel").unwrap();
                let projection_identity =
                    super::file_identity(&std::fs::File::open(&projection).unwrap()).unwrap();
                let mut private_roots = std::fs::read_dir(&hook_workspace)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .filter(|path| {
                        path.file_name().is_some_and(|name| {
                            name.to_string_lossy().starts_with(".unica-dump-guard-")
                        })
                    })
                    .collect::<Vec<_>>();
                assert_eq!(private_roots.len(), 1);
                let private = private_roots.remove(0);
                let foreign = private.join("execution").join("foreign-source");
                std::fs::create_dir(&foreign).unwrap();
                std::fs::write(foreign.join("sentinel.txt"), b"foreign source sentinel").unwrap();
                let snapshot = super::capture_tree_target_nofollow(&private).unwrap();
                *hook_retained.borrow_mut() =
                    Some((private, snapshot, projection, projection_identity));
            },
            || {
                super::VerifiedFullDumpPublication::prepare(
                    &context,
                    super::FullDumpSelection {
                        config_path: &config,
                        source_set: Some("main"),
                        extension: None,
                        expected_target: None,
                    },
                    store,
                    &runner,
                    &cancellation,
                    |mut primary, local, _| {
                        if let Some(local) = local {
                            super::merge_yaml_values(&mut primary, local);
                        }
                        Ok(primary)
                    },
                    |effective, _| Ok((effective, None)),
                )
            },
        );
        let failure = match result {
            Ok(publication) => {
                drop(publication);
                panic!("a foreign projection must refuse canonical preparation");
            }
            Err(failure) => failure,
        };
        assert!(matches!(failure.phase, super::FullDumpPhase::Preparation));
        assert!(failure
            .detail
            .contains("local config overlay appeared during full dump"));
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            super::capture_tree_target_nofollow(&target).unwrap(),
            public_before
        );
        assert!(
            !failure.publication_uncertain,
            "private preparation retention must not claim an unknown public publication"
        );
        let retained = retained.borrow();
        let (private, snapshot, projection, projection_identity) =
            retained.as_ref().expect("projection hook must run");
        assert_eq!(
            super::capture_tree_target_nofollow(private).unwrap(),
            *snapshot,
            "preparation cleanup must preserve all private identities and bytes"
        );
        assert_eq!(
            std::fs::read(private.join("execution/foreign-source/sentinel.txt")).unwrap(),
            b"foreign source sentinel"
        );
        assert_eq!(
            super::file_identity(&std::fs::File::open(projection).unwrap()).unwrap(),
            *projection_identity,
            "the rejected foreign projection must retain its identity"
        );
        assert_eq!(
            std::fs::read(projection).unwrap(),
            b"foreign projection sentinel"
        );
        let recovery = failure
            .recovery
            .as_ref()
            .expect("retained preparation location must remain identity-proven")
            .as_ref()
            .expect("retained canonical preparation data needs a recovery location");
        assert_eq!(
            std::fs::canonicalize(recovery).unwrap(),
            std::fs::canonicalize(private).unwrap()
        );
        let expected_identity = match snapshot {
            super::TreeSnapshot::Directory { identity, .. } => *identity,
            super::TreeSnapshot::Absent => panic!("the captured private root must be present"),
        };
        assert_eq!(
            super::file_identity(
                &super::open_directory_nofollow(&std::fs::canonicalize(recovery).unwrap()).unwrap(),
            )
            .unwrap(),
            expected_identity,
            "recovery must identify the actual retained private owner"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    fn verify_canonical_ready_owned_output_cleanup(platform_changed: bool) {
        let (root, context, config) = workspace("canonical-ready-owned-output");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let target_before = TreeSnapshot::capture_target(&target).unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        publication.begin_execution();
        let stage = publication.staged_target().to_path_buf();
        let private_root = publication.prepared.private.root.clone();
        // The test owns this synchronous writer. Do not use fill_canonical_stage:
        // that helper also changes the private cleanup ledger and masks the gap.
        std::fs::write(
            stage.join("Configuration.xml"),
            if platform_changed {
                valid_configuration_owner()
            } else {
                "<invalid-owned-xml>".to_string()
            },
        )
        .unwrap();
        std::fs::write(stage.join("payload.bsl"), b"owned completed writer bytes").unwrap();
        if platform_changed {
            std::fs::write(&platform.executable, b"changed after Selected").unwrap();
        }
        let failure = publication.publish(&cancellation).unwrap_err();
        assert!(
            matches!(
                failure.phase,
                super::FullDumpPhase::Integrity if platform_changed
            ) || matches!(failure.phase, super::FullDumpPhase::Validation) && !platform_changed
        );
        assert!(publication.prepared.private.writer_completion_pending);
        assert!(!cancellation.protected_process_started());
        assert_eq!(
            TreeSnapshot::capture_target(&target).unwrap(),
            target_before
        );
        // This test's writer returned synchronously above. This explicit proof is
        // confined to the guard test; ProcessStreamOutput does not supply it.
        publication.confirm_execution_finished();
        let finish = publication.finish();
        assert!(finish.is_ok(), "known-finished owned output must clean");
        let finish = finish.unwrap();
        assert!(finish.warnings.is_empty());
        assert!(finish.retained.unwrap().is_none());
        assert!(!private_root.exists());
        assert_eq!(
            TreeSnapshot::capture_target(&target).unwrap(),
            target_before
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_ready_invalid_owned_xml_cleans_after_test_owned_writer_finishes() {
        verify_canonical_ready_owned_output_cleanup(false);
    }

    #[test]
    fn canonical_ready_platform_recheck_refusal_cleans_after_test_owned_writer_finishes() {
        verify_canonical_ready_owned_output_cleanup(true);
    }

    fn write_canonical_ready_output(publication: &super::VerifiedFullDumpPublication) {
        std::fs::write(
            publication.staged_target().join("Configuration.xml"),
            valid_configuration_owner(),
        )
        .unwrap();
        std::fs::write(
            publication.staged_target().join("payload.bsl"),
            b"owned completed writer bytes",
        )
        .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn canonical_ready_root_replacement_after_capture_remains_unowned() {
        let (root, context, config) = workspace("canonical-ready-root-replacement");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let target_before = TreeSnapshot::capture_target(&target).unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        publication.begin_execution();
        write_canonical_ready_output(&publication);
        let stage = publication.staged_target().to_path_buf();
        let displaced = root.join("displaced-owned-ready-stage");
        let owned_before = TreeSnapshot::capture_target(&stage).unwrap();
        let hook_stage = stage.clone();
        let hook_displaced = displaced.clone();
        let result = with_publication_hook(
            PublicationCheckpoint::AfterStagedCapture,
            move || {
                std::fs::rename(&hook_stage, &hook_displaced).unwrap();
                std::fs::create_dir(&hook_stage).unwrap();
                std::fs::write(hook_stage.join("foreign.txt"), b"foreign after Ready").unwrap();
            },
            || publication.publish(&cancellation),
        );
        assert!(result.is_err(), "a replaced Ready root cannot publish");
        assert!(matches!(
            result.unwrap_err().phase,
            super::FullDumpPhase::Integrity
        ));
        let foreign_before = TreeSnapshot::capture_target(&stage).unwrap();
        assert!(publication.prepared.private.writer_completion_pending);
        assert!(!cancellation.protected_process_started());
        publication.confirm_execution_finished();
        let finish = publication.finish();
        assert!(finish.is_err());
        let failure = match finish {
            Err(failure) => failure,
            Ok(_) => unreachable!(),
        };
        assert!(matches!(failure.phase, super::FullDumpPhase::Integrity));
        assert_eq!(
            TreeSnapshot::capture_target(&stage).unwrap(),
            foreign_before
        );
        assert_eq!(
            TreeSnapshot::capture_target(&displaced).unwrap(),
            owned_before
        );
        assert_eq!(
            TreeSnapshot::capture_target(&target).unwrap(),
            target_before
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_ready_same_inode_mutation_after_capture_is_not_recaptured() {
        let (root, context, config) = workspace("canonical-ready-content-mutation");
        let target = context.cwd.join("src");
        seed_valid_target(&target);
        let target_before = TreeSnapshot::capture_target(&target).unwrap();
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        publication.begin_execution();
        write_canonical_ready_output(&publication);
        let stage = publication.staged_target().to_path_buf();
        let payload = stage.join("payload.bsl");
        let payload_before = super::file_identity(&std::fs::File::open(&payload).unwrap()).unwrap();
        let hook_payload = payload.clone();
        let result = with_publication_hook(
            PublicationCheckpoint::AfterStagedCapture,
            move || std::fs::write(&hook_payload, b"late same-inode replacement bytes").unwrap(),
            || publication.publish(&cancellation),
        );
        assert!(result.is_err(), "Ready bytes cannot be silently refreshed");
        assert!(matches!(
            result.unwrap_err().phase,
            super::FullDumpPhase::Publication
        ));
        assert_eq!(
            super::file_identity(&std::fs::File::open(&payload).unwrap()).unwrap(),
            payload_before
        );
        assert!(publication.prepared.private.writer_completion_pending);
        assert!(!cancellation.protected_process_started());
        publication.confirm_execution_finished();
        let finished = publication.finish().unwrap();
        assert!(matches!(&finished.retained, Ok(None)));
        assert!(finished.warnings.is_empty());
        assert_eq!(
            std::fs::symlink_metadata(&stage).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        assert_eq!(
            TreeSnapshot::capture_target(&target).unwrap(),
            target_before
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_published_repeat_never_adopts_a_new_stage_name() {
        let (root, context, config) = workspace("canonical-published-foreign-stage");
        let runner = DumpRunner::valid();
        let platform = fixed_platform(&root);
        let mut publication =
            canonical_publication(&context, &config, &root.join("projection"), &runner);
        let cancellation = CancellationToken::new();
        attest_canonical(&mut publication, &platform, &runner, &cancellation);
        publication.prepare_dispatch_parent(&cancellation).unwrap();
        publication.begin_execution();
        write_canonical_ready_output(&publication);
        let stage = publication.staged_target().to_path_buf();
        let published = publication.publish(&cancellation).unwrap();
        assert_eq!(published.files, 2);
        assert!(publication.prepared.private.writer_completion_pending);
        let target_before = TreeSnapshot::capture_target(publication.target()).unwrap();
        std::fs::create_dir(&stage).unwrap();
        std::fs::write(stage.join("foreign.txt"), b"foreign after publication").unwrap();
        let foreign_before = TreeSnapshot::capture_target(&stage).unwrap();
        assert_eq!(publication.publish(&cancellation).unwrap().files, 2);
        assert_eq!(
            TreeSnapshot::capture_target(publication.target()).unwrap(),
            target_before
        );
        publication.confirm_execution_finished();
        let finish = publication.finish().unwrap();
        assert!(!finish.warnings.is_empty());
        assert_eq!(
            TreeSnapshot::capture_target(&stage).unwrap(),
            foreign_before
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
