use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, bytes).unwrap();
}

fn configured_workspace(label: &str) -> tempfile::TempDir {
    let root = tempfile::Builder::new().prefix(label).tempdir().unwrap();
    write(
        &root.path().join("v8project.yaml"),
        concat!(
            "format: DESIGNER\n",
            "source-set:\n",
            "  - name: main\n",
            "    type: CONFIGURATION\n",
            "    path: src\n",
        ),
    );
    write(
        &root.path().join("src/Configuration.xml"),
        b"<MetaDataObject><Configuration/></MetaDataObject>",
    );
    root
}

pub fn aggregate_selection_directory_handles_cross_former_128_and_keep_finality(
    discovery: Arc<dyn unica_source_contract::read::SourceMapDiscovery>,
) {
    let discover_project_source_admission =
        |root: &Path, checkpoint: &mut dyn FnMut() -> Result<(), String>| {
            super::discover_project_source_admission(root, Arc::clone(&discovery), checkpoint)
        };

    let root = tempfile::tempdir().unwrap();
    let mut config = String::from("format: DESIGNER\nsource-set:\n");
    for index in 0..129 {
        let (name, kind) = if index == 0 {
            ("main".to_string(), "CONFIGURATION")
        } else {
            (format!("extension-{index}"), "EXTENSION")
        };
        config.push_str(&format!(
            "  - name: {name}\n    type: {kind}\n    path: source-{index}\n"
        ));
        write(
            &root
                .path()
                .join(format!("source-{index}/Configuration.xml")),
            b"<MetaDataObject><Configuration/></MetaDataObject>",
        );
    }
    write(&root.path().join("v8project.yaml"), &config);
    let admission = discover_project_source_admission(root.path(), &mut || Ok(()))
        .expect("129 declared source roots must be admitted");
    assert_eq!(admission.map.source_sets.len(), 129);
    assert_eq!(admission.map.effective_source_set.as_deref(), Some("main"));
    assert!(admission.evidence.directories.len() > 128);
    let deadline = ProviderDeadline::from_budget(std::time::Duration::from_secs(30));
    let cancellation = CancellationToken::new();
    admission
        .evidence
        .validate(deadline, &cancellation)
        .unwrap();
    write(
        &root.path().join("v8project.yaml"),
        format!("{config}\n# changed\n"),
    );
    assert_eq!(
        admission
            .evidence
            .validate(deadline, &cancellation)
            .unwrap_err()
            .kind(),
        SourceSelectionEvidenceErrorKind::Changed
    );
}

pub fn aggregate_selection_exact_bytes_cross_former_32mib_without_source_walk(
    discovery: Arc<dyn unica_source_contract::read::SourceMapDiscovery>,
) {
    let discover_project_source_admission =
        |root: &Path, checkpoint: &mut dyn FnMut() -> Result<(), String>| {
            super::discover_project_source_admission(root, Arc::clone(&discovery), checkpoint)
        };

    let root = tempfile::tempdir().unwrap();
    let mut config = String::from(
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: main\n",
    );
    write(
        &root.path().join("main/Configuration.xml"),
        b"<MetaDataObject><Configuration/></MetaDataObject>",
    );
    let mut descriptor = b"<MetaDataObject><ExternalDataProcessor/><!--".to_vec();
    descriptor.resize(7 * 1024 * 1024 - b"--></MetaDataObject>".len(), b' ');
    descriptor.extend_from_slice(b"--></MetaDataObject>");
    for index in 0..5 {
        config.push_str(&format!(
                "  - name: external-{index}\n    type: EXTERNAL_DATA_PROCESSORS\n    path: external-{index}\n"
            ));
        write(
            &root
                .path()
                .join(format!("external-{index}/ConfigDumpInfo.xml")),
            &descriptor,
        );
    }
    write(&root.path().join("v8project.yaml"), config);
    let admission = discover_project_source_admission(root.path(), &mut || Ok(()))
        .expect("exact selector inputs may exceed 32 MiB in aggregate");
    assert_eq!(admission.map.source_sets.len(), 6);
    let exact_bytes = admission
        .evidence
        .paths
        .values()
        .filter_map(|input| match input {
            RetainedPathObservation::RegularFile {
                bytes: Some(bytes), ..
            } => Some(bytes.len()),
            _ => None,
        })
        .sum::<usize>();
    assert!(exact_bytes > 32 * 1024 * 1024);
    let deadline = ProviderDeadline::from_budget(std::time::Duration::from_secs(30));
    let cancellation = CancellationToken::new();
    admission
        .evidence
        .validate(deadline, &cancellation)
        .unwrap();
    let changed = root.path().join("external-4/ConfigDumpInfo.xml");
    write(
        &changed,
        b"<MetaDataObject><ExternalDataProcessor/><!-- changed --></MetaDataObject>",
    );
    assert_eq!(
        admission
            .evidence
            .validate(deadline, &cancellation)
            .unwrap_err()
            .kind(),
        SourceSelectionEvidenceErrorKind::Changed
    );
}

pub fn aggregate_selection_membership_crosses_former_16384_across_sources(
    discovery: Arc<dyn unica_source_contract::read::SourceMapDiscovery>,
) {
    let discover_project_source_admission =
        |root: &Path, checkpoint: &mut dyn FnMut() -> Result<(), String>| {
            super::discover_project_source_admission(root, Arc::clone(&discovery), checkpoint)
        };

    let root = tempfile::tempdir().unwrap();
    let mut config = String::from("format: DESIGNER\nsource-set:\n");
    for index in 0..2 {
        config.push_str(&format!(
                "  - name: external-{index}\n    type: EXTERNAL_DATA_PROCESSORS\n    path: external-{index}\n"
            ));
        for member in 0..9000 {
            write(
                &root
                    .path()
                    .join(format!("external-{index}/member-{member:05}.txt")),
                b"",
            );
        }
        write(
            &root
                .path()
                .join(format!("external-{index}/zzProcessor.xml")),
            b"<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
        );
    }
    write(&root.path().join("v8project.yaml"), config);
    let admission = discover_project_source_admission(root.path(), &mut || Ok(()))
        .expect("enumeration work across source sets may exceed 16384 members");
    assert_eq!(admission.map.source_sets.len(), 2);
    for index in 0..2 {
        assert_eq!(
            admission.evidence.memberships[Path::new(&format!("external-{index}"))].len(),
            9001
        );
        assert!(admission.map.source_sets[index]
            .format_evidence
            .iter()
            .any(|path| path.ends_with("zzProcessor.xml")));
    }
    let deadline = ProviderDeadline::from_budget(std::time::Duration::from_secs(30));
    let cancellation = CancellationToken::new();
    admission
        .evidence
        .validate(deadline, &cancellation)
        .unwrap();
    write(&root.path().join("external-1/late-non-xml.txt"), b"");
    assert_eq!(
        admission
            .evidence
            .validate(deadline, &cancellation)
            .unwrap_err()
            .kind(),
        SourceSelectionEvidenceErrorKind::Changed
    );
}

pub fn published_replacement_of_a_retained_source_map_file_passes_the_final_gate(
    discovery: Arc<dyn unica_source_contract::read::SourceMapDiscovery>,
) {
    let discover_project_source_admission =
        |root: &Path, checkpoint: &mut dyn FnMut() -> Result<(), String>| {
            super::discover_project_source_admission(root, Arc::clone(&discovery), checkpoint)
        };

    let root = configured_workspace("unica-source-selection-published-replacement");
    let _budgets = install_actor_selection_test_budgets(ActorSelectionTestBudgets::generous());
    let mut checkpoint = || Ok(());
    let evidence = discover_project_source_admission(root.path(), &mut checkpoint)
        .expect("admission over the configured workspace")
        .evidence;
    let deadline = ProviderDeadline::from_budget(std::time::Duration::from_secs(30));
    let cancellation = CancellationToken::new();
    evidence
        .validate(deadline, &cancellation)
        .expect("untouched workspace validates");

    // The apply publishes a new Configuration.xml the way the transaction
    // does: a fresh file renamed over the old name, so the identity moves.
    let post_image: Vec<u8> =
        b"<MetaDataObject><Configuration><ChildObjects/></Configuration></MetaDataObject>".to_vec();
    let staged = root.path().join("src/Configuration.xml.staged");
    std::fs::write(&staged, &post_image).unwrap();
    std::fs::rename(&staged, root.path().join("src/Configuration.xml")).unwrap();

    let strict = evidence
        .validate(deadline, &cancellation)
        .expect_err("a moved identity is a change without a publication plan");
    assert!(strict.to_string().contains("identity changed"), "{strict}");

    let mut published = BTreeMap::new();
    published.insert(PathBuf::from("src/Configuration.xml"), post_image.clone());
    evidence
        .validate_final_with_published(&published, deadline, &cancellation)
        .expect("the planned replacement passes the final gate");

    let mut mismatched = BTreeMap::new();
    mismatched.insert(
        PathBuf::from("src/Configuration.xml"),
        b"<MetaDataObject><Configuration/></MetaDataObject>".to_vec(),
    );
    evidence
        .validate_final_with_published(&mismatched, deadline, &cancellation)
        .expect_err("bytes that differ from the plan stay a change");
}

pub fn actor_admission_rejects_aggregate_exact_byte_budget(
    discovery: Arc<dyn unica_source_contract::read::SourceMapDiscovery>,
) {
    let discover_project_source_admission =
        |root: &Path, checkpoint: &mut dyn FnMut() -> Result<(), String>| {
            super::discover_project_source_admission(root, Arc::clone(&discovery), checkpoint)
        };

    let defaults = SelectionEvidenceBudgets::actor_admission();
    assert_eq!(
        [
            defaults.exact_bytes,
            defaults.evidence_records,
            defaults.enumerated_members,
            defaults.unique_directories,
            defaults.route_and_name_bytes,
        ],
        [None; 5]
    );
    let root = configured_workspace("unica-source-selection-byte-budget");
    let _budgets = install_actor_selection_test_budgets(ActorSelectionTestBudgets {
        exact_bytes: Some(32),
        ..ActorSelectionTestBudgets::generous()
    });
    let mut checkpoint = || Ok(());

    let error = discover_project_source_admission(root.path(), &mut checkpoint)
        .expect_err("actor admission ignored its aggregate exact-byte budget");

    assert_eq!(
        error,
        "project source-map actor exact-byte budget exceeds 32 bytes"
    );
}

pub fn actor_admission_charges_repeated_exact_work_before_second_read(
    discovery: Arc<dyn unica_source_contract::read::SourceMapDiscovery>,
) {
    let discover_project_source_admission =
        |root: &Path, checkpoint: &mut dyn FnMut() -> Result<(), String>| {
            super::discover_project_source_admission(root, Arc::clone(&discovery), checkpoint)
        };

    let root = tempfile::tempdir().unwrap();
    let config = concat!(
        "format: EDT\n",
        "source-set:\n",
        "  - name: first\n    type: EXTERNAL_DATA_PROCESSORS\n    path: shared\n",
        "  - name: second\n    type: EXTERNAL_DATA_PROCESSORS\n    path: shared\n",
    );
    write(&root.path().join("v8project.yaml"), config);
    write(&root.path().join("shared/ConfigDumpInfo.xml"), b"12345678");
    let exact_work_limit = config.len() + 8;
    let _budgets = install_actor_selection_test_budgets(ActorSelectionTestBudgets {
        exact_work_bytes: Some(exact_work_limit),
        ..ActorSelectionTestBudgets::generous()
    });
    reset_regular_exact_read_metrics();
    let mut checkpoint = || Ok(());

    let error = discover_project_source_admission(root.path(), &mut checkpoint)
        .expect_err("second source-set exact observation escaped the pass-global work budget");

    assert_eq!(
        error,
        format!("project source-map actor exact-work budget exceeds {exact_work_limit} bytes")
    );
    let (attempts, bytes, _) = regular_exact_read_metrics();
    assert_eq!(
        attempts, 2,
        "second exact observation started a content read"
    );
    assert_eq!(
        bytes, exact_work_limit,
        "second exact observation read content before work-budget rejection"
    );
}

pub fn actor_admission_bounds_unique_retained_directories_without_ulimit(
    discovery: Arc<dyn unica_source_contract::read::SourceMapDiscovery>,
) {
    let discover_project_source_admission =
        |root: &Path, checkpoint: &mut dyn FnMut() -> Result<(), String>| {
            super::discover_project_source_admission(root, Arc::clone(&discovery), checkpoint)
        };

    let root = tempfile::tempdir().unwrap();
    write(
        &root.path().join("v8project.yaml"),
        concat!(
            "format: DESIGNER\n",
            "source-set:\n",
            "  - name: first\n    type: CONFIGURATION\n    path: first\n",
            "  - name: second\n    type: EXTENSION\n    path: second\n",
        ),
    );
    std::fs::create_dir(root.path().join("first")).unwrap();
    std::fs::create_dir(root.path().join("second")).unwrap();
    let _budgets = install_actor_selection_test_budgets(ActorSelectionTestBudgets {
        unique_directories: Some(2),
        ..ActorSelectionTestBudgets::generous()
    });
    let mut checkpoint = || Ok(());

    let error = discover_project_source_admission(root.path(), &mut checkpoint)
        .expect_err("actor admission retained more unique directory handles than its budget");

    assert_eq!(
        error,
        "project source-map actor unique-directory budget exceeds 2 entries"
    );

    let workspace =
        RetainedDirectoryCapability::open(&std::fs::canonicalize(root.path()).unwrap()).unwrap();
    let mut pass = RetainedSelectionPass::new(workspace).unwrap();
    assert!(matches!(
        pass.observe_directory(Path::new("first"), &mut checkpoint)
            .unwrap(),
        RetainedDirectoryObservation::Present(_)
    ));
    let error = match pass.observe_directory(Path::new("second"), &mut checkpoint) {
        Ok(_) => panic!("a third unique retained directory escaped the budget"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        "project source-map actor unique-directory budget exceeds 2 entries"
    );
    assert_eq!(
        pass.observation_counts_for_test().0,
        2,
        "retained directory evidence exceeded its deterministic handle budget"
    );
}

pub fn actor_admission_accepts_external_processor_and_report_with_non_evidence_members(
    discovery: Arc<dyn unica_source_contract::read::SourceMapDiscovery>,
) {
    let discover_project_source_admission =
        |root: &Path, checkpoint: &mut dyn FnMut() -> Result<(), String>| {
            super::discover_project_source_admission(root, Arc::clone(&discovery), checkpoint)
        };

    let root = tempfile::tempdir().unwrap();
    write(
        &root.path().join("v8project.yaml"),
        concat!(
            "format: DESIGNER\nsource-set:\n",
            "  - name: processors\n    type: EXTERNAL_DATA_PROCESSORS\n    path: epf\n",
            "  - name: reports\n    type: EXTERNAL_REPORTS\n    path: erf\n",
        ),
    );
    for (directory, kind, name) in [
        ("epf", "ExternalDataProcessor", "Import"),
        ("erf", "ExternalReport", "Sales"),
    ] {
        write(
                &root.path().join(format!("{directory}/{name}.xml")),
                format!("<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><{kind}><Properties><Name>{name}</Name></Properties></{kind}></MetaDataObject>"),
            );
        write(
            &root
                .path()
                .join(format!("{directory}/{name}/Ext/ObjectModule.bsl")),
            "Procedure Execute() Export\nEndProcedure\n",
        );
    }
    // Each directory consumes two member slots but contributes one format marker.
    // The second enumeration must fit the remaining member budget, not reserve
    // the larger format-evidence ceiling all over again.
    let mut checkpoint = || Ok(());
    let admitted = discover_project_source_admission(root.path(), &mut checkpoint)
        .expect("two small external source sets must share one admission pass");
    let sets = &admitted.map().source_sets;
    assert_eq!(sets.len(), 2);
    assert_eq!(sets[0].kind, SourceSetKind::ExternalProcessor);
    assert_eq!(sets[1].kind, SourceSetKind::ExternalReport);
    assert_eq!(sets[0].source_format, SourceFormat::PlatformXml);
    assert_eq!(sets[1].source_format, SourceFormat::PlatformXml);
    assert_eq!(sets[0].format_evidence, ["epf/Import.xml"]);
    assert_eq!(sets[1].format_evidence, ["erf/Sales.xml"]);
}

pub fn actor_admission_bounds_global_membership_across_external_source_sets(
    discovery: Arc<dyn unica_source_contract::read::SourceMapDiscovery>,
) {
    let discover_project_source_admission =
        |root: &Path, checkpoint: &mut dyn FnMut() -> Result<(), String>| {
            super::discover_project_source_admission(root, Arc::clone(&discovery), checkpoint)
        };

    let root = tempfile::tempdir().unwrap();
    write(
        &root.path().join("v8project.yaml"),
        concat!(
            "format: DESIGNER\n",
            "source-set:\n",
            "  - name: processors\n",
            "    type: EXTERNAL_DATA_PROCESSORS\n",
            "    path: epf\n",
            "  - name: reports\n",
            "    type: EXTERNAL_REPORTS\n",
            "    path: erf\n",
        ),
    );
    for relative in [
        "epf/first.xml",
        "epf/second.xml",
        "erf/third.xml",
        "erf/fourth.xml",
    ] {
        std::fs::create_dir_all(root.path().join(relative)).unwrap();
    }
    let _budgets = install_actor_selection_test_budgets(ActorSelectionTestBudgets {
        enumerated_members: Some(3),
        ..ActorSelectionTestBudgets::generous()
    });
    let mut checkpoint = || Ok(());

    let error = discover_project_source_admission(root.path(), &mut checkpoint)
        .expect_err("per-source-set enumeration reset the actor's global membership budget");

    assert_eq!(
        error,
        "project source-map actor enumerated-member budget exceeds 3 entries"
    );
}

pub fn actor_admission_counts_repeated_membership_enumeration_globally(
    discovery: Arc<dyn unica_source_contract::read::SourceMapDiscovery>,
) {
    let discover_project_source_admission =
        |root: &Path, checkpoint: &mut dyn FnMut() -> Result<(), String>| {
            super::discover_project_source_admission(root, Arc::clone(&discovery), checkpoint)
        };

    let root = tempfile::tempdir().unwrap();
    write(
        &root.path().join("v8project.yaml"),
        concat!(
            "format: DESIGNER\n",
            "source-set:\n",
            "  - name: first\n    type: EXTERNAL_REPORTS\n    path: shared\n",
            "  - name: second\n    type: EXTERNAL_REPORTS\n    path: shared\n",
            "  - name: third\n    type: EXTERNAL_REPORTS\n    path: shared\n",
        ),
    );
    for relative in ["shared/first.xml", "shared/second.xml"] {
        std::fs::create_dir_all(root.path().join(relative)).unwrap();
    }
    let _budgets = install_actor_selection_test_budgets(ActorSelectionTestBudgets {
        enumerated_members: Some(4),
        ..ActorSelectionTestBudgets::generous()
    });
    let mut checkpoint = || Ok(());

    let error = discover_project_source_admission(root.path(), &mut checkpoint)
        .expect_err("repeated membership enumeration escaped the actor-global work budget");

    assert_eq!(
        error,
        "project source-map actor enumerated-member budget exceeds 4 entries"
    );
}

pub fn actor_admission_rejects_total_evidence_record_budget(
    discovery: Arc<dyn unica_source_contract::read::SourceMapDiscovery>,
) {
    let discover_project_source_admission =
        |root: &Path, checkpoint: &mut dyn FnMut() -> Result<(), String>| {
            super::discover_project_source_admission(root, Arc::clone(&discovery), checkpoint)
        };

    let root = configured_workspace("unica-source-selection-record-budget");
    let _budgets = install_actor_selection_test_budgets(ActorSelectionTestBudgets {
        evidence_records: Some(3),
        ..ActorSelectionTestBudgets::generous()
    });
    let mut checkpoint = || Ok(());

    let error = discover_project_source_admission(root.path(), &mut checkpoint)
        .expect_err("actor admission ignored its aggregate evidence-record budget");

    assert_eq!(
        error,
        "project source-map actor evidence-record budget exceeds 3 entries"
    );
}

pub fn actor_admission_rejects_route_and_name_byte_budget(
    discovery: Arc<dyn unica_source_contract::read::SourceMapDiscovery>,
) {
    let discover_project_source_admission =
        |root: &Path, checkpoint: &mut dyn FnMut() -> Result<(), String>| {
            super::discover_project_source_admission(root, Arc::clone(&discovery), checkpoint)
        };

    let root = tempfile::tempdir().unwrap();
    write(
        &root.path().join("v8project.yaml"),
        concat!(
            "format: DESIGNER\n",
            "source-set:\n",
            "  - name: main\n",
            "    type: CONFIGURATION\n",
            "    path: retained-route-is-long\n",
        ),
    );
    std::fs::create_dir(root.path().join("retained-route-is-long")).unwrap();
    let _budgets = install_actor_selection_test_budgets(ActorSelectionTestBudgets {
        route_and_name_bytes: Some(8),
        ..ActorSelectionTestBudgets::generous()
    });
    let mut checkpoint = || Ok(());

    let error = discover_project_source_admission(root.path(), &mut checkpoint)
        .expect_err("actor admission ignored retained route/name bytes");

    assert_eq!(
        error,
        "project source-map actor route/name budget exceeds 8 bytes"
    );
}

pub fn actor_admission_comparison_honors_cancellation(
    discovery: Arc<dyn unica_source_contract::read::SourceMapDiscovery>,
) {
    let discover_project_source_admission =
        |root: &Path, checkpoint: &mut dyn FnMut() -> Result<(), String>| {
            super::discover_project_source_admission(root, Arc::clone(&discovery), checkpoint)
        };

    let root = configured_workspace("unica-source-selection-compare-cancel");
    let cancellation = CancellationToken::new();
    let cancel_during_compare = cancellation.clone();
    let _hook = install_pass_comparison_test_hook(move || {
        cancel_during_compare.cancel();
    });
    let mut checkpoint = || {
        if cancellation.is_cancelled() {
            Err("project source-map comparison cancelled".to_string())
        } else {
            Ok(())
        }
    };

    let error = discover_project_source_admission(root.path(), &mut checkpoint)
        .expect_err("pass comparison did not consult cancellation authority");

    assert_eq!(error, "project source-map comparison cancelled");
}

pub fn actor_admission_comparison_honors_deadline(
    discovery: Arc<dyn unica_source_contract::read::SourceMapDiscovery>,
) {
    let discover_project_source_admission =
        |root: &Path, checkpoint: &mut dyn FnMut() -> Result<(), String>| {
            super::discover_project_source_admission(root, Arc::clone(&discovery), checkpoint)
        };

    let root = configured_workspace("unica-source-selection-compare-deadline");
    let expired = Arc::new(AtomicBool::new(false));
    let expire_during_compare = Arc::clone(&expired);
    let _hook = install_pass_comparison_test_hook(move || {
        expire_during_compare.store(true, Ordering::SeqCst);
    });
    let mut checkpoint = || {
        if expired.load(Ordering::SeqCst) {
            Err("project source-map comparison deadline exceeded".to_string())
        } else {
            Ok(())
        }
    };

    let error = discover_project_source_admission(root.path(), &mut checkpoint)
        .expect_err("pass comparison did not consult deadline authority");

    assert_eq!(error, "project source-map comparison deadline exceeded");
}

pub fn retained_selection_pass_rejects_inconsistent_regular_repeat(
    discovery: Arc<dyn unica_source_contract::read::SourceMapDiscovery>,
) {
    let discover_project_source_admission =
        |root: &Path, checkpoint: &mut dyn FnMut() -> Result<(), String>| {
            super::discover_project_source_admission(root, Arc::clone(&discovery), checkpoint)
        };

    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("marker.xml");
    write(&marker, b"first");
    let workspace =
        RetainedDirectoryCapability::open(&std::fs::canonicalize(root.path()).unwrap()).unwrap();
    let mut pass = RetainedSelectionPass::new(workspace).unwrap();
    let mut checkpoint = || Ok(());
    pass.observe_regular(Path::new("marker.xml"), 32, &mut checkpoint)
        .unwrap();
    write(&marker, b"second");

    let error = pass
        .observe_regular(Path::new("marker.xml"), 32, &mut checkpoint)
        .err()
        .expect("inconsistent repeated exact bytes were accepted");

    assert_eq!(
        error,
        "project source-map actor observation changed within one pass: marker.xml"
    );

    let root = configured_workspace("unica-source-selection-two-pass-change");
    let config = root.path().join("v8project.yaml");
    let mut first = std::fs::read(&config).unwrap();
    first.extend_from_slice(b"# first\n");
    write(&config, &first);
    let later = String::from_utf8(first.clone())
        .unwrap()
        .replace("# first", "# later")
        .into_bytes();
    assert_eq!(first.len(), later.len());
    let _read_hook = install_regular_exact_after_read_hook(move || write(&config, later));

    let error = discover_project_source_admission(root.path(), &mut checkpoint)
        .expect_err("two different observations of the same source map were accepted");

    assert_eq!(
        error,
        "project source-map changed during retained actor admission"
    );
}
