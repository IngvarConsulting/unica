#![allow(dead_code, unused_imports)]

use super::internal::*;
use crate::application::UnicaApplication;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::compile_transaction::{
    with_commit_failpoint, CommitFailpoint,
};
use crate::infrastructure::native_operations::single_file_publisher::with_before_commit_hook;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_context(name: &str) -> WorkspaceContext {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("unica-meta-{name}-{nanos}"));
    fs::create_dir_all(&root).unwrap();
    WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".build").join("unica"),
        workspace_epoch: 1,
    }
}

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

fn canonical_path(path: &Path) -> PathBuf {
    crate::infrastructure::source_roots::normalize_path_identity(path).unwrap()
}

const TEST_MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";
const TEST_V8_NS: &str = "http://v8.1c.ru/8.1/data/core";
const TEST_XR_NS: &str = "http://v8.1c.ru/8.3/xcf/readable";

fn write_owner(
    source_dir: &Path,
    object_type: &str,
    object_name: &str,
    languages: &[&str],
) -> PathBuf {
    fs::create_dir_all(source_dir.join("Languages")).unwrap();
    let language_nodes = languages
        .iter()
        .map(|name| format!("<Language>{name}</Language>"))
        .collect::<String>();
    let configuration = format!(
        r#"<MetaDataObject xmlns="{TEST_MD_NS}" version="2.20">
<Configuration uuid="11111111-1111-4111-8111-111111111111">
<Properties><Name>Owner</Name></Properties>
<ChildObjects>{language_nodes}<{object_type}>{object_name}</{object_type}></ChildObjects>
</Configuration></MetaDataObject>"#
    );
    fs::write(source_dir.join("Configuration.xml"), configuration).unwrap();
    source_dir.to_path_buf()
}

fn meta_validate_args(path: &Path) -> Map<String, Value> {
    Map::from_iter([
        (
            "ObjectPath".to_string(),
            Value::String(path.display().to_string()),
        ),
        ("Detailed".to_string(), Value::Bool(true)),
    ])
}

fn sample_meta_named(object_type: &str, object_name: &str) -> String {
    sample_meta_object_xml(object_type, object_name, "", "\t\t<ChildObjects/>")
}

fn sample_document_xml(register_records: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
	<Document uuid="11111111-1111-4111-8111-111111111111">
		<Properties>
			<Name>SampleShipment</Name>
			<Synonym/>
			<Comment/>
			{register_records}
			<PostInPrivilegedMode>true</PostInPrivilegedMode>
			<UnpostInPrivilegedMode>true</UnpostInPrivilegedMode>
		</Properties>
		<ChildObjects/>
	</Document>
</MetaDataObject>
"#
    )
}

fn sample_meta_object_xml(
    object_type: &str,
    object_name: &str,
    extra_properties: &str,
    child_objects: &str,
) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
	<{object_type} uuid="11111111-1111-4111-8111-111111111111">
		<Properties>
			<Name>{object_name}</Name>
			<Synonym/>
			<Comment/>
{extra_properties}
		</Properties>
{child_objects}
	</{object_type}>
</MetaDataObject>
"#
    )
}

fn sample_register_xml(object_type: &str) -> String {
    sample_meta_object_xml(object_type, "SampleStock", "", "\t\t<ChildObjects/>")
}

fn sample_enum_xml() -> String {
    sample_meta_object_xml("Enum", "SampleStatuses", "", "\t\t<ChildObjects/>")
}

fn sample_catalog_xml() -> String {
    sample_meta_object_xml(
        "Catalog",
        "SampleContracts",
        "\t\t\t<Owners/>\n\t\t\t<InputByString/>\n\t\t\t<BasedOn/>",
        "\t\t<ChildObjects/>",
    )
}

fn sample_document_journal_xml() -> String {
    sample_meta_object_xml(
        "DocumentJournal",
        "SampleJournal",
        "",
        "\t\t<ChildObjects/>",
    )
}

fn sample_document_with_child_objects(child_objects: &str) -> String {
    sample_document_xml("<RegisterRecords/>").replace(
        "\t\t<ChildObjects/>",
        &format!("\t\t<ChildObjects>\n{child_objects}\n\t\t</ChildObjects>"),
    )
}

fn sample_attribute(name: &str, type_xml: &str, fill_value_xml: &str) -> String {
    format!(
        "\t\t\t<Attribute uuid=\"33333333-3333-4333-8333-333333333333\">
\t\t\t\t<Properties>
\t\t\t\t\t<Name>{name}</Name>
\t\t\t\t\t<Synonym/>
\t\t\t\t\t<Comment/>
{type_xml}
\t\t\t\t\t<PasswordMode>false</PasswordMode>
\t\t\t\t\t<Format/>
\t\t\t\t\t<EditFormat/>
\t\t\t\t\t<ToolTip/>
\t\t\t\t\t<MarkNegatives>false</MarkNegatives>
\t\t\t\t\t<Mask/>
\t\t\t\t\t<MultiLine>false</MultiLine>
\t\t\t\t\t<ExtendedEdit>false</ExtendedEdit>
\t\t\t\t\t<MinValue xsi:nil=\"true\"/>
\t\t\t\t\t<MaxValue xsi:nil=\"true\"/>
\t\t\t\t\t<FillFromFillingValue>false</FillFromFillingValue>
{fill_value_xml}
\t\t\t\t\t<FillChecking>DontCheck</FillChecking>
\t\t\t\t\t<Indexing>DontIndex</Indexing>
\t\t\t\t</Properties>
\t\t\t</Attribute>"
    )
}

fn sample_object_with_tabular_fill_value(object_type: &str) -> String {
    sample_meta_object_xml(
            object_type,
            "SampleObject",
            "",
            "\t\t<ChildObjects>
\t\t\t<TabularSection uuid=\"22222222-2222-4222-8222-222222222222\">
\t\t\t\t<Properties>
\t\t\t\t\t<Name>SampleItems</Name>
\t\t\t\t\t<Synonym/>
\t\t\t\t\t<Comment/>
\t\t\t\t</Properties>
\t\t\t\t<ChildObjects>
\t\t\t\t\t<Attribute uuid=\"33333333-3333-4333-8333-333333333333\">
\t\t\t\t\t\t<Properties>
\t\t\t\t\t\t\t<Name>Status</Name>
\t\t\t\t\t\t\t<Synonym/>
\t\t\t\t\t\t\t<Comment/>
\t\t\t\t\t\t\t<Type>
\t\t\t\t\t\t\t\t<v8:Type>cfg:EnumRef.SampleStatus</v8:Type>
\t\t\t\t\t\t\t</Type>
\t\t\t\t\t\t\t<FillValue xsi:type=\"xr:DesignTimeRef\">Enum.SampleStatus.EnumValue.Default</FillValue>
\t\t\t\t\t\t\t<FillChecking>DontCheck</FillChecking>
\t\t\t\t\t\t</Properties>
\t\t\t\t\t</Attribute>
\t\t\t\t</ChildObjects>
\t\t\t</TabularSection>
\t\t</ChildObjects>",
        )
}

fn sample_object_with_line_number_length(object_type: &str, line_number_length: &str) -> String {
    sample_meta_object_xml(
        object_type,
        "SampleObject",
        "",
        &format!(
            "\t\t<ChildObjects>
\t\t\t<TabularSection uuid=\"22222222-2222-4222-8222-222222222222\">
\t\t\t\t<Properties>
\t\t\t\t\t<Name>SampleItems</Name>
\t\t\t\t\t<Synonym/>
\t\t\t\t\t<Comment/>
\t\t\t\t\t<ToolTip/>
\t\t\t\t\t<FillChecking>DontCheck</FillChecking>
\t\t\t\t\t<LineNumberLength>{line_number_length}</LineNumberLength>
\t\t\t\t</Properties>
\t\t\t\t<ChildObjects/>
\t\t\t</TabularSection>
\t\t</ChildObjects>"
        ),
    )
}

fn write_owner_with_compatibility(
    source_dir: &Path,
    object_type: &str,
    object_name: &str,
    compatibility_mode: &str,
) {
    fs::create_dir_all(source_dir).unwrap();
    write_file(
        &source_dir.join("Configuration.xml"),
        &format!(
            r#"<MetaDataObject xmlns="{TEST_MD_NS}" version="2.20">
<Configuration uuid="11111111-1111-4111-8111-111111111111">
<Properties>
<Name>Owner</Name>
<CompatibilityMode>{compatibility_mode}</CompatibilityMode>
</Properties>
<ChildObjects><{object_type}>{object_name}</{object_type}></ChildObjects>
</Configuration>
</MetaDataObject>"#
        ),
    );
}

fn register_record_args(object_path: &Path) -> Map<String, Value> {
    let mut args = Map::new();
    args.insert(
        "ObjectPath".to_string(),
        json!(object_path.display().to_string()),
    );
    args.insert("Operation".to_string(), json!("add-registerRecord"));
    args.insert(
        "Value".to_string(),
        json!("AccumulationRegister.SampleUnshippedGoods"),
    );
    args
}

fn meta_edit_args(object_path: &Path, operation: &str, value: &str) -> Map<String, Value> {
    let mut args = Map::new();
    args.insert(
        "ObjectPath".to_string(),
        json!(object_path.display().to_string()),
    );
    args.insert("Operation".to_string(), json!(operation));
    args.insert("Value".to_string(), json!(value));
    args
}

fn meta_edit_definition_args(object_path: &Path, definition_path: &Path) -> Map<String, Value> {
    let mut args = Map::new();
    args.insert(
        "ObjectPath".to_string(),
        json!(object_path.display().to_string()),
    );
    args.insert(
        "DefinitionFile".to_string(),
        json!(definition_path.display().to_string()),
    );
    args
}

fn sample_catalog_with_autonumbering(value: &str) -> String {
    sample_catalog_xml().replace(
        "\t\t\t<Owners/>",
        &format!("\t\t\t<Autonumbering>{value}</Autonumbering>\n\t\t\t<Owners/>"),
    )
}

fn boolean_contract_cases() -> &'static [(&'static str, &'static [&'static str])] {
    &[
        (
            "AccountingFlag",
            &[
                "PasswordMode",
                "MarkNegatives",
                "MultiLine",
                "ExtendedEdit",
                "FillFromFillingValue",
            ],
        ),
        (
            "AccountingRegister",
            &[
                "UseStandardCommands",
                "IncludeHelpInContents",
                "Correspondence",
                "EnableTotalsSplitting",
            ],
        ),
        (
            "AccumulationRegister",
            &[
                "UseStandardCommands",
                "IncludeHelpInContents",
                "EnableTotalsSplitting",
            ],
        ),
        (
            "AddressingAttribute",
            &[
                "PasswordMode",
                "MarkNegatives",
                "MultiLine",
                "ExtendedEdit",
                "FillFromFillingValue",
            ],
        ),
        (
            "Attribute",
            &[
                "PasswordMode",
                "MarkNegatives",
                "MultiLine",
                "ExtendedEdit",
                "FillFromFillingValue",
            ],
        ),
        (
            "BusinessProcess",
            &[
                "UseStandardCommands",
                "CheckUnique",
                "Autonumbering",
                "CreateTaskInPrivilegedMode",
                "IncludeHelpInContents",
                "UpdateDataHistoryImmediatelyAfterWrite",
                "ExecuteAfterWriteDataHistoryVersionProcessing",
            ],
        ),
        (
            "CalculationRegister",
            &[
                "UseStandardCommands",
                "ActionPeriod",
                "BasePeriod",
                "IncludeHelpInContents",
            ],
        ),
        (
            "Catalog",
            &[
                "Hierarchical",
                "LimitLevelCount",
                "FoldersOnTop",
                "UseStandardCommands",
                "CheckUnique",
                "Autonumbering",
                "QuickChoice",
                "IncludeHelpInContents",
                "UpdateDataHistoryImmediatelyAfterWrite",
                "ExecuteAfterWriteDataHistoryVersionProcessing",
            ],
        ),
        (
            "ChartOfAccounts",
            &[
                "UseStandardCommands",
                "IncludeHelpInContents",
                "CheckUnique",
                "QuickChoice",
                "AutoOrderByCode",
                "UpdateDataHistoryImmediatelyAfterWrite",
                "ExecuteAfterWriteDataHistoryVersionProcessing",
            ],
        ),
        (
            "ChartOfCalculationTypes",
            &[
                "UseStandardCommands",
                "QuickChoice",
                "ActionPeriodUse",
                "IncludeHelpInContents",
                "UpdateDataHistoryImmediatelyAfterWrite",
                "ExecuteAfterWriteDataHistoryVersionProcessing",
            ],
        ),
        (
            "ChartOfCharacteristicTypes",
            &[
                "UseStandardCommands",
                "IncludeHelpInContents",
                "Hierarchical",
                "FoldersOnTop",
                "CheckUnique",
                "Autonumbering",
                "QuickChoice",
                "UpdateDataHistoryImmediatelyAfterWrite",
                "ExecuteAfterWriteDataHistoryVersionProcessing",
            ],
        ),
        ("Command", &["ModifiesData"]),
        (
            "CommonModule",
            &[
                "Global",
                "ClientManagedApplication",
                "Server",
                "ExternalConnection",
                "ClientOrdinaryApplication",
                "Client",
                "ServerCall",
                "Privileged",
            ],
        ),
        (
            "Constant",
            &[
                "UseStandardCommands",
                "PasswordMode",
                "MarkNegatives",
                "MultiLine",
                "ExtendedEdit",
                "UpdateDataHistoryImmediatelyAfterWrite",
                "ExecuteAfterWriteDataHistoryVersionProcessing",
            ],
        ),
        (
            "DataProcessor",
            &["UseStandardCommands", "IncludeHelpInContents"],
        ),
        (
            "Dimension",
            &[
                "PasswordMode",
                "MarkNegatives",
                "MultiLine",
                "ExtendedEdit",
                "DenyIncompleteValues",
                "BaseDimension",
                "UseInTotals",
                "FillFromFillingValue",
                "Master",
                "MainFilter",
                "Balance",
            ],
        ),
        (
            "DocumentJournal",
            &["UseStandardCommands", "IncludeHelpInContents"],
        ),
        (
            "Document",
            &[
                "UseStandardCommands",
                "CheckUnique",
                "Autonumbering",
                "PostInPrivilegedMode",
                "UnpostInPrivilegedMode",
                "IncludeHelpInContents",
                "UpdateDataHistoryImmediatelyAfterWrite",
                "ExecuteAfterWriteDataHistoryVersionProcessing",
            ],
        ),
        ("Enum", &["UseStandardCommands", "QuickChoice"]),
        (
            "ExchangePlan",
            &[
                "UseStandardCommands",
                "QuickChoice",
                "DistributedInfoBase",
                "IncludeConfigurationExtensions",
                "IncludeHelpInContents",
                "UpdateDataHistoryImmediatelyAfterWrite",
                "ExecuteAfterWriteDataHistoryVersionProcessing",
            ],
        ),
        (
            "ExtDimensionAccountingFlag",
            &[
                "PasswordMode",
                "MarkNegatives",
                "MultiLine",
                "ExtendedEdit",
                "FillFromFillingValue",
            ],
        ),
        (
            "InformationRegister",
            &[
                "UseStandardCommands",
                "MainFilterOnPeriod",
                "IncludeHelpInContents",
                "EnableTotalsSliceFirst",
                "EnableTotalsSliceLast",
                "UpdateDataHistoryImmediatelyAfterWrite",
                "ExecuteAfterWriteDataHistoryVersionProcessing",
            ],
        ),
        ("Operation", &["Nillable", "Transactioned"]),
        ("Parameter", &["Nillable"]),
        ("Report", &["UseStandardCommands", "IncludeHelpInContents"]),
        (
            "Resource",
            &[
                "PasswordMode",
                "MarkNegatives",
                "MultiLine",
                "ExtendedEdit",
                "Balance",
                "FillFromFillingValue",
            ],
        ),
        ("ScheduledJob", &["Use", "Predefined"]),
        (
            "Task",
            &[
                "UseStandardCommands",
                "CheckUnique",
                "Autonumbering",
                "IncludeHelpInContents",
                "UpdateDataHistoryImmediatelyAfterWrite",
                "ExecuteAfterWriteDataHistoryVersionProcessing",
            ],
        ),
    ]
}

fn sample_with_invalid_boolean_node(node_type: &str, property: &str) -> String {
    if META_COMPILE_SUPPORTED_TYPES.contains(&node_type) {
        return sample_meta_object_xml(
            node_type,
            "BooleanProbe",
            &format!("\t\t\t<{property}>banana</{property}>"),
            "\t\t<ChildObjects/>",
        );
    }

    let (root_type, child) = match node_type {
        "AccountingFlag" | "ExtDimensionAccountingFlag" => ("ChartOfAccounts", node_type),
        "AddressingAttribute" => ("Task", node_type),
        "Dimension" | "Resource" => ("InformationRegister", node_type),
        "Operation" | "Parameter" => ("WebService", node_type),
        _ => ("Catalog", node_type),
    };
    let invalid_child = format!(
            "\t\t\t<{child}>\n\t\t\t\t<Properties>\n\t\t\t\t\t<Name>BooleanChild</Name>\n\t\t\t\t\t<Comment/>\n\t\t\t\t\t<{property}>banana</{property}>\n\t\t\t\t</Properties>\n\t\t\t\t<ChildObjects/>\n\t\t\t</{child}>"
        );
    sample_meta_object_xml(
        root_type,
        "BooleanProbe",
        "",
        &format!("\t\t<ChildObjects>\n{invalid_child}\n\t\t</ChildObjects>"),
    )
}

#[test]
fn edit_meta_rejects_invalid_boolean_inline_without_writing() {
    let context = temp_context("invalid-boolean-inline");
    let object_path = context.cwd.join("Catalogs/BooleanProbe.xml");
    let original = sample_catalog_with_autonumbering("true");

    for invalid in ["banana", "1", "0", "yes"] {
        write_file(&object_path, &original);
        let before = fs::read(&object_path).unwrap();
        let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "modify-property",
                &format!("Autonumbering={invalid}"),
            ),
            &context,
        );

        assert!(!outcome.ok, "{invalid}: {outcome:?}");
        assert!(
            outcome.errors.iter().any(|error| {
                error.contains("Autonumbering")
                    && error.contains("xs:boolean")
                    && error.contains("8.3.27")
            }),
            "{invalid}: {:?}",
            outcome.errors
        );
        assert_eq!(fs::read(&object_path).unwrap(), before, "{invalid}");
    }

    let _ = fs::remove_dir_all(&context.cwd);
}

/// Verifies inline previews expose the exact projected XML without writing it.
#[test]
fn preview_meta_edit_reports_projected_inline_diff_without_writing() {
    let context = temp_context("preview-inline-change");
    let object_path = context.cwd.join("Catalogs/Preview.xml");
    write_file(&object_path, &sample_catalog_xml());
    let before = fs::read(&object_path).unwrap();

    let execution = preview_meta_edit_with_data(
        &meta_edit_args(&object_path, "modify-property", "Comment=Previewed"),
        &context,
    );
    let outcome = &execution.outcome;

    assert!(outcome.ok, "{outcome:?}");
    assert!(outcome.summary.contains("planned native metadata edit"));
    assert_eq!(
        outcome.changes,
        vec![format!("would update {}", object_path.display())]
    );
    let data = execution.data.as_ref().unwrap();
    assert!(data.changed);
    assert_eq!(data.counts.modified, 1);
    let diff = data.diff.as_deref().unwrap_or_default();
    assert!(diff.contains("--- a/"), "{diff}");
    assert!(diff.contains("+++ b/"), "{diff}");
    assert!(diff.contains("-\t\t\t<Comment/>"), "{diff}");
    assert!(
        diff.contains("+\t\t\t<Comment>Previewed</Comment>"),
        "{diff}"
    );
    assert_eq!(fs::read(&object_path).unwrap(), before);

    let _ = fs::remove_dir_all(&context.cwd);
}

/// Verifies a missing preview target remains a useful dry-run failure.
#[test]
fn preview_meta_edit_rejects_missing_object() {
    let context = temp_context("preview-missing-object");
    let object_path = context.cwd.join("Catalogs/Missing.xml");

    let outcome = preview_meta_edit(
        &meta_edit_args(&object_path, "modify-property", "Comment=Previewed"),
        &context,
    );

    assert!(!outcome.ok, "{outcome:?}");
    assert!(outcome.summary.starts_with("dry run:"), "{outcome:?}");
    assert!(outcome
        .errors
        .iter()
        .any(|error| error.contains("Object file not found:")));
    assert!(outcome.changes.is_empty());
    assert!(outcome.artifacts.is_empty());

    let _ = fs::remove_dir_all(&context.cwd);
}

/// Verifies DefinitionFile previews use the same exact diff contract.
#[test]
fn preview_meta_edit_reports_definition_file_diff_without_writing() {
    let context = temp_context("preview-definition-file");
    let object_path = context.cwd.join("Catalogs/Preview.xml");
    let definition_path = context.cwd.join("meta-edit.json");
    write_file(&object_path, &sample_catalog_xml());
    write_file(
        &definition_path,
        &json!({"modify": {"properties": {"Comment": "Defined"}}}).to_string(),
    );
    let before = fs::read(&object_path).unwrap();

    let execution = preview_meta_edit_with_data(
        &meta_edit_definition_args(&object_path, &definition_path),
        &context,
    );
    let outcome = &execution.outcome;

    assert!(outcome.ok, "{outcome:?}");
    let diff = execution
        .data
        .as_ref()
        .unwrap()
        .diff
        .as_deref()
        .unwrap_or_default();
    assert!(diff.contains("--- a/"), "{diff}");
    assert!(diff.contains("-\t\t\t<Comment/>"), "{diff}");
    assert!(diff.contains("+\t\t\t<Comment>Defined</Comment>"), "{diff}");
    assert_eq!(fs::read(&object_path).unwrap(), before);

    let _ = fs::remove_dir_all(&context.cwd);
}

/// Verifies byte-identical previews report a no-op and omit the diff.
#[test]
fn preview_meta_edit_reports_no_file_changes_without_diff() {
    let context = temp_context("preview-no-op");
    let object_path = context.cwd.join("Catalogs/Preview.xml");
    let xml = sample_catalog_xml().replace("<Comment/>", "<Comment>Unchanged</Comment>");
    write_file(&object_path, &xml);
    let before = fs::read(&object_path).unwrap();

    let execution = preview_meta_edit_with_data(
        &meta_edit_args(&object_path, "modify-property", "Comment=Unchanged"),
        &context,
    );
    let outcome = &execution.outcome;

    assert!(outcome.ok, "{outcome:?}");
    assert!(
        outcome.summary.contains("found no metadata changes"),
        "{outcome:?}"
    );
    assert!(outcome.changes.is_empty());
    // ADR-0023: "nothing changed" is a value, and no diff means no diff.
    let data = execution.data.as_ref().unwrap();
    assert!(!data.changed);
    assert!(data.diff.is_none());
    assert_eq!(fs::read(&object_path).unwrap(), before);

    let _ = fs::remove_dir_all(&context.cwd);
}

/// Verifies a renderer fault does not turn a valid metadata edit into a failure.
#[test]
fn projected_diff_render_failure_becomes_warning() {
    let mut warnings = Vec::new();

    let diff =
        meta_edit_projected_diff(&mut warnings, Err("synthetic renderer failure".to_string()));

    assert!(diff.is_none());
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("synthetic renderer failure"));
}

#[test]
fn edit_meta_rejects_invalid_boolean_definition_map_without_writing() {
    let context = temp_context("invalid-boolean-definition");
    let object_path = context.cwd.join("Catalogs/BooleanProbe.xml");
    let definition_path = context.cwd.join("edit.json");
    write_file(&object_path, &sample_catalog_with_autonumbering("true"));
    write_file(
        &definition_path,
        &json!({"modify": {"properties": {"Autonumbering": "banana"}}}).to_string(),
    );
    let before = fs::read(&object_path).unwrap();

    let outcome = edit_meta(
        &meta_edit_definition_args(&object_path, &definition_path),
        &context,
    );

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome.errors.iter().any(|error| {
            error.contains("Autonumbering")
                && error.contains("xs:boolean")
                && error.contains("8.3.27")
        }),
        "{:?}",
        outcome.errors
    );
    assert_eq!(fs::read(&object_path).unwrap(), before);
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_normalizes_valid_boolean_values_to_canonical_xml() {
    let context = temp_context("valid-boolean-canonical");
    let object_path = context.cwd.join("Catalogs/BooleanProbe.xml");
    let definition_path = context.cwd.join("edit.json");

    for (raw, expected) in [("TRUE", "true"), ("false", "false")] {
        write_file(&object_path, &sample_catalog_with_autonumbering("true"));
        let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "modify-property",
                &format!("Autonumbering={raw}"),
            ),
            &context,
        );
        let updated = fs::read_to_string(&object_path).unwrap();
        assert!(outcome.ok, "{raw}: {outcome:?}");
        assert!(
            updated.contains(&format!("<Autonumbering>{expected}</Autonumbering>")),
            "{raw}: {updated}"
        );
    }

    for (raw, expected) in [(true, "true"), (false, "false")] {
        write_file(&object_path, &sample_catalog_with_autonumbering("false"));
        write_file(
            &definition_path,
            &json!({"modify": {"properties": {"Autonumbering": raw}}}).to_string(),
        );
        let outcome = edit_meta(
            &meta_edit_definition_args(&object_path, &definition_path),
            &context,
        );
        let updated = fs::read_to_string(&object_path).unwrap();
        assert!(outcome.ok, "{raw}: {outcome:?}");
        assert!(
            updated.contains(&format!("<Autonumbering>{expected}</Autonumbering>")),
            "{raw}: {updated}"
        );
    }

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_rejects_unrelated_edit_when_existing_boolean_is_invalid() {
    let context = temp_context("existing-invalid-boolean");
    let object_path = context.cwd.join("Catalogs/BooleanProbe.xml");
    write_file(&object_path, &sample_catalog_with_autonumbering("banana"));
    let before = fs::read(&object_path).unwrap();

    let outcome = edit_meta(
        &meta_edit_args(&object_path, "modify-property", "Comment=changed"),
        &context,
    );

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome.errors.iter().any(|error| {
            error.contains("Autonumbering")
                && error.contains("xs:boolean")
                && error.contains("8.3.27")
        }),
        "{:?}",
        outcome.errors
    );
    assert_eq!(fs::read(&object_path).unwrap(), before);
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_rejects_unrelated_edit_when_existing_enum_is_invalid() {
    let context = temp_context("existing-invalid-enum");
    let object_path = context.cwd.join("Catalogs/EnumProbe.xml");
    let original = sample_catalog_xml().replace(
        "\t\t\t<Owners/>",
        "\t\t\t<HierarchyType>Bogus</HierarchyType>\n\t\t\t<Owners/>",
    );
    write_file(&object_path, &original);
    let before = fs::read(&object_path).unwrap();

    let outcome = edit_meta(
        &meta_edit_args(&object_path, "modify-property", "Comment=changed"),
        &context,
    );

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome.errors.iter().any(|error| {
            error.contains("HierarchyType") && error.contains("Bogus") && error.contains("8.3.27")
        }),
        "{:?}",
        outcome.errors
    );
    assert_eq!(fs::read(&object_path).unwrap(), before);
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_checks_existing_child_enum_before_unrelated_edit() {
    let context = temp_context("existing-invalid-child-enum");
    let object_path = context.cwd.join("Documents/EnumProbe.xml");
    let attribute = sample_attribute(
        "ProbeAttribute",
        "\t\t\t\t\t<Type>\n\t\t\t\t\t\t<v8:Type>xs:string</v8:Type>\n\t\t\t\t\t</Type>",
        "\t\t\t\t\t<FillValue xsi:type=\"xs:string\"/>",
    )
    .replace(
        "<FillChecking>DontCheck</FillChecking>",
        "<FillChecking>ShowWarning</FillChecking>",
    );
    write_file(
        &object_path,
        &sample_document_with_child_objects(&attribute),
    );
    let before = fs::read(&object_path).unwrap();

    let outcome = edit_meta(
        &meta_edit_args(&object_path, "modify-property", "Comment=changed"),
        &context,
    );

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome.errors.iter().any(|error| {
            error.contains("Attribute.FillChecking")
                && error.contains("ShowWarning")
                && error.contains("8.3.27")
        }),
        "{:?}",
        outcome.errors
    );
    assert_eq!(fs::read(&object_path).unwrap(), before);
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_checks_every_relevant_boolean_object_and_child_property() {
    let context = temp_context("boolean-contract-table");
    let object_path = context.cwd.join("BooleanProbe.xml");

    for (node_type, properties) in boolean_contract_cases() {
        for property in *properties {
            write_file(
                &object_path,
                &sample_with_invalid_boolean_node(node_type, property),
            );
            let before = fs::read(&object_path).unwrap();
            let outcome = edit_meta(
                &meta_edit_args(&object_path, "modify-property", "Comment=changed"),
                &context,
            );

            assert!(!outcome.ok, "{node_type}.{property}: {outcome:?}");
            assert!(
                outcome.errors.iter().any(|error| {
                    error.contains(node_type)
                        && error.contains(property)
                        && error.contains("xs:boolean")
                }),
                "{node_type}.{property}: {:?}",
                outcome.errors
            );
            assert_eq!(
                fs::read(&object_path).unwrap(),
                before,
                "{node_type}.{property}"
            );
        }
    }

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_does_not_classify_text_or_attribute_use_as_boolean() {
    let context = temp_context("boolean-name-collisions");
    let catalog_path = context.cwd.join("Catalogs/BooleanProbe.xml");
    write_file(&catalog_path, &sample_catalog_with_autonumbering("true"));

    let text_outcome = edit_meta(
        &meta_edit_args(&catalog_path, "modify-property", "Comment=TRUE"),
        &context,
    );
    assert!(text_outcome.ok, "{text_outcome:?}");
    assert!(fs::read_to_string(&catalog_path)
        .unwrap()
        .contains("<Comment>TRUE</Comment>"));

    let document_path = context.cwd.join("Documents/BooleanProbe.xml");
    let document = sample_document_with_child_objects(&sample_attribute(
        "ProbeAttribute",
        "\t\t\t\t\t<Type>\n\t\t\t\t\t\t<v8:Type>xs:string</v8:Type>\n\t\t\t\t\t</Type>",
        "\t\t\t\t\t<FillValue xsi:type=\"xs:string\"/>",
    ));
    write_file(&document_path, &document);
    let use_outcome = edit_meta(
        &meta_edit_args(
            &document_path,
            "modify-attribute",
            "ProbeAttribute: use=ForItem",
        ),
        &context,
    );
    assert!(use_outcome.ok, "{use_outcome:?}");
    assert!(fs::read_to_string(&document_path)
        .unwrap()
        .contains("<Use>ForItem</Use>"));

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_modify_property_comment_replaces_self_closing_object_comment() {
    let context = temp_context("modify-property-comment-self-closing");
    let object_path = context.cwd.join("Catalogs").join("SampleContracts.xml");
    write_file(&object_path, &sample_catalog_xml());

    let outcome = edit_meta(
        &meta_edit_args(&object_path, "modify-property", "Comment=TEST-COMMENT"),
        &context,
    );

    assert!(outcome.ok, "{:?}", outcome.errors);
    let updated = fs::read_to_string(&object_path).unwrap();
    assert!(updated.contains("<Comment>TEST-COMMENT</Comment>"));
    assert_eq!(updated.matches("<Comment").count(), 1, "{updated}");
    assert!(!updated.contains("<Comment/>"), "{updated}");
    Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn meta_edit_post_write_failure_restores_the_exact_source_bytes() {
    let context = temp_context("post-write-rollback");
    let object_path = context.cwd.join("Catalogs").join("SampleContracts.xml");
    write_file(&object_path, &sample_catalog_xml());
    let original = fs::read(&object_path).unwrap();
    let args = meta_edit_args(&object_path, "modify-property", "Comment=UPDATED-COMMENT");

    let outcome = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
        edit_meta(&args, &context)
    });

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| error.contains("post-write validation")),
        "{outcome:?}"
    );
    assert_eq!(fs::read(&object_path).unwrap(), original);
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn meta_edit_rolls_back_if_format_owner_changes_during_publication() {
    let context = temp_context("format-owner-race");
    let source = context.cwd.join("src");
    let object_path = source.join("Catalogs/SampleContracts.xml");
    let owner_path = source.join("Configuration.xml");
    write_file(
        &context.cwd.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    );
    write_file(
            &owner_path,
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><Configuration/></MetaDataObject>",
        );
    write_file(&object_path, &sample_catalog_xml());
    let object_before = fs::read(&object_path).unwrap();
    let concurrent_owner = b"<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.21\"><Configuration/></MetaDataObject>".to_vec();
    let owner_for_hook = owner_path.clone();
    let owner_bytes_for_hook = concurrent_owner.clone();
    let args = meta_edit_args(&object_path, "modify-property", "Comment=UPDATED-COMMENT");

    let outcome = with_before_commit_hook(
        move |_| fs::write(&owner_for_hook, &owner_bytes_for_hook).unwrap(),
        || edit_meta(&args, &context),
    );

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome.errors.join("\n").contains("read guard"),
        "{outcome:?}"
    );
    assert_eq!(fs::read(&object_path).unwrap(), object_before);
    assert_eq!(fs::read(&owner_path).unwrap(), concurrent_owner);
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_modify_property_comment_replaces_existing_object_comment() {
    let context = temp_context("modify-property-comment-existing");
    let object_path = context.cwd.join("Catalogs").join("SampleContracts.xml");
    let xml = sample_catalog_xml().replace("<Comment/>", "<Comment>OLD</Comment>");
    write_file(&object_path, &xml);

    let outcome = edit_meta(
        &meta_edit_args(&object_path, "modify-property", "Comment=TEST-COMMENT"),
        &context,
    );

    assert!(outcome.ok, "{:?}", outcome.errors);
    let updated = fs::read_to_string(&object_path).unwrap();
    assert!(updated.contains("<Comment>TEST-COMMENT</Comment>"));
    assert!(!updated.contains("<Comment>OLD</Comment>"));
    assert_eq!(updated.matches("<Comment").count(), 1, "{updated}");
    Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_modify_property_comment_rejects_duplicate_without_mutation() {
    let context = temp_context("modify-property-comment-duplicate");
    let object_path = context.cwd.join("Catalogs").join("SampleContracts.xml");
    let xml = sample_catalog_xml().replace(
        "<Comment/>",
        "<Comment>FIRST</Comment>\n\t\t\t<Comment>SECOND</Comment>",
    );
    write_file(&object_path, &xml);
    let before = fs::read(&object_path).unwrap();

    let outcome = edit_meta(
        &meta_edit_args(&object_path, "modify-property", "Comment=TEST-COMMENT"),
        &context,
    );

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| error.contains("2 direct <Comment>")),
        "{:?}",
        outcome.errors
    );
    assert_eq!(fs::read(&object_path).unwrap(), before);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_modify_property_same_comment_is_byte_identical_noop() {
    let context = temp_context("modify-property-comment-noop");
    let object_path = context.cwd.join("Catalogs").join("SampleContracts.xml");
    let xml = sample_catalog_xml().replace("<Comment/>", "<Comment>TEST-COMMENT</Comment>");
    fs::create_dir_all(object_path.parent().unwrap()).unwrap();
    write_utf8_bom(&object_path, &xml).unwrap();
    let before = fs::read(&object_path).unwrap();

    let execution = edit_meta_with_data(
        &meta_edit_args(&object_path, "modify-property", "Comment=TEST-COMMENT"),
        &context,
    );
    let outcome = &execution.outcome;

    assert!(outcome.ok, "{:?}", outcome.errors);
    assert!(outcome.changes.is_empty(), "{:?}", outcome.changes);
    assert_eq!(fs::read(&object_path).unwrap(), before);
    assert!(!execution.data.as_ref().unwrap().changed);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_rejects_unknown_scalar_property_without_writing() {
    let context = temp_context("modify-property-unknown-scalar");
    let object_path = context.cwd.join("Catalogs").join("SampleContracts.xml");
    write_file(&object_path, &sample_catalog_xml());
    let before = fs::read(&object_path).unwrap();

    let outcome = edit_meta(
        &meta_edit_args(&object_path, "modify-property", "Bogus=x"),
        &context,
    );

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| error.contains("Bogus") && error.contains("does not exist")),
        "{outcome:?}"
    );
    assert!(outcome.changes.is_empty(), "{outcome:?}");
    assert_eq!(fs::read(&object_path).unwrap(), before);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_rejects_invalid_known_enum_without_writing() {
    let context = temp_context("modify-property-invalid-enum");
    let object_path = context.cwd.join("Catalogs").join("SampleContracts.xml");
    let xml = sample_catalog_xml().replace(
        "\t\t\t<Owners/>",
        "\t\t\t<HierarchyType>HierarchyFoldersAndItems</HierarchyType>\n\t\t\t<Owners/>",
    );
    write_file(&object_path, &xml);
    let before = fs::read(&object_path).unwrap();

    let outcome = edit_meta(
        &meta_edit_args(&object_path, "modify-property", "HierarchyType=Bogus"),
        &context,
    );

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome.errors.iter().any(|error| {
            error.contains("HierarchyType") && error.contains("Bogus") && error.contains("8.3.27")
        }),
        "{outcome:?}"
    );
    assert!(outcome.changes.is_empty(), "{outcome:?}");
    assert_eq!(fs::read(&object_path).unwrap(), before);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_adds_register_record_to_document() {
    let context = temp_context("add-register-record");
    let object_path = context.cwd.join("Documents").join("SampleShipment.xml");
    write_file(&object_path, &sample_document_xml("<RegisterRecords/>"));

    let outcome = edit_meta(&register_record_args(&object_path), &context);
    assert!(outcome.ok, "{:?}", outcome.errors);

    let updated = fs::read_to_string(&object_path).unwrap();
    assert!(updated.contains("<RegisterRecords>"));
    assert!(updated.contains(
        "<xr:Item xsi:type=\"xr:MDObjectRef\">AccumulationRegister.SampleUnshippedGoods</xr:Item>"
    ));
    Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_rejects_duplicate_register_record() {
    let context = temp_context("duplicate-register-record");
    let object_path = context.cwd.join("Documents").join("SampleShipment.xml");
    let original = sample_document_xml(
        r#"<RegisterRecords>
				<xr:Item xsi:type="xr:MDObjectRef">AccumulationRegister.SampleUnshippedGoods</xr:Item>
			</RegisterRecords>"#,
    );
    write_file(&object_path, &original);

    let outcome = edit_meta(&register_record_args(&object_path), &context);
    assert!(!outcome.ok, "{:?}", outcome.stdout);
    assert!(outcome
        .errors
        .iter()
        .any(|error| error.contains("already exists")));
    assert_eq!(fs::read_to_string(&object_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_register_record_dry_run_does_not_write_file() {
    let context = temp_context("dry-run-register-record");
    let object_path = context.cwd.join("Documents").join("SampleShipment.xml");
    let original = sample_document_xml("<RegisterRecords/>");
    write_file(&object_path, &original);

    let result = UnicaApplication::new()
        .call_tool("unica.meta.edit", &register_record_args(&object_path))
        .unwrap();

    assert!(result.ok);
    assert!(result.summary.contains("dry run"));
    assert_eq!(result.cache.mode, "dry-run");
    assert_eq!(fs::read_to_string(&object_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_adds_attribute_to_document() {
    let context = temp_context("add-attribute");
    let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
    write_file(&object_path, &sample_document_xml("<RegisterRecords/>"));

    let outcome = edit_meta(
        &meta_edit_args(
            &object_path,
            "add-attribute",
            "SampleCargoPlaceCode: String(50)",
        ),
        &context,
    );
    assert!(outcome.ok, "{:?}", outcome.errors);

    let updated = fs::read_to_string(&object_path).unwrap();
    assert!(updated.contains("<Attribute uuid=\""));
    assert!(updated.contains("<Name>SampleCargoPlaceCode</Name>"));
    assert!(updated.contains("<v8:Length>50</v8:Length>"));
    Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_adds_tabular_section_to_document() {
    let context = temp_context("add-tabular-section");
    let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
    write_file(&object_path, &sample_document_xml("<RegisterRecords/>"));

    let outcome = edit_meta(
        &meta_edit_args(&object_path, "add-ts", "SampleItems"),
        &context,
    );
    assert!(outcome.ok, "{:?}", outcome.errors);

    let updated = fs::read_to_string(&object_path).unwrap();
    assert!(updated.contains("<TabularSection uuid=\""));
    assert!(updated.contains("<Name>SampleItems</Name>"));
    Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_adds_tabular_section_with_inline_columns() {
    let context = temp_context("add-tabular-section-with-inline-columns");
    let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
    write_file(&object_path, &sample_document_xml("<RegisterRecords/>"));

    let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "add-ts",
                "SampleItems: SampleSourceDocument: DocumentRef.SampleSale, SampleQuantity: Number(15,3)",
            ),
            &context,
        );
    assert!(outcome.ok, "{:?}", outcome.errors);

    let updated = fs::read_to_string(&object_path).unwrap();
    assert!(updated.contains("<TabularSection uuid=\""));
    assert!(updated.contains("<Name>SampleItems</Name>"));
    assert!(!updated.contains("<Name>SampleItems: SampleSourceDocument"));
    assert!(updated.contains("<Name>SampleSourceDocument</Name>"));
    assert!(updated.contains("<v8:Type>cfg:DocumentRef.SampleSale</v8:Type>"));
    assert!(updated.contains("<Name>SampleQuantity</Name>"));
    Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_omits_line_number_length_for_external_tabular_sections() {
    for object_type in ["ExternalReport", "ExternalDataProcessor"] {
        let context = temp_context(&format!(
            "add-unbounded-tabular-section-{}",
            object_type.to_ascii_lowercase()
        ));
        let object_path = context.cwd.join(format!("{object_type}.xml"));
        write_file(
            &object_path,
            &sample_meta_named(object_type, "SampleExternalObject"),
        );

        let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "add-ts",
                "SampleItems: SampleValue: String(100)",
            ),
            &context,
        );

        assert!(outcome.ok, "{object_type}: {outcome:?}");
        let updated = fs::read_to_string(&object_path).unwrap();
        let document = Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();
        let section = document
            .descendants()
            .find(|node| {
                node.is_element()
                    && node.tag_name().name() == "TabularSection"
                    && meta_info_child(*node, "Properties")
                        .and_then(|properties| meta_info_child_text(properties, "Name"))
                        .as_deref()
                        == Some("SampleItems")
            })
            .expect("SampleItems tabular section");
        let properties = meta_info_child(section, "Properties").unwrap();
        assert!(
            meta_info_child(properties, "LineNumberLength").is_none(),
            "{object_type}: {updated}"
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }
}

#[test]
fn edit_meta_adds_attribute_to_tabular_section() {
    let context = temp_context("add-tabular-section-attribute");
    let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
    let mut xml = sample_document_xml("<RegisterRecords/>");
    xml = xml.replace(
            "\t\t<ChildObjects/>",
            "\t\t<ChildObjects>\n\t\t\t<TabularSection uuid=\"22222222-2222-4222-8222-222222222222\">\n\t\t\t\t<Properties>\n\t\t\t\t\t<Name>SampleItems</Name>\n\t\t\t\t\t<Synonym/>\n\t\t\t\t\t<Comment/>\n\t\t\t\t\t<ToolTip/>\n\t\t\t\t\t<FillChecking>DontCheck</FillChecking>\n\t\t\t\t</Properties>\n\t\t\t\t<ChildObjects/>\n\t\t\t</TabularSection>\n\t\t</ChildObjects>",
        );
    write_file(&object_path, &xml);

    let outcome = edit_meta(
        &meta_edit_args(
            &object_path,
            "add-ts-attribute",
            "SampleItems.SampleSourceDocument: DocumentRef.SampleSale",
        ),
        &context,
    );
    assert!(outcome.ok, "{:?}", outcome.errors);

    let updated = fs::read_to_string(&object_path).unwrap();
    assert!(updated.contains("<Name>SampleSourceDocument</Name>"));
    assert!(updated.contains("<v8:Type>cfg:DocumentRef.SampleSale</v8:Type>"));
    Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_adds_attribute_to_non_empty_tabular_section() {
    let context = temp_context("add-non-empty-tabular-section-attribute");
    let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
    let mut xml = sample_document_xml("<RegisterRecords/>");
    xml = xml.replace(
            "\t\t<ChildObjects/>",
            "\t\t<ChildObjects>\n\t\t\t<TabularSection uuid=\"22222222-2222-4222-8222-222222222222\">\n\t\t\t\t<Properties>\n\t\t\t\t\t<Name>SampleItems</Name>\n\t\t\t\t\t<Synonym/>\n\t\t\t\t\t<Comment/>\n\t\t\t\t\t<ToolTip/>\n\t\t\t\t\t<FillChecking>DontCheck</FillChecking>\n\t\t\t\t</Properties>\n\t\t\t\t<ChildObjects>\n\t\t\t\t\t<Attribute uuid=\"33333333-3333-4333-8333-333333333333\">\n\t\t\t\t\t\t<Properties>\n\t\t\t\t\t\t\t<Name>ExistingItem</Name>\n\t\t\t\t\t\t\t<Synonym/>\n\t\t\t\t\t\t\t<Comment/>\n\t\t\t\t\t\t\t<Type/>\n\t\t\t\t\t\t</Properties>\n\t\t\t\t\t</Attribute>\n\t\t\t\t</ChildObjects>\n\t\t\t</TabularSection>\n\t\t</ChildObjects>",
        );
    write_file(&object_path, &xml);

    let outcome = edit_meta(
        &meta_edit_args(
            &object_path,
            "add-ts-attribute",
            "SampleItems.SampleSourceDocument: DocumentRef.SampleSale",
        ),
        &context,
    );
    assert!(outcome.ok, "{:?}", outcome.errors);

    let updated = fs::read_to_string(&object_path).unwrap();
    assert!(updated.contains("<Name>ExistingItem</Name>"));
    assert!(updated.contains("<Name>SampleSourceDocument</Name>"));
    assert!(updated.contains("<v8:Type>cfg:DocumentRef.SampleSale</v8:Type>"));
    Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_adds_tabular_attribute_to_bom_xml_with_cyrillic_section() {
    let context = temp_context("add-bom-cyrillic-tabular-section-attribute");
    let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
    let mut xml = sample_document_xml("<RegisterRecords/>");
    xml = xml.replace(
            "\t\t<ChildObjects/>",
            "\t\t<ChildObjects>\n\t\t\t<TabularSection uuid=\"22222222-2222-4222-8222-222222222222\">\n\t\t\t\t<Properties>\n\t\t\t\t\t<Name>Товары</Name>\n\t\t\t\t\t<Synonym/>\n\t\t\t\t\t<Comment/>\n\t\t\t\t\t<ToolTip/>\n\t\t\t\t\t<FillChecking>DontCheck</FillChecking>\n\t\t\t\t</Properties>\n\t\t\t\t<ChildObjects>\n\t\t\t\t\t<Attribute uuid=\"33333333-3333-4333-8333-333333333333\">\n\t\t\t\t\t\t<Properties>\n\t\t\t\t\t\t\t<Name>Номенклатура</Name>\n\t\t\t\t\t\t\t<Synonym/>\n\t\t\t\t\t\t\t<Comment/>\n\t\t\t\t\t\t\t<Type/>\n\t\t\t\t\t\t</Properties>\n\t\t\t\t\t</Attribute>\n\t\t\t\t</ChildObjects>\n\t\t\t</TabularSection>\n\t\t</ChildObjects>",
        );
    write_file(&object_path, &format!("\u{feff}{xml}"));

    let outcome = edit_meta(
        &meta_edit_args(
            &object_path,
            "add-ts-attribute",
            "Товары.кшРеализация: DocumentRef.РеализацияТоваровУслуг",
        ),
        &context,
    );
    assert!(outcome.ok, "{:?}", outcome.errors);

    let updated = fs::read_to_string(&object_path).unwrap();
    assert!(updated.starts_with('\u{feff}'));
    assert!(updated.contains("<Name>Номенклатура</Name>"));
    assert!(updated.contains("<Name>кшРеализация</Name>"));
    Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_add_tabular_attribute_reports_missing_target() {
    let context = temp_context("missing-tabular-section");
    let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
    write_file(&object_path, &sample_document_xml("<RegisterRecords/>"));

    let outcome = edit_meta(
        &meta_edit_args(
            &object_path,
            "add-ts-attribute",
            "SampleItems.SampleSourceDocument: DocumentRef.SampleSale",
        ),
        &context,
    );

    assert!(!outcome.ok, "{:?}", outcome.stdout);
    assert!(outcome
        .errors
        .iter()
        .any(|error| error.contains("TabularSection 'SampleItems' not found")));

    let _ = fs::remove_dir_all(&context.cwd);
}
#[test]
fn edit_meta_removes_attribute_from_tabular_section() {
    let context = temp_context("remove-tabular-section-attribute");
    let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
    let mut xml = sample_document_xml("<RegisterRecords/>");
    xml = xml.replace(
            "\t\t<ChildObjects/>",
            "\t\t<ChildObjects>\n\t\t\t<TabularSection uuid=\"22222222-2222-4222-8222-222222222222\">\n\t\t\t\t<Properties>\n\t\t\t\t\t<Name>SampleItems</Name>\n\t\t\t\t\t<Synonym/>\n\t\t\t\t\t<Comment/>\n\t\t\t\t\t<ToolTip/>\n\t\t\t\t\t<FillChecking>DontCheck</FillChecking>\n\t\t\t\t</Properties>\n\t\t\t\t<ChildObjects>\n\t\t\t\t\t<Attribute uuid=\"33333333-3333-4333-8333-333333333333\">\n\t\t\t\t\t\t<Properties>\n\t\t\t\t\t\t\t<Name>ExistingItem</Name>\n\t\t\t\t\t\t\t<Synonym/>\n\t\t\t\t\t\t\t<Comment/>\n\t\t\t\t\t\t\t<Type/>\n\t\t\t\t\t\t</Properties>\n\t\t\t\t\t</Attribute>\n\t\t\t\t\t<Attribute uuid=\"44444444-4444-4444-8444-444444444444\">\n\t\t\t\t\t\t<Properties>\n\t\t\t\t\t\t\t<Name>ObsoleteItem</Name>\n\t\t\t\t\t\t\t<Synonym/>\n\t\t\t\t\t\t\t<Comment/>\n\t\t\t\t\t\t\t<Type/>\n\t\t\t\t\t\t</Properties>\n\t\t\t\t\t</Attribute>\n\t\t\t\t</ChildObjects>\n\t\t\t</TabularSection>\n\t\t</ChildObjects>",
        );
    write_file(&object_path, &xml);

    let outcome = edit_meta(
        &meta_edit_args(
            &object_path,
            "remove-ts-attribute",
            "SampleItems.ObsoleteItem",
        ),
        &context,
    );
    assert!(outcome.ok, "{:?}", outcome.errors);

    let updated = fs::read_to_string(&object_path).unwrap();
    assert!(updated.contains("<Name>ExistingItem</Name>"));
    assert!(!updated.contains("<Name>ObsoleteItem</Name>"));
    Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_remove_tabular_attribute_reports_missing_attribute() {
    let context = temp_context("missing-tabular-section-attribute");
    let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
    let mut xml = sample_document_xml("<RegisterRecords/>");
    xml = xml.replace(
            "\t\t<ChildObjects/>",
            "\t\t<ChildObjects>\n\t\t\t<TabularSection uuid=\"22222222-2222-4222-8222-222222222222\">\n\t\t\t\t<Properties>\n\t\t\t\t\t<Name>SampleItems</Name>\n\t\t\t\t\t<Synonym/>\n\t\t\t\t\t<Comment/>\n\t\t\t\t\t<ToolTip/>\n\t\t\t\t\t<FillChecking>DontCheck</FillChecking>\n\t\t\t\t</Properties>\n\t\t\t\t<ChildObjects/>\n\t\t\t</TabularSection>\n\t\t</ChildObjects>",
        );
    write_file(&object_path, &xml);

    let outcome = edit_meta(
        &meta_edit_args(
            &object_path,
            "remove-ts-attribute",
            "SampleItems.MissingItem",
        ),
        &context,
    );

    assert!(!outcome.ok, "{:?}", outcome.stdout);
    assert!(outcome
        .errors
        .iter()
        .any(|error| error.contains("Attribute 'SampleItems.MissingItem' not found")));

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_modifies_attribute_synonym_and_comment() {
    let context = temp_context("modify-attribute");
    let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
    let mut xml = sample_document_xml("<RegisterRecords/>");
    xml = xml.replace(
            "\t\t<ChildObjects/>",
            "\t\t<ChildObjects>\n\t\t\t<Attribute uuid=\"33333333-3333-4333-8333-333333333333\">\n\t\t\t\t<Properties>\n\t\t\t\t\t<Name>SampleCargoPlaceCode</Name>\n\t\t\t\t\t<Synonym/>\n\t\t\t\t\t<Comment/>\n\t\t\t\t\t<Type>\n\t\t\t\t\t\t<v8:Type>xs:string</v8:Type>\n\t\t\t\t\t\t<v8:StringQualifiers>\n\t\t\t\t\t\t\t<v8:Length>50</v8:Length>\n\t\t\t\t\t\t</v8:StringQualifiers>\n\t\t\t\t\t</Type>\n\t\t\t\t</Properties>\n\t\t\t</Attribute>\n\t\t</ChildObjects>",
        );
    write_file(&object_path, &xml);

    let outcome = edit_meta(
        &meta_edit_args(
            &object_path,
            "modify-attribute",
            "SampleCargoPlaceCode: synonym=Код грузового места, comment=TZ-SAMPLE",
        ),
        &context,
    );
    assert!(outcome.ok, "{:?}", outcome.errors);

    let updated = fs::read_to_string(&object_path).unwrap();
    assert!(updated.contains("<Name>SampleCargoPlaceCode</Name>"));
    assert!(updated.contains("<v8:content>Код грузового места</v8:content>"));
    assert!(updated.contains("<Comment>TZ-SAMPLE</Comment>"));
    assert!(updated.contains("<v8:Length>50</v8:Length>"));
    Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_modifies_tabular_attribute_synonym_comment_and_allowed_sign() {
    let context = temp_context("modify-tabular-attribute");
    let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
    let mut xml = sample_document_xml("<RegisterRecords/>");
    xml = xml.replace(
            "\t\t<ChildObjects/>",
            "\t\t<ChildObjects>\n\t\t\t<TabularSection uuid=\"22222222-2222-4222-8222-222222222222\">\n\t\t\t\t<Properties>\n\t\t\t\t\t<Name>SampleItems</Name>\n\t\t\t\t\t<Synonym/>\n\t\t\t\t\t<Comment/>\n\t\t\t\t\t<ToolTip/>\n\t\t\t\t\t<FillChecking>DontCheck</FillChecking>\n\t\t\t\t</Properties>\n\t\t\t\t<ChildObjects>\n\t\t\t\t\t<Attribute uuid=\"33333333-3333-4333-8333-333333333333\">\n\t\t\t\t\t\t<Properties>\n\t\t\t\t\t\t\t<Name>SampleQuantity</Name>\n\t\t\t\t\t\t\t<Synonym/>\n\t\t\t\t\t\t\t<Comment/>\n\t\t\t\t\t\t\t<Type>\n\t\t\t\t\t\t\t\t<v8:Type>xs:decimal</v8:Type>\n\t\t\t\t\t\t\t\t<v8:NumberQualifiers>\n\t\t\t\t\t\t\t\t\t<v8:Digits>15</v8:Digits>\n\t\t\t\t\t\t\t\t\t<v8:FractionDigits>3</v8:FractionDigits>\n\t\t\t\t\t\t\t\t</v8:NumberQualifiers>\n\t\t\t\t\t\t\t</Type>\n\t\t\t\t\t\t</Properties>\n\t\t\t\t\t</Attribute>\n\t\t\t\t</ChildObjects>\n\t\t\t</TabularSection>\n\t\t</ChildObjects>",
        );
    write_file(&object_path, &format!("\u{feff}{xml}"));

    let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "modify-ts-attribute",
                "SampleItems.SampleQuantity: synonym=Количество, comment=TZ-SAMPLE, v8:AllowedSign=Nonnegative",
            ),
            &context,
        );
    assert!(outcome.ok, "{:?}", outcome.errors);

    let updated = fs::read_to_string(&object_path).unwrap();
    assert!(updated.starts_with('\u{feff}'));
    assert!(updated.contains("<Name>SampleQuantity</Name>"));
    assert!(updated.contains("<v8:content>Количество</v8:content>"));
    assert!(updated.contains("<Comment>TZ-SAMPLE</Comment>"));
    assert!(updated.contains("<v8:AllowedSign>Nonnegative</v8:AllowedSign>"));
    assert!(updated.contains("<v8:FractionDigits>3</v8:FractionDigits>"));
    Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_modifies_tabular_section_properties() {
    let context = temp_context("modify-tabular-section");
    let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
    let xml = sample_document_with_child_objects(
        "\t\t\t<TabularSection uuid=\"22222222-2222-4222-8222-222222222222\">
\t\t\t\t<Properties>
\t\t\t\t\t<Name>SampleItems</Name>
\t\t\t\t\t<Synonym/>
\t\t\t\t\t<Comment/>
\t\t\t\t\t<ToolTip/>
\t\t\t\t\t<FillChecking>DontCheck</FillChecking>
\t\t\t\t</Properties>
\t\t\t\t<ChildObjects/>
\t\t\t</TabularSection>",
    );
    write_file(&object_path, &format!("\u{feff}{xml}"));

    let outcome = edit_meta(
        &meta_edit_args(
            &object_path,
            "modify-ts",
            "SampleItems: synonym=Товарный состав, fillChecking=ShowError",
        ),
        &context,
    );
    assert!(outcome.ok, "{:?}", outcome.errors);

    let updated = fs::read_to_string(&object_path).unwrap();
    assert!(updated.starts_with('\u{feff}'));
    assert!(updated.contains("<Name>SampleItems</Name>"));
    assert!(updated.contains("<v8:content>Товарный состав</v8:content>"));
    assert!(updated.contains("<FillChecking>ShowError</FillChecking>"));
    Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_modifies_line_number_length_without_reformatting_xml() {
    let context = temp_context("modify-line-number-length");
    let source_dir = context.cwd.join("src");
    write_owner_with_compatibility(&source_dir, "Document", "SampleObject", "Version8_3_27");
    let object_path = source_dir.join("Documents").join("SampleObject.xml");
    let original = format!(
        "\u{feff}{}",
        sample_object_with_line_number_length("Document", "5").replace('\n', "\r\n")
    );
    write_file(&object_path, &original);

    let outcome = edit_meta(
        &meta_edit_args(&object_path, "modify-ts", "SampleItems: lineNumberLength=9"),
        &context,
    );

    assert!(outcome.ok, "{outcome:?}");
    let updated = fs::read_to_string(&object_path).unwrap();
    assert_eq!(
        updated,
        original.replace(
            "<LineNumberLength>5</LineNumberLength>",
            "<LineNumberLength>9</LineNumberLength>"
        )
    );
    assert!(updated.starts_with('\u{feff}'));
    assert!(updated.contains("\r\n"));

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_definition_file_modifies_line_number_length() {
    let context = temp_context("definition-line-number-length");
    let source_dir = context.cwd.join("src");
    write_owner_with_compatibility(&source_dir, "Document", "SampleObject", "Version8_3_27");
    let object_path = source_dir.join("Documents").join("SampleObject.xml");
    let definition_path = context.cwd.join("meta-edit.json");
    write_file(
        &object_path,
        &sample_object_with_line_number_length("Document", "5"),
    );
    write_file(
        &definition_path,
        r#"{
  "modify": {
    "tabularSections": {
      "SampleItems": {
        "lineNumberLength": 9
      }
    }
  }
}"#,
    );

    let outcome = edit_meta(
        &meta_edit_definition_args(&object_path, &definition_path),
        &context,
    );

    assert!(outcome.ok, "{outcome:?}");
    assert!(fs::read_to_string(&object_path)
        .unwrap()
        .contains("<LineNumberLength>9</LineNumberLength>"));

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_accepts_line_number_length_key_aliases() {
    let context = temp_context("line-number-length-aliases");
    let source_dir = context.cwd.join("src");
    write_owner_with_compatibility(&source_dir, "Document", "SampleObject", "Version8_3_27");
    let object_path = source_dir.join("Documents").join("SampleObject.xml");
    let original = sample_object_with_line_number_length("Document", "5");

    for key in [
        "lineNumberLength",
        "line_number_length",
        "line-number-length",
    ] {
        write_file(&object_path, &original);
        let outcome = edit_meta(
            &meta_edit_args(&object_path, "modify-ts", &format!("SampleItems: {key}=9")),
            &context,
        );

        assert!(outcome.ok, "{key}: {outcome:?}");
        assert!(
            fs::read_to_string(&object_path)
                .unwrap()
                .contains("<LineNumberLength>9</LineNumberLength>"),
            "{key}"
        );
    }

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_rejects_invalid_line_number_length_without_writing() {
    let context = temp_context("invalid-line-number-length");
    let source_dir = context.cwd.join("src");
    write_owner_with_compatibility(&source_dir, "Document", "SampleObject", "Version8_3_27");
    let object_path = source_dir.join("Documents").join("SampleObject.xml");
    let original = sample_object_with_line_number_length("Document", "5");

    for value in ["", "4", "10", "5.5", "text", "-1"] {
        write_file(&object_path, &original);
        let before = fs::read(&object_path).unwrap();
        let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "modify-ts",
                &format!("SampleItems: lineNumberLength={value}"),
            ),
            &context,
        );

        assert!(!outcome.ok, "{value}: {outcome:?}");
        assert!(
            outcome.errors.iter().any(|error| {
                error.contains("LineNumberLength")
                    && error.contains("integer")
                    && error.contains("5..=9")
            }),
            "{value}: {:?}",
            outcome.errors
        );
        assert_eq!(fs::read(&object_path).unwrap(), before, "{value}");
    }

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_rejects_line_number_length_for_unbounded_tabular_sections() {
    for object_type in [
        "Report",
        "DataProcessor",
        "ExternalReport",
        "ExternalDataProcessor",
    ] {
        let context = temp_context(&format!(
            "line-number-length-{}",
            object_type.to_ascii_lowercase()
        ));
        let object_path = if let Some(directory) = meta_compile_type_plural(object_type) {
            let source_dir = context.cwd.join("src");
            write_owner_with_compatibility(
                &source_dir,
                object_type,
                "SampleObject",
                "Version8_3_27",
            );
            source_dir.join(directory).join("SampleObject.xml")
        } else {
            context.cwd.join(format!("{object_type}.xml"))
        };
        let original = sample_object_with_line_number_length(object_type, "5");
        write_file(&object_path, &original);
        let before = fs::read(&object_path).unwrap();

        let outcome = edit_meta(
            &meta_edit_args(&object_path, "modify-ts", "SampleItems: lineNumberLength=9"),
            &context,
        );

        assert!(!outcome.ok, "{object_type}: {outcome:?}");
        assert!(
            outcome.errors.iter().any(|error| {
                error.contains("LineNumberLength") && error.contains("not applicable")
            }),
            "{object_type}: {:?}",
            outcome.errors
        );
        assert_eq!(fs::read(&object_path).unwrap(), before, "{object_type}");

        let _ = fs::remove_dir_all(&context.cwd);
    }
}

#[test]
fn edit_meta_rejects_line_number_length_fixed_by_compatibility_mode() {
    let context = temp_context("fixed-line-number-length");
    let source_dir = context.cwd.join("src");
    write_owner_with_compatibility(&source_dir, "Document", "SampleObject", "Version8_3_26");
    let object_path = source_dir.join("Documents").join("SampleObject.xml");
    let original = sample_object_with_line_number_length("Document", "5");
    write_file(&object_path, &original);
    let before = fs::read(&object_path).unwrap();

    let outcome = edit_meta(
        &meta_edit_args(&object_path, "modify-ts", "SampleItems: lineNumberLength=9"),
        &context,
    );

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome.errors.iter().any(|error| {
            error.contains("LineNumberLength")
                && error.contains("fixed at 5")
                && error.contains("Version8_3_26")
        }),
        "{:?}",
        outcome.errors
    );
    assert_eq!(fs::read(&object_path).unwrap(), before);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn line_number_length_policy_covers_supported_compatibility_generations() {
    for fixed in [
        "Version8_1",
        "Version8_2_13",
        "Version8_3_1",
        "Version8_3_25",
        "Version8_3_26",
    ] {
        assert!(
            matches!(
                meta_edit_line_number_length_policy_from_mode(fixed),
                MetaEditLineNumberLengthPolicy::FixedFive
            ),
            "{fixed}"
        );
    }
    for editable in ["DontUse", "Version8_3_27"] {
        assert!(
            matches!(
                meta_edit_line_number_length_policy_from_mode(editable),
                MetaEditLineNumberLengthPolicy::Editable
            ),
            "{editable}"
        );
    }
    assert!(matches!(
        meta_edit_line_number_length_policy_from_mode("Bogus"),
        MetaEditLineNumberLengthPolicy::UnknownCompatibility
    ));
}

#[test]
fn line_number_length_policy_uses_effective_platform_version() {
    for (mode, platform_line) in [
        ("DontUse", "8.3.27"),
        ("DontUse", "8.5.4"),
        ("Version8_3_27", "8.5.4"),
    ] {
        assert!(
            matches!(
                meta_edit_line_number_length_policy_for_platform(mode, platform_line),
                MetaEditLineNumberLengthPolicy::Editable
            ),
            "{mode} on {platform_line}"
        );
    }

    for (mode, platform_line) in [("DontUse", "8.3.26"), ("Version8_3_24", "8.5.4")] {
        assert!(
            matches!(
                meta_edit_line_number_length_policy_for_platform(mode, platform_line),
                MetaEditLineNumberLengthPolicy::FixedFive
            ),
            "{mode} on {platform_line}"
        );
    }

    for platform_line in ["8.3.27.2074", "invalid"] {
        assert!(
            matches!(
                meta_edit_line_number_length_policy_for_platform("DontUse", platform_line),
                MetaEditLineNumberLengthPolicy::UnknownCompatibility
            ),
            "{platform_line}"
        );
    }
}

#[test]
fn line_number_length_policy_rejects_unsupported_future_mode() {
    for unsupported in ["Version8_5_1", "Version999_0_0"] {
        assert!(
            matches!(
                meta_edit_line_number_length_policy_from_mode(unsupported),
                MetaEditLineNumberLengthPolicy::UnknownCompatibility
            ),
            "{unsupported}"
        );
    }
}

#[test]
fn edit_meta_rejects_source_map_remap_after_line_number_length_authorization() {
    let context = temp_context("line-number-length-source-map-race");
    let project_map = context.cwd.join("v8project.yaml");
    let source_dir = context.cwd.join("src");
    write_file(
            &project_map,
            "format: DESIGNER\nsource-set:\n  - name: configuration\n    type: CONFIGURATION\n    path: src\n",
        );
    write_owner_with_compatibility(&source_dir, "Document", "SampleObject", "Version8_3_27");
    write_owner_with_compatibility(
        &source_dir.join("Documents"),
        "Document",
        "SampleObject",
        "Version8_3_26",
    );
    let object_path = source_dir.join("Documents").join("SampleObject.xml");
    let original = sample_object_with_line_number_length("Document", "5");
    write_file(&object_path, &original);
    let before = fs::read(&object_path).unwrap();
    let project_map_for_hook = project_map.clone();

    let outcome = with_meta_edit_after_line_number_length_policy_hook(
        move || {
            fs::write(
                    project_map_for_hook,
                    "format: DESIGNER\nsource-set:\n  - name: remapped\n    type: CONFIGURATION\n    path: src/Documents\n",
                )
                .unwrap();
        },
        || {
            edit_meta(
                &meta_edit_args(&object_path, "modify-ts", "SampleItems: lineNumberLength=9"),
                &context,
            )
        },
    );

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| error.contains("v8project.yaml") && error.contains("changed")),
        "{:?}",
        outcome.errors
    );
    assert_eq!(fs::read(&object_path).unwrap(), before);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn line_number_length_owner_policy_is_requested_only_for_matching_changes() {
    assert!(meta_edit_inline_requests_line_number_length(
        "modify-ts",
        "Items: line-number-length=9"
    ));
    assert!(!meta_edit_inline_requests_line_number_length(
        "modify-ts",
        "Items: synonym=lineNumberLength"
    ));
    assert!(!meta_edit_inline_requests_line_number_length(
        "modify-attribute",
        "LineNumberLength: synonym=Length"
    ));

    assert!(meta_edit_definition_requests_line_number_length(&json!({
        "modify": {
            "tabularSections": {
                "Items": {"line_number_length": 9}
            }
        }
    })));
    assert!(!meta_edit_definition_requests_line_number_length(&json!({
        "modify": {
            "tabularSections": {
                "Items": {
                    "modify": {
                        "LineNumberLength": {"synonym": "Length"}
                    }
                }
            }
        }
    })));
}

#[test]
fn edit_meta_rejects_line_number_length_without_compatibility_context() {
    let context = temp_context("unknown-line-number-length-compatibility");
    let object_path = context.cwd.join("Documents").join("SampleObject.xml");
    let original = sample_object_with_line_number_length("Document", "5");
    write_file(&object_path, &original);
    let before = fs::read(&object_path).unwrap();

    let outcome = edit_meta(
        &meta_edit_args(&object_path, "modify-ts", "SampleItems: lineNumberLength=9"),
        &context,
    );

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome.errors.iter().any(|error| {
            error.contains("LineNumberLength")
                && error.contains("CompatibilityMode")
                && error.contains("cannot be determined")
        }),
        "{:?}",
        outcome.errors
    );
    assert_eq!(fs::read(&object_path).unwrap(), before);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_adds_register_dimensions_and_resources() {
    let context = temp_context("add-register-fields");
    let object_path = context
        .cwd
        .join("InformationRegisters")
        .join("SampleStock.xml");
    write_file(&object_path, &sample_register_xml("InformationRegister"));

    let dimension = edit_meta(
        &meta_edit_args(
            &object_path,
            "add-dimension",
            "SampleWarehouse: CatalogRef.Warehouses | master, mainFilter",
        ),
        &context,
    );
    assert!(dimension.ok, "{:?}", dimension.errors);

    let resource = edit_meta(
        &meta_edit_args(&object_path, "add-resource", "SampleQty: Number(15,3)"),
        &context,
    );
    assert!(resource.ok, "{:?}", resource.errors);

    let updated = fs::read_to_string(&object_path).unwrap();
    assert!(updated.contains("<Dimension uuid="));
    assert!(updated.contains("<Name>SampleWarehouse</Name>"));
    assert!(updated.contains("<Master>true</Master>"));
    assert!(updated.contains("<MainFilter>true</MainFilter>"));
    assert!(updated.contains("<Resource uuid="));
    assert!(updated.contains("<Name>SampleQty</Name>"));
    assert!(updated.contains("<v8:FractionDigits>3</v8:FractionDigits>"));
    Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_adds_and_removes_enum_values_and_simple_children() {
    let context = temp_context("enum-and-simple-children");
    let object_path = context.cwd.join("Enums").join("SampleStatuses.xml");
    write_file(&object_path, &sample_enum_xml());

    for (operation, value) in [
        ("add-enumValue", "Pending ;; Obsolete"),
        ("add-form", "FormItem"),
        ("add-template", "PrintTemplate"),
        ("add-command", "OpenCommand"),
        ("modify-enumValue", "Pending: synonym=Ожидает"),
        ("remove-enumValue", "Obsolete"),
    ] {
        let outcome = edit_meta(&meta_edit_args(&object_path, operation, value), &context);
        assert!(outcome.ok, "{operation}: {:?}", outcome.errors);
    }

    let updated = fs::read_to_string(&object_path).unwrap();
    assert!(updated.contains("<EnumValue uuid="));
    assert!(updated.contains("<Name>Pending</Name>"));
    assert!(updated.contains("<v8:content>Ожидает</v8:content>"));
    assert!(!updated.contains("<Name>Obsolete</Name>"));
    assert!(updated.contains("<Form uuid="));
    assert!(updated.contains("<FormType>Ordinary</FormType>"));
    assert!(updated.contains("<Template uuid="));
    assert!(updated.contains("<TemplateType>SpreadsheetDocument</TemplateType>"));
    assert!(updated.contains("<Command uuid="));
    assert!(updated.contains("<Representation>Auto</Representation>"));
    Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_rejects_invalid_enum_column_and_simple_child_names_without_writing() {
    let context = temp_context("invalid-added-child-names");
    let enum_path = context.cwd.join("Enums").join("SampleStatuses.xml");
    let journal_path = context
        .cwd
        .join("DocumentJournals")
        .join("SampleJournal.xml");
    let document_path = context.cwd.join("Documents").join("SampleDocument.xml");

    for (path, original, operation, value) in [
        (&enum_path, sample_enum_xml(), "add-enumValue", "Bad Name"),
        (&enum_path, sample_enum_xml(), "add-form", "Bad Name"),
        (&enum_path, sample_enum_xml(), "add-template", "Bad Name"),
        (&enum_path, sample_enum_xml(), "add-command", "Bad Name"),
        (
            &document_path,
            sample_document_xml("<RegisterRecords/>"),
            "add-ts",
            "Bad Name",
        ),
        (
            &journal_path,
            sample_document_journal_xml(),
            "add-column",
            "Bad Name: DocumentRef.SampleDocument",
        ),
    ] {
        write_file(path, &original);
        let before = fs::read(path).unwrap();

        let outcome = edit_meta(&meta_edit_args(path, operation, value), &context);

        assert!(!outcome.ok, "{operation}: {outcome:?}");
        assert!(
            outcome.errors.iter().any(|error| {
                error.contains("Bad Name") && error.contains("valid 1C identifier")
            }),
            "{operation}: {:?}",
            outcome.errors
        );
        assert!(outcome.changes.is_empty(), "{operation}: {outcome:?}");
        assert_eq!(fs::read(path).unwrap(), before, "{operation}");
    }

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_rejects_every_invalid_name_rename_before_mutating_xml() {
    let targets = [
        MetaEditModifyTarget::Attribute {
            fill_value_allowed: true,
        },
        MetaEditModifyTarget::RegisterField,
        MetaEditModifyTarget::EnumValue,
        MetaEditModifyTarget::Column,
        MetaEditModifyTarget::TabularSection {
            line_number_length: MetaEditLineNumberLengthPolicy::Editable,
        },
    ];

    for target in targets {
        let mut xml = "<Properties><Name>ValidName</Name></Properties>".to_string();
        let before = xml.clone();
        let length = xml.len();

        let error = meta_edit_modify_properties_range(&mut xml, 0..length, "name=Bad Name", target)
            .unwrap_err();

        assert!(error.contains("Bad Name"), "{error}");
        assert!(error.contains("valid 1C identifier"), "{error}");
        assert_eq!(xml, before);
    }
}

#[test]
fn edit_meta_rejects_invalid_object_name_rename_without_writing() {
    let context = temp_context("invalid-object-rename");
    let object_path = context.cwd.join("Catalogs").join("SampleContracts.xml");
    write_file(&object_path, &sample_catalog_xml());
    let before = fs::read(&object_path).unwrap();

    let outcome = edit_meta(
        &meta_edit_args(&object_path, "modify-property", "Name=Bad Name"),
        &context,
    );

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| { error.contains("Bad Name") && error.contains("valid 1C identifier") }),
        "{:?}",
        outcome.errors
    );
    assert!(outcome.changes.is_empty(), "{outcome:?}");
    assert_eq!(fs::read(&object_path).unwrap(), before);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_adds_document_journal_columns() {
    let context = temp_context("add-document-journal-column");
    let object_path = context
        .cwd
        .join("DocumentJournals")
        .join("SampleJournal.xml");
    write_file(&object_path, &sample_document_journal_xml());

    let outcome = edit_meta(
        &meta_edit_args(
            &object_path,
            "add-column",
            "SampleKind: EnumRef.SampleKinds",
        ),
        &context,
    );
    assert!(outcome.ok, "{:?}", outcome.errors);

    let updated = fs::read_to_string(&object_path).unwrap();
    assert!(updated.contains("<Column uuid="));
    assert!(updated.contains("<Name>SampleKind</Name>"));
    assert!(updated.contains("<References>"));
    assert!(updated.contains(">EnumRef.SampleKinds</xr:Item>"));
    Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_sets_adds_and_removes_complex_properties() {
    let context = temp_context("complex-properties");
    let object_path = context.cwd.join("Catalogs").join("SampleContracts.xml");
    write_file(&object_path, &sample_catalog_xml());

    for (operation, value) in [
        (
            "set-owners",
            "Catalog.SampleCounterparties ;; Catalog.SampleOrganizations",
        ),
        ("remove-owner", "Catalog.SampleOrganizations"),
        ("add-inputByString", "StandardAttribute.Description"),
        ("add-basedOn", "Document.SampleOrder"),
    ] {
        let outcome = edit_meta(&meta_edit_args(&object_path, operation, value), &context);
        assert!(outcome.ok, "{operation}: {:?}", outcome.errors);
    }

    let updated = fs::read_to_string(&object_path).unwrap();
    assert!(updated.contains("<Owners>"));
    assert!(updated.contains(">Catalog.SampleCounterparties</xr:Item>"));
    assert!(!updated.contains(">Catalog.SampleOrganizations</xr:Item>"));
    assert!(updated.contains("<InputByString>"));
    assert!(updated
        .contains("<xr:Field>Catalog.SampleContracts.StandardAttribute.Description</xr:Field>"));
    assert!(updated.contains(">Document.SampleOrder</xr:Item>"));
    Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_definition_file_processes_json_dsl() {
    let context = temp_context("definition-file-json-dsl");
    let object_path = context.cwd.join("Catalogs").join("SampleContracts.xml");
    let definition_path = context.cwd.join("meta-edit.json");
    write_file(&object_path, &sample_catalog_xml());
    write_file(
        &definition_path,
        r#"{
  "add": {
    "attributes": [
      { "name": "SampleNote", "type": ["String", "Number(10,2)"], "indexing": "Index" }
    ],
    "tabularSections": [
      { "name": "Items", "attrs": ["Item: CatalogRef.Items", "Qty: Number(15,3)"] }
    ],
    "forms": ["FormItem"]
  },
  "modify": {
    "properties": {
      "Owners": ["Catalog.SampleCounterparties"],
      "InputByString": ["StandardAttribute.Description"]
    },
    "tabularSections": {
      "Items": {
        "add": ["Discount: Number(5,2)"]
      }
    }
  }
}"#,
    );

    let outcome = edit_meta(
        &meta_edit_definition_args(&object_path, &definition_path),
        &context,
    );
    assert!(outcome.ok, "{:?}", outcome.errors);

    let updated = fs::read_to_string(&object_path).unwrap();
    assert!(updated.contains("<Name>SampleNote</Name>"));
    assert!(updated.contains("<v8:Type>xs:string</v8:Type>"));
    assert!(updated.contains("<v8:Type>xs:decimal</v8:Type>"));
    assert!(updated.contains("<Name>Items</Name>"));
    assert!(updated.contains("<Name>Discount</Name>"));
    assert!(updated.contains("<Name>FormItem</Name>"));
    assert!(updated.contains(">Catalog.SampleCounterparties</xr:Item>"));
    assert!(updated
        .contains("<xr:Field>Catalog.SampleContracts.StandardAttribute.Description</xr:Field>"));
    Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_rejects_tabular_section_type_change() {
    let context = temp_context("modify-tabular-section-type");
    let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
    let xml = sample_document_with_child_objects(
        "\t\t\t<TabularSection uuid=\"22222222-2222-4222-8222-222222222222\">
\t\t\t\t<Properties>
\t\t\t\t\t<Name>SampleItems</Name>
\t\t\t\t\t<Synonym/>
\t\t\t\t\t<Comment/>
\t\t\t\t\t<ToolTip/>
\t\t\t\t\t<FillChecking>DontCheck</FillChecking>
\t\t\t\t</Properties>
\t\t\t\t<ChildObjects/>
\t\t\t</TabularSection>",
    );
    write_file(&object_path, &xml);

    let outcome = edit_meta(
        &meta_edit_args(&object_path, "modify-ts", "SampleItems: type=String(50)"),
        &context,
    );

    assert!(!outcome.ok, "{:?}", outcome.stdout);
    assert!(outcome
        .errors
        .iter()
        .any(|error| error.contains("Unsupported modify property key 'type'")));
    assert!(!fs::read_to_string(&object_path).unwrap().contains("<Type>"));

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_add_attribute_supports_batch_values() {
    let context = temp_context("add-attribute-batch");
    let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
    write_file(&object_path, &sample_document_xml("<RegisterRecords/>"));

    let outcome = edit_meta(
        &meta_edit_args(
            &object_path,
            "add-attribute",
            "SampleCargoPlaceCode: String(50) ;; SampleWeight: Number(10,2)",
        ),
        &context,
    );
    assert!(outcome.ok, "{:?}", outcome.errors);

    let updated = fs::read_to_string(&object_path).unwrap();
    assert!(updated.contains("<Name>SampleCargoPlaceCode</Name>"));
    assert!(updated.contains("<v8:Length>50</v8:Length>"));
    assert!(updated.contains("<Name>SampleWeight</Name>"));
    assert!(updated.contains("<v8:Digits>10</v8:Digits>"));
    assert!(!updated.contains(";; SampleWeight"));
    Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_rejects_invalid_type_expressions_without_writing() {
    let context = temp_context("invalid-type-expression");
    let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
    let original = sample_document_with_child_objects(&sample_attribute(
        "Value",
        "\t\t\t\t\t<Type>\n\t\t\t\t\t\t<v8:Type>xs:string</v8:Type>\n\t\t\t\t\t</Type>",
        "\t\t\t\t\t<FillValue xsi:type=\"xs:string\"/>",
    ));

    for (operation, value) in [
        ("add-attribute", "Broken: String(foo)"),
        ("modify-attribute", "Value: type=Number(x,2)"),
    ] {
        write_file(&object_path, &original);
        let outcome = edit_meta(&meta_edit_args(&object_path, operation, value), &context);

        assert!(!outcome.ok, "{operation}: {:?}", outcome.stdout);
        assert!(
            outcome.errors.iter().any(|error| error.contains("8.3.27")),
            "{operation}: {:?}",
            outcome.errors
        );
        assert_eq!(fs::read_to_string(&object_path).unwrap(), original);
    }

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_add_attribute_supports_inline_position() {
    let context = temp_context("add-attribute-position");
    let object_path = context.cwd.join("Documents").join("SampleShipment.xml");
    let child_objects = format!(
            "{}\n{}",
            sample_attribute(
                "Organization",
                "\t\t\t\t\t<Type>\n\t\t\t\t\t\t<v8:Type>cfg:CatalogRef.Organizations</v8:Type>\n\t\t\t\t\t</Type>",
                "\t\t\t\t\t<FillValue xsi:nil=\"true\"/>",
            ),
            sample_attribute(
                "Comment",
                "\t\t\t\t\t<Type>\n\t\t\t\t\t\t<v8:Type>xs:string</v8:Type>\n\t\t\t\t\t</Type>",
                "\t\t\t\t\t<FillValue xsi:type=\"xs:string\"/>",
            )
        );
    write_file(
        &object_path,
        &sample_document_with_child_objects(&child_objects),
    );

    let outcome = edit_meta(
        &meta_edit_args(
            &object_path,
            "add-attribute",
            "Warehouse: CatalogRef.Warehouses >> after Organization",
        ),
        &context,
    );
    assert!(outcome.ok, "{:?}", outcome.errors);

    let updated = fs::read_to_string(&object_path).unwrap();
    let organization = updated.find("<Name>Organization</Name>").unwrap();
    let warehouse = updated.find("<Name>Warehouse</Name>").unwrap();
    let comment = updated.find("<Name>Comment</Name>").unwrap();
    assert!(organization < warehouse && warehouse < comment, "{updated}");
    Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_add_tabular_attribute_supports_json_position() {
    let context = temp_context("add-ts-attribute-json-position");
    let object_path = context.cwd.join("Documents").join("SampleShipment.xml");
    let definition_path = context.cwd.join("meta-edit.json");
    let xml = sample_document_with_child_objects(
        "\t\t\t<TabularSection uuid=\"33333333-3333-4333-8333-333333333333\">
\t\t\t\t<Properties>
\t\t\t\t\t<Name>Items</Name>
\t\t\t\t\t<Synonym/>
\t\t\t\t\t<Comment/>
\t\t\t\t\t<ToolTip/>
\t\t\t\t\t<FillChecking>DontCheck</FillChecking>
\t\t\t\t</Properties>
\t\t\t\t<ChildObjects>
\t\t\t\t\t<Attribute uuid=\"44444444-4444-4444-8444-444444444444\">
\t\t\t\t\t\t<Properties>
\t\t\t\t\t\t\t<Name>Price</Name>
\t\t\t\t\t\t\t<Synonym/>
\t\t\t\t\t\t\t<Comment/>
\t\t\t\t\t\t\t<Type>
\t\t\t\t\t\t\t\t<v8:Type>xs:decimal</v8:Type>
\t\t\t\t\t\t\t</Type>
\t\t\t\t\t\t</Properties>
\t\t\t\t\t</Attribute>
\t\t\t\t\t<Attribute uuid=\"55555555-5555-4555-8555-555555555555\">
\t\t\t\t\t\t<Properties>
\t\t\t\t\t\t\t<Name>Amount</Name>
\t\t\t\t\t\t\t<Synonym/>
\t\t\t\t\t\t\t<Comment/>
\t\t\t\t\t\t\t<Type>
\t\t\t\t\t\t\t\t<v8:Type>xs:decimal</v8:Type>
\t\t\t\t\t\t\t</Type>
\t\t\t\t\t\t</Properties>
\t\t\t\t\t</Attribute>
\t\t\t\t</ChildObjects>
\t\t\t</TabularSection>",
    );
    write_file(&object_path, &xml);
    write_file(
        &definition_path,
        r#"{
  "modify": {
    "tabularSections": {
      "Items": {
        "add": [
          { "name": "Discount", "type": "Number(5,2)", "before": "Amount" }
        ]
      }
    }
  }
}"#,
    );

    let outcome = edit_meta(
        &meta_edit_definition_args(&object_path, &definition_path),
        &context,
    );
    assert!(outcome.ok, "{:?}", outcome.errors);

    let updated = fs::read_to_string(&object_path).unwrap();
    let price = updated.find("<Name>Price</Name>").unwrap();
    let discount = updated.find("<Name>Discount</Name>").unwrap();
    let amount = updated.find("<Name>Amount</Name>").unwrap();
    assert!(price < discount && discount < amount, "{updated}");
    Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_rejects_attribute_rename_to_existing_name() {
    let context = temp_context("rename-attribute-duplicate");
    let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
    let child_objects = format!(
        "{}\n{}",
        sample_attribute(
            "ExistingA",
            "\t\t\t\t\t<Type>\n\t\t\t\t\t\t<v8:Type>xs:string</v8:Type>\n\t\t\t\t\t</Type>",
            "\t\t\t\t\t<FillValue xsi:type=\"xs:string\"/>"
        ),
        sample_attribute(
            "ExistingB",
            "\t\t\t\t\t<Type>\n\t\t\t\t\t\t<v8:Type>xs:string</v8:Type>\n\t\t\t\t\t</Type>",
            "\t\t\t\t\t<FillValue xsi:type=\"xs:string\"/>"
        )
    );
    write_file(
        &object_path,
        &sample_document_with_child_objects(&child_objects),
    );

    let outcome = edit_meta(
        &meta_edit_args(
            &object_path,
            "modify-attribute",
            "ExistingA: name=ExistingB",
        ),
        &context,
    );

    assert!(!outcome.ok, "{:?}", outcome.stdout);
    assert!(outcome
        .errors
        .iter()
        .any(|error| error.contains("Attribute 'ExistingB' already exists")));

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_rejects_tabular_attribute_rename_to_existing_name() {
    let context = temp_context("rename-tabular-attribute-duplicate");
    let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
    let xml = sample_document_with_child_objects(
        "\t\t\t<TabularSection uuid=\"22222222-2222-4222-8222-222222222222\">
\t\t\t\t<Properties>
\t\t\t\t\t<Name>SampleItems</Name>
\t\t\t\t\t<Synonym/>
\t\t\t\t\t<Comment/>
\t\t\t\t\t<ToolTip/>
\t\t\t\t\t<FillChecking>DontCheck</FillChecking>
\t\t\t\t</Properties>
\t\t\t\t<ChildObjects>
\t\t\t\t\t<Attribute uuid=\"33333333-3333-4333-8333-333333333333\">
\t\t\t\t\t\t<Properties>
\t\t\t\t\t\t\t<Name>ExistingA</Name>
\t\t\t\t\t\t\t<Synonym/>
\t\t\t\t\t\t\t<Comment/>
\t\t\t\t\t\t\t<Type/>
\t\t\t\t\t\t</Properties>
\t\t\t\t\t</Attribute>
\t\t\t\t\t<Attribute uuid=\"44444444-4444-4444-8444-444444444444\">
\t\t\t\t\t\t<Properties>
\t\t\t\t\t\t\t<Name>ExistingB</Name>
\t\t\t\t\t\t\t<Synonym/>
\t\t\t\t\t\t\t<Comment/>
\t\t\t\t\t\t\t<Type/>
\t\t\t\t\t\t</Properties>
\t\t\t\t\t</Attribute>
\t\t\t\t</ChildObjects>
\t\t\t</TabularSection>",
    );
    write_file(&object_path, &xml);

    let outcome = edit_meta(
        &meta_edit_args(
            &object_path,
            "modify-ts-attribute",
            "SampleItems.ExistingA: name=ExistingB",
        ),
        &context,
    );

    assert!(!outcome.ok, "{:?}", outcome.stdout);
    assert!(outcome
        .errors
        .iter()
        .any(|error| error.contains("Attribute 'SampleItems.ExistingB' already exists")));

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_type_change_updates_existing_fill_value() {
    let context = temp_context("modify-attribute-type-fill-value");
    let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
    let xml = sample_document_with_child_objects(&sample_attribute(
            "SampleCargoPlaceCode",
            "\t\t\t\t\t<Type>\n\t\t\t\t\t\t<v8:Type>xs:string</v8:Type>\n\t\t\t\t\t\t<v8:StringQualifiers>\n\t\t\t\t\t\t\t<v8:Length>50</v8:Length>\n\t\t\t\t\t\t</v8:StringQualifiers>\n\t\t\t\t\t</Type>",
            "\t\t\t\t\t<FillValue xsi:type=\"xs:string\"/>",
        ));
    write_file(&object_path, &xml);

    let outcome = edit_meta(
        &meta_edit_args(
            &object_path,
            "modify-attribute",
            "SampleCargoPlaceCode: type=Number(15,2)",
        ),
        &context,
    );
    assert!(outcome.ok, "{:?}", outcome.errors);

    let updated = fs::read_to_string(&object_path).unwrap();
    assert!(updated.contains("<v8:Type>xs:decimal</v8:Type>"));
    assert!(updated.contains("<FillValue xsi:type=\"xs:decimal\">0</FillValue>"));
    assert!(!updated.contains("<FillValue xsi:type=\"xs:string\"/>"));
    Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_sets_enum_fill_value() {
    let context = temp_context("modify-attribute-enum-fill-value");
    let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
    let xml = format!(
            "\u{feff}{}",
            sample_document_with_child_objects(&sample_attribute(
                "Status",
                "\t\t\t\t\t<Type>\n\t\t\t\t\t\t<v8:Type>cfg:EnumRef.SampleStatus</v8:Type>\n\t\t\t\t\t</Type>",
                "\t\t\t\t\t<FillValue xsi:nil=\"true\"/>",
            ))
            .replace("<Synonym/>", "<Synonym />")
            .trim_end_matches('\n')
            .replace('\n', "\r\n")
        );
    let mut patched = xml.clone();
    let modified = meta_edit_modify_top_attribute_properties(
        &mut patched,
        "Status",
        "fillValue=Enum.SampleStatus.EnumValue.Default",
    )
    .unwrap();
    let expected = xml.replace(
        "<FillValue xsi:nil=\"true\"/>",
        "<FillValue xsi:type=\"xr:DesignTimeRef\">Enum.SampleStatus.EnumValue.Default</FillValue>",
    );
    assert_eq!(modified, 1);
    assert_eq!(patched, expected);
    write_file(&object_path, &xml);

    let outcome = edit_meta(
        &meta_edit_args(
            &object_path,
            "modify-attribute",
            "Status: fillValue=Enum.SampleStatus.EnumValue.Default",
        ),
        &context,
    );

    assert!(outcome.ok, "{:?}", outcome.errors);
    let updated = fs::read_to_string(&object_path).unwrap();
    assert_eq!(updated, expected);
    Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_resets_tabular_attribute_fill_value() {
    let context = temp_context("modify-tabular-attribute-reset-fill-value");
    let object_path = context
        .cwd
        .join("DataProcessors")
        .join("SampleProcessor.xml");
    let xml = sample_object_with_tabular_fill_value("DataProcessor");
    write_file(&object_path, &xml);

    let outcome = edit_meta(
        &meta_edit_args(
            &object_path,
            "modify-ts-attribute",
            "SampleItems.Status: fillValue=nil",
        ),
        &context,
    );

    assert!(outcome.ok, "{:?}", outcome.errors);
    let updated = fs::read_to_string(&object_path).unwrap();
    assert!(!updated.starts_with('\u{feff}'));
    assert!(updated.contains("<FillValue xsi:nil=\"true\"/>"));
    assert!(updated.contains("<FillChecking>DontCheck</FillChecking>"));
    assert!(!updated.contains("Enum.SampleStatus.EnumValue.Default"));
    Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_rejects_fill_value_for_stored_object_tabular_attribute() {
    let context = temp_context("reject-stored-tabular-fill-value");
    let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
    let original = sample_object_with_tabular_fill_value("Document");
    write_file(&object_path, &original);

    let outcome = edit_meta(
        &meta_edit_args(
            &object_path,
            "modify-ts-attribute",
            "SampleItems.Status: fillValue=nil",
        ),
        &context,
    );

    assert!(!outcome.ok, "{:?}", outcome.stdout);
    assert!(outcome
        .errors
        .iter()
        .any(|error| error.contains("Unsupported modify property key 'fillValue'")));
    assert_eq!(fs::read_to_string(&object_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_rejects_fill_value_when_property_is_absent() {
    let context = temp_context("modify-attribute-missing-fill-value-property");
    let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
    let original = sample_document_with_child_objects(&sample_attribute(
            "Status",
            "\t\t\t\t\t<Type>\n\t\t\t\t\t\t<v8:Type>cfg:EnumRef.SampleStatus</v8:Type>\n\t\t\t\t\t</Type>",
            "",
        ));
    write_file(&object_path, &original);

    let outcome = edit_meta(
        &meta_edit_args(
            &object_path,
            "modify-attribute",
            "Status: fillValue=Enum.SampleStatus.EnumValue.Default",
        ),
        &context,
    );

    assert!(!outcome.ok, "{:?}", outcome.stdout);
    assert!(outcome
        .errors
        .iter()
        .any(|error| error.contains("Property 'FillValue' is not available for this attribute")));
    assert_eq!(fs::read_to_string(&object_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_fill_value_dry_run_does_not_write_file() {
    let context = temp_context("modify-attribute-fill-value-dry-run");
    let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
    let original = sample_document_with_child_objects(&sample_attribute(
            "Status",
            "\t\t\t\t\t<Type>\n\t\t\t\t\t\t<v8:Type>cfg:EnumRef.SampleStatus</v8:Type>\n\t\t\t\t\t</Type>",
            "\t\t\t\t\t<FillValue xsi:nil=\"true\"/>",
        ));
    write_file(&object_path, &original);
    let mut args = meta_edit_args(
        &object_path,
        "modify-attribute",
        "Status: fillValue=Enum.SampleStatus.EnumValue.Default",
    );
    args.insert("dryRun".to_string(), json!(true));

    let result = UnicaApplication::new()
        .call_tool("unica.meta.edit", &args)
        .unwrap();

    assert!(result.ok, "{:?}", result.errors);
    assert!(result.summary.contains("dry run"));
    assert_eq!(fs::read_to_string(&object_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

fn validate_stdout_with_synonym(test_name: &str, synonym_xml: &str) -> String {
    let context = temp_context(test_name);
    let object_path = context.cwd.join("Documents").join("SampleShipment.xml");
    let xml = sample_document_xml("<RegisterRecords/>").replace("<Synonym/>", synonym_xml);
    write_owner(&context.cwd, "Document", "SampleShipment", &["Русский"]);
    write_language_fixture(&context, "Русский", "ru");
    write_file(&object_path, &xml);

    let mut args = serde_json::Map::new();
    args.insert(
        "ObjectPath".to_string(),
        json!(object_path.to_string_lossy().to_string()),
    );
    let outcome = validate_meta(&args, &context);
    let stdout = outcome.stdout.clone().unwrap_or_default();

    let _ = fs::remove_dir_all(&context.cwd);
    stdout
}

fn localized_text(items: &[(&str, &str)]) -> String {
    items
            .iter()
            .map(|(language, content)| {
                format!(
                    "<v8:item><v8:lang>{language}</v8:lang><v8:content>{content}</v8:content></v8:item>"
                )
            })
            .collect::<String>()
}

fn write_language_fixture(context: &WorkspaceContext, name: &str, code: &str) {
    write_file(
        &context.cwd.join("Languages").join(format!("{name}.xml")),
        &sample_language_named(name, code),
    );
}

fn sample_language_named(name: &str, code: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
  <Language uuid="22222222-2222-4222-8222-222222222222">
    <Properties>
      <Name>{name}</Name>
      <Synonym/>
      <Comment/>
      <LanguageCode>{code}</LanguageCode>
    </Properties>
  </Language>
</MetaDataObject>
"#
    )
}

fn validate_registered_object(
    object_type: &str,
    object_name: &str,
    object_xml: &str,
    languages: &[(&str, &str)],
) -> AdapterOutcome {
    let context = temp_context(&format!("registered-{object_type}-{object_name}"));
    let language_names = languages.iter().map(|(name, _)| *name).collect::<Vec<_>>();
    let src = write_owner(
        &context.cwd.join("src"),
        object_type,
        object_name,
        &language_names,
    );
    for (name, code) in languages {
        write_file(
            &src.join("Languages").join(format!("{name}.xml")),
            &sample_language_named(name, code),
        );
    }
    let object = src
        .join(format!("{object_type}s"))
        .join(format!("{object_name}.xml"));
    write_file(&object, object_xml);
    let outcome = validate_meta(&meta_validate_args(&object), &context);
    let _ = fs::remove_dir_all(&context.cwd);
    outcome
}

fn outcome_text(outcome: &AdapterOutcome) -> String {
    format!(
        "{}\n{}\n{}",
        outcome.stdout.clone().unwrap_or_default(),
        outcome.warnings.join("\n"),
        outcome.errors.join("\n")
    )
}

fn localized_property(name: &str, values: &[(&str, &str)]) -> String {
    let items = values
        .iter()
        .map(|(language, content)| {
            format!(
                "<v8:item><v8:lang>{language}</v8:lang>\
                     <v8:content>{content}</v8:content></v8:item>"
            )
        })
        .collect::<String>();
    format!("<{name}>{items}</{name}>")
}

fn sample_common_module_named(name: &str, synonyms: &[(&str, &str)]) -> String {
    let synonym = localized_property("Synonym", synonyms);
    format!(
        r#"<MetaDataObject xmlns="{TEST_MD_NS}" xmlns:v8="{TEST_V8_NS}" version="2.20">
<CommonModule uuid="33333333-3333-4333-8333-333333333333">
<Properties><Name>{name}</Name>{synonym}<Comment/></Properties>
<ChildObjects/>
</CommonModule></MetaDataObject>"#
    )
}

fn sample_enum_with_presentations(
    name: &str,
    synonyms: &[(&str, &str)],
    list_presentations: &[(&str, &str)],
) -> String {
    let synonym = localized_property("Synonym", synonyms);
    let list_presentation = localized_property("ListPresentation", list_presentations);
    format!(
        r#"<MetaDataObject xmlns="{TEST_MD_NS}" xmlns:v8="{TEST_V8_NS}"
 xmlns:xr="{TEST_XR_NS}" version="2.20">
<Enum uuid="44444444-4444-4444-8444-444444444444">
<InternalInfo>
<xr:GeneratedType name="EnumRef.{name}" category="Ref">
<xr:TypeId>55555555-5555-4555-8555-555555555551</xr:TypeId>
<xr:ValueId>55555555-5555-4555-8555-555555555552</xr:ValueId>
</xr:GeneratedType>
<xr:GeneratedType name="EnumManager.{name}" category="Manager">
<xr:TypeId>55555555-5555-4555-8555-555555555553</xr:TypeId>
<xr:ValueId>55555555-5555-4555-8555-555555555554</xr:ValueId>
</xr:GeneratedType>
<xr:GeneratedType name="EnumList.{name}" category="List">
<xr:TypeId>55555555-5555-4555-8555-555555555555</xr:TypeId>
<xr:ValueId>55555555-5555-4555-8555-555555555556</xr:ValueId>
</xr:GeneratedType>
</InternalInfo>
<Properties><Name>{name}</Name>{synonym}<Comment/>
{list_presentation}</Properties>
<ChildObjects/>
</Enum></MetaDataObject>"#
    )
}

fn validate_stdout_with_presentations(
    test_name: &str,
    synonym_items: &[(&str, &str)],
    list_presentation_items: &[(&str, &str)],
    configured_languages: &[(&str, &str)],
) -> String {
    assert!(
        !configured_languages.is_empty(),
        "presentation validation requires an explicit language profile"
    );
    let context = temp_context(test_name);
    let object_path = context.cwd.join("Documents").join("SampleShipment.xml");
    let synonym = format!("<Synonym>{}</Synonym>", localized_text(synonym_items));
    let list_presentation = format!(
        "<ListPresentation>{}</ListPresentation>",
        localized_text(list_presentation_items)
    );
    let xml = sample_document_xml("<RegisterRecords/>")
        .replace("<Synonym/>", &synonym)
        .replace("<Comment/>", &format!("<Comment/>{list_presentation}"));
    let language_names = configured_languages
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>();
    write_owner(&context.cwd, "Document", "SampleShipment", &language_names);
    write_file(&object_path, &xml);

    for (name, code) in configured_languages {
        write_language_fixture(&context, name, code);
    }

    let mut args = serde_json::Map::new();
    args.insert(
        "ObjectPath".to_string(),
        json!(object_path.to_string_lossy().to_string()),
    );
    let stdout = validate_meta(&args, &context).stdout.unwrap_or_default();

    let _ = fs::remove_dir_all(&context.cwd);
    stdout
}

#[test]
fn validate_meta_allows_self_closing_synonym() {
    let stdout = validate_stdout_with_synonym("validate-empty-synonym", "<Synonym/>");
    assert!(!stdout.contains("Synonym is empty"), "{stdout}");
}

#[test]
fn validate_meta_rejects_non_external_object_without_owner() {
    let context = temp_context("missing-owner");
    let object = context.cwd.join("Enums/Detached.xml");
    write_file(&object, &sample_meta_named("Enum", "Detached"));

    let outcome = validate_meta(&meta_validate_args(&object), &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome.errors.join("\n").contains("Configuration.xml"),
        "{outcome:?}"
    );
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn validate_meta_rejects_object_missing_from_owner_registration() {
    let context = temp_context("missing-registration");
    let src = write_owner(&context.cwd.join("src"), "Enum", "Other", &["Русский"]);
    let object = src.join("Enums/Detached.xml");
    write_file(&object, &sample_meta_named("Enum", "Detached"));

    let outcome = validate_meta(&meta_validate_args(&object), &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome.errors.join("\n").contains("not registered"),
        "{outcome:?}"
    );
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn validate_meta_external_descriptor_ignores_neighbor_configuration() {
    let context = temp_context("external-owner");
    write_file(
        &context.cwd.join("Configuration.xml"),
        r#"<broken-neighbor version="2.21">"#,
    );
    let object = context.cwd.join("tools/Standalone.xml");
    write_file(
        &object,
        &sample_meta_named("ExternalDataProcessor", "Standalone"),
    );

    let inspection = inspect_meta_validation_reads(&object, &context);
    let owner = inspection.context.expect("external descriptor owns itself");

    assert_eq!(owner.owner_kind, MetaValidationOwnerKind::External);
    assert_eq!(inspection.paths, vec![canonical_path(&object)]);
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn meta_validation_context_classifies_registered_extension_owner() {
    let context = temp_context("extension-owner");
    write_file(
            &context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: extension\n    type: EXTENSION\n    path: extension\n",
        );
    let source_dir = write_owner(
        &context.cwd.join("extension"),
        "CommonModule",
        "ExtensionModule",
        &[],
    );
    let object = source_dir.join("CommonModules/ExtensionModule.xml");
    write_file(
        &object,
        &sample_meta_named("CommonModule", "ExtensionModule"),
    );

    let inspection = inspect_meta_validation_reads(&object, &context);
    let owner = inspection.context.expect("registered extension owner");

    assert_eq!(owner.owner_kind, MetaValidationOwnerKind::Extension);
    assert_eq!(
        inspection.paths,
        vec![
            canonical_path(&object),
            canonical_path(&source_dir.join("Configuration.xml"))
        ]
    );
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn validate_meta_rejects_list_type_without_registered_languages() {
    let context = temp_context("missing-language-profile");
    let src = write_owner(&context.cwd.join("src"), "Enum", "Statuses", &[]);
    let object = src.join("Enums/Statuses.xml");
    write_file(&object, &sample_meta_named("Enum", "Statuses"));

    let outcome = validate_meta(&meta_validate_args(&object), &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome
            .errors
            .join("\n")
            .contains("has no registered language profile"),
        "{outcome:?}"
    );
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn meta_validation_reads_missing_registered_language_before_reporting_error() {
    let context = temp_context("missing-language-file");
    let src = write_owner(&context.cwd.join("src"), "Enum", "Statuses", &["Russian"]);
    let object = src.join("Enums/Statuses.xml");
    write_file(&object, &sample_meta_named("Enum", "Statuses"));

    let inspection = inspect_meta_validation_reads(&object, &context);

    assert_eq!(
        inspection.paths,
        vec![
            canonical_path(&object),
            canonical_path(&src.join("Configuration.xml")),
            canonical_path(&src).join("Languages/Russian.xml")
        ]
    );
    let error = inspection
        .context
        .expect_err("missing registered language must fail");
    assert!(
        error.starts_with("registered language file not found: "),
        "{error}"
    );
    assert!(
        error.ends_with(&format!(
            "Languages{}Russian.xml",
            std::path::MAIN_SEPARATOR
        )),
        "{error}"
    );
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn validate_meta_rejects_malformed_registered_language() {
    let context = temp_context("malformed-language");
    let src = write_owner(&context.cwd.join("src"), "Enum", "Statuses", &["Russian"]);
    let object = src.join("Enums/Statuses.xml");
    let language = src.join("Languages/Russian.xml");
    write_file(&object, &sample_meta_named("Enum", "Statuses"));
    write_file(&language, "<broken-language");

    let outcome = validate_meta(&meta_validate_args(&object), &context);
    let errors = outcome.errors.join("\n");

    assert!(!outcome.ok, "{outcome:?}");
    assert!(errors.contains("failed to parse"), "{outcome:?}");
    assert!(
        errors.contains(&canonical_path(&language).display().to_string()),
        "{outcome:?}"
    );
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn validate_meta_rejects_empty_registered_language_code() {
    let context = temp_context("empty-language-code");
    let src = write_owner(&context.cwd.join("src"), "Enum", "Statuses", &["Russian"]);
    let object = src.join("Enums/Statuses.xml");
    let language = src.join("Languages/Russian.xml");
    write_file(&object, &sample_meta_named("Enum", "Statuses"));
    write_file(&language, &sample_language_named("Russian", ""));

    let outcome = validate_meta(&meta_validate_args(&object), &context);
    let errors = outcome.errors.join("\n");

    assert!(!outcome.ok, "{outcome:?}");
    assert!(errors.contains("empty LanguageCode"), "{outcome:?}");
    assert!(
        errors.contains(&canonical_path(&language).display().to_string()),
        "{outcome:?}"
    );
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn meta_validation_deduplicates_language_codes_in_registration_order() {
    let context = temp_context("language-code-order");
    let src = write_owner(
        &context.cwd.join("src"),
        "Enum",
        "Statuses",
        &["RussianOne", "English", "RussianTwo"],
    );
    let object = src.join("Enums/Statuses.xml");
    write_file(&object, &sample_meta_named("Enum", "Statuses"));
    for (name, code) in [
        ("RussianOne", "ru"),
        ("English", "en"),
        ("RussianTwo", "ru"),
    ] {
        write_file(
            &src.join("Languages").join(format!("{name}.xml")),
            &sample_language_named(name, code),
        );
    }

    let inspection = inspect_meta_validation_reads(&object, &context);
    let owner = inspection.context.expect("complete language profile");

    assert_eq!(owner.language_codes, vec!["ru", "en"]);
    assert_eq!(
        inspection.paths,
        vec![
            canonical_path(&object),
            canonical_path(&src.join("Configuration.xml")),
            canonical_path(&src.join("Languages/RussianOne.xml")),
            canonical_path(&src.join("Languages/English.xml")),
            canonical_path(&src.join("Languages/RussianTwo.xml")),
        ]
    );
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn meta_validate_batch_read_set_stably_deduplicates_shared_owner() {
    let context = temp_context("batch-read-set");
    let src = context.cwd.join("src");
    let configuration = src.join("Configuration.xml");
    let language = src.join("Languages/Russian.xml");
    let first = src.join("Enums/First.xml");
    let second = src.join("Enums/Second.xml");
    write_file(
        &configuration,
        &format!(
            r#"<MetaDataObject xmlns="{TEST_MD_NS}" version="2.20">
<Configuration uuid="11111111-1111-4111-8111-111111111111">
<Properties><Name>Owner</Name></Properties>
<ChildObjects><Language>Russian</Language><Enum>First</Enum><Enum>Second</Enum></ChildObjects>
</Configuration></MetaDataObject>"#
        ),
    );
    write_file(&language, &sample_language_named("Russian", "ru"));
    write_file(&first, &sample_meta_named("Enum", "First"));
    write_file(&second, &sample_meta_named("Enum", "Second"));
    let args = Map::from_iter([(
        "ObjectPath".to_string(),
        Value::String(format!("{}|{}", first.display(), second.display())),
    )]);

    let dependencies = meta_validate_format_dependency_paths(&args, &context).unwrap();

    assert_eq!(
        dependencies,
        vec![
            canonical_path(&first),
            canonical_path(&configuration),
            canonical_path(&language),
            canonical_path(&second)
        ]
    );
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn post_write_metadata_owner_shape_does_not_require_workspace_owner() {
    let context = temp_context("post-write-local");
    let object = context.cwd.join("CommonModules/Local.xml");
    write_file(
        &object,
        &sample_meta_object_xml("CommonModule", "Local", "", "\t\t<ChildObjects/>"),
    );
    write_file(
        &context.cwd.join("Configuration.xml"),
        "<malformed-neighbor",
    );

    validate_metadata_owner_shape_8_3_27(&object, &context, "test")
        .expect("post-write validation must not read a neighboring owner");

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn validate_meta_accepts_filled_short_synonym() {
    let synonym = "<Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Отгрузка</v8:content></v8:item></Synonym>";
    let stdout = validate_stdout_with_synonym("validate-filled-synonym", synonym);
    assert!(!stdout.contains("Synonym is empty"), "{stdout}");
    assert!(!stdout.contains("longer than 38 characters"), "{stdout}");
}

#[test]
fn validate_meta_warns_on_long_synonym() {
    let synonym = "<Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Очень длинное наименование для командного интерфейса</v8:content></v8:item></Synonym>";
    let stdout = validate_stdout_with_synonym("validate-long-synonym", synonym);
    assert!(stdout.contains("longer than 38 characters"), "{stdout}");
}

#[test]
fn validate_meta_allows_empty_synonym() {
    let stdout = validate_stdout_with_synonym(
        "validate-empty-synonym-is-allowed",
        "<Synonym><v8:item><v8:lang>en</v8:lang><v8:content/></v8:item></Synonym>",
    );
    assert!(!stdout.contains("Synonym is empty"), "{stdout}");
}

#[test]
fn validate_meta_does_not_apply_list_command_limit_to_common_module() {
    let outcome = validate_registered_object(
        "CommonModule",
        "LongModule",
        &sample_common_module_named(
            "LongModule",
            &[(
                "ru",
                "Очень длинный синоним общего модуля для проверки ограничения",
            )],
        ),
        &[],
    );
    let stdout = outcome_text(&outcome);

    assert!(outcome.ok, "{outcome:?}");
    assert!(!stdout.contains("longer than 38 characters"), "{stdout}");
}

#[test]
fn validate_meta_prefers_list_presentation_per_registered_language() {
    let outcome = validate_registered_object(
        "Enum",
        "Status",
        &sample_enum_with_presentations(
            "Status",
            &[
                (
                    "ru",
                    "Очень длинный синоним для командного интерфейса перечисления",
                ),
                ("en", "Status"),
            ],
            &[("ru", "Статусы")],
        ),
        &[("Русский", "ru"), ("English", "en")],
    );
    let stdout = outcome_text(&outcome);

    assert!(outcome.ok, "{outcome:?}");
    assert!(!stdout.contains("language 'ru'"), "{stdout}");
}

#[test]
fn validate_meta_uses_synonym_when_registered_language_has_no_list_presentation() {
    let outcome = validate_registered_object(
        "Enum",
        "Status",
        &sample_enum_with_presentations(
            "Status",
            &[(
                "en",
                "A very long status title intended for the command interface",
            )],
            &[("ru", "Статусы")],
        ),
        &[("Русский", "ru"), ("English", "en")],
    );
    let stdout = outcome_text(&outcome);

    assert!(outcome.ok, "{outcome:?}");
    assert!(stdout.contains("Synonym"), "{stdout}");
    assert!(stdout.contains("language 'en'"), "{stdout}");
}

#[test]
fn validate_meta_skips_missing_or_empty_text_for_registered_language() {
    let outcome = validate_registered_object(
        "Enum",
        "Status",
        &sample_enum_with_presentations(
            "Status",
            &[("ru", "Статус"), ("en", "")],
            &[("ru", "Статусы")],
        ),
        &[("Русский", "ru"), ("English", "en")],
    );
    let stdout = outcome_text(&outcome);

    assert!(outcome.ok, "{outcome:?}");
    assert!(!stdout.contains("language 'en'"), "{stdout}");
}

#[test]
fn validate_meta_checks_every_configured_language() {
    let stdout = validate_stdout_with_presentations(
        "validate-every-configured-language",
        &[
            ("ru", "Отгрузка"),
            (
                "en",
                "A very long shipment title intended for the command interface",
            ),
        ],
        &[],
        &[("Русский", "ru"), ("English", "en")],
    );
    assert!(stdout.contains("language 'en'"), "{stdout}");
    assert!(stdout.contains("longer than 38 characters"), "{stdout}");
}

#[test]
fn validate_meta_prefers_list_presentation_per_language() {
    let stdout = validate_stdout_with_presentations(
        "validate-list-presentation-per-language",
        &[
            ("ru", "Очень длинное наименование для командного интерфейса"),
            ("en", "Shipment"),
        ],
        &[("ru", "Отгрузки")],
        &[("Русский", "ru"), ("English", "en")],
    );
    assert!(!stdout.contains("language 'ru'"), "{stdout}");
    assert!(!stdout.contains("longer than 38 characters"), "{stdout}");
}

#[test]
fn validate_meta_ignores_unconfigured_translation() {
    let stdout = validate_stdout_with_presentations(
        "validate-ignore-unconfigured-language",
        &[
            (
                "en",
                "A very long shipment title intended for the command interface",
            ),
            ("ru", "Отгрузка"),
        ],
        &[],
        &[("Русский", "ru")],
    );
    assert!(!stdout.contains("longer than 38 characters"), "{stdout}");
}

#[test]
fn validate_meta_ignores_non_v8_language_elements() {
    let xml = sample_document_xml("<RegisterRecords/>")
            .replace(
                "xmlns:xsi=",
                "xmlns:foo=\"urn:unrelated\" xmlns:xsi=",
            )
            .replace(
                "<Synonym/>",
                "<Synonym><v8:item><foo:lang>en</foo:lang><v8:content>Shipment</v8:content></v8:item></Synonym>",
            );
    let document = Document::parse(&xml).unwrap();
    let type_node = document
        .root_element()
        .children()
        .find(|node| node.is_element())
        .unwrap();
    let properties = meta_info_child(type_node, "Properties").unwrap();
    let synonym = meta_info_child(properties, "Synonym");

    assert_eq!(
        meta_validate_localized_values(synonym),
        vec![(None, "Shipment".to_string())]
    );
}

fn validate_stdout_for_information_register(
    write_mode: &str,
    use_standard_commands: &str,
) -> String {
    // The validator requires an InternalInfo block with seven GeneratedType
    // entries and a Type block on every dimension; without them validation
    // fails with unrelated errors before the target warning is checked.
    let internal_info = concat!(
            "\t\t<InternalInfo>\n",
            "\t\t\t<xr:GeneratedType name=\"InformationRegisterRecord.SampleRegister\" category=\"Record\">\n",
            "\t\t\t\t<xr:TypeId>11111111-1111-4111-8111-111111111111</xr:TypeId>\n",
            "\t\t\t\t<xr:ValueId>21111111-1111-4111-8111-111111111111</xr:ValueId>\n",
            "\t\t\t</xr:GeneratedType>\n",
            "\t\t\t<xr:GeneratedType name=\"InformationRegisterManager.SampleRegister\" category=\"Manager\">\n",
            "\t\t\t\t<xr:TypeId>12222222-2222-4222-8222-222222222222</xr:TypeId>\n",
            "\t\t\t\t<xr:ValueId>22222222-2222-4222-8222-222222222222</xr:ValueId>\n",
            "\t\t\t</xr:GeneratedType>\n",
            "\t\t\t<xr:GeneratedType name=\"InformationRegisterSelection.SampleRegister\" category=\"Selection\">\n",
            "\t\t\t\t<xr:TypeId>13333333-3333-4333-8333-333333333333</xr:TypeId>\n",
            "\t\t\t\t<xr:ValueId>23333333-3333-4333-8333-333333333333</xr:ValueId>\n",
            "\t\t\t</xr:GeneratedType>\n",
            "\t\t\t<xr:GeneratedType name=\"InformationRegisterList.SampleRegister\" category=\"List\">\n",
            "\t\t\t\t<xr:TypeId>14444444-4444-4444-8444-444444444444</xr:TypeId>\n",
            "\t\t\t\t<xr:ValueId>24444444-4444-4444-8444-444444444444</xr:ValueId>\n",
            "\t\t\t</xr:GeneratedType>\n",
            "\t\t\t<xr:GeneratedType name=\"InformationRegisterRecordSet.SampleRegister\" category=\"RecordSet\">\n",
            "\t\t\t\t<xr:TypeId>15555555-5555-4555-8555-555555555555</xr:TypeId>\n",
            "\t\t\t\t<xr:ValueId>25555555-5555-4555-8555-555555555555</xr:ValueId>\n",
            "\t\t\t</xr:GeneratedType>\n",
            "\t\t\t<xr:GeneratedType name=\"InformationRegisterRecordKey.SampleRegister\" category=\"RecordKey\">\n",
            "\t\t\t\t<xr:TypeId>16666666-6666-4666-8666-666666666666</xr:TypeId>\n",
            "\t\t\t\t<xr:ValueId>26666666-6666-4666-8666-666666666666</xr:ValueId>\n",
            "\t\t\t</xr:GeneratedType>\n",
            "\t\t\t<xr:GeneratedType name=\"InformationRegisterRecordManager.SampleRegister\" category=\"RecordManager\">\n",
            "\t\t\t\t<xr:TypeId>17777777-7777-4777-8777-777777777777</xr:TypeId>\n",
            "\t\t\t\t<xr:ValueId>27777777-7777-4777-8777-777777777777</xr:ValueId>\n",
            "\t\t\t</xr:GeneratedType>\n",
            "\t\t</InternalInfo>",
        );
    let xml = sample_meta_object_xml(
            "InformationRegister",
            "SampleRegister",
            &format!(
                "\t\t\t<UseStandardCommands>{use_standard_commands}</UseStandardCommands>\n\t\t\t<WriteMode>{write_mode}</WriteMode>"
            ),
            "\t\t<ChildObjects>\n\t\t\t<Dimension uuid=\"66666666-6666-4666-8666-666666666666\">\n\t\t\t\t<Properties>\n\t\t\t\t\t<Name>SampleDimension</Name>\n\t\t\t\t\t<Type><v8:Type>xs:string</v8:Type></Type>\n\t\t\t\t</Properties>\n\t\t\t</Dimension>\n\t\t</ChildObjects>",
        )
        // Anchor on the register name so the dimension's more deeply indented
        // <Properties> (SampleDimension) is not matched by the shared substring.
        .replace(
            "\t\t<Properties>\n\t\t\t<Name>SampleRegister",
            &format!("{internal_info}\n\t\t<Properties>\n\t\t\t<Name>SampleRegister"),
        );
    let outcome = validate_registered_object(
        "InformationRegister",
        "SampleRegister",
        &xml,
        &[("Русский", "ru")],
    );
    assert!(outcome.ok, "{outcome:?}");
    outcome_text(&outcome)
}

const SUBORDINATE_REGISTER_WARNING: &str =
    "subordinate registers are not shown in the command interface";

#[test]
fn validate_meta_warns_on_subordinate_register_in_command_interface() {
    let stdout = validate_stdout_for_information_register("RecorderSubordinate", "true");
    assert!(stdout.contains(SUBORDINATE_REGISTER_WARNING), "{stdout}");
}

#[test]
fn validate_meta_allows_independent_register_in_command_interface() {
    let stdout = validate_stdout_for_information_register("Independent", "true");
    assert!(!stdout.contains(SUBORDINATE_REGISTER_WARNING), "{stdout}");
}

#[test]
fn validate_meta_allows_subordinate_register_without_standard_commands() {
    let stdout = validate_stdout_for_information_register("RecorderSubordinate", "false");
    assert!(!stdout.contains(SUBORDINATE_REGISTER_WARNING), "{stdout}");
}

#[test]
fn edit_meta_rejects_unknown_modify_attribute_key() {
    let context = temp_context("modify-attribute-unknown-key");
    let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
    let xml = sample_document_with_child_objects(&sample_attribute(
        "SampleCargoPlaceCode",
        "\t\t\t\t\t<Type>\n\t\t\t\t\t\t<v8:Type>xs:string</v8:Type>\n\t\t\t\t\t</Type>",
        "\t\t\t\t\t<FillValue xsi:type=\"xs:string\"/>",
    ));
    write_file(&object_path, &xml);

    let outcome = edit_meta(
        &meta_edit_args(
            &object_path,
            "modify-attribute",
            "SampleCargoPlaceCode: typo=1",
        ),
        &context,
    );

    assert!(!outcome.ok, "{:?}", outcome.stdout);
    assert!(outcome
        .errors
        .iter()
        .any(|error| error.contains("Unsupported modify property key 'typo'")));
    assert!(!fs::read_to_string(&object_path).unwrap().contains("<typo>"));

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_rejects_register_record_duplicate_with_formatted_text() {
    let context = temp_context("duplicate-register-record-formatted");
    let object_path = context.cwd.join("Documents").join("SampleShipment.xml");
    let original = sample_document_xml(
        r#"<RegisterRecords>
				<xr:Item xsi:type="xr:MDObjectRef">
					AccumulationRegister.SampleUnshippedGoods
				</xr:Item>
			</RegisterRecords>"#,
    );
    write_file(&object_path, &original);

    let outcome = edit_meta(&register_record_args(&object_path), &context);

    assert!(!outcome.ok, "{:?}", outcome.stdout);
    assert!(outcome
        .errors
        .iter()
        .any(|error| error.contains("already exists")));
    assert_eq!(fs::read_to_string(&object_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_dry_run_rejects_unsupported_operation() {
    let context = temp_context("dry-run-unsupported-operation");
    let object_path = context.cwd.join("Documents").join("SampleShipment.xml");
    write_file(&object_path, &sample_document_xml("<RegisterRecords/>"));
    let mut args = meta_edit_args(&object_path, "definitely-unsupported", "Value");
    args.insert("dryRun".to_string(), json!(true));

    let error = UnicaApplication::new()
        .call_tool("unica.meta.edit", &args)
        .unwrap_err();

    assert!(error.contains("unsupported Operation"));

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_meta_dry_run_accepts_definition_file_mode() {
    let context = temp_context("dry-run-definition-file");
    let object_path = context.cwd.join("Documents").join("SampleShipment.xml");
    let definition_path = context.cwd.join("edit.json");
    write_file(&object_path, &sample_document_xml("<RegisterRecords/>"));
    write_file(&definition_path, "{}");
    let mut args = Map::new();
    args.insert(
        "ObjectPath".to_string(),
        json!(object_path.display().to_string()),
    );
    args.insert(
        "DefinitionFile".to_string(),
        json!(definition_path.display().to_string()),
    );
    args.insert("dryRun".to_string(), json!(true));

    let result = UnicaApplication::new()
        .call_tool("unica.meta.edit", &args)
        .unwrap();

    assert!(result.ok);

    let _ = fs::remove_dir_all(&context.cwd);
}
