//! Existing real source discovery tests across Designer and the workspace owner.
use std::path::Path;
use std::sync::Arc;
use unica_source_contract::ProjectSourceMap;
use unica_source_designer::project_sources::ActorSourceMapDiscovery;
use unica_workspace::discovery::{ProjectDiscoveryReader, ProjectSourceMapProvenance};
fn discover_project_source_map(root: &Path) -> Result<ProjectSourceMap, String> {
    unica_source_designer::project_sources::discover_project_source_map(
        &ProjectDiscoveryReader::new(root),
    )
}
fn discover_project_source_map_controlled(
    root: &Path,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<ProjectSourceMap, String> {
    unica_source_designer::project_sources::discover_project_source_map_controlled(
        &ProjectDiscoveryReader::new(root),
        checkpoint,
    )
}
fn discover_project_source_map_with_provenance(
    root: &Path,
) -> Result<(ProjectSourceMap, ProjectSourceMapProvenance), String> {
    let reader = ProjectDiscoveryReader::new(root);
    let mut provenance = ProjectSourceMapProvenance::default();
    let map = unica_source_designer::project_sources::discover_project_source_map_with_provenance(
        &reader,
        &mut provenance,
    )?;
    Ok((map, provenance))
}
fn discover_project_source_admission(
    root: &Path,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<unica_workspace::selection_evidence::ResolvedProjectSourceAdmission, String> {
    unica_workspace::selection_evidence::discover_project_source_admission(
        root,
        Arc::new(ActorSourceMapDiscovery::new(
            unica_workspace::paths::host_path_text,
        )),
        checkpoint,
    )
}
fn discover_project_source_map_internal(
    root: &Path,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
    state: &mut unica_source_designer::project_sources::test_support::DiscoveryFixtureState<'_>,
) -> Result<ProjectSourceMap, String> {
    state.discover(root, checkpoint)
}
fn snapshot_project_map_input(
    path: &Path,
    read_contents: bool,
    state: &mut unica_source_designer::project_sources::test_support::DiscoveryFixtureState<'_>,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<Option<Vec<u8>>, String> {
    state.snapshot_input(path, read_contents, checkpoint)
}

mod tests {

    use super::*;
    use std::collections::BTreeMap;
    use unica_source_contract::{SourceFormat, SourceSetKind, SourceSetState};
    use unica_source_designer::project_sources::test_support::{
        DiscoveryFixtureState as SourceMapDiscoveryState, ExtensionLayoutShape,
        BASE_CONFIGURATION_LAYOUTS, EXTENSION_LAYOUTS, SOURCE_ROOT_MARKERS,
    };
    use unica_workspace::compile_transaction::CompileTransaction;
    use unica_workspace::discovery::{ProjectDiscoveryReader, ProjectSourceMapProvenance};

    const HEALTH_LEAF_MAIN_CONFIG: &str = concat!(
        "format: DESIGNER\n",
        "source-set:\n",
        "  - name: main\n",
        "    type: CONFIGURATION\n",
        "    path: src\n",
    );

    fn health_leaf_fixture(config: &str) -> tempfile::TempDir {
        let fixture = tempfile::tempdir().unwrap();
        write(&fixture.path().join("v8project.yaml"), config);
        write(
            &fixture.path().join("src/Configuration.xml"),
            "<MetaDataObject/>",
        );
        fixture
    }

    fn assert_health_leaf_main(map: &ProjectSourceMap) {
        assert_eq!(map.source_sets.len(), 1);
        assert_eq!(map.effective_source_set.as_deref(), Some("main"));
        assert_eq!(map.source_sets[0].path, "src");
        assert_eq!(map.source_sets[0].source_state, SourceSetState::Supported);
        assert_eq!(map.source_sets[0].format_probe_error, None);
        assert_source_set(
            map,
            "main",
            SourceSetKind::Configuration,
            SourceFormat::PlatformXml,
            &["src/Configuration.xml"],
        );
    }

    fn assert_health_leaf_both_routes(fixture: &tempfile::TempDir, reason: &str) {
        let map = discover_project_source_map_controlled(fixture.path(), &mut || Ok(()))
            .unwrap_or_else(|error| panic!("{reason}: controlled health refused: {error}"));
        assert_health_leaf_main(&map);
        let admission = discover_project_source_admission(fixture.path(), &mut || Ok(()))
            .unwrap_or_else(|error| panic!("{reason}: actor admission refused: {error}"));
        assert_health_leaf_main(admission.map());
    }

    fn health_leaf_large_input_fixture() -> tempfile::TempDir {
        let mut config = String::from(HEALTH_LEAF_MAIN_CONFIG);
        config.push_str("ignored: '");
        config.push_str(&"x".repeat(9 * 1024 * 1024));
        config.push_str("'\n");
        assert!(config.len() > 8 * 1024 * 1024);
        health_leaf_fixture(&config)
    }

    fn health_format_bytes_fixture() -> (tempfile::TempDir, String) {
        let fixture = tempfile::tempdir().unwrap();
        let path = format!("{}/{}/src", "a".repeat(180), "b".repeat(180));
        let marker = format!("{path}/Configuration.xml");
        write(&fixture.path().join(&marker), "<MetaDataObject/>");
        let mut config = String::from("format: DESIGNER\nsource-set:\n");
        for index in 0..12_000 {
            config.push_str(&format!(
                "  - name: set{index:05}\n    type: CONFIGURATION\n    path: {path}\n"
            ));
        }
        assert!(config.len() < 8 * 1024 * 1024);
        assert!(marker.len() * 12_000 > 4 * 1024 * 1024);
        write(&fixture.path().join("v8project.yaml"), &config);
        (fixture, path)
    }

    fn assert_health_format_bytes(map: &ProjectSourceMap, path: &str) {
        assert_eq!(map.source_sets.len(), 12_000);
        assert!(map.source_sets.len() < 16 * 1024);
        for (index, set) in map.source_sets.iter().enumerate() {
            assert_eq!(set.name, format!("set{index:05}"));
            assert_eq!(set.path, path);
            assert_eq!(set.kind, SourceSetKind::Configuration);
            assert_eq!(set.source_format, SourceFormat::PlatformXml);
            assert_eq!(set.source_state, SourceSetState::Supported);
            assert_eq!(set.format_probe_error, None);
            assert_eq!(
                set.format_evidence,
                vec![format!("{path}/Configuration.xml")]
            );
        }
    }

    #[test]
    fn controlled_health_keeps_format_evidence_past_four_mib_below_the_old_entry_limit() {
        let (fixture, path) = health_format_bytes_fixture();
        let map = discover_project_source_map_controlled(fixture.path(), &mut || Ok(()))
            .expect("valid format evidence beyond four MiB must keep every source");
        assert_health_format_bytes(&map, &path);
    }

    #[test]
    fn actor_health_keeps_format_evidence_past_four_mib_below_the_old_entry_limit() {
        let (fixture, path) = health_format_bytes_fixture();
        let admission = discover_project_source_admission(fixture.path(), &mut || Ok(()))
            .expect("actor format evidence beyond four MiB must keep every source");
        assert_health_format_bytes(admission.map(), &path);
    }

    fn health_external_membership_fixture() -> tempfile::TempDir {
        let fixture = tempfile::tempdir().unwrap();
        write(&fixture.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: epf\n");
        write(
            &fixture.path().join("epf/zz-final.xml"),
            "<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
        );
        for index in 0..16_385 {
            fs::File::create(fixture.path().join(format!("epf/noise-{index:05}.txt"))).unwrap();
        }
        fixture
    }

    fn assert_health_external_membership(map: &ProjectSourceMap) {
        assert_eq!(map.source_sets.len(), 1);
        assert_eq!(map.source_sets[0].path, "epf");
        assert_eq!(
            map.source_sets[0].source_state,
            SourceSetState::Supported,
            "external source: {:?}",
            map.source_sets[0]
        );
        assert_eq!(map.source_sets[0].format_probe_error, None);
        assert_source_set(
            map,
            "external",
            SourceSetKind::ExternalProcessor,
            SourceFormat::PlatformXml,
            &["epf/zz-final.xml"],
        );
    }

    #[test]
    fn controlled_health_external_descriptor_survives_more_than_16384_siblings() {
        let fixture = health_external_membership_fixture();
        let map = discover_project_source_map_controlled(fixture.path(), &mut || Ok(()))
            .expect("external descriptor discovery must inspect all siblings without a quota");
        assert_health_external_membership(&map);
    }

    #[test]
    fn actor_health_external_descriptor_survives_more_than_16384_siblings_and_binds_membership() {
        let fixture = health_external_membership_fixture();
        let admission = discover_project_source_admission(fixture.path(), &mut || Ok(()))
            .expect("actor external discovery must inspect all siblings without a quota");
        assert_health_external_membership(admission.map());
        let evidence = admission.into_evidence();
        let deadline = unica_work_control::provider_deadline::ProviderDeadline::no_deadline();
        let cancellation = unica_work_control::cancellation::CancellationToken::new();
        evidence.validate(deadline, &cancellation).unwrap();
        fs::File::create(fixture.path().join("epf/noise-after-admission.txt")).unwrap();
        let unchanged_projection =
            discover_project_source_map_controlled(fixture.path(), &mut || Ok(())).unwrap();
        assert_health_external_membership(&unchanged_projection);
        assert_eq!(
            evidence
                .validate(deadline, &cancellation)
                .unwrap_err()
                .kind(),
            unica_workspace::selection_evidence::SourceSelectionEvidenceErrorKind::Changed
        );
    }

    #[test]
    fn controlled_health_accepts_yaml_input_past_eight_mib() {
        let fixture = health_leaf_large_input_fixture();
        let map = discover_project_source_map_controlled(fixture.path(), &mut || Ok(()))
            .expect("valid ignored YAML input past8MiB must retain its useful source");
        assert_health_leaf_main(&map);
    }

    #[test]
    fn actor_health_accepts_yaml_input_past_eight_mib_and_binds_late_bytes() {
        let fixture = health_leaf_large_input_fixture();
        let admission = discover_project_source_admission(fixture.path(), &mut || Ok(()))
            .expect("actor admission must retain valid YAML input past8MiB");
        assert_health_leaf_main(admission.map());
        let evidence = admission.into_evidence();
        // Срок — у каждой проверки свой: общий срок съедала бы работа между
        // ними, и на медленном раннере вторая проверка отвечала бы `Deadline`
        // раньше, чем дошла бы до изменённых байтов.
        let deadline = || {
            unica_work_control::provider_deadline::ProviderDeadline::from_budget(
                std::time::Duration::from_secs(5),
            )
        };
        let cancellation = unica_work_control::cancellation::CancellationToken::new();
        evidence.validate(deadline(), &cancellation).unwrap();

        use std::io::{Seek, SeekFrom, Write};
        let mut file = fs::OpenOptions::new()
            .write(true)
            .open(fixture.path().join("v8project.yaml"))
            .unwrap();
        file.seek(SeekFrom::Start(8 * 1024 * 1024 + 1)).unwrap();
        file.write_all(b"y").unwrap();
        drop(file);
        let unchanged_map =
            discover_project_source_map_controlled(fixture.path(), &mut || Ok(())).unwrap();
        assert_health_leaf_main(&unchanged_map);
        let error = evidence.validate(deadline(), &cancellation).unwrap_err();
        assert_eq!(
            error.kind(),
            unica_workspace::selection_evidence::SourceSelectionEvidenceErrorKind::Changed,
        );
    }

    fn health_leaf_many_sources_fixture(declared: bool) -> tempfile::TempDir {
        let fixture = tempfile::tempdir().unwrap();
        let mut config = String::from("format: DESIGNER\nsource-set:\n");
        for index in 0..1025 {
            let name = format!("extension_{index:04}");
            let path = format!("src/cfe/{name}");
            write(
                &fixture.path().join(&path).join("Configuration.xml"),
                "<MetaDataObject/>",
            );
            if declared {
                config.push_str(&format!(
                    "  - name: {name}\n    type: EXTENSION\n    path: {path}\n"
                ));
            }
        }
        if declared {
            write(&fixture.path().join("v8project.yaml"), &config);
        }
        fixture
    }

    fn assert_health_leaf_many_sources(map: &ProjectSourceMap) {
        assert_eq!(map.source_sets.len(), 1025);
        let actual: BTreeMap<_, _> = map
            .source_sets
            .iter()
            .map(|source| {
                assert_eq!(source.kind, SourceSetKind::Extension);
                assert_eq!(source.source_state, SourceSetState::Supported);
                assert_eq!(source.source_format, SourceFormat::PlatformXml);
                assert_eq!(source.format_probe_error, None);
                assert_eq!(
                    source.format_evidence,
                    vec![format!("{}/Configuration.xml", source.path)]
                );
                (source.name.clone(), source.path.clone())
            })
            .collect();
        let expected: BTreeMap<_, _> = (0..1025)
            .map(|index| {
                let name = format!("extension_{index:04}");
                (name.clone(), format!("src/cfe/{name}"))
            })
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn controlled_health_accepts_1025_declared_source_sets() {
        let fixture = health_leaf_many_sources_fixture(true);
        let map = discover_project_source_map_controlled(fixture.path(), &mut || Ok(()))
            .expect("health must discover all1025 useful declared source sets");
        assert_health_leaf_many_sources(&map);
    }

    #[test]
    fn actor_health_accepts_1025_declared_source_sets() {
        let fixture = health_leaf_many_sources_fixture(true);
        let admission = discover_project_source_admission(fixture.path(), &mut || Ok(()))
            .expect("actor must admit all1025 useful declared source sets");
        assert_health_leaf_many_sources(admission.map());
    }

    #[test]
    fn controlled_health_autodetects_1025_source_directories() {
        let fixture = health_leaf_many_sources_fixture(false);
        let map = discover_project_source_map_controlled(fixture.path(), &mut || Ok(()))
            .expect("health enumeration must reach every useful source beyond1024");
        assert_health_leaf_many_sources(&map);
    }

    #[test]
    fn actor_health_autodetects_1025_source_directories() {
        let fixture = health_leaf_many_sources_fixture(false);
        let admission = discover_project_source_admission(fixture.path(), &mut || Ok(()))
            .expect("retained actor membership must reach every useful source beyond1024");
        assert_health_leaf_many_sources(admission.map());
    }

    #[test]
    fn health_yaml_selected_values_past_two_mib_keep_their_useful_source() {
        let mut config = String::from(HEALTH_LEAF_MAIN_CONFIG);
        config.push_str("    ignored: '");
        config.push_str(&"x".repeat(2 * 1024 * 1024 + 1));
        config.push_str("'\n");
        let fixture = health_leaf_fixture(&config);
        assert_health_leaf_both_routes(&fixture, "selected value past2MiB");
    }

    #[test]
    fn health_yaml_ignored_nodes_past_65536_keep_their_useful_source() {
        let mut config = String::from(HEALTH_LEAF_MAIN_CONFIG);
        config.push_str("ignored: [");
        config.push_str(&"0,".repeat(65537));
        config.push_str("]\n");
        let fixture = health_leaf_fixture(&config);
        assert_health_leaf_both_routes(&fixture, "ignored syntactic nodes past65536");
    }

    #[test]
    fn health_yaml_ignored_alias_bytes_past_sixteen_mib_keep_their_useful_source() {
        let mut config = String::from(HEALTH_LEAF_MAIN_CONFIG);
        config.push_str("template: &large '");
        config.push_str(&"x".repeat(32 * 1024));
        config.push_str("'\nignored: [");
        config.push_str(&"*large,".repeat(513));
        config.push_str("]\n");
        assert!(config.len() < 8 * 1024 * 1024);
        let fixture = health_leaf_fixture(&config);
        assert_health_leaf_both_routes(&fixture, "ignored expanded alias bytes past16MiB");
    }

    #[test]
    fn health_yaml_ignored_depth_past_256_keeps_its_useful_source() {
        let mut config = String::from(HEALTH_LEAF_MAIN_CONFIG);
        config.push_str("ignored: ");
        config.push_str(&"[".repeat(257));
        config.push('0');
        config.push_str(&"]".repeat(257));
        config.push('\n');
        let fixture = health_leaf_fixture(&config);
        assert_health_leaf_both_routes(&fixture, "ignored nesting past256");
    }

    #[test]
    fn health_yaml_ignored_alias_depth_past_256_keeps_its_useful_source() {
        let mut config = String::from(HEALTH_LEAF_MAIN_CONFIG);
        config.push_str("template: &deep ");
        config.push_str(&"[".repeat(180));
        config.push('0');
        config.push_str(&"]".repeat(180));
        config.push_str("\nignored: ");
        config.push_str(&"[".repeat(100));
        config.push_str("*deep");
        config.push_str(&"]".repeat(100));
        config.push('\n');
        let fixture = health_leaf_fixture(&config);
        assert_health_leaf_both_routes(&fixture, "ignored expanded alias depth past256");
    }

    #[test]
    fn health_yaml_keeps_route_specific_duplicate_and_anchor_semantics() {
        for tail in [
            "ignored: {a: 1, a: 2}\n",
            "ignored: 1\nignored: 2\n",
            "old: &shadow first\nignored: &shadow {value: *shadow}\n",
            "basePath: null\n",
            "basePath: !!null null\n",
            "basePath: !local null\n",
            "true: ignored\n",
        ] {
            let fixture = health_leaf_fixture(&format!("{HEALTH_LEAF_MAIN_CONFIG}{tail}"));
            assert_health_leaf_both_routes(&fixture, "legacy ignored YAML grammar");
        }
        for config in [
            HEALTH_LEAF_MAIN_CONFIG.replace("format: DESIGNER", "format: !!str DESIGNER"),
            HEALTH_LEAF_MAIN_CONFIG.replace("source-set:", "field: &field source-set\n*field :"),
        ] {
            let fixture = health_leaf_fixture(&config);
            assert_health_leaf_both_routes(&fixture, "legacy typed scalar and alias-key grammar");
        }
        for (base_path, directory) in [("'null'", "null"), ("0", "0")] {
            let config = format!("basePath: {base_path}\n{HEALTH_LEAF_MAIN_CONFIG}");
            let fixture = health_leaf_fixture(&config);
            let relative = format!("{directory}/src");
            write(
                &fixture.path().join(&relative).join("Configuration.xml"),
                "<MetaDataObject/>",
            );
            let map = discover_project_source_map_controlled(fixture.path(), &mut || Ok(()))
                .expect("quoted null and numeric basePath retain their literal path meaning");
            let admission = discover_project_source_admission(fixture.path(), &mut || Ok(()))
                .expect("actor keeps quoted null and numeric basePath meaning");
            for map in [&map, admission.map()] {
                assert_eq!(map.source_sets.len(), 1);
                assert_eq!(map.source_sets[0].path, relative);
                assert_eq!(map.source_sets[0].source_state, SourceSetState::Supported);
                assert_eq!(
                    map.source_sets[0].format_evidence,
                    vec![format!("{relative}/Configuration.xml")]
                );
            }
        }
        for format_value in ["7", "!local DESIGNER"] {
            let config = HEALTH_LEAF_MAIN_CONFIG
                .replace("format: DESIGNER", &format!("format: {format_value}"));
            let fixture = health_leaf_fixture(&config);
            let error = discover_project_source_map_controlled(fixture.path(), &mut || Ok(()))
                .expect_err("format must keep its direct string type requirement");
            let actor_error =
                match discover_project_source_admission(fixture.path(), &mut || Ok(())) {
                    Ok(_) => panic!("actor must preserve the direct format string requirement"),
                    Err(error) => error,
                };
            for error in [error, actor_error] {
                assert!(error.contains("field `format` must be a string"), "{error}");
            }
        }
        for tail in [
            "format: DESIGNER\n",
            "ignored: *missing\n",
            "ignored: &self {value: *self}\n",
        ] {
            let fixture = health_leaf_fixture(&format!("{HEALTH_LEAF_MAIN_CONFIG}{tail}"));
            let result = discover_project_source_map_controlled(fixture.path(), &mut || Ok(()));
            let error = result.expect_err("known duplicate/unresolved anchor must still refuse");
            let actor_error =
                match discover_project_source_admission(fixture.path(), &mut || Ok(())) {
                    Ok(_) => panic!("actor must refuse known duplicate/unresolved anchor"),
                    Err(error) => error,
                };
            for error in [error, actor_error] {
                if tail.starts_with("format:") {
                    assert!(error.contains("duplicate top-level field"), "{error}");
                } else {
                    assert!(error.contains("unresolved or recursive anchor"), "{error}");
                }
            }
        }
    }

    #[test]
    fn health_yaml_rejects_malformed_tail_and_extra_document_after_useful_source() {
        for tail in ["ignored: [\n", "---\nignored: valid\n"] {
            let fixture = health_leaf_fixture(&format!("{HEALTH_LEAF_MAIN_CONFIG}{tail}"));
            let error = discover_project_source_map_controlled(fixture.path(), &mut || Ok(()))
                .expect_err("a useful source cannot hide invalid trailing YAML");
            assert!(error.contains("YAML") || error.contains("parse"), "{error}");
            let actor_error =
                match discover_project_source_admission(fixture.path(), &mut || Ok(())) {
                    Ok(_) => panic!("actor must refuse invalid trailing YAML"),
                    Err(error) => error,
                };
            assert!(
                actor_error.contains("YAML") || actor_error.contains("parse"),
                "{actor_error}"
            );
        }
    }

    fn large_reserved_external_descriptor_fixture() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        write(
            &root.path().join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: main\n",
                "    type: EXTERNAL_DATA_PROCESSORS\n",
                "    path: epf\n",
            ),
        );
        let mut bytes = b"<MetaDataObject><ExternalDataProcessor/><!--".to_vec();
        bytes.extend(std::iter::repeat_n(b'x', 9 * 1024 * 1024));
        bytes.extend_from_slice(b"--></MetaDataObject>");
        assert!(bytes.len() > 8 * 1024 * 1024);
        assert!(bytes.len() < 10 * 1024 * 1024);
        let descriptor = root.path().join("epf/ConfigDumpInfo.xml");
        fs::create_dir_all(descriptor.parent().unwrap()).unwrap();
        fs::write(descriptor, bytes).unwrap();
        root
    }

    fn assert_large_reserved_descriptor_is_observed(map: &ProjectSourceMap) {
        let [source] = map.source_sets.as_slice() else {
            panic!("expected one source set, got {:?}", map.source_sets);
        };
        assert_eq!(
            source.source_state,
            SourceSetState::Supported,
            "large valid XML was not observed: {source:?}",
        );
        assert_eq!(source.format_probe_error, None);
        assert_source_set(
            map,
            "main",
            SourceSetKind::ExternalProcessor,
            SourceFormat::PlatformXml,
            &["epf/ConfigDumpInfo.xml"],
        );
    }

    #[test]
    fn ordinary_source_map_recognizes_large_reserved_external_descriptor() {
        let fixture = large_reserved_external_descriptor_fixture();
        let map = discover_project_source_map(fixture.path()).unwrap();
        assert_large_reserved_descriptor_is_observed(&map);
    }

    #[test]
    fn controlled_source_map_recognizes_large_reserved_external_descriptor() {
        let fixture = large_reserved_external_descriptor_fixture();
        let map = discover_project_source_map_controlled(fixture.path(), &mut || Ok(())).unwrap();
        assert_large_reserved_descriptor_is_observed(&map);
    }

    #[test]
    fn actor_admission_recognizes_large_reserved_descriptor_and_binds_exact_content() {
        let fixture = large_reserved_external_descriptor_fixture();
        let admission = discover_project_source_admission(fixture.path(), &mut || Ok(())).unwrap();
        assert_large_reserved_descriptor_is_observed(admission.map());
        let evidence = admission.into_evidence();
        // Срок — у каждой проверки свой: общий срок съедала бы работа между
        // ними, и на медленном раннере вторая проверка отвечала бы `Deadline`
        // раньше, чем дошла бы до изменённых байтов.
        let deadline = || {
            unica_work_control::provider_deadline::ProviderDeadline::from_budget(
                std::time::Duration::from_secs(5),
            )
        };
        let cancellation = unica_work_control::cancellation::CancellationToken::new();
        evidence.validate(deadline(), &cancellation).unwrap();

        // Change a comment byte after the former boundary without changing the
        // XML kind, file identity or length. Exact selection evidence must notice.
        use std::io::{Seek, SeekFrom, Write};
        let mut file = fs::OpenOptions::new()
            .write(true)
            .open(fixture.path().join("epf/ConfigDumpInfo.xml"))
            .unwrap();
        file.seek(SeekFrom::Start(8 * 1024 * 1024 + 1)).unwrap();
        file.write_all(b"y").unwrap();
        drop(file);
        let error = evidence.validate(deadline(), &cancellation).unwrap_err();
        assert_eq!(
            error.kind(),
            unica_workspace::selection_evidence::SourceSelectionEvidenceErrorKind::Changed,
        );
    }

    /// Объявленный, но пустой набор — законное состояние, а не ошибка: так
    /// рабочее пространство выглядит до знакомства с Unica. Прежде такому
    /// набору приписывался формат из объявления, будто его наблюдали, и
    /// читатель не мог отличить «работаем» от «надо наполнить».
    #[test]
    fn a_declared_but_empty_source_set_is_declared_not_observed() {
        let root = temp_workspace("unica-source-state-declared");
        fs::write(
            root.join("v8project.yaml"),
            b"format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("src")).unwrap();

        let map = discover_project_source_map(&root).unwrap();
        let set = map
            .source_sets
            .iter()
            .find(|set| set.name == "main")
            .expect("объявленный набор перечислен наравне с наполненными");

        assert_eq!(set.source_state, SourceSetState::Declared);
        assert!(
            !set.source_state.is_observed(),
            "формат взят из объявления, а не наблюдён"
        );
        assert_eq!(
            set.format_evidence,
            vec!["v8project.yaml:format=DESIGNER".to_string()],
            "доказательство указывает на объявление, а не на файл"
        );

        let _ = fs::remove_dir_all(&root);
    }

    /// Обратная сторона: как только выгрузка на месте, состояние наблюдённое,
    /// и доказательство указывает на файл.
    #[test]
    fn a_filled_source_set_is_supported_and_its_evidence_points_at_a_file() {
        let root = temp_workspace("unica-source-state-supported");
        fs::write(
            root.join("v8project.yaml"),
            b"format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        write(&root.join("src/Configuration.xml"), "<MetaDataObject/>");

        let map = discover_project_source_map(&root).unwrap();
        let set = map
            .source_sets
            .iter()
            .find(|set| set.name == "main")
            .expect("наполненный набор перечислен");

        assert_eq!(set.source_state, SourceSetState::Supported);
        assert!(set.source_state.is_observed());
        assert_eq!(
            set.format_evidence,
            vec!["src/Configuration.xml".to_string()]
        );

        let _ = fs::remove_dir_all(&root);
    }

    use std::ffi::{OsStr, OsString};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_WORKSPACE_NONCE: AtomicU64 = AtomicU64::new(0);

    #[test]
    pub(crate) fn external_actor_positive_witness_uses_no_process_global_counter() {
        let source = include_str!("../../unica-source-designer/src/project_sources.rs");
        for forbidden in [
            ["EXTERNAL_ACTOR_", "WITNESS_RUNS"].concat(),
            ["reset_external_actor_", "witness_runs"].concat(),
            ["external_actor_", "witness_runs"].concat(),
        ] {
            assert!(
                !source.contains(&forbidden),
                "external-positive aggregate still depends on process-global state: {forbidden}"
            );
        }
    }

    #[test]
    fn source_map_input_read_honors_cooperative_checkpoint_between_chunks() {
        let root = temp_workspace("unica-source-map-controlled-read");
        let config = root.join("v8project.yaml");
        let mut raw = b"format: DESIGNER\nsource-set: []\n".to_vec();
        raw.extend(std::iter::repeat_n(b'#', 192 * 1024));
        fs::write(&config, &raw).unwrap();
        let reader = ProjectDiscoveryReader::new(&root);
        let mut provenance = ProjectSourceMapProvenance::default();
        let mut state = SourceMapDiscoveryState::transactional(&reader, &mut provenance);
        let mut checkpoints = 0;

        let error =
            snapshot_project_map_input(&config, true, &mut state, &mut || -> Result<(), String> {
                checkpoints += 1;
                if checkpoints == 4 {
                    Err("source discovery stopped by checkpoint".to_string())
                } else {
                    Ok(())
                }
            })
            .unwrap_err();

        assert_eq!(error, "source discovery stopped by checkpoint");
        assert_eq!(checkpoints, 4);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ordinary_source_map_keeps_accepting_large_valid_project_config() {
        let root = temp_workspace("unica-source-map-large-ordinary-config");
        let mut config =
            b"format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n"
                .to_vec();
        config.extend(std::iter::repeat_n(b'#', 8 * 1024 * 1024));
        fs::write(root.join("v8project.yaml"), config).unwrap();
        write(&root.join("src/Configuration.xml"), "<MetaDataObject/>");

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "main",
            SourceSetKind::Configuration,
            SourceFormat::PlatformXml,
            &["src/Configuration.xml"],
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn transactional_provenance_keeps_accepting_large_source_marker() {
        let root = temp_workspace("unica-source-map-large-provenance-marker");
        write(
            &root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        );
        let marker = root.join("src/Configuration.xml");
        fs::create_dir_all(marker.parent().unwrap()).unwrap();
        fs::File::create(&marker)
            .unwrap()
            .set_len(9 * 1024 * 1024)
            .unwrap();

        let (map, provenance) = discover_project_source_map_with_provenance(&root).unwrap();
        assert_eq!(map.source_sets[0].source_format, SourceFormat::PlatformXml);
        provenance
            .bind_to(&mut CompileTransaction::new())
            .expect("unchanged large marker must remain a valid transaction precondition");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn injected_health_evidence_entry_allowance_refuses_without_partial_results() {
        let root = temp_workspace("unica-source-map-health-evidence-budget");
        write(
            &root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: epf\n",
        );
        for name in ["A.xml", "B.xml", "C.xml"] {
            write(
                &root.join("epf").join(name),
                "<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
            );
        }
        let mut checkpoint = || Ok(());
        let reader = ProjectDiscoveryReader::new(&root);
        let mut state =
            SourceMapDiscoveryState::health_with_evidence_limits(&reader, Some(2), None);

        let error = discover_project_source_map_internal(&root, &mut checkpoint, &mut state)
            .expect_err("the explicitly injected evidence-entry allowance must refuse");

        assert!(error.contains("format evidence"), "{error}");
        assert!(error.contains("2 entries"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn injected_health_evidence_byte_allowance_refuses_without_partial_results() {
        let fixture = tempfile::tempdir().unwrap();
        write(&fixture.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: epf\n");
        for name in ["A.xml", "B.xml"] {
            write(
                &fixture.path().join("epf").join(name),
                "<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
            );
        }
        let reader = ProjectDiscoveryReader::new(fixture.path());
        let mut state = SourceMapDiscoveryState::health_with_evidence_limits(
            &reader,
            None,
            Some("epf/A.xml".len()),
        );
        let error =
            discover_project_source_map_internal(fixture.path(), &mut || Ok(()), &mut state)
                .expect_err("the explicitly injected evidence-byte allowance must refuse");
        assert!(error.contains("format evidence"), "{error}");
        assert!(error.contains("9 bytes"), "{error}");
    }

    #[test]
    fn health_source_map_does_not_probe_through_linked_source_root() {
        use unica_workspace::test_support::{
            create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let root = temp_workspace("unica-source-map-health-linked-root");
        let external = temp_workspace("unica-source-map-health-external");
        write(
            &root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: epf\n",
        );
        write(
            &external.join("SecretPayroll.xml"),
            "<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
        );
        match create_directory_link_fixture_for_test(&external, root.join("epf")).unwrap() {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => return,
        }
        let mut checkpoint = || Ok(());

        let map = discover_project_source_map_controlled(&root, &mut checkpoint).unwrap();

        assert_eq!(map.source_sets.len(), 1);
        assert_eq!(
            map.source_sets[0].format_evidence,
            vec!["v8project.yaml:format=DESIGNER"]
        );
        assert!(!map.source_sets[0]
            .format_evidence
            .iter()
            .any(|evidence| evidence.contains("SecretPayroll")));
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(external).unwrap();
    }

    #[test]
    fn health_source_map_does_not_probe_through_linked_marker_parent() {
        use unica_workspace::test_support::{
            create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let root = temp_workspace("unica-source-map-health-linked-marker");
        let external = temp_workspace("unica-source-map-health-marker-external");
        write(
            &root.join("v8project.yaml"),
            "source-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        );
        fs::create_dir_all(root.join("src")).unwrap();
        write(
            &external.join("Configuration.mdo"),
            "<mdclass:Configuration secret='payroll'/>",
        );
        match create_directory_link_fixture_for_test(&external, root.join("src/Configuration"))
            .unwrap()
        {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => return,
        }
        let mut checkpoint = || Ok(());

        let result = discover_project_source_map_controlled(&root, &mut checkpoint);

        match result {
            Ok(map) => {
                assert_ne!(map.source_sets[0].source_format, SourceFormat::Edt);
                assert!(!map.source_sets[0]
                    .format_evidence
                    .iter()
                    .any(|evidence| evidence.contains("Configuration/Configuration.mdo")));
            }
            Err(reason) => assert!(!reason.contains("payroll"), "{reason}"),
        }
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(external).unwrap();
    }

    #[test]
    fn health_source_map_does_not_probe_absolute_external_source_root() {
        let root = temp_workspace("unica-source-map-health-absolute-root");
        let external = temp_workspace("unica-source-map-health-absolute-external");
        write(
            &external.join("SecretPayroll.xml"),
            "<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
        );
        write(
            &root.join("v8project.yaml"),
            &format!(
                "source-set:\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: {}\n",
                external.display()
            ),
        );
        let mut checkpoint = || Ok(());

        let map = discover_project_source_map_controlled(&root, &mut checkpoint).unwrap();

        assert!(map.source_sets[0].format_evidence.is_empty());
        assert_eq!(map.source_sets[0].source_format, SourceFormat::Unknown);
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(external).unwrap();
    }

    #[test]
    fn health_source_map_skips_deep_unknown_yaml_without_serde_materialization() {
        let root = temp_workspace("unica-source-map-health-unknown-yaml-depth");
        let depth = 512;
        let mut config = String::from(
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\nignored: ",
        );
        config.push_str(&"[".repeat(depth));
        config.push('0');
        config.push_str(&"]".repeat(depth));
        write(&root.join("v8project.yaml"), &config);
        write(&root.join("src/Configuration.xml"), "<MetaDataObject/>");
        let mut checkpoint = || Ok(());

        let map = discover_project_source_map_controlled(&root, &mut checkpoint)
            .expect("unknown deep YAML must not hide the useful source");
        assert_health_leaf_main(&map);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn health_source_map_skips_unknown_alias_expansion_without_materialization() {
        let root = temp_workspace("unica-source-map-health-unknown-alias-nodes");
        let anchored_nodes = 512;
        let alias_count = 128;
        let mut config = String::from(
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\ntemplate: &many [",
        );
        config.push_str(&"{},".repeat(anchored_nodes));
        config.push_str("]\nignored:\n");
        config.push_str(&"  - *many\n".repeat(alias_count));
        write(&root.join("v8project.yaml"), &config);
        write(&root.join("src/Configuration.xml"), "<MetaDataObject/>");
        let mut checkpoint = || Ok(());

        let map = discover_project_source_map_controlled(&root, &mut checkpoint)
            .expect("unknown aliases must not hide the useful source");
        assert_health_leaf_main(&map);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn health_source_map_skips_unknown_alias_depth_without_materialization() {
        let root = temp_workspace("unica-source-map-health-alias-expanded-depth");
        let anchor_depth = 180;
        let alias_site_depth = 100;
        let mut config = String::from(
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\ntemplate: &deep ",
        );
        config.push_str(&"[".repeat(anchor_depth));
        config.push('0');
        config.push_str(&"]".repeat(anchor_depth));
        config.push_str("\nignored: ");
        config.push_str(&"[".repeat(alias_site_depth));
        config.push_str("*deep");
        config.push_str(&"]".repeat(alias_site_depth));
        write(&root.join("v8project.yaml"), &config);
        write(&root.join("src/Configuration.xml"), "<MetaDataObject/>");
        let mut checkpoint = || Ok(());

        let map = discover_project_source_map_controlled(&root, &mut checkpoint)
            .expect("unknown alias depth must not hide the useful source");
        assert_health_leaf_main(&map);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn health_autodetect_does_not_follow_linked_marker_parent() {
        use unica_workspace::test_support::{
            create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let root = temp_workspace("unica-source-map-health-autodetect-linked");
        let external = temp_workspace("unica-source-map-health-autodetect-external");
        fs::create_dir_all(root.join("src")).unwrap();
        write(
            &external.join("Configuration.mdo"),
            "<mdclass:Configuration secret='payroll'/>",
        );
        match create_directory_link_fixture_for_test(&external, root.join("src/Configuration"))
            .unwrap()
        {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => return,
        }
        let mut checkpoint = || Ok(());

        let map = discover_project_source_map_controlled(&root, &mut checkpoint).unwrap();

        assert!(map.source_sets.is_empty(), "{:?}", map.source_sets);
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(external).unwrap();
    }

    #[test]
    fn autodetect_discovers_configuration_extension_source_sets() {
        let root = temp_workspace("unica-source-map-autodetect-extensions");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(
            &root.join("src/cfe/test_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );
        write(
            &root.join("src/cfe/test_edreg/Configuration.xml"),
            "<MetaDataObject/>",
        );
        // A stray file that is not itself an extension source root must not be reported.
        write(&root.join("src/cfe/README.md"), "not an extension");

        let map = discover_project_source_map(&root).unwrap();

        assert_eq!(map.source_sets.len(), 3, "{:?}", map.source_sets);
        assert_source_set(
            &map,
            "main",
            SourceSetKind::Configuration,
            SourceFormat::PlatformXml,
            &["src/cf/Configuration.xml"],
        );
        assert_source_set(
            &map,
            "test_ed",
            SourceSetKind::Extension,
            SourceFormat::PlatformXml,
            &["src/cfe/test_ed/Configuration.xml"],
        );
        assert_source_set(
            &map,
            "test_edreg",
            SourceSetKind::Extension,
            SourceFormat::PlatformXml,
            &["src/cfe/test_edreg/Configuration.xml"],
        );
        assert_eq!(map.effective_source_set.as_deref(), Some("main"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn health_autodetect_discovers_configuration_extension_source_sets() {
        let root = temp_workspace("unica-source-map-health-autodetect-extensions");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(
            &root.join("src/cfe/test_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );
        let mut checkpoint = || Ok(());

        let map = discover_project_source_map_controlled(&root, &mut checkpoint).unwrap();

        assert_eq!(map.source_sets.len(), 2, "{:?}", map.source_sets);
        assert_source_set(
            &map,
            "test_ed",
            SourceSetKind::Extension,
            SourceFormat::PlatformXml,
            &["src/cfe/test_ed/Configuration.xml"],
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn autodetect_without_extensions_directory_keeps_reporting_only_main() {
        let root = temp_workspace("unica-source-map-autodetect-no-extensions");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");

        let map = discover_project_source_map(&root).unwrap();

        assert_eq!(map.source_sets.len(), 1, "{:?}", map.source_sets);
        assert_eq!(map.source_sets[0].name, "main");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn health_autodetect_enforces_combined_source_set_limit_including_main() {
        let root = temp_workspace("unica-source-map-health-autodetect-combined-limit");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(
            &root.join("src/cfe/test_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );
        let mut checkpoint = || Ok(());
        let reader = ProjectDiscoveryReader::new(&root);
        let mut state = SourceMapDiscoveryState::health_with_source_set_limit(&reader, 1);

        let error = discover_project_source_map_internal(&root, &mut checkpoint, &mut state)
            .expect_err("main (1) + one autodetected extension (1) exceeds a limit of 1");

        assert!(error.contains("limit") || error.contains('1'), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn health_autodetect_accepts_combined_source_set_count_at_the_limit() {
        let root = temp_workspace("unica-source-map-health-autodetect-limit-boundary");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(
            &root.join("src/cfe/test_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );
        let mut checkpoint = || Ok(());
        let reader = ProjectDiscoveryReader::new(&root);
        let mut state = SourceMapDiscoveryState::health_with_source_set_limit(&reader, 2);

        let map = discover_project_source_map_internal(&root, &mut checkpoint, &mut state)
            .expect("main (1) + one autodetected extension (1) is exactly the limit of 2");

        assert_eq!(map.source_sets.len(), 2, "{:?}", map.source_sets);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn autodetect_skips_a_container_that_is_not_a_directory() {
        let root = temp_workspace("unica-source-map-autodetect-extensions-not-a-directory");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        // `src/cfe` exists as a regular file: a definite answer that no extension
        // container lives here, exactly like a link or an absent path. The layout
        // contributes nothing and the base configuration beside it is untouched —
        // collapsing the whole map instead would report an incomplete layout for a
        // workspace whose configuration is perfectly readable.
        write(&root.join("src/cfe"), "not a directory");

        assert_routes_agree(&root, &[("main", SourceSetKind::Configuration, "src/cf")]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn autodetect_propagates_an_unreadable_extensions_container() {
        use unica_workspace::test_support::set_unix_mode_for_test;

        let root = temp_workspace("unica-source-map-autodetect-extensions-unreadable");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(
            &root.join("src/cfe/test_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );
        let container = root.join("src/cfe");
        if !set_unix_mode_for_test(&container, 0o000).unwrap() {
            fs::remove_dir_all(&root).unwrap();
            return;
        }
        if std::fs::read_dir(&container).is_ok() {
            // Running with a privilege that ignores the mode; the case does not exist.
            set_unix_mode_for_test(&container, 0o755).unwrap();
            fs::remove_dir_all(&root).unwrap();
            return;
        }

        // A container that might hold extensions but could not be read is *not* a
        // definite "no extensions here". Reporting a map the filesystem never confirmed
        // would hide every extension behind one permission bit.
        let mut checkpoint = || Ok(());
        let ordinary = discover_project_source_map(&root)
            .expect_err("an unreadable extensions container must not be reported as empty");
        let hardened = discover_project_source_map_controlled(&root, &mut checkpoint)
            .expect_err("the hardened route must not report an unreadable container as empty");

        assert!(ordinary.contains("cfe"), "{ordinary}");
        assert!(
            hardened.contains("cfe") || hardened.contains("directory"),
            "{hardened}"
        );
        set_unix_mode_for_test(&container, 0o755).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn autodetect_skips_extension_directory_named_main() {
        let root = temp_workspace("unica-source-map-autodetect-extension-named-main");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        // An extension directory literally named `main` would otherwise collide with the
        // base configuration's reserved name: named lookups (`resolve_named_source_set`)
        // would reject it as ambiguous while default selection would silently pick whichever
        // came first. Autodetection must not produce that collision.
        write(
            &root.join("src/cfe/main/Configuration.xml"),
            "<MetaDataObject/>",
        );

        let map = discover_project_source_map(&root).unwrap();

        assert_eq!(map.source_sets.len(), 1, "{:?}", map.source_sets);
        assert_eq!(map.source_sets[0].kind, SourceSetKind::Configuration);
        fs::remove_dir_all(root).unwrap();
    }

    /// Both discovery routes must report the same source map for the same workspace.
    /// The ordinary and the hardened route are two implementations of one policy, and
    /// the defects this matrix pins down were all route drift: a marker list that grew
    /// on one side only, an entry filter that existed on one side only. Asserting the
    /// two agree on every supported layout is what keeps them from drifting again.
    fn assert_routes_agree(root: &Path, expected: &[(&str, SourceSetKind, &str)]) {
        let mut checkpoint = || Ok(());
        let ordinary = discover_project_source_map(root).unwrap_or_else(|error| {
            panic!("ordinary route failed for {}: {error}", root.display())
        });
        let hardened = discover_project_source_map_controlled(root, &mut checkpoint)
            .unwrap_or_else(|error| {
                panic!("hardened route failed for {}: {error}", root.display())
            });

        let render = |map: &ProjectSourceMap| {
            map.source_sets
                .iter()
                .map(|source_set| {
                    (
                        source_set.name.clone(),
                        source_set.kind,
                        source_set.path.clone(),
                    )
                })
                .collect::<Vec<_>>()
        };
        let expected = expected
            .iter()
            .map(|(name, kind, path)| ((*name).to_string(), *kind, (*path).to_string()))
            .collect::<Vec<_>>();
        assert_eq!(render(&ordinary), expected, "ordinary route");
        assert_eq!(render(&hardened), expected, "hardened route");
    }

    #[test]
    fn autodetect_skips_entries_that_are_not_extension_directories() {
        let root = temp_workspace("unica-source-map-autodetect-extensions-stray-entries");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(
            &root.join("src/cfe/test_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );
        // A container holds whatever the workspace put there. `.gitkeep` is how git
        // carries an otherwise empty directory, `.DS_Store` appears from one Finder
        // visit, and neither is an extension. Discovery classifies and skips them; a
        // route that aborts on the first of them reports an incomplete layout for a
        // workspace whose layout is complete.
        write(&root.join("src/cfe/.gitkeep"), "");
        write(&root.join("src/cfe/.DS_Store"), "\0\0");
        write(&root.join("src/cfe/README.md"), "not an extension");

        assert_routes_agree(
            &root,
            &[
                ("main", SourceSetKind::Configuration, "src/cf"),
                ("test_ed", SourceSetKind::Extension, "src/cfe/test_ed"),
            ],
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn autodetect_skips_linked_entries_in_the_extensions_container() {
        use unica_workspace::test_support::{
            create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let root = temp_workspace("unica-source-map-autodetect-extensions-link");
        let external = temp_workspace("unica-source-map-autodetect-extensions-link-external");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(
            &root.join("src/cfe/test_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );
        write(&external.join("Configuration.xml"), "<MetaDataObject/>");
        fs::create_dir_all(root.join("src/cfe")).unwrap();
        match create_directory_link_fixture_for_test(&external, root.join("src/cfe/escaped"))
            .unwrap()
        {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
                fs::remove_dir_all(&root).unwrap();
                fs::remove_dir_all(&external).unwrap();
                return;
            }
        }

        // A linked source root is rejected by `resolve_named_source_set`, so publishing
        // one would advertise a source set every `unica.code.*` tool then refuses to
        // open. It is not a candidate on either route, and it does not abort discovery
        // of the real extension beside it.
        assert_routes_agree(
            &root,
            &[
                ("main", SourceSetKind::Configuration, "src/cf"),
                ("test_ed", SourceSetKind::Extension, "src/cfe/test_ed"),
            ],
        );
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(external).unwrap();
    }

    #[test]
    fn autodetect_skips_a_linked_extensions_container() {
        use unica_workspace::test_support::{
            create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let root = temp_workspace("unica-source-map-autodetect-linked-container");
        let external = temp_workspace("unica-source-map-autodetect-linked-container-external");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(
            &external.join("test_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );
        fs::create_dir_all(root.join("src")).unwrap();
        match create_directory_link_fixture_for_test(&external, root.join("src/cfe")).unwrap() {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
                fs::remove_dir_all(&root).unwrap();
                fs::remove_dir_all(&external).unwrap();
                return;
            }
        }

        // The container itself is a link, so there is no real container here and no
        // extension to autodetect. Enumerating through it would publish source roots
        // outside the workspace that `resolve_named_source_set` then refuses, and
        // failing would hide the base configuration that is perfectly fine.
        assert_routes_agree(&root, &[("main", SourceSetKind::Configuration, "src/cf")]);
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(external).unwrap();
    }

    #[test]
    fn autodetect_discovers_a_single_extension_at_the_container_root() {
        let root = temp_workspace("unica-source-map-autodetect-extension-at-container-root");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        // The canonical layout of this repository: `src/cfe` *is* one extension's dump
        // root (`ExtensionPath: "src/cfe"` in the cfe-* skills, and the `path: src/cfe`
        // fixture in `tests/ci/test_unica_mcp_script_parity.py`).
        write(&root.join("src/cfe/Configuration.xml"), "<MetaDataObject/>");

        assert_routes_agree(
            &root,
            &[
                ("main", SourceSetKind::Configuration, "src/cf"),
                ("cfe", SourceSetKind::Extension, "src/cfe"),
            ],
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn autodetect_at_the_container_root_does_not_report_object_directories_as_extensions() {
        let root = temp_workspace("unica-source-map-autodetect-container-root-objects");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(&root.join("src/cfe/Configuration.xml"), "<MetaDataObject/>");
        // When the container is itself an extension root, its children are that
        // extension's object directories, not sibling extensions — exactly as the base
        // configuration scan stops at its first match.
        write(
            &root.join("src/cfe/Catalogs/Товары/Configuration.xml"),
            "<MetaDataObject/>",
        );

        assert_routes_agree(
            &root,
            &[
                ("main", SourceSetKind::Configuration, "src/cf"),
                ("cfe", SourceSetKind::Extension, "src/cfe"),
            ],
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn autodetect_discovers_extensions_under_the_named_extensions_container() {
        let root = temp_workspace("unica-source-map-autodetect-extensions-container");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        // The second documented layout of this repository: `src/extensions/<Name>`
        // (`cfe-borrow`, `cfe-init` and `cfe-patch-method` all address it that way).
        write(
            &root.join("src/extensions/MyExtension/Configuration.xml"),
            "<MetaDataObject/>",
        );

        assert_routes_agree(
            &root,
            &[
                ("main", SourceSetKind::Configuration, "src/cf"),
                (
                    "MyExtension",
                    SourceSetKind::Extension,
                    "src/extensions/MyExtension",
                ),
            ],
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn autodetect_discovers_edt_extension_source_sets() {
        let root = temp_workspace("unica-source-map-autodetect-edt-extensions");
        // The base scan recognises three markers; an extension scan that recognised
        // fewer would find a configuration in EDT layout and miss the identical
        // extension beside it.
        write(
            &root.join("src/cf/src/Configuration/Configuration.mdo"),
            "<mdclass:Configuration/>",
        );
        write(
            &root.join("src/cfe/test_ed/src/Configuration/Configuration.mdo"),
            "<mdclass:Configuration/>",
        );

        assert_routes_agree(
            &root,
            &[
                ("main", SourceSetKind::Configuration, "src/cf"),
                ("test_ed", SourceSetKind::Extension, "src/cfe/test_ed"),
            ],
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn autodetect_publishes_an_extension_named_main_when_nothing_else_claims_the_name() {
        let root = temp_workspace("unica-source-map-autodetect-only-extension-named-main");
        // `main` is reserved for the base configuration, but reserving it when no base
        // configuration was found empties the map: the workspace's only source root
        // disappears and `unica.project.map` reports nothing at all.
        write(
            &root.join("src/cfe/main/Configuration.xml"),
            "<MetaDataObject/>",
        );

        assert_routes_agree(&root, &[("main", SourceSetKind::Extension, "src/cfe/main")]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn autodetect_gives_the_reserved_name_to_one_claimant_only() {
        let root = temp_workspace("unica-source-map-autodetect-two-extensions-named-main");
        // No base configuration, and two layouts both derive `main`. Exactly one entry
        // may carry the reserved name: two would make `sourceSet: "main"` ambiguous for
        // named lookup while `select_default_source_set` silently kept resolving it to
        // whichever came first. The name goes to the first claimant in catalog order —
        // a fixed, documented order rather than a filesystem accident.
        write(
            &root.join("src/cfe/main/Configuration.xml"),
            "<MetaDataObject/>",
        );
        write(
            &root.join("src/extensions/main/Configuration.xml"),
            "<MetaDataObject/>",
        );

        assert_routes_agree(&root, &[("main", SourceSetKind::Extension, "src/cfe/main")]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn autodetect_keeps_a_non_utf8_extension_name_from_hiding_its_siblings() {
        // The fixture lives behind the platform facade (ADR-0009): only some platforms
        // and filesystems accept a name that is not valid UTF-8, and none of that
        // knowledge belongs in a discovery test.
        let Some(invalid_component) =
            unica_workspace::test_support::non_utf8_relative_path_for_test()
        else {
            return;
        };

        let root = temp_workspace("unica-source-map-autodetect-non-utf8-extension");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(
            &root.join("src/cfe/test_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );
        // A source-set name is a JSON string, so a directory name that is not valid
        // UTF-8 cannot become one. It is skipped deliberately — and a sibling that
        // cannot be named must not hide the ones that can.
        let invalid = root.join("src/cfe").join(&invalid_component);
        // APFS and NTFS refuse to store the name at all; the case only exists on a
        // filesystem that accepts arbitrary bytes, so skip where it cannot be built.
        if fs::create_dir_all(&invalid).is_err() {
            fs::remove_dir_all(&root).unwrap();
            return;
        }
        write(&invalid.join("Configuration.xml"), "<MetaDataObject/>");

        assert_routes_agree(
            &root,
            &[
                ("main", SourceSetKind::Configuration, "src/cf"),
                ("test_ed", SourceSetKind::Extension, "src/cfe/test_ed"),
            ],
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn transactional_autodetect_binds_the_extensions_container_listing() {
        let root = temp_workspace("unica-source-map-autodetect-container-provenance");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(
            &root.join("src/cfe/test_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );

        let (map, provenance) = discover_project_source_map_with_provenance(&root).unwrap();
        assert_eq!(map.source_sets.len(), 2, "{:?}", map.source_sets);
        // Discovery now depends on the *listing* of the container, not only on files it
        // read. A concurrent checkout that adds an extension between discovery and
        // commit changes the computed map while every recorded file input still agrees,
        // so the listing itself has to be bound to the transaction.
        write(
            &root.join("src/cfe/late_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );

        let error = provenance
            .bind_to(&mut CompileTransaction::new())
            .expect_err("an extension that appeared after discovery must invalidate the map");

        assert!(error.contains("cfe"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    /// The scenario matrix is derived from the production layout and marker
    /// catalogs. Exact catalog assertions make an intentional new layout a
    /// review event instead of allowing it to bypass the near-miss matrix.
    #[test]
    fn autodetect_catalog_contract_is_closed_over_production_layouts() {
        assert_eq!(BASE_CONFIGURATION_LAYOUTS, [".", "src", "src/cf"]);
        assert_eq!(
            SOURCE_ROOT_MARKERS,
            [
                "Configuration.xml",
                "Configuration/Configuration.mdo",
                "src/Configuration/Configuration.mdo",
            ]
        );
        assert_eq!(EXTENSION_LAYOUTS.len(), 2);
        assert_eq!(EXTENSION_LAYOUTS[0].directory, "src/cfe");
        assert_eq!(
            EXTENSION_LAYOUTS[0].shape,
            ExtensionLayoutShape::RootOrContainer
        );
        assert_eq!(EXTENSION_LAYOUTS[1].directory, "src/extensions");
        assert_eq!(EXTENSION_LAYOUTS[1].shape, ExtensionLayoutShape::Container);

        for layout in BASE_CONFIGURATION_LAYOUTS {
            for marker in SOURCE_ROOT_MARKERS {
                let root = temp_workspace(&format!(
                    "unica-source-map-catalog-base-{}-{}",
                    layout.replace(['.', '/'], "root"),
                    marker.replace('/', "-")
                ));
                write(&root.join(layout).join(marker), "<marker/>");
                let winning_layout = BASE_CONFIGURATION_LAYOUTS
                    .iter()
                    .find(|candidate| {
                        SOURCE_ROOT_MARKERS.iter().any(|candidate_marker| {
                            root.join(candidate).join(candidate_marker).is_file()
                        })
                    })
                    .expect("the written marker belongs to a catalog layout");
                assert_routes_agree(
                    &root,
                    &[("main", SourceSetKind::Configuration, *winning_layout)],
                );
                fs::remove_dir_all(root).unwrap();
            }
        }

        for layout in EXTENSION_LAYOUTS {
            for marker in SOURCE_ROOT_MARKERS {
                let root = temp_workspace(&format!(
                    "unica-source-map-catalog-extension-{}-{}",
                    layout.directory.replace('/', "-"),
                    marker.replace('/', "-")
                ));
                let child = root.join(layout.directory).join("Named");
                write(&child.join(marker), "<marker/>");
                assert_routes_agree(
                    &root,
                    &[(
                        "Named",
                        SourceSetKind::Extension,
                        &format!("{}/Named", layout.directory),
                    )],
                );
                fs::remove_dir_all(root).unwrap();
            }
        }

        for near_miss in ["sources", "src/cfes", "src/extension"] {
            let root = temp_workspace(&format!(
                "unica-source-map-catalog-near-miss-{}",
                near_miss.replace('/', "-")
            ));
            write(
                &root.join(near_miss).join("Named/Configuration.xml"),
                "<marker/>",
            );
            assert_routes_agree(&root, &[]);
            fs::remove_dir_all(root).unwrap();
        }

        autodetect_skips_a_container_that_is_not_a_directory();
        autodetect_propagates_an_unreadable_extensions_container();
        autodetect_skips_entries_that_are_not_extension_directories();
        autodetect_skips_linked_entries_in_the_extensions_container();
        autodetect_skips_a_linked_extensions_container();
        autodetect_discovers_a_single_extension_at_the_container_root();
        autodetect_at_the_container_root_does_not_report_object_directories_as_extensions();
        autodetect_discovers_extensions_under_the_named_extensions_container();
        autodetect_gives_the_reserved_name_to_one_claimant_only();
        transactional_autodetect_binds_the_extensions_container_listing();
    }

    #[test]
    fn transactional_autodetect_binds_an_absent_extensions_container() {
        let root = temp_workspace("unica-source-map-autodetect-absent-container-provenance");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");

        let (map, provenance) = discover_project_source_map_with_provenance(&root).unwrap();
        assert_eq!(map.source_sets.len(), 1, "{:?}", map.source_sets);
        // The absence of the container is an input too: a checkout that creates it wins
        // a source set that discovery never saw.
        write(
            &root.join("src/cfe/late_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );

        let error = provenance
            .bind_to(&mut CompileTransaction::new())
            .expect_err(
                "an extensions container that appeared after discovery invalidates the map",
            );

        assert!(error.contains("cfe"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn transactional_autodetect_tolerates_a_link_beside_the_extensions() {
        use unica_workspace::test_support::{
            create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let root = temp_workspace("unica-source-map-autodetect-container-provenance-link");
        let external = temp_workspace("unica-source-map-autodetect-container-provenance-external");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(
            &root.join("src/cfe/test_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );
        fs::create_dir_all(external.join("payload")).unwrap();
        match create_directory_link_fixture_for_test(&external, root.join("src/cfe/linked"))
            .unwrap()
        {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
                fs::remove_dir_all(&root).unwrap();
                fs::remove_dir_all(&external).unwrap();
                return;
            }
        }

        let (map, provenance) = discover_project_source_map_with_provenance(&root).unwrap();

        assert_eq!(map.source_sets.len(), 2, "{:?}", map.source_sets);
        // Discovery skipped the link, so the guard must not refuse to represent the
        // container that holds it: a guard that cannot be taken is a guarantee that is
        // silently absent.
        provenance
            .bind_to(&mut CompileTransaction::new())
            .expect("a link beside the extensions must not make the container unguardable");
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(external).unwrap();
    }

    #[test]
    fn transactional_autodetect_binds_a_container_path_that_is_not_a_directory() {
        let root = temp_workspace("unica-source-map-autodetect-file-container-provenance");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(&root.join("src/cfe"), "not a directory");

        let (map, provenance) = discover_project_source_map_with_provenance(&root).unwrap();

        assert_eq!(map.source_sets.len(), 1, "{:?}", map.source_sets);
        // Discovery read "no child directories here" from that path, and the guard has
        // to be able to say exactly that. A guard that cannot be taken turns every
        // compile transaction in the workspace into a planning failure.
        provenance
            .bind_to(&mut CompileTransaction::new())
            .expect("a non-directory container must still be representable as a guard");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn transactional_autodetect_binds_a_linked_container_path() {
        use unica_workspace::test_support::{
            create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let root = temp_workspace("unica-source-map-autodetect-linked-container-provenance");
        let external = temp_workspace("unica-source-map-autodetect-linked-container-prov-external");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        fs::create_dir_all(external.join("test_ed")).unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        match create_directory_link_fixture_for_test(&external, root.join("src/cfe")).unwrap() {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
                fs::remove_dir_all(&root).unwrap();
                fs::remove_dir_all(&external).unwrap();
                return;
            }
        }

        let (map, provenance) = discover_project_source_map_with_provenance(&root).unwrap();

        assert_eq!(map.source_sets.len(), 1, "{:?}", map.source_sets);
        provenance
            .bind_to(&mut CompileTransaction::new())
            .expect("a linked container must still be representable as a guard");
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(external).unwrap();
    }

    #[test]
    fn transactional_autodetect_catches_a_container_path_that_became_a_directory() {
        let root = temp_workspace("unica-source-map-autodetect-file-container-became-dir");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        let container = root.join("src/cfe");
        write(&container, "not a directory");

        let (_, provenance) = discover_project_source_map_with_provenance(&root).unwrap();
        // Recording "holds no child directories" for a non-directory path is only safe
        // while the transition that *would* change the map is still caught: replacing
        // the file with a real container wins source sets discovery never saw.
        fs::remove_file(&container).unwrap();
        write(
            &container.join("late_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );

        let error = provenance
            .bind_to(&mut CompileTransaction::new())
            .expect_err("a container that appeared after discovery must invalidate the map");

        assert!(error.contains("cfe"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn transactional_autodetect_tolerates_stray_entries_beside_the_extensions() {
        let root = temp_workspace("unica-source-map-autodetect-container-provenance-stray");
        write(&root.join("src/cf/Configuration.xml"), "<MetaDataObject/>");
        write(
            &root.join("src/cfe/test_ed/Configuration.xml"),
            "<MetaDataObject/>",
        );
        write(&root.join("src/cfe/.gitkeep"), "");

        let (_, provenance) = discover_project_source_map_with_provenance(&root).unwrap();

        // The guard binds what discovery consumed — which direct children are
        // directories — and nothing else. A stray file beside the extensions is not an
        // input, so it must not turn every compile transaction into a conflict.
        provenance
            .bind_to(&mut CompileTransaction::new())
            .expect("a stray file beside the extensions is not a discovery input");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn detects_edt_configuration_and_platform_external_processor_source_sets() {
        let root = temp_workspace("unica-source-map-multi");
        write(
            &root.join("v8project.yaml"),
            r#"
format: EDT
source-set:
  - name: main
    type: CONFIGURATION
    path: src
  - name: external-processors
    type: EXTERNAL_DATA_PROCESSORS
    path: epf
"#,
        );
        write(&root.join("src/.project"), "<projectDescription/>");
        write(
            &root.join("src/Configuration/Configuration.mdo"),
            "<mdclass:Configuration/>",
        );
        write(
            &root.join("epf/PriceLoader.xml"),
            "<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
        );
        write(&root.join("epf/ConfigDumpInfo.xml"), "<ConfigDumpInfo/>");

        let map = discover_project_source_map(&root).unwrap();

        assert_eq!(map.source_sets.len(), 2);
        assert_source_set(
            &map,
            "main",
            SourceSetKind::Configuration,
            SourceFormat::Edt,
            &["src/.project", "src/Configuration/Configuration.mdo"],
        );
        assert_source_set(
            &map,
            "external-processors",
            SourceSetKind::ExternalProcessor,
            SourceFormat::PlatformXml,
            &["epf/PriceLoader.xml"],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn controlled_discovery_accepts_uppercase_external_xml_extension() {
        let root = temp_workspace("unica-source-map-uppercase-external-xml");
        write(
            &root.join("v8project.yaml"),
            r#"
format: EDT
source-set:
  - name: external-reports
    type: EXTERNAL_REPORTS
    path: erf
"#,
        );
        write(
            &root.join("erf/Report.XML"),
            "<MetaDataObject><ExternalReport/></MetaDataObject>",
        );
        let mut checkpoint = || Ok(());

        let map = discover_project_source_map_controlled(&root, &mut checkpoint).unwrap();

        assert_source_set(
            &map,
            "external-reports",
            SourceSetKind::ExternalReport,
            SourceFormat::PlatformXml,
            &["erf/Report.XML"],
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    pub(crate) fn actor_admission_preserves_declared_external_processor_and_report_map() {
        let root = temp_workspace("unica-source-map-actor-external-map");
        write(
            &root.join("v8project.yaml"),
            r#"
format: DESIGNER
source-set:
  - name: processors
    type: EXTERNAL_DATA_PROCESSORS
    path: epf
  - name: reports
    type: EXTERNAL_REPORTS
    path: erf
"#,
        );
        write(
            &root.join("epf/PriceLoader.xml"),
            "<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
        );
        write(
            &root.join("erf/Sales.XML"),
            "<MetaDataObject><ExternalReport/></MetaDataObject>",
        );
        let mut checkpoint = || Ok(());

        let admission = discover_project_source_admission(&root, &mut checkpoint).unwrap();

        assert_source_set(
            admission.map(),
            "processors",
            SourceSetKind::ExternalProcessor,
            SourceFormat::PlatformXml,
            &["epf/PriceLoader.xml"],
        );
        assert_source_set(
            admission.map(),
            "reports",
            SourceSetKind::ExternalReport,
            SourceFormat::PlatformXml,
            &["erf/Sales.XML"],
        );
        assert!(admission.source_root_identity(Path::new("epf")).is_some());
        assert!(admission.source_root_identity(Path::new("erf")).is_some());
        admission
            .into_evidence()
            .validate(
                unica_work_control::provider_deadline::ProviderDeadline::from_budget(
                    std::time::Duration::from_secs(5),
                ),
                &unica_work_control::cancellation::CancellationToken::new(),
            )
            .unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    pub(crate) fn actor_admission_external_config_dump_info_content_change_invalidates_evidence() {
        let root = temp_workspace("unica-source-map-actor-external-content-change");
        write(
            &root.join("v8project.yaml"),
            r#"
format: DESIGNER
source-set:
  - name: processors
    type: EXTERNAL_DATA_PROCESSORS
    path: epf
"#,
        );
        let descriptor = root.join("epf/ConfigDumpInfo.xml");
        write(
            &descriptor,
            "<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
        );
        let mut checkpoint = || Ok(());
        let admission = discover_project_source_admission(&root, &mut checkpoint).unwrap();
        write(
            &descriptor,
            "<MetaDataObject><ExternalReport/></MetaDataObject>",
        );

        let error = admission
            .into_evidence()
            .validate(
                unica_work_control::provider_deadline::ProviderDeadline::from_budget(
                    std::time::Duration::from_secs(5),
                ),
                &unica_work_control::cancellation::CancellationToken::new(),
            )
            .unwrap_err();

        assert_eq!(
            error.kind(),
            unica_workspace::selection_evidence::SourceSelectionEvidenceErrorKind::Changed
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    pub(crate) fn actor_admission_external_descriptor_absence_to_appearance_invalidates_evidence() {
        let root = temp_workspace("unica-source-map-actor-external-absence-change");
        write(
            &root.join("v8project.yaml"),
            r#"
format: DESIGNER
source-set:
  - name: reports
    type: EXTERNAL_REPORTS
    path: erf
"#,
        );
        fs::create_dir_all(root.join("erf")).unwrap();
        let mut checkpoint = || Ok(());
        let admission = discover_project_source_admission(&root, &mut checkpoint).unwrap();
        write(
            &root.join("erf/Appeared.xml"),
            "<MetaDataObject><ExternalReport/></MetaDataObject>",
        );

        let error = admission
            .into_evidence()
            .validate(
                unica_work_control::provider_deadline::ProviderDeadline::from_budget(
                    std::time::Duration::from_secs(5),
                ),
                &unica_work_control::cancellation::CancellationToken::new(),
            )
            .unwrap_err();

        assert_eq!(
            error.kind(),
            unica_workspace::selection_evidence::SourceSelectionEvidenceErrorKind::Changed
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn controlled_discovery_skips_directory_with_external_xml_name() {
        let root = temp_workspace("unica-source-map-external-xml-directory");
        write(
            &root.join("v8project.yaml"),
            r#"
source-set:
  - name: external-reports
    type: EXTERNAL_REPORTS
    path: erf
"#,
        );
        write(
            &root.join("erf/Report.xml"),
            "<MetaDataObject><ExternalReport/></MetaDataObject>",
        );
        fs::create_dir_all(root.join("erf/Archive.xml")).unwrap();
        let mut checkpoint = || Ok(());

        let map = discover_project_source_map_controlled(&root, &mut checkpoint).unwrap();

        assert_source_set(
            &map,
            "external-reports",
            SourceSetKind::ExternalReport,
            SourceFormat::PlatformXml,
            &["erf/Report.xml"],
        );
        assert!(map.source_sets[0].format_probe_error.is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn config_dump_info_alone_is_not_external_source_format_evidence() {
        let root = temp_workspace("unica-source-map-external-cdfi-runtime-state");
        write(
            &root.join("v8project.yaml"),
            r#"
source-set:
  - name: external-processors
    type: EXTERNAL_DATA_PROCESSORS
    path: epf
  - name: external-reports
    type: EXTERNAL_REPORTS
    path: erf
"#,
        );
        write(&root.join("epf/ConfigDumpInfo.xml"), "<ConfigDumpInfo/>");
        write(&root.join("erf/configdumpinfo.xml"), "<ConfigDumpInfo/>");

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "external-processors",
            SourceSetKind::ExternalProcessor,
            SourceFormat::Unknown,
            &[],
        );
        assert_source_set(
            &map,
            "external-reports",
            SourceSetKind::ExternalReport,
            SourceFormat::Unknown,
            &[],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn external_object_named_config_dump_info_remains_platform_xml_evidence() {
        let root = temp_workspace("unica-source-map-external-object-named-cdfi");
        write(
            &root.join("v8project.yaml"),
            r#"
source-set:
  - name: external-processors
    type: EXTERNAL_DATA_PROCESSORS
    path: epf
"#,
        );
        write(
            &root.join("epf/ConfigDumpInfo.xml"),
            "<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
        );

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "external-processors",
            SourceSetKind::ExternalProcessor,
            SourceFormat::PlatformXml,
            &["epf/ConfigDumpInfo.xml"],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn nested_external_tag_does_not_make_config_dump_info_source_evidence() {
        let root = temp_workspace("unica-source-map-nested-external-tag-cdfi");
        write(
            &root.join("v8project.yaml"),
            r#"
source-set:
  - name: external-processors
    type: EXTERNAL_DATA_PROCESSORS
    path: epf
"#,
        );
        write(
            &root.join("epf/ConfigDumpInfo.xml"),
            "<MetaDataObject><Properties><ExternalDataProcessor/></Properties></MetaDataObject>",
        );

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "external-processors",
            SourceSetKind::ExternalProcessor,
            SourceFormat::Unknown,
            &[],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_config_dump_info_is_not_external_source_format_evidence() {
        let root = temp_workspace("unica-source-map-malformed-external-cdfi");
        write(
            &root.join("v8project.yaml"),
            r#"
source-set:
  - name: external-reports
    type: EXTERNAL_REPORTS
    path: erf
"#,
        );
        write(
            &root.join("erf/ConfigDumpInfo.xml"),
            "<<<<<<< ours\n<ConfigDumpInfo/>\n=======\n<ConfigDumpInfo/>\n>>>>>>> theirs",
        );

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "external-reports",
            SourceSetKind::ExternalReport,
            SourceFormat::Unknown,
            &[],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn symlinked_config_dump_info_is_not_external_source_format_evidence() {
        let root = temp_workspace("unica-source-map-symlinked-external-cdfi");
        write(
            &root.join("v8project.yaml"),
            r#"
source-set:
  - name: external-processors
    type: EXTERNAL_DATA_PROCESSORS
    path: epf
"#,
        );
        write(
            &root.join("outside.xml"),
            "<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
        );
        fs::create_dir_all(root.join("epf")).unwrap();
        let Some(symlink_result) = unica_workspace::test_support::create_file_symlink_for_test(
            root.join("outside.xml"),
            root.join("epf/ConfigDumpInfo.xml"),
        ) else {
            return;
        };
        symlink_result.unwrap();

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "external-processors",
            SourceSetKind::ExternalProcessor,
            SourceFormat::Unknown,
            &[],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn detects_single_platform_configuration_source_set() {
        let root = temp_workspace("unica-source-map-platform");
        write(
            &root.join("v8project.yaml"),
            r#"
format: DESIGNER
source-set:
  - name: main
    type: CONFIGURATION
    path: src
"#,
        );
        write(&root.join("src/Configuration.xml"), "<MetaDataObject/>");
        write(&root.join("src/ConfigDumpInfo.xml"), "<ConfigDumpInfo/>");

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "main",
            SourceSetKind::Configuration,
            SourceFormat::PlatformXml,
            &["src/Configuration.xml"],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn config_dump_info_is_not_platform_source_format_evidence() {
        let root = temp_workspace("unica-source-map-cdfi-runtime-state");
        write(
            &root.join("v8project.yaml"),
            r#"
source-set:
  - name: main
    type: CONFIGURATION
    path: src
"#,
        );
        write(&root.join("src/ConfigDumpInfo.xml"), "<ConfigDumpInfo/>");

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "main",
            SourceSetKind::Configuration,
            SourceFormat::Unknown,
            &[],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ignores_legacy_v8tr_config_environment_override() {
        let root = temp_workspace("unica-source-map-ignore-v8tr-config");
        write(
            &root.join("v8project.yaml"),
            r#"
format: DESIGNER
source-set:
  - name: main
    type: CONFIGURATION
    path: src
"#,
        );
        write(
            &root.join("custom.yaml"),
            r#"
format: DESIGNER
source-set:
  - name: env
    type: CONFIGURATION
    path: env-src
"#,
        );
        write(&root.join("src/Configuration.xml"), "<MetaDataObject/>");
        write(&root.join("env-src/Configuration.xml"), "<MetaDataObject/>");
        let _guard = EnvVarGuard::set("V8TR_CONFIG", root.join("custom.yaml"));

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "main",
            SourceSetKind::Configuration,
            SourceFormat::PlatformXml,
            &["src/Configuration.xml"],
        );
        assert!(
            map.source_sets
                .iter()
                .all(|source_set| source_set.name != "env"),
            "legacy V8TR_CONFIG source set must be ignored: {map:?}"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn detects_single_edt_configuration_source_set() {
        let root = temp_workspace("unica-source-map-edt");
        write(
            &root.join("v8project.yaml"),
            r#"
format: EDT
source-set:
  - name: main
    type: CONFIGURATION
    path: src
"#,
        );
        write(&root.join("src/.project"), "<projectDescription/>");
        write(
            &root.join("src/Configuration/Configuration.mdo"),
            "<mdclass:Configuration/>",
        );

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "main",
            SourceSetKind::Configuration,
            SourceFormat::Edt,
            &["src/.project", "src/Configuration/Configuration.mdo"],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn conflicting_markers_inside_one_source_set_are_invalid_not_mixed() {
        let root = temp_workspace("unica-source-map-invalid");
        write(
            &root.join("v8project.yaml"),
            r#"
source-set:
  - name: main
    type: CONFIGURATION
    path: src
"#,
        );
        write(&root.join("src/Configuration.xml"), "<MetaDataObject/>");
        write(
            &root.join("src/Configuration/Configuration.mdo"),
            "<mdclass:Configuration/>",
        );

        let map = discover_project_source_map(&root).unwrap();

        assert_source_set(
            &map,
            "main",
            SourceSetKind::Configuration,
            SourceFormat::Invalid,
            &[
                "src/Configuration.xml",
                "src/Configuration/Configuration.mdo",
            ],
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configured_source_provenance_rejects_late_edt_marker() {
        let root = temp_workspace("unica-source-map-late-edt-marker");
        write(
            &root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        );
        write(&root.join("src/Configuration.xml"), "<MetaDataObject/>");

        let (map, provenance) = discover_project_source_map_with_provenance(&root).unwrap();
        assert_source_set(
            &map,
            "main",
            SourceSetKind::Configuration,
            SourceFormat::PlatformXml,
            &["src/Configuration.xml"],
        );
        write(&root.join("src/.project"), "<projectDescription/>");

        let error = provenance
            .bind_to(&mut CompileTransaction::new())
            .expect_err("an EDT marker absent during classification must stay absent");

        assert!(error.contains(".project"), "{error}");
        assert!(error.contains("absence guard"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configured_source_provenance_rejects_changed_edt_marker() {
        let root = temp_workspace("unica-source-map-changed-edt-marker");
        write(
            &root.join("v8project.yaml"),
            "format: EDT\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        );
        let marker = root.join("src/.project");
        write(&marker, "<projectDescription/>");

        let (_, provenance) = discover_project_source_map_with_provenance(&root).unwrap();
        write(
            &marker,
            "<projectDescription><name>changed</name></projectDescription>",
        );

        let error = provenance
            .bind_to(&mut CompileTransaction::new())
            .expect_err("the exact EDT marker bytes must stay unchanged");

        assert!(error.contains(".project"), "{error}");
        assert!(error.contains("changed"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn effective_source_root_rejects_relative_workspace_escape() {
        let root = temp_workspace("unica-source-map-relative-escape");
        write(
            &root.join("v8project.yaml"),
            "source-set:\n  - name: main\n    type: CONFIGURATION\n    path: ../outside\n",
        );

        let map = discover_project_source_map(&root).unwrap();

        assert!(map.effective_source_set.is_none());
        assert!(map.effective_source_root.is_none());
        assert!(map
            .source_selection_error
            .as_deref()
            .is_some_and(
                |error| error.starts_with("invalid_source_root:") && error.contains("workspace")
            ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn effective_source_root_rejects_absolute_workspace_escape() {
        let root = temp_workspace("unica-source-map-absolute-escape");
        let outside = temp_workspace("unica-source-map-outside");
        write(
            &root.join("v8project.yaml"),
            &format!(
                "source-set:\n  - name: main\n    type: CONFIGURATION\n    path: {}\n",
                outside.display()
            ),
        );

        let map = discover_project_source_map(&root).unwrap();

        assert!(map.effective_source_root.is_none());
        assert!(map
            .source_selection_error
            .as_deref()
            .is_some_and(|error| error.starts_with("invalid_source_root:")));
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn selection_errors_use_the_stable_invalid_source_root_prefix() {
        let ambiguous = temp_workspace("unica-source-map-ambiguous-prefix");
        write(&ambiguous.join("v8project.yaml"), "source-set:\n  - name: app\n    type: CONFIGURATION\n    path: app\n  - name: tests\n    type: CONFIGURATION\n    path: tests\n");
        let map = discover_project_source_map(&ambiguous).unwrap();
        assert_eq!(map.source_selection_error.as_deref(), Some("invalid_source_root: sourceDir is required because configuration source sets are ambiguous: app, tests"));

        let missing = temp_workspace("unica-source-map-missing-prefix");
        let map = discover_project_source_map(&missing).unwrap();
        assert_eq!(map.source_selection_error.as_deref(), Some("invalid_source_root: sourceDir is required because no configuration source set was found"));
        fs::remove_dir_all(ambiguous).unwrap();
        fs::remove_dir_all(missing).unwrap();
    }

    fn assert_source_set(
        map: &ProjectSourceMap,
        name: &str,
        kind: SourceSetKind,
        source_format: SourceFormat,
        expected_evidence: &[&str],
    ) {
        let source_set = map
            .source_sets
            .iter()
            .find(|source_set| source_set.name == name)
            .unwrap_or_else(|| panic!("source set {name} not found in {map:?}"));
        assert_eq!(source_set.kind, kind);
        assert_eq!(source_set.source_format, source_format);
        assert_eq!(
            source_set.format_evidence,
            expected_evidence
                .iter()
                .map(|evidence| (*evidence).to_string())
                .collect::<Vec<_>>()
        );
    }

    fn temp_workspace(prefix: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let nonce = TEMP_WORKSPACE_NONCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "{prefix}-{}-{timestamp}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: impl AsRef<OsStr>) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }
}
