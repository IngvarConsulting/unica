use super::UnicaApplication;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use uuid::Uuid;

struct TempWorkspace(PathBuf);

impl TempWorkspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "unica-external-reader-surface-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn call(
    workspace: &Path,
    tool: &str,
    extra: impl IntoIterator<Item = (String, Value)>,
) -> super::OperationResult {
    let mut args = Map::from_iter([(
        "cwd".to_string(),
        Value::String(workspace.display().to_string()),
    )]);
    args.extend(extra);
    UnicaApplication::new()
        .call_tool(tool, &args)
        .unwrap_or_else(|error| panic!("{tool}: {error}"))
}

fn external_workspace(source_set_type: &str, init_tool: &str, name: &str) -> TempWorkspace {
    let workspace = TempWorkspace::new();
    let project = format!(
        "format: DESIGNER\nsource-set:\n  - name: external\n    type: {source_set_type}\n    path: external\n"
    );
    std::fs::write(workspace.path().join("v8project.yaml"), project).unwrap();
    let initialized = call(
        workspace.path(),
        init_tool,
        [
            ("Name".to_string(), json!(name)),
            ("OutputDir".to_string(), json!("external")),
            ("dryRun".to_string(), json!(false)),
        ],
    );
    assert!(initialized.ok, "{initialized:?}");
    let form = call(
        workspace.path(),
        "unica.form.add",
        [
            (
                "ObjectPath".to_string(),
                json!(format!("external/{name}.xml")),
            ),
            ("FormName".to_string(), json!("Main")),
            ("Purpose".to_string(), json!("Object")),
            ("SetDefault".to_string(), json!(true)),
            ("dryRun".to_string(), json!(false)),
        ],
    );
    assert!(form.ok, "{form:?}");

    let descriptor_path = workspace.path().join(format!("external/{name}.xml"));
    let descriptor = std::fs::read_to_string(&descriptor_path).unwrap();
    let descriptor = descriptor.replacen(
        "</ChildObjects>",
        "\t\t\t<Template>Print</Template>\n\t\t</ChildObjects>",
        1,
    );
    assert!(descriptor.contains("<Template>Print</Template>"));
    std::fs::write(&descriptor_path, descriptor).unwrap();
    let template = workspace
        .path()
        .join(format!("external/{name}/Templates/Print"));
    std::fs::create_dir_all(template.join("Ext")).unwrap();
    std::fs::write(
        workspace
            .path()
            .join(format!("external/{name}/Templates/Print.xml")),
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">",
            "<Template><Properties><Name>Print</Name><Synonym/><Comment/><TemplateType>SpreadsheetDocument</TemplateType></Properties></Template>",
            "</MetaDataObject>",
        ),
    )
    .unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/platform_8_3_27/mxl/Template.xml"),
        template.join("Ext/Template.xml"),
    )
    .unwrap();
    std::fs::create_dir_all(workspace.path().join(format!("external/{name}/Ext"))).unwrap();
    std::fs::write(
        workspace
            .path()
            .join(format!("external/{name}/Ext/ObjectModule.bsl")),
        "Procedure Run()\nEndProcedure\n",
    )
    .unwrap();
    workspace
}

#[test]
fn external_artifact_readers_share_one_logical_owner_profile() {
    for (source_set_type, init_tool, owner_kind, name) in [
        (
            "EXTERNAL_DATA_PROCESSORS",
            "unica.epf.init",
            "ExternalDataProcessor",
            "Review",
        ),
        (
            "EXTERNAL_REPORTS",
            "unica.erf.init",
            "ExternalReport",
            "Analysis",
        ),
    ] {
        assert_external_readers(source_set_type, init_tool, owner_kind, name);
    }
}

fn assert_external_readers(source_set_type: &str, init_tool: &str, owner_kind: &str, name: &str) {
    let workspace = external_workspace(source_set_type, init_tool, name);
    let owner = format!("{owner_kind}.{name}");

    let children = call(
        workspace.path(),
        "unica.source.children",
        [
            ("sourceSet".to_string(), json!("external")),
            ("metadataPath".to_string(), json!(owner)),
            ("limit".to_string(), json!(50)),
        ],
    );
    assert!(children.ok, "{children:?}");
    assert_eq!(children.data.as_ref().unwrap()["completeness"], "complete");

    for (tool, metadata_path) in [
        ("unica.meta.info", format!("{owner_kind}.{name}")),
        ("unica.form.info", format!("{owner_kind}.{name}.Form.Main")),
        (
            "unica.mxl.info",
            format!("{owner_kind}.{name}.Template.Print"),
        ),
    ] {
        let result = call(
            workspace.path(),
            tool,
            [
                ("sourceSet".to_string(), json!("external")),
                ("metadataPath".to_string(), json!(metadata_path)),
            ],
        );
        assert!(result.ok, "{tool}: {result:?}");
        if tool == "unica.meta.info" {
            assert_eq!(result.data.as_ref().unwrap()["kind"], owner_kind);
        }
    }

    for (tool, field, value) in [
        (
            "unica.form.info",
            "FormPath",
            format!("external/{name}/Forms/Main"),
        ),
        (
            "unica.mxl.info",
            "TemplatePath",
            format!("external/{name}/Templates/Print"),
        ),
    ] {
        let result = call(workspace.path(), tool, [(field.to_string(), json!(value))]);
        assert!(result.ok, "{tool}: {result:?}");
    }

    std::fs::write(
        workspace
            .path()
            .join(format!("external/{name}/Templates/Print.xml")),
        "<broken>",
    )
    .unwrap();
    let invalid = call(
        workspace.path(),
        "unica.meta.info",
        [
            ("sourceSet".to_string(), json!("external")),
            ("metadataPath".to_string(), json!(owner)),
        ],
    );
    assert!(
        !invalid.ok,
        "corrupt external child evidence must remain visible"
    );
}
