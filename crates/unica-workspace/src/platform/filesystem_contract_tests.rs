use std::path::{Path, PathBuf};
#[cfg(unix)]
pub fn saved_source_diff_keeps_literal_backslash_and_nested_paths_distinct(
    renderer: crate::preview::SavedSourceDiffFactory,
) {
    use super::filesystem::RetainedDirectoryCapability;
    use crate::apply::ApplyStagedState;
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;
    use unica_work_control::cancellation::CancellationToken;
    use unica_work_control::provider_deadline::ProviderDeadline;

    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("A")).unwrap();
    let literal = Path::new(r"A\B.bin");
    let nested = Path::new("A/B.bin");
    fs::write(root.path().join(literal), b"literal-before").unwrap();
    fs::write(root.path().join(nested), b"nested-before").unwrap();
    let canonical = fs::canonicalize(root.path()).unwrap();
    let mut staged = ApplyStagedState::from_retained_root(
        Arc::new(RetainedDirectoryCapability::open(&canonical).unwrap()),
        ProviderDeadline::no_deadline(),
        CancellationToken::new(),
        crate::actor::apply_writer_authority_for_test(),
    )
    .forbid_generated_subtree();
    let enumerated = staged.enumerate_tree(Path::new("")).unwrap();
    assert_eq!(enumerated.len(), 2);
    assert!(enumerated.contains(&literal.to_path_buf()));
    assert!(enumerated.contains(&nested.to_path_buf()));
    staged
        .replace(literal, b"literal-before", b"literal-after".to_vec())
        .unwrap();
    staged
        .replace(nested, b"nested-before", b"nested-after".to_vec())
        .unwrap();
    let transaction = staged.finalize().unwrap();
    let images = transaction.saved_source_images();
    assert_eq!(images.len(), 2);
    let distinct_paths = images
        .iter()
        .map(|image| image.path.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        distinct_paths.len(),
        2,
        "different retained files must keep different public diff paths"
    );
    let expected = BTreeMap::from([
        ((r"A\B.bin", "before"), "literal-before"),
        ((r"A\B.bin", "after"), "literal-after"),
        (("A/B.bin", "before"), "nested-before"),
        (("A/B.bin", "after"), "nested-after"),
    ]);
    let layout = renderer(&images, &|| Ok(())).unwrap();
    let mut cursor = None;
    let mut observed = BTreeMap::new();
    loop {
        let page = layout
            .page(
                &images,
                "distinct-retained-source-paths",
                1,
                cursor.as_deref(),
                &|| Ok(()),
            )
            .unwrap();
        assert_eq!(
            page,
            layout
                .page(
                    &images,
                    "distinct-retained-source-paths",
                    1,
                    cursor.as_deref(),
                    &|| Ok(())
                )
                .unwrap()
        );
        let chunks = page["chunks"].as_array().unwrap();
        assert_eq!(chunks.len(), 1);
        let chunk = &chunks[0];
        assert_eq!(chunk["encoding"], "utf8");
        assert_eq!(chunk["startByte"], 0);
        let key = (
            chunk["path"].as_str().unwrap().to_owned(),
            chunk["side"].as_str().unwrap().to_owned(),
        );
        let content = chunk["content"].as_str().unwrap().to_owned();
        assert!(observed.insert(key, content).is_none());
        cursor = page["cursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            assert_eq!(page["stoppedBy"], "complete");
            break;
        }
        assert_eq!(page["stoppedBy"], "limit");
    }
    let expected = expected
        .into_iter()
        .map(|((path, side), content)| ((path.to_owned(), side.to_owned()), content.to_owned()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(observed, expected);
    assert_eq!(
        fs::read(root.path().join(literal)).unwrap(),
        b"literal-before"
    );
    assert_eq!(
        fs::read(root.path().join(nested)).unwrap(),
        b"nested-before"
    );
}

#[cfg(not(unix))]
pub fn saved_source_diff_keeps_literal_backslash_and_nested_paths_distinct(
    _renderer: crate::preview::SavedSourceDiffFactory,
) {
    panic!("this existing physical fixture requires its original supported host");
}
#[cfg(target_os = "linux")]
pub fn saved_source_diff_keeps_native_names_and_replacement_character_distinct(
    renderer: crate::preview::SavedSourceDiffFactory,
    classifier: &dyn unica_source_contract::read::SourceArtifactClassifier,
) {
    use super::filesystem::RetainedDirectoryCapability;
    use unica_source_contract::{SourceFormat, SourceProfile, SourceSetKind};

    use crate::apply::ApplyStagedState;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use std::collections::{BTreeMap, BTreeSet};
    use std::ffi::OsString;
    use std::fs;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    use std::path::Path;
    use std::sync::Arc;
    use unica_source_contract::read::RevisionArtifactDisposition;
    use unica_work_control::cancellation::CancellationToken;
    use unica_work_control::provider_deadline::ProviderDeadline;

    let root = tempfile::tempdir().unwrap();
    let prefix = Path::new("Documents/First/Templates/T/Ext/Template");
    fs::create_dir_all(root.path().join(prefix)).unwrap();
    let names = [
        OsString::from_vec(b"A-\x80.bin".to_vec()),
        OsString::from_vec(b"A-\x81.bin".to_vec()),
        OsString::from("A-\u{fffd}.bin"),
    ];
    let files = names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let relative = prefix.join(name);
            let bytes = vec![0, 0x80 + u8::try_from(index).unwrap(), 0xff];
            fs::write(root.path().join(&relative), &bytes).unwrap();
            (relative, bytes)
        })
        .collect::<Vec<_>>();
    let policy = classifier
        .bind(
            SourceSetKind::Configuration,
            SourceFormat::PlatformXml,
            SourceProfile::platform_xml_8_3_27_format_2_20(),
        )
        .unwrap();
    let canonical = fs::canonicalize(root.path()).unwrap();
    let retained = Arc::new(RetainedDirectoryCapability::open(&canonical).unwrap());
    assert!(!retained
        .child_name_comparator()
        .unwrap()
        .names_equivalent(&names[0], &names[1])
        .unwrap());
    let mut staged = ApplyStagedState::from_retained_root(
        retained,
        ProviderDeadline::no_deadline(),
        CancellationToken::new(),
        crate::actor::apply_writer_authority_for_test(),
    )
    .forbid_generated_subtree();
    let enumerated = staged.enumerate_tree(prefix).unwrap();
    assert_eq!(enumerated.len(), files.len());
    for (relative, bytes) in &files {
        assert!(enumerated.contains(relative));
        assert_eq!(
            policy.classify(relative),
            RevisionArtifactDisposition::Content
        );
        staged.remove(relative, bytes).unwrap();
    }
    let transaction = staged.finalize().unwrap();
    let images = transaction.saved_source_images();
    assert_eq!(images.len(), files.len());
    assert_eq!(
        images
            .iter()
            .map(|image| image.path.as_str())
            .collect::<BTreeSet<_>>()
            .len(),
        files.len(),
        "the two native names and literal replacement character must remain distinct"
    );
    let layout = renderer(&images, &|| Ok(())).unwrap();
    let mut cursor = None;
    let mut observed = BTreeMap::new();
    let mut public_paths = BTreeSet::new();
    loop {
        let page = layout
            .page(
                &images,
                "native-retained-paths",
                1,
                cursor.as_deref(),
                &|| Ok(()),
            )
            .unwrap();
        assert_eq!(
            page,
            layout
                .page(
                    &images,
                    "native-retained-paths",
                    1,
                    cursor.as_deref(),
                    &|| Ok(())
                )
                .unwrap()
        );
        let chunks = page["chunks"].as_array().unwrap();
        assert_eq!(chunks.len(), 1);
        let chunk = &chunks[0];
        let encoding = chunk["pathEncoding"].as_str().unwrap();
        let path = chunk["path"].as_str().unwrap();
        public_paths.insert((encoding.to_owned(), path.to_owned()));
        let native = match encoding {
            "utf8" => {
                assert!(path.ends_with("A-\u{fffd}.bin"));
                path.as_bytes().to_vec()
            }
            "base64-unix-bytes" => STANDARD.decode(path).unwrap(),
            _ => panic!("unexpected Unix path encoding"),
        };
        let content = chunk["content"].as_str().unwrap();
        let bytes = match chunk["encoding"].as_str().unwrap() {
            "utf8" => content.as_bytes().to_vec(),
            "base64" => STANDARD.decode(content).unwrap(),
            _ => panic!("unexpected content encoding"),
        };
        let side = chunk["side"].as_str().unwrap().to_owned();
        assert_eq!(chunk["present"], side == "before");
        assert!(observed.insert((native, side), bytes).is_none());
        cursor = page["cursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            assert_eq!(page["stoppedBy"], "complete");
            break;
        }
        assert_eq!(page["stoppedBy"], "limit");
    }
    assert_eq!(public_paths.len(), 3);
    assert_eq!(
        public_paths
            .iter()
            .filter(|(encoding, _)| encoding == "utf8")
            .count(),
        1
    );
    assert_eq!(
        public_paths
            .iter()
            .filter(|(encoding, _)| encoding == "base64-unix-bytes")
            .count(),
        2
    );
    let mut expected = BTreeMap::new();
    for (relative, bytes) in &files {
        let native = relative.as_os_str().as_bytes().to_vec();
        expected.insert((native.clone(), "before".to_owned()), bytes.clone());
        expected.insert((native, "after".to_owned()), Vec::new());
        assert_eq!(fs::read(root.path().join(relative)).unwrap(), *bytes);
    }
    assert_eq!(observed, expected);
}

#[cfg(not(target_os = "linux"))]
pub fn saved_source_diff_keeps_native_names_and_replacement_character_distinct(
    _renderer: crate::preview::SavedSourceDiffFactory,
    _classifier: &dyn unica_source_contract::read::SourceArtifactClassifier,
) {
    panic!("this existing physical fixture requires its original supported host");
}
#[cfg(windows)]
pub fn source_set_relative_path_accepts_a_verbatim_root_without_widening_containment(
    observe: fn(
        &unica_source_contract::workspace::WorkspaceContext,
        &Path,
        &Path,
    ) -> Option<PathBuf>,
) {
    use crate::paths::normalize_path_identity;
    use std::fs;
    use unica_source_contract::workspace::WorkspaceContext;

    let workspace = unique_temp_root("source-set-verbatim-root");
    let module = workspace.join("src/CommonModules/Shared/Ext/Module.bsl");
    let outside = workspace.join("src-neighbor/Module.bsl");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    fs::create_dir_all(outside.parent().unwrap()).unwrap();
    fs::write(&module, b"Procedure Probe()\nEndProcedure").unwrap();
    fs::write(&outside, b"Procedure Other()\nEndProcedure").unwrap();
    let regular_workspace = normalize_path_identity(&workspace).unwrap();
    let module = normalize_path_identity(&module).unwrap();
    let outside = normalize_path_identity(&outside).unwrap();
    let regular_text = regular_workspace.to_string_lossy();
    let verbatim_workspace = if let Some(unc) = regular_text.strip_prefix(r"\\") {
        PathBuf::from(format!(r"\\?\UNC\{unc}"))
    } else {
        PathBuf::from(format!(r"\\?\{regular_text}"))
    };
    let verbatim_source = verbatim_workspace.join("src");
    let context = WorkspaceContext {
        cwd: verbatim_workspace.clone(),
        workspace_root: verbatim_workspace.clone(),
        cache_root: verbatim_workspace.join(".build/unica"),
        workspace_epoch: 1,
    };

    assert!(module.strip_prefix(&verbatim_source).is_err());
    assert_eq!(
        observe(&context, &verbatim_source, &module),
        Some(PathBuf::from("CommonModules/Shared/Ext/Module.bsl"))
    );
    assert_eq!(
        observe(&context, &verbatim_source, &outside),
        None,
        "a sibling with the same lexical prefix must stay outside the source set"
    );

    fs::remove_dir_all(workspace).unwrap();
}

#[cfg(not(windows))]
pub fn source_set_relative_path_accepts_a_verbatim_root_without_widening_containment(
    _observe: fn(
        &unica_source_contract::workspace::WorkspaceContext,
        &Path,
        &Path,
    ) -> Option<PathBuf>,
) {
    panic!("this existing physical fixture requires its original supported host");
}
#[cfg(unix)]
pub fn source_root_policy_rejects_parent_traversal_after_directory_symlink(
    observe: fn(&Path, PathBuf) -> Result<PathBuf, String>,
) {
    use std::os::unix::fs::symlink;
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let workspace = std::env::temp_dir().join(format!(
        "unica-source-roots-parent-workspace-{}-{nanos}",
        std::process::id()
    ));
    let outside = std::env::temp_dir().join(format!(
        "unica-source-roots-parent-outside-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    symlink(&outside, workspace.join("external")).unwrap();

    let error = observe(&workspace, workspace.join("external/../escaped-new")).unwrap_err();

    assert!(error.contains("sourceDir must be inside workspace root"));
    let _ = std::fs::remove_dir_all(workspace);
    let _ = std::fs::remove_dir_all(outside);
}

#[cfg(not(unix))]
pub fn source_root_policy_rejects_parent_traversal_after_directory_symlink(
    _observe: fn(&Path, PathBuf) -> Result<PathBuf, String>,
) {
    panic!("this existing physical fixture requires its original supported host");
}

#[cfg(windows)]
fn unique_temp_root(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "unica-filesystem-{name}-{}-{nanos}",
        std::process::id()
    ))
}
