use super::SourceLayoutInspector;
use crate::{ProjectCheckId, ProjectCheckOutcome, ProjectHealthFact};
use unica_source_contract::{SourceFormat, SourceSetKind};
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;

use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use unica_source_contract::workspace::WorkspaceContext;
use unica_workspace::test_support::canonical_path;
use unica_workspace::test_support::{
    create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
};

#[test]
fn root_identity_equal_to_workspace_is_one_primary_fact() {
    for configured in [".", "./", "src/.."] {
        let fixture = layout_fixture(configured);
        let inspection = SourceLayoutInspector::inspect(
            &fixture.context,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(1)),
            &unica_workspace::discovery::ProjectDiscoveryReader::new(
                &fixture.context.workspace_root,
            ),
        )
        .unwrap();

        assert_eq!(inspection.facts.len(), 1, "configured path: {configured}");
        let ProjectHealthFact::SourceRootIsWorkspace { evidence, .. } = &inspection.facts[0] else {
            panic!("root-is-workspace fact expected for {configured}");
        };
        assert!(
            evidence.iter().any(|value| value.contains(".build")),
            "{evidence:?}"
        );
        assert!(!inspection.source_targets_complete);
        assert!(inspection.repository_targets_complete);
        assert!(inspection.roots.is_empty());
        assert!(inspection.observations.iter().any(|observation| {
            observation.id == ProjectCheckId::SourceGeneratedPaths
                && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
        }));
    }
}

#[test]
fn malformed_project_config_is_a_typed_incomplete_snapshot() {
    let fixture = raw_layout_fixture("source-set: [\n");

    let inspection = inspect(&fixture);

    assert!(inspection.source_sets.is_none());
    assert!(!inspection.source_targets_complete);
    assert!(inspection.roots.is_empty());
    assert!(matches!(
        inspection.facts.as_slice(),
        [ProjectHealthFact::SourceInspectionIncomplete { reason }]
            if reason.contains("failed to parse")
    ));
}

#[test]
fn oversized_invalid_project_config_is_a_typed_yaml_refusal() {
    let fixture = raw_layout_fixture("");
    let config = fixture.context.workspace_root.join("v8project.yaml");
    fs::File::create(&config)
        .unwrap()
        .set_len(8 * 1024 * 1024 + 1)
        .unwrap();

    let inspection = inspect_without_deadline(&fixture);

    assert!(
        matches!(
            inspection.facts.as_slice(),
            [ProjectHealthFact::SourceInspectionIncomplete { reason }]
                if reason.contains("failed to parse") && !reason.contains("exceeds")
        ),
        "{:?}",
        inspection.facts
    );
}

#[test]
fn project_config_past_eight_mib_keeps_its_exact_useful_root() {
    let fixture = useful_layout_fixture(&large_layout_config());
    assert_exact_main_layout(&fixture, &inspect_without_deadline(&fixture));
}

#[test]
fn project_config_past_eight_mib_rejects_malformed_tail_and_extra_document() {
    for tail in ["broken: [\n", "---\nformat: DESIGNER\nsource-set: []\n"] {
        let fixture = useful_layout_fixture(&format!("{}{tail}", large_layout_config()));
        let inspection = inspect_without_deadline(&fixture);
        assert!(inspection.source_sets.is_none());
        assert!(!inspection.source_targets_complete);
        assert!(!inspection.repository_targets_complete);
        assert!(inspection.roots.is_empty());
        assert!(
            matches!(
                inspection.facts.as_slice(),
                [ProjectHealthFact::SourceInspectionIncomplete { reason }]
                    if reason.contains("failed to parse") && !reason.contains("exceeds")
            ),
            "tail={tail:?}; facts={:?}",
            inspection.facts
        );
        assert!(matches!(
            inspection.observations.as_slice(),
            [observation] if observation.id == ProjectCheckId::SourceDiscovery
                && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
        ));
    }
}

#[test]
fn large_platform_marker_does_not_consume_health_source_map_bytes() {
    let fixture = raw_layout_fixture(
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    );
    fs::create_dir_all(fixture.context.workspace_root.join("src")).unwrap();
    fs::File::create(fixture.context.workspace_root.join("src/Configuration.xml"))
        .unwrap()
        .set_len(9 * 1024 * 1024)
        .unwrap();

    let inspection = inspect(&fixture);

    assert!(inspection.source_targets_complete, "{:?}", inspection.facts);
    assert!(inspection.facts.is_empty(), "{:?}", inspection.facts);
    assert_eq!(
        inspection.source_sets.as_ref().unwrap()[0].source_format,
        SourceFormat::PlatformXml
    );
}

#[test]
fn source_layout_inspects_all_1025_declared_source_roots() {
    let mut config = "format: DESIGNER\nsource-set:\n".to_string();
    for index in 0..=1024 {
        config.push_str(&format!(
            "  - name: set{index}\n    type: CONFIGURATION\n    path: src/{index}\n"
        ));
    }
    let fixture = raw_layout_fixture(&config);
    for index in 0..=1024 {
        let root = fixture.context.workspace_root.join(format!("src/{index}"));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("Configuration.xml"), "<MetaDataObject/>").unwrap();
    }

    let inspection = inspect_without_deadline(&fixture);

    assert!(inspection.source_targets_complete, "{:?}", inspection.facts);
    assert!(
        inspection.repository_targets_complete,
        "{:?}",
        inspection.facts
    );
    assert!(inspection.facts.is_empty(), "{:?}", inspection.facts);
    let sets = inspection
        .source_sets
        .as_ref()
        .expect("all declared source sets");
    assert_eq!(sets.len(), 1025);
    assert_eq!(inspection.roots.len(), 1025);
    for (index, set) in sets.iter().enumerate() {
        assert_eq!(set.name, format!("set{index}"));
        assert_eq!(set.path, format!("src/{index}"));
        assert_eq!(set.kind, SourceSetKind::Configuration);
        assert_eq!(set.source_format, SourceFormat::PlatformXml);
        let root = inspection
            .roots
            .iter()
            .find(|root| root.source_set.name == set.name)
            .expect("every declared source has an inspected root");
        assert_eq!(
            root.path,
            canonical_path(&fixture.context.workspace_root.join(&set.path))
        );
    }
    assert!(inspection
        .observations
        .iter()
        .all(|observation| matches!(observation.outcome, ProjectCheckOutcome::Completed)));
}

#[test]
fn health_source_layout_keeps_all_defaulted_rows_and_their_real_ambiguity() {
    let mut config = "format: DESIGNER\nsource-set:\n".to_string();
    for _ in 0..400_000 {
        config.push_str("  - {}\n");
    }
    let fixture = raw_layout_fixture(&config);

    let inspection = inspect_without_deadline(&fixture);

    let sets = inspection
        .source_sets
        .as_ref()
        .unwrap_or_else(|| panic!("all defaulted source rows: {:?}", inspection.facts));
    assert_eq!(sets.len(), 400_000);
    let wrong_row = sets.iter().find(|set| {
        set.name != "main"
            || !set.path.is_empty()
            || set.kind != SourceSetKind::Configuration
            || set.source_format != SourceFormat::PlatformXml
    });
    assert!(
        wrong_row.is_none(),
        "unexpected defaulted row: {wrong_row:?}"
    );
    assert!(!inspection.source_targets_complete);
    assert!(!inspection.repository_targets_complete);
    assert!(inspection.roots.is_empty());
    assert!(
        matches!(
            inspection.facts.as_slice(),
            [ProjectHealthFact::SourceNameAmbiguous { name, count }]
                if name == "main" && *count == 400_000
        ),
        "{:?}",
        inspection.facts
    );
}

#[test]
fn health_yaml_parser_preserves_numeric_base_path_semantics() {
    let fixture = raw_layout_fixture(
        "format: DESIGNER\nbasePath: 123\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    );
    fs::create_dir_all(fixture.context.workspace_root.join("123/src")).unwrap();

    let inspection = inspect(&fixture);

    assert!(inspection.source_sets.is_some(), "{:?}", inspection.facts);
    assert_eq!(inspection.source_sets.as_ref().unwrap()[0].path, "123/src");
}

#[test]
fn health_yaml_parser_rejects_numeric_format_like_ordinary_parser() {
    let fixture = raw_layout_fixture("format: 123\nsource-set: []\n");

    let inspection = inspect(&fixture);

    assert!(matches!(
        inspection.facts.as_slice(),
        [ProjectHealthFact::SourceInspectionIncomplete { reason }]
            if reason.contains("field `format` must be a string")
    ));
}

#[test]
fn health_yaml_parser_rejects_null_format_like_ordinary_parser() {
    let fixture = raw_layout_fixture("format: null\nsource-set: []\n");

    let inspection = inspect(&fixture);

    assert!(matches!(
        inspection.facts.as_slice(),
        [ProjectHealthFact::SourceInspectionIncomplete { reason }]
            if reason.contains("field `format` must be a string")
    ));
}

#[test]
fn health_yaml_parser_rejects_duplicate_top_level_contract_fields() {
    for duplicate in [
        "format: DESIGNER\nformat: EDT\n",
        "basePath: src\nbasePath: other\n",
        "source-set: []\nsource-set: []\n",
    ] {
        let fixture = raw_layout_fixture(duplicate);
        let inspection = inspect(&fixture);
        assert!(
            matches!(
                inspection.facts.as_slice(),
                [ProjectHealthFact::SourceInspectionIncomplete { reason }]
                    if reason.contains("duplicate")
            ),
            "config={duplicate:?}; facts={:?}",
            inspection.facts
        );
    }
}

#[test]
fn health_yaml_aliases_keep_exact_selected_values_and_real_name_ambiguity() {
    let name = "x".repeat(4096);
    let fixture = raw_layout_fixture(&format!(
        "defaults: &large {{name: {}, type: CONFIGURATION, path: src}}\nsource-set:\n{}",
        name,
        "  - *large\n".repeat(1024)
    ));

    let inspection = inspect_without_deadline(&fixture);

    let sets = inspection
        .source_sets
        .as_ref()
        .expect("expanded alias source rows");
    assert_eq!(sets.len(), 1024);
    assert!(sets.iter().all(|set| set.name == name
        && set.path == "src"
        && set.kind == SourceSetKind::Configuration));
    assert!(!inspection.source_targets_complete);
    assert!(!inspection.repository_targets_complete);
    assert!(inspection.roots.is_empty());
    assert!(
        matches!(
            inspection.facts.as_slice(),
            [ProjectHealthFact::SourceNameAmbiguous { name: ambiguous_name, count }]
                if ambiguous_name == &name && *count == 1024
        ),
        "{:?}",
        inspection.facts
    );
}

#[test]
fn health_yaml_parser_accepts_small_bounded_source_set_alias() {
    let fixture = raw_layout_fixture(
        "shared: &main {name: main, type: CONFIGURATION, path: src}\nformat: DESIGNER\nsource-set: [*main]\n",
    );
    fs::create_dir_all(fixture.context.workspace_root.join("src")).unwrap();

    let inspection = inspect(&fixture);

    assert!(inspection.source_sets.is_some(), "{:?}", inspection.facts);
    assert_eq!(inspection.source_sets.as_ref().unwrap()[0].name, "main");
    assert_eq!(inspection.source_sets.as_ref().unwrap()[0].path, "src");
}

#[test]
fn health_yaml_parser_accepts_anchor_tokens_inside_unknown_block_scalar() {
    let fixture = raw_layout_fixture(
        "notes: |\n  * ordinary bullet\n  & ordinary ampersand\nformat: DESIGNER\nsource-set: []\n",
    );

    let inspection = inspect(&fixture);

    assert!(inspection.source_sets.is_some(), "{:?}", inspection.facts);
}

#[test]
fn health_yaml_many_selected_container_nodes_keep_the_useful_layout() {
    let fixture = useful_layout_fixture(&format!(
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n    ignored: [{}]\n",
        "[],".repeat(140_000)
    ));

    assert_exact_main_layout(&fixture, &inspect_without_deadline(&fixture));
}

#[test]
fn health_yaml_aliased_root_key_keeps_the_useful_layout_after_many_selected_nodes() {
    let fixture = useful_layout_fixture(&format!(
        "format: DESIGNER\nkey: &contract source-set\n? *contract\n:\n  - name: main\n    type: CONFIGURATION\n    path: src\n    ignored: [{}]\n",
        "[],".repeat(140_000)
    ));

    assert_exact_main_layout(&fixture, &inspect_without_deadline(&fixture));
}

#[test]
fn unknown_source_format_makes_source_targets_incomplete() {
    let fixture =
        raw_layout_fixture("source-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n");
    fs::create_dir_all(fixture.context.workspace_root.join("src")).unwrap();

    let inspection = inspect(&fixture);

    assert!(!inspection.source_targets_complete);
    assert!(inspection
        .facts
        .iter()
        .any(|fact| matches!(fact, ProjectHealthFact::SourceFormatUnknown { .. })));
}

#[test]
fn conflicting_source_format_makes_source_targets_incomplete() {
    let fixture =
        raw_layout_fixture("source-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n");
    fs::create_dir_all(fixture.context.workspace_root.join("src/Configuration")).unwrap();
    fs::write(
        fixture.context.workspace_root.join("src/Configuration.xml"),
        "<MetaDataObject/>",
    )
    .unwrap();
    fs::write(
        fixture
            .context
            .workspace_root
            .join("src/Configuration/Configuration.mdo"),
        "<mdclass:Configuration/>",
    )
    .unwrap();

    let inspection = inspect(&fixture);

    assert!(!inspection.source_targets_complete);
    assert!(inspection
        .facts
        .iter()
        .any(|fact| matches!(fact, ProjectHealthFact::SourceFormatInvalid { .. })));
}

static DISCOVERY_DEADLINE_TICKS: AtomicUsize = AtomicUsize::new(0);
static DISCOVERY_DEADLINE_ORIGIN: OnceLock<Instant> = OnceLock::new();

fn advancing_discovery_clock() -> Instant {
    let tick = DISCOVERY_DEADLINE_TICKS.fetch_add(1, Ordering::SeqCst) as u64;
    *DISCOVERY_DEADLINE_ORIGIN.get_or_init(Instant::now) + Duration::from_millis(tick)
}

#[test]
fn source_discovery_honors_deadline_at_internal_checkpoints() {
    let fixture = raw_layout_fixture(
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    );
    DISCOVERY_DEADLINE_TICKS.store(0, Ordering::SeqCst);
    let deadline = ProviderDeadline::with_clock(
        advancing_discovery_clock() + Duration::from_millis(5),
        advancing_discovery_clock,
    );

    let inspection = SourceLayoutInspector::inspect(
        &fixture.context,
        &CancellationToken::new(),
        deadline,
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();

    assert!(!inspection.source_targets_complete);
    assert!(inspection.source_sets.is_none());
    assert!(matches!(
        inspection.facts.as_slice(),
        [ProjectHealthFact::SourceInspectionIncomplete { reason }]
            if reason == "project health deadline expired during source discovery"
    ));
}

static LAYOUT_DEADLINE_TICKS: AtomicUsize = AtomicUsize::new(0);
static LAYOUT_DEADLINE_ORIGIN: OnceLock<Instant> = OnceLock::new();

fn advancing_layout_clock() -> Instant {
    let tick = LAYOUT_DEADLINE_TICKS.fetch_add(1, Ordering::SeqCst) as u64;
    *LAYOUT_DEADLINE_ORIGIN.get_or_init(Instant::now) + Duration::from_millis(tick)
}

#[test]
fn every_layout_deadline_discards_partial_results_into_a_typed_snapshot() {
    let fixture = raw_layout_fixture(
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    );
    fs::create_dir_all(fixture.context.workspace_root.join("src")).unwrap();

    let mut timed_out_phases = std::collections::BTreeSet::new();
    for budget_millis in 0..128 {
        LAYOUT_DEADLINE_TICKS.store(0, Ordering::SeqCst);
        let deadline = ProviderDeadline::with_clock(
            advancing_layout_clock() + Duration::from_millis(budget_millis),
            advancing_layout_clock,
        );
        let inspection = SourceLayoutInspector::inspect(
            &fixture.context,
            &CancellationToken::new(),
            deadline,
            &unica_workspace::discovery::ProjectDiscoveryReader::new(
                &fixture.context.workspace_root,
            ),
        )
        .unwrap_or_else(|error| {
            panic!("budget {budget_millis} ms must produce a typed snapshot: {error:?}")
        });

        if let Some(reason) = inspection.facts.iter().find_map(|fact| match fact {
            ProjectHealthFact::SourceInspectionIncomplete { reason }
                if reason.contains("deadline") =>
            {
                Some(reason.clone())
            }
            _ => None,
        }) {
            timed_out_phases.insert(reason);
            assert!(
                inspection.source_sets.is_none(),
                "budget {budget_millis} retained partial source sets"
            );
            assert_eq!(inspection.observations.len(), 1);
            assert!(matches!(
                inspection.observations[0].outcome,
                ProjectCheckOutcome::NotRun { .. }
            ));
        }
    }
    for phase in [
        "before source discovery",
        "during source discovery",
        "while grouping source sets",
        "while inspecting source sets",
    ] {
        assert!(
            timed_out_phases.iter().any(|reason| reason.contains(phase)),
            "deadline phase {phase:?} was not exercised: {timed_out_phases:?}"
        );
    }
}

#[test]
fn complete_empty_discovery_reports_no_source_sets() {
    let fixture = raw_layout_fixture("format: DESIGNER\nsource-set: []\n");

    let inspection = inspect(&fixture);

    assert_eq!(inspection.source_sets.as_ref().map(Vec::len), Some(0));
    assert!(!inspection.source_targets_complete);
    assert!(matches!(
        inspection.facts.as_slice(),
        [ProjectHealthFact::NoSourceSets]
    ));
}

#[test]
fn duplicate_names_have_one_group_fact_and_no_ambiguous_root() {
    let fixture = raw_layout_fixture(
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src/a\n  - name: main\n    type: EXTENSION\n    path: src/b\n",
    );
    fs::create_dir_all(fixture.context.workspace_root.join("src/a")).unwrap();
    fs::create_dir_all(fixture.context.workspace_root.join("src/b")).unwrap();

    let inspection = inspect(&fixture);

    assert!(!inspection.source_targets_complete);
    assert!(inspection.roots.is_empty());
    assert!(matches!(
        inspection.facts.as_slice(),
        [ProjectHealthFact::SourceNameAmbiguous { name, count }]
            if name == "main" && *count == 2
    ));
}

#[test]
fn missing_source_root_is_reported_without_becoming_fatal() {
    let fixture = layout_fixture("src/missing");

    let inspection = inspect(&fixture);

    assert!(inspection.source_targets_complete);
    assert_eq!(inspection.roots.len(), 1);
    assert!(matches!(
        inspection.facts.as_slice(),
        [ProjectHealthFact::SourcePathMissing { source_set, path }]
            if source_set == "main" && path == "src/missing"
    ));
    assert!(inspection.observations.iter().any(|observation| {
        observation.id == ProjectCheckId::SourceGeneratedPaths
            && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
    }));
}

#[test]
fn unreadable_source_root_is_not_ready_for_source_operations() {
    use unica_workspace::test_support::set_unix_mode_for_test;

    let fixture = layout_fixture("src");
    let root = fixture.context.workspace_root.join("src");
    fs::create_dir_all(&root).unwrap();
    if !set_unix_mode_for_test(&root, 0o000).unwrap() {
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::UnixPermissionBits,
            "the test sets Unix permission bits",
        );
        return;
    }

    let inspection = inspect(&fixture);

    set_unix_mode_for_test(&root, 0o700).unwrap();
    assert!(!inspection.source_targets_complete);
    assert!(inspection.roots.is_empty());
    assert!(inspection.facts.iter().any(|fact| matches!(
        fact,
        ProjectHealthFact::SourcePathUnsafe { reason, .. }
            if reason.contains("open") || reason.contains("denied")
    )));
    assert!(inspection.observations.iter().any(|observation| {
        observation.id == ProjectCheckId::SourceGeneratedPaths
            && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
    }));
}

#[test]
fn regular_file_cannot_be_a_source_root() {
    let fixture = layout_fixture("src-file");
    fs::write(
        fixture.context.workspace_root.join("src-file"),
        "not a directory",
    )
    .unwrap();

    let inspection = inspect(&fixture);

    assert!(!inspection.source_targets_complete);
    assert!(inspection.roots.is_empty());
    assert!(inspection.facts.iter().any(|fact| matches!(
        fact,
        ProjectHealthFact::SourcePathUnsafe { reason, .. }
            if reason.contains("not a directory")
    )));
    assert!(inspection.observations.iter().any(|observation| {
        observation.id == ProjectCheckId::SourceGeneratedPaths
            && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
    }));
}

#[test]
fn outside_source_root_is_a_typed_unsafe_path() {
    let fixture = layout_fixture("../outside");

    let inspection = inspect(&fixture);

    assert!(!inspection.source_targets_complete);
    assert!(inspection.roots.is_empty());
    assert!(matches!(
        inspection.facts.as_slice(),
        [ProjectHealthFact::SourcePathUnsafe { source_set, path, .. }]
            if source_set == "main" && path == "../outside"
    ));
}

#[test]
fn build_and_cache_are_independent_facts_for_a_nested_root() {
    let fixture = layout_fixture("src");
    fs::create_dir_all(fixture.context.workspace_root.join("src/.build/unica")).unwrap();
    let sentinel = fixture
        .context
        .workspace_root
        .join("src/.build/foreign-tool.data");
    fs::write(&sentinel, b"keep these bytes").unwrap();
    let mut context = fixture.context.clone();
    context.cache_root = context.workspace_root.join("src/.build/unica");

    let inspection = SourceLayoutInspector::inspect(
        &context,
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(1)),
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&context.workspace_root),
    )
    .unwrap();

    assert_eq!(inspection.facts.len(), 2);
    assert!(inspection
        .facts
        .iter()
        .any(|fact| matches!(fact, ProjectHealthFact::CacheInsideSourceSet { .. })));
    assert!(inspection
        .facts
        .iter()
        .any(|fact| matches!(fact, ProjectHealthFact::GeneratedBuildPresent { .. })));
    assert_eq!(fs::read(sentinel).unwrap(), b"keep these bytes");
}

#[test]
fn linked_alias_to_workspace_reports_the_primary_identity_cause_with_evidence() {
    let fixture = layout_fixture("alias");
    let outcome = create_directory_link_fixture_for_test(
        &fixture.context.workspace_root,
        fixture.context.workspace_root.join("alias"),
    )
    .unwrap();
    if outcome != FileLinkFixtureOutcome::Created {
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::Symlink,
            "creating a link fixture needs the symlink privilege",
        );
        return;
    }

    let inspection = inspect(&fixture);

    assert!(matches!(
        inspection.facts.as_slice(),
        [ProjectHealthFact::SourceRootIsWorkspace { evidence, .. }]
            if evidence.iter().any(|item| item.contains("linked route: "))
    ));
}

#[test]
fn linked_route_to_another_nested_root_is_unsafe() {
    let fixture = layout_fixture("alias");
    let nested = fixture.context.workspace_root.join("actual");
    fs::create_dir_all(&nested).unwrap();
    let outcome = create_directory_link_fixture_for_test(
        &nested,
        fixture.context.workspace_root.join("alias"),
    )
    .unwrap();
    if outcome != FileLinkFixtureOutcome::Created {
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::Symlink,
            "creating a link fixture needs the symlink privilege",
        );
        return;
    }

    let inspection = inspect(&fixture);

    assert!(!inspection.source_targets_complete);
    assert!(inspection.roots.is_empty());
    assert!(matches!(
        inspection.facts.as_slice(),
        [ProjectHealthFact::SourcePathUnsafe { reason, .. }]
            if reason.contains("symbolic link or reparse point")
    ));
    assert!(inspection.observations.iter().any(|observation| {
        observation.id == ProjectCheckId::SourceFormat
            && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
    }));
    assert!(inspection.observations.iter().any(|observation| {
        observation.id == ProjectCheckId::SourceGeneratedPaths
            && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
    }));
}

#[test]
fn linked_route_without_trusted_format_does_not_pass_format_or_generated_checks() {
    let fixture = raw_layout_fixture(
        "source-set:\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: epf\n",
    );
    let external = fixture.context.workspace_root.join("external");
    fs::create_dir_all(&external).unwrap();
    fs::write(external.join("SecretPayroll.xml"), "<MetaDataObject/>").unwrap();
    let outcome = create_directory_link_fixture_for_test(
        &external,
        fixture.context.workspace_root.join("epf"),
    )
    .unwrap();
    if outcome != FileLinkFixtureOutcome::Created {
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::Symlink,
            "creating a link fixture needs the symlink privilege",
        );
        return;
    }

    let inspection = inspect(&fixture);

    for check in [
        ProjectCheckId::SourceFormat,
        ProjectCheckId::SourceGeneratedPaths,
    ] {
        assert!(inspection.observations.iter().any(|observation| {
            observation.id == check
                && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
        }));
    }
    assert!(!inspection.source_sets.unwrap()[0]
        .format_evidence
        .iter()
        .any(|evidence| evidence.contains("SecretPayroll")));
}

struct LayoutFixture {
    _temp: TempDir,
    context: WorkspaceContext,
}

fn layout_fixture(configured_path: &str) -> LayoutFixture {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(workspace.join("src")).unwrap();
    fs::create_dir_all(workspace.join(".build/unica")).unwrap();
    fs::write(
        workspace.join("v8project.yaml"),
        format!(
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: {configured_path}\n"
        ),
    )
    .unwrap();
    fs::write(workspace.join("Configuration.xml"), "<MetaDataObject/>").unwrap();
    LayoutFixture {
        _temp: temp,
        context: WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build/unica"),
            workspace_epoch: 0,
        },
    }
}

fn raw_layout_fixture(config: &str) -> LayoutFixture {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    fs::write(workspace.join("v8project.yaml"), config).unwrap();
    LayoutFixture {
        _temp: temp,
        context: WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build/unica"),
            workspace_epoch: 0,
        },
    }
}

fn inspect(fixture: &LayoutFixture) -> super::SourceLayoutInspection {
    SourceLayoutInspector::inspect(
        &fixture.context,
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(1)),
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap()
}

fn inspect_without_deadline(fixture: &LayoutFixture) -> super::SourceLayoutInspection {
    SourceLayoutInspector::inspect(
        &fixture.context,
        &CancellationToken::new(),
        ProviderDeadline::no_deadline(),
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap()
}

fn useful_layout_fixture(config: &str) -> LayoutFixture {
    let fixture = raw_layout_fixture(config);
    fs::create_dir_all(fixture.context.workspace_root.join("src")).unwrap();
    fs::write(
        fixture.context.workspace_root.join("src/Configuration.xml"),
        "<MetaDataObject/>",
    )
    .unwrap();
    fixture
}

fn large_layout_config() -> String {
    format!(
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n# {}\n",
        "x".repeat(9 * 1024 * 1024)
    )
}

fn assert_exact_main_layout(fixture: &LayoutFixture, inspection: &super::SourceLayoutInspection) {
    assert!(inspection.source_targets_complete, "{:?}", inspection.facts);
    assert!(
        inspection.repository_targets_complete,
        "{:?}",
        inspection.facts
    );
    assert!(inspection.facts.is_empty(), "{:?}", inspection.facts);
    let sets = inspection.source_sets.as_ref().expect("useful main source");
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].name, "main");
    assert_eq!(sets[0].path, "src");
    assert_eq!(sets[0].kind, SourceSetKind::Configuration);
    assert_eq!(sets[0].source_format, SourceFormat::PlatformXml);
    assert_eq!(inspection.roots.len(), 1);
    assert_eq!(inspection.roots[0].source_set.name, "main");
    assert_eq!(
        inspection.roots[0].path,
        canonical_path(&fixture.context.workspace_root.join("src"))
    );
    assert!(inspection
        .observations
        .iter()
        .all(|observation| matches!(observation.outcome, ProjectCheckOutcome::Completed)));
}
