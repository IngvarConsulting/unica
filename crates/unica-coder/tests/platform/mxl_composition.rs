use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};
use unica_templates_designer::mxl_codec::test_support::platform_mxl_fixture;

use unica_templates_designer::mxl_root::{empty_spreadsheet_document_xml, MXL_DOCUMENT_NS};

use super::*;

#[test]
fn mxl_v12_wrapper_and_shared_core_return_the_same_typed_data() {
    let context = test_context("info-shared-core-parity");
    let template_path = mxl_info_template_path(&context);
    fs::copy(platform_mxl_fixture(), &template_path).unwrap();
    let args = path_args(&template_path);
    let wrapper = analyze_mxl_info(&args, &context, &WorkspaceSupportStateReader::new(&context))
        .data
        .unwrap();
    let core = parse_mxl_info_xml(
        &fs::read(&template_path).unwrap(),
        &wrapper.name,
        wrapper.support.clone(),
        false,
    )
    .unwrap();

    assert_eq!(
        serde_json::to_value(wrapper).unwrap(),
        serde_json::to_value(core).unwrap()
    );
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn mxl_validate_rejects_non_document_root() {
    let (context, template_path) = write_test_mxl(
        "validate-root",
        &format!("<SpreadsheetDocument xmlns=\"{MXL_DOCUMENT_NS}\"/>"),
    );

    let outcome = validate_mxl(&path_args(&template_path), &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(outcome.errors.iter().any(|error| {
        error.contains("SpreadsheetDocument") && error.contains("expected 'document'")
    }));
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn mxl_validate_rejects_wrong_document_namespace() {
    let (context, template_path) =
        write_test_mxl("validate-namespace", "<document xmlns=\"urn:not-mxl\"/>");

    let outcome = validate_mxl(&path_args(&template_path), &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(outcome
        .errors
        .iter()
        .any(|error| error.contains("urn:not-mxl") && error.contains(MXL_DOCUMENT_NS)));
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn mxl_validate_accepts_platform_8_3_27_empty_fixture() {
    let context = test_context("validate-platform-empty");
    let fixture = platform_mxl_fixture();

    let outcome = validate_mxl(&path_args(&fixture), &context);

    assert!(outcome.ok, "{outcome:?}");
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn mxl_validate_rejects_noncanonical_empty_sentinel_lookalike() {
    let xml = empty_spreadsheet_document_xml().replace(
        "<templateMode>true</templateMode>",
        "<templateMode>false</templateMode>",
    );
    let (context, template_path) = write_test_mxl("validate-empty-lookalike", &xml);

    let outcome = validate_mxl(&path_args(&template_path), &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(outcome
        .errors
        .iter()
        .any(|error| error.contains("height=0")));
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn mxl_info_reports_zero_rows_for_platform_8_3_27_empty_fixture() {
    let context = test_context("info-platform-empty");
    let template_path = mxl_info_template_path(&context);
    fs::copy(platform_mxl_fixture(), &template_path).unwrap();
    let args = path_args(&template_path);

    let execution = analyze_mxl_info(&args, &context, &WorkspaceSupportStateReader::new(&context));

    assert!(execution.outcome.ok, "{:?}", execution.outcome);
    // ADR-0023: the template description is data, not a JSON string in stdout.
    let report = serde_json::to_value(execution.data.unwrap()).unwrap();
    assert_eq!(report["rows"], 0);
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn mxl_info_preserves_explicit_height_for_first_empty_row() {
    let xml = empty_spreadsheet_document_xml().replace(
        "\t<vgRows>0</vgRows>",
        "\t<height>1</height>\n\t<vgRows>0</vgRows>",
    );
    let (context, template_path) = write_test_mxl("info-first-empty-row", &xml);
    let args = path_args(&template_path);

    let execution = analyze_mxl_info(&args, &context, &WorkspaceSupportStateReader::new(&context));

    assert!(execution.outcome.ok, "{:?}", execution.outcome);
    // ADR-0023: the template description is data, not a JSON string in stdout.
    let report = serde_json::to_value(execution.data.unwrap()).unwrap();
    assert_eq!(report["rows"], 1);
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn mxl_info_does_not_treat_height_zero_lookalike_as_platform_empty() {
    let xml = empty_spreadsheet_document_xml().replace(
        "\t<vgRows>0</vgRows>",
        "\t<height>0</height>\n\t<vgRows>0</vgRows>",
    );
    let (context, template_path) = write_test_mxl("info-height-zero-lookalike", &xml);
    let args = path_args(&template_path);

    let execution = analyze_mxl_info(&args, &context, &WorkspaceSupportStateReader::new(&context));

    assert!(execution.outcome.ok, "{:?}", execution.outcome);
    // ADR-0023: the template description is data, not a JSON string in stdout.
    let report = serde_json::to_value(execution.data.unwrap()).unwrap();
    assert_eq!(report["rows"], 1);
    let _ = fs::remove_dir_all(&context.cwd);
}

/// `TemplatePath` is the only address this reader publishes, so both
/// documented forms of it must work: the descriptor file and the layout
/// directory that resolves to `Ext/Template.xml`.
#[test]
fn mxl_info_resolves_a_layout_directory_to_its_template() {
    let context = test_context("info-layout-directory");
    let layout = context.cwd.join("src/Reports/Sales/Templates/Main");
    let template_path = layout.join("Ext").join("Template.xml");
    fs::create_dir_all(template_path.parent().unwrap()).unwrap();
    fs::write(&template_path, empty_spreadsheet_document_xml()).unwrap();

    let execution = analyze_mxl_info(
        &path_args(&layout),
        &context,
        &WorkspaceSupportStateReader::new(&context),
    );

    assert!(execution.outcome.ok, "{:?}", execution.outcome);
    assert_eq!(
        execution.outcome.artifacts,
        vec![
            unica_workspace::paths::normalize_path_identity(&template_path)
                .unwrap()
                .display()
                .to_string()
        ]
    );
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn mxl_info_rejects_wrong_document_namespace() {
    let (context, template_path) =
        write_test_mxl("info-namespace", "<document xmlns=\"urn:not-mxl\"/>");

    let execution = analyze_mxl_info(
        &path_args(&template_path),
        &context,
        &WorkspaceSupportStateReader::new(&context),
    );

    let outcome = execution.outcome;
    assert!(!outcome.ok, "{outcome:?}");
    assert!(execution.data.is_none());
    assert!(outcome.artifacts.is_empty());
    assert!(outcome
        .errors
        .iter()
        .any(|error| error.contains("urn:not-mxl") && error.contains(MXL_DOCUMENT_NS)));
    let _ = fs::remove_dir_all(&context.cwd);
}

fn path_args(path: &Path) -> Map<String, Value> {
    Map::from_iter([(
        "TemplatePath".to_string(),
        json!(path.display().to_string()),
    )])
}

fn write_test_mxl(name: &str, xml: &str) -> (WorkspaceContext, PathBuf) {
    let context = test_context(name);
    let template_path = mxl_info_template_path(&context);
    fs::write(&template_path, xml).unwrap();
    (context, template_path)
}

fn mxl_info_template_path(context: &WorkspaceContext) -> PathBuf {
    context
        .cwd
        .join("src/Reports/Sales/Templates/Main/Ext/Template.xml")
}

fn test_context(name: &str) -> WorkspaceContext {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let cwd = std::env::temp_dir().join(format!("unica-mxl-{name}-{nanos}"));
    let source = cwd.join("src");
    fs::create_dir_all(source.join("Reports/Sales/Templates/Main/Ext")).unwrap();
    fs::write(
        cwd.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Demo</Name></Properties><ChildObjects><Report>Sales</Report></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
    fs::write(
            source.join("Reports/Sales.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Report><Properties><Name>Sales</Name></Properties><ChildObjects><Template>Main</Template></ChildObjects></Report></MetaDataObject>"#,
        )
        .unwrap();
    fs::write(
            source.join("Reports/Sales/Templates/Main.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Template uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"><Properties><Name>Main</Name><TemplateType>SpreadsheetDocument</TemplateType></Properties></Template></MetaDataObject>"#,
        )
        .unwrap();
    WorkspaceContext {
        cwd: cwd.clone(),
        workspace_root: cwd.clone(),
        cache_root: cwd.join(".build/unica"),
        workspace_epoch: 0,
    }
}
