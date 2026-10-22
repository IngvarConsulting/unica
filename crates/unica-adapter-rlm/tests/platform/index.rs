use super::*;
#[cfg(test)]
use fs2::FileExt;
use std::cell::RefCell;
#[cfg(test)]
use std::collections::BTreeMap;
#[cfg(test)]
use std::ffi::OsString;
#[cfg(test)]
use std::fs::{File, OpenOptions};
#[cfg(test)]
use std::sync::Arc;
use unica_work_control::cancellation::CancellationToken;
#[cfg(test)]
use unica_workspace::paths::path_starts_with_host_root;
#[cfg(test)]
use unica_workspace::state_fs::observed_state_file_identity;
#[cfg(test)]
use unica_workspace::test_support as testing;

#[cfg(test)]
fn environment(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
    let entries: BTreeMap<String, OsString> = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), OsString::from(*value)))
        .collect();
    move |name: &str| entries.get(name).cloned()
}

fn index_command_env<'a>(command: &'a IndexCommand, name: &str) -> &'a std::ffi::OsStr {
    command
        .env
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_os_str())
        .unwrap_or_else(|| panic!("index command must carry {name}"))
}

#[cfg(test)]
fn default_source_root(context: &WorkspaceContext) -> PathBuf {
    context.workspace_root.join("src")
}

#[cfg(test)]
fn generation_db_path(context: &WorkspaceContext, relative: &str) -> PathBuf {
    rlm_generation_root(context, &default_source_root(context))
        .unwrap()
        .join(relative)
}

#[cfg(test)]
fn status_path(context: &WorkspaceContext) -> PathBuf {
    super::status_path(context, &default_source_root(context)).unwrap()
}

#[cfg(test)]
fn lock_path(context: &WorkspaceContext) -> PathBuf {
    super::lock_path(context, &default_source_root(context)).unwrap()
}

#[cfg(test)]
fn read_bsl_index_status(context: &WorkspaceContext) -> Option<BslIndexStatus> {
    super::read_bsl_index_status(context, &default_source_root(context)).unwrap()
}

#[cfg(test)]
fn write_status(context: &WorkspaceContext, status: BslIndexStatus) -> Result<(), String> {
    super::write_status(context, &default_source_root(context), status)
}

#[test]
fn neutral_provider_state_root_valid_override_wins() {
    let root = neutral_provider_state_root_with(environment(&[
        ("UNICA_PROVIDER_STATE_DIR", "/state/unica"),
        ("HOME", "/home/user"),
    ]));

    assert_eq!(root, Some(PathBuf::from("/state/unica")));
}

#[test]
fn neutral_provider_state_root_empty_override_falls_through_to_home() {
    let root = neutral_provider_state_root_with(environment(&[
        ("UNICA_PROVIDER_STATE_DIR", ""),
        ("HOME", "/home/user"),
    ]));

    assert_eq!(
        root,
        Some(
            PathBuf::from("/home/user")
                .join(".unica")
                .join("provider-state")
        )
    );
}

#[test]
fn neutral_provider_state_root_empty_home_falls_through_to_userprofile() {
    let root = neutral_provider_state_root_with(environment(&[
        ("HOME", ""),
        ("USERPROFILE", "C:/Users/user"),
    ]));

    assert_eq!(
        root,
        Some(
            PathBuf::from("C:/Users/user")
                .join(".unica")
                .join("provider-state")
        )
    );
}

#[test]
fn neutral_provider_state_root_all_empty_values_report_missing_root() {
    let mut context = test_context("empty-direct-runtime-environment");
    context.cache_root = context.workspace_root.join(".build/unica");
    let external_base = neutral_provider_state_root_with(environment(&[
        ("UNICA_PROVIDER_STATE_DIR", ""),
        ("HOME", ""),
        ("USERPROFILE", ""),
    ]));

    assert_eq!(external_base, None);
    let error =
        rlm_provider_state_root_with(&context, &context.workspace_root, external_base).unwrap_err();
    assert_eq!(
            error,
            "UNICA_PROVIDER_STATE_DIR, HOME, or USERPROFILE is required for RLM state outside sourceRoot"
        );
    cleanup(&context);
}

#[test]
fn safe_root_failure_never_falls_back_to_generation_markers_inside_sources() {
    let mut context = test_context("safe-root-no-source-fallback");
    context.cache_root = context.workspace_root.join(".build/unica");
    let source_root = context.workspace_root.clone();

    let error = rlm_provider_state_root_with(&context, &source_root, None).unwrap_err();

    assert!(error.contains("required for RLM state outside sourceRoot"));
    assert!(!source_root.join("rlm-bsl/index-v17/bsl_index.db").exists());
    assert!(!source_root
        .join("caches/rlm-bsl/index-v17/bsl_index_status.json")
        .exists());
    assert!(!source_root
        .join("locks/rlm-bsl/index-v17/bsl_index.lock")
        .exists());
    cleanup(&context);
}

#[test]
fn safe_root_failure_is_unavailable_at_the_start_boundary() {
    let mut context = test_context("safe-root-boundaries");
    let source_root = default_source_root(&context);
    fs::create_dir_all(&source_root).unwrap();
    let first = context.workspace_root.join("provider-state-cycle-a");
    let second = context.workspace_root.join("provider-state-cycle-b");
    let first_link = testing::create_dir_symlink_for_test(&second, &first)
        .expect("symlink fixtures exist on Unix and Windows");
    first_link.unwrap();
    testing::create_dir_symlink_for_test(&first, &second)
        .expect("the platform that created the first link must expose the second fixture")
        .unwrap();
    context.cache_root = first;
    let runner = RecordingIndexRunner::default();
    let service = WorkspaceIndexService::with_runner(&runner, &FIXTURE_ENVIRONMENT);

    let report = service.start_isolated_build(&context, &source_root, &CancellationToken::new());

    assert!(report.warnings.iter().any(|warning| {
        warning.starts_with("rlm index unavailable:")
            && warning.contains("failed to resolve existing path ancestor")
    }));
    assert!(runner.backgrounds.borrow().is_empty());
    assert!(!source_root.join("rlm-bsl/index-v17/bsl_index.db").exists());
    assert!(!source_root
        .join("caches/rlm-bsl/index-v17/bsl_index_status.json")
        .exists());
    assert!(!source_root
        .join("locks/rlm-bsl/index-v17/bsl_index.lock")
        .exists());
    testing::remove_dir_symlink_for_test(&second).unwrap();
    testing::remove_dir_symlink_for_test(&context.cache_root).unwrap();
    cleanup(&context);
}

#[test]
fn rlm_provider_state_scopes_the_existing_safe_cache_layout() {
    let context = test_context("safe-provider-root");
    let source_root = context.workspace_root.join("src");
    fs::create_dir_all(&source_root).unwrap();

    let actual = rlm_provider_state_root_with(
        &context,
        &source_root,
        Some(context.workspace_root.parent().unwrap().join("host-data")),
    )
    .unwrap();

    let safe_parent = normalize_path_identity(&context.cache_root.join("provider-state")).unwrap();
    assert!(actual.starts_with(&safe_parent));
    assert_ne!(
        actual,
        normalize_path_identity(&context.cache_root).unwrap()
    );
    let identity = actual.file_name().unwrap().to_string_lossy();
    let digest = identity.strip_prefix("rlm-").unwrap();
    assert_eq!(digest.len(), 64);
    assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
    cleanup(&context);
}

#[test]
fn rlm_provider_state_moves_outside_a_workspace_wide_source_root() {
    let mut context = test_context("unsafe-provider-root");
    context.cache_root = context.workspace_root.join(".build/unica");
    let external = context.workspace_root.parent().unwrap().join("host-data");

    let first =
        rlm_provider_state_root_with(&context, &context.workspace_root, Some(external.clone()))
            .unwrap();
    let second =
        rlm_provider_state_root_with(&context, &context.workspace_root, Some(external)).unwrap();

    assert_eq!(first, second);
    assert!(!path_starts_with_host_root(&first, &context.workspace_root));
    let identity = first.file_name().unwrap().to_string_lossy();
    let digest = identity.strip_prefix("rlm-").unwrap();
    assert_eq!(digest.len(), 64);
    assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(
        first.join(RLM_PRODUCT_DIR).join(RLM_INDEX_GENERATION),
        first.join("rlm-bsl/index-v17")
    );
    cleanup(&context);
}

#[test]
fn rlm_provider_state_separates_source_roots() {
    let context = test_context("separate-provider-roots");
    let external = context.workspace_root.parent().unwrap().join("host-data");
    let first_source = context.workspace_root.join("src/configuration");
    let second_source = context.workspace_root.join("src/extension");
    fs::create_dir_all(&first_source).unwrap();
    fs::create_dir_all(&second_source).unwrap();

    let first =
        rlm_provider_state_root_with(&context, &first_source, Some(external.clone())).unwrap();
    let second = rlm_provider_state_root_with(&context, &second_source, Some(external)).unwrap();

    assert_ne!(first, second);
    cleanup(&context);
}

#[test]
fn rlm_provider_state_identity_preserves_unix_path_case() {
    // INV-CACHE-PROVIDER-STATE-OUTSIDE-SOURCE: distinct normalized pairs
    // must not share persistent provider state on case-sensitive Unix filesystems.
    let fixture = test_context("provider-state-case-identity");
    let parent = fixture.workspace_root.parent().unwrap();
    let external = parent.join("host-data");
    let Some((upper_workspace, lower_workspace)) =
        testing::case_distinct_provider_paths_for_test(parent)
    else {
        cleanup(&fixture);
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::CaseSensitiveFilesystem,
            "case-distinct provider paths need a case-sensitive host",
        );
        return;
    };
    let context_for = |workspace_root: PathBuf| WorkspaceContext {
        cwd: workspace_root.clone(),
        cache_root: workspace_root.join(".build/unica"),
        workspace_root,
        workspace_epoch: 1,
    };
    let upper = rlm_provider_state_root_with(
        &context_for(upper_workspace.clone()),
        &upper_workspace,
        Some(external.clone()),
    )
    .unwrap();
    let lower = rlm_provider_state_root_with(
        &context_for(lower_workspace.clone()),
        &lower_workspace,
        Some(external),
    )
    .unwrap();

    assert_ne!(upper, lower);
    cleanup(&fixture);
}

#[test]
fn rlm_provider_state_identity_preserves_distinct_non_utf8_unix_paths() {
    // INV-CACHE-PROVIDER-STATE-OUTSIDE-SOURCE: distinct normalized pairs
    // must not share persistent provider state when their path text is not UTF-8.
    let fixture = test_context("provider-state-non-utf8-identity");
    let Some((first_workspace, second_workspace)) =
        testing::distinct_non_utf8_provider_paths_for_test(&fixture.workspace_root)
    else {
        cleanup(&fixture);
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::NonUtf8FileNames,
            "this host cannot hold two distinct non-UTF-8 paths",
        );
        return;
    };
    fs::create_dir_all(&first_workspace).unwrap();
    fs::create_dir_all(&second_workspace).unwrap();
    assert_ne!(
        fs::canonicalize(&first_workspace).unwrap(),
        fs::canonicalize(&second_workspace).unwrap()
    );
    let external = fixture.workspace_root.join("host-data");
    let context_for = |workspace_root: PathBuf| WorkspaceContext {
        cwd: workspace_root.clone(),
        cache_root: workspace_root.join(".build/unica"),
        workspace_root,
        workspace_epoch: 1,
    };
    let first = rlm_provider_state_root_with(
        &context_for(first_workspace.clone()),
        &first_workspace,
        Some(external.clone()),
    )
    .unwrap();
    let second = rlm_provider_state_root_with(
        &context_for(second_workspace.clone()),
        &second_workspace,
        Some(external),
    )
    .unwrap();

    assert_ne!(first, second);
    cleanup(&fixture);
}

pub fn rlm_coordination_contract(
    lease_for: impl Fn(
        unica_workspace::actor::WorkspaceIdentity,
        &WorkspaceContext,
        &str,
        &Path,
    ) -> unica_workspace::actor::ProviderSourceLease,
) {
    use unica_source_contract::{SourceFormat, SourceProfile, SourceSetKind};

    use unica_workspace::actor::{WorkspaceIdentity, WorkspaceSourceSetInput};

    let context = test_context("separate-coordination-roots");
    let first_source = context.workspace_root.join("src/configuration");
    let second_source = context.workspace_root.join("src/extension");
    fs::create_dir_all(&first_source).unwrap();
    fs::create_dir_all(&second_source).unwrap();

    let first_root = rlm_provider_state_root(&context, &first_source).unwrap();
    let second_root = rlm_provider_state_root(&context, &second_source).unwrap();
    let first_status = super::status_path(&context, &first_source).unwrap();
    let second_status = super::status_path(&context, &second_source).unwrap();
    let first_lock = super::lock_path(&context, &first_source).unwrap();
    let second_lock = super::lock_path(&context, &second_source).unwrap();

    assert_eq!(
        first_status,
        first_root.join("caches/rlm-bsl/index-v17/bsl_index_status.json")
    );
    assert_eq!(
        first_lock,
        first_root.join("locks/rlm-bsl/index-v17/bsl_index.lock")
    );
    assert_eq!(
        second_status,
        second_root.join("caches/rlm-bsl/index-v17/bsl_index_status.json")
    );
    assert_eq!(
        second_lock,
        second_root.join("locks/rlm-bsl/index-v17/bsl_index.lock")
    );
    assert_ne!(first_status, second_status);
    assert_ne!(first_lock, second_lock);

    let runner = RecordingIndexRunner::default();
    for (index, (name, profile)) in [
        ("main", "program"),
        ("renamed", "program"),
        ("main", "program-and-service"),
    ]
    .into_iter()
    .enumerate()
    {
        let identity = WorkspaceIdentity::new(
            &context,
            [WorkspaceSourceSetInput::new(
                name,
                &first_source,
                SourceSetKind::Configuration,
                SourceFormat::PlatformXml,
                SourceProfile::platform_xml_8_3_27_format_2_20(),
            )],
            profile,
        )
        .unwrap();
        let lease = lease_for(identity, &context, name, &first_source);
        let admitted_source = lease.source_root().to_path_buf();
        let service = WorkspaceIndexService::with_runner(&runner, &FIXTURE_ENVIRONMENT)
            .with_source_lease(lease);
        let report =
            service.start_isolated_build(&context, &admitted_source, &CancellationToken::new());

        assert_eq!(
            runner.backgrounds.borrow().len(),
            index + 1,
            "another actor scope blocked index start for {name}/{profile}: {report:?}"
        );
    }

    let backgrounds = runner.backgrounds.borrow();
    for (index, job) in backgrounds.iter().enumerate() {
        assert!(job.lock_path.is_file());
        assert!(!job.status_path.exists());
        for previous in &backgrounds[..index] {
            assert_ne!(job.lock_path, previous.lock_path);
            assert_ne!(job.status_path, previous.status_path);
            assert_ne!(
                index_command_env(&job.primary, "RLM_INDEX_DIR"),
                index_command_env(&previous.primary, "RLM_INDEX_DIR")
            );
        }
    }
    drop(backgrounds);
    drop(runner);
    cleanup(&context);
}

#[test]
fn worker_terminal_observation_tracks_real_failure_cancellation_and_publication() {
    for case in [
        "failure",
        "cancelled",
        "timeout",
        "completed",
        "publication-failed",
    ] {
        let context = test_context(&format!("maintenance-terminal-{case}"));
        let source_root = context.workspace_root.join("src");
        fs::create_dir_all(&source_root).unwrap();
        let runner = RecordingIndexRunner::default();
        let service = WorkspaceIndexService::with_runner(&runner, &FIXTURE_ENVIRONMENT);
        let report =
            service.start_isolated_build(&context, &source_root, &CancellationToken::new());
        assert_eq!(report.maintenance, IndexMaintenanceStatus::Queued);
        let job = runner.backgrounds.borrow_mut().pop().unwrap();
        let observation = Arc::clone(&job.lock_lease.observation);
        let status_path = job.status_path.clone();
        run_background_job_with(job, |command, _| {
            assert_eq!(
                observed_status(&observation),
                IndexMaintenanceStatus::Running
            );
            match case {
                "failure" => Err("controlled execution failure".to_string()),
                "cancelled" | "timeout" => {
                    let mut output = IndexOutput::success("");
                    output.cancelled = case == "cancelled";
                    output.timed_out = case == "timeout";
                    Ok(output)
                }
                _ if command.args.get(1).is_some_and(|arg| arg == "info") => {
                    let db = PathBuf::from(index_command_env(command, "RLM_INDEX_DIR"))
                        .join("bsl_index.db");
                    fs::create_dir_all(db.parent().unwrap()).unwrap();
                    fs::write(&db, b"usable candidate").unwrap();
                    if case == "publication-failed" {
                        fs::create_dir_all(&status_path).unwrap();
                    }
                    Ok(IndexOutput::success(format!(
                        "Index: {}\nStatus: fresh",
                        db.display()
                    )))
                }
                _ => Ok(IndexOutput::success("build complete")),
            }
        });
        let observed = observed_status(&observation);
        match case {
            "completed" => {
                assert_eq!(observed, IndexMaintenanceStatus::Completed);
                assert!(service
                    .active_build(&context, &source_root)
                    .unwrap()
                    .is_some());
            }
            "cancelled" => assert!(
                matches!(observed, IndexMaintenanceStatus::Cancelled(_)),
                "{observed:?}"
            ),
            _ => assert!(
                matches!(observed, IndexMaintenanceStatus::Failed(_)),
                "{case}: {observed:?}"
            ),
        }
        assert!(
            service.maintenance_status(&context, &source_root).is_none(),
            "a terminal job no longer advertises active work"
        );
        cleanup(&context);
    }
}

#[test]
fn registered_owner_observes_maintenance_while_its_file_is_exclusively_locked() {
    let context = test_context("maintenance-own-locked-observation");
    let source_root = context.workspace_root.join("src");
    fs::create_dir_all(&source_root).unwrap();
    let path = super::lock_path(&context, &source_root).unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let lease = acquire_index_lock(&path, "build", &source_root)
        .unwrap()
        .expect("own the actual index lock");
    lease
        .owner
        .lock()
        .unwrap()
        .file
        .seek(SeekFrom::Start(7))
        .unwrap();

    for state in [
        IndexMaintenanceStatus::Queued,
        IndexMaintenanceStatus::Running,
    ] {
        *lease.observation.lock().unwrap() = MaintenanceObservation::Observed(state.clone());
        assert!(lease.validate_ownership());
        assert_eq!(
            observed_maintenance(&path),
            Some(state),
            "the registered owner must observe its state without reopening locked bytes"
        );
        assert_eq!(
            lease.owner.lock().unwrap().file.stream_position().unwrap(),
            7
        );
    }
    drop(lease);
    assert!(observed_maintenance(&path).is_none());
    cleanup(&context);
}

#[test]
fn registered_owner_rejects_in_place_record_substitution_without_overwriting_it() {
    for case in ["foreign-id", "inactive", "wrong-schema", "invalid-json"] {
        let context = test_context(&format!("maintenance-in-place-{case}"));
        let source_root = context.workspace_root.join("src");
        fs::create_dir_all(&source_root).unwrap();
        let path = super::lock_path(&context, &source_root).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut lease = acquire_index_lock(&path, "build", &source_root)
            .unwrap()
            .unwrap();
        *lease.observation.lock().unwrap() =
            MaintenanceObservation::Observed(IndexMaintenanceStatus::Queued);
        let mut owner = lease.owner.lock().unwrap();
        let identity = state_file_identity(&owner.file).unwrap();
        let mut foreign = owner.lock.clone();
        match case {
            "foreign-id" => foreign.lock_id = "foreign-owner".to_string(),
            "inactive" => foreign.mark_released(),
            "wrong-schema" => foreign.schema_version += 1,
            _ => {}
        }
        let bytes = if case == "invalid-json" {
            b"broken-json".to_vec()
        } else {
            serde_json::to_vec(&foreign).unwrap()
        };
        owner.file.set_len(0).unwrap();
        owner.file.seek(SeekFrom::Start(0)).unwrap();
        owner.file.write_all(&bytes).unwrap();
        owner.file.flush().unwrap();
        assert_eq!(state_file_identity(&owner.file).unwrap(), identity);
        drop(owner);
        assert!(observed_maintenance(&path).is_none(), "{case}");
        assert!(!lease.validate_ownership(), "{case}");
        assert!(!lease.refresh(123), "{case}");
        drop(lease);
        assert_eq!(
            fs::read(&path).unwrap(),
            bytes,
            "{case}: stale owner wrote the lock"
        );
        cleanup(&context);
    }
}

#[test]
fn poisoned_owner_never_proves_maintenance_or_writes_a_release_record() {
    let context = test_context("maintenance-poisoned-owner");
    let source_root = context.workspace_root.join("src");
    fs::create_dir_all(&source_root).unwrap();
    let path = super::lock_path(&context, &source_root).unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut lease = acquire_index_lock(&path, "build", &source_root)
        .unwrap()
        .unwrap();
    let owner = Arc::clone(&lease.owner);
    assert!(std::panic::catch_unwind(move || {
        let _guard = owner.lock().unwrap();
        panic!("controlled owner mutex poison");
    })
    .is_err());
    assert!(observed_maintenance(&path).is_none());
    assert!(!lease.validate_ownership());
    assert!(!lease.refresh(123));
    drop(lease);
    assert!(
        read_lock_path(&path).unwrap().is_active(),
        "poisoned owner must not publish a release"
    );
    cleanup(&context);
}

#[test]
fn foreign_or_replaced_lock_never_proves_queued_or_running_maintenance() {
    use std::io::Read;

    let context = test_context("maintenance-ownership");
    let source_root = context.workspace_root.join("src");
    fs::create_dir_all(&source_root).unwrap();
    let runner = RecordingIndexRunner::default();
    let service = WorkspaceIndexService::with_runner(&runner, &FIXTURE_ENVIRONMENT);
    let path = super::lock_path(&context, &source_root).unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    write_lock_path(&path, BslIndexLock::new("build", &source_root)).unwrap();
    let before = fs::read(&path).unwrap();
    let mut foreign = File::options().read(true).write(true).open(&path).unwrap();
    let held_identity = observed_state_file_identity(&foreign).unwrap();
    foreign.lock_exclusive().unwrap();
    let report = service.start_isolated_build(&context, &source_root, &CancellationToken::new());
    assert_eq!(report.maintenance, IndexMaintenanceStatus::Unknown);
    assert!(runner.backgrounds.borrow().is_empty());
    assert_eq!(
        observed_state_file_identity(&File::open(&path).unwrap()).unwrap(),
        held_identity,
        "the foreign owner's named file must not be replaced"
    );
    // Windows enforces the byte-range lock against a newly opened reader,
    // including another handle in this process. Inspect through its owner
    // while keeping that lock held and restoring the owner's file cursor.
    let position = foreign.stream_position().unwrap();
    foreign.seek(SeekFrom::Start(0)).unwrap();
    let mut after = Vec::new();
    foreign.read_to_end(&mut after).unwrap();
    foreign.seek(SeekFrom::Start(position)).unwrap();
    assert_eq!(after, before);
    foreign.unlock().unwrap();
    drop(foreign);
    assert_eq!(
        service
            .start_isolated_build(&context, &source_root, &CancellationToken::new())
            .maintenance,
        IndexMaintenanceStatus::Queued
    );
    let job = runner.backgrounds.borrow_mut().pop().unwrap();
    let observation = Arc::clone(&job.lock_lease.observation);
    write_lock_path(&path, BslIndexLock::new("build", &source_root)).unwrap();
    let successor = fs::read(&path).unwrap();
    assert!(service.maintenance_status(&context, &source_root).is_none());
    let report = service.start_isolated_build(&context, &source_root, &CancellationToken::new());
    assert_eq!(report.maintenance, IndexMaintenanceStatus::Unknown);
    run_background_job_with(job, |_, _| panic!("a replaced owner cannot execute"));
    assert_eq!(
        observed_status(&observation),
        IndexMaintenanceStatus::Unknown
    );
    assert_eq!(fs::read(&path).unwrap(), successor);
    cleanup(&context);
}

#[test]
fn builder_17_uses_a_new_generation_and_leaves_older_generations_untouched() {
    let context = test_context("generation-cutover");
    let source_root = context.workspace_root.join("src");
    fs::create_dir_all(&source_root).unwrap();
    let state_root = rlm_provider_state_root(&context, &source_root).unwrap();
    let legacy = state_root.join("rlm-tools-bsl");
    fs::create_dir_all(&legacy).unwrap();
    fs::write(legacy.join("builder-14-sentinel"), "keep").unwrap();
    // RLM 1.42 writes builder-17 schema; a builder-15 index of RLM 1.33
    // stays where it was and is never handed to the new reader.
    let builder_15 = state_root.join("rlm-bsl/index-v15");
    fs::create_dir_all(&builder_15).unwrap();
    fs::write(builder_15.join("bsl_index.db"), "builder-15-db").unwrap();

    let commands =
        WorkspaceIndexService::with_runner(&RecordingIndexRunner::default(), &FIXTURE_ENVIRONMENT)
            .commands(&context, &source_root, &CancellationToken::new())
            .unwrap();
    let actual = PathBuf::from(index_command_env(&commands.info, "RLM_INDEX_DIR"));

    assert_eq!(actual, state_root.join("rlm-bsl/index-v17"));
    assert_eq!(
        fs::read_to_string(builder_15.join("bsl_index.db")).unwrap(),
        "builder-15-db"
    );
    assert_eq!(index_command_env(&commands.info, "PYTHONUTF8"), "1");
    assert_eq!(
        index_command_env(&commands.info, "PYTHONIOENCODING"),
        "utf-8:surrogateescape"
    );
    assert_eq!(
        fs::read_to_string(legacy.join("builder-14-sentinel")).unwrap(),
        "keep"
    );
    cleanup(&context);
}

/// Чтение активной сборки запускает уборку прежнего поколения той же пары:
/// давно не тронутое index-v15 исчезает, текущее поколение остаётся.
#[test]
fn active_build_collects_an_idle_previous_generation_but_not_the_current_one() {
    let context = test_context("generation-collection-wiring");
    let source_root = default_source_root(&context);
    fs::create_dir_all(&source_root).unwrap();
    let pair_root = rlm_provider_state_root(&context, &source_root).unwrap();
    let previous = [
        pair_root.join("rlm-bsl/index-v15"),
        pair_root.join("caches/rlm-bsl/index-v15"),
        pair_root.join("locks/rlm-bsl/index-v15"),
    ];
    for directory in &previous {
        fs::create_dir_all(directory).unwrap();
        fs::write(directory.join("payload"), b"builder-15").unwrap();
    }
    let current = pair_root.join("rlm-bsl").join(RLM_INDEX_GENERATION);
    fs::create_dir_all(&current).unwrap();
    fs::write(current.join("payload"), b"current").unwrap();
    let old = SystemTime::now() - Duration::from_secs(2 * 24 * 60 * 60);
    let aged = previous.iter().chain([&current]).try_for_each(|directory| {
        File::options()
            .write(true)
            .open(directory.join("payload"))
            .and_then(|payload| payload.set_modified(old))
            .expect("backdate the payload file");
        File::open(directory)?.set_modified(old)
    });
    if aged.is_err() {
        cleanup(&context);
        // Windows cannot open a directory this way to set its time: the
        // wiring is proven on Unix only.
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::DirectoryMtime,
            "this host cannot set a directory's modification time",
        );
        return;
    }

    let _ =
        WorkspaceIndexService::with_runner(&RecordingIndexRunner::default(), &FIXTURE_ENVIRONMENT)
            .active_build(&context, &source_root);
    let deadline = Instant::now() + Duration::from_secs(10);
    while previous.iter().any(|directory| directory.exists()) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }

    for directory in &previous {
        assert!(!directory.exists(), "{}", directory.display());
    }
    assert_eq!(fs::read(current.join("payload")).unwrap(), b"current");
    cleanup(&context);
}

#[test]
fn builder_generation_environment_preserves_native_path_bytes() {
    let mut context = test_context("native-generation-environment");
    let Some(invalid_component) = testing::non_utf8_relative_path_for_test() else {
        cleanup(&context);
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::NonUtf8FileNames,
            "this host has no file names outside UTF-8",
        );
        return;
    };
    context.cache_root = context.workspace_root.join(invalid_component);
    let source_root = default_source_root(&context);
    fs::create_dir_all(&source_root).unwrap();
    let expected_reader_environment = rlm_generation_root(&context, &source_root).unwrap();

    let commands =
        WorkspaceIndexService::with_runner(&RecordingIndexRunner::default(), &FIXTURE_ENVIRONMENT)
            .commands(&context, &source_root, &CancellationToken::new())
            .unwrap();

    assert_eq!(
        PathBuf::from(index_command_env(&commands.info, "RLM_INDEX_DIR")),
        expected_reader_environment
    );
    assert_eq!(index_command_env(&commands.info, "PYTHONUTF8"), "1");
    assert_eq!(
        index_command_env(&commands.info, "PYTHONIOENCODING"),
        "utf-8:surrogateescape"
    );
    assert_eq!(PathBuf::from(&commands.info.args[2]), source_root);
    cleanup(&context);
}

#[test]
fn builder_command_preserves_native_source_root_argument_bytes() {
    let context = test_context("native-source-argument");
    let Some(invalid_component) = testing::non_utf8_relative_path_for_test() else {
        cleanup(&context);
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::NonUtf8FileNames,
            "this host has no file names outside UTF-8",
        );
        return;
    };
    let source_root = context.workspace_root.join(invalid_component);

    let commands =
        WorkspaceIndexService::with_runner(&RecordingIndexRunner::default(), &FIXTURE_ENVIRONMENT)
            .commands(&context, &source_root, &CancellationToken::new())
            .unwrap();

    assert_eq!(PathBuf::from(&commands.info.args[2]), source_root);
    cleanup(&context);
}

#[test]
fn builder_15_rejects_a_generation_alias_to_builder_14_storage() {
    let context = test_context("generation-alias-to-builder-14");
    let source_root = default_source_root(&context);
    fs::create_dir_all(&source_root).unwrap();
    let pair_root = rlm_provider_state_root(&context, &source_root).unwrap();
    let legacy = pair_root.join("rlm-tools-bsl/index");
    fs::create_dir_all(&legacy).unwrap();
    let legacy_db = legacy.join("bsl_index.db");
    fs::write(&legacy_db, b"builder-14-db").unwrap();
    let alias = pair_root.join("rlm-bsl/index-v17");
    fs::create_dir_all(alias.parent().unwrap()).unwrap();
    let link = testing::create_dir_symlink_for_test(&legacy, &alias)
        .expect("symlink fixtures exist on Unix and Windows");
    link.unwrap();

    let error =
        WorkspaceIndexService::with_runner(&RecordingIndexRunner::default(), &FIXTURE_ENVIRONMENT)
            .commands(&context, &source_root, &CancellationToken::new())
            .unwrap_err();

    assert!(error.contains("symbolic link or reparse point"));
    assert_eq!(fs::read(&legacy_db).unwrap(), b"builder-14-db");
    assert!(!legacy.join(STATUS_FILE_NAME).exists());
    cleanup(&context);
}

#[test]
fn legacy_status_and_lock_do_not_gate_builder_15() {
    let context = test_context("legacy-markers");
    let source_root = context.workspace_root.join("src");
    fs::create_dir_all(&source_root).unwrap();
    fs::create_dir_all(context.cache_root.join("caches")).unwrap();
    fs::create_dir_all(context.cache_root.join("locks")).unwrap();
    fs::write(
        context.cache_root.join("caches/bsl_index_status.json"),
        "legacy",
    )
    .unwrap();
    fs::write(context.cache_root.join("locks/bsl_index.lock"), "legacy").unwrap();

    assert!(!super::status_path(&context, &source_root)
        .unwrap()
        .ends_with("caches/bsl_index_status.json"));
    assert!(!super::lock_path(&context, &source_root)
        .unwrap()
        .ends_with("locks/bsl_index.lock"));
    assert_eq!(
        fs::read_to_string(context.cache_root.join("caches/bsl_index_status.json")).unwrap(),
        "legacy"
    );
    assert_eq!(
        fs::read_to_string(context.cache_root.join("locks/bsl_index.lock")).unwrap(),
        "legacy"
    );
    cleanup(&context);
}

#[test]
fn current_pair_scoped_builder_14_bytes_are_ignored_and_preserved() {
    let context = test_context("pair-scoped-legacy-markers");
    let source_root = context.workspace_root.join("src");
    fs::create_dir_all(&source_root).unwrap();
    let pair_root = rlm_provider_state_root(&context, &source_root).unwrap();
    let legacy_db = pair_root.join("rlm-tools-bsl/index/bsl_index.db");
    let legacy_status = pair_root.join("caches/bsl_index_status.json");
    let legacy_lock = pair_root.join("locks/bsl_index.lock");
    for path in [&legacy_db, &legacy_status, &legacy_lock] {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
    }
    fs::write(&legacy_db, b"builder-14-db").unwrap();
    fs::write(&legacy_status, b"builder-14-status").unwrap();
    fs::write(&legacy_lock, b"builder-14-lock").unwrap();
    let before = [
        fs::read(&legacy_db).unwrap(),
        fs::read(&legacy_status).unwrap(),
        fs::read(&legacy_lock).unwrap(),
    ];
    let runner = RecordingIndexRunner::default();

    let report = WorkspaceIndexService::with_runner(&runner, &FIXTURE_ENVIRONMENT)
        .start_isolated_build(&context, &source_root, &CancellationToken::new());

    assert_eq!(report.maintenance, IndexMaintenanceStatus::Queued);
    assert_eq!(runner.backgrounds.borrow()[0].action, "build");
    let candidate = PathBuf::from(index_command_env(
        &runner.backgrounds.borrow()[0].primary,
        "RLM_INDEX_DIR",
    ));
    assert!(
        candidate.starts_with(
            rlm_generation_root(&context, &source_root)
                .unwrap()
                .join("builds")
        ),
        "{candidate:?}"
    );
    assert_eq!(fs::read(&legacy_db).unwrap(), before[0]);
    assert_eq!(fs::read(&legacy_status).unwrap(), before[1]);
    assert_eq!(fs::read(&legacy_lock).unwrap(), before[2]);
    cleanup(&context);
}

#[test]
fn active_build_rejects_db_outside_declared_build_directory() {
    enum InvalidDbLocation {
        Builder14,
        SiblingBuild,
    }

    for (name, location) in [
        ("builder-14", InvalidDbLocation::Builder14),
        ("sibling-build", InvalidDbLocation::SiblingBuild),
    ] {
        let context = test_context(&format!("active-build-{name}"));
        let source_root = default_source_root(&context);
        fs::create_dir_all(&source_root).unwrap();
        let build_id = "a".repeat(32);
        let db_path = match location {
            InvalidDbLocation::Builder14 => rlm_provider_state_root(&context, &source_root)
                .unwrap()
                .join("rlm-tools-bsl/index/bsl_index.db"),
            InvalidDbLocation::SiblingBuild => {
                rlm_build_root(&context, &source_root, &"b".repeat(32))
                    .unwrap()
                    .join("bsl_index.db")
            }
        };
        fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        fs::write(&db_path, name).unwrap();
        write_status(
            &context,
            BslIndexStatus::ready(&source_root, &db_path).with_build_id(build_id.clone()),
        )
        .unwrap();

        assert!(db_path.is_file(), "{name}: fixture DB must exist");
        assert_eq!(
            read_bsl_index_status(&context).unwrap().build_id.as_deref(),
            Some(build_id.as_str()),
            "{name}: marker must reach the build-directory check"
        );
        let active = WorkspaceIndexService::with_runner(
            &RecordingIndexRunner::default(),
            &FIXTURE_ENVIRONMENT,
        )
        .active_build(&context, &source_root)
        .unwrap();
        assert_eq!(active, None, "{name}: DB outside its build became active");
        cleanup(&context);
    }
}

#[test]
fn active_builder_15_lock_beats_ready_marker_for_cache_readiness() {
    let context = test_context("cache-readiness-active-lock");
    let source_root = context.workspace_root.join("src");
    fs::create_dir_all(&source_root).unwrap();
    let db_path = generation_db_path(&context, "ready/bsl_index.db");
    fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    fs::write(&db_path, b"builder-15-db").unwrap();
    write_status(&context, BslIndexStatus::ready(&source_root, &db_path)).unwrap();
    write_fresh_lock(&context, "update");

    assert!(!bsl_index_is_ready(&context, &FIXTURE_ENVIRONMENT));
    cleanup(&context);
}

#[test]
fn background_worker_never_publishes_ready_for_db_outside_builder_15_generation() {
    let context = test_context("worker-legacy-db-never-readable");
    let source_root = context.workspace_root.join("src");
    fs::create_dir_all(&source_root).unwrap();
    let legacy_db = rlm_provider_state_root(&context, &source_root)
        .unwrap()
        .join("rlm-tools-bsl/index/bsl_index.db");
    fs::create_dir_all(legacy_db.parent().unwrap()).unwrap();
    fs::write(&legacy_db, b"builder-14-db").unwrap();
    let job = test_background_job(&context, "build");

    run_background_job_with(job, |command, _lease| {
        if command.args.get(1).is_some_and(|arg| arg == "info") {
            Ok(IndexOutput::success(format!(
                "Index: {}\n  Status:   fresh\n",
                legacy_db.display()
            )))
        } else {
            Ok(IndexOutput::success("Index built"))
        }
    });

    assert_ne!(read_bsl_index_status(&context).unwrap().status, "ready");
    assert_eq!(fs::read(&legacy_db).unwrap(), b"builder-14-db");
    cleanup(&context);
}

#[test]
fn cancellation_prefix_is_stable_for_cancelled_index_output() {
    let readiness = readiness_from_info(&IndexOutput {
        status_success: false,
        status: "cancelled".to_string(),
        stdout: String::new(),
        stderr: String::new(),
        timed_out: false,
        cancelled: true,
        duration_ms: 0,
    });

    assert!(matches!(
        readiness,
        IndexReadiness::Unavailable(error) if error.starts_with("cancelled:")
    ));
}

#[test]
fn info_parser_preserves_exact_stale_status() {
    for status in [
        "stale (content)",
        "stale (age)",
        "stale (structure changed)",
    ] {
        let readiness = readiness_from_info(&IndexOutput::success(format!(
            "Index: /tmp/bsl_index.db\n  Status:   {status}\n"
        )));
        assert_eq!(
            readiness,
            IndexReadiness::Stale {
                status: status.to_string()
            }
        );
    }
}

#[test]
fn incomplete_info_is_retryable_but_never_readable() {
    let readiness = readiness_from_info(&IndexOutput::success(
        "Index: /tmp/bsl_index.db\n  Status:   incomplete\n",
    ));

    assert_eq!(readiness, IndexReadiness::Incomplete);
}

#[test]
fn path_normalization_failures_do_not_match_index_identity() {
    let context = test_context("invalid-path-identity");
    let dangling = context.workspace_root.join("dangling");
    let symlink =
        testing::create_file_symlink_for_test(context.workspace_root.join("missing"), &dangling)
            .expect("symlink fixtures exist on Unix and Windows");
    symlink.unwrap();
    let dangling_text = dangling.display().to_string();

    assert!(!stored_path_matches(Some(&dangling_text), &dangling));
    cleanup(&context);
}

#[test]
fn registered_ownership_check_detects_replacement_without_disk_validation() {
    let context = test_context("registered-ownership");
    let job = test_background_job(&context, "update");
    let lock = lock_path(&context);

    assert!(job.lock_lease.registered_ownership_is_current());
    register_active_lock(&lock, "replacement-owner", &job.lock_lease.owner);
    assert!(!job.lock_lease.registered_ownership_is_current());

    unregister_active_lock(&lock, "replacement-owner");
    drop(job);
    cleanup(&context);
}

#[test]
fn running_command_is_cancelled_when_registered_ownership_is_replaced() {
    let context = test_context("running-command-displaced");
    let mut job = test_background_job(&context, "update");
    let command = long_running_index_command(&context);
    let lock = lock_path(&context);
    let owner = Arc::clone(&job.lock_lease.owner);
    let replacement = thread::spawn(move || {
        thread::sleep(Duration::from_millis(500));
        register_active_lock(&lock, "replacement-owner", &owner);
    });

    let started = Instant::now();
    let output = run_index_command_with_heartbeat(&command, Some(&mut job.lock_lease))
        .expect("ownership loss should return a managed child output");
    replacement.join().unwrap();

    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(output.cancelled);
    unregister_active_lock(&lock_path(&context), "replacement-owner");
    drop(job);
    cleanup(&context);
}

#[test]
fn successful_background_job_records_last_run_metrics_in_status() {
    let context = test_context("metrics");
    let source_root = context.workspace_root.join("src");
    let build_id = "0123456789abcdef0123456789abcdef";
    let build_dir = rlm_build_root(&context, &source_root, build_id).unwrap();
    let db_path = build_dir.join("bsl_index.db");
    fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    fs::write(&db_path, "").unwrap();
    let status = status_path(&context);
    let lock = lock_path(&context);
    fs::create_dir_all(lock.parent().unwrap()).unwrap();
    let lock_lease = acquire_index_lock(&lock, "build", &context.workspace_root.join("src"))
        .unwrap()
        .expect("lock should be acquired for background job");

    let mut primary = print_lines_command(
        &context.workspace_root,
        true,
        &[
            "Index built in 1.2s".to_string(),
            "  Index:    v14".to_string(),
            "  Modules:  24".to_string(),
            "  Methods:  617".to_string(),
            "  DB size:  1.3 MB".to_string(),
        ],
        CancellationToken::new(),
    );
    let mut info = print_lines_command(
        &context.workspace_root,
        false,
        &[
            format!("Index: {}", db_path.display()),
            "  Status:   fresh".to_string(),
        ],
        CancellationToken::new(),
    );
    for command in [&mut primary, &mut info] {
        command.env = rlm_process_environment(build_dir.clone());
    }
    run_background_job(IndexBackgroundJob {
        action: "build".to_string(),
        context: context.clone(),
        source_root,
        source_lease: None,
        primary,
        info,
        status_path: status.clone(),
        lock_path: lock.clone(),
        lock_lease,
    });

    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&status).unwrap()).unwrap();
    let metrics = value
        .get("last_run")
        .expect("ready status should include last_run metrics");
    assert_eq!(metrics["action"], "build");
    assert_eq!(metrics["timed_out"], false);
    assert!(metrics["duration_ms"].as_u64().unwrap() > 0);
    assert!(metrics["finished_at"].as_u64().unwrap() >= metrics["started_at"].as_u64().unwrap());
    assert_eq!(metrics["index_version"], "v14");
    assert_eq!(metrics["modules"], 24);
    assert_eq!(metrics["methods"], 617);
    assert_eq!(metrics["db_size"], "1.3 MB");
    let current = read_lock_path(&lock).expect("completed job should leave a marker");
    assert_eq!(current.state, "released");
    assert!(current.child_pid.is_some());
    cleanup(&context);
}

#[test]
fn actor_bound_background_job_publishes_no_result_after_mid_command_root_swap() {
    let context = test_context("actor-bound-background-root-replacement");
    let source_root = context.workspace_root.join("src");
    fs::create_dir_all(&source_root).unwrap();
    let source_root = fs::canonicalize(source_root).unwrap();
    let source_lease = unica_workspace::test_support::provider_source_lease(&source_root).unwrap();
    let mut job = test_background_job(&context, "build");
    job.source_lease = Some(source_lease);
    write_status_path(
        &job.status_path,
        BslIndexStatus::building("build", Some(&source_root)),
    )
    .unwrap();
    let before = fs::read(&job.status_path).unwrap();
    let observation = Arc::clone(&job.lock_lease.observation);
    let displaced = context.workspace_root.join("src-displaced");
    let replacement = source_root.clone();

    run_background_job_with(job, move |_command, _lease| {
        fs::rename(&replacement, &displaced).unwrap();
        fs::create_dir_all(&replacement).unwrap();
        Ok(IndexOutput::success("Index built"))
    });

    assert_eq!(fs::read(status_path(&context)).unwrap(), before);
    assert_eq!(
        observed_status(&observation),
        IndexMaintenanceStatus::Unknown
    );
    cleanup(&context);
}

#[test]
fn actor_bound_index_discards_background_start_error_after_root_replacement() {
    struct ReplacingStartRunner {
        source_root: PathBuf,
        displaced: PathBuf,
    }

    impl IndexRunner for ReplacingStartRunner {
        fn start_background(&self, _job: IndexBackgroundJob) -> Result<(), String> {
            fs::rename(&self.source_root, &self.displaced).unwrap();
            fs::create_dir_all(&self.source_root).unwrap();
            Err("FOREIGN-REPLACEMENT-DIAGNOSTICS".to_string())
        }
    }

    let context = test_context("actor-bound-background-start-root-replacement");
    let source_root = context.workspace_root.join("src");
    fs::create_dir_all(&source_root).unwrap();
    let source_root = normalize_path_identity(&source_root).unwrap();
    let source_lease = unica_workspace::test_support::provider_source_lease(&source_root).unwrap();
    let runner = ReplacingStartRunner {
        source_root: source_root.clone(),
        displaced: context.workspace_root.join("src-displaced"),
    };
    let service = WorkspaceIndexService::with_runner(&runner, &FIXTURE_ENVIRONMENT)
        .with_source_lease(source_lease);
    let report = service.start_isolated_build(&context, &source_root, &CancellationToken::new());

    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("changed after admission")),
        "{report:?}"
    );
    assert!(read_bsl_index_status(&context).is_none());
    cleanup(&context);
}

#[test]
fn cancelled_index_info_returns_promptly() {
    let context = test_context("cancelled-info");
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let command = print_lines_command(
        &context.workspace_root,
        true,
        &["Index: /tmp/bsl_index.db".to_string()],
        cancellation,
    );

    let started = Instant::now();
    let output = run_index_command(&command).expect("cancelled command should return output");

    assert!(output.cancelled);
    assert!(!output.status_success);
    assert!(started.elapsed() < Duration::from_secs(2));
    cleanup(&context);
}

#[test]
fn timed_out_index_info_returns_promptly_without_cancellation() {
    let context = test_context("timed-out-info");
    let mut command = print_lines_command(
        &context.workspace_root,
        true,
        &["Index: /tmp/bsl_index.db".to_string()],
        CancellationToken::new(),
    );
    command.timeout = Duration::ZERO;

    let started = Instant::now();
    let output = run_index_command(&command).expect("timed-out command should return output");

    assert!(output.timed_out);
    assert!(!output.cancelled);
    assert!(!output.status_success);
    assert!(started.elapsed() < Duration::from_secs(2));
    cleanup(&context);
}

#[test]
fn managed_cancelled_output_never_maps_to_success() {
    let output = map_managed_output(
        unica_process::ManagedOutput {
            status_success: true,
            status: "exit status: 0".to_string(),
            stdout: String::new(),
            stderr: String::new(),
            timed_out: false,
            cancelled: true,
            stdout_truncated: false,
            stderr_truncated: false,
            stdout_had_invalid_utf8: false,
            stderr_had_invalid_utf8: false,
        },
        Duration::from_millis(1),
    );

    assert!(!output.status_success);
    assert!(output.cancelled);
    assert!(!output.timed_out);
}

#[test]
fn managed_timed_out_output_never_maps_to_success() {
    let output = map_managed_output(
        unica_process::ManagedOutput {
            status_success: true,
            status: "exit status: 0".to_string(),
            stdout: String::new(),
            stderr: String::new(),
            timed_out: true,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            stdout_had_invalid_utf8: false,
            stderr_had_invalid_utf8: false,
        },
        Duration::from_millis(1),
    );

    assert!(!output.status_success);
    assert!(output.timed_out);
    assert!(!output.cancelled);
}

#[test]
fn managed_truncation_is_visible_at_index_boundary() {
    let output = map_managed_output(
        unica_process::ManagedOutput {
            status_success: false,
            status: "exit status: 0".into(),
            stdout: "tail".into(),
            stderr: "diagnostic tail".into(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: true,
            stderr_truncated: true,
            stdout_had_invalid_utf8: false,
            stderr_had_invalid_utf8: false,
        },
        Duration::from_millis(1),
    );
    assert!(output.stderr.contains("stdout capture truncated"));
    assert!(output.stderr.contains("earlier stderr diagnostics omitted"));
}

#[test]
fn cancelled_background_job_records_failure_and_releases_lock() {
    let context = test_context("cancelled-background");
    let status = status_path(&context);
    let lock = lock_path(&context);
    fs::create_dir_all(lock.parent().unwrap()).unwrap();
    let lock_lease = acquire_index_lock(&lock, "build", &context.workspace_root.join("src"))
        .unwrap()
        .expect("lock should be acquired for background job");
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    run_background_job(IndexBackgroundJob {
        action: "build".to_string(),
        context: context.clone(),
        source_root: context.workspace_root.join("src"),
        source_lease: None,
        primary: print_lines_command(
            &context.workspace_root,
            true,
            &["Index built".to_string()],
            cancellation,
        ),
        info: print_lines_command(
            &context.workspace_root,
            false,
            &["Index not found: /tmp/bsl_index.db".to_string()],
            CancellationToken::new(),
        ),
        status_path: status.clone(),
        lock_path: lock.clone(),
        lock_lease,
    });

    let current_status: BslIndexStatus =
        serde_json::from_str(&fs::read_to_string(&status).unwrap()).unwrap();
    assert_eq!(current_status.status, "failed");
    assert!(current_status
        .message
        .as_deref()
        .is_some_and(|message| message.starts_with("cancelled:")));
    assert!(current_status.last_run.is_some());
    let current_lock = read_lock_path(&lock).expect("cancelled job should leave a marker");
    assert_eq!(current_lock.state, "released");
    cleanup(&context);
}

#[test]
fn released_lock_does_not_block_next_index_build() {
    let context = test_context("released-lock");
    fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
    write_released_lock(&context, "build");
    write_old_building_status(&context, "build");
    let runner = RecordingIndexRunner::default();
    let service = WorkspaceIndexService::with_runner(&runner, &FIXTURE_ENVIRONMENT);

    let report = service.start_isolated_build(
        &context,
        &context.workspace_root.join("src"),
        &CancellationToken::new(),
    );

    assert_eq!(report.maintenance, IndexMaintenanceStatus::Queued);
    assert_eq!(
        runner.backgrounds.borrow()[0].primary.args[0..2],
        ["index", "build"]
    );
    cleanup(&context);
}

#[test]
fn invalid_lock_without_building_status_is_treated_as_active() {
    let context = test_context("invalid-lock-active");
    let source_root = default_source_root(&context);
    fs::create_dir_all(&source_root).unwrap();
    write_usable_build(&context, &"c".repeat(32));
    let lock = lock_path(&context);
    fs::create_dir_all(lock.parent().unwrap()).unwrap();
    fs::write(&lock, b"").unwrap();
    let lock_before = fs::read(&lock).unwrap();

    assert!(!bsl_index_is_ready(&context, &FIXTURE_ENVIRONMENT));
    assert_eq!(
        fs::read(&lock).unwrap(),
        lock_before,
        "fresh invalid lock must not be recovered"
    );
    cleanup(&context);
}

#[test]
fn fresh_invalid_lock_with_stale_status_is_treated_as_active() {
    let context = test_context("invalid-lock-with-stale-status");
    let source_root = default_source_root(&context);
    fs::create_dir_all(&source_root).unwrap();
    let lock = lock_path(&context);
    fs::create_dir_all(lock.parent().unwrap()).unwrap();
    fs::write(&lock, b"").unwrap();
    write_old_building_status(&context, "build");
    let lock_before = fs::read(&lock).unwrap();
    let status_before = fs::read(status_path(&context)).unwrap();

    assert!(active_lock(&context, &source_root).unwrap());
    assert_eq!(fs::read(&lock).unwrap(), lock_before);
    assert_eq!(fs::read(status_path(&context)).unwrap(), status_before);
    assert_eq!(read_bsl_index_status(&context).unwrap().status, "building");
    cleanup(&context);
}

#[test]
fn stale_legacy_lock_is_recovered_by_active_lock() {
    let context = test_context("stale-legacy-lock");
    let source_root = default_source_root(&context);
    fs::create_dir_all(&source_root).unwrap();
    let lock = lock_path(&context);
    fs::create_dir_all(lock.parent().unwrap()).unwrap();
    fs::write(&lock, b"").unwrap();
    write_old_building_status(&context, "build");
    make_lock_file_old(&context);

    assert!(!active_lock(&context, &source_root).unwrap());
    assert_eq!(read_lock_path(&lock).unwrap().state, "recovered");
    assert_eq!(read_bsl_index_status(&context).unwrap().status, "failed");
    cleanup(&context);
}

#[test]
fn stale_lock_held_by_current_process_is_still_active() {
    let context = test_context("stale-held-lock");
    fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
    let lock = lock_path(&context);
    fs::create_dir_all(lock.parent().unwrap()).unwrap();
    let mut lease = acquire_index_lock(&lock, "build", &context.workspace_root.join("src"))
        .unwrap()
        .expect("lock should be acquired");
    force_lock_updated_at(
        &mut lease,
        now_secs().saturating_sub(LOCK_STALE_AFTER.as_secs() + 1),
    );
    assert!(active_lock(&context, &context.workspace_root.join("src")).unwrap());
    drop(lease);
    cleanup(&context);
}

#[test]
fn cleanup_does_not_remove_lock_replaced_by_new_owner() {
    let context = test_context("cleanup-owner");
    let lock = lock_path(&context);
    fs::create_dir_all(lock.parent().unwrap()).unwrap();
    let lease = acquire_index_lock(&lock, "build", &context.workspace_root.join("src"))
        .unwrap()
        .expect("old owner should acquire lock");
    let mut new_lock = BslIndexLock::new("build", &context.workspace_root.join("src"));
    new_lock.lock_id = "new-owner".to_string();
    write_lock_path(&lock, new_lock.clone()).unwrap();

    drop(lease);

    let current = read_lock_path(&lock).expect("replacement lock should remain");
    assert_eq!(current.lock_id, new_lock.lock_id);
    cleanup(&context);
}

#[test]
fn heartbeat_does_not_overwrite_lock_replaced_by_new_owner() {
    let context = test_context("heartbeat-owner");
    let lock = lock_path(&context);
    fs::create_dir_all(lock.parent().unwrap()).unwrap();
    let mut lease = acquire_index_lock(&lock, "build", &context.workspace_root.join("src"))
        .unwrap()
        .expect("old owner should acquire lock");
    let mut new_lock = BslIndexLock::new("build", &context.workspace_root.join("src"));
    new_lock.lock_id = "new-owner".to_string();
    write_lock_path(&lock, new_lock.clone()).unwrap();

    lease.refresh(42);

    let current = read_lock_path(&lock).expect("replacement lock should remain readable");
    assert_eq!(current.lock_id, new_lock.lock_id);
    assert_eq!(current.child_pid, new_lock.child_pid);
    drop(lease);
    cleanup(&context);
}

#[test]
fn failed_background_start_does_not_remove_lock_replaced_by_new_owner() {
    let context = test_context("start-background-owner");
    fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
    let lock = lock_path(&context);
    let runner = FailingReplacingIndexRunner {
        replacement_lock_id: "new-owner".to_string(),
    };
    let service = WorkspaceIndexService::with_runner(&runner, &FIXTURE_ENVIRONMENT);

    let report = service.start_isolated_build(
        &context,
        &context.workspace_root.join("src"),
        &CancellationToken::new(),
    );

    assert!(report
        .warnings
        .iter()
        .any(|warning| warning.contains("simulated background start failure")));
    let current = read_lock_path(&lock).expect("replacement lock should remain");
    assert_eq!(current.lock_id, "new-owner");
    cleanup(&context);
}

#[test]
fn stale_recovery_does_not_overwrite_an_unexpected_held_record() {
    let context = test_context("recovery-unexpected-record");
    let source_root = context.workspace_root.join("src");
    fs::create_dir_all(&source_root).unwrap();
    let path = super::lock_path(&context, &source_root).unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    write_lock_path(&path, BslIndexLock::new("build", &source_root)).unwrap();
    let before = fs::read(&path).unwrap();
    let recovered = mark_lock_recovered(
        &path,
        Some("another-expected-lock-id"),
        &source_root,
        "must not recover a different record",
    );
    let after = fs::read(&path).unwrap();
    assert!(
        !recovered,
        "recovery must reject a different expected lock id before writing"
    );
    assert_eq!(
        after, before,
        "recovery must preserve the unexpected record bytes"
    );
    cleanup(&context);
}

#[test]
fn stale_structured_lock_is_recovered_by_active_lock() {
    let context = test_context("stale-structured-recovered");
    let source_root = default_source_root(&context);
    fs::create_dir_all(&source_root).unwrap();
    write_stale_lock(&context, "build");
    write_old_building_status(&context, "build");

    assert!(!active_lock(&context, &source_root).unwrap());

    let current = read_lock_path(&lock_path(&context)).expect("stale lock should remain as marker");
    assert_eq!(current.state, "recovered");
    assert_eq!(read_bsl_index_status(&context).unwrap().status, "failed");
    cleanup(&context);
}

#[test]
fn stale_recovery_preserves_the_matching_record_identity_and_fields() {
    let context = test_context("recovery-matching-record");
    let source_root = context.workspace_root.join("src");
    fs::create_dir_all(&source_root).unwrap();
    let path = super::lock_path(&context, &source_root).unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut original = BslIndexLock::new("update", &source_root);
    original.owner_pid = 12345;
    original.child_pid = Some(23456);
    original.started_at = now_secs().saturating_sub(LOCK_STALE_AFTER.as_secs() + 1);
    original.updated_at = original.started_at;
    write_lock_path(&path, original.clone()).unwrap();
    let identity = observed_state_file_identity(&File::open(&path).unwrap()).unwrap();
    let started = now_secs();
    let reason = "recover the matching stale record";

    assert!(mark_lock_recovered(
        &path,
        Some(&original.lock_id),
        &source_root,
        reason,
    ));

    let current = read_lock_path(&path).unwrap();
    assert_eq!(
        observed_state_file_identity(&File::open(&path).unwrap()).unwrap(),
        identity
    );
    assert_eq!(current.schema_version, original.schema_version);
    assert_eq!(current.lock_id, original.lock_id);
    assert_eq!(current.owner_pid, original.owner_pid);
    assert_eq!(current.action, original.action);
    assert_eq!(current.source_root, original.source_root);
    assert_eq!(current.started_at, original.started_at);
    assert_eq!(current.child_pid, original.child_pid);
    assert_eq!(current.state, "recovered");
    assert!(current.updated_at >= started);
    assert_eq!(current.released_at, Some(current.updated_at));
    assert_eq!(current.message.as_deref(), Some(reason));
    cleanup(&context);
}

#[test]
fn stale_recovery_with_expected_identity_preserves_unreadable_records() {
    let context = test_context("recovery-unreadable-record");
    let source_root = context.workspace_root.join("src");
    fs::create_dir_all(&source_root).unwrap();
    let path = super::lock_path(&context, &source_root).unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    assert!(!mark_lock_recovered(
        &path,
        Some("expected-record-id"),
        &source_root,
        "expected record disappeared",
    ));
    assert!(
        !path.exists(),
        "refusal must not create a replacement marker"
    );
    for bytes in [b"".as_slice(), b"{", b"\xff"] {
        fs::write(&path, bytes).unwrap();
        let identity = observed_state_file_identity(&File::open(&path).unwrap()).unwrap();

        assert!(!mark_lock_recovered(
            &path,
            Some("expected-record-id"),
            &source_root,
            "identity could not be read",
        ));

        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(
            observed_state_file_identity(&File::open(&path).unwrap()).unwrap(),
            identity
        );
    }
    cleanup(&context);
}

#[derive(Default)]
struct RecordingIndexRunner {
    backgrounds: RefCell<Vec<IndexBackgroundJob>>,
}

impl IndexRunner for RecordingIndexRunner {
    fn start_background(&self, job: IndexBackgroundJob) -> Result<(), String> {
        self.backgrounds.borrow_mut().push(job);
        Ok(())
    }
}

#[cfg(test)]
struct FailingReplacingIndexRunner {
    replacement_lock_id: String,
}

#[cfg(test)]
impl IndexRunner for FailingReplacingIndexRunner {
    fn start_background(&self, job: IndexBackgroundJob) -> Result<(), String> {
        let mut replacement = BslIndexLock::new("build", &job.source_root);
        replacement.lock_id = self.replacement_lock_id.clone();
        write_lock_path(&job.lock_path, replacement).unwrap();
        Err("simulated background start failure".to_string())
    }
}

#[cfg(test)]
fn force_lock_updated_at(lease: &mut IndexLockLease, updated_at: u64) {
    let mut owner = lease.owner.lock().unwrap();
    owner.lock.updated_at = updated_at;
    owner.write_record().unwrap();
}

#[cfg(test)]
impl IndexOutput {
    fn success(stdout: impl Into<String>) -> Self {
        Self {
            status_success: true,
            status: "exit status: 0".to_string(),
            stdout: stdout.into(),
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            duration_ms: 0,
        }
    }
}

#[cfg(test)]
fn inert_index_command(context: &WorkspaceContext, verb: &str) -> IndexCommand {
    IndexCommand {
        program: PathBuf::from("unused-by-scripted-runner"),
        args: vec![
            "index".into(),
            verb.into(),
            context.workspace_root.join("src").into_os_string(),
        ],
        cwd: context.workspace_root.clone(),
        env: vec![(
            "RLM_INDEX_DIR".into(),
            rlm_generation_root(context, &default_source_root(context))
                .unwrap()
                .into_os_string(),
        )],
        // These fixtures assert on a finished run, not on how fast it
        // finished. The Windows fixture shells out to `powershell`, whose
        // cold start alone can take seconds on a loaded runner, so a tight
        // budget reports `timed_out` for a command that did its job.
        timeout: Duration::from_secs(30),
        cancellation: CancellationToken::new(),
    }
}

#[cfg(test)]
fn long_running_index_command(context: &WorkspaceContext) -> IndexCommand {
    let command = unica_process::test_support::long_running_command();
    IndexCommand {
        program: command.program,
        args: command.args.into_iter().map(Into::into).collect(),
        cwd: context.workspace_root.clone(),
        env: Vec::new(),
        timeout: Duration::from_secs(30),
        cancellation: CancellationToken::new(),
    }
}

#[cfg(test)]
fn test_background_job(context: &WorkspaceContext, action: &str) -> IndexBackgroundJob {
    let lock = lock_path(context);
    fs::create_dir_all(lock.parent().unwrap()).unwrap();
    let lock_lease = acquire_index_lock(&lock, action, &context.workspace_root.join("src"))
        .unwrap()
        .expect("test background job should acquire lock");
    IndexBackgroundJob {
        action: action.to_string(),
        context: context.clone(),
        source_root: context.workspace_root.join("src"),
        source_lease: None,
        primary: inert_index_command(context, action),
        info: inert_index_command(context, "info"),
        status_path: status_path(context),
        lock_path: lock,
        lock_lease,
    }
}

#[cfg(test)]
fn print_lines_command(
    cwd: &Path,
    sleep_first: bool,
    lines: &[String],
    cancellation: CancellationToken,
) -> IndexCommand {
    let command = unica_process::test_support::line_printing_command(sleep_first, lines);
    let context = WorkspaceContext {
        cwd: cwd.to_path_buf(),
        workspace_root: cwd.to_path_buf(),
        cache_root: cwd.join(".build/unica"),
        workspace_epoch: 1,
    };
    IndexCommand {
        program: command.program,
        args: command.args.into_iter().map(Into::into).collect(),
        cwd: cwd.to_path_buf(),
        env: vec![(
            "RLM_INDEX_DIR".into(),
            rlm_generation_root(&context, &default_source_root(&context))
                .unwrap()
                .into_os_string(),
        )],
        // See `index_command`: the budget bounds a hung fixture, it does
        // not assert how quickly `powershell` starts.
        timeout: Duration::from_secs(30),
        cancellation,
    }
}

fn test_context(name: &str) -> WorkspaceContext {
    let root = std::env::temp_dir().join(format!("unica-index-{name}-{}", now_nanos()));
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "source-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    create_fake_plugin_root(&root);
    WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".build").join("unica"),
        workspace_epoch: 1,
    }
}

fn create_fake_plugin_root(root: &Path) {
    let plugin_root = root.join("plugins").join("unica");
    fs::create_dir_all(plugin_root.join("skills")).unwrap();
    fs::create_dir_all(plugin_root.join("third-party")).unwrap();
    for target in ["darwin-arm64", "linux-x64"] {
        fs::create_dir_all(plugin_root.join("bin").join(target)).unwrap();
        fs::write(
            plugin_root.join("bin").join(target).join("rlm-bsl-index"),
            "rlm-index",
        )
        .unwrap();
    }
    fs::create_dir_all(plugin_root.join("bin/win-x64")).unwrap();
    fs::write(
        plugin_root.join("bin/win-x64").join("rlm-bsl-index.exe"),
        "rlm-index",
    )
    .unwrap();
    fs::write(
            plugin_root.join("third-party/manifest.json"),
            r#"{
  "schemaVersion": 2,
  "tools": [
    {
      "name": "rlm-bsl-index",
      "binaries": {
        "darwin-arm64": {"targetTriple": "aarch64-apple-darwin", "binaryPath": "bin/darwin-arm64/rlm-bsl-index", "sha256": "fa6a77fa531fa57e7781010a7cec69b7be4b7b58903365153bf1f66e851ab213"},
        "linux-x64": {"targetTriple": "x86_64-unknown-linux-gnu", "binaryPath": "bin/linux-x64/rlm-bsl-index", "sha256": "fa6a77fa531fa57e7781010a7cec69b7be4b7b58903365153bf1f66e851ab213"},
        "win-x64": {"targetTriple": "x86_64-pc-windows-msvc", "binaryPath": "bin/win-x64/rlm-bsl-index.exe", "sha256": "fa6a77fa531fa57e7781010a7cec69b7be4b7b58903365153bf1f66e851ab213"}
      }
    }
  ]
}"#,
        )
        .unwrap();
}

#[cfg(test)]
fn write_stale_lock(context: &WorkspaceContext, action: &str) {
    fs::create_dir_all(lock_path(context).parent().unwrap()).unwrap();
    let mut lock = BslIndexLock::new(action, &context.workspace_root.join("src"));
    lock.started_at = now_secs().saturating_sub(LOCK_STALE_AFTER.as_secs() + 1);
    lock.updated_at = lock.started_at;
    write_lock_path(&lock_path(context), lock).unwrap();
}

#[cfg(test)]
fn write_fresh_lock(context: &WorkspaceContext, action: &str) {
    fs::create_dir_all(lock_path(context).parent().unwrap()).unwrap();
    let lock = BslIndexLock::new(action, &context.workspace_root.join("src"));
    write_lock_path(&lock_path(context), lock).unwrap();
}

#[cfg(test)]
fn write_released_lock(context: &WorkspaceContext, action: &str) {
    fs::create_dir_all(lock_path(context).parent().unwrap()).unwrap();
    let now = now_secs();
    let text = serde_json::json!({
        "schema_version": LOCK_SCHEMA_VERSION,
        "lock_id": "released",
        "owner_pid": 999999,
        "action": action,
        "source_root": context.workspace_root.join("src").display().to_string(),
        "started_at": now,
        "updated_at": now,
        "state": "released",
        "released_at": now
    });
    fs::write(
        lock_path(context),
        serde_json::to_string_pretty(&text).unwrap() + "\n",
    )
    .unwrap();
}

#[cfg(test)]
fn write_old_building_status(context: &WorkspaceContext, action: &str) {
    let mut status = BslIndexStatus::building(action, Some(&context.workspace_root.join("src")));
    status.updated_at = now_secs().saturating_sub(LOCK_STALE_AFTER.as_secs() + 1);
    write_status(context, status).unwrap();
}

#[cfg(test)]
fn write_usable_build(context: &WorkspaceContext, build_id: &str) -> PathBuf {
    let source_root = default_source_root(context);
    let build_root = rlm_build_root(context, &source_root, build_id).unwrap();
    let db_path = build_root.join("bsl_index.db");
    fs::create_dir_all(&build_root).unwrap();
    fs::write(&db_path, b"usable index").unwrap();
    write_status(
        context,
        BslIndexStatus::ready(&source_root, &db_path).with_build_id(build_id.to_string()),
    )
    .unwrap();
    db_path
}

#[cfg(test)]
fn make_lock_file_old(context: &WorkspaceContext) {
    use std::fs::FileTimes;

    const JANUARY_1_2000_UTC: Duration = Duration::from_secs(946_684_800);
    let file = OpenOptions::new()
        .write(true)
        .open(lock_path(context))
        .unwrap();
    file.set_times(FileTimes::new().set_modified(UNIX_EPOCH + JANUARY_1_2000_UTC))
        .unwrap();
}

#[test]
fn isolated_build_keeps_last_usable_index_until_candidate_is_ready() {
    let context = test_context("isolated-build-active-cutover");
    let source_root = default_source_root(&context);
    fs::create_dir_all(&source_root).unwrap();
    let old_id = "1".repeat(32);
    let old_directory = rlm_build_root(&context, &source_root, &old_id).unwrap();
    let old_db = old_directory.join("bsl_index.db");
    fs::create_dir_all(&old_directory).unwrap();
    fs::write(&old_db, b"old usable database").unwrap();
    write_status(
        &context,
        BslIndexStatus::ready(&source_root, &old_db).with_build_id(old_id.clone()),
    )
    .unwrap();
    let old_marker = fs::read(status_path(&context)).unwrap();
    let runner = RecordingIndexRunner::default();
    let service = WorkspaceIndexService::with_runner(&runner, &FIXTURE_ENVIRONMENT);

    let report = service.start_isolated_build(&context, &source_root, &CancellationToken::new());
    assert_eq!(report.maintenance, IndexMaintenanceStatus::Queued);
    let failed_job = runner.backgrounds.borrow_mut().pop().unwrap();
    let failed_directory = PathBuf::from(index_command_env(&failed_job.info, "RLM_INDEX_DIR"));
    assert_ne!(failed_directory, old_directory);
    assert_eq!(fs::read(status_path(&context)).unwrap(), old_marker);
    assert_eq!(
        service
            .active_build(&context, &source_root)
            .unwrap()
            .unwrap()
            .id,
        old_id
    );
    run_background_job_with(failed_job, |_command, _lease| {
        Err("candidate build failed".to_string())
    });
    assert_eq!(fs::read(status_path(&context)).unwrap(), old_marker);
    assert_eq!(
        service
            .active_build(&context, &source_root)
            .unwrap()
            .unwrap()
            .db_path,
        old_db
    );

    let report = service.start_isolated_build(&context, &source_root, &CancellationToken::new());
    assert_eq!(report.maintenance, IndexMaintenanceStatus::Queued);
    let ready_job = runner.backgrounds.borrow_mut().pop().unwrap();
    let candidate_directory = PathBuf::from(index_command_env(&ready_job.info, "RLM_INDEX_DIR"));
    let candidate_id = candidate_directory
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_ne!(candidate_directory, failed_directory);
    let candidate_db = candidate_directory.join("bsl_index.db");
    fs::create_dir_all(&candidate_directory).unwrap();
    fs::write(&candidate_db, b"new usable database").unwrap();
    let source_file = source_root.join("Module.bsl");
    run_background_job_with(ready_job, |command, _lease| {
        if command.args.get(1).is_some_and(|verb| verb == "build") {
            fs::write(&source_file, b"source edited while building").unwrap();
            Ok(IndexOutput::success("Index built"))
        } else {
            Ok(IndexOutput::success(format!(
                "Index: {}\n  Status: fresh\n",
                candidate_db.display()
            )))
        }
    });

    let active = service
        .active_build(&context, &source_root)
        .unwrap()
        .unwrap();
    assert_eq!(active.id, candidate_id);
    assert_eq!(active.db_path, candidate_db);
    assert_eq!(fs::read(&old_db).unwrap(), b"old usable database");
    let marker = read_bsl_index_status(&context).unwrap();
    assert_eq!(marker.build_id.as_deref(), Some(candidate_id.as_str()));
    cleanup(&context);
}

#[test]
fn legacy_ready_marker_requires_an_isolated_build_before_becoming_active() {
    let context = test_context("legacy-ready-isolated-migration");
    let source_root = default_source_root(&context);
    fs::create_dir_all(&source_root).unwrap();
    let legacy_directory = rlm_generation_root(&context, &source_root).unwrap();
    let legacy_db = legacy_directory.join("bsl_index.db");
    fs::create_dir_all(&legacy_directory).unwrap();
    fs::write(&legacy_db, b"legacy mutable database").unwrap();
    // Older ready markers have no build_id and point into the shared
    // generation directory, which an older builder may still overwrite.
    write_status(&context, BslIndexStatus::ready(&source_root, &legacy_db)).unwrap();
    let legacy_marker = fs::read(status_path(&context)).unwrap();
    let runner = RecordingIndexRunner::default();
    let service = WorkspaceIndexService::with_runner(&runner, &FIXTURE_ENVIRONMENT);
    assert_eq!(service.active_build(&context, &source_root).unwrap(), None);
    assert!(!bsl_index_is_ready(&context, &FIXTURE_ENVIRONMENT));

    let report = service.start_isolated_build(&context, &source_root, &CancellationToken::new());
    assert_eq!(report.maintenance, IndexMaintenanceStatus::Queued);
    let job = runner.backgrounds.borrow_mut().pop().unwrap();
    let candidate_directory = PathBuf::from(index_command_env(&job.info, "RLM_INDEX_DIR"));
    let candidate_id = isolated_build_id(&job.info).unwrap();
    assert_eq!(
        candidate_directory,
        legacy_directory.join("builds").join(&candidate_id)
    );
    assert_eq!(
        PathBuf::from(index_command_env(&job.primary, "RLM_INDEX_DIR")),
        candidate_directory
    );
    assert_eq!(fs::read(status_path(&context)).unwrap(), legacy_marker);
    assert_eq!(service.active_build(&context, &source_root).unwrap(), None);
    let candidate_db = candidate_directory.join("bsl_index.db");
    run_background_job_with(job, |command, _lease| {
        if command.args.get(1).is_some_and(|verb| verb == "build") {
            fs::create_dir_all(&candidate_directory).unwrap();
            fs::write(&candidate_db, b"isolated database").unwrap();
            Ok(IndexOutput::success("Index built"))
        } else {
            Ok(IndexOutput::success(format!(
                "Index: {}\n  Status: fresh\n",
                candidate_db.display()
            )))
        }
    });
    let active = service
        .active_build(&context, &source_root)
        .unwrap()
        .unwrap();
    assert_eq!(active.id, candidate_id);
    assert_eq!(active.directory, candidate_directory);
    assert_eq!(active.db_path, candidate_db);
    assert!(bsl_index_is_ready(&context, &FIXTURE_ENVIRONMENT));
    assert_eq!(fs::read(&legacy_db).unwrap(), b"legacy mutable database");
    fs::write(&legacy_db, b"rewritten by legacy builder").unwrap();
    assert_eq!(fs::read(&active.db_path).unwrap(), b"isolated database");
    assert_eq!(
        service.active_build(&context, &source_root).unwrap(),
        Some(active)
    );
    cleanup(&context);
}

#[test]
fn active_build_rejects_foreign_root_and_invalid_build_route() {
    let context = test_context("active-build-route");
    let source_root = default_source_root(&context);
    fs::create_dir_all(&source_root).unwrap();
    let build_id = "2".repeat(32);
    let directory = rlm_build_root(&context, &source_root, &build_id).unwrap();
    let db_path = directory.join("bsl_index.db");
    fs::create_dir_all(&directory).unwrap();
    fs::write(&db_path, b"usable").unwrap();
    write_status(
        &context,
        BslIndexStatus::ready(&context.workspace_root.join("other"), &db_path)
            .with_build_id(build_id.clone()),
    )
    .unwrap();
    assert_eq!(usable_index_build(&context, &source_root).unwrap(), None);

    write_status(
        &context,
        BslIndexStatus::ready(&source_root, &db_path).with_build_id("../outside".to_string()),
    )
    .unwrap();
    assert!(usable_index_build(&context, &source_root)
        .unwrap_err()
        .contains("invalid RLM index build identifier"));
    cleanup(&context);
}

#[test]
fn candidate_symlink_route_never_replaces_last_usable_marker() {
    let context = test_context("candidate-symlink-route");
    let source_root = default_source_root(&context);
    fs::create_dir_all(&source_root).unwrap();
    let old_id = "3".repeat(32);
    let old_directory = rlm_build_root(&context, &source_root, &old_id).unwrap();
    let old_db = old_directory.join("bsl_index.db");
    fs::create_dir_all(&old_directory).unwrap();
    fs::write(&old_db, b"old usable database").unwrap();
    write_status(
        &context,
        BslIndexStatus::ready(&source_root, &old_db).with_build_id(old_id.clone()),
    )
    .unwrap();
    let old_marker = fs::read(status_path(&context)).unwrap();
    let runner = RecordingIndexRunner::default();
    let service = WorkspaceIndexService::with_runner(&runner, &FIXTURE_ENVIRONMENT);
    service.start_isolated_build(&context, &source_root, &CancellationToken::new());
    let job = runner.backgrounds.borrow_mut().pop().unwrap();
    let candidate_directory = PathBuf::from(index_command_env(&job.info, "RLM_INDEX_DIR"));
    let outside = context.workspace_root.join("foreign-db");
    fs::create_dir_all(&candidate_directory).unwrap();
    fs::create_dir_all(&outside).unwrap();
    let link = testing::create_dir_symlink_for_test(&outside, candidate_directory.join("sub"))
        .expect("symlink fixtures exist on Unix and Windows");
    link.unwrap();
    fs::write(outside.join("index.db"), b"foreign DB").unwrap();
    let escaped_db = candidate_directory.join("sub/index.db");
    run_background_job_with(job, |command, _lease| {
        if command.args.get(1).is_some_and(|verb| verb == "build") {
            Ok(IndexOutput::success("Index built"))
        } else {
            Ok(IndexOutput::success(format!(
                "Index: {}\n  Status: fresh\n",
                escaped_db.display()
            )))
        }
    });
    assert_eq!(fs::read(status_path(&context)).unwrap(), old_marker);
    assert_eq!(
        service
            .active_build(&context, &source_root)
            .unwrap()
            .unwrap()
            .id,
        old_id
    );

    service.start_isolated_build(&context, &source_root, &CancellationToken::new());
    let job = runner.backgrounds.borrow_mut().pop().unwrap();
    let replaced_directory = PathBuf::from(index_command_env(&job.info, "RLM_INDEX_DIR"));
    fs::create_dir_all(replaced_directory.parent().unwrap()).unwrap();
    testing::create_dir_symlink_for_test(&outside, &replaced_directory)
        .expect("symlinks were supported for the first candidate")
        .unwrap();
    let escaped_db = replaced_directory.join("index.db");
    run_background_job_with(job, |command, _lease| {
        if command.args.get(1).is_some_and(|verb| verb == "build") {
            Ok(IndexOutput::success("Index built"))
        } else {
            Ok(IndexOutput::success(format!(
                "Index: {}\n  Status: fresh\n",
                escaped_db.display()
            )))
        }
    });
    assert_eq!(fs::read(status_path(&context)).unwrap(), old_marker);
    assert_eq!(
        service
            .active_build(&context, &source_root)
            .unwrap()
            .unwrap()
            .id,
        old_id
    );
    cleanup(&context);
}

fn cleanup(context: &WorkspaceContext) {
    let _ = fs::remove_dir_all(&context.workspace_root);
}

struct FixtureEnvironment;
static FIXTURE_ENVIRONMENT: FixtureEnvironment = FixtureEnvironment;
impl crate::environment::RlmEnvironment for FixtureEnvironment {
    fn find_plugin_root(&self, cwd: &Path) -> Option<PathBuf> {
        Some(cwd.join("plugins/unica"))
    }
    fn resolve_bundled_tool(
        &self,
        root: &Path,
        _name: &str,
        _allow: bool,
    ) -> Result<
        unica_artifact_delivery::engine::BundledTool,
        unica_artifact_delivery::engine::BundledToolError,
    > {
        use unica_workspace::full_dump::{current_host_platform, HostPlatform};
        let relative = match current_host_platform() {
            HostPlatform::Windows => "bin/win-x64/rlm-bsl-index.exe",
            HostPlatform::Linux => "bin/linux-x64/rlm-bsl-index",
            HostPlatform::Other => "bin/darwin-arm64/rlm-bsl-index",
        };
        Ok(unica_artifact_delivery::engine::BundledTool {
            program: root.join(relative),
            warnings: Vec::new(),
            missing: None,
        })
    }
    fn resolve_source_root(
        &self,
        context: &WorkspaceContext,
    ) -> Result<unica_source_contract::workspace::ResolvedSourceRoot, String> {
        Ok(unica_source_contract::workspace::ResolvedSourceRoot {
            source_set: None,
            path: context.workspace_root.join("src"),
        })
    }
}
