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

/// The `unica://` URI of the module body MCP addresses as `address`.
fn body_uri(documents: &LspDocuments, space: &str, address: &str) -> String {
    documents
        .spaces()
        .format(&DocumentUri {
            space: space.to_owned(),
            address: address.to_owned(),
        })
        .unwrap()
}

const BODY: &str = "main:CommonModule.ОбщегоНазначения.Body";

#[test]
fn module_bodies_are_documents_and_two_mains_do_not_mix() {
    let (ut, bp) = (space("// УТ\n"), space("// БП\n"));
    let documents = LspDocuments::new(
        "editor",
        vec![layout("УТ", &ut.root), layout("БП", &bp.root)],
    );

    let ut_uri = body_uri(&documents, "ут", BODY);
    let bp_uri = body_uri(&documents, "бп", BODY);
    assert_ne!(ut_uri, bp_uri);
    let located = documents.locate(&ut_uri).unwrap();
    assert_eq!(located.relative, Path::new(MODULE));
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

    // Only a module body is a document: not the module node, not a
    // method body, not a file path.
    for address in [
        "main:CommonModule.ОбщегоНазначения",
        "main:CommonModule.ОбщегоНазначения.Module",
        "main:CommonModule.ОбщегоНазначения.Method.Ф.Body",
    ] {
        assert!(
            matches!(
                documents.locate(&body_uri(&documents, "ут", address)),
                Err(LspDocumentError::NotAModule)
            ),
            "{address}"
        );
    }
    // A line selector is not part of a document address.
    assert!(documents.locate(&format!("{ut_uri}%23L1")).is_err());
    // Russian kind names are the same address.
    assert_eq!(
        documents
            .locate("unica://ут/main/ОбщийМодуль.ОбщегоНазначения.Тело")
            .unwrap()
            .relative,
        Path::new(MODULE)
    );
    let file = file_uri(&ut.root.join("src").join(MODULE));
    assert!(matches!(
        documents.locate(&file),
        Err(LspDocumentError::Uri(UriError::Malformed(_)))
    ));
    assert!(documents.content(&file, &reader(&ut.root)).is_err());
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
    let uri = body_uri(&editor, "ут", BODY);

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
    let uri = body_uri(&documents, "ут", BODY);
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
fn the_body_address_mcp_gives_a_module_is_its_document_without_open() {
    let ws = space("// общий\n");
    let src = ws.root.join("src");
    let documents = LspDocuments::new("editor", vec![layout("Sendbox", &ws.root)]);
    let read = reader(&ws.root);
    for (relative, body, text) in [
        (MODULE, BODY, None),
        (
            "Catalogs/Валюты/Ext/ObjectModule.bsl",
            "main:Catalog.Валюты.Module.Object.Body",
            Some("// объект\n"),
        ),
        (
            "Catalogs/Валюты/Ext/ManagerModule.bsl",
            "main:Catalog.Валюты.Module.Manager.Body",
            Some("// менеджер\n"),
        ),
        (
            "Catalogs/Валюты/Forms/ФормаЭлемента/Ext/Form/Module.bsl",
            "main:Catalog.Валюты.Form.ФормаЭлемента.Module.Form.Body",
            Some("// форма\n"),
        ),
        (
            "CommonForms/Выбор/Ext/Form/Module.bsl",
            "main:CommonForm.Выбор.Module.Form.Body",
            Some("// общая форма\n"),
        ),
        (
            "Ext/ManagedApplicationModule.bsl",
            "main:Module.ManagedApplication.Body",
            Some("// приложение\n"),
        ),
    ] {
        let path = src.join(relative);
        if let Some(text) = text {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, text).unwrap();
        }
        // The module address `unica.resolve` gives the file, followed by
        // `Body`, is the address MCP reads the text by.
        let module = crate::infrastructure::event_sources::module_address_for_relative(
            "main",
            SourceSetKind::Configuration,
            PlatformProfile::v8_3_27(),
            Path::new(relative),
        )
        .unwrap();
        assert_eq!(format!("{module}.Body"), body);

        let uri = body_uri(&documents, "sendbox", body);
        let expected = format!("unica://sendbox/{}", body.replacen(':', "/", 1));
        assert_eq!(
            percent_encoding::percent_decode_str(&uri)
                .decode_utf8()
                .unwrap(),
            expected
        );
        // Encoded or typed as is, the URI reads the file without didOpen.
        let disk = fs::read_to_string(&path).unwrap();
        for asked in [uri.as_str(), expected.as_str()] {
            assert_eq!(
                documents.content(asked, &read).unwrap().as_deref(),
                Some(disk.as_str()),
                "{asked}"
            );
        }
    }
}
