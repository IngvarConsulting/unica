use super::*;
use crate::application::lsp::position::Utf16Position;
use std::fs;
use std::path::Path;
use std::time::Instant;

const MODULE: &str = "CommonModules/ОбщегоНазначения/Ext/Module.bsl";

struct Space {
    _temp: tempfile::TempDir,
    root: PathBuf,
}

fn space(text: &str) -> Space {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let module = root.join("src").join(MODULE);
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    fs::write(module, text).unwrap();
    Space { _temp: temp, root }
}

fn layout(name: &str, root: &Path) -> SpaceLayout {
    SpaceLayout {
        folder: WorkspaceFolder {
            name: name.to_owned(),
            root: root.to_owned(),
        },
        source_sets: vec![SourceSetLayout {
            name: "main".to_owned(),
            kind: SourceSetKind::Configuration,
            root: root.join("src"),
            profile: PlatformProfile::v8_3_27(),
        }],
    }
}

fn reader(root: &Path) -> ProviderReadAuthority {
    let root = Arc::new(unica_workspace::read::open_source_directory(&root.join("src")).unwrap());
    ProviderReadAuthority::new("main", "actor-main", SourceSetKind::Configuration, root)
}

fn file_uri(path: &Path) -> String {
    url::Url::from_file_path(path).unwrap().into()
}

#[test]
fn files_and_unica_uris_map_both_ways_and_two_mains_do_not_mix() {
    let (ut, bp) = (space("// УТ\n"), space("// БП\n"));
    let documents = LspDocuments::new(
        "editor",
        vec![layout("УТ", &ut.root), layout("БП", &bp.root)],
    );

    let ut_uri = documents
        .uri_of_file(&file_uri(&ut.root.join("src").join(MODULE)))
        .unwrap();
    let bp_uri = documents
        .uri_of_file(&file_uri(&bp.root.join("src").join(MODULE)))
        .unwrap();
    assert_ne!(ut_uri, bp_uri);
    assert_eq!(
        documents.locate(&ut_uri).unwrap().document,
        DocumentUri {
            space: "ут".to_owned(),
            address: "main:CommonModule.ОбщегоНазначения".to_owned(),
        }
    );
    assert_eq!(
        documents.file_of_uri(&bp_uri).unwrap(),
        file_uri(&bp.root.join("src").join(MODULE))
    );
    assert_eq!(
        documents.content(&ut_uri, &reader(&ut.root)).unwrap(),
        Some("// УТ\n".to_owned())
    );
    assert_eq!(
        documents.content(&bp_uri, &reader(&bp.root)).unwrap(),
        Some("// БП\n".to_owned())
    );

    // A read of the other space's `main` does not answer for this one.
    assert!(matches!(
        documents.content(&ut_uri, &reader(&bp.root)),
        Err(LspDocumentError::UnknownSourceSet(_))
    ));

    // A file outside the module layout, outside every source set, or of
    // another scheme has no document.
    let stray = ut
        .root
        .join("src/CommonModules/ОбщегоНазначения/Ext/Other.bsl");
    assert!(matches!(
        documents.uri_of_file(&file_uri(&stray)),
        Err(LspDocumentError::NotAModule)
    ));
    assert!(matches!(
        documents.uri_of_file(&file_uri(&ut.root.join("README.md"))),
        Err(LspDocumentError::OutsideSourceSets)
    ));
    assert!(matches!(
        documents.uri_of_file("https://example.test/Module.bsl"),
        Err(LspDocumentError::NotAFile)
    ));
    // A space of another connection is refused.
    let other = LspDocuments::new("other", vec![layout("УТ", &ut.root)]);
    assert!(matches!(
        other.locate(&bp_uri),
        Err(LspDocumentError::Uri(UriError::ForeignSpace(space))) if space == "бп"
    ));
}

#[test]
fn an_open_buffer_is_what_its_connection_reads_and_the_disk_is_untouched() {
    let ut = space("Процедура А()\nКонецПроцедуры\n");
    let overlay = DocumentOverlay::default();
    let mut editor = LspDocuments::new("editor", vec![layout("УТ", &ut.root)]);
    let mut agent = LspDocuments::new("agent", vec![layout("УТ", &ut.root)]);
    let uri = editor
        .uri_of_file(&file_uri(&ut.root.join("src").join(MODULE)))
        .unwrap();

    editor
        .open(
            &overlay,
            &uri,
            1,
            "Процедура А()\nКонецПроцедуры\n".to_owned(),
        )
        .unwrap();
    editor
        .change(
            &overlay,
            &uri,
            2,
            &[ContentChange::Range {
                start: Utf16Position {
                    line: 0,
                    character: 10,
                },
                end: Utf16Position {
                    line: 0,
                    character: 11,
                },
                text: "Б😀".to_owned(),
            }],
        )
        .unwrap();
    let edited = "Процедура Б😀()\nКонецПроцедуры\n";

    let editor_read =
        reader(&ut.root).with_documents(overlay.session_documents("editor", "main").unwrap());
    assert_eq!(
        editor.content(&uri, &editor_read).unwrap().as_deref(),
        Some(edited)
    );
    // The analysis read of the editor's session sees the buffer as well.
    let target = module_target(
        &QualifiedAddress::parse("main:CommonModule.ОбщегоНазначения").unwrap(),
        &layout("УТ", &ut.root).source_sets[0],
    )
    .unwrap();
    assert_eq!(
        editor_read.module_source(&target).unwrap().as_deref(),
        Some(edited)
    );
    let agent_read =
        reader(&ut.root).with_documents(overlay.session_documents("agent", "main").unwrap());
    assert_eq!(
        agent.content(&uri, &agent_read).unwrap().as_deref(),
        Some("Процедура А()\nКонецПроцедуры\n")
    );
    assert_eq!(
        fs::read_to_string(ut.root.join("src").join(MODULE)).unwrap(),
        "Процедура А()\nКонецПроцедуры\n"
    );

    // A stale version and a change to an unopened module are refused and
    // leave the buffer as it was.
    assert!(matches!(
        editor.change(&overlay, &uri, 2, &[ContentChange::Full(String::new())]),
        Err(LspDocumentError::Change(
            DocumentChangeError::StaleVersion { .. }
        ))
    ));
    assert!(matches!(
        agent.change(&overlay, &uri, 1, &[]),
        Err(LspDocumentError::NotOpen)
    ));
    assert_eq!(
        overlay
            .get("editor", "main", Path::new(MODULE))
            .unwrap()
            .unwrap()
            .version,
        2
    );

    // `apply` that touched the module, another file, an epoch change.
    let touched = [PathBuf::from(MODULE)];
    assert_eq!(
        editor.changed_documents("ут", "main", Some(&touched)),
        vec![uri.clone()]
    );
    assert!(editor
        .changed_documents("ут", "main", Some(&[PathBuf::from("Other.bsl")]))
        .is_empty());
    assert_eq!(
        editor.changed_documents("ут", "main", None),
        vec![uri.clone()]
    );

    editor.close(&overlay, &uri).unwrap();
    assert!(overlay.is_empty());
    assert_eq!(
        editor.content(&uri, &reader(&ut.root)).unwrap().as_deref(),
        Some("Процедура А()\nКонецПроцедуры\n")
    );
}

/// The whole of a large module in one `workspace/textDocumentContent`.
/// Point `UNICA_LSP_UT_SOURCE` at a Designer dump of a large configuration
/// (its `src`), run with `--ignored --nocapture`.
#[test]
#[ignore = "needs a large local Designer dump in UNICA_LSP_UT_SOURCE"]
fn a_large_common_module_comes_whole_in_one_request() {
    let Some(source) = std::env::var_os("UNICA_LSP_UT_SOURCE").map(PathBuf::from) else {
        return;
    };
    let source = source.canonicalize().unwrap();
    let workspace = source.parent().unwrap().to_owned();
    let documents = LspDocuments::new(
        "editor",
        vec![SpaceLayout {
            folder: WorkspaceFolder {
                name: "УТ".to_owned(),
                root: workspace,
            },
            source_sets: vec![SourceSetLayout {
                name: "main".to_owned(),
                kind: SourceSetKind::Configuration,
                root: source.clone(),
                profile: PlatformProfile::v8_3_27(),
            }],
        }],
    );
    let uri = documents
        .uri_of_file(&file_uri(&source.join(MODULE)))
        .unwrap();
    let read = ProviderReadAuthority::new(
        "main",
        "actor-main",
        SourceSetKind::Configuration,
        Arc::new(unica_workspace::read::open_source_directory(&source).unwrap()),
    );
    let started = Instant::now();
    let text = documents.content(&uri, &read).unwrap().unwrap();
    let elapsed = started.elapsed();
    let disk = fs::read_to_string(source.join(MODULE)).unwrap();
    assert_eq!(text, disk.trim_start_matches('\u{feff}'));
    println!(
        "textDocumentContent {uri}: {} lines, {} bytes in {:.1} ms",
        text.lines().count(),
        text.len(),
        elapsed.as_secs_f64() * 1000.0
    );
}

#[test]
fn unica_uris_of_common_object_and_form_modules_read_the_disk_without_open() {
    let ws = space("// общий\n");
    let src = ws.root.join("src");
    let object = src.join("Catalogs/Валюты/Ext/ObjectModule.bsl");
    let form = src.join("Catalogs/Валюты/Forms/ФормаЭлемента/Ext/Form/Module.bsl");
    let manager = src.join("Catalogs/Валюты/Ext/ManagerModule.bsl");
    for (path, text) in [
        (&object, "// объект\n"),
        (&form, "// форма\n"),
        (&manager, "// менеджер\n"),
    ] {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let documents = LspDocuments::new("editor", vec![layout("Sendbox", &ws.root)]);
    let read = reader(&ws.root);
    for (path, expected) in [
        (
            src.join(MODULE),
            "unica://sendbox/main/CommonModule.ОбщегоНазначения",
        ),
        (
            object.clone(),
            "unica://sendbox/main/Catalog.Валюты.Module.Object",
        ),
        (
            manager.clone(),
            "unica://sendbox/main/Catalog.Валюты.Module.Manager",
        ),
        (
            form.clone(),
            "unica://sendbox/main/Catalog.Валюты.Form.ФормаЭлемента.Module.Form",
        ),
    ] {
        let uri = documents.uri_of_file(&file_uri(&path)).unwrap();
        assert_eq!(
            percent_encoding::percent_decode_str(&uri)
                .decode_utf8()
                .unwrap(),
            expected
        );
        // The address an editor types, unencoded, names the same document.
        let disk = fs::read_to_string(&path).unwrap();
        for asked in [uri.as_str(), expected] {
            assert_eq!(
                documents.content(asked, &read).unwrap().as_deref(),
                Some(disk.as_str()),
                "{asked}"
            );
        }
        assert_eq!(documents.file_of_uri(&uri).unwrap(), file_uri(&path));
    }
}
