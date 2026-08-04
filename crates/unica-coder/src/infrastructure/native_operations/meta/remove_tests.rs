use super::remove::{
    meta_remove_should_skip_file, metadata_files_recursive_bounded,
    metadata_files_recursive_with_limits, plan_meta_remove_subsystem_replacements,
    plan_meta_remove_subsystem_replacements_bounded, MetaRemoveTraversalLimits,
};
use super::{
    force_meta_remove_reparse_path, with_before_meta_remove_subsystem_child_inspection_hook,
    with_meta_remove_forced_reparse_paths,
};
use crate::domain::workspace::WorkspaceContext;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_context(name: &str) -> WorkspaceContext {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("unica-meta-remove-{name}-{nanos}"));
    fs::create_dir_all(&root).unwrap();
    WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".build").join("unica"),
        workspace_epoch: 1,
    }
}

#[test]
fn reference_scan_entry_budget_stops_incrementally_at_a_test_limit() {
    let context = temp_context("reference-entry-budget");
    let root = context.cwd.join("scan");
    fs::create_dir(&root).unwrap();
    for name in ["A.xml", "B.xml", "C.xml"] {
        fs::write(root.join(name), b"<Root/>").unwrap();
    }
    let mut visited_directories = std::collections::HashSet::new();
    let mut visited_entries = 0usize;

    let result = metadata_files_recursive_bounded(
        &root,
        0,
        MetaRemoveTraversalLimits {
            max_depth: 4,
            max_entries: 1,
        },
        &mut visited_directories,
        &mut visited_entries,
    );
    let error = match result {
        Ok(_) => panic!("the injected one-entry budget must stop the scan"),
        Err(error) => error,
    };

    assert!(error.contains("maximum of 1 entries"), "{error}");
    assert_eq!(
        visited_entries, 1,
        "the scanner must stop before retaining or inspecting entries beyond the budget"
    );
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn reference_scan_skips_only_the_exact_subsystems_component() {
    let root = PathBuf::from("/workspace/src");
    let object_xml = root.join("Catalogs/Victim.xml");
    let object_dir = root.join("Catalogs/Victim");

    assert!(meta_remove_should_skip_file(
        &root.join("Subsystems/Main.xml"),
        &root,
        &object_xml,
        &object_dir,
        true,
        true,
    ));
    assert!(!meta_remove_should_skip_file(
        &root.join("SubsystemsArchive/Reference.xml"),
        &root,
        &object_xml,
        &object_dir,
        true,
        true,
    ));
    assert!(!meta_remove_should_skip_file(
        &root.join("SubsystemsNotes.xml"),
        &root,
        &object_xml,
        &object_dir,
        true,
        true,
    ));
}

#[test]
fn reference_scan_depth_budget_stops_before_recursive_descent() {
    let context = temp_context("reference-depth-budget");
    let root = context.cwd.join("scan");
    fs::create_dir_all(root.join("Level1/Level2")).unwrap();
    fs::write(root.join("Level1/Level2/deep.xml"), b"<Root/>").unwrap();

    let result = metadata_files_recursive_with_limits(
        &root,
        MetaRemoveTraversalLimits {
            max_depth: 1,
            max_entries: 8,
        },
    );
    let error = match result {
        Ok(_) => panic!("the injected depth budget must reject the deeper directory"),
        Err(error) => error,
    };

    assert!(error.contains("maximum depth of 1"), "{error}");
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn reference_scan_rejects_a_direct_symlink_before_any_recursive_descent() {
    let context = temp_context("reference-symlink-before-recursion");
    let root = context.cwd.join("scan");
    fs::create_dir_all(root.join("A-directory")).unwrap();
    crate::infrastructure::platform::filesystem::create_test_directory_link(
        &context.cwd.join("external"),
        &root.join("Z-symlink-directory"),
    )
    .unwrap();

    let result = metadata_files_recursive_with_limits(
        &root,
        MetaRemoveTraversalLimits {
            max_depth: 0,
            max_entries: 8,
        },
    );
    let error = match result {
        Ok(_) => {
            panic!("a direct symlink must fail before descending into the regular directory")
        }
        Err(error) => error,
    };

    assert!(error.contains("symbolic link or reparse point"), "{error}");
    assert!(
        !error.contains("maximum depth"),
        "direct symlink rejection must retain precedence over recursive depth failure: {error}"
    );
    let _ = fs::remove_dir_all(&context.cwd);
}

fn subsystem_descriptor_bytes(name: &str) -> Vec<u8> {
    format!(
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><Subsystem><Properties><Name>{name}</Name></Properties><ChildObjects><Content/></ChildObjects></Subsystem></MetaDataObject>"
        )
        .into_bytes()
}

fn plan_subsystem_replacements_for_test(root: &Path) -> Result<(), String> {
    let mut replacements = Vec::new();
    let mut descriptor_reads = Vec::new();
    plan_meta_remove_subsystem_replacements(
        root,
        "Catalog.Victim",
        &mut replacements,
        &mut descriptor_reads,
    )
}

fn plan_subsystem_replacements_with_limits_for_test(
    root: &Path,
    limits: MetaRemoveTraversalLimits,
) -> (Result<(), String>, usize) {
    let mut replacements = Vec::new();
    let mut descriptor_reads = Vec::new();
    let mut visited_directories = HashSet::new();
    let mut visited_entries = 0usize;
    let result = plan_meta_remove_subsystem_replacements_bounded(
        root,
        "Catalog.Victim",
        &mut replacements,
        &mut descriptor_reads,
        0,
        limits,
        &mut visited_directories,
        &mut visited_entries,
    );
    (result, visited_entries)
}

#[test]
fn subsystem_planner_rejects_forced_reparse_at_every_inspection_point() {
    for point in ["root", "entry", "child"] {
        let context = temp_context(&format!("subsystem-reparse-{point}"));
        let root = context.cwd.join("Subsystems");
        let descriptor = root.join("Parent.xml");
        let child = root.join("Parent/Subsystems");
        fs::create_dir_all(&child).unwrap();
        fs::write(&descriptor, subsystem_descriptor_bytes("Parent")).unwrap();
        let forced = match point {
            "root" => root.clone(),
            "entry" => descriptor.clone(),
            "child" => child.clone(),
            _ => unreachable!(),
        };

        let error = with_meta_remove_forced_reparse_paths([forced], || {
            plan_subsystem_replacements_for_test(&root)
        })
        .expect_err("every subsystem planner inspection point must fail closed on reparse");

        assert!(
            error.contains("symbolic link or reparse point"),
            "{point}: {error}"
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }
}

#[test]
fn subsystem_planner_rejects_reparse_injected_at_child_inspection_window() {
    use std::cell::Cell;
    use std::rc::Rc;

    let context = temp_context("subsystem-reparse-race");
    let root = context.cwd.join("Subsystems");
    let descriptor = root.join("Parent.xml");
    let child = root.join("Parent/Subsystems");
    fs::create_dir_all(&child).unwrap();
    fs::write(&descriptor, subsystem_descriptor_bytes("Parent")).unwrap();
    let expected_child = child.clone();
    let injected = Rc::new(Cell::new(false));
    let injected_for_hook = Rc::clone(&injected);

    let error = with_meta_remove_forced_reparse_paths(Vec::new(), || {
        with_before_meta_remove_subsystem_child_inspection_hook(
            move |inspected| {
                assert_eq!(inspected, expected_child);
                injected_for_hook.set(true);
                force_meta_remove_reparse_path(inspected.to_path_buf());
            },
            || plan_subsystem_replacements_for_test(&root),
        )
    })
    .expect_err("a reparse injected at the child inspection window must fail closed");

    assert!(
        injected.get(),
        "test hook must cover the child inspection window"
    );
    assert!(error.contains("symbolic link or reparse point"), "{error}");
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn subsystem_planner_rejects_descent_beyond_meta_remove_depth_budget() {
    let context = temp_context("subsystem-depth-budget");
    let mut directory = context.cwd.join("Subsystems");
    fs::create_dir_all(&directory).unwrap();
    for depth in 0..=1 {
        let name = format!("Nested{depth}");
        fs::write(
            directory.join(format!("{name}.xml")),
            subsystem_descriptor_bytes(&name),
        )
        .unwrap();
        directory = directory.join(name).join("Subsystems");
        fs::create_dir_all(&directory).unwrap();
    }

    let (result, _) = plan_subsystem_replacements_with_limits_for_test(
        &context.cwd.join("Subsystems"),
        MetaRemoveTraversalLimits {
            max_depth: 1,
            max_entries: 8,
        },
    );
    let error = result
        .expect_err("subsystem recursion beyond the meta.remove depth budget must fail closed");

    assert!(
        error.contains("subsystem traversal exceeded the maximum depth"),
        "{error}"
    );
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn subsystem_planner_stops_before_retaining_entries_beyond_meta_remove_budget() {
    let context = temp_context("subsystem-entry-budget");
    let root = context.cwd.join("Subsystems");
    fs::create_dir(&root).unwrap();
    for name in ["A.txt", "B.txt"] {
        fs::write(root.join(name), b"not a subsystem descriptor").unwrap();
    }

    let (result, visited_entries) = plan_subsystem_replacements_with_limits_for_test(
        &root,
        MetaRemoveTraversalLimits {
            max_depth: 4,
            max_entries: 1,
        },
    );
    let error = result
        .expect_err("subsystem traversal must stop before retaining entries beyond the budget");

    assert!(
        error.contains("subsystem traversal exceeded the maximum of 1 entries"),
        "{error}"
    );
    assert_eq!(
        visited_entries, 1,
        "the subsystem planner must stop before retaining or inspecting entries beyond the budget"
    );
    let _ = fs::remove_dir_all(&context.cwd);
}
