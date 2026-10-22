use super::{
    declared_source_repo_path, git_environment_removals_from, materialize_staged_ignore_files,
    normalize_path_identity, parse_check_ignore_verbose_z, parse_git_index_entries,
    partial_clone_config_is_enabled, GitIndexEntry, GitRepositoryInspector, IgnoreMatch,
    MaterializedGitPath, StagedBlobReadSafety, StagedIgnoreMaterializationError,
};
use crate::inspection::layout::SourceLayoutInspector;
use crate::{ProjectCheckId, ProjectCheckOutcome, ProjectHealthFact, ProjectHealthInspectionError};
use std::cell::{Cell, RefCell};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use unica_process::runner::{ProcessCommand, ProcessOutput, ProcessRunner};
use unica_source_contract::workspace::WorkspaceContext;
use unica_source_contract::SourceFormat;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;

static CONFIG_DUMP_OWNERSHIP_CLOCK_TICKS: AtomicUsize = AtomicUsize::new(0);
static CONFIG_DUMP_OWNERSHIP_CLOCK_ORIGIN: OnceLock<Instant> = OnceLock::new();

fn advancing_config_dump_ownership_clock() -> Instant {
    let tick = CONFIG_DUMP_OWNERSHIP_CLOCK_TICKS.fetch_add(1, Ordering::SeqCst) as u64;
    *CONFIG_DUMP_OWNERSHIP_CLOCK_ORIGIN.get_or_init(Instant::now) + Duration::from_millis(tick)
}

static OWNER_INDEX_CLOCK_TICKS: AtomicUsize = AtomicUsize::new(0);
static OWNER_INDEX_CLOCK_ORIGIN: OnceLock<Instant> = OnceLock::new();

fn advancing_owner_index_clock() -> Instant {
    let tick = OWNER_INDEX_CLOCK_TICKS.fetch_add(1, Ordering::SeqCst) as u64;
    *OWNER_INDEX_CLOCK_ORIGIN.get_or_init(Instant::now) + Duration::from_millis(tick)
}

static GENERATED_PATH_CLOCK_TICKS: AtomicUsize = AtomicUsize::new(0);
static GENERATED_PATH_CLOCK_ORIGIN: OnceLock<Instant> = OnceLock::new();

fn advancing_generated_path_clock() -> Instant {
    let tick = GENERATED_PATH_CLOCK_TICKS.fetch_add(1, Ordering::SeqCst) as u64;
    *GENERATED_PATH_CLOCK_ORIGIN.get_or_init(Instant::now) + Duration::from_millis(tick)
}

#[test]
fn declared_ignore_candidate_preserves_git_spelling_and_normalizes_lexically() {
    let fixture = parent_repository_fixture();
    let layout = inspect_layout(&fixture.context);
    let mut root = layout.roots[0].clone();
    root.source_set.path = "src/../SRC".into();
    let repository_root = normalize_path_identity(&fixture.repository_root).unwrap();

    assert_eq!(
        declared_source_repo_path(
            &repository_root,
            &fixture.context,
            &root,
            Path::new("ConfigDumpInfo.xml"),
        )
        .as_deref(),
        Some("workspace/SRC/ConfigDumpInfo.xml")
    );
}

#[test]
fn source_owner_index_timeout_marks_ignore_not_run_for_every_source_set() {
    let fixture = health_fixture(false);
    let layout = inspect_layout(&fixture.context);
    let mut timed_out_inspection = None;

    for budget_millis in 1..64 {
        OWNER_INDEX_CLOCK_TICKS.store(0, Ordering::SeqCst);
        let deadline = ProviderDeadline::with_clock(
            advancing_owner_index_clock() + Duration::from_millis(budget_millis),
            advancing_owner_index_clock,
        );
        let mut no_partial_clone = process_output(false, "");
        no_partial_clone.status = "exit status: 1".into();
        let runner = SequenceRunner::outputs(vec![
            process_output(
                true,
                &format!("{}\ntrue\n", fixture.context.workspace_root.display()),
            ),
            process_output(true, ""),
            no_partial_clone,
        ]);
        let inspection = GitRepositoryInspector::with_runner(&runner)
            .inspect_base(
                &fixture.context,
                &layout,
                &CancellationToken::new(),
                deadline,
            )
            .unwrap();
        if inspection.observations.iter().any(|observation| {
            observation.id == ProjectCheckId::RepositoryIgnore
                && observation.source_set.is_none()
                && matches!(
                    &observation.outcome,
                    ProjectCheckOutcome::NotRun { reason }
                        if reason == "source-root ownership index construction timed out"
                )
        }) {
            timed_out_inspection = Some(inspection);
            break;
        }
    }

    let inspection = timed_out_inspection.expect("fixture must expire while building owners");
    assert!(inspection.observations.iter().any(|observation| {
        observation.id == ProjectCheckId::RepositoryIgnore
            && observation.source_set.as_deref() == Some("main")
            && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
    }));
}

#[test]
fn config_dump_info_ownership_deadline_is_typed_not_fatal() {
    let fixture = health_fixture(false);
    let path = format!("{}/ConfigDumpInfo.xml", "deep/".repeat(600));
    let entries = vec![GitIndexEntry {
        repo_path: path.clone(),
        blob_oid: Some("a".repeat(40)),
        mode: Some("100644".into()),
    }];
    let runner = SequenceRunner::outputs(vec![process_output(true, "<ConfigDumpInfo/>")]);
    let layout = inspect_layout(&fixture.context);
    let owners = super::SourceRootOwners::new(
        fixture.context.workspace_root.as_path(),
        &layout.roots,
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(1)),
    )
    .unwrap();
    CONFIG_DUMP_OWNERSHIP_CLOCK_TICKS.store(0, Ordering::SeqCst);
    let deadline = ProviderDeadline::with_clock(
        advancing_config_dump_ownership_clock() + Duration::from_millis(4),
        advancing_config_dump_ownership_clock,
    );

    let inspection = GitRepositoryInspector::with_runner(&runner)
        .inspect_config_dump_info(
            &entries,
            fixture.context.workspace_root.as_path(),
            &owners,
            &StagedBlobReadSafety::LocalOnlyGuaranteed,
            &CancellationToken::new(),
            deadline,
        )
        .expect("deadline is reported inside the typed config-dump outcome");

    assert!(matches!(
        inspection,
        Err(super::ConfigDumpInfoIndexInspectionError::TimedOut)
    ));
}

#[test]
fn generated_path_timeout_discards_partial_ordinary_facts() {
    let fixture = health_fixture(false);
    let layout = inspect_layout(&fixture.context);
    let cancellation = CancellationToken::new();
    let owners = super::SourceRootOwners::new(
        fixture.context.workspace_root.as_path(),
        &layout.roots,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(1)),
    )
    .unwrap();
    let mut entries = vec![GitIndexEntry {
        repo_path: "src/.build/first.bin".into(),
        blob_oid: Some("a".repeat(40)),
        mode: Some("100644".into()),
    }];
    entries.extend((1..=256).map(|index| GitIndexEntry {
        repo_path: format!("ordinary/{index}.txt"),
        blob_oid: Some(format!("{index:040x}")),
        mode: Some("100644".into()),
    }));
    GENERATED_PATH_CLOCK_TICKS.store(0, Ordering::SeqCst);
    let deadline = ProviderDeadline::with_clock(
        advancing_generated_path_clock() + Duration::from_millis(3),
        advancing_generated_path_clock,
    );

    let result = super::tracked_generated_facts(
        &entries,
        fixture.context.workspace_root.as_path(),
        &fixture.context,
        &owners,
        &cancellation,
        deadline,
    );

    assert!(matches!(
        result,
        Err(super::GeneratedPathInspectionError::TimedOut)
    ));
}

#[test]
fn generated_equal_root_fact_expansion_is_bounded() {
    let fixture = health_fixture(false);
    let source_sets = (0..5)
        .map(|index| format!("  - name: owner-{index}\n    type: CONFIGURATION\n    path: src\n"))
        .collect::<String>();
    fs::write(
        fixture.context.workspace_root.join("v8project.yaml"),
        format!("format: DESIGNER\nsource-set:\n{source_sets}"),
    )
    .unwrap();
    let layout = inspect_layout(&fixture.context);
    let cancellation = CancellationToken::new();
    let owners = super::SourceRootOwners::new(
        fixture.context.workspace_root.as_path(),
        &layout.roots,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(1)),
    )
    .unwrap();
    let entries = (0..820)
        .map(|index| GitIndexEntry {
            repo_path: format!("src/.build/generated-{index}.bin"),
            blob_oid: Some(format!("{index:040x}")),
            mode: Some("100644".into()),
        })
        .collect::<Vec<_>>();

    let result = super::tracked_generated_facts(
        &entries,
        fixture.context.workspace_root.as_path(),
        &fixture.context,
        &owners,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(1)),
    );

    assert!(matches!(
        result,
        Err(super::GeneratedPathInspectionError::Incomplete(reason))
            if reason.contains("fact budget")
    ));
}

#[test]
fn equal_root_runtime_sidecar_reports_ambiguous_owners() {
    let fixture = health_fixture(false);
    fs::write(
        fixture.context.workspace_root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: first\n    type: CONFIGURATION\n    path: src\n  - name: second\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    let layout = inspect_layout(&fixture.context);
    let cancellation = CancellationToken::new();
    let owners = super::SourceRootOwners::new(
        fixture.context.workspace_root.as_path(),
        &layout.roots,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(1)),
    )
    .unwrap();
    let entries = vec![GitIndexEntry {
        repo_path: "src/ConfigDumpInfo.xml".into(),
        blob_oid: Some("a".repeat(40)),
        mode: Some("100644".into()),
    }];
    let runner = SequenceRunner::outputs(vec![process_output(true, "<ConfigDumpInfo/>")]);

    let facts = GitRepositoryInspector::with_runner(&runner)
        .inspect_config_dump_info(
            &entries,
            fixture.context.workspace_root.as_path(),
            &owners,
            &StagedBlobReadSafety::LocalOnlyGuaranteed,
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(1)),
        )
        .unwrap()
        .unwrap();

    assert_eq!(facts.len(), 3, "{facts:?}");
    for source_set in [None, Some("first"), Some("second")] {
        assert!(
            facts.iter().any(|fact| matches!(
                fact,
                ProjectHealthFact::ConfigDumpInfoUnclassified {
                    source_set: actual,
                    reason,
                    ..
                } if actual.as_deref() == source_set
                    && reason.contains("ambiguous")
                    && reason.contains("first")
                    && reason.contains("second")
            )),
            "missing ambiguous fact for {source_set:?}: {facts:?}"
        );
    }
}

#[test]
fn equal_root_runtime_sidecar_reason_is_bounded() {
    let fixture = health_fixture(false);
    let first = format!("first-{}", "a".repeat(4_096));
    let second = format!("second-{}", "b".repeat(4_096));
    fs::write(
        fixture.context.workspace_root.join("v8project.yaml"),
        format!(
            "format: DESIGNER\nsource-set:\n  - name: {first}\n    type: CONFIGURATION\n    path: src\n  - name: {second}\n    type: CONFIGURATION\n    path: src\n"
        ),
    )
    .unwrap();
    let layout = inspect_layout(&fixture.context);
    let cancellation = CancellationToken::new();
    let owners = super::SourceRootOwners::new(
        fixture.context.workspace_root.as_path(),
        &layout.roots,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(1)),
    )
    .unwrap();
    let entries = vec![GitIndexEntry {
        repo_path: "src/ConfigDumpInfo.xml".into(),
        blob_oid: Some("a".repeat(40)),
        mode: Some("100644".into()),
    }];
    let runner = SequenceRunner::outputs(vec![process_output(true, "<ConfigDumpInfo/>")]);

    let facts = GitRepositoryInspector::with_runner(&runner)
        .inspect_config_dump_info(
            &entries,
            fixture.context.workspace_root.as_path(),
            &owners,
            &StagedBlobReadSafety::LocalOnlyGuaranteed,
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(1)),
        )
        .unwrap()
        .unwrap();

    assert_eq!(facts.len(), 3);
    assert!(
        facts.iter().all(|fact| matches!(
            fact,
            ProjectHealthFact::ConfigDumpInfoUnclassified { reason, .. }
                if reason.len() <= 512
        )),
        "{facts:?}"
    );
}

#[test]
fn equal_root_inconclusive_config_dump_info_reports_every_owner() {
    let fixture = health_fixture(false);
    fs::write(
        fixture.context.workspace_root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: first\n    type: CONFIGURATION\n    path: src\n  - name: second\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    let layout = inspect_layout(&fixture.context);
    let cancellation = CancellationToken::new();
    let owners = super::SourceRootOwners::new(
        fixture.context.workspace_root.as_path(),
        &layout.roots,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(1)),
    )
    .unwrap();
    let entries = vec![GitIndexEntry {
        repo_path: "src/ConfigDumpInfo.xml".into(),
        blob_oid: Some("a".repeat(40)),
        mode: Some("100644".into()),
    }];
    let runner = SequenceRunner::outputs(vec![process_output(true, "<not-config-dump/> ")]);

    let facts = GitRepositoryInspector::with_runner(&runner)
        .inspect_config_dump_info(
            &entries,
            fixture.context.workspace_root.as_path(),
            &owners,
            &StagedBlobReadSafety::LocalOnlyGuaranteed,
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(1)),
        )
        .unwrap()
        .unwrap();

    for source_set in [None, Some("first"), Some("second")] {
        assert!(
            facts.iter().any(|fact| matches!(
                fact,
                ProjectHealthFact::ConfigDumpInfoUnclassified {
                    source_set: actual,
                    reason,
                    ..
                } if actual.as_deref() == source_set
                    && reason == "staged blob classification is inconclusive"
            )),
            "missing inconclusive fact for {source_set:?}: {facts:?}"
        );
    }
}

#[test]
fn project_health_git_index_parser_groups_stages_and_preserves_unusual_paths() {
    let entries = parse_git_index_entries(concat!(
        "100644 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 1\tconflict/ConfigDumpInfo.xml\0",
        "100644 bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb 2\tconflict/ConfigDumpInfo.xml\0",
        "120000 cccccccccccccccccccccccccccccccccccccccc 0\tlinked\0",
        "100644 0000000000000000000000000000000000000000 0\tnew\0",
        "100755 dddddddddddddddddddddddddddddddddddddddd 0\tline\nbreak,ok\0",
    ))
    .unwrap();

    assert_eq!(entries.len(), 4);
    assert_eq!(entries[0].repo_path, "conflict/ConfigDumpInfo.xml");
    assert_eq!(entries[0].blob_oid, None);
    assert_eq!(entries[1].repo_path, "line\nbreak,ok");
    assert_eq!(
        entries[1].blob_oid.as_deref(),
        Some("dddddddddddddddddddddddddddddddddddddddd")
    );
    assert_eq!(entries[1].mode.as_deref(), Some("100755"));
    assert_eq!(entries[2].blob_oid, None);
    assert_eq!(entries[3].blob_oid, None);
}

#[test]
fn partial_clone_parser_preserves_whitespace_inside_remote_name() {
    assert!(partial_clone_config_is_enabled(
        "remote.foo bar.promisor\ntrue\0"
    ));
    assert!(!partial_clone_config_is_enabled(
        "remote.foo bar.promisor\nfalse\0"
    ));
}

#[test]
fn project_health_git_ignore_rejects_index_entry_without_unique_blob() {
    let repository = TempDir::new().unwrap();
    let runner = SequenceRunner::outputs(Vec::new());
    let error = materialize_staged_ignore_files(
        &runner,
        repository.path(),
        &[GitIndexEntry {
            repo_path: ".gitignore".into(),
            blob_oid: None,
            mode: None,
        }],
        &[],
        &StagedBlobReadSafety::LocalOnlyGuaranteed,
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(1)),
    )
    .unwrap_err();

    assert!(
        matches!(
            error,
            StagedIgnoreMaterializationError::Incomplete(ref reason)
                if reason.contains("does not have one regular stage-0 blob")
        ),
        "{error:?}"
    );
    assert!(runner.commands.borrow().is_empty());
}

#[test]
fn project_health_git_ignore_materializes_distinct_staged_blobs_with_bounded_processes() {
    struct BatchOnlyRunner {
        calls: RefCell<Vec<(Vec<String>, Vec<u8>)>>,
        first_oid: String,
        second_oid: String,
    }

    impl ProcessRunner for BatchOnlyRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            panic!(
                "one child per staged .gitignore is not bounded: {:?}",
                command.args
            );
        }

        fn run_with_input(
            &self,
            command: &ProcessCommand,
            input: &[u8],
        ) -> Result<ProcessOutput, String> {
            self.calls
                .borrow_mut()
                .push((command.args.clone(), input.to_vec()));
            let first = &self.first_oid;
            let second = &self.second_oid;
            if command
                .args
                .iter()
                .any(|arg| arg.starts_with("--batch-check="))
            {
                Ok(process_output(
                    true,
                    &format!("{first} blob 4\n{second} blob 4\n"),
                ))
            } else if command.args.iter().any(|arg| arg == "--batch") {
                Ok(process_output(
                    true,
                    &format!("{first} blob 4\none\n\n{second} blob 4\nt\0o\n\n"),
                ))
            } else {
                panic!(
                    "unexpected staged .gitignore Git command: {:?}",
                    command.args
                );
            }
        }
    }

    let repository = TempDir::new().unwrap();
    let first_oid = "a".repeat(40);
    let second_oid = "b".repeat(40);
    let runner = BatchOnlyRunner {
        calls: RefCell::new(Vec::new()),
        first_oid: first_oid.clone(),
        second_oid: second_oid.clone(),
    };
    let materialized = materialize_staged_ignore_files(
        &runner,
        repository.path(),
        &[
            GitIndexEntry {
                repo_path: "src/first/.gitignore".into(),
                blob_oid: Some(first_oid.clone()),
                mode: Some("100644".into()),
            },
            GitIndexEntry {
                repo_path: "src/second/.gitignore".into(),
                blob_oid: Some(second_oid.clone()),
                mode: Some("100644".into()),
            },
        ],
        &[],
        &StagedBlobReadSafety::LocalOnlyGuaranteed,
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(7)),
    )
    .unwrap();

    assert_eq!(
        fs::read(materialized.path().join("src/first/.gitignore")).unwrap(),
        b"one\n"
    );
    assert_eq!(
        fs::read(materialized.path().join("src/second/.gitignore")).unwrap(),
        b"t\0o\n"
    );
    let calls = runner.calls.borrow();
    assert_eq!(calls.len(), 2);
    let expected_input = format!("{first_oid}\n{second_oid}\n").into_bytes();
    for (_, input) in calls.iter() {
        assert_eq!(*input, expected_input);
    }
}

#[test]
fn project_health_git_ignore_batch_reads_1024_real_staged_blobs_with_two_children() {
    struct CountingGitRunner<'a> {
        inner: &'a dyn ProcessRunner,
        commands: RefCell<Vec<Vec<String>>>,
    }

    impl ProcessRunner for CountingGitRunner<'_> {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            self.commands.borrow_mut().push(command.args.clone());
            self.inner.run(command)
        }

        fn run_with_input(
            &self,
            command: &ProcessCommand,
            input: &[u8],
        ) -> Result<ProcessOutput, String> {
            self.commands.borrow_mut().push(command.args.clone());
            self.inner.run_with_input(command, input)
        }
    }

    let repository = TempDir::new().unwrap();
    git(repository.path(), &["init"]);
    for index in 0..super::MAX_STAGED_IGNORE_FILES {
        let directory = repository.path().join(format!("policy/{index:04}"));
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join(".gitignore"),
            format!("ignored-{index:04}\n"),
        )
        .unwrap();
    }
    git(repository.path(), &["add", "policy"]);
    let output = Command::new("git")
        .args(["ls-files", "--cached", "--stage", "-z"])
        .current_dir(repository.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let entries = parse_git_index_entries(&String::from_utf8(output.stdout).unwrap()).unwrap();
    assert_eq!(entries.len(), super::MAX_STAGED_IGNORE_FILES);
    let runner = CountingGitRunner {
        inner: unica_process::runner::system_process_runner(),
        commands: RefCell::new(Vec::new()),
    };

    let materialized = materialize_staged_ignore_files(
        &runner,
        repository.path(),
        &entries,
        &[],
        &StagedBlobReadSafety::LocalOnlyGuaranteed,
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(30)),
    )
    .unwrap();

    assert_eq!(
        fs::read(materialized.path().join("policy/0000/.gitignore")).unwrap(),
        b"ignored-0000\n"
    );
    assert_eq!(
        fs::read(materialized.path().join("policy/1023/.gitignore")).unwrap(),
        b"ignored-1023\n"
    );
    let commands = runner.commands.borrow();
    assert_eq!(
        commands.len(),
        2,
        "one child per blob exceeds the check budget"
    );
    assert!(commands[0]
        .iter()
        .any(|arg| arg.starts_with("--batch-check=")));
    assert!(commands[1].iter().any(|arg| arg == "--batch"));
}

#[test]
fn project_health_git_ignore_batch_reuses_duplicate_oid_but_charges_each_path() {
    let repository = TempDir::new().unwrap();
    let oid = "a".repeat(40);
    let entries = [
        GitIndexEntry {
            repo_path: "first/.gitignore".into(),
            blob_oid: Some(oid.clone()),
            mode: Some("100644".into()),
        },
        GitIndexEntry {
            repo_path: "second/.gitignore".into(),
            blob_oid: Some(oid.clone()),
            mode: Some("100644".into()),
        },
    ];
    let runner = SequenceRunner::outputs(vec![
        process_output(true, &format!("{oid} blob 4\n")),
        process_output(true, &format!("{oid} blob 4\none\n\n")),
    ]);
    let materialized = materialize_staged_ignore_files(
        &runner,
        repository.path(),
        &entries,
        &[],
        &StagedBlobReadSafety::LocalOnlyGuaranteed,
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(7)),
    )
    .unwrap();
    for path in ["first/.gitignore", "second/.gitignore"] {
        assert_eq!(fs::read(materialized.path().join(path)).unwrap(), b"one\n");
    }
    assert_eq!(runner.commands.borrow().len(), 2);

    let too_large = SequenceRunner::outputs(vec![process_output(
        true,
        &format!("{oid} blob {}\n", 5 * 1024 * 1024),
    )]);
    let error = materialize_staged_ignore_files(
        &too_large,
        repository.path(),
        &entries,
        &[],
        &StagedBlobReadSafety::LocalOnlyGuaranteed,
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(7)),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        StagedIgnoreMaterializationError::Incomplete(ref reason)
            if reason.contains("exceeds 8388608 bytes")
    ));
    assert_eq!(too_large.commands.borrow().len(), 1);
}

#[test]
fn project_health_git_ignore_batch_rejects_wrong_oid_type_size_and_framing() {
    let repository = TempDir::new().unwrap();
    let oid = "a".repeat(40);
    let files = [(".gitignore", oid.as_str())];
    let cases = [
        (
            format!("{} blob 4\n", "b".repeat(40)),
            None,
            "wrong staged .gitignore blob object id",
        ),
        (
            format!("{oid} tree 4\n"),
            None,
            "invalid staged .gitignore blob metadata",
        ),
        (
            format!("{oid} blob unknown\n"),
            None,
            "invalid staged .gitignore blob size",
        ),
        (
            format!("{oid} blob 4\n"),
            Some(format!("{} blob 4\none\n\n", "b".repeat(40))),
            "invalid staged .gitignore batch framing",
        ),
        (
            format!("{oid} blob 4\n"),
            Some(format!("{oid} blob 5\none\n\n")),
            "invalid staged .gitignore batch framing",
        ),
        (
            format!("{oid} blob 4\n"),
            Some(format!("{oid} blob 4\none\n")),
            "incomplete staged .gitignore blob bytes",
        ),
        (
            format!("{oid} blob 4\n"),
            Some(format!("{oid} blob 4\none\n\nextra")),
            "unexpected trailing staged .gitignore blob bytes",
        ),
    ];
    for (metadata, blob_response, expected_reason) in cases {
        let mut outputs = vec![process_output(true, &metadata)];
        if let Some(response) = blob_response {
            outputs.push(process_output(true, &response));
        }
        let runner = SequenceRunner::outputs(outputs);
        let error = super::read_staged_ignore_blobs(
            &runner,
            repository.path(),
            &files,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(7)),
        )
        .unwrap_err();
        assert!(
            matches!(
                error,
                StagedIgnoreMaterializationError::Incomplete(ref reason)
                    if reason.contains(expected_reason)
            ),
            "{error:?}"
        );
    }
}

#[test]
fn project_health_git_ignore_batch_rejects_invalid_utf8_without_per_blob_retry() {
    let repository = TempDir::new().unwrap();
    let oid = "a".repeat(40);
    let mut binary_output = process_output(true, &format!("{oid} blob 1\n�\n"));
    binary_output.stdout_had_invalid_utf8 = true;
    let runner = SequenceRunner::outputs(vec![
        process_output(true, &format!("{oid} blob 1\n")),
        binary_output,
    ]);
    let error = super::read_staged_ignore_blobs(
        &runner,
        repository.path(),
        &[(".gitignore", oid.as_str())],
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(7)),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        StagedIgnoreMaterializationError::Incomplete(ref reason)
            if reason.contains("invalid UTF-8")
    ));
    assert_eq!(runner.commands.borrow().len(), 2);
}

#[test]
fn project_health_git_ignore_batch_rejects_truncated_output_without_per_blob_retry() {
    let repository = TempDir::new().unwrap();
    let oid = "a".repeat(40);
    let mut truncated = process_output(true, &format!("{oid} blob 4\none"));
    truncated.stdout_truncated = true;
    let runner = SequenceRunner::outputs(vec![
        process_output(true, &format!("{oid} blob 4\n")),
        truncated,
    ]);
    let error = super::read_staged_ignore_blobs(
        &runner,
        repository.path(),
        &[(".gitignore", oid.as_str())],
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(7)),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        StagedIgnoreMaterializationError::Incomplete(ref reason)
            if reason.contains("stdout was truncated")
    ));
    assert_eq!(runner.commands.borrow().len(), 2);
}

#[test]
fn project_health_git_ignore_rejects_host_identity_collisions() {
    let repository = TempDir::new().unwrap();
    let repository_identity = normalize_path_identity(repository.path()).unwrap();
    if unica_workspace::paths::host_filesystem_case_sensitive(&repository_identity).unwrap() {
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::CaseInsensitiveFilesystem,
            "identity collisions exist only on a case-insensitive volume",
        );
        return;
    }
    let runner = SequenceRunner::outputs(Vec::new());
    let entries = [
        GitIndexEntry {
            repo_path: "src/A/.gitignore".into(),
            blob_oid: Some("a".repeat(40)),
            mode: Some("100644".into()),
        },
        GitIndexEntry {
            repo_path: "src/a/.gitignore".into(),
            blob_oid: Some("b".repeat(40)),
            mode: Some("100644".into()),
        },
    ];

    let error = materialize_staged_ignore_files(
        &runner,
        repository.path(),
        &entries,
        &[],
        &StagedBlobReadSafety::LocalOnlyGuaranteed,
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(1)),
    )
    .unwrap_err();

    assert_eq!(
        error,
        StagedIgnoreMaterializationError::PathIdentityCollision {
            first: MaterializedGitPath {
                path: "src/A/.gitignore".into(),
                role: "staged ignore policy",
            },
            second: MaterializedGitPath {
                path: "src/a/.gitignore".into(),
                role: "staged ignore policy",
            },
        }
    );
    assert!(runner.commands.borrow().is_empty());
}

#[test]
fn project_health_git_ignore_bounds_staged_policy_file_count() {
    let repository = TempDir::new().unwrap();
    let runner = SequenceRunner::outputs(Vec::new());
    let entries = (0..=super::MAX_STAGED_IGNORE_FILES)
        .map(|index| GitIndexEntry {
            repo_path: format!("nested/{index}/.gitignore"),
            blob_oid: Some(format!("{index:040x}")),
            mode: Some("100644".into()),
        })
        .collect::<Vec<_>>();

    let error = materialize_staged_ignore_files(
        &runner,
        repository.path(),
        &entries,
        &[],
        &StagedBlobReadSafety::LocalOnlyGuaranteed,
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(1)),
    )
    .unwrap_err();

    assert!(
        matches!(
            error,
            StagedIgnoreMaterializationError::Incomplete(ref reason)
                if reason.contains("at most 1024")
        ),
        "{error:?}"
    );
    assert!(runner.commands.borrow().is_empty());
}

#[test]
fn project_health_git_ignore_bounds_total_staged_policy_bytes() {
    let repository = TempDir::new().unwrap();
    let oid = "a".repeat(40);
    let runner = SequenceRunner::outputs(vec![process_output(
        true,
        &format!("{oid} blob {}\n", super::MAX_STAGED_IGNORE_TOTAL_BYTES + 1),
    )]);

    let error = materialize_staged_ignore_files(
        &runner,
        repository.path(),
        &[GitIndexEntry {
            repo_path: ".gitignore".into(),
            blob_oid: Some(oid),
            mode: Some("100644".into()),
        }],
        &[],
        &StagedBlobReadSafety::LocalOnlyGuaranteed,
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(1)),
    )
    .unwrap_err();

    assert!(
        matches!(
            error,
            StagedIgnoreMaterializationError::Incomplete(ref reason)
                if reason.contains("exceeds 8388608 bytes")
        ),
        "{error:?}"
    );
    assert_eq!(
        runner.commands.borrow()[0].capture_limits,
        Some((
            super::MAX_STAGED_IGNORE_BATCH_METADATA_BYTES + 1,
            256 * 1024
        ))
    );
    assert_eq!(runner.commands.borrow().len(), 1);
}

#[test]
fn project_health_git_removes_every_inherited_git_variable() {
    assert_eq!(
        git_environment_removals_from([
            (OsString::from("PATH"), OsString::from("/bin")),
            (OsString::from("GIT_DIR"), OsString::from("elsewhere")),
            (
                OsString::from("GIT_INDEX_FILE"),
                OsString::from("alternate-index"),
            ),
            (
                OsString::from("GIT_OBJECT_DIRECTORY"),
                OsString::from("alternate-objects"),
            ),
            (OsString::from("GIT_FUTURE_OVERRIDE"), OsString::from("1")),
            (OsString::from("git_dir"), OsString::from("mixed-case")),
        ]),
        vec![
            OsString::from("GIT_DIR"),
            OsString::from("GIT_FUTURE_OVERRIDE"),
            OsString::from("GIT_INDEX_FILE"),
            OsString::from("GIT_OBJECT_DIRECTORY"),
            OsString::from("git_dir"),
        ]
    );
}

#[test]
fn project_health_git_commands_use_large_but_bounded_capture() {
    let command = super::process_command(
        Path::new("."),
        ["status"],
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(1)),
    );

    assert_eq!(
        command.capture_limits,
        Some((
            super::super::PROJECT_HEALTH_STDOUT_CAPTURE_LIMIT,
            256 * 1024
        ))
    );
    assert_eq!(
        super::super::PROJECT_HEALTH_STDOUT_CAPTURE_LIMIT,
        64 * 1024 * 1024
    );
    assert!(
        command
            .env
            .iter()
            .any(|(name, value)| { name == "GIT_NO_LAZY_FETCH" && value == "1" }),
        "{:?}",
        command.env
    );
    for (name, value) in [
        ("GIT_NO_REPLACE_OBJECTS", "1"),
        ("GIT_TERMINAL_PROMPT", "0"),
    ] {
        assert!(
            command
                .env
                .iter()
                .any(|(actual_name, actual_value)| actual_name == name && actual_value == value),
            "{name}: {:?}",
            command.env
        );
    }
    assert!(
        command
            .env
            .iter()
            .any(|(name, value)| { name == "GIT_OPTIONAL_LOCKS" && value == "0" }),
        "{:?}",
        command.env
    );
}

#[test]
fn project_health_git_index_parser_rejects_malformed_nul_record() {
    assert!(parse_git_index_entries("100644 oid 0 without-tab\0").is_err());
    assert!(parse_git_index_entries("100644 abc 0\tpath-without-terminal-nul").is_err());
}

#[test]
fn project_health_git_ignore_parser_reads_verbose_z_quadruples() {
    assert_eq!(
        parse_check_ignore_verbose_z(concat!(
            ".gitignore\0",
            "007\0",
            "**/.build/\0",
            "workspace/src/.build/probe\0"
        ))
        .unwrap(),
        vec![IgnoreMatch {
            source: ".gitignore".into(),
            line: "007".into(),
            pattern: "**/.build/".into(),
            path: "workspace/src/.build/probe".into(),
        }]
    );
    assert!(parse_check_ignore_verbose_z(concat!(".gitignore\0", "1\0", "pattern\0")).is_err());
}

#[test]
fn project_health_git_ignore_parser_honors_cooperative_checkpoint() {
    let mut checkpoints = 0;
    let error = super::parse_check_ignore_verbose_z_with_checkpoint(
        concat!(
            ".gitignore\0",
            "1\0",
            "**/.build/\0",
            ".build/unica/probe\0"
        ),
        &mut || {
            checkpoints += 1;
            if checkpoints == 2 {
                Err(super::IgnoreInspectionError::TimedOut)
            } else {
                Ok(())
            }
        },
    )
    .unwrap_err();

    assert!(matches!(error, super::IgnoreInspectionError::TimedOut));
}

#[test]
fn project_health_git_ignore_fact_composition_is_all_or_nothing() {
    let candidates = vec![super::IgnoreCandidate {
        source_set: Some("main".into()),
        repo_path: "src/.build/.unica-health-probe".into(),
    }];
    let entries = vec![GitIndexEntry {
        repo_path: ".gitignore".into(),
        blob_oid: Some("a".repeat(40)),
        mode: Some("100644".into()),
    }];

    let result = super::ignore_facts_with_checkpoint(
        &candidates,
        &[],
        &[],
        &entries,
        Path::new("/staged"),
        &mut || Err(super::IgnoreInspectionError::TimedOut),
    );

    assert!(matches!(
        result,
        Err(super::IgnoreInspectionError::TimedOut)
    ));
}

#[test]
fn git_no_match_requires_exact_exit_code_one() {
    let output = |status: &str| ProcessOutput {
        status: status.into(),
        ..process_output(false, "")
    };

    assert!(super::status_is_no_match(&output("exit status: 1")));
    for status in ["exit status: 128", "exit code: 10", "signal: 1 (SIGHUP)"] {
        assert!(!super::status_is_no_match(&output(status)), "{status}");
    }
}

#[test]
fn project_health_git_missing_executable_is_typed_and_stops_discovery() {
    let fixture = health_fixture(false);
    let runner = SequenceRunner::errors(vec![
        "process_failed: No such file or directory (os error 2)",
    ]);
    let layout = inspect_layout(&fixture.context);

    let inspection = GitRepositoryInspector::with_runner(&runner)
        .inspect_base(
            &fixture.context,
            &layout,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(1)),
        )
        .unwrap();

    assert_eq!(runner.commands.borrow().len(), 1);
    assert!(matches!(
        inspection.facts.as_slice(),
        [ProjectHealthFact::GitExecutableUnavailable { reason }]
            if reason.contains("No such file")
    ));
    assert!(inspection.observations.iter().any(|observation| {
        observation.id == ProjectCheckId::RepositoryIndex
            && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
    }));
}

#[test]
fn project_health_git_non_not_found_spawn_error_is_inspection_incomplete() {
    let fixture = health_fixture(false);
    let runner = SequenceRunner::errors(vec!["process_failed: Permission denied (os error 13)"]);
    let layout = inspect_layout(&fixture.context);

    let inspection = GitRepositoryInspector::with_runner(&runner)
        .inspect_base(
            &fixture.context,
            &layout,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(1)),
        )
        .unwrap();

    assert!(matches!(
        inspection.facts.as_slice(),
        [ProjectHealthFact::GitInspectionIncomplete {
            check: ProjectCheckId::RepositoryDiscovery,
            reason,
            ..
        }] if reason.contains("Permission denied")
    ));
}

#[test]
fn project_health_git_index_error_after_cancellation_cancels_inspection() {
    let fixture = health_fixture(false);
    let cancellation = CancellationToken::new();
    let runner = LateCancellingIndexRunner {
        repository_root: fixture.context.workspace_root.clone(),
        cancellation: cancellation.clone(),
        calls: Cell::new(0),
    };
    let layout = inspect_layout(&fixture.context);

    let result = GitRepositoryInspector::with_runner(&runner).inspect_base(
        &fixture.context,
        &layout,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(1)),
    );

    assert!(matches!(
        result,
        Err(ProjectHealthInspectionError::Cancelled)
    ));
}

#[test]
fn project_health_git_error_after_cancellation_is_cancelled() {
    let fixture = health_fixture(false);
    let cancellation = CancellationToken::new();
    let runner = CancellingRunner {
        cancellation: cancellation.clone(),
    };
    let layout = inspect_layout(&fixture.context);

    let result = GitRepositoryInspector::with_runner(&runner).inspect_base(
        &fixture.context,
        &layout,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(1)),
    );

    assert!(matches!(
        result,
        Err(crate::ProjectHealthInspectionError::Cancelled)
    ));
}

#[test]
fn project_health_git_absent_repository_is_typed_and_stops_after_discovery() {
    let fixture = health_fixture(false);
    let layout = inspect_layout(&fixture.context);

    let inspection = GitRepositoryInspector::new()
        .inspect_base(
            &fixture.context,
            &layout,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(2)),
        )
        .unwrap();

    assert!(matches!(
        inspection.facts.as_slice(),
        [ProjectHealthFact::GitRepositoryAbsent]
    ));
    assert!(inspection.repository_root.is_none());
    assert!(inspection.observations.iter().any(|observation| {
        observation.id == ProjectCheckId::RepositoryIndex
            && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
    }));
}

#[test]
fn project_health_git_literal_replacement_character_is_not_lossy() {
    let fixture = health_fixture(false);
    let runner = SequenceRunner::outputs(vec![
        ProcessOutput {
            status_success: true,
            status: "exit status: 0".into(),
            stdout: format!("{}\ntrue\n", fixture.context.workspace_root.display()),
            stderr: "valid \u{fffd}".into(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            stdout_had_invalid_utf8: false,
            stderr_had_invalid_utf8: false,
        },
        ProcessOutput {
            status_success: true,
            status: "exit status: 0".into(),
            stdout: String::new(),
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            stdout_had_invalid_utf8: false,
            stderr_had_invalid_utf8: false,
        },
        ProcessOutput {
            status: "exit status: 1".into(),
            ..process_output(false, "")
        },
        ProcessOutput {
            status_success: true,
            status: "exit status: 0".into(),
            stdout: String::new(),
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            stdout_had_invalid_utf8: false,
            stderr_had_invalid_utf8: false,
        },
        ProcessOutput {
            status: "exit status: 1".into(),
            ..process_output(false, "")
        },
    ]);
    let layout = inspect_layout(&fixture.context);

    let inspection = GitRepositoryInspector::with_runner(&runner)
        .inspect_base(
            &fixture.context,
            &layout,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(1)),
        )
        .unwrap();

    assert!(!inspection.facts.iter().any(|fact| matches!(
        fact,
        ProjectHealthFact::GitInspectionIncomplete {
            check: ProjectCheckId::RepositoryDiscovery,
            ..
        }
    )));
}

#[test]
fn project_health_git_config_dump_timeout_is_incomplete_not_unclassified() {
    let fixture = health_fixture(false);
    let oid = "a".repeat(40);
    let runner = SequenceRunner::outputs(vec![
        process_output(
            true,
            &format!("{}\ntrue\n", fixture.context.workspace_root.display()),
        ),
        process_output(true, &format!("100644 {oid} 0\tsrc/ConfigDumpInfo.xml\0")),
        ProcessOutput {
            status: "exit status: 1".into(),
            ..process_output(false, "")
        },
        ProcessOutput {
            timed_out: true,
            ..process_output(false, "")
        },
        ProcessOutput {
            status: "exit status: 1".into(),
            ..process_output(false, "")
        },
        ProcessOutput {
            status: "exit status: 1".into(),
            ..process_output(false, "")
        },
    ]);
    let layout = inspect_layout(&fixture.context);

    let inspection = GitRepositoryInspector::with_runner(&runner)
        .inspect_base(
            &fixture.context,
            &layout,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(1)),
        )
        .unwrap();

    assert!(inspection.observations.iter().any(|observation| {
        observation.id == ProjectCheckId::RepositoryConfigDumpInfo
            && observation.source_set.is_none()
            && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
    }));
    assert!(inspection.observations.iter().any(|observation| {
        observation.id == ProjectCheckId::RepositoryConfigDumpInfo
            && observation.source_set.as_deref() == Some("main")
            && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
    }));
    assert!(inspection.facts.iter().any(|fact| matches!(
        fact,
        ProjectHealthFact::GitInspectionTimeout {
            check: ProjectCheckId::RepositoryConfigDumpInfo,
            ..
        }
    )));
    assert!(!inspection.facts.iter().any(|fact| matches!(
        fact,
        ProjectHealthFact::ConfigDumpInfoUnclassified { path, .. }
            if path == "src/ConfigDumpInfo.xml"
    )));
}

#[test]
fn project_health_old_git_partial_clone_never_reads_staged_blobs() {
    let fixture = health_fixture(false);
    let oid = "a".repeat(40);
    let no_match = || ProcessOutput {
        status: "exit status: 1".into(),
        ..process_output(false, "")
    };
    let runner = SequenceRunner::outputs(vec![
        process_output(
            true,
            &format!("{}\ntrue\n", fixture.context.workspace_root.display()),
        ),
        process_output(true, &format!("100644 {oid} 0\tsrc/ConfigDumpInfo.xml\0")),
        process_output(true, "remote.origin.promisor\ntrue\0"),
        process_output(true, "git version 2.40.1\n"),
        no_match(),
        no_match(),
    ]);
    let layout = inspect_layout(&fixture.context);

    let inspection = GitRepositoryInspector::with_runner(&runner)
        .inspect_base(
            &fixture.context,
            &layout,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(1)),
        )
        .unwrap();

    assert!(inspection.observations.iter().any(|observation| {
        observation.id == ProjectCheckId::RepositoryConfigDumpInfo
            && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
    }));
    assert!(inspection.facts.iter().any(|fact| matches!(
        fact,
        ProjectHealthFact::GitInspectionIncomplete {
            check: ProjectCheckId::RepositoryConfigDumpInfo,
            reason,
            ..
        } if reason.contains("Git 2.40.1") && reason.contains("partial clone")
    )));
    assert!(
        runner
            .commands
            .borrow()
            .iter()
            .all(|command| { !command.args.iter().any(|argument| argument == "cat-file") }),
        "{:?}",
        runner.commands.borrow()
    );
    let config_command = &runner.commands.borrow()[2];
    assert!(!config_command.args.iter().any(|arg| arg == "--local"));
}

#[test]
fn project_health_git_parent_repository_uses_tracked_portable_ignore() {
    let fixture = parent_repository_fixture();
    fs::write(
        fixture.repository_root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    git(&fixture.repository_root, &["add", ".gitignore"]);
    let layout = inspect_layout(&fixture.context);

    let inspection = GitRepositoryInspector::new()
        .inspect_base(
            &fixture.context,
            &layout,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        )
        .unwrap();

    assert_eq!(
        inspection.repository_root.as_deref(),
        Some(
            unica_workspace::paths::normalize_path_identity(&fixture.repository_root)
                .unwrap()
                .as_path()
        )
    );
    assert!(
        !inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::IgnoreRuleMissing { .. }
                | ProjectHealthFact::IgnoreRuleLocalOnly { .. }
        )),
        "{:?}",
        inspection.facts
    );
    assert!(inspection.observations.iter().all(|observation| {
        observation.id != ProjectCheckId::RepositoryIgnore
            || matches!(observation.outcome, ProjectCheckOutcome::Completed)
    }));
}

#[test]
#[cfg_attr(
    windows,
    ignore = "host capability: Windows file names cannot contain a newline"
)]
fn project_health_git_discovers_repository_root_containing_newline() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("repo\nname");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/Configuration.xml"), "<MetaDataObject/>\n").unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    git(&root, &["init"]);
    let context = WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".build/unica"),
        workspace_epoch: 1,
    };
    let layout = inspect_layout(&context);

    let inspection = GitRepositoryInspector::new()
        .inspect_base(
            &context,
            &layout,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        )
        .unwrap();

    assert_eq!(
        inspection.repository_root,
        Some(fs::canonicalize(&root).unwrap())
    );

    fs::write(
        root.join(".gitattributes"),
        "*.xml text eol=lf\n*.bsl text eol=lf\n*.bin -text\n",
    )
    .unwrap();
    git(&root, &["add", ".gitattributes", "src/Configuration.xml"]);
    let inspection = crate::inspection::resources::SourceResourcePolicyInspector::new()
        .inspect(
            inspection.repository_root.as_ref().unwrap(),
            &layout.roots,
            &GitRepositoryInspector::new()
                .inspect_base(
                    &context,
                    &layout,
                    &CancellationToken::new(),
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                )
                .unwrap()
                .entries,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        )
        .unwrap();
    assert!(
        inspection.observations.iter().all(|observation| {
            observation.id != ProjectCheckId::RepositoryAttributes
                || matches!(observation.outcome, ProjectCheckOutcome::Completed)
        }),
        "{:?}",
        inspection.facts
    );
}

#[test]
fn project_health_git_info_exclude_is_local_only() {
    let fixture = parent_repository_fixture();
    fs::write(
        fixture.repository_root.join(".git/info/exclude"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    let layout = inspect_layout(&fixture.context);

    let inspection = GitRepositoryInspector::new()
        .inspect_base(
            &fixture.context,
            &layout,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        )
        .unwrap();

    assert!(
        inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::IgnoreRuleLocalOnly { origin, .. }
                if origin.contains("exclude")
        )),
        "{:?}",
        inspection.facts
    );
}

#[test]
fn project_health_git_ignore_uses_staged_empty_file_not_valid_worktree_file() {
    let fixture = parent_repository_fixture();
    fs::write(fixture.repository_root.join(".gitignore"), "").unwrap();
    git(&fixture.repository_root, &["add", ".gitignore"]);
    fs::write(
        fixture.repository_root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    let layout = inspect_layout(&fixture.context);

    let inspection = GitRepositoryInspector::new()
        .inspect_base(
            &fixture.context,
            &layout,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        )
        .unwrap();

    assert!(
        inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::IgnoreRuleLocalOnly { path, .. }
                if path == "workspace/src/ConfigDumpInfo.xml"
        )),
        "{:?}",
        inspection.facts
    );
}

#[test]
fn project_health_git_global_excludes_cannot_impersonate_staged_gitignore() {
    let fixture = parent_repository_fixture();
    fs::write(fixture.repository_root.join(".gitignore"), "").unwrap();
    git(&fixture.repository_root, &["add", ".gitignore"]);
    fs::write(
        fixture.repository_root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    let excludes = fixture.repository_root.join(".gitignore");
    git(
        &fixture.repository_root,
        &["config", "core.excludesFile", excludes.to_str().unwrap()],
    );
    let layout = inspect_layout(&fixture.context);

    let inspection = GitRepositoryInspector::new()
        .inspect_base(
            &fixture.context,
            &layout,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        )
        .unwrap();

    assert!(
        inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::IgnoreRuleLocalOnly { path, .. }
                if path == "workspace/src/ConfigDumpInfo.xml"
        )),
        "{:?}",
        inspection.facts
    );
}

#[test]
fn project_health_git_duplicate_local_ignore_does_not_mask_portable_staged_rule() {
    let fixture = parent_repository_fixture();
    fs::write(
        fixture.repository_root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    git(&fixture.repository_root, &["add", ".gitignore"]);
    fs::write(
        fixture.repository_root.join(".git/info/exclude"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    let layout = inspect_layout(&fixture.context);

    let inspection = GitRepositoryInspector::new()
        .inspect_base(
            &fixture.context,
            &layout,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        )
        .unwrap();

    assert!(
        !inspection
            .facts
            .iter()
            .any(|fact| matches!(fact, ProjectHealthFact::IgnoreRuleLocalOnly { .. })),
        "{:?}",
        inspection.facts
    );
}

#[test]
fn project_health_git_ignore_uses_valid_staged_file_not_empty_worktree_file() {
    let fixture = parent_repository_fixture();
    fs::write(
        fixture.repository_root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    git(&fixture.repository_root, &["add", ".gitignore"]);
    fs::write(fixture.repository_root.join(".gitignore"), "").unwrap();
    let layout = inspect_layout(&fixture.context);

    let inspection = GitRepositoryInspector::new()
        .inspect_base(
            &fixture.context,
            &layout,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        )
        .unwrap();

    assert!(
        !inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::IgnoreRuleMissing { path, .. }
                | ProjectHealthFact::IgnoreRuleLocalOnly { path, .. }
                if path == "workspace/src/ConfigDumpInfo.xml"
        )),
        "{:?}",
        inspection.facts
    );
}

#[test]
fn project_health_staged_ignore_read_does_not_run_smudge_filter() {
    let fixture = parent_repository_fixture();
    fs::write(
        fixture.repository_root.join(".gitattributes"),
        ".gitignore filter=project-health-probe\n",
    )
    .unwrap();
    fs::write(
        fixture.repository_root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    git(
        &fixture.repository_root,
        &[
            "config",
            "filter.project-health-probe.smudge",
            "unica-project-health-smudge-command-must-not-run",
        ],
    );
    git(
        &fixture.repository_root,
        &["add", ".gitattributes", ".gitignore"],
    );
    let layout = inspect_layout(&fixture.context);

    let inspection = GitRepositoryInspector::new()
        .inspect_base(
            &fixture.context,
            &layout,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        )
        .unwrap();

    assert!(
        !inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::IgnoreRuleMissing { .. }
                | ProjectHealthFact::IgnoreRuleLocalOnly { .. }
        )),
        "{:?}",
        inspection.facts
    );
}

#[test]
fn project_health_git_untracked_gitignore_is_local_only() {
    let fixture = parent_repository_fixture();
    fs::write(
        fixture.repository_root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    let layout = inspect_layout(&fixture.context);

    let inspection = GitRepositoryInspector::new()
        .inspect_base(
            &fixture.context,
            &layout,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        )
        .unwrap();

    assert!(
        inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::IgnoreRuleLocalOnly { origin, .. }
                if origin.contains(".gitignore")
        )),
        "{:?}",
        inspection.facts
    );
}

#[test]
fn project_health_git_missing_ignore_rules_do_not_create_probe_files() {
    let fixture = parent_repository_fixture();
    let layout = inspect_layout(&fixture.context);

    let inspection = GitRepositoryInspector::new()
        .inspect_base(
            &fixture.context,
            &layout,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        )
        .unwrap();

    assert!(
        inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::IgnoreRuleMissing { path, .. }
                if path == "workspace/src/ConfigDumpInfo.xml"
        )),
        "{:?}",
        inspection.facts
    );
    assert!(!fixture
        .workspace_root
        .join("src/ConfigDumpInfo.xml")
        .exists());
    assert!(!fixture
        .workspace_root
        .join("src/.build/.unica-health-probe")
        .exists());
}

#[test]
fn unknown_and_invalid_source_formats_do_not_pass_format_dependent_ignore() {
    for configured_format in [None, Some("UNSUPPORTED")] {
        let fixture = health_fixture(true);
        fs::remove_file(fixture.context.workspace_root.join("src/Configuration.xml")).unwrap();
        let format = configured_format
            .map(|value| format!("format: {value}\n"))
            .unwrap_or_default();
        fs::write(
            fixture.context.workspace_root.join("v8project.yaml"),
            format!(
                "{format}source-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n"
            ),
        )
        .unwrap();
        fs::write(
            fixture
                .context
                .workspace_root
                .join("src/ConfigDumpInfo.xml"),
            "<ConfigDumpInfo/>",
        )
        .unwrap();
        git(
            &fixture.context.workspace_root,
            &["add", "src/ConfigDumpInfo.xml"],
        );
        let layout = inspect_layout(&fixture.context);
        assert!(matches!(
            layout.roots[0].source_set.source_format,
            SourceFormat::Unknown | SourceFormat::Invalid
        ));

        let inspection = GitRepositoryInspector::new()
            .inspect_base(
                &fixture.context,
                &layout,
                &CancellationToken::new(),
                ProviderDeadline::from_budget(Duration::from_secs(5)),
            )
            .unwrap();

        assert!(
            inspection.observations.iter().any(|observation| {
                observation.id == ProjectCheckId::RepositoryIgnore
                    && observation.source_set.as_deref() == Some("main")
                    && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
            }),
            "format={configured_format:?}; observations={:?}",
            inspection.observations
        );
        assert!(
            !inspection.facts.iter().any(|fact| matches!(
                fact,
                ProjectHealthFact::IgnoreRuleMissing { path, .. }
                    | ProjectHealthFact::IgnoreRuleLocalOnly { path, .. }
                    if path == "src/ConfigDumpInfo.xml"
            )),
            "format={configured_format:?}; facts={:?}",
            inspection.facts
        );
        let mut observations = layout
            .observations
            .iter()
            .cloned()
            .chain(inspection.observations.iter().cloned())
            .collect::<Vec<_>>();
        observations.extend(crate::inspection::resources::resource_observations(
            layout.source_sets.iter().flatten(),
            "source-set targets are incomplete",
            layout.source_targets_complete,
        ));
        let snapshot = crate::ProjectHealthSnapshot {
            workspace_root: fixture.context.workspace_root.display().to_string(),
            cache_root: fixture.context.cache_root.display().to_string(),
            repository_root: inspection
                .repository_root
                .as_ref()
                .map(|path| path.display().to_string()),
            source_sets: layout.source_sets.clone(),
            source_targets_complete: layout.source_targets_complete,
            observations,
            facts: layout
                .facts
                .iter()
                .cloned()
                .chain(inspection.facts.iter().cloned())
                .collect(),
        };
        assert!(
            crate::evaluate_project_health(snapshot).is_ok(),
            "format={configured_format:?}; facts={:?}",
            inspection.facts
        );
    }
}

#[test]
fn incomplete_format_does_not_suppress_ignore_for_a_proven_sibling_set() {
    let fixture = health_fixture(true);
    fs::create_dir_all(fixture.context.workspace_root.join("unknown")).unwrap();
    fs::write(
        fixture.context.workspace_root.join("v8project.yaml"),
        "source-set:\n  - name: designer\n    type: CONFIGURATION\n    path: src\n  - name: unknown\n    type: CONFIGURATION\n    path: unknown\n",
    )
    .unwrap();
    fs::write(
        fixture.context.workspace_root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    git(&fixture.context.workspace_root, &["add", ".gitignore"]);
    let layout = inspect_layout(&fixture.context);

    let inspection = GitRepositoryInspector::new()
        .inspect_base(
            &fixture.context,
            &layout,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        )
        .unwrap();

    assert!(
        inspection.observations.iter().any(|observation| {
            observation.id == ProjectCheckId::RepositoryIgnore
                && observation.source_set.as_deref() == Some("designer")
                && matches!(observation.outcome, ProjectCheckOutcome::Completed)
        }),
        "{:?}",
        inspection.observations
    );
    assert!(
        inspection.observations.iter().any(|observation| {
            observation.id == ProjectCheckId::RepositoryIgnore
                && observation.source_set.as_deref() == Some("unknown")
                && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
        }),
        "{:?}",
        inspection.observations
    );
    assert!(
        inspection.observations.iter().any(|observation| {
            observation.id == ProjectCheckId::RepositoryIgnore
                && observation.source_set.is_none()
                && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
        }),
        "{:?}",
        inspection.observations
    );
}

#[test]
fn project_health_working_ignore_timeout_is_not_reported_as_a_missing_rule() {
    let fixture = health_fixture(true);
    let no_match = || ProcessOutput {
        status: "exit status: 1".into(),
        ..process_output(false, "")
    };
    let runner = SequenceRunner::outputs(vec![
        process_output(
            true,
            &format!("{}\ntrue\n", fixture.context.workspace_root.display()),
        ),
        process_output(true, ""),
        no_match(),
        no_match(),
        ProcessOutput {
            timed_out: true,
            ..process_output(false, "")
        },
    ]);
    let layout = inspect_layout(&fixture.context);

    let inspection = GitRepositoryInspector::with_runner(&runner)
        .inspect_base(
            &fixture.context,
            &layout,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(1)),
        )
        .unwrap();

    assert!(inspection
        .observations
        .iter()
        .filter(|observation| observation.id == ProjectCheckId::RepositoryIgnore)
        .all(|observation| matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })));
    assert!(inspection.facts.iter().any(|fact| matches!(
        fact,
        ProjectHealthFact::GitInspectionTimeout {
            check: ProjectCheckId::RepositoryIgnore,
            source_set: None,
        }
    )));
    assert!(!inspection.facts.iter().any(|fact| matches!(
        fact,
        ProjectHealthFact::IgnoreRuleMissing { .. } | ProjectHealthFact::IgnoreRuleLocalOnly { .. }
    )));
}

#[test]
fn project_health_git_reports_missing_ignore_for_every_equal_root_owner() {
    let fixture = health_fixture(true);
    fs::write(
        fixture.context.workspace_root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: first\n    type: CONFIGURATION\n    path: src\n  - name: second\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    let layout = inspect_layout(&fixture.context);

    let inspection = GitRepositoryInspector::new()
        .inspect_base(
            &fixture.context,
            &layout,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        )
        .unwrap();

    for owner in ["first", "second"] {
        assert!(
            inspection.facts.iter().any(|fact| matches!(
                fact,
                ProjectHealthFact::IgnoreRuleMissing {
                    source_set: Some(source_set),
                    path,
                } if source_set == owner && path == "src/ConfigDumpInfo.xml"
            )),
            "owner={owner}; facts={:?}",
            inspection.facts
        );
    }
}

#[test]
fn project_health_git_runtime_sidecar_and_legitimate_descriptor_are_distinct() {
    let fixture = parent_repository_fixture();
    fs::write(
        fixture.workspace_root.join("src/ConfigDumpInfo.xml"),
        "<ConfigDumpInfo/>",
    )
    .unwrap();
    fs::create_dir_all(fixture.workspace_root.join("epf")).unwrap();
    fs::write(
        fixture.workspace_root.join("epf/ConfigDumpInfo.xml"),
        "<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
    )
    .unwrap();
    git(
        &fixture.repository_root,
        &[
            "add",
            "workspace/src/ConfigDumpInfo.xml",
            "workspace/epf/ConfigDumpInfo.xml",
        ],
    );
    let layout = inspect_layout(&fixture.context);

    let inspection = GitRepositoryInspector::new()
        .inspect_base(
            &fixture.context,
            &layout,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        )
        .unwrap();

    assert!(
        inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::RuntimeSidecarTracked { path, .. }
                if path == "workspace/src/ConfigDumpInfo.xml"
        )),
        "{:?}",
        inspection.facts
    );
    assert!(
        !inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::ConfigDumpInfoUnclassified { path, .. }
                | ProjectHealthFact::RuntimeSidecarTracked { path, .. }
                if path == "workspace/epf/ConfigDumpInfo.xml"
        )),
        "{:?}",
        inspection.facts
    );
}

#[test]
fn project_health_git_tracked_generated_paths_are_independent_of_ignore_matches() {
    let fixture = parent_repository_fixture();
    let generated = fixture.workspace_root.join("src/.build/generated.bin");
    fs::create_dir_all(generated.parent().unwrap()).unwrap();
    fs::write(&generated, b"generated").unwrap();
    git(
        &fixture.repository_root,
        &["add", "-f", "workspace/src/.build/generated.bin"],
    );
    fs::write(
        fixture.repository_root.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    git(&fixture.repository_root, &["add", ".gitignore"]);
    let layout = inspect_layout(&fixture.context);

    let inspection = GitRepositoryInspector::new()
        .inspect_base(
            &fixture.context,
            &layout,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        )
        .unwrap();

    assert!(
        inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::GeneratedPathTracked { path, .. }
                if path == "workspace/src/.build/generated.bin"
        )),
        "{:?}",
        inspection.facts
    );
}

struct SequenceRunner {
    commands: RefCell<Vec<ProcessCommand>>,
    results: RefCell<Vec<Result<ProcessOutput, String>>>,
}

struct CancellingRunner {
    cancellation: CancellationToken,
}

impl ProcessRunner for CancellingRunner {
    fn run(&self, _command: &ProcessCommand) -> Result<ProcessOutput, String> {
        self.cancellation.cancel();
        Err("process failed while cancellation was requested".into())
    }
}

impl SequenceRunner {
    fn outputs(outputs: Vec<ProcessOutput>) -> Self {
        Self {
            commands: RefCell::new(Vec::new()),
            results: RefCell::new(outputs.into_iter().map(Ok).collect()),
        }
    }

    fn errors(errors: Vec<&str>) -> Self {
        Self {
            commands: RefCell::new(Vec::new()),
            results: RefCell::new(errors.into_iter().map(|error| Err(error.into())).collect()),
        }
    }
}

impl ProcessRunner for SequenceRunner {
    fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
        self.commands.borrow_mut().push(command.clone());
        self.results.borrow_mut().remove(0)
    }

    fn run_with_input(
        &self,
        command: &ProcessCommand,
        _input: &[u8],
    ) -> Result<ProcessOutput, String> {
        self.run(command)
    }
}

fn process_output(status_success: bool, stdout: &str) -> ProcessOutput {
    ProcessOutput {
        status_success,
        status: if status_success {
            "exit status: 0".into()
        } else {
            "exit status: 2".into()
        },
        stdout: stdout.into(),
        stderr: String::new(),
        timed_out: false,
        cancelled: false,
        stdout_truncated: false,
        stderr_truncated: false,
        stdout_had_invalid_utf8: false,
        stderr_had_invalid_utf8: false,
    }
}

struct HealthFixture {
    _temp: TempDir,
    context: WorkspaceContext,
}

struct LateCancellingIndexRunner {
    repository_root: PathBuf,
    cancellation: CancellationToken,
    calls: Cell<usize>,
}

impl ProcessRunner for LateCancellingIndexRunner {
    fn run(&self, _command: &ProcessCommand) -> Result<ProcessOutput, String> {
        let call = self.calls.get();
        self.calls.set(call + 1);
        if call == 0 {
            Ok(process_output(
                true,
                &format!("{}\ntrue\n", self.repository_root.display()),
            ))
        } else {
            self.cancellation.cancel();
            Err("index process failed after cancellation".into())
        }
    }

    fn run_with_input(
        &self,
        command: &ProcessCommand,
        _input: &[u8],
    ) -> Result<ProcessOutput, String> {
        self.run(command)
    }
}

fn health_fixture(initialize_git: bool) -> HealthFixture {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/Configuration.xml"), "<MetaDataObject/>").unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    if initialize_git {
        git(&root, &["init"]);
    }
    HealthFixture {
        _temp: temp,
        context: WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        },
    }
}

struct ParentRepositoryFixture {
    _temp: TempDir,
    repository_root: PathBuf,
    workspace_root: PathBuf,
    context: WorkspaceContext,
}

fn parent_repository_fixture() -> ParentRepositoryFixture {
    let temp = TempDir::new().unwrap();
    let repository_root = temp.path().to_path_buf();
    git(&repository_root, &["init"]);
    let workspace_root = repository_root.join("workspace");
    fs::create_dir_all(workspace_root.join("src")).unwrap();
    fs::write(
        workspace_root.join("src/Configuration.xml"),
        "<MetaDataObject/>",
    )
    .unwrap();
    fs::write(
        workspace_root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    ParentRepositoryFixture {
        _temp: temp,
        repository_root: repository_root.clone(),
        workspace_root: workspace_root.clone(),
        context: WorkspaceContext {
            cwd: workspace_root.clone(),
            workspace_root: workspace_root.clone(),
            cache_root: workspace_root.join(".build/unica"),
            workspace_epoch: 1,
        },
    }
}

fn inspect_layout(context: &WorkspaceContext) -> crate::inspection::layout::SourceLayoutInspection {
    SourceLayoutInspector::inspect(
        context,
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(2)),
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&(context).workspace_root),
    )
    .unwrap()
}

fn git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
