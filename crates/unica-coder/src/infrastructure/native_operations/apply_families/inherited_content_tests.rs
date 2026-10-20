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
                SourceProfile::platform_xml_8_3_27_format_2_20(),
            ),
        ],
        "inherited-content-refusal",
    )
    .unwrap();
    let actor = crate::infrastructure::source_workspace::new_actor(identity, context).unwrap();
    let binding: ProviderRootBinding = actor.bind_provider_root("ext", &ext).unwrap();
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
    let request = ApplyRequest::parse(args.as_object().unwrap(), &["ext"]).unwrap();
    let before = tree_snapshot(&ext);
    let planned = plan_hidden_v13_apply(&request, &binding, &admission).map(|_| ());
    (planned, tree_snapshot(&ext) == before)
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

/// #1381 (вторая вычитка): заимствованный справочник без `CodeType` не даёт
/// вида кодов; запись предопределённых отказывает тем же текстом, что и
/// чтение, а не проверяет коды по придуманному `String`.
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
