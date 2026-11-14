//! #1381: запись в содержимое заимствованного объекта, которого расширение не
//! несёт, отказывает одним предметным отказом с продолжением к родителю.

use super::plan_hidden_v13_apply;
use crate::domain::apply::ApplyRequest;
use unica_source_contract::{SourceFormat, SourceProfile, SourceSetKind};
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;

use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;
use unica_source_contract::proposal::ApplyPlanErrorKind;
use unica_source_contract::workspace::WorkspaceContext;
use unica_workspace::actor::{ProviderRootBinding, WorkspaceIdentity, WorkspaceSourceSetInput};
use unica_workspace::test_support::tree_snapshot;

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn unborrow(path: &Path) {
    let text = std::fs::read_to_string(path).unwrap();
    let start = text.find("<ExtendedConfigurationObject>").unwrap();
    let end = text.find("</ExtendedConfigurationObject>").unwrap()
        + "</ExtendedConfigurationObject>".len();
    std::fs::write(path, format!("{}{}", &text[..start], &text[end..])).unwrap();
}

/// Plans one operation against the platform export of `workspace-borrowed`,
/// after `prepare` has changed the copied extension tree.
fn plan(prepare: &dyn Fn(&Path), at: &str, op: Value) -> (Result<(), super::ApplyPlanError>, bool) {
    let (planned, untouched) = plan_in(
        prepare,
        SourceProfile::platform_xml_8_3_27_format_2_20(),
        at,
        op,
    );
    (planned.map(|_| ()), untouched)
}

/// [`plan`] with the extension set in `ext_profile`, answering the plan.
fn plan_in(
    prepare: &dyn Fn(&Path),
    ext_profile: SourceProfile,
    at: &str,
    op: Value,
) -> (
    Result<
        (
            unica_workspace::apply::ApplyStagedState,
            unica_source_contract::proposal::PlannedApplyEffects,
        ),
        super::ApplyPlanError,
    >,
    bool,
) {
    let root = tempfile::tempdir().unwrap();
    copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-borrowed"),
        root.path(),
    );
    let workspace_root = std::fs::canonicalize(root.path()).unwrap();
    let main = workspace_root.join("src/cf");
    let ext = workspace_root.join("src/ext");
    prepare(&ext);
    let context = WorkspaceContext {
        cwd: workspace_root.clone(),
        workspace_root: workspace_root.clone(),
        cache_root: workspace_root.join(".build/unica"),
        workspace_epoch: 1,
    };
    let identity = WorkspaceIdentity::new(
        &context,
        [
            WorkspaceSourceSetInput::new(
                "main",
                &main,
                SourceSetKind::Configuration,
                SourceFormat::PlatformXml,
                SourceProfile::platform_xml_8_3_27_format_2_20(),
            ),
            WorkspaceSourceSetInput::new(
                "ext",
                &ext,
                SourceSetKind::Extension,
                SourceFormat::PlatformXml,
                ext_profile,
            ),
        ],
        "inherited-content-refusal",
    )
    .unwrap();
    let actor = crate::infrastructure::source_workspace::new_actor(identity, context).unwrap();
    let binding: ProviderRootBinding = actor.bind_provider_root("ext", &ext).unwrap();
    let parent = actor.bind_provider_root("main", &main).unwrap();
    let admission = actor
        .admit_apply_with_dependencies(
            &binding,
            &[parent],
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
    let args = json!({"at": at, "ops": [op], "dryRun": true});
    let request = ApplyRequest::parse(args.as_object().unwrap(), &["ext", "main"]).unwrap();
    let before = tree_snapshot(&ext);
    let planned = plan_hidden_v13_apply(&request, &binding, &admission);
    (planned, tree_snapshot(&ext) == before)
}

/// A form borrowed into an extension of another format than its parent's
/// takes the extension's format: the new files join the extension set.
/// rule: INV.SOURCE.WRITABLE-PROFILE
#[test]
fn a_borrowed_form_takes_the_format_of_its_extension_set() {
    use unica_source_contract::format_identity::PlatformLine;
    let ext_in_2_21 = |ext: &Path| {
        // The form is not borrowed yet; every root of the extension is 2.21.
        std::fs::remove_file(ext.join("DataProcessors/Обработка/Forms/Форма.xml")).unwrap();
        let owner = ext.join("DataProcessors/Обработка.xml");
        let text = std::fs::read_to_string(&owner).unwrap();
        std::fs::write(&owner, text.replace("<Form>Форма</Form>", "")).unwrap();
        fn convert(dir: &Path) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    convert(&path);
                } else if path.extension().is_some_and(|ext| ext == "xml") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    std::fs::write(&path, text.replace("version=\"2.20\"", "version=\"2.21\""))
                        .unwrap();
                }
            }
        }
        convert(ext);
    };
    let (planned, _) = plan_in(
        &ext_in_2_21,
        SourceProfile::platform_xml_of_line(PlatformLine::new(8, 5, 1)).unwrap(),
        "ext:Configuration",
        json!({"op":"object.borrow","args":{"from":"main:DataProcessor.Обработка.Form.Форма"}}),
    );
    let (staged, _) = planned.expect("the form is borrowed");
    let texts = staged
        .planned_changes()
        .into_iter()
        .filter_map(|change| match change.current {
            unica_source_contract::proposal::StagedFileState::Bytes(bytes) => {
                Some((change.relative_path, String::from_utf8(bytes).unwrap()))
            }
            unica_source_contract::proposal::StagedFileState::Absent => None,
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let palette = "xmlns:pal=\"http://v8.1c.ru/8.1/data/ui/colors/palette\"";
    for relative in [
        "DataProcessors/Обработка/Forms/Форма.xml",
        "DataProcessors/Обработка/Forms/Форма/Ext/Form.xml",
    ] {
        let text = &texts[Path::new(relative)];
        assert!(text.contains("version=\"2.21\""), "{relative}");
        assert!(!text.contains("version=\"2.20\""), "{relative}");
        assert!(text.contains(palette), "{relative}");
    }
}

const FORM: &str = "ext:DataProcessor.Обработка.Form.Форма";

fn content_operations() -> Vec<(&'static str, Value, &'static str)> {
    vec![
        (
            "ext:Role.SalesReader",
            json!({"op":"right.add","args":{"items":[{"object":"DataProcessor.Обработка","rights":{"Use":true}}]}}),
            "ext:Role.SalesReader",
        ),
        (
            FORM,
            json!({"op":"item.add","args":{"items":[{"name":"Н1"}]}}),
            "ext:DataProcessor.Обработка",
        ),
        (
            "ext:CommonForm.ОбщаяФорма.Module.Form",
            json!({"op":"code.insert","args":{"text":"\nПроцедура ПР_А()\nКонецПроцедуры\n"}}),
            "ext:CommonForm.ОбщаяФорма",
        ),
        (
            "ext:DataProcessor.Обработка.Form.Форма.Module.Form",
            json!({"op":"code.replace","args":{"text":"x"}}),
            "ext:DataProcessor.Обработка",
        ),
        (
            "ext:DataProcessor.Обработка.Template.Макет",
            json!({"op":"area.add","args":{"items":[{"name":"Шапка","cells":{"R1C1":"x"}}]}}),
            "ext:DataProcessor.Обработка",
        ),
        (
            "ext:DataProcessor.Обработка.Template.СхемаКомпоновки",
            json!({"op":"parameter.add","args":{"items":[{"name":"П1"}]}}),
            "ext:DataProcessor.Обработка",
        ),
        (
            "ext:XDTOPackage.ПакетОбмена",
            json!({"op":"objectType.add","args":{"values":{"name":"Т1"}}}),
            "ext:XDTOPackage.ПакетОбмена",
        ),
    ]
}

/// rule: INV.SOURCE.BORROWED-OBJECT-INHERITED-CONTENT
#[test]
fn writes_into_inherited_content_refuse_with_the_way_to_the_parent() {
    for (at, op, owner) in content_operations() {
        let name = op["op"].as_str().unwrap().to_string();
        let (planned, untouched) = plan(&|_| {}, at, op);
        let error = planned.expect_err(&format!("{name} at {at} must be refused"));
        assert_eq!(error.kind(), ApplyPlanErrorKind::NotFound, "{name}");
        assert!(
            error.to_string().contains("inherited from the parent"),
            "{name}: the refusal names inherited content"
        );
        let next = error.next().expect("the refusal continues to the parent");
        assert_eq!(next["tool"], "unica.view", "{name}");
        assert_eq!(next["args"]["at"], owner, "{name}");
        assert!(untouched, "{name}: a refused plan writes nothing");
    }
    // Описание заимствованного объекта по-прежнему пишется.
    let (planned, _) = plan(
        &|_| {},
        "ext:Role.SalesReader",
        json!({"op":"props.set","args":{"values":{"Comment":"x"}}}),
    );
    assert!(
        planned.is_ok(),
        "props.set writes the descriptor, not the rights"
    );
}

/// Свой файл у заимствованного объекта или собственный владелец формы
/// снимают отказ о наследовании: отвечает обычный планировщик.
/// rule: INV.SOURCE.BORROWED-OBJECT-INHERITED-CONTENT
#[test]
fn inherited_refusal_needs_a_borrowed_object_without_its_file() {
    let own_owner = |ext: &Path| unborrow(&ext.join("DataProcessors/Обработка.xml"));
    let own_role = |ext: &Path| unborrow(&ext.join("Roles/SalesReader.xml"));
    for (prepare, at, op) in [
        (
            &own_owner as &dyn Fn(&Path),
            FORM,
            json!({"op":"item.add","args":{"items":[{"name":"Н1"}]}}),
        ),
        (
            &own_role,
            "ext:Role.SalesReader",
            json!({"op":"right.add","args":{"items":[{"object":"DataProcessor.Обработка","rights":{"Use":true}}]}}),
        ),
    ] {
        let (planned, _) = plan(prepare, at, op);
        if let Err(error) = planned {
            assert!(
                !error.to_string().contains("inherited from the parent"),
                "{at}: an object that is not borrowed with its own owner inherits nothing"
            );
        }
    }
    let with_rights = |ext: &Path| {
        let rights = ext.join("Roles/SalesReader/Ext/Rights.xml");
        std::fs::create_dir_all(rights.parent().unwrap()).unwrap();
        std::fs::copy(ext.join("../cf/Roles/SalesReader/Ext/Rights.xml"), rights).unwrap();
    };
    let (planned, _) = plan(
        &with_rights,
        "ext:Role.SalesReader",
        json!({"op":"right.add","args":{"items":[{"object":"DataProcessor.Обработка","rights":{"Use":true}}]}}),
    );
    assert!(
        planned.is_ok(),
        "a borrowed role with its own rights is written"
    );
}

/// Plans one operation on a copy of an acceptance workspace with one source set.
fn plan_single(
    workspace: &str,
    set: &str,
    kind: SourceSetKind,
    at: &str,
    op: Value,
) -> Result<(), super::ApplyPlanError> {
    let root = tempfile::tempdir().unwrap();
    copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance")
            .join(workspace),
        root.path(),
    );
    let workspace_root = std::fs::canonicalize(root.path()).unwrap();
    let source = workspace_root.join("src");
    let context = WorkspaceContext {
        cwd: workspace_root.clone(),
        workspace_root: workspace_root.clone(),
        cache_root: workspace_root.join(".build/unica"),
        workspace_epoch: 1,
    };
    let identity = WorkspaceIdentity::new(
        &context,
        [WorkspaceSourceSetInput::new(
            set,
            &source,
            kind,
            SourceFormat::PlatformXml,
            SourceProfile::platform_xml_8_3_27_format_2_20(),
        )],
        "predefined-code-type",
    )
    .unwrap();
    let actor = crate::infrastructure::source_workspace::new_actor(identity, context).unwrap();
    let binding: ProviderRootBinding = actor.bind_provider_root(set, &source).unwrap();
    let admission = actor
        .admit_apply(
            &binding,
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
    let args = json!({"at": at, "ops": [op], "dryRun": true});
    let request = ApplyRequest::parse(args.as_object().unwrap(), &[set]).unwrap();
    plan_hidden_v13_apply(&request, &binding, &admission).map(|_| ())
}

/// Заимствованный справочник без `CodeType` не даёт
/// вида кодов; запись предопределённых отказывает тем же текстом, что и
/// чтение, а не проверяет коды по придуманному `String`.
/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
#[test]
fn predefined_writes_refuse_an_owner_without_code_type() {
    let item = json!({"name": "Основной", "id": "a1b2c3d4-1111-4111-8111-111111111111"});
    for op in [
        json!({"op": "predefinedItem.add", "args": {"items": [item.clone()]}}),
        json!({"op": "predefinedItem.set", "args": {"at": "ext:Catalog.CorpusCatalog.PredefinedItem.Основной", "values": {"description": "x"}}}),
    ] {
        let error = plan_single(
            "workspace-incomplete-borrowed",
            "ext",
            SourceSetKind::Extension,
            "ext:Catalog.CorpusCatalog",
            op,
        )
        .expect_err("an unknown CodeType must not be defaulted");
        assert_eq!(
            error.to_string(),
            unica_metadata_designer::predefined::MISSING_PREDEFINED_CODE_TYPE
        );
    }
    if let Err(error) = plan_single(
        "workspace",
        "main",
        SourceSetKind::Configuration,
        "main:Catalog.Валюты",
        json!({"op": "predefinedItem.add", "args": {"items": [item]}}),
    ) {
        assert_ne!(
            error.to_string(),
            unica_metadata_designer::predefined::MISSING_PREDEFINED_CODE_TYPE,
            "an owner with CodeType is not refused for its absence"
        );
    }
}

/// The properties of the first enumeration value of `text`, its uuids aside.
fn enum_value_properties(text: &str) -> Vec<String> {
    let start = text.find("<EnumValue ").unwrap();
    let end = text[start..].find("</EnumValue>").unwrap() + start;
    text[start..end]
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("<ExtendedConfigurationObject>"))
        .map(str::to_owned)
        .collect()
}

/// The files an `object.borrow` of `from` writes into the `empty_extension`
/// corpus of 2.21 over the `all_kinds` corpus of the same format.
fn borrow_in_2_21(from: &str) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    borrow_in(
        CORPUS_2_21,
        unica_source_contract::format_identity::PlatformLine::new(8, 5, 1),
        from,
    )
}

/// The corpora of formats 2.21 and 2.22.
const CORPUS_2_21: &str = "../../tests/fixtures/formats/platform_xml/2.21";
const CORPUS_2_22: &str = "../../tests/fixtures/formats/platform_xml/2.22";

/// The files an `object.borrow` of `from` writes into the `empty_extension`
/// corpus of the format at `corpus` over its `all_kinds` corpus.
fn borrow_in(
    corpus: &str,
    line: unica_source_contract::format_identity::PlatformLine,
    from: &str,
) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join(corpus);
    let root = tempfile::tempdir().unwrap();
    let workspace_root = std::fs::canonicalize(root.path()).unwrap();
    let main = workspace_root.join("cf");
    let ext = workspace_root.join("ext");
    copy_tree(&corpus.join("all_kinds/src"), &main);
    copy_tree(&corpus.join("empty_extension/ext"), &ext);
    std::fs::write(
        workspace_root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: cf\n  - name: ext\n    type: EXTENSION\n    path: ext\n",
    )
    .unwrap();
    let profile = SourceProfile::platform_xml_of_line(line).unwrap();
    let context = WorkspaceContext {
        cwd: workspace_root.clone(),
        workspace_root: workspace_root.clone(),
        cache_root: workspace_root.join(".build/unica"),
        workspace_epoch: 1,
    };
    let identity = WorkspaceIdentity::new(
        &context,
        [
            WorkspaceSourceSetInput::new(
                "main",
                &main,
                SourceSetKind::Configuration,
                SourceFormat::PlatformXml,
                profile,
            ),
            WorkspaceSourceSetInput::new(
                "ext",
                &ext,
                SourceSetKind::Extension,
                SourceFormat::PlatformXml,
                profile,
            ),
        ],
        "borrowed-enum-value-2-21",
    )
    .unwrap();
    let actor = crate::infrastructure::source_workspace::new_actor(identity, context).unwrap();
    let binding = actor.bind_provider_root("ext", &ext).unwrap();
    let parent = actor.bind_provider_root("main", &main).unwrap();
    let admission = actor
        .admit_apply_with_dependencies(
            &binding,
            &[parent],
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
    let args = json!({"at":"ext:Configuration","dryRun":true,"ops":[{"op":"object.borrow","args":{
        "from":from}}]});
    let request = ApplyRequest::parse(args.as_object().unwrap(), &["ext", "main"]).unwrap();
    let (staged, _) = plan_hidden_v13_apply(&request, &binding, &admission).unwrap();
    staged
        .planned_changes()
        .into_iter()
        .filter_map(|change| match change.current {
            unica_source_contract::proposal::StagedFileState::Bytes(bytes) => {
                Some((change.relative_path, bytes))
            }
            unica_source_contract::proposal::StagedFileState::Absent => None,
        })
        .collect()
}

/// A borrowed enumeration value of an extension 2.21 is written as 8.5.1
/// exports it: without the color a new value of that format has.
/// rule: INV.SOURCE.WRITABLE-PROFILE
#[test]
fn a_borrowed_enumeration_value_of_format_2_21_has_the_platform_shape() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/formats/platform_xml/2.21");
    let files =
        borrow_in_2_21("main:Enum.СпособыУстановкиКурсаВалюты.EnumValue.ЗагрузкаИзИнтернета");
    let written =
        String::from_utf8(files[Path::new("Enums/СпособыУстановкиКурсаВалюты.xml")].clone())
            .unwrap();
    let platform =
        std::fs::read_to_string(corpus.join("cfe_borrow/extension-enum-value.xml")).unwrap();
    assert_eq!(
        enum_value_properties(&written),
        enum_value_properties(&platform)
    );
    assert!(!written.contains("<Color>"));
}

/// A form borrowed into an extension 2.21 is the descriptor and Form.xml 8.5.1
/// exports for it: the descriptor has no interface compatibility mode, which
/// a new form of that format has.
/// rule: INV.SOURCE.WRITABLE-PROFILE
#[test]
fn a_borrowed_form_of_format_2_21_is_what_the_platform_exports() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/formats/platform_xml/2.21/cfe_borrow");
    let files = borrow_in_2_21("main:Catalog.Валюты.Form.ФормаЭлемента");
    for (relative, platform) in [
        (
            "Catalogs/Валюты/Forms/ФормаЭлемента.xml",
            "extension-form-borrowed.xml",
        ),
        (
            "Catalogs/Валюты/Forms/ФормаЭлемента/Ext/Form.xml",
            "extension-form-borrowed-Form.xml",
        ),
    ] {
        // The form's own uuid is fresh for each borrow.
        let fresh = |bytes: &[u8]| {
            let text = String::from_utf8(bytes.to_vec()).unwrap();
            match text.find("<Form uuid=\"") {
                Some(start) => {
                    let value = start + "<Form uuid=\"".len();
                    format!("{}{}", &text[..value], &text[value + 36..])
                }
                None => text,
            }
        };
        assert_eq!(
            fresh(&files[Path::new(relative)]),
            fresh(&std::fs::read(corpus.join(platform)).unwrap()),
            "{relative}"
        );
    }
}

/// What a borrowed object or member of 2.22 does when its configuration
/// object is missing, as the platform writes it after the parent link.
const ACTION_ON_MISSING: &str =
    "<ActionOnConfigurationObjectMissing>Auto</ActionOnConfigurationObjectMissing>";

/// The local names of the properties of the first descriptor object of
/// `bytes`.
fn property_names(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let document = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).unwrap();
    let properties = document
        .descendants()
        .find(|node| node.has_tag_name("Properties"))
        .unwrap();
    properties
        .children()
        .filter(|node| node.is_element())
        .map(|node| node.tag_name().name().to_string())
        .collect()
}

/// A borrowed enumeration value of an extension 2.22 names what it does when
/// its configuration value is missing, as 8.5.4 exports it.
/// rule: INV.SOURCE.WRITABLE-PROFILE
#[test]
fn a_borrowed_enumeration_value_of_format_2_22_has_the_platform_shape() {
    use unica_source_contract::format_identity::PlatformLine;
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/formats/platform_xml/2.22");
    let files = borrow_in(
        CORPUS_2_22,
        PlatformLine::new(8, 5, 4),
        "main:Enum.СпособыУстановкиКурсаВалюты.EnumValue.ЗагрузкаИзИнтернета",
    );
    let written =
        String::from_utf8(files[Path::new("Enums/СпособыУстановкиКурсаВалюты.xml")].clone())
            .unwrap();
    let platform =
        std::fs::read_to_string(corpus.join("cfe_borrow/extension-enum-value.xml")).unwrap();
    assert_eq!(
        enum_value_properties(&written),
        enum_value_properties(&platform)
    );
    assert!(enum_value_properties(&written).contains(&ACTION_ON_MISSING.to_string()));
}

/// A form borrowed into an extension 2.22 is the descriptor and Form.xml 8.5.4
/// exports for it, with the actions on a missing object and on an extension
/// change error.
/// rule: INV.SOURCE.WRITABLE-PROFILE
#[test]
fn a_borrowed_form_of_format_2_22_is_what_the_platform_exports() {
    use unica_source_contract::format_identity::PlatformLine;
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/formats/platform_xml/2.22/cfe_borrow");
    let files = borrow_in(
        CORPUS_2_22,
        PlatformLine::new(8, 5, 4),
        "main:Catalog.Валюты.Form.ФормаЭлемента",
    );
    for (relative, platform) in [
        (
            "Catalogs/Валюты/Forms/ФормаЭлемента.xml",
            "extension-form-borrowed.xml",
        ),
        (
            "Catalogs/Валюты/Forms/ФормаЭлемента/Ext/Form.xml",
            "extension-form-borrowed-Form.xml",
        ),
    ] {
        // The form's own uuid is fresh for each borrow.
        let fresh = |bytes: &[u8]| {
            let text = String::from_utf8(bytes.to_vec()).unwrap();
            match text.find("<Form uuid=\"") {
                Some(start) => {
                    let value = start + "<Form uuid=\"".len();
                    format!("{}{}", &text[..value], &text[value + 36..])
                }
                None => text,
            }
        };
        assert_eq!(
            fresh(&files[Path::new(relative)]),
            fresh(&std::fs::read(corpus.join(platform)).unwrap()),
            "{relative}"
        );
    }
}

/// A borrowed object of an extension 2.22 names what it does when its
/// configuration object is missing right after its parent link, as 8.5.4
/// exports it.
/// rule: INV.SOURCE.WRITABLE-PROFILE
#[test]
fn a_borrowed_object_of_format_2_22_names_its_action_on_a_missing_object() {
    use unica_source_contract::format_identity::PlatformLine;
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/formats/platform_xml/2.22/cfe_borrow");
    let catalog = borrow_in(
        CORPUS_2_22,
        PlatformLine::new(8, 5, 4),
        "main:Catalog.Валюты",
    );
    assert_eq!(
        property_names(&catalog[Path::new("Catalogs/Валюты.xml")]),
        property_names(&std::fs::read(corpus.join("extension-catalog-members.xml")).unwrap())
    );
    let document = borrow_in(
        CORPUS_2_22,
        PlatformLine::new(8, 5, 4),
        "main:Document.Приход",
    );
    let written = property_names(&document[Path::new("Documents/Приход.xml")]);
    let platform = property_names(&std::fs::read(corpus.join("extension-document.xml")).unwrap());
    assert_eq!(written, platform);
    assert_eq!(written[4], "ActionOnConfigurationObjectMissing");
    // 8.5.1 writes no such property.
    let earlier = borrow_in(
        CORPUS_2_21,
        PlatformLine::new(8, 5, 1),
        "main:Catalog.Валюты",
    );
    assert!(
        !String::from_utf8(earlier[Path::new("Catalogs/Валюты.xml")].clone())
            .unwrap()
            .contains("ActionOnConfigurationObjectMissing")
    );
}

/// A borrowed document names its register records, empty, as 8.3.27 exports
/// it (#2996).
/// rule: INV.SOURCE.WRITABLE-PROFILE
#[test]
fn a_borrowed_document_of_format_2_20_has_empty_register_records() {
    use unica_source_contract::format_identity::PlatformLine;
    let document = borrow_in(
        "../../tests/fixtures/formats/platform_xml/2.20",
        PlatformLine::new(8, 3, 27),
        "main:Document.Приход",
    );
    let written = &document[Path::new("Documents/Приход.xml")];
    let platform = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../tests/fixtures/formats/platform_xml/2.20/incomplete_borrowed/extension-document.xml",
    ))
    .unwrap();
    assert_eq!(property_names(written), property_names(&platform));
    assert!(String::from_utf8(written.clone())
        .unwrap()
        .contains("<RegisterRecords/>"));
}
