use super::*;
use crate::infrastructure::native_operations::typed_result::NativeInvocationContext;
use crate::infrastructure::native_operations::NativeOperationAdapter;
use roxmltree::Document;
use serde_json::{json, Map};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use unica_forms_designer::root::FORM_LOGFORM_NS;
use unica_forms_designer::test_support::assert_platform_text_uses_crlf_without_bare_lf;
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::compile_transaction::{with_commit_failpoint, CommitFailpoint};
use unica_workspace::single_file_publisher::{
    with_before_commit_hook, with_publish_failpoints, PublishCheckpoint,
};

fn temp_context(name: &str) -> WorkspaceContext {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("unica-form-{name}-{nanos}"));
    fs::create_dir_all(&root).unwrap();
    WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".build").join("unica"),
        workspace_epoch: 1,
    }
}

fn form_info_workspace(name: &str) -> (WorkspaceContext, PathBuf) {
    let context = temp_context(name);
    let source = context.workspace_root.join("src");
    let form_path = source.join("Catalogs/Items/Forms/Order/Ext/Form.xml");
    fs::create_dir_all(form_path.parent().unwrap()).unwrap();
    fs::write(
        context.workspace_root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Demo</Name></Properties><ChildObjects><Catalog>Items</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
    fs::write(
            source.join("Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Items</Name></Properties><ChildObjects><Form>Order</Form></ChildObjects></Catalog></MetaDataObject>"#,
        )
        .unwrap();
    fs::write(
            source.join("Catalogs/Items/Forms/Order.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"><Properties><Name>Order</Name></Properties></Form></MetaDataObject>"#,
        )
        .unwrap();
    (context, form_path)
}

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

#[test]
fn form_validate_rejects_wrong_root_namespace() {
    let context = temp_context("validate-wrong-root-ns");
    let form_path = context.cwd.join("Form.xml");
    write_file(&form_path, &editable_contract_form("urn:not-logform", ""));

    let outcome = validate_form(&form_path_args(&form_path), &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(outcome
        .errors
        .iter()
        .any(|error| { error.contains("urn:not-logform") && error.contains(FORM_LOGFORM_NS) }));
    let _ = fs::remove_dir_all(&context.cwd);
}

/// 8.3.27 loads `Representation` on a form command only from
/// `DefaultRepresentation`; `PictureAndText` breaks the load with an XDTO
/// property mismatch (#1436), while `TextPicture` is the platform's own.
#[test]
fn form_validate_rejects_button_representation_on_command() {
    let form = |representation: &str| {
        format!(
                "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<Form xmlns=\"{FORM_LOGFORM_NS}\" version=\"2.20\">\n\t<AutoCommandBar name=\"FormCommandBar\" id=\"-1\"/>\n\t<ChildItems/>\n\t<Attributes/>\n\t<Commands>\n\t\t<Command name=\"Find\" id=\"1\">\n\t\t\t<Action>Find</Action>\n\t\t\t<Representation>{representation}</Representation>\n\t\t</Command>\n\t</Commands>\n</Form>\n"
            )
    };
    let context = temp_context("validate-command-representation");
    let form_path = context.cwd.join("Form.xml");

    // An empty or blank element is outside the set too, not a silent Auto.
    for invalid in ["PictureAndText", "None", "Compact", "", " "] {
        write_file(&form_path, &form(invalid));
        let outcome = validate_form(&form_path_args(&form_path), &context);
        assert!(!outcome.ok, "{invalid}: {outcome:?}");
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("Command 'Find'")
                    && error.contains(invalid)
                    && error.contains("TextPicture")),
            "{invalid}: {:?}",
            outcome.errors
        );
    }
    for valid in ["Auto", "Text", "Picture", "TextPicture"] {
        write_file(&form_path, &form(valid));
        let outcome = validate_form(&form_path_args(&form_path), &context);
        assert!(outcome.ok, "{valid}: {outcome:?}");
    }
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_info_rejects_wrong_root_namespace() {
    let (context, form_path) = form_info_workspace("info-wrong-root-ns");
    write_file(&form_path, &editable_contract_form("urn:not-logform", ""));

    let outcome = analyze_form_info(
        &form_path_args(&form_path),
        &context,
        &WorkspaceSupportStateReader::new(&context),
    );

    assert!(!outcome.ok, "{outcome:?}");
    assert!(outcome.artifacts.is_empty());
    assert!(outcome
        .errors
        .iter()
        .any(|error| { error.contains("urn:not-logform") && error.contains(FORM_LOGFORM_NS) }));
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_edit_rejects_wrong_root_namespace_without_write() {
    let context = temp_context("edit-wrong-root-ns");
    let form_path = context.cwd.join("Form.xml");
    let original = editable_contract_form("urn:not-logform", "").into_bytes();
    fs::write(&form_path, &original).unwrap();
    let mut args = form_path_args(&form_path);
    args.insert(
        "definition".to_string(),
        json!({"attributes": [{"name": "Added", "type": "string"}]}),
    );

    let outcome = edit_form(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(outcome
        .errors
        .iter()
        .any(|error| { error.contains("urn:not-logform") && error.contains(FORM_LOGFORM_NS) }));
    assert_eq!(fs::read(&form_path).unwrap(), original);
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_edit_declares_dcs_qname_namespaces() {
    let context = temp_context("edit-dcs-qnames");
    let form_path = context.cwd.join("Form.xml");
    write_file(
        &form_path,
        &editable_contract_form(FORM_LOGFORM_NS, &format!(" xmlns:v8=\"{FORM_V8_NS}\"")),
    );
    let mut args = form_path_args(&form_path);
    args.insert(
        "definition".to_string(),
        json!({
            "attributes": [{
                "name": "DcsTypes",
                "type": "dcssch:DataCompositionSchema|dcsset:Filter|dcscor:Field"
            }]
        }),
    );

    let outcome = edit_form(&args, &context);

    assert!(outcome.ok, "{outcome:?}");
    let updated = fs::read_to_string(&form_path).unwrap();
    let document = Document::parse(&updated).unwrap();
    let root = document.root_element();
    for (prefix, expected) in [
        (
            "dcssch",
            "http://v8.1c.ru/8.1/data-composition-system/schema",
        ),
        (
            "dcsset",
            "http://v8.1c.ru/8.1/data-composition-system/settings",
        ),
        ("dcscor", "http://v8.1c.ru/8.1/data-composition-system/core"),
    ] {
        assert_eq!(root.lookup_namespace_uri(Some(prefix)), Some(expected));
    }
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_edit_declares_v8ui_and_ent_qname_namespaces() {
    let context = temp_context("edit-v8ui-ent-qnames");
    let form_path = context.cwd.join("Form.xml");
    write_file(
        &form_path,
        &editable_contract_form(FORM_LOGFORM_NS, &format!(" xmlns:v8=\"{FORM_V8_NS}\"")),
    );
    let mut args = form_path_args(&form_path);
    args.insert(
        "definition".to_string(),
        json!({
            "attributes": [{
                "name": "PlatformTypes",
                "type": "v8ui:Color|ent:AccountType"
            }]
        }),
    );

    let outcome = edit_form(&args, &context);

    assert!(outcome.ok, "{outcome:?}");
    let updated = fs::read_to_string(&form_path).unwrap();
    let document = Document::parse(&updated).unwrap();
    let root = document.root_element();
    assert_eq!(
        root.lookup_namespace_uri(Some("v8ui")),
        Some("http://v8.1c.ru/8.1/data/ui")
    );
    assert_eq!(
        root.lookup_namespace_uri(Some("ent")),
        Some("http://v8.1c.ru/8.1/data/enterprise")
    );
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_edit_rejects_emitted_prefix_bound_to_wrong_namespace_without_write() {
    let context = temp_context("edit-wrong-emitted-prefix");
    let form_path = context.cwd.join("Form.xml");
    let original = editable_contract_form(
        FORM_LOGFORM_NS,
        &format!(" xmlns:v8=\"{FORM_V8_NS}\" xmlns:v8ui=\"urn:wrong-v8ui\""),
    )
    .into_bytes();
    fs::write(&form_path, &original).unwrap();
    let mut args = form_path_args(&form_path);
    args.insert(
        "definition".to_string(),
        json!({"attributes": [{"name": "Color", "type": "v8ui:Color"}]}),
    );

    let outcome = edit_form(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(outcome.errors.iter().any(|error| {
        error.contains("v8ui")
            && error.contains("urn:wrong-v8ui")
            && error.contains("http://v8.1c.ru/8.1/data/ui")
    }));
    assert_eq!(fs::read(&form_path).unwrap(), original);
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_edit_rejects_escaped_and_unbound_emitted_qnames_without_write() {
    let cases = [
        ("ampersand", "foo:A&B", "", "fixed 8.3.27"),
        ("less-than", "foo:A<B", "", "fixed 8.3.27"),
        ("undeclared", "foo:Type", "", "fixed 8.3.27"),
        (
            "conflicting-known-prefix",
            "v8ui:Color",
            " xmlns:v8ui=\"urn:wrong-v8ui\"",
            "expected 'http://v8.1c.ru/8.1/data/ui'",
        ),
    ];

    for (name, type_name, extra_namespaces, expected_error) in cases {
        let context = temp_context(&format!("edit-emitted-qname-{name}"));
        let form_path = context.cwd.join("Form.xml");
        let original = editable_contract_form(
            FORM_LOGFORM_NS,
            &format!(" xmlns:v8=\"{FORM_V8_NS}\"{extra_namespaces}"),
        )
        .into_bytes();
        fs::write(&form_path, &original).unwrap();
        let mut args = form_path_args(&form_path);
        args.insert(
            "definition".to_string(),
            json!({"attributes": [{"name": "UnsafeType", "type": type_name}]}),
        );

        let outcome = edit_form(&args, &context);

        assert!(!outcome.ok, "{name}: {outcome:?}");
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains(expected_error)),
            "{name}: {outcome:?}"
        );
        assert_eq!(fs::read(&form_path).unwrap(), original, "{name}");
        let _ = fs::remove_dir_all(&context.cwd);
    }
}

#[test]
fn form_validate_rejects_undeclared_qname_prefix() {
    let context = temp_context("validate-undeclared-type-prefix");
    let form_path = context.cwd.join("Form.xml");
    write_file(
        &form_path,
        &form_with_declared_type("", "missing:CatalogRef.Goods"),
    );

    let outcome = validate_form(&form_path_args(&form_path), &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(outcome.errors.iter().any(|error| {
        error.contains("missing:CatalogRef.Goods") && error.contains("undeclared prefix 'missing'")
    }));
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_edit_rejects_existing_undeclared_qname_without_byte_changes() {
    let context = temp_context("edit-existing-undeclared-type-prefix");
    let form_path = context.cwd.join("Form.xml");
    let original = form_with_declared_type("", "missing:CatalogRef.Goods").into_bytes();
    fs::write(&form_path, &original).unwrap();
    let mut args = form_path_args(&form_path);
    args.insert(
        "definition".to_string(),
        json!({"attributes": [{"name": "Added", "type": "string"}]}),
    );

    let outcome = edit_form(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(outcome.errors.iter().any(|error| {
        error.contains("missing:CatalogRef.Goods") && error.contains("undeclared prefix 'missing'")
    }));
    assert!(outcome.changes.is_empty(), "{outcome:?}");
    assert_eq!(fs::read(&form_path).unwrap(), original);
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_validate_rejects_known_qname_prefix_bound_to_wrong_namespace() {
    let context = temp_context("validate-wrong-type-prefix-binding");
    let form_path = context.cwd.join("Form.xml");
    write_file(
        &form_path,
        &form_with_declared_type(" xmlns:cfg=\"urn:wrong-config\"", "cfg:CatalogRef.Goods"),
    );

    let outcome = validate_form(&form_path_args(&form_path), &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(outcome.errors.iter().any(|error| {
        error.contains("cfg:CatalogRef.Goods")
            && error.contains("urn:wrong-config")
            && error.contains("http://v8.1c.ru/8.1/data/enterprise/current-config")
    }));
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_validate_accepts_declared_config_qname_alias() {
    let context = temp_context("validate-config-type-alias");
    let form_path = context.cwd.join("Form.xml");
    write_file(
        &form_path,
        &form_with_declared_type(
            " xmlns:custom=\"http://v8.1c.ru/8.1/data/enterprise/current-config\"",
            "custom:CatalogRef.Goods",
        ),
    );

    let outcome = validate_form(&form_path_args(&form_path), &context);

    assert!(outcome.ok, "{outcome:?}");
    let _ = fs::remove_dir_all(&context.cwd);
}

fn form_path_args(path: &Path) -> Map<String, Value> {
    Map::from_iter([("FormPath".to_string(), json!(path.display().to_string()))])
}

fn editable_contract_form(namespace: &str, extra_namespaces: &str) -> String {
    format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<Form xmlns=\"{namespace}\"{extra_namespaces} version=\"2.20\">\n\t<AutoCommandBar name=\"FormCommandBar\" id=\"-1\"/>\n\t<ChildItems/>\n\t<Attributes/>\n\t<Commands/>\n</Form>\n"
        )
}

fn form_with_declared_type(extra_namespaces: &str, type_name: &str) -> String {
    format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<Form xmlns=\"{FORM_LOGFORM_NS}\" xmlns:v8=\"{FORM_V8_NS}\"{extra_namespaces} version=\"2.20\">\n\t<AutoCommandBar name=\"FormCommandBar\" id=\"-1\"/>\n\t<ChildItems/>\n\t<Attributes>\n\t\t<Attribute name=\"Value\" id=\"1\">\n\t\t\t<Type><v8:Type>{type_name}</v8:Type></Type>\n\t\t</Attribute>\n\t</Attributes>\n\t<Commands/>\n</Form>\n"
        )
}

fn empty_catalog_xml(line_ending: &str, trailing_newline: bool) -> String {
    let mut text = [
            r#"<?xml version="1.0" encoding="utf-8"?>"#,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" version="2.20">"#,
            r#"	<Catalog uuid="00000000-0000-0000-0000-000000000001">"#,
            r#"		<InternalInfo>"#,
            r#"			<xr:GeneratedType name="CatalogObject.Goods" category="Object">"#,
            r#"				<xr:TypeId>00000000-0000-4000-8000-000000000101</xr:TypeId>"#,
            r#"				<xr:ValueId>00000000-0000-4000-8000-000000000102</xr:ValueId>"#,
            r#"			</xr:GeneratedType>"#,
            r#"			<xr:GeneratedType name="CatalogRef.Goods" category="Ref">"#,
            r#"				<xr:TypeId>00000000-0000-4000-8000-000000000103</xr:TypeId>"#,
            r#"				<xr:ValueId>00000000-0000-4000-8000-000000000104</xr:ValueId>"#,
            r#"			</xr:GeneratedType>"#,
            r#"			<xr:GeneratedType name="CatalogSelection.Goods" category="Selection">"#,
            r#"				<xr:TypeId>00000000-0000-4000-8000-000000000105</xr:TypeId>"#,
            r#"				<xr:ValueId>00000000-0000-4000-8000-000000000106</xr:ValueId>"#,
            r#"			</xr:GeneratedType>"#,
            r#"			<xr:GeneratedType name="CatalogList.Goods" category="List">"#,
            r#"				<xr:TypeId>00000000-0000-4000-8000-000000000107</xr:TypeId>"#,
            r#"				<xr:ValueId>00000000-0000-4000-8000-000000000108</xr:ValueId>"#,
            r#"			</xr:GeneratedType>"#,
            r#"			<xr:GeneratedType name="CatalogManager.Goods" category="Manager">"#,
            r#"				<xr:TypeId>00000000-0000-4000-8000-000000000109</xr:TypeId>"#,
            r#"				<xr:ValueId>00000000-0000-4000-8000-000000000110</xr:ValueId>"#,
            r#"			</xr:GeneratedType>"#,
            r#"		</InternalInfo>"#,
            r#"		<Properties>"#,
            r#"			<Name>Goods</Name>"#,
            r#"			<Synonym>Goods</Synonym>"#,
            r#"			<DefaultListForm/>"#,
            r#"		</Properties>"#,
            r#"		<ChildObjects/>"#,
            r#"	</Catalog>"#,
            r#"</MetaDataObject>"#,
        ]
        .join(line_ending);
    if trailing_newline {
        text.push_str(line_ending);
    }
    text
}

fn add_list_form_args(object_path: &Path, form_name: &str) -> Map<String, serde_json::Value> {
    let mut args = Map::new();
    args.insert(
        "ObjectPath".to_string(),
        json!(object_path.display().to_string()),
    );
    args.insert("FormName".to_string(), json!(form_name));
    args.insert("Purpose".to_string(), json!("List"));
    args.insert("Synonym".to_string(), json!("List form"));
    args
}

fn add_object_form_args(object_path: &Path, form_name: &str) -> Map<String, Value> {
    Map::from_iter([
        (
            "ObjectPath".to_string(),
            json!(object_path.display().to_string()),
        ),
        ("FormName".to_string(), json!(form_name)),
        ("Purpose".to_string(), json!("Object")),
        ("Synonym".to_string(), json!("Added form")),
    ])
}

fn create_external_owner(
    context: &WorkspaceContext,
    root_type: &str,
    object_name: &str,
) -> PathBuf {
    let (operation, tool_name) = match root_type {
        "ExternalDataProcessor" => ("epf-init", "unica.epf.init"),
        "ExternalReport" => ("erf-init", "unica.erf.init"),
        other => panic!("unsupported external fixture root: {other}"),
    };
    let args = Map::from_iter([
        ("Name".to_string(), json!(object_name)),
        ("OutputDir".to_string(), json!("external")),
    ]);
    let outcome = crate::infrastructure::native_operations::external::apply(
        operation, tool_name, &args, context,
    )
    .expect("external init operation must be registered");
    assert!(outcome.ok, "{outcome:?}");
    context
        .cwd
        .join("external")
        .join(format!("{object_name}.xml"))
}

fn external_attribute_xml(password_mode: &str, indexing: &str) -> String {
    format!(
        r#"
			<Attribute uuid="00000000-0000-4000-8000-000000000201">
				<Properties>
					<Name>ExistingAttribute</Name>
					<Type>
						<v8:Type>xs:string</v8:Type>
					</Type>
					<PasswordMode>{password_mode}</PasswordMode>
					<Indexing>{indexing}</Indexing>
				</Properties>
			</Attribute>
		"#
    )
}

#[test]
fn add_form_accepts_valid_external_processor_and_report_owners() {
    for (case, root_type, existing_child) in [
        (
            "processor-template",
            "ExternalDataProcessor",
            "\n\t\t\t<Template>ExistingTemplate</Template>\n\t\t".to_string(),
        ),
        (
            "report-attribute",
            "ExternalReport",
            external_attribute_xml("false", "DontIndex"),
        ),
    ] {
        let context = temp_context(&format!("add-valid-external-{case}"));
        let object_name = if root_type == "ExternalReport" {
            "ExternalSalesReport"
        } else {
            "ExternalImportProcessor"
        };
        let owner = create_external_owner(&context, root_type, object_name);
        let source = fs::read_to_string(&owner).unwrap();
        let source = source.replace(
            "\t\t<ChildObjects/>",
            &format!("\t\t<ChildObjects>{existing_child}</ChildObjects>"),
        );
        fs::write(&owner, source).unwrap();
        let args = add_object_form_args(&owner, "AddedForm");

        let outcome = add_form(&args, &context);

        assert!(outcome.ok, "{case}: {outcome:?}");
        let updated = fs::read_to_string(&owner).unwrap();
        assert!(updated.contains("<Form>AddedForm</Form>"), "{updated}");
        assert!(
            updated.contains(if root_type == "ExternalReport" {
                "<Attribute uuid=\"00000000-0000-4000-8000-000000000201\">"
            } else {
                "<Template>ExistingTemplate</Template>"
            }),
            "{updated}"
        );
        let forms = context.cwd.join("external").join(object_name).join("Forms");
        assert!(forms.join("AddedForm.xml").is_file());
        assert!(forms.join("AddedForm/Ext/Form.xml").is_file());
        assert!(forms.join("AddedForm/Ext/Form/Module.bsl").is_file());
        let _ = fs::remove_dir_all(&context.cwd);
    }
}

/// #340. The context of a form is decided by the object that owns it, not
/// by whether some `Configuration.xml` happens to sit above it in the
/// tree. An external data processor stored under a workspace that also
/// holds a configuration was read as configuration context, so the main
/// attribute `epf.init` had just written was rejected as invalid there.
#[test]
fn form_validation_reads_context_from_the_owning_object_not_the_tree_above() {
    let context = temp_context("external-form-under-configuration");
    // A configuration lives in the same workspace, above the external
    // object — the layout the report describes.
    fs::create_dir_all(&context.cwd).unwrap();
    fs::write(
        context.cwd.join("Configuration.xml"),
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">\n\
             \t<Configuration uuid=\"55555555-5555-5555-5555-555555555555\">\n\
             \t\t<Properties>\n\t\t\t<Name>Sample</Name>\n\t\t\t<NamePrefix/>\n\t\t</Properties>\n\
             \t\t<ChildObjects/>\n\t</Configuration>\n</MetaDataObject>\n",
    )
    .unwrap();
    let owner = create_external_owner(&context, "ExternalDataProcessor", "ContextProcessor");
    assert!(owner.is_file());
    let added = add_form(&add_object_form_args(&owner, "MainForm"), &context);
    assert!(added.ok, "{added:?}");
    let form_path = context
        .cwd
        .join("external")
        .join("ContextProcessor")
        .join("Forms")
        .join("MainForm/Ext/Form.xml");

    assert!(
        !form_is_config_context(&ProjectDiscoveryReader::new(&context.cwd), &form_path),
        "a form owned by an external object is never configuration context"
    );

    let outcome = validate_form(
        &Map::from_iter([(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        )]),
        &context,
    );

    assert!(
        !outcome
            .errors
            .iter()
            .chain(outcome.warnings.iter())
            .any(|line| line.contains("External* type in configuration context")),
        "{outcome:?}"
    );
    let _ = fs::remove_dir_all(&context.cwd);
}

/// #389. `form-dsl-spec` documents `label` as `LabelDecoration`, the
/// pattern reference recommends it for forms built with
/// `unica.form.compile`, and `form-edit` says it uses the same DSL keys.
/// The native compiler had no such discriminator and answered
/// `Unsupported form element in native compiler`.
#[test]
fn form_compile_accepts_the_documented_label_decoration() {
    let context = temp_context("compile-label-decoration");
    let form_path = context.cwd.join("Form.xml");
    fs::create_dir_all(&context.cwd).unwrap();
    fs::write(&form_path, form_edit_remove_test_xml("")).unwrap();

    let outcome = edit_form(
        &Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            (
                "definition".to_string(),
                json!({
                    "elements": [{
                        "label": "ИнформационнаяНадпись",
                        "title": "Выберите параметры",
                        "hyperlink": true,
                        "width": 40
                    }]
                }),
            ),
        ]),
        &context,
    );

    assert!(outcome.ok, "{outcome:?}");
    let xml = fs::read_to_string(&form_path).unwrap();
    assert!(
        xml.contains("<LabelDecoration name=\"ИнформационнаяНадпись\""),
        "{xml}"
    );
    assert!(xml.contains("<Hyperlink>true</Hyperlink>"), "{xml}");
    assert!(xml.contains("<Width>40</Width>"), "{xml}");
    assert!(xml.contains("Выберите параметры"), "{xml}");
    let _ = fs::remove_dir_all(&context.cwd);
}

/// Review of #450: `LabelDecoration` allows only `Click` and
/// `URLProcessing`. Classifying `label` as a `LabelField` let `OnChange`
/// through preflight and would have emitted an event the platform does
/// not accept on a decoration.
#[test]
fn form_compile_refuses_a_label_event_the_decoration_does_not_allow() {
    let context = temp_context("label-event-registry");
    let form_path = context.cwd.join("Form.xml");
    fs::create_dir_all(&context.cwd).unwrap();
    fs::write(&form_path, form_edit_remove_test_xml("")).unwrap();

    let outcome = edit_form(
        &Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            (
                "definition".to_string(),
                json!({
                    "elements": [{
                        "label": "Подсказка",
                        "on": ["OnChange"]
                    }]
                }),
            ),
        ]),
        &context,
    );

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| error.contains("OnChange")),
        "{outcome:?}"
    );
    let _ = fs::remove_dir_all(&context.cwd);
}

/// #341. One writer must not produce a form the other refuses. `form.add`
/// wrote a `Form.xml` without a root `ChildItems`, so the very next
/// `form.edit` failed with `No <ChildItems> section found in form`.
#[test]
fn add_form_produces_a_form_the_editor_accepts() {
    let context = temp_context("add-form-round-trip");
    let owner = create_external_owner(&context, "ExternalDataProcessor", "RoundTripProcessor");
    let added = add_form(&add_object_form_args(&owner, "MainForm"), &context);
    assert!(added.ok, "{added:?}");
    let form_path = context
        .cwd
        .join("external")
        .join("RoundTripProcessor")
        .join("Forms")
        .join("MainForm/Ext/Form.xml");

    let edited = edit_form(
        &Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            (
                "definition".to_string(),
                json!({ "elements": [{ "input": "Комментарий" }] }),
            ),
        ]),
        &context,
    );

    assert!(edited.ok, "{edited:?}");
    let xml = fs::read_to_string(&form_path).unwrap();
    assert!(xml.contains("<ChildItems>"), "{xml}");
    assert!(xml.contains("Комментарий"), "{xml}");
    let _ = fs::remove_dir_all(&context.cwd);
}

/// Review of #446: without `Attributes`, a root that has both `Parameters`
/// and `Commands` must still receive `ChildItems` before `Parameters`.
#[test]
fn form_edit_creates_child_items_before_parameters_when_attributes_are_absent() {
    let context = temp_context("child-items-anchor-order");
    let form_path = context.cwd.join("Form.xml");
    fs::create_dir_all(&context.cwd).unwrap();
    fs::write(
        &form_path,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\n\
             \t<AutoCommandBar name=\"ФормаКоманднаяПанель\" id=\"-1\"/>\n\
             \t<Parameters/>\n\t<Commands/>\n</Form>\n",
    )
    .unwrap();

    let edited = edit_form(
        &Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            (
                "definition".to_string(),
                json!({ "elements": [{ "input": "Комментарий" }] }),
            ),
        ]),
        &context,
    );

    assert!(edited.ok, "{edited:?}");
    let xml = fs::read_to_string(&form_path).unwrap();
    let items = xml.find("<ChildItems>").expect("the section was created");
    let parameters = xml.find("<Parameters").expect("Parameters stays");
    let commands = xml.find("<Commands").expect("Commands stays");
    assert!(
        items < parameters && parameters < commands,
        "8.3.27 orders ChildItems -> Parameters -> Commands: {xml}"
    );
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn add_form_rejects_invalid_external_owner_boolean_and_enum_without_writes() {
    for (case, root_type, password_mode, indexing, expected_property) in [
        (
            "processor-boolean",
            "ExternalDataProcessor",
            "truthy",
            "DontIndex",
            "PasswordMode",
        ),
        (
            "report-enum",
            "ExternalReport",
            "false",
            "truthy",
            "Indexing",
        ),
    ] {
        let context = temp_context(&format!("add-invalid-external-{case}"));
        let owner = create_external_owner(&context, root_type, "InvalidExternalOwner");
        let source = fs::read_to_string(&owner).unwrap();
        let source = source.replace(
            "\t\t<ChildObjects/>",
            &format!(
                "\t\t<ChildObjects>{}</ChildObjects>",
                external_attribute_xml(password_mode, indexing)
            ),
        );
        fs::write(&owner, source).unwrap();
        let owner_before = fs::read(&owner).unwrap();
        let forms = context.cwd.join("external/InvalidExternalOwner/Forms");
        let args = add_object_form_args(&owner, "RejectedForm");

        let outcome = add_form(&args, &context);

        assert!(!outcome.ok, "{case}: {outcome:?}");
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains(expected_property)
                    && error.contains("fixed 8.3.27 contract")),
            "{case}: {outcome:?}"
        );
        assert_eq!(fs::read(&owner).unwrap(), owner_before, "{case}");
        assert!(!forms.exists(), "{case}: {}", forms.display());
        let _ = fs::remove_dir_all(&context.cwd);
    }
}

#[test]
fn add_form_post_write_failure_restores_owner_and_removes_scaffold() {
    let context = temp_context("add-post-write-failure");
    let root_xml = context.cwd.join("src/Catalogs/Goods.xml");
    write_file(&root_xml, &empty_catalog_xml("\r\n", false));
    let descriptor = context.cwd.join("src/Catalogs/Goods/Forms/ListForm.xml");
    let form_xml = context
        .cwd
        .join("src/Catalogs/Goods/Forms/ListForm/Ext/Form.xml");
    let module = context
        .cwd
        .join("src/Catalogs/Goods/Forms/ListForm/Ext/Form/Module.bsl");
    let before = [
        (root_xml.clone(), fs::read(&root_xml).ok()),
        (descriptor.clone(), fs::read(&descriptor).ok()),
        (form_xml.clone(), fs::read(&form_xml).ok()),
        (module.clone(), fs::read(&module).ok()),
    ];
    let args = add_list_form_args(&root_xml, "ListForm");

    let outcome = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
        add_form(&args, &context)
    });

    assert!(!outcome.ok, "{outcome:?}");
    for (path, expected) in before {
        assert_eq!(fs::read(&path).ok(), expected, "{}", path.display());
    }
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn add_form_rejects_platform_invalid_owner_boolean_before_creating_scaffold() {
    let context = temp_context("add-invalid-owner-boolean");
    let root_xml = context.cwd.join("src/Catalogs/Goods.xml");
    let invalid_owner = empty_catalog_xml("\n", true).replace(
        "\t\t\t<DefaultListForm/>",
        "\t\t\t<DefaultListForm/>\n\t\t\t<IncludeHelpInContents>truthy</IncludeHelpInContents>",
    );
    write_file(&root_xml, &invalid_owner);
    let owner_before = fs::read(&root_xml).unwrap();
    let forms_dir = context.cwd.join("src/Catalogs/Goods/Forms");
    let args = add_list_form_args(&root_xml, "ListForm");

    let outcome = add_form(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("IncludeHelpInContents")
                    && error.contains("true or false")),
            "{outcome:?}"
        );
    assert_eq!(fs::read(&root_xml).unwrap(), owner_before);
    assert!(!forms_dir.exists());
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn add_form_rejects_path_form_name_without_mutating_owner_or_escape_target() {
    let context = temp_context("add-path-form-name");
    let root_xml = context.cwd.join("src/Catalogs/Goods.xml");
    write_file(&root_xml, &empty_catalog_xml("\n", true));
    let root_before = fs::read(&root_xml).unwrap();
    let escaped_descriptor = context.cwd.join("src/Catalogs/Goods/Forms/../Escaped.xml");
    let args = add_list_form_args(&root_xml, "../Escaped");

    let outcome = add_form(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(outcome.errors.iter().any(|error| {
        error.contains("FormName")
            && error.contains("XML NCName")
            && error.contains("single path component")
    }));
    assert_eq!(fs::read(&root_xml).unwrap(), root_before);
    assert!(!escaped_descriptor.exists());
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
pub(crate) fn add_form_rejects_partial_existing_scaffold_before_any_mutation() {
    let context = temp_context("add-partial-scaffold");
    let root_xml = context.cwd.join("src/Catalogs/Goods.xml");
    let descriptor = context.cwd.join("src/Catalogs/Goods/Forms/ListForm.xml");
    let form_xml = context
        .cwd
        .join("src/Catalogs/Goods/Forms/ListForm/Ext/Form.xml");
    let module = context
        .cwd
        .join("src/Catalogs/Goods/Forms/ListForm/Ext/Form/Module.bsl");
    write_file(&root_xml, &empty_catalog_xml("\r\n", false));
    write_file(
        &form_xml,
        "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.17\"><broken></Form>\n",
    );
    write_file(&module, "// pre-existing module\n");
    let before = [
        (root_xml.clone(), fs::read(&root_xml).ok()),
        (descriptor.clone(), fs::read(&descriptor).ok()),
        (form_xml.clone(), fs::read(&form_xml).ok()),
        (module.clone(), fs::read(&module).ok()),
    ];
    let args = add_list_form_args(&root_xml, "ListForm");

    let outcome = add_form(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(outcome
        .errors
        .iter()
        .any(|error| { error.contains("create-only") && error.contains("Form.xml") }));
    for (path, expected) in before {
        assert_eq!(fs::read(&path).ok(), expected, "{}", path.display());
    }
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
pub(crate) fn add_form_rejects_scaffold_member_created_after_planning() {
    let context = temp_context("add-late-scaffold-member");
    let root_xml = context.cwd.join("src/Catalogs/Goods.xml");
    let descriptor = context.cwd.join("src/Catalogs/Goods/Forms/ListForm.xml");
    let form_xml = context
        .cwd
        .join("src/Catalogs/Goods/Forms/ListForm/Ext/Form.xml");
    let module = context
        .cwd
        .join("src/Catalogs/Goods/Forms/ListForm/Ext/Form/Module.bsl");
    write_file(&root_xml, &empty_catalog_xml("\n", true));
    let owner_before = fs::read(&root_xml).unwrap();
    let concurrent = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"/></MetaDataObject>"#.to_vec();
    let descriptor_for_hook = descriptor.clone();
    let concurrent_for_hook = concurrent.clone();
    let args = add_list_form_args(&root_xml, "ListForm");

    let outcome = with_before_commit_hook(
        move |_| fs::write(&descriptor_for_hook, concurrent_for_hook).unwrap(),
        || add_form(&args, &context),
    );

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome.errors.join("\n").contains("created concurrently")
            || outcome.errors.join("\n").contains("already exists")
            || outcome.errors.join("\n").contains("preimage"),
        "{outcome:?}"
    );
    assert_eq!(fs::read(&root_xml).unwrap(), owner_before);
    assert_eq!(fs::read(&descriptor).unwrap(), concurrent);
    assert!(!form_xml.exists(), "{outcome:?}");
    assert!(!module.exists(), "{outcome:?}");
    assert!(outcome.changes.is_empty(), "{outcome:?}");
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn add_form_set_default_false_leaves_empty_default_slot() {
    let context = temp_context("add-set-default-false");
    let root_xml = context.cwd.join("src").join("Catalogs").join("Goods.xml");
    write_file(&root_xml, &empty_catalog_xml("\n", true));

    let mut args = add_list_form_args(&root_xml, "ListForm");
    args.insert("SetDefault".to_string(), json!(false));

    let outcome = add_form(&args, &context);

    assert!(outcome.ok, "{:?}", outcome.errors);
    for generated_path in [
        context.cwd.join("src/Catalogs/Goods/Forms/ListForm.xml"),
        context
            .cwd
            .join("src/Catalogs/Goods/Forms/ListForm/Ext/Form.xml"),
    ] {
        let generated = fs::read_to_string(generated_path).unwrap();
        assert!(generated.contains(r#"version="2.20""#), "{generated}");
        assert!(!generated.contains(r#"version="2.17""#), "{generated}");
    }
    let updated = fs::read_to_string(&root_xml).unwrap();
    assert!(updated.contains("<DefaultListForm/>"), "{updated}");
    assert!(
        !updated.contains("<DefaultListForm>Catalog.Goods.Form.ListForm</DefaultListForm>"),
        "{updated}"
    );
    assert!(updated.contains("<Form>ListForm</Form>"), "{updated}");
    assert!(
        !outcome
            .stdout
            .as_deref()
            .unwrap_or("")
            .contains("DefaultListForm: Catalog.Goods.Form.ListForm"),
        "{:?}",
        outcome.stdout
    );

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn add_form_sets_default_when_explicit_true_or_omitted_for_empty_slot() {
    for (case, set_default_arg) in [("explicit-true", Some(true)), ("omitted", None)] {
        let context = temp_context(case);
        let root_xml = context.cwd.join("src").join("Catalogs").join("Goods.xml");
        write_file(&root_xml, &empty_catalog_xml("\n", true));

        let mut args = add_list_form_args(&root_xml, "ListForm");
        if let Some(value) = set_default_arg {
            args.insert("SetDefault".to_string(), json!(value));
        }

        let outcome = add_form(&args, &context);

        assert!(outcome.ok, "{case}: {:?}", outcome.errors);
        let updated = fs::read_to_string(&root_xml).unwrap();
        assert!(
            updated.contains("<DefaultListForm>Catalog.Goods.Form.ListForm</DefaultListForm>"),
            "{case}: {updated}"
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }
}

#[test]
fn add_form_set_default_true_overwrites_existing_default_slot() {
    let context = temp_context("add-set-default-overwrite");
    let root_xml = context.cwd.join("src").join("Catalogs").join("Goods.xml");
    let source = empty_catalog_xml("\n", true).replace(
        "<DefaultListForm/>",
        "<DefaultListForm>Catalog.Goods.Form.OldListForm</DefaultListForm>",
    );
    write_file(&root_xml, &source);

    let mut args = add_list_form_args(&root_xml, "ListForm");
    args.insert("SetDefault".to_string(), json!(true));

    let outcome = add_form(&args, &context);

    assert!(outcome.ok, "{:?}", outcome.errors);
    let updated = fs::read_to_string(&root_xml).unwrap();
    assert!(
        updated.contains("<DefaultListForm>Catalog.Goods.Form.ListForm</DefaultListForm>"),
        "{updated}"
    );
    assert!(
        !updated.contains("Catalog.Goods.Form.OldListForm"),
        "{updated}"
    );

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn add_then_remove_form_round_trips_empty_catalog_parent_xml() {
    let context = temp_context("add-remove-roundtrip");
    let root_xml = context.cwd.join("Catalogs").join("Goods.xml");
    let original = empty_catalog_xml("\r\n", false);
    write_file(&root_xml, &original);

    let mut add_args = add_list_form_args(&root_xml, "ListForm");
    add_args.insert("SetDefault".to_string(), json!(false));
    let add_outcome = add_form(&add_args, &context);
    assert!(add_outcome.ok, "{:?}", add_outcome.errors);

    let mut remove_args = Map::new();
    remove_args.insert("ObjectName".to_string(), json!("Goods"));
    remove_args.insert("FormName".to_string(), json!("ListForm"));
    remove_args.insert("SrcDir".to_string(), json!("Catalogs"));
    let remove_outcome = remove_form(&remove_args, &context);
    assert!(remove_outcome.ok, "{:?}", remove_outcome.errors);

    let updated = fs::read_to_string(&root_xml)
        .unwrap()
        .trim_start_matches('\u{feff}')
        .to_string();
    assert_eq!(updated, original);
    assert!(
        !context.cwd.join("Catalogs/Goods/Forms").exists(),
        "the platform removes an empty Forms collection directory"
    );

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn remove_form_rejects_path_names_before_deleting_any_target() {
    for (case, object_name, form_name) in [
        ("object", "../Goods", "ListForm"),
        ("form", "Goods", "../ListForm"),
    ] {
        let context = temp_context(&format!("remove-path-{case}"));
        let src_dir = context.cwd.join("src/Catalogs");
        let root_xml = if case == "object" {
            context.cwd.join("src/Goods.xml")
        } else {
            src_dir.join("Goods.xml")
        };
        let object_dir = if case == "object" {
            context.cwd.join("src/Goods")
        } else {
            src_dir.join("Goods")
        };
        let forms_dir = object_dir.join("Forms");
        let form_meta = forms_dir.join(format!("{form_name}.xml"));
        let form_dir = forms_dir.join(form_name);
        write_file(&root_xml, &empty_catalog_xml("\n", true));
        write_file(&form_meta, "<MetaDataObject/>\n");
        write_file(&form_dir.join("Ext/Form.xml"), "<Form/>\n");
        let before = [
            (root_xml.clone(), fs::read(&root_xml).ok()),
            (form_meta.clone(), fs::read(&form_meta).ok()),
            (
                form_dir.join("Ext/Form.xml"),
                fs::read(form_dir.join("Ext/Form.xml")).ok(),
            ),
        ];
        let args = Map::from_iter([
            ("ObjectName".to_string(), json!(object_name)),
            ("FormName".to_string(), json!(form_name)),
            ("SrcDir".to_string(), json!("src/Catalogs")),
        ]);

        let outcome = remove_form(&args, &context);

        assert!(!outcome.ok, "{case}: {outcome:?}");
        assert!(outcome.errors.iter().any(|error| {
            error.contains("XML NCName") && error.contains("single path component")
        }));
        for (path, expected) in before {
            assert_eq!(fs::read(&path).ok(), expected, "{}", path.display());
        }
        let _ = fs::remove_dir_all(&context.cwd);
    }
}

#[test]
fn remove_form_post_write_failure_restores_owner_and_scaffold() {
    let context = temp_context("remove-post-write-failure");
    let root_xml = context.cwd.join("src/Catalogs/Goods.xml");
    let form_meta = context.cwd.join("src/Catalogs/Goods/Forms/ListForm.xml");
    let form_xml = context
        .cwd
        .join("src/Catalogs/Goods/Forms/ListForm/Ext/Form.xml");
    let module = context
        .cwd
        .join("src/Catalogs/Goods/Forms/ListForm/Ext/Form/Module.bsl");
    let owner = empty_catalog_xml("\r\n", false).replace(
        "<ChildObjects/>",
        "<ChildObjects>\r\n\t\t\t<Form>ListForm</Form>\r\n\t\t</ChildObjects>",
    );
    write_file(&root_xml, &owner);
    write_file(&form_meta, "<MetaDataObject version=\"2.20\"/>\n");
    write_file(&form_xml, "<Form version=\"2.20\"/>\n");
    write_file(&module, "// module\n");
    let before = [
        (root_xml.clone(), fs::read(&root_xml).ok()),
        (form_meta.clone(), fs::read(&form_meta).ok()),
        (form_xml.clone(), fs::read(&form_xml).ok()),
        (module.clone(), fs::read(&module).ok()),
    ];
    let args = Map::from_iter([
        ("ObjectName".to_string(), json!("Goods")),
        ("FormName".to_string(), json!("ListForm")),
        ("SrcDir".to_string(), json!("src/Catalogs")),
    ]);

    let outcome = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
        remove_form(&args, &context)
    });

    assert!(!outcome.ok, "{outcome:?}");
    for (path, expected) in before {
        assert_eq!(fs::read(&path).ok(), expected, "{}", path.display());
    }
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
pub(crate) fn remove_form_rejects_payload_directory_that_appears_after_absent_probe() {
    let context = temp_context("remove-late-payload-directory");
    let root_xml = context.cwd.join("src/Catalogs/Goods.xml");
    let forms_dir = context.cwd.join("src/Catalogs/Goods/Forms");
    let form_meta = forms_dir.join("ListForm.xml");
    let sibling_meta = forms_dir.join("OtherForm.xml");
    let late_form_dir = forms_dir.join("ListForm");
    let owner = empty_catalog_xml("\n", true).replace(
        "\t\t<ChildObjects/>",
        "\t\t<ChildObjects>\n\t\t\t<Form>ListForm</Form>\n\t\t</ChildObjects>",
    );
    write_file(&root_xml, &owner);
    write_file(
        &form_meta,
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form/></MetaDataObject>"#,
    );
    write_file(
        &sibling_meta,
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form/></MetaDataObject>"#,
    );
    let owner_before = fs::read(&root_xml).unwrap();
    let descriptor_before = fs::read(&form_meta).unwrap();
    let late_form_for_hook = late_form_dir.join("Ext/Form.xml");
    let args = Map::from_iter([
        ("ObjectName".to_string(), json!("Goods")),
        ("FormName".to_string(), json!("ListForm")),
        ("SrcDir".to_string(), json!("src/Catalogs")),
    ]);

    let outcome = with_before_commit_hook(
        move |_| {
            write_file(
                &late_form_for_hook,
                r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"/>"#,
            );
        },
        || remove_form(&args, &context),
    );

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome.errors.join("\n").contains("pair member"),
        "{outcome:?}"
    );
    assert_eq!(fs::read(&root_xml).unwrap(), owner_before);
    assert_eq!(fs::read(&form_meta).unwrap(), descriptor_before);
    assert!(late_form_dir.join("Ext/Form.xml").is_file());
    assert!(sibling_meta.is_file());
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn remove_form_prefers_newer_child_over_older_descriptor_without_deleting_either() {
    let context = temp_context("remove-mixed-format-tree");
    let root_xml = context.cwd.join("src/Catalogs/Goods.xml");
    let form_meta = context.cwd.join("src/Catalogs/Goods/Forms/ListForm.xml");
    let form_xml = context
        .cwd
        .join("src/Catalogs/Goods/Forms/ListForm/Ext/Form.xml");
    let owner = empty_catalog_xml("\n", true).replace(
        "\t\t<ChildObjects/>",
        "\t\t<ChildObjects>\n\t\t\t<Form>ListForm</Form>\n\t\t</ChildObjects>",
    );
    write_file(&root_xml, &owner);
    write_file(
        &form_meta,
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Form/></MetaDataObject>"#,
    );
    write_file(
        &form_xml,
        r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.21"/>"#,
    );
    let before = [
        (root_xml.clone(), fs::read(&root_xml).unwrap()),
        (form_meta.clone(), fs::read(&form_meta).unwrap()),
        (form_xml.clone(), fs::read(&form_xml).unwrap()),
    ];
    let args = Map::from_iter([
        ("ObjectName".to_string(), json!("Goods")),
        ("FormName".to_string(), json!("ListForm")),
        ("SrcDir".to_string(), json!("src/Catalogs")),
    ]);

    let outcome = remove_form(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    let diagnostics = outcome.errors.join("\n");
    assert!(diagnostics.contains("2.21"), "{diagnostics}");
    assert!(diagnostics.contains("1C 8.5"), "{diagnostics}");
    assert!(
        !diagnostics.contains("older than supported"),
        "{diagnostics}"
    );
    for (path, expected) in before {
        assert_eq!(fs::read(&path).unwrap(), expected, "{}", path.display());
    }
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn remove_form_rejects_platform_invalid_owner_boolean_without_deleting_scaffold() {
    let context = temp_context("remove-invalid-owner-boolean");
    let root_xml = context.cwd.join("src/Catalogs/Goods.xml");
    let form_meta = context.cwd.join("src/Catalogs/Goods/Forms/ListForm.xml");
    let form_xml = context
        .cwd
        .join("src/Catalogs/Goods/Forms/ListForm/Ext/Form.xml");
    let module = context
        .cwd
        .join("src/Catalogs/Goods/Forms/ListForm/Ext/Form/Module.bsl");
    let invalid_owner = empty_catalog_xml("\n", true)
            .replace(
                "\t\t\t<DefaultListForm/>",
                "\t\t\t<DefaultListForm>Catalog.Goods.Form.ListForm</DefaultListForm>\n\t\t\t<IncludeHelpInContents>truthy</IncludeHelpInContents>",
            )
            .replace(
                "\t\t<ChildObjects/>",
                "\t\t<ChildObjects>\n\t\t\t<Form>ListForm</Form>\n\t\t</ChildObjects>",
            );
    write_file(&root_xml, &invalid_owner);
    write_file(&form_meta, "<MetaDataObject version=\"2.20\"/>\n");
    write_file(&form_xml, "<Form version=\"2.20\"/>\n");
    write_file(&module, "// module\n");
    let before = [
        (root_xml.clone(), fs::read(&root_xml).unwrap()),
        (form_meta.clone(), fs::read(&form_meta).unwrap()),
        (form_xml.clone(), fs::read(&form_xml).unwrap()),
        (module.clone(), fs::read(&module).unwrap()),
    ];
    let args = Map::from_iter([
        ("ObjectName".to_string(), json!("Goods")),
        ("FormName".to_string(), json!("ListForm")),
        ("SrcDir".to_string(), json!("src/Catalogs")),
    ]);

    let outcome = remove_form(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("IncludeHelpInContents")
                    && error.contains("true or false")),
            "{outcome:?}"
        );
    for (path, expected) in before {
        assert_eq!(fs::read(&path).unwrap(), expected, "{}", path.display());
    }
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn remove_form_does_not_collapse_unrelated_empty_child_objects() {
    let context = temp_context("remove-preserve-unrelated-childobjects");
    let root_xml = context.cwd.join("src").join("Catalogs").join("Goods.xml");
    let form_meta = context
        .cwd
        .join("src")
        .join("Catalogs")
        .join("Goods")
        .join("Forms")
        .join("ListForm.xml");
    let form_content = context
        .cwd
        .join("src")
        .join("Catalogs")
        .join("Goods")
        .join("Forms")
        .join("ListForm")
        .join("Ext")
        .join("Form.xml");
    let owner = empty_catalog_xml("\n", true).replace(
            "\t\t<ChildObjects/>",
            "\t\t<ChildObjects>\n\t\t\t<TabularSection uuid=\"00000000-0000-4000-8000-000000000201\">\n\t\t\t\t<Properties>\n\t\t\t\t\t<Name>Rows</Name>\n\t\t\t\t\t<Synonym/>\n\t\t\t\t\t<Comment/>\n\t\t\t\t\t<ToolTip/>\n\t\t\t\t\t<FillChecking>DontCheck</FillChecking>\n\t\t\t\t</Properties>\n\t\t\t\t<ChildObjects></ChildObjects>\n\t\t\t</TabularSection>\n\t\t\t<Form>ListForm</Form>\n\t\t</ChildObjects>",
        );
    write_file(&root_xml, &owner);
    write_file(
        &form_meta,
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form/></MetaDataObject>"#,
    );
    write_file(
        &form_content,
        r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"/>"#,
    );

    let mut args = Map::new();
    args.insert("ObjectName".to_string(), json!("Goods"));
    args.insert("FormName".to_string(), json!("ListForm"));
    args.insert("SrcDir".to_string(), json!("src/Catalogs"));

    let outcome = remove_form(&args, &context);

    assert!(outcome.ok, "{:?}", outcome.errors);
    let updated = fs::read_to_string(&root_xml).unwrap();
    assert!(
        updated.contains("<ChildObjects></ChildObjects>"),
        "{updated}"
    );
    assert!(!updated.contains("<Form>ListForm</Form>"), "{updated}");

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn validate_form_rejects_bare_type_values() {
    let context = temp_context("bare-type");
    let form_path = context
        .cwd
        .join("Catalogs")
        .join("Goods")
        .join("Forms")
        .join("ItemForm")
        .join("Ext")
        .join("Form.xml");
    write_file(
        &form_path,
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1">
		<Autofill>true</Autofill>
	</AutoCommandBar>
	<Attributes>
		<Attribute name="BrokenButtonType" id="1">
			<Type>CommandBarButton</Type>
		</Attribute>
	</Attributes>
</Form>
"#,
    );

    let mut args = Map::new();
    args.insert(
        "FormPath".to_string(),
        json!(form_path.display().to_string()),
    );

    let outcome = validate_form(&args, &context);
    let stdout = outcome.stdout.as_deref().unwrap_or("");

    assert!(!outcome.ok, "{stdout}");
    assert!(
        stdout
            .contains("[ERROR] 12. Type \"CommandBarButton\": bare type without namespace prefix"),
        "{stdout}"
    );

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn validate_form_ignores_ui_element_type_properties() {
    let context = temp_context("ui-type-property");
    let form_path = context.cwd.join("Form.xml");
    write_file(
        &form_path,
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/>
	<ChildItems>
		<Button name="RunParityActionButton" id="1">
			<Type>CommandBarButton</Type>
			<CommandName>Form.Command.RunParityAction</CommandName>
			<ExtendedTooltip name="RunParityActionButtonРасширеннаяПодсказка" id="2"/>
		</Button>
	</ChildItems>
	<Commands>
		<Command name="RunParityAction" id="1">
			<Action>RunParityAction</Action>
		</Command>
	</Commands>
</Form>
"#,
    );

    let mut args = Map::new();
    args.insert(
        "FormPath".to_string(),
        json!(form_path.display().to_string()),
    );
    args.insert("Detailed".to_string(), json!(true));

    let outcome = validate_form(&args, &context);
    let stdout = outcome.stdout.as_deref().unwrap_or("");

    assert!(outcome.ok, "{stdout}");
    assert!(
        !stdout.contains("CommandBarButton\": bare type"),
        "{stdout}"
    );
    assert!(
        stdout.contains("12. Types: no type values to check"),
        "{stdout}"
    );

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn remove_form_clears_all_default_form_slots_referencing_removed_form() {
    let context = temp_context("remove-default-slots");
    let root_xml = context.cwd.join("src").join("Catalogs").join("Goods.xml");
    let form_meta = context
        .cwd
        .join("src")
        .join("Catalogs")
        .join("Goods")
        .join("Forms")
        .join("ListForm.xml");
    let form_content = context
        .cwd
        .join("src")
        .join("Catalogs")
        .join("Goods")
        .join("Forms")
        .join("ListForm")
        .join("Ext")
        .join("Form.xml");
    let owner = empty_catalog_xml("\n", true)
            .replace(
                "\t\t\t<DefaultListForm/>",
                "\t\t\t<DefaultObjectForm>Catalog.Goods.Form.ListForm</DefaultObjectForm>\n\t\t\t<DefaultListForm>Catalog.Goods.Form.ListForm</DefaultListForm>\n\t\t\t<DefaultChoiceForm>Catalog.Goods.Form.ListForm</DefaultChoiceForm>\n\t\t\t<DefaultRecordForm>Catalog.Goods.Form.ListForm</DefaultRecordForm>\n\t\t\t<DefaultForm>Catalog.Goods.Form.OtherForm</DefaultForm>",
            )
            .replace(
                "\t\t<ChildObjects/>",
                "\t\t<ChildObjects>\n\t\t\t<Form>ListForm</Form>\n\t\t\t<Form>OtherForm</Form>\n\t\t</ChildObjects>",
            );
    write_file(&root_xml, &owner);
    write_file(
        &form_meta,
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form/></MetaDataObject>"#,
    );
    write_file(
        &form_content,
        r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"/>"#,
    );

    let mut args = Map::new();
    args.insert("ObjectName".to_string(), json!("Goods"));
    args.insert("FormName".to_string(), json!("ListForm"));
    args.insert("SrcDir".to_string(), json!("src/Catalogs"));

    let outcome = remove_form(&args, &context);

    assert!(outcome.ok, "{:?}", outcome.errors);
    let updated = fs::read_to_string(&root_xml).unwrap();
    assert!(
        !updated.contains("<Form>Catalog.Goods.Form.ListForm</Form>"),
        "{updated}"
    );
    for tag in [
        "DefaultObjectForm",
        "DefaultListForm",
        "DefaultChoiceForm",
        "DefaultRecordForm",
    ] {
        assert!(updated.contains(&format!("<{tag}/>")), "{tag}: {updated}");
    }
    assert!(
        updated.contains("<DefaultForm>Catalog.Goods.Form.OtherForm</DefaultForm>"),
        "{updated}"
    );
    assert!(!form_meta.exists());
    assert!(!form_content.exists());

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_rejects_duplicate_attribute_and_command_names() {
    let context = temp_context("edit-duplicates");
    let form_path = context.cwd.join("Form.xml");
    let json_path = context.cwd.join("edit.json");
    write_file(&form_path, editable_form_xml(false));
    write_file(
        &json_path,
        r#"{
  "attributes": [
    {"name": "Object", "type": "CatalogObject.ParityCatalog"}
  ],
  "commands": [
    {"name": "Refresh", "title": "Refresh again"}
  ]
}
"#,
    );

    let mut args = Map::new();
    args.insert(
        "FormPath".to_string(),
        json!(form_path.display().to_string()),
    );
    args.insert(
        "JsonPath".to_string(),
        json!(json_path.display().to_string()),
    );

    let outcome = edit_form(&args, &context);
    assert!(!outcome.ok, "{outcome:?}");
    let stderr = outcome.stderr.unwrap_or_default();
    assert!(
        stderr.contains("Attribute 'Object' already exists in form"),
        "{stderr}"
    );

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_edit_contract_rejects_unknown_json_path_section_before_write() {
    let context = temp_context("edit-strict-json-path");
    let form_path = context.cwd.join("Form.xml");
    let json_path = context.cwd.join("edit.json");
    write_file(&form_path, editable_form_xml(false));
    let original = fs::read(&form_path).unwrap();
    write_file(&json_path, r#"{"unexpectedSection": []}"#);

    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "JsonPath".to_string(),
            json!(json_path.display().to_string()),
        ),
    ]);

    let outcome = edit_form(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| error.contains("FORM_EDIT_UNKNOWN_SECTION")),
        "{:?}",
        outcome.errors
    );
    assert_eq!(fs::read(&form_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_edit_remove_preview_plans_exact_subtree_and_reports_contained_nodes() {
    let context = temp_context("edit-remove-exact");
    let form_path = context.cwd.join("Form.xml");
    let original = form_edit_remove_test_xml(
        r#"		<InputField name="Target" id="1">
			<DataPath>Object.Target</DataPath>
			<ContextMenu name="TargetContextMenu" id="2"/>
			<ExtendedTooltip name="TargetExtendedTooltip" id="3"/>
		</InputField>
		<InputField name="TargetDetails" id="4">
			<ContextMenu name="TargetDetailsContextMenu" id="5"/>
			<ExtendedTooltip name="TargetDetailsExtendedTooltip" id="6"/>
		</InputField>
"#,
    )
    .into_bytes();
    fs::write(&form_path, &original).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({"removeElements": [{"name": "Target"}]}),
        ),
    ]);

    let execution = preview_with_data(&args, &context);
    let outcome = &execution.outcome;

    assert!(outcome.ok, "{outcome:?}");
    assert!(
        outcome
            .changes
            .iter()
            .any(|change| change.contains("would update")),
        "{outcome:?}"
    );
    let data = execution
        .data
        .as_ref()
        .expect("form.edit answers with data");
    let removed = data
        .removed
        .iter()
        .map(|item| (item.name.as_str(), item.kind.as_str(), &item.reason))
        .collect::<Vec<_>>();
    assert_eq!(
        removed,
        vec![
            ("Target", "InputField", &FormEditRemovalReason::Requested),
            (
                "TargetContextMenu",
                "ContextMenu",
                &FormEditRemovalReason::Contained
            ),
            (
                "TargetExtendedTooltip",
                "ExtendedTooltip",
                &FormEditRemovalReason::Contained
            ),
        ],
        "{data:?}"
    );
    assert!(
        !data.removed.iter().any(|item| item.name == "TargetDetails"),
        "a sibling subtree must stay: {data:?}"
    );
    assert_eq!(fs::read(&form_path).unwrap(), original);

    let outcome = edit_form(&args, &context);
    assert!(outcome.ok, "{outcome:?}");
    let updated = fs::read_to_string(&form_path).unwrap();
    assert!(
        !updated.contains("<InputField name=\"Target\" id=\"1\">"),
        "{updated}"
    );
    assert!(!updated.contains("TargetContextMenu"), "{updated}");
    assert!(!updated.contains("TargetExtendedTooltip"), "{updated}");
    assert!(updated.contains("name=\"TargetDetails\""), "{updated}");

    let applied = fs::read(&form_path).unwrap();
    let repeated = edit_form(&args, &context);
    assert!(!repeated.ok, "{repeated:?}");
    assert!(
        repeated
            .errors
            .iter()
            .any(|error| error.contains("FORM_ELEMENT_NOT_FOUND")),
        "{repeated:?}"
    );
    assert_eq!(fs::read(&form_path).unwrap(), applied);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_edit_remove_matches_element_names_exactly_without_trimming() {
    let context = temp_context("edit-remove-exact-whitespace");
    let form_path = context.cwd.join("Form.xml");
    let original =
        form_edit_remove_test_xml("\t\t<InputField name=\"Target\" id=\"1\"/>\n").into_bytes();
    fs::write(&form_path, &original).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({"removeElements": [{"name": "Target "}]}),
        ),
    ]);

    let outcome = preview_form_edit(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| error.contains("FORM_ELEMENT_NOT_FOUND")),
        "{outcome:?}"
    );
    assert_eq!(fs::read(&form_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_edit_remove_applies_to_an_input_field_inside_table_child_items() {
    let context = temp_context("edit-remove-table-column-input");
    let form_path = context.cwd.join("Form.xml");
    fs::write(&form_path, form_edit_remove_test_xml("")).unwrap();
    let create_args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({
                "attributes": [{
                    "name": "Rows",
                    "type": "ТаблицаЗначений",
                    "columns": [{"name": "Value", "type": "string"}]
                }],
                "elements": [{
                    "table": "Rows",
                    "path": "Rows",
                    "columns": [{"input": "Value", "path": "Rows.Value"}]
                }]
            }),
        ),
    ]);
    let created = edit_form(&create_args, &context);
    assert!(created.ok, "{created:?}");
    let before_removal = fs::read_to_string(&form_path).unwrap();
    assert!(
        before_removal.contains("<Table name=\"Rows\""),
        "{before_removal}"
    );
    assert!(
        before_removal.contains("<InputField name=\"Value\""),
        "{before_removal}"
    );

    let remove_args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({"removeElements": [{"name": "Value"}]}),
        ),
    ]);
    let removed = edit_form(&remove_args, &context);
    assert!(removed.ok, "{removed:?}");
    let updated = fs::read_to_string(&form_path).unwrap();
    assert!(updated.contains("<Table name=\"Rows\""), "{updated}");
    assert!(!updated.contains("<InputField name=\"Value\""), "{updated}");
    assert!(
        updated.contains("<Column name=\"Value\""),
        "attribute column must remain: {updated}"
    );
    let validation = validate_form(&remove_args, &context);
    assert!(validation.ok, "{validation:?}");

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_edit_remove_apply_is_atomic_when_a_later_target_is_missing() {
    let context = temp_context("edit-remove-atomic-missing");
    let form_path = context.cwd.join("Form.xml");
    let original = form_edit_remove_test_xml(
        r#"		<InputField name="First" id="1"/>
		<InputField name="Second" id="2"/>
"#,
    )
    .into_bytes();
    fs::write(&form_path, &original).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({
                "removeElements": [
                    {"name": "First"},
                    {"name": "Missing"},
                    {"name": "Second"}
                ]
            }),
        ),
    ]);

    let outcome = edit_form(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| error.contains("FORM_ELEMENT_NOT_FOUND")),
        "{outcome:?}"
    );
    assert!(outcome.changes.is_empty(), "{outcome:?}");
    assert_eq!(fs::read(&form_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_edit_remove_rolls_back_when_a_mixed_addition_is_invalid() {
    let context = temp_context("edit-remove-atomic-invalid-addition");
    let form_path = context.cwd.join("Form.xml");
    let original = form_edit_remove_test_xml(
        r#"		<InputField name="Target" id="1">
			<ContextMenu name="TargetContextMenu" id="2"/>
			<ExtendedTooltip name="TargetExtendedTooltip" id="3"/>
		</InputField>
"#,
    )
    .into_bytes();
    fs::write(&form_path, &original).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({
                "removeElements": [{"name": "Target"}],
                "elements": [{"input": "Broken", "path": "Missing.Value"}]
            }),
        ),
    ]);

    let preview = preview_form_edit(&args, &context);
    assert!(!preview.ok, "{preview:?}");
    assert!(
        preview
            .errors
            .iter()
            .any(|error| error.contains("attribute 'Missing' not found")),
        "{preview:?}"
    );
    assert_eq!(fs::read(&form_path).unwrap(), original);

    let apply = edit_form(&args, &context);
    assert!(!apply.ok, "{apply:?}");
    assert_eq!(apply.errors, preview.errors);
    assert_eq!(fs::read(&form_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_edit_remove_apply_preserves_bom_and_crlf() {
    let context = temp_context("edit-remove-bom-crlf");
    let form_path = context.cwd.join("Form.xml");
    let crlf_xml = form_edit_remove_test_xml(
        r#"		<InputField name="RemoveMe" id="1">
			<ContextMenu name="RemoveMeContextMenu" id="2"/>
			<ExtendedTooltip name="RemoveMeExtendedTooltip" id="3"/>
		</InputField>
		<InputField name="KeepMe" id="4">
			<ContextMenu name="KeepMeContextMenu" id="5"/>
			<ExtendedTooltip name="KeepMeExtendedTooltip" id="6"/>
		</InputField>
"#,
    )
    .replace('\n', "\r\n");
    let mut original = vec![0xef, 0xbb, 0xbf];
    original.extend_from_slice(crlf_xml.as_bytes());
    fs::write(&form_path, &original).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({"removeElements": [{"name": "RemoveMe"}]}),
        ),
    ]);

    let outcome = edit_form(&args, &context);

    assert!(outcome.ok, "{outcome:?}");
    let updated = fs::read(&form_path).unwrap();
    assert!(updated.starts_with(&[0xef, 0xbb, 0xbf]), "{updated:?}");
    assert!(
        !updated[3..].starts_with(&[0xef, 0xbb, 0xbf]),
        "{updated:?}"
    );
    let updated_text = std::str::from_utf8(&updated[3..]).unwrap();
    assert_platform_text_uses_crlf_without_bare_lf(updated_text);
    assert!(
        !updated_text.contains("name=\"RemoveMe\""),
        "{updated_text}"
    );
    assert!(updated_text.contains("name=\"KeepMe\""), "{updated_text}");

    let validation = validate_form(&args, &context);
    assert!(validation.ok, "{validation:?}");

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_edit_preview_apply_and_no_op_validate_the_projected_form() {
    let context = temp_context("edit-remove-project-validation");
    let form_path = context.cwd.join("Form.xml");
    let original = form_edit_remove_test_xml(
        r#"		<InputField name="RemoveMe" id="1">
			<ContextMenu name="RemoveMeContextMenu" id="2"/>
			<ExtendedTooltip name="RemoveMeExtendedTooltip" id="3"/>
		</InputField>
		<InputField name="AlreadyInvalid" id="4"/>
"#,
    )
    .into_bytes();
    fs::write(&form_path, &original).unwrap();
    let removal_args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({"removeElements": [{"name": "RemoveMe"}]}),
        ),
    ]);

    let preview = preview_form_edit(&removal_args, &context);
    assert!(!preview.ok, "{preview:?}");
    assert!(
        preview
            .errors
            .iter()
            .any(|error| error.contains("AlreadyInvalid") && error.contains("missing companion")),
        "{preview:?}"
    );
    assert_eq!(fs::read(&form_path).unwrap(), original);

    let apply = edit_form(&removal_args, &context);
    assert!(!apply.ok, "{apply:?}");
    assert_eq!(apply.errors, preview.errors);
    assert_eq!(fs::read(&form_path).unwrap(), original);

    let no_op_args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        ("definition".to_string(), json!({})),
    ]);
    let no_op = edit_form(&no_op_args, &context);
    assert!(!no_op.ok, "{no_op:?}");
    assert!(
        no_op
            .errors
            .iter()
            .any(|error| error.contains("AlreadyInvalid") && error.contains("missing companion")),
        "{no_op:?}"
    );
    assert_eq!(fs::read(&form_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_edit_remove_preview_reports_missing_target_with_stable_code() {
    let context = temp_context("edit-remove-missing");
    let form_path = context.cwd.join("Form.xml");
    let original = form_edit_remove_test_xml("").into_bytes();
    fs::write(&form_path, &original).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({"removeElements": [{"name": "Missing"}]}),
        ),
    ]);

    let outcome = preview_form_edit(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| error.contains("FORM_ELEMENT_NOT_FOUND")),
        "{outcome:?}"
    );
    assert_eq!(fs::read(&form_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_edit_remove_preview_rejects_ambiguous_public_target() {
    let context = temp_context("edit-remove-ambiguous");
    let form_path = context.cwd.join("Form.xml");
    let original = form_edit_remove_test_xml(
        r#"		<InputField name="Duplicate" id="1"/>
		<UsualGroup name="Container" id="2">
			<ChildItems>
				<InputField name="Duplicate" id="3"/>
			</ChildItems>
		</UsualGroup>
"#,
    )
    .into_bytes();
    fs::write(&form_path, &original).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({"removeElements": [{"name": "Duplicate"}]}),
        ),
    ]);

    let outcome = preview_form_edit(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| error.contains("FORM_EDIT_REMOVE_ELEMENT_AMBIGUOUS")),
        "{outcome:?}"
    );
    assert_eq!(fs::read(&form_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_edit_remove_scopes_extension_targets_to_the_working_tree() {
    let context = temp_context("edit-remove-extension-working-tree");
    let form_path = context.cwd.join("Form.xml");
    let target = r#"		<InputField name="Target" id="1">
			<ContextMenu name="TargetContextMenu" id="2"/>
			<ExtendedTooltip name="TargetExtendedTooltip" id="3"/>
		</InputField>
"#;
    let baseline = format!(
        r#"{target}		<InputField name="BaselineDependent" id="4">
			<DataPath>Items.Target.CurrentData.Value</DataPath>
			<ContextMenu name="BaselineDependentContextMenu" id="5"/>
			<ExtendedTooltip name="BaselineDependentExtendedTooltip" id="6"/>
		</InputField>
"#
    );
    let original = form_edit_remove_extension_test_xml(target, baseline.as_str()).into_bytes();
    fs::write(&form_path, &original).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({"removeElements": [{"name": "Target"}]}),
        ),
    ]);

    let preview = preview_form_edit(&args, &context);
    assert!(preview.ok, "{preview:?}");
    assert_eq!(fs::read(&form_path).unwrap(), original);

    let apply = edit_form(&args, &context);
    assert!(apply.ok, "{apply:?}");
    let updated = fs::read_to_string(&form_path).unwrap();
    let document = Document::parse(&updated).unwrap();
    let root = document.root_element();
    let base_form = form_validation_child(root, "BaseForm").unwrap();
    assert!(
        form_validation_child(root, "ChildItems")
            .unwrap()
            .descendants()
            .all(|node| node.attribute("name") != Some("Target")),
        "{updated}"
    );
    assert!(
        base_form
            .descendants()
            .any(|node| node.attribute("name") == Some("Target")),
        "{updated}"
    );

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_edit_remove_never_targets_a_baseline_only_element() {
    let context = temp_context("edit-remove-extension-baseline-only");
    let form_path = context.cwd.join("Form.xml");
    let baseline_target = r#"		<InputField name="Target" id="1">
			<ContextMenu name="TargetContextMenu" id="2"/>
			<ExtendedTooltip name="TargetExtendedTooltip" id="3"/>
		</InputField>
"#;
    let original = form_edit_remove_extension_test_xml("", baseline_target).into_bytes();
    fs::write(&form_path, &original).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({"removeElements": [{"name": "Target"}]}),
        ),
    ]);

    let preview = preview_form_edit(&args, &context);
    assert!(!preview.ok, "{preview:?}");
    assert!(
        preview
            .errors
            .iter()
            .any(|error| error.contains("FORM_ELEMENT_NOT_FOUND")),
        "{preview:?}"
    );
    assert_eq!(fs::read(&form_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_edit_remove_preview_rejects_protected_root_and_nested_targets() {
    for (case, name, child_items) in [
        ("root", "FormCommandBar", ""),
        (
            "nested",
            "TargetContextMenu",
            r#"		<InputField name="Target" id="1">
			<ContextMenu name="TargetContextMenu" id="2"/>
		</InputField>
"#,
        ),
    ] {
        let context = temp_context(&format!("edit-remove-protected-{case}"));
        let form_path = context.cwd.join("Form.xml");
        let original = form_edit_remove_test_xml(child_items).into_bytes();
        fs::write(&form_path, &original).unwrap();
        let args = Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            (
                "definition".to_string(),
                json!({"removeElements": [{"name": name}]}),
            ),
        ]);

        let outcome = preview_form_edit(&args, &context);

        assert!(!outcome.ok, "{case}: {outcome:?}");
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("FORM_EDIT_REMOVE_ELEMENT_PROTECTED")),
            "{case}: {outcome:?}"
        );
        assert_eq!(fs::read(&form_path).unwrap(), original);

        let _ = fs::remove_dir_all(&context.cwd);
    }
}

#[test]
fn form_edit_remove_preview_rejects_overlapping_requested_subtrees() {
    let context = temp_context("edit-remove-overlap");
    let form_path = context.cwd.join("Form.xml");
    let original = form_edit_remove_test_xml(
        r#"		<UsualGroup name="Container" id="1">
			<ChildItems>
				<InputField name="Nested" id="2"/>
			</ChildItems>
			<ExtendedTooltip name="ContainerExtendedTooltip" id="3"/>
		</UsualGroup>
"#,
    )
    .into_bytes();
    fs::write(&form_path, &original).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({
                "removeElements": [
                    {"name": "Container"},
                    {"name": "Nested"}
                ]
            }),
        ),
    ]);

    let outcome = preview_form_edit(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| error.contains("FORM_EDIT_REMOVE_ELEMENT_OVERLAP")),
        "{outcome:?}"
    );
    assert_eq!(fs::read(&form_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

fn assert_form_edit_remove_rejected_identically(
    case: &str,
    child_items: &str,
    definition: Value,
    expected_diagnostic_parts: &[&str],
) {
    let context = temp_context(case);
    let form_path = context.cwd.join("Form.xml");
    let original = form_edit_remove_test_xml(child_items).into_bytes();
    fs::write(&form_path, &original).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        ("definition".to_string(), definition),
    ]);

    let preview = preview_form_edit(&args, &context);
    assert!(!preview.ok, "{case} preview: {preview:?}");
    assert_eq!(
        fs::read(&form_path).unwrap(),
        original,
        "{case} preview changed source bytes"
    );

    let apply = edit_form(&args, &context);
    assert!(!apply.ok, "{case} apply: {apply:?}");
    assert_eq!(
        fs::read(&form_path).unwrap(),
        original,
        "{case} apply changed source bytes"
    );
    assert_eq!(
        preview.errors, apply.errors,
        "{case}: preview/apply errors differ"
    );
    assert_eq!(
        preview.stderr, apply.stderr,
        "{case}: preview/apply stderr differs"
    );
    for expected in expected_diagnostic_parts {
        assert!(
            preview.errors.iter().any(|error| error.contains(expected)),
            "{case}: missing {expected:?} in {preview:?}"
        );
    }

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_edit_remove_rejects_new_element_names_at_every_definition_depth() {
    let child_items = "\t\t<InputField name=\"Target\" id=\"1\"/>\n";
    for (case, elements, property) in [
        (
            "edit-remove-conflicting-root-element",
            json!([{"input": "Target"}]),
            "elements[0]",
        ),
        (
            "edit-remove-conflicting-child-element",
            json!([{"group": "AddedGroup", "children": [{"input": "Target"}]}]),
            "elements[0].children[0]",
        ),
        (
            "edit-remove-conflicting-column-element",
            json!([{
                "table": "AddedTable",
                "path": "Rows",
                "columns": [{"input": "Target", "path": "Rows.Target"}]
            }]),
            "elements[0].columns[0]",
        ),
    ] {
        assert_form_edit_remove_rejected_identically(
            case,
            child_items,
            json!({
                "removeElements": [{"name": "Target"}],
                "elements": elements
            }),
            &["FORM_EDIT_REMOVE_DEFINITION_CONFLICT", property, "Target"],
        );
    }
}

#[test]
fn form_edit_remove_rejects_into_and_after_targets_in_removed_subtrees() {
    let child_items = r#"		<UsualGroup name="Container" id="1">
			<ChildItems>
				<InputField name="Nested" id="2"/>
			</ChildItems>
		</UsualGroup>
"#;
    for (property, target) in [("into", "Nested"), ("after", "Container")] {
        assert_form_edit_remove_rejected_identically(
            &format!("edit-remove-conflicting-{property}"),
            child_items,
            json!({
                "removeElements": [{"name": "Container"}],
                (property): target,
                "elements": [{"input": "Added"}]
            }),
            &["FORM_EDIT_REMOVE_DEFINITION_CONFLICT", property, target],
        );
    }
}

#[test]
fn form_edit_remove_does_not_treat_event_names_as_contained_elements() {
    let context = temp_context("edit-remove-event-name-is-not-element");
    let form_path = context.cwd.join("Form.xml");
    let original = form_edit_remove_test_xml(
        r#"		<InputField name="Target" id="1">
			<Events>
				<Event name="OnChange">TargetOnChange</Event>
			</Events>
			<ContextMenu name="TargetContextMenu" id="2"/>
			<ExtendedTooltip name="TargetExtendedTooltip" id="3"/>
		</InputField>
		<UsualGroup name="OnChange" id="4">
			<ChildItems/>
			<ExtendedTooltip name="OnChangeExtendedTooltip" id="5"/>
		</UsualGroup>
"#,
    )
    .into_bytes();
    fs::write(&form_path, &original).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({
                "removeElements": [{"name": "Target"}],
                "into": "OnChange",
                "elements": [{"input": "Added"}]
            }),
        ),
    ]);

    let preview = preview_with_data(&args, &context);
    assert!(preview.outcome.ok, "{:?}", preview.outcome);
    let data = serde_json::to_value(preview.data).unwrap();
    assert!(
        data["removed"]
            .as_array()
            .unwrap()
            .iter()
            .all(|removed| removed["name"] != "OnChange"),
        "{data}"
    );
    assert_eq!(fs::read(&form_path).unwrap(), original);

    let apply = edit_form(&args, &context);
    assert!(apply.ok, "{apply:?}");
    let updated = fs::read_to_string(&form_path).unwrap();
    assert!(!updated.contains("TargetOnChange"), "{updated}");
    assert!(updated.contains("name=\"OnChange\""), "{updated}");
    assert!(updated.contains("name=\"Added\""), "{updated}");

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_edit_remove_rejects_element_events_for_removed_subtree_nodes() {
    let child_items = r#"		<InputField name="Target" id="1">
			<DataPath>Object.Target</DataPath>
			<ContextMenu name="TargetContextMenu" id="2"/>
			<ExtendedTooltip name="TargetExtendedTooltip" id="3"/>
		</InputField>
"#;
    assert_form_edit_remove_rejected_identically(
        "edit-remove-conflicting-element-event",
        child_items,
        json!({
            "removeElements": [{"name": "Target"}],
            "elementEvents": [{
                "element": "TargetContextMenu",
                "name": "OnChange",
                "handler": "TargetOnChange"
            }]
        }),
        &[
            "FORM_EDIT_REMOVE_DEFINITION_CONFLICT",
            "elementEvents[0].element",
            "TargetContextMenu",
        ],
    );
}

#[test]
fn form_edit_remove_rejects_surviving_supported_xml_references() {
    let cases = [
        (
            "binding-path",
            r#"		<InputField name="Target" id="1"/>
		<InputField name="Dependent" id="2">
			<DataPath>Items.Target.CurrentData.Value</DataPath>
		</InputField>
"#,
            "DataPath",
            "Dependent",
        ),
        (
            "standard-command",
            r#"		<InputField name="Target" id="1"/>
		<Button name="Dependent" id="2">
			<CommandName>Form.Item.Target.StandardCommand.Add</CommandName>
		</Button>
"#,
            "CommandName",
            "Dependent",
        ),
        (
            "dotted-standard-command-target",
            r#"		<Table name="Table.Group" id="1"/>
			<Button name="Dependent" id="2">
				<CommandName>Form.Item.Table.Group.StandardCommand.Add</CommandName>
			</Button>
"#,
            "CommandName",
            "Dependent",
        ),
        (
            "contained-companion-standard-command",
            r#"		<InputField name="Target" id="1">
			<ContextMenu name="TargetContextMenu" id="2"/>
			<ExtendedTooltip name="TargetExtendedTooltip" id="3"/>
		</InputField>
			<Button name="Dependent" id="4">
				<CommandName>Form.Item.TargetContextMenu.StandardCommand.Add</CommandName>
				<ExtendedTooltip name="DependentExtendedTooltip" id="5"/>
			</Button>
"#,
            "CommandName",
            "Dependent",
        ),
        (
            "dotted-binding-path",
            r#"		<Table name="Table.Group" id="1"/>
			<InputField name="Dependent" id="2">
				<DataPath>Items.Table.Group.CurrentData.Value</DataPath>
				<ContextMenu name="DependentContextMenu" id="3"/>
				<ExtendedTooltip name="DependentExtendedTooltip" id="4"/>
			</InputField>
"#,
            "DataPath",
            "Dependent",
        ),
        (
            "addition-source-item",
            r#"		<Table name="Target" id="1"/>
		<SearchStringAddition name="Dependent" id="2">
			<AdditionSource>
				<Item>Target</Item>
				<Type>SearchStringRepresentation</Type>
			</AdditionSource>
		</SearchStringAddition>
"#,
            "AdditionSource/Item",
            "Dependent",
        ),
    ];

    for (case, child_items, property, owner) in cases {
        let removal_target = if case.starts_with("dotted-") {
            "Table.Group"
        } else {
            "Target"
        };
        let reference_target = if case == "contained-companion-standard-command" {
            "TargetContextMenu"
        } else {
            removal_target
        };
        assert_form_edit_remove_rejected_identically(
            &format!("edit-remove-dangling-{case}"),
            child_items,
            json!({"removeElements": [{"name": removal_target}]}),
            &[
                "FORM_EDIT_REMOVE_SURVIVING_REFERENCE",
                property,
                owner,
                reference_target,
            ],
        );
    }
}

#[test]
fn form_edit_remove_rejects_prefixed_items_reference_for_inline_and_json_path() {
    let child_items = r#"		<InputField name="Target" id="1"/>
		<InputField name="Dependent" id="2">
			<DataPath>~Items.Target.CurrentData.Value</DataPath>
		</InputField>
"#;
    let definition = json!({"removeElements": [{"name": "Target"}]});

    for definition_mode in ["definition", "JsonPath"] {
        let context = temp_context(&format!(
            "edit-remove-prefixed-items-{}",
            definition_mode.to_ascii_lowercase()
        ));
        let form_path = context.cwd.join("Form.xml");
        let original_form = form_edit_remove_test_xml(child_items).into_bytes();
        fs::write(&form_path, &original_form).unwrap();
        let mut args = Map::from_iter([(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        )]);
        let definition_path = context.cwd.join("edit.json");
        let original_definition = serde_json::to_vec_pretty(&definition).unwrap();
        if definition_mode == "definition" {
            args.insert("definition".to_string(), definition.clone());
        } else {
            fs::write(&definition_path, &original_definition).unwrap();
            args.insert(
                "JsonPath".to_string(),
                json!(definition_path.display().to_string()),
            );
        }

        let preview = preview_form_edit(&args, &context);
        let form_after_preview = fs::read(&form_path).unwrap();
        let apply = edit_form(&args, &context);
        let form_after_apply = fs::read(&form_path).unwrap();

        assert!(!preview.ok, "{definition_mode} preview: {preview:?}");
        assert!(!apply.ok, "{definition_mode} apply: {apply:?}");
        assert_eq!(
            preview.errors, apply.errors,
            "{definition_mode}: preview/apply errors differ"
        );
        assert_eq!(
            preview.stderr, apply.stderr,
            "{definition_mode}: preview/apply stderr differs"
        );
        let expected_diagnostic = "FORM_EDIT_REMOVE_SURVIVING_REFERENCE: surviving element \
`Dependent` property `DataPath` references removed element `Target` \
(value `~Items.Target.CurrentData.Value`)";
        for outcome in [&preview, &apply] {
            assert!(
                outcome
                    .errors
                    .iter()
                    .any(|error| error == expected_diagnostic),
                "{definition_mode}: {outcome:?}"
            );
        }
        assert_eq!(
            form_after_preview, original_form,
            "{definition_mode}: preview changed source bytes"
        );
        assert_eq!(
            form_after_apply, original_form,
            "{definition_mode}: apply changed source bytes"
        );
        if definition_mode == "JsonPath" {
            assert_eq!(
                fs::read(&definition_path).unwrap(),
                original_definition,
                "JsonPath definition bytes changed"
            );
        }

        let _ = fs::remove_dir_all(&context.cwd);
    }
}

#[test]
fn form_edit_remove_rejects_only_exact_surviving_references_outside_removed_subtrees() {
    let context = temp_context("edit-remove-reference-exactness");
    let form_path = context.cwd.join("Form.xml");
    let original = form_edit_remove_test_xml(
        r#"		<InputField name="Table" id="1">
			<ContextMenu name="TableContextMenu" id="2"/>
			<ExtendedTooltip name="TableExtendedTooltip" id="3"/>
		</InputField>
		<Table name="Table.Group" id="4">
			<ContextMenu name="TableGroupContextMenu" id="5"/>
			<AutoCommandBar name="TableGroupCommandBar" id="6"/>
			<SearchStringAddition name="TableGroupSearchString" id="7"/>
			<ViewStatusAddition name="TableGroupViewStatus" id="8"/>
			<SearchControlAddition name="TableGroupSearchControl" id="9"/>
		</Table>
		<InputField name="Survivor" id="10">
			<DataPath>~Items.Table.Group.CurrentData.Value</DataPath>
			<ContextMenu name="SurvivorContextMenu" id="11"/>
			<ExtendedTooltip name="SurvivorExtendedTooltip" id="12"/>
		</InputField>
"#,
    )
    .into_bytes();
    fs::write(&form_path, &original).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({"removeElements": [{"name": "Table"}]}),
        ),
    ]);

    let preview = preview_form_edit(&args, &context);

    assert!(preview.ok, "{preview:?}");
    assert_eq!(fs::read(&form_path).unwrap(), original);
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_refuses_exhausted_ui_ids_without_writing() {
    let context = temp_context("edit-exhausted-ui-ids");
    let form_path = context.cwd.join("Form.xml");
    for highest in [usize::MAX, usize::MAX - 1] {
        let original = form_edit_remove_test_xml(&format!(
            r#"<HTMLDocumentField name="Imported" id="{highest}"/>"#
        ));
        fs::write(&form_path, &original).unwrap();
        let mut unchanged = original.clone();
        let root_start = Document::parse(&original)
            .unwrap()
            .root_element()
            .range()
            .start;
        form_edit_apply_definition(
            &mut unchanged,
            &json!({}),
            &form_path,
            root_start,
            Some("ru"),
        )
        .unwrap();
        assert_eq!(unchanged, original, "a no-op does not allocate an ID");
        let args = Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            (
                "definition".to_string(),
                json!({"elements": [{"input": "Added"}]}),
            ),
        ]);
        for edit in [preview_form_edit, edit_form] {
            let outcome = edit(&args, &context);
            assert!(!outcome.ok, "{outcome:?}");
            assert!(
                format!("{outcome:?}").contains("Form ID space exhausted"),
                "{outcome:?}"
            );
            assert_eq!(fs::read_to_string(&form_path).unwrap(), original);
        }
    }
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_allocates_ids_above_all_existing_ui_nodes() {
    let context = temp_context("edit-imported-ui-ids");
    let form_path = context.cwd.join("Form.xml");
    // Imported forms need not number the element before its companions.
    for kind in ["HTMLDocumentField", "CheckBoxField", "PictureField"] {
        for extension in [false, true] {
            let highest = if extension { 1_000_000 } else { 5 };
            let element = format!(
                r#"<UsualGroup name="Container" id="1"><ExtendedTooltip name="ContainerTooltip" id="2"/><ChildItems><{kind} name="Imported" id="{highest}"><ContextMenu name="ImportedMenu" id="3"/><ExtendedTooltip name="ImportedTooltip" id="4"/></{kind}></ChildItems></UsualGroup>"#
            );
            let original = editable_form_xml(extension).replace(
                "\t<ChildItems>\n\t</ChildItems>",
                &format!("<ChildItems>{element}</ChildItems>"),
            );
            fs::write(&form_path, &original).unwrap();
            let args = Map::from_iter([
                (
                    "FormPath".to_string(),
                    json!(form_path.display().to_string()),
                ),
                (
                    "definition".to_string(),
                    json!({"elements": [{"input": "Added"}]}),
                ),
            ]);
            let preview = preview_form_edit(&args, &context);
            assert!(preview.ok, "{kind}, extension={extension}: {preview:?}");
            assert_eq!(fs::read_to_string(&form_path).unwrap(), original);
            let applied = edit_form(&args, &context);
            assert!(applied.ok, "{kind}, extension={extension}: {applied:?}");
            let updated = fs::read_to_string(&form_path).unwrap();
            let doc = Document::parse(&updated).unwrap();
            let added = doc
                .descendants()
                .find(|n| n.attribute("name") == Some("Added"))
                .unwrap();
            assert_eq!(added.attribute("id").unwrap(), (highest + 1).to_string());
            let ids: Vec<_> = doc
                .descendants()
                .filter(|n| n.ancestors().any(|a| a.has_tag_name("ChildItems")))
                .filter_map(|n| n.attribute("id"))
                .collect();
            assert_eq!(
                ids.len(),
                ids.iter().collect::<HashSet<_>>().len(),
                "{updated}"
            );
            assert!(
                updated.contains(&element),
                "existing subtree changed: {updated}"
            );
        }
    }
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_reserves_companion_ids_without_mixing_attribute_and_command_ids() {
    let context = temp_context("edit-companion-ids");
    let form_path = context.cwd.join("Form.xml");
    let table = r#"<Table name="Rows" id="1"><ContextMenu name="RowsMenu" id="2"/><AutoCommandBar name="RowsBar" id="3"/><SearchStringAddition name="RowsSearch" id="4"/><ViewStatusAddition name="RowsStatus" id="5"/><SearchControlAddition name="RowsSearchControl" id="6"/></Table>"#;
    for highest_node in [
        "RowsBar",
        "RowsSearch",
        "RowsStatus",
        "RowsSearchControl",
        "RootTooltip",
    ] {
        let original = form_edit_remove_test_xml(table)
                .replace("<AutoCommandBar name=\"FormCommandBar\" id=\"-1\"/>",
                    "<AutoCommandBar name=\"FormCommandBar\" id=\"-1\"><ExtendedTooltip name=\"RootTooltip\" id=\"7\"/></AutoCommandBar>")
                .replace("<Attributes/>", "<Attributes><Attribute name=\"ExistingAttribute\" id=\"100\"/></Attributes>")
                .replace("<Commands/>", "<Commands><Command name=\"ExistingCommand\" id=\"200\"><Action>ExistingCommand</Action></Command></Commands>");
        let doc = Document::parse(&original).unwrap();
        let highest = doc
            .descendants()
            .find(|n| n.attribute("name") == Some(highest_node))
            .unwrap();
        let old_id = highest.attribute("id").unwrap();
        let original = original.replace(
            &format!("name=\"{highest_node}\" id=\"{old_id}\""),
            &format!("name=\"{highest_node}\" id=\"10\""),
        );
        fs::write(&form_path, &original).unwrap();
        let args = Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            (
                "definition".to_string(),
                json!({"elements": [{"input": "Added"}]}),
            ),
        ]);
        let preview = preview_form_edit(&args, &context);
        assert!(preview.ok, "{highest_node}: {preview:?}");
        assert_eq!(fs::read_to_string(&form_path).unwrap(), original);
        let applied = edit_form(&args, &context);
        assert!(applied.ok, "{highest_node}: {applied:?}");
        let updated = fs::read_to_string(&form_path).unwrap();
        let doc = Document::parse(&updated).unwrap();
        let added = doc
            .descendants()
            .find(|n| n.attribute("name") == Some("Added"))
            .unwrap();
        assert_eq!(
            added.attribute("id"),
            Some("11"),
            "{highest_node}: {updated}"
        );
        assert_eq!(
            doc.descendants()
                .find(|n| n.attribute("name") == Some("ExistingAttribute"))
                .unwrap()
                .attribute("id"),
            Some("100")
        );
        assert_eq!(
            doc.descendants()
                .find(|n| n.attribute("name") == Some("ExistingCommand"))
                .unwrap()
                .attribute("id"),
            Some("200")
        );
    }
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_uses_extension_id_floor_when_base_form_exists() {
    let context = temp_context("edit-extension-ids");
    let form_path = context.cwd.join("Form.xml");
    let json_path = context.cwd.join("edit.json");
    write_file(&form_path, editable_form_xml(true));
    write_file(
        &json_path,
        r#"{
  "attributes": [
    {"name": "NewAttribute", "type": "string"}
  ],
  "commands": [
    {"name": "NewCommand", "title": "New command", "action": "NewCommand"}
  ],
  "elements": [
    {"input": "NewAttribute", "path": "NewAttribute"}
  ]
}
"#,
    );

    let mut args = Map::new();
    args.insert(
        "FormPath".to_string(),
        json!(form_path.display().to_string()),
    );
    args.insert(
        "JsonPath".to_string(),
        json!(json_path.display().to_string()),
    );

    let outcome = edit_form(&args, &context);
    assert!(outcome.ok, "{outcome:?}");
    let updated = fs::read_to_string(&form_path).unwrap();
    assert!(updated.contains("id=\"1000000\""), "{updated}");
    assert!(updated.contains("id=\"1000001\""), "{updated}");

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_emits_valuetable_attribute_columns() {
    let context = temp_context("edit-valuetable-columns");
    let form_path = context.cwd.join("Form.xml");
    let json_path = context.cwd.join("edit.json");
    write_file(
        &form_path,
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1">
		<Autofill>true</Autofill>
	</AutoCommandBar>
	<ChildItems>
		<UsualGroup name="ГруппаДанных" id="1">
			<ChildItems/>
			<ExtendedTooltip name="ГруппаДанныхРасширеннаяПодсказка" id="2"/>
		</UsualGroup>
	</ChildItems>
	<Attributes/>
	<Commands/>
</Form>
"#,
    );
    write_file(
        &json_path,
        r#"{
  "into": "ГруппаДанных",
  "attributes": [
    {
      "name": "ТаблицаДанных",
      "type": "ТаблицаЗначений",
      "columns": [
        {"name": "НомерСтроки", "type": "decimal(5,0)"},
        {"name": "Значение", "type": "string(200)"}
      ]
    }
  ],
  "elements": [
    {
      "table": "ТаблицаДанных",
      "path": "ТаблицаДанных",
      "columns": [
        {"input": "НомерСтроки", "path": "ТаблицаДанных.НомерСтроки"},
        {"input": "Значение", "path": "ТаблицаДанных.Значение"}
      ]
    }
  ]
}
"#,
    );

    let mut args = Map::new();
    args.insert(
        "FormPath".to_string(),
        json!(form_path.display().to_string()),
    );
    args.insert(
        "JsonPath".to_string(),
        json!(json_path.display().to_string()),
    );

    let outcome = edit_form(&args, &context);
    assert!(outcome.ok, "{outcome:?}");
    let updated = fs::read_to_string(&form_path).unwrap();
    assert!(updated.contains("<Columns>"), "{updated}");
    assert!(
        updated.contains("<Column name=\"НомерСтроки\" id=\"1\">"),
        "{updated}"
    );
    assert!(
        updated.contains("<Column name=\"Значение\" id=\"2\">"),
        "{updated}"
    );
    assert!(
        updated.contains("<v8:Type>xs:decimal</v8:Type>"),
        "{updated}"
    );
    assert!(updated.contains("<v8:NumberQualifiers>"), "{updated}");
    assert!(
        updated.contains("<v8:Type>xs:string</v8:Type>"),
        "{updated}"
    );
    assert!(updated.contains("<v8:StringQualifiers>"), "{updated}");
    assert!(
        updated.contains("<DataPath>ТаблицаДанных.НомерСтроки</DataPath>"),
        "{updated}"
    );
    assert!(
        updated.contains("<DataPath>ТаблицаДанных.Значение</DataPath>"),
        "{updated}"
    );

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_rejects_duplicate_attribute_column_names() {
    let context = temp_context("edit-duplicate-attribute-columns");
    let form_path = context.cwd.join("Form.xml");
    let json_path = context.cwd.join("edit.json");
    write_file(&form_path, editable_form_xml(false));
    write_file(
        &json_path,
        r#"{
  "attributes": [
    {
      "name": "ТаблицаДанных",
      "type": "ValueTable",
      "columns": [
        {"name": "Значение", "type": "string"},
        {"name": "Значение", "type": "string"}
      ]
    }
  ]
}
"#,
    );

    let mut args = Map::new();
    args.insert(
        "FormPath".to_string(),
        json!(form_path.display().to_string()),
    );
    args.insert(
        "JsonPath".to_string(),
        json!(json_path.display().to_string()),
    );

    let outcome = edit_form(&args, &context);
    assert!(!outcome.ok, "{outcome:?}");
    let stderr = outcome.stderr.unwrap_or_default();
    assert!(
        stderr.contains(
            "Duplicate column name 'Значение' in attribute 'ТаблицаДанных' edit definition"
        ),
        "{stderr}"
    );

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_post_write_failure_restores_the_exact_source_bytes() {
    let context = temp_context("edit-post-write-rollback");
    let form_path = context.cwd.join("Form.xml");
    write_file(&form_path, editable_form_xml(false));
    let original = fs::read(&form_path).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({"attributes": [{"name": "Added", "type": "String"}]}),
        ),
    ]);

    let outcome = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
        edit_form(&args, &context)
    });

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| error.contains("post-write validation")),
        "{outcome:?}"
    );
    assert_eq!(fs::read(&form_path).unwrap(), original);
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
pub(crate) fn edit_form_rejects_stale_preimage_without_overwriting_concurrent_change() {
    let context = temp_context("edit-stale-preimage");
    let form_path = context.cwd.join("Form.xml");
    let original = editable_contract_form(FORM_LOGFORM_NS, "");
    write_file(&form_path, &original);
    let concurrent = original
        .replace("</Form>", "\t<!-- concurrent change -->\n</Form>")
        .into_bytes();
    let hook_path = form_path.clone();
    let hook_bytes = concurrent.clone();
    let mut args = form_path_args(&form_path);
    args.insert(
        "definition".to_string(),
        json!({"attributes": [{"name": "Added", "type": "string"}]}),
    );

    let outcome = with_before_commit_hook(
        move |path| {
            assert_eq!(path, hook_path);
            fs::write(path, &hook_bytes).unwrap();
        },
        || edit_form(&args, &context),
    );

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome.errors.join("\n").contains("changed")
            || outcome.errors.join("\n").contains("preimage"),
        "{outcome:?}"
    );
    assert_eq!(fs::read(&form_path).unwrap(), concurrent);
    assert!(outcome.changes.is_empty(), "{outcome:?}");
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_surfaces_cleanup_warning_after_committed_validation() {
    let context = temp_context("edit-cleanup-warning");
    let form_path = context.cwd.join("Form.xml");
    write_file(&form_path, &editable_contract_form(FORM_LOGFORM_NS, ""));
    let mut args = form_path_args(&form_path);
    args.insert(
        "definition".to_string(),
        json!({"attributes": [{"name": "Added", "type": "string"}]}),
    );

    let outcome =
        with_publish_failpoints(&[PublishCheckpoint::Cleanup], || edit_form(&args, &context));

    assert!(outcome.ok, "{outcome:?}");
    assert!(outcome
        .warnings
        .iter()
        .any(|warning| warning.contains("injected publication cleanup failure")));
    assert!(fs::read_to_string(&form_path)
        .unwrap()
        .contains("name=\"Added\""));
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn auto_command_bar_preview_apply_and_repeat_are_safe() {
    let context = temp_context("auto-command-bar");
    let path = context.cwd.join("Form.xml");
    fs::write(&path, editable_form_xml(false)).unwrap();
    let args = Map::from_iter([
        ("FormPath".to_string(), json!(path.display().to_string())),
        (
            "definition".to_string(),
            json!({"elements":[{"button":"RunButton","command":"Refresh"}],"into":"ФормаКоманднаяПанель"}),
        ),
    ]);
    let before = unica_workspace::test_support::tree_snapshot(&context.cwd);
    let preview = preview_form_edit(&args, &context);
    assert!(preview.ok, "{preview:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&context.cwd),
        before
    );
    let applied = edit_form(&args, &context);
    assert!(applied.ok, "{applied:?}");
    let after = unica_workspace::test_support::tree_snapshot(&context.cwd);
    let repeated = edit_form(&args, &context);
    assert!(repeated.ok, "{repeated:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&context.cwd),
        after
    );
    let xml = fs::read_to_string(&path).unwrap();
    let doc = Document::parse(&xml).unwrap();
    let button = doc
        .descendants()
        .find(|n| n.attribute("name") == Some("RunButton"))
        .unwrap();
    assert_eq!(
        button
            .parent_element()
            .unwrap()
            .parent_element()
            .unwrap()
            .tag_name()
            .name(),
        "AutoCommandBar"
    );
    assert!(button
        .children()
        .any(|n| n.has_tag_name("Type") && n.text() == Some("CommandBarButton")));
    fs::remove_dir_all(&context.cwd).unwrap();
}

#[test]
fn edit_form_emits_button_bound_to_form_command() {
    let context = temp_context("edit-button");
    let form_path = context.cwd.join("Form.xml");
    let json_path = context.cwd.join("edit.json");
    write_file(&form_path, editable_form_xml(false));
    write_file(
        &json_path,
        r#"{
  "commands": [
    {"name": "RunParityAction", "title": "Run parity action", "action": "RunParityAction"}
  ],
  "elements": [
    {
      "button": "RunParityActionButton",
      "type": "commandBar",
      "command": "RunParityAction",
      "title": "Run parity action"
    }
  ]
}
"#,
    );

    let mut args = Map::new();
    args.insert(
        "FormPath".to_string(),
        json!(form_path.display().to_string()),
    );
    args.insert(
        "JsonPath".to_string(),
        json!(json_path.display().to_string()),
    );

    let outcome = edit_form(&args, &context);
    assert!(outcome.ok, "{outcome:?}");
    let updated = fs::read_to_string(&form_path).unwrap();
    assert!(
        updated.contains("<Button name=\"RunParityActionButton\""),
        "{updated}"
    );
    assert!(
        updated.contains("<Type>CommandBarButton</Type>"),
        "{updated}"
    );
    assert!(
        updated.contains("<CommandName>Form.Command.RunParityAction</CommandName>"),
        "{updated}"
    );
    assert!(
        updated.contains("<ExtendedTooltip name=\"RunParityActionButtonРасширеннаяПодсказка\""),
        "{updated}"
    );

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_adds_command_bar_button_into_existing_container() {
    let context = temp_context("edit-command-bar");
    let form_path = context.cwd.join("Form.xml");
    let json_path = context.cwd.join("edit.json");
    write_file(
        &form_path,
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/>
	<ChildItems>
		<CommandBar name="ПанельДействий" id="1">
			<ChildItems/>
		</CommandBar>
	</ChildItems>
	<Attributes/>
	<Commands/>
</Form>
"#,
    );
    write_file(
        &json_path,
        r#"{
  "into": "ПанельДействий",
  "elements": [
    {
      "button": "Заполнить",
      "type": "commandBar",
      "command": "Заполнить",
      "locationInCommandBar": "InAdditionalSubmenu"
    }
  ],
  "commands": [
    { "name": "Заполнить", "action": "ЗаполнитьОбработка" }
  ]
}
"#,
    );

    let mut args = Map::new();
    args.insert(
        "FormPath".to_string(),
        json!(form_path.display().to_string()),
    );
    args.insert(
        "JsonPath".to_string(),
        json!(json_path.display().to_string()),
    );

    let outcome = edit_form(&args, &context);
    assert!(outcome.ok, "{outcome:?}");
    let updated = fs::read_to_string(&form_path).unwrap();
    assert_eq!(
        updated
            .matches("<CommandBar name=\"ПанельДействий\"")
            .count(),
        1
    );
    assert_eq!(updated.matches("<Command name=\"Заполнить\"").count(), 1);
    assert!(updated.contains("<Button name=\"Заполнить\""), "{updated}");
    assert!(
        updated.contains("<Type>CommandBarButton</Type>"),
        "{updated}"
    );
    assert!(
        updated.contains("<CommandName>Form.Command.Заполнить</CommandName>"),
        "{updated}"
    );
    assert!(
        updated.contains("<LocationInCommandBar>InAdditionalSubmenu</LocationInCommandBar>"),
        "{updated}"
    );

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_creates_child_items_for_target_container() {
    let context = temp_context("edit-command-bar-no-child-items");
    let form_path = context.cwd.join("Form.xml");
    let json_path = context.cwd.join("edit.json");
    write_file(
        &form_path,
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/>
	<ChildItems>
		<CommandBar name="ПанельДействий" id="1"/>
	</ChildItems>
	<Attributes/>
	<Commands/>
</Form>
"#,
    );
    write_file(
        &json_path,
        r#"{
  "into": "ПанельДействий",
  "elements": [
    {
      "button": "Заполнить",
      "type": "commandBar",
      "command": "Заполнить"
    }
  ],
  "commands": [
    { "name": "Заполнить", "action": "ЗаполнитьОбработка" }
  ]
}
"#,
    );

    let mut args = Map::new();
    args.insert(
        "FormPath".to_string(),
        json!(form_path.display().to_string()),
    );
    args.insert(
        "JsonPath".to_string(),
        json!(json_path.display().to_string()),
    );

    let outcome = edit_form(&args, &context);
    assert!(outcome.ok, "{outcome:?}");
    let updated = fs::read_to_string(&form_path).unwrap();
    assert!(
        updated.contains("<CommandBar name=\"ПанельДействий\" id=\"1\">"),
        "{updated}"
    );
    assert!(
        updated.contains("\t\t\t<ChildItems>\n\t\t\t\t<Button name=\"Заполнить\""),
        "{updated}"
    );
    assert!(updated.contains("<Button name=\"Заполнить\""), "{updated}");

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_creates_child_items_after_self_closing_extended_tooltip() {
    let context = temp_context("edit-group-with-extended-tooltip");
    let form_path = context.cwd.join("Form.xml");
    let json_path = context.cwd.join("edit.json");
    write_file(
        &form_path,
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/>
	<ChildItems>
		<UsualGroup name="ГруппаЗамены" id="1">
			<ExtendedTooltip name="ГруппаЗаменыРасширеннаяПодсказка" id="2"/>
		</UsualGroup>
	</ChildItems>
	<Attributes/>
	<Commands/>
</Form>
"#,
    );
    write_file(
        &json_path,
        r#"{
  "into": "ГруппаЗамены",
  "elements": [
    { "table": "ТаблицаЗамены" }
  ]
}
"#,
    );

    let mut args = Map::new();
    args.insert(
        "FormPath".to_string(),
        json!(form_path.display().to_string()),
    );
    args.insert(
        "JsonPath".to_string(),
        json!(json_path.display().to_string()),
    );

    let outcome = edit_form(&args, &context);
    assert!(outcome.ok, "{outcome:?}");
    let updated = fs::read_to_string(&form_path).unwrap();
    let tooltip_pos = updated
        .find("<ExtendedTooltip name=\"ГруппаЗаменыРасширеннаяПодсказка\" id=\"2\"/>")
        .unwrap();
    let child_items_pos = updated[tooltip_pos..]
        .find("<ChildItems>")
        .map(|pos| tooltip_pos + pos)
        .unwrap();
    assert!(tooltip_pos < child_items_pos, "{updated}");
    assert!(
        updated.contains("<Table name=\"ТаблицаЗамены\""),
        "{updated}"
    );
    Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

    let validate_outcome = validate_form(&args, &context);
    assert!(validate_outcome.ok, "{validate_outcome:?}");

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_inserts_element_after_existing_element() {
    let context = temp_context("edit-after-element");
    let form_path = context.cwd.join("Form.xml");
    let json_path = context.cwd.join("edit.json");
    write_file(
        &form_path,
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/>
	<ChildItems>
		<Button name="ExistingButton" id="1">
			<ExtendedTooltip name="ExistingButtonTooltip" id="2"/>
		</Button>
	</ChildItems>
	<Attributes/>
	<Commands/>
</Form>
"#,
    );
    write_file(
        &json_path,
        r#"{
  "after": "ExistingButton",
  "elements": [
    {"button": "InsertedButton", "type": "commandBar"}
  ]
}
"#,
    );

    let mut args = Map::new();
    args.insert(
        "FormPath".to_string(),
        json!(form_path.display().to_string()),
    );
    args.insert(
        "JsonPath".to_string(),
        json!(json_path.display().to_string()),
    );

    let outcome = edit_form(&args, &context);
    assert!(outcome.ok, "{outcome:?}");
    let updated = fs::read_to_string(&form_path).unwrap();
    let existing_pos = updated.find("ExistingButton").unwrap();
    let inserted_pos = updated.find("InsertedButton").unwrap();
    assert!(existing_pos < inserted_pos, "{updated}");

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_rejects_duplicate_element_name() {
    let context = temp_context("edit-duplicate-element-command");
    let form_path = context.cwd.join("Form.xml");
    let json_path = context.cwd.join("edit.json");
    write_file(
        &form_path,
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.17">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/>
	<ChildItems>
		<Button name="Заполнить" id="1"/>
	</ChildItems>
	<Attributes/>
	<Commands>
		<Command name="Заполнить" id="2"/>
	</Commands>
</Form>
"#,
    );
    let original = fs::read_to_string(&form_path).unwrap();
    write_file(
        &json_path,
        r#"{
  "elements": [
    {"button": "Заполнить", "type": "commandBar"}
  ]
}
"#,
    );

    let mut args = Map::new();
    args.insert(
        "FormPath".to_string(),
        json!(form_path.display().to_string()),
    );
    args.insert(
        "JsonPath".to_string(),
        json!(json_path.display().to_string()),
    );

    let outcome = edit_form(&args, &context);
    assert!(!outcome.ok, "{outcome:?}");
    let stderr = outcome.stderr.unwrap_or_default();
    assert!(
        stderr.contains("Element 'Заполнить' already exists in form"),
        "{stderr}"
    );
    assert_eq!(fs::read_to_string(&form_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_rejects_into_target_outside_child_items_tree() {
    let context = temp_context("edit-into-command-name");
    let form_path = context.cwd.join("Form.xml");
    let json_path = context.cwd.join("edit.json");
    write_file(
        &form_path,
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.17">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/>
	<ChildItems/>
	<Attributes/>
	<Commands>
		<Command name="Заполнить" id="1"/>
	</Commands>
</Form>
"#,
    );
    let original = fs::read_to_string(&form_path).unwrap();
    write_file(
        &json_path,
        r#"{
  "into": "Заполнить",
  "elements": [
    {"button": "InsertedButton", "type": "commandBar"}
  ]
}
"#,
    );

    let mut args = Map::new();
    args.insert(
        "FormPath".to_string(),
        json!(form_path.display().to_string()),
    );
    args.insert(
        "JsonPath".to_string(),
        json!(json_path.display().to_string()),
    );

    let outcome = edit_form(&args, &context);
    assert!(!outcome.ok, "{outcome:?}");
    let stderr = outcome.stderr.unwrap_or_default();
    assert!(
        stderr.contains("Target group 'Заполнить' not found"),
        "{stderr}"
    );
    assert_eq!(fs::read_to_string(&form_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_rejects_nested_duplicate_element_name() {
    let context = temp_context("edit-nested-duplicate-element");
    let form_path = context.cwd.join("Form.xml");
    let json_path = context.cwd.join("edit.json");
    write_file(
        &form_path,
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.17">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/>
	<ChildItems>
		<Button name="Заполнить" id="1"/>
	</ChildItems>
	<Attributes/>
	<Commands/>
</Form>
"#,
    );
    let original = fs::read_to_string(&form_path).unwrap();
    write_file(
        &json_path,
        r#"{
  "elements": [
    {
      "cmdBar": "ПанельДействий",
      "children": [
        {"button": "Заполнить", "type": "commandBar"}
      ]
    }
  ]
}
"#,
    );

    let mut args = Map::new();
    args.insert(
        "FormPath".to_string(),
        json!(form_path.display().to_string()),
    );
    args.insert(
        "JsonPath".to_string(),
        json!(json_path.display().to_string()),
    );

    let outcome = edit_form(&args, &context);
    assert!(!outcome.ok, "{outcome:?}");
    let stderr = outcome.stderr.unwrap_or_default();
    assert!(
        stderr.contains("Element 'Заполнить' already exists in form"),
        "{stderr}"
    );
    assert_eq!(fs::read_to_string(&form_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_rejects_duplicate_element_name_inside_definition_tree() {
    let context = temp_context("edit-nested-duplicate-definition");
    let form_path = context.cwd.join("Form.xml");
    let json_path = context.cwd.join("edit.json");
    write_file(
        &form_path,
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.17">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/>
	<ChildItems/>
	<Attributes/>
	<Commands/>
</Form>
"#,
    );
    let original = fs::read_to_string(&form_path).unwrap();
    write_file(
        &json_path,
        r#"{
  "elements": [
    {
      "cmdBar": "ПанельДействий",
      "children": [
        {"button": "Заполнить", "type": "commandBar"},
        {"button": "Заполнить", "type": "commandBar"}
      ]
    }
  ]
}
"#,
    );

    let mut args = Map::new();
    args.insert(
        "FormPath".to_string(),
        json!(form_path.display().to_string()),
    );
    args.insert(
        "JsonPath".to_string(),
        json!(json_path.display().to_string()),
    );

    let outcome = edit_form(&args, &context);
    assert!(!outcome.ok, "{outcome:?}");
    let stderr = outcome.stderr.unwrap_or_default();
    assert!(
        stderr.contains("Element 'Заполнить' already exists in edit definition"),
        "{stderr}"
    );
    assert_eq!(fs::read_to_string(&form_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_keeps_child_items_self_closing_when_no_elements_are_emitted() {
    let context = temp_context("edit-empty-elements");
    let form_path = context.cwd.join("Form.xml");
    let json_path = context.cwd.join("edit.json");
    write_file(
        &form_path,
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.17">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/>
	<ChildItems/>
	<Attributes/>
	<Commands/>
</Form>
"#,
    );
    write_file(
        &json_path,
        r#"{
  "elements": [
    {"autoCmdBar": "IgnoredAutoCommandBar"}
  ]
}
"#,
    );

    let mut args = Map::new();
    args.insert(
        "FormPath".to_string(),
        json!(form_path.display().to_string()),
    );
    args.insert(
        "JsonPath".to_string(),
        json!(json_path.display().to_string()),
    );

    let outcome = edit_form(&args, &context);
    assert!(outcome.ok, "{outcome:?}");
    let updated = fs::read_to_string(&form_path).unwrap();
    assert!(updated.contains("<ChildItems/>"), "{updated}");
    assert!(
        !updated.contains("<ChildItems>\n\n</ChildItems>"),
        "{updated}"
    );

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_rejects_invalid_nested_element_enum_without_writing_file() {
    let context = temp_context("edit-invalid-generated-xml");
    let form_path = context.cwd.join("Form.xml");
    let json_path = context.cwd.join("edit.json");
    write_file(
        &form_path,
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.17">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/>
	<ChildItems/>
	<Attributes/>
	<Commands/>
</Form>
"#,
    );
    let original = fs::read_to_string(&form_path).unwrap();
    write_file(
        &json_path,
        r#"{
  "elements": [
    {
      "name": "Группа",
      "group": "Vertical",
      "children": [
        {
          "check": "ФлагПроверки",
          "checkBoxType": "Bad<Name"
        }
      ]
    }
  ]
}
"#,
    );

    let mut args = Map::new();
    args.insert(
        "FormPath".to_string(),
        json!(form_path.display().to_string()),
    );
    args.insert(
        "JsonPath".to_string(),
        json!(json_path.display().to_string()),
    );

    let outcome = edit_form(&args, &context);
    assert!(!outcome.ok, "{outcome:?}");
    let stderr = outcome.stderr.unwrap_or_default();
    assert!(stderr.contains("checkBoxType"), "{stderr}");
    assert!(stderr.contains("8.3.27"), "{stderr}");
    assert_eq!(fs::read_to_string(&form_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_rejects_duplicate_command_name() {
    let context = temp_context("edit-duplicate-command");
    let form_path = context.cwd.join("Form.xml");
    let json_path = context.cwd.join("edit.json");
    write_file(
        &form_path,
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.17">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/>
	<ChildItems/>
	<Attributes/>
	<Commands>
		<Command name="Заполнить" id="1"/>
	</Commands>
</Form>
"#,
    );
    let original = fs::read_to_string(&form_path).unwrap();
    write_file(
        &json_path,
        r#"{
  "commands": [
    {"name": "Заполнить", "action": "ЗаполнитьОбработка"}
  ]
}
"#,
    );

    let mut args = Map::new();
    args.insert(
        "FormPath".to_string(),
        json!(form_path.display().to_string()),
    );
    args.insert(
        "JsonPath".to_string(),
        json!(json_path.display().to_string()),
    );

    let outcome = edit_form(&args, &context);
    assert!(!outcome.ok, "{outcome:?}");
    let stderr = outcome.stderr.unwrap_or_default();
    assert!(
        stderr.contains("Command 'Заполнить' already exists in form"),
        "{stderr}"
    );
    assert_eq!(fs::read_to_string(&form_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_emits_std_command_with_multi_dot_item_path() {
    let context = temp_context("edit-button-std-command");
    let form_path = context.cwd.join("Form.xml");
    let json_path = context.cwd.join("edit.json");
    write_file(&form_path, editable_form_xml(false));
    write_file(
        &json_path,
        r#"{
  "elements": [
    {
      "button": "AddGroupButton",
      "type": "commandBar",
      "stdCommand": "Table.Group.Add"
    }
  ]
}
"#,
    );

    let mut args = Map::new();
    args.insert(
        "FormPath".to_string(),
        json!(form_path.display().to_string()),
    );
    args.insert(
        "JsonPath".to_string(),
        json!(json_path.display().to_string()),
    );

    let outcome = edit_form(&args, &context);
    assert!(outcome.ok, "{outcome:?}");
    let updated = fs::read_to_string(&form_path).unwrap();
    assert!(
        updated.contains("<CommandName>Form.Item.Table.Group.StandardCommand.Add</CommandName>"),
        "{updated}"
    );
    assert!(
        !updated.contains("<CommandName>Form.Item.Table.StandardCommand.Group.Add</CommandName>"),
        "{updated}"
    );

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn validate_form_checks_form_event_against_main_attribute_context() {
    let cases = [
        (Some("CatalogObject.Goods"), true, None),
        (
            Some("DataProcessorObject.EventProbe"),
            false,
            Some("FORM_EVENT_NOT_ALLOWED"),
        ),
        (Some("DynamicList"), false, Some("FORM_EVENT_NOT_ALLOWED")),
        (None, false, Some("FORM_EVENT_CONTEXT_UNKNOWN")),
    ];

    for (main_type, expected_ok, expected_code) in cases {
        let context = temp_context("validate-form-event-context");
        let form_path = context.cwd.join("Form.xml");
        write_file(
            &form_path,
            &event_form_xml(
                main_type,
                r#"\t<Events>\n\t\t<Event name="OnReadAtServer">OnReadAtServer</Event>\n\t</Events>\n"#,
                "",
                false,
            ),
        );
        let args = Map::from_iter([(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        )]);

        let outcome = validate_form(&args, &context);

        assert_eq!(outcome.ok, expected_ok, "{main_type:?}: {outcome:?}");
        if let Some(code) = expected_code {
            assert!(
                outcome.errors.iter().any(|error| error.contains(code)),
                "{main_type:?}: {:?}",
                outcome.errors
            );
        }
        let _ = fs::remove_dir_all(&context.cwd);
    }
}

#[test]
fn validate_form_reports_unresolved_borrowed_event_context_without_false_failure() {
    let context = temp_context("validate-borrowed-event-context");
    let form_path = context.cwd.join("Form.xml");
    write_file(
        &form_path,
        &event_form_xml(
            None,
            r#"\t<Events>\n\t\t<Event name="OnReadAtServer" callType="Before">OnReadAtServer</Event>\n\t</Events>\n"#,
            "",
            true,
        ),
    );
    let args = Map::from_iter([(
        "FormPath".to_string(),
        json!(form_path.display().to_string()),
    )]);

    let outcome = validate_form(&args, &context);

    assert!(outcome.ok, "{outcome:?}");
    let stdout = outcome.stdout.unwrap_or_default();
    assert!(stdout.contains("[WARN]"), "{stdout}");
    assert!(stdout.contains("FORM_EVENT_CONTEXT_UNKNOWN"), "{stdout}");
    assert!(stdout.contains("was not verified"), "{stdout}");

    let borrowed_reference = event_form_xml(
            None,
            r#"\t<Events>\n\t\t<Event name="OnReadAtServer" callType="Before">OnReadAtServer</Event>\n\t</Events>\n"#,
            "",
            true,
        )
        .replace(
            "\t<BaseForm version=\"2.20\"/>",
            "\t<BaseForm version=\"2.20\">Catalog.Goods.Form.ItemForm</BaseForm>",
        );
    write_file(&form_path, &borrowed_reference);
    let borrowed_reference_outcome = validate_form(&args, &context);
    assert!(
        borrowed_reference_outcome.ok,
        "{borrowed_reference_outcome:?}"
    );
    assert!(
        borrowed_reference_outcome
            .stdout
            .as_deref()
            .unwrap_or_default()
            .contains("was not verified"),
        "{borrowed_reference_outcome:?}"
    );

    let embedded_base_context = event_form_xml(
            None,
            r#"\t<Events>\n\t\t<Event name="OnReadAtServer" callType="Before">OnReadAtServer</Event>\n\t</Events>\n"#,
            "",
            true,
        )
        .replace(
            "\t<BaseForm version=\"2.20\"/>",
            concat!(
                "\t<BaseForm version=\"2.20\">\n",
                "\t\t<Attributes>\n",
                "\t\t\t<Attribute name=\"BaseObject\" id=\"1\">\n",
                "\t\t\t\t<Type><v8:Type>cfg:CatalogObject.Goods</v8:Type></Type>\n",
                "\t\t\t\t<MainAttribute>true</MainAttribute>\n",
                "\t\t\t</Attribute>\n",
                "\t\t</Attributes>\n",
                "\t</BaseForm>"
            ),
        );
    write_file(&form_path, &embedded_base_context);
    let embedded_base_outcome = validate_form(&args, &context);
    assert!(embedded_base_outcome.ok, "{embedded_base_outcome:?}");
    assert!(
        !embedded_base_outcome
            .stdout
            .as_deref()
            .unwrap_or_default()
            .contains("FORM_EVENT_CONTEXT_UNKNOWN"),
        "{embedded_base_outcome:?}"
    );

    for (index, malformed_context) in [
            event_form_xml(
                None,
                r#"\t<Events>\n\t\t<Event name="OnReadAtServer" callType="Before">OnReadAtServer</Event>\n\t</Events>\n"#,
                "",
                true,
            )
            .replace(
                "\t<Attributes/>",
                concat!(
                    "\t<Attributes>\n",
                    "\t\t<Attribute name=\"Object\" id=\"1\">\n",
                    "\t\t\t<Type/>\n",
                    "\t\t\t<MainAttribute>true</MainAttribute>\n",
                    "\t\t</Attribute>\n",
                    "\t</Attributes>"
                ),
            ),
            event_form_xml(
                None,
                r#"\t<Events>\n\t\t<Event name="OnReadAtServer" callType="Before">OnReadAtServer</Event>\n\t</Events>\n"#,
                "",
                true,
            )
            .replace(
                "\t<BaseForm version=\"2.20\"/>",
                concat!(
                    "\t<BaseForm version=\"2.20\">\n",
                    "\t\t<Attributes>\n",
                    "\t\t\t<Attribute name=\"BaseObject\" id=\"1\">\n",
                    "\t\t\t\t<Type/>\n",
                    "\t\t\t\t<MainAttribute>true</MainAttribute>\n",
                    "\t\t\t</Attribute>\n",
                    "\t\t</Attributes>\n",
                    "\t</BaseForm>"
                ),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            write_file(&form_path, &malformed_context);
            let invalid_context = validate_form(&args, &context);
            assert!(!invalid_context.ok, "case {index}: {invalid_context:?}");
            assert!(
                invalid_context
                    .errors
                    .iter()
                    .any(|error| error.contains("FORM_EVENT_CONTEXT_UNKNOWN")),
                "case {index}: {invalid_context:?}"
            );
            assert!(
                !invalid_context
                    .stdout
                    .as_deref()
                    .unwrap_or_default()
                    .contains("was not verified"),
                "case {index}: {invalid_context:?}"
            );
        }

    write_file(
        &form_path,
        &event_form_xml(
            None,
            r#"\t<Events>\n\t\t<Event name="OnReadAtServer" callType="after">OnReadAtServer</Event>\n\t</Events>\n"#,
            "",
            true,
        ),
    );
    let invalid_call_type = validate_form(&args, &context);
    assert!(!invalid_call_type.ok, "{invalid_call_type:?}");
    assert!(invalid_call_type
        .errors
        .iter()
        .any(|error| error.contains("FORM_EVENT_INVALID_CALL_TYPE")));

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn validate_form_rejects_event_for_wrong_element_kind_and_duplicates() {
    let context = temp_context("validate-element-events");
    let form_path = context.cwd.join("Form.xml");
    write_file(
        &form_path,
        &event_form_xml(
            Some("CatalogObject.Goods"),
            r#"\t<Events>\n\t\t<Event name="OnOpen">OnOpen</Event>\n\t\t<Event name="OnOpen">OnOpenAgain</Event>\n\t</Events>\n"#,
            r#"\t\t<InputField name="Name" id="1">\n\t\t\t<DataPath>Object.Name</DataPath>\n\t\t\t<Events>\n\t\t\t\t<Event name="OnCreateAtServer">NameOnCreateAtServer</Event>\n\t\t\t</Events>\n\t\t\t<ContextMenu name="NameContextMenu" id="2"/>\n\t\t\t<ExtendedTooltip name="NameExtendedTooltip" id="3"/>\n\t\t</InputField>\n"#,
            false,
        ),
    );
    let args = Map::from_iter([(
        "FormPath".to_string(),
        json!(form_path.display().to_string()),
    )]);

    let outcome = validate_form(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| error.contains("FORM_EVENT_DUPLICATE")),
        "{:?}",
        outcome.errors
    );
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| error.contains("FORM_EVENT_NOT_ALLOWED")),
        "{:?}",
        outcome.errors
    );

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn validate_form_rejects_table_event_without_direct_data_path() {
    let context = temp_context("validate-unbound-table-event");
    let form_path = context.cwd.join("Form.xml");
    write_file(
        &form_path,
        &event_form_xml(
            Some("CatalogObject.Goods"),
            "",
            concat!(
                "\t\t<Table name=\"Rows\" id=\"1\">\n",
                "\t\t\t<DataPath>   </DataPath>\n",
                "\t\t\t<Events>\n",
                "\t\t\t\t<Event name=\"Selection\">RowsSelection</Event>\n",
                "\t\t\t</Events>\n",
                "\t\t\t<ContextMenu name=\"RowsContextMenu\" id=\"2\"/>\n",
                "\t\t\t<AutoCommandBar name=\"RowsCommandBar\" id=\"3\"/>\n",
                "\t\t\t<ExtendedTooltip name=\"RowsTooltip\" id=\"4\"/>\n",
                "\t\t</Table>\n"
            ),
            false,
        ),
    );
    let args = Map::from_iter([(
        "FormPath".to_string(),
        json!(form_path.display().to_string()),
    )]);

    let outcome = validate_form(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome.errors.iter().any(|error| {
            error.contains("FORM_EVENT_NOT_ALLOWED") && error.contains("non-empty direct DataPath")
        }),
        "{:?}",
        outcome.errors
    );

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn validate_form_checks_root_command_bar_and_companion_event_owners() {
    let context = temp_context("validate-companion-events");
    let form_path = context.cwd.join("Form.xml");
    let xml = event_form_xml(
        Some("CatalogObject.Goods"),
        "",
        concat!(
            "\t\t<InputField name=\"Name\" id=\"1\">\n",
            "\t\t\t<ContextMenu name=\"NameContextMenu\" id=\"2\"/>\n",
            "\t\t\t<ExtendedTooltip name=\"NameTooltip\" id=\"3\">\n",
            "\t\t\t\t<Events><Event name=\"OnCreateAtServer\">BadTooltipEvent</Event></Events>\n",
            "\t\t\t</ExtendedTooltip>\n",
            "\t\t</InputField>\n"
        ),
        false,
    )
    .replace(
        "\t<AutoCommandBar name=\"FormCommandBar\" id=\"-1\"/>",
        concat!(
            "\t<AutoCommandBar name=\"FormCommandBar\" id=\"-1\">\n",
            "\t\t<Events><Event name=\"Click\">BadBarEvent</Event></Events>\n",
            "\t</AutoCommandBar>"
        ),
    );
    write_file(&form_path, &xml);
    let args = Map::from_iter([(
        "FormPath".to_string(),
        json!(form_path.display().to_string()),
    )]);

    let outcome = validate_form(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    let errors = outcome.errors.join("\n");
    assert!(errors.contains("FormCommandBar"), "{errors}");
    assert!(errors.contains("NameTooltip"), "{errors}");
    assert!(
        errors.matches("FORM_EVENT_NOT_ALLOWED").count() >= 2,
        "{errors}"
    );

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_adds_supported_inline_form_and_element_events() {
    let (context, form_path) = form_info_workspace("edit-events-inline");
    let original = event_form_xml(
        Some("CatalogObject.Goods"),
        "",
        r#"\t\t<InputField name="Name" id="1">\n\t\t\t<DataPath>Object.Name</DataPath>\n\t\t\t<ContextMenu name="NameContextMenu" id="2"/>\n\t\t\t<ExtendedTooltip name="NameExtendedTooltip" id="3"/>\n\t\t</InputField>\n"#,
        false,
    );
    write_file(&form_path, &original);
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({
                "formEvents": [
                    {"name": "OnCreateAtServer", "handler": "OnCreateAtServer"}
                ],
                "elementEvents": [
                    {"element": "Name", "name": "OnChange", "handler": "NameOnChange"}
                ]
            }),
        ),
    ]);

    let outcome = edit_form(&args, &context);

    assert!(outcome.ok, "{outcome:?}");
    assert_eq!(outcome.changes.len(), 1, "{outcome:?}");
    let updated = fs::read_to_string(&form_path).unwrap();
    assert_eq!(updated.matches("name=\"OnCreateAtServer\"").count(), 1);
    assert_eq!(updated.matches("name=\"OnChange\"").count(), 1);
    assert!(
        updated.contains("<Event name=\"OnChange\">NameOnChange</Event>"),
        "{updated}"
    );
    let validate_args = Map::from_iter([(
        "FormPath".to_string(),
        json!(form_path.display().to_string()),
    )]);
    let validation = validate_form(&validate_args, &context);
    assert!(validation.ok, "{validation:?}");
    let info = analyze_form_info_with_data(
        &validate_args,
        &context,
        &WorkspaceSupportStateReader::new(&context),
    );
    assert!(info.outcome.ok, "{:?}", info.outcome);
    let info_data = info.data.as_ref().expect("form.info answers with data");
    assert!(
        info_data
            .events
            .iter()
            .any(|event| event.name == "OnCreateAtServer" && event.handler == "OnCreateAtServer"),
        "{info_data:?}"
    );

    let _ = fs::remove_dir_all(&context.cwd);
}

/// A `<Command>` shaped like a platform dump with Title, ToolTip,
/// Shortcut, Picture, Action and Representation.

#[test]
fn form_edit_rejects_invalid_type_without_writing() {
    let context = temp_context("edit-invalid-8-3-27-type");
    let form_path = context.cwd.join("Form.xml");
    let original = event_form_xml(None, "", "", false).into_bytes();
    fs::write(&form_path, &original).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({"attributes": [{"name": "Broken", "type": "UnknownType"}]}),
        ),
    ]);

    let outcome = edit_form(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(outcome.changes.is_empty(), "{outcome:?}");
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| error.contains("UnknownType") && error.contains("8.3.27")),
        "{outcome:?}"
    );
    assert_eq!(fs::read(&form_path).unwrap(), original);
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_compile_rejects_element_enum_values_outside_8_3_27_without_writing() {
    let cases = [
        (
            "horizontalAlign",
            json!({"elements": [{"autoCmdBar": "Bar", "horizontalAlign": "Bogus"}]}),
        ),
        (
            "pagesRepresentation",
            json!({"elements": [{"pages": "Pages", "pagesRepresentation": "Bogus"}]}),
        ),
        (
            "currentRowUse",
            json!({"elements": [{"pages": "Pages", "currentRowUse": "Bogus"}]}),
        ),
        (
            "commandBarLocation",
            json!({"elements": [{"table": "Rows", "commandBarLocation": "Bogus"}]}),
        ),
        (
            "representation",
            json!({"elements": [{"table": "Rows", "representation": "Bogus"}]}),
        ),
        (
            "initialTreeView",
            json!({"elements": [{"table": "Rows", "initialTreeView": "Bogus"}]}),
        ),
        (
            "choiceFoldersAndItems",
            json!({"elements": [{
                "table": "Rows",
                "_dynList": true,
                "choiceFoldersAndItems": "Bogus"
            }]}),
        ),
        (
            "updateOnDataChange",
            json!({"elements": [{
                "table": "Rows",
                "_dynList": true,
                "updateOnDataChange": "Bogus"
            }]}),
        ),
        (
            "group",
            json!({"elements": [{"name": "Group", "group": "AlwaysVertical"}]}),
        ),
        (
            "behavior",
            json!({"elements": [{"group": "Group", "behavior": "Bogus"}]}),
        ),
        (
            "representation",
            json!({"elements": [{"group": "Group", "representation": "Bogus"}]}),
        ),
        (
            "currentRowUse",
            json!({"elements": [{"group": "Group", "currentRowUse": "Bogus"}]}),
        ),
        (
            "checkBoxType",
            json!({"elements": [{"check": "Check", "checkBoxType": "Bogus"}]}),
        ),
        (
            "titleLocation",
            json!({"elements": [{"check": "Check", "titleLocation": "Bogus"}]}),
        ),
        (
            "titleLocation",
            json!({"elements": [{"input": "Input", "titleLocation": "Bogus"}]}),
        ),
        (
            "type",
            json!({"elements": [{"button": "Button", "type": "Bogus"}]}),
        ),
        (
            "representation",
            json!({"elements": [{"button": "Button", "representation": "Bogus"}]}),
        ),
        (
            "locationInCommandBar",
            json!({"elements": [{"button": "Button", "locationInCommandBar": "Bogus"}]}),
        ),
    ];

    for (index, (field, definition)) in cases.into_iter().enumerate() {
        let context = temp_context(&format!("compile-invalid-element-enum-{index}"));
        let definition_path = context.cwd.join("form.json");
        let form_path = context.cwd.join("Form.xml");
        let original = b"do-not-replace-invalid-form";
        write_file(
            &definition_path,
            &serde_json::to_string(&definition).unwrap(),
        );
        fs::write(&form_path, original).unwrap();
        let args = Map::from_iter([
            (
                "JsonPath".to_string(),
                json!(definition_path.display().to_string()),
            ),
            (
                "OutputPath".to_string(),
                json!(form_path.display().to_string()),
            ),
        ]);

        let outcome = compile_form(&args, &context);

        assert!(!outcome.ok, "{field}: {outcome:?}");
        assert!(
            outcome.errors.iter().any(|error| {
                error.contains(field)
                    && error.contains("8.3.27")
                    && (error.contains("Bogus") || error.contains("AlwaysVertical"))
            }),
            "{field}: {outcome:?}"
        );
        assert!(outcome.changes.is_empty(), "{field}: {outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{field}: {outcome:?}");
        assert_eq!(fs::read(&form_path).unwrap(), original, "{field}");

        let _ = fs::remove_dir_all(&context.cwd);
    }
}

#[test]
fn form_compile_rejects_markup_in_each_element_enum_family_without_writing() {
    let cases = [
        (
            "representation",
            json!({"elements": [{
                "group": "Group",
                "representation": "</Representation><Injected/>"
            }]}),
        ),
        (
            "checkBoxType",
            json!({"elements": [{
                "check": "Check",
                "checkBoxType": "</CheckBoxType><Injected/>"
            }]}),
        ),
        (
            "titleLocation",
            json!({"elements": [{
                "input": "Input",
                "titleLocation": "</TitleLocation><Injected/>"
            }]}),
        ),
        (
            "representation",
            json!({"elements": [{
                "button": "Button",
                "representation": "</Representation><Injected/>"
            }]}),
        ),
    ];

    for (index, (field, definition)) in cases.into_iter().enumerate() {
        let context = temp_context(&format!("compile-markup-element-enum-{index}"));
        let definition_path = context.cwd.join("form.json");
        let form_path = context.cwd.join("Form.xml");
        let original = b"do-not-replace-markup-form";
        write_file(
            &definition_path,
            &serde_json::to_string(&definition).unwrap(),
        );
        fs::write(&form_path, original).unwrap();
        let args = Map::from_iter([
            (
                "JsonPath".to_string(),
                json!(definition_path.display().to_string()),
            ),
            (
                "OutputPath".to_string(),
                json!(form_path.display().to_string()),
            ),
        ]);

        let outcome = compile_form(&args, &context);

        assert!(!outcome.ok, "{field}: {outcome:?}");
        assert!(
            outcome.errors.iter().any(|error| {
                error.contains(field) && error.contains("8.3.27") && error.contains("Injected")
            }),
            "{field}: {outcome:?}"
        );
        assert!(outcome.changes.is_empty(), "{field}: {outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{field}: {outcome:?}");
        assert_eq!(fs::read(&form_path).unwrap(), original, "{field}");

        let _ = fs::remove_dir_all(&context.cwd);
    }
}

#[test]
fn form_compile_rejects_xsd_invalid_attribute_columns_without_mutation() {
    let cases = [
        (
            "scalar-columns",
            json!({
                "attributes": [{
                    "name": "Scalar",
                    "type": "String",
                    "columns": [{"name": "Value", "type": "String"}]
                }]
            }),
            "columns are supported only for ValueTable or ValueTree",
        ),
        (
            "duplicate-columns",
            json!({
                "attributes": [{
                    "name": "Rows",
                    "type": "ValueTable",
                    "columns": [
                        {"name": "Value", "type": "String"},
                        {"name": "Value", "type": "Number"}
                    ]
                }]
            }),
            "Duplicate column name 'Value'",
        ),
        (
            "empty-column-name",
            json!({
                "attributes": [{
                    "name": "Rows",
                    "type": "ValueTree",
                    "columns": [{"name": " ", "type": "String"}]
                }]
            }),
            "requires non-empty name",
        ),
    ];

    for (case, definition, expected_error) in cases {
        let context = temp_context(&format!("compile-invalid-attribute-columns-{case}"));
        let definition_path = context.cwd.join("form.json");
        let form_path = context.cwd.join("Form.xml");
        let original =
            br#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"/>"#.to_vec();
        write_file(
            &definition_path,
            &serde_json::to_string(&definition).unwrap(),
        );
        fs::write(&form_path, &original).unwrap();
        let args = Map::from_iter([
            (
                "JsonPath".to_string(),
                json!(definition_path.display().to_string()),
            ),
            (
                "OutputPath".to_string(),
                json!(form_path.display().to_string()),
            ),
        ]);

        let outcome = compile_form(&args, &context);

        assert!(!outcome.ok, "{case}: {outcome:?}");
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains(expected_error)),
            "{case}: {outcome:?}"
        );
        assert_eq!(fs::read(&form_path).unwrap(), original, "{case}");
        assert!(outcome.changes.is_empty(), "{case}: {outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{case}: {outcome:?}");
        assert!(outcome.stdout.is_none(), "{case}: {outcome:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }
}

#[test]
fn form_compile_owner_read_failure_does_not_publish_new_or_replacement_form() {
    for existing_output in [false, true] {
        let context = temp_context(if existing_output {
            "compile-owner-read-failure-replace"
        } else {
            "compile-owner-read-failure-create"
        });
        let definition_path = context.cwd.join("form.json");
        let owner_path = context.cwd.join("src/Catalogs/Goods.xml");
        let output_path = context
            .cwd
            .join("src/Catalogs/Goods/Forms/ItemForm/Ext/Form.xml");
        write_file(&definition_path, "{}");
        fs::create_dir_all(&owner_path).unwrap();
        if existing_output {
            write_file(
                &output_path,
                r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"/>"#,
            );
        }
        let output_before = fs::read(&output_path).ok();
        let args = Map::from_iter([
            (
                "JsonPath".to_string(),
                json!(definition_path.display().to_string()),
            ),
            (
                "OutputPath".to_string(),
                json!(output_path.display().to_string()),
            ),
        ]);

        let outcome = compile_form(&args, &context);

        assert!(!outcome.ok, "existing={existing_output}: {outcome:?}");
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("form parent metadata owner is not a regular file")),
            "existing={existing_output}: {outcome:?}"
        );
        assert_eq!(
            fs::read(&output_path).ok(),
            output_before,
            "existing={existing_output}"
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }
}

#[test]
fn form_compile_directly_rejects_existing_wrong_root_without_write() {
    let context = temp_context("compile-existing-wrong-root");
    let definition_path = context.cwd.join("form.json");
    let output_path = context.cwd.join("Form.xml");
    write_file(&definition_path, "{}");
    let original = b"<garbage/>".to_vec();
    fs::write(&output_path, &original).unwrap();
    let args = Map::from_iter([
        (
            "JsonPath".to_string(),
            json!(definition_path.display().to_string()),
        ),
        (
            "OutputPath".to_string(),
            json!(output_path.display().to_string()),
        ),
    ]);

    let outcome = compile_form(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| error.contains("declared platform XML target root")),
        "{outcome:?}"
    );
    assert_eq!(fs::read(&output_path).unwrap(), original);
    assert!(outcome.changes.is_empty(), "{outcome:?}");
    assert!(outcome.artifacts.is_empty(), "{outcome:?}");
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_compile_post_write_failure_restores_owner_and_new_or_replacement_form() {
    for existing_output in [false, true] {
        let context = temp_context(if existing_output {
            "compile-post-write-replace"
        } else {
            "compile-post-write-create"
        });
        let definition_path = context.cwd.join("form.json");
        let owner_path = context.cwd.join("src/Catalogs/Goods.xml");
        let output_path = context
            .cwd
            .join("src/Catalogs/Goods/Forms/ItemForm/Ext/Form.xml");
        write_file(&definition_path, "{}");
        write_file(&owner_path, &empty_catalog_xml("\r\n", false));
        if existing_output {
            write_file(
                &output_path,
                r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"/>"#,
            );
        }
        let owner_before = fs::read(&owner_path).unwrap();
        let output_before = fs::read(&output_path).ok();
        let args = Map::from_iter([
            (
                "JsonPath".to_string(),
                json!(definition_path.display().to_string()),
            ),
            (
                "OutputPath".to_string(),
                json!(output_path.display().to_string()),
            ),
        ]);

        let outcome = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
            compile_form(&args, &context)
        });

        assert!(!outcome.ok, "existing={existing_output}: {outcome:?}");
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("post-write validation")),
            "existing={existing_output}: {outcome:?}"
        );
        assert_eq!(fs::read(&owner_path).unwrap(), owner_before);
        assert_eq!(
            fs::read(&output_path).ok(),
            output_before,
            "existing={existing_output}"
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }
}

#[test]
pub(crate) fn form_compile_rolls_back_if_unchanged_parent_owner_changes_during_publication() {
    let context = temp_context("compile-parent-owner-race");
    let source = context.cwd.join("src");
    let definition_path = context.cwd.join("form.json");
    let configuration_path = source.join("Configuration.xml");
    let owner_path = source.join("Catalogs/Goods.xml");
    let output_path = source.join("Catalogs/Goods/Forms/ItemForm/Ext/Form.xml");
    write_file(
        &context.cwd.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    );
    write_file(
            &configuration_path,
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><Configuration/></MetaDataObject>",
        );
    write_file(&definition_path, "{}");
    let owner = register_form_in_object_text(&empty_catalog_xml("\n", true), "ItemForm");
    write_file(&owner_path, &owner);
    let concurrent_owner = owner
        .replace(
            "</MetaDataObject>",
            "\t<!-- concurrent semantic owner change -->\n</MetaDataObject>",
        )
        .into_bytes();
    let owner_for_hook = owner_path.clone();
    let owner_bytes_for_hook = concurrent_owner.clone();
    let args = Map::from_iter([
        (
            "JsonPath".to_string(),
            json!(definition_path.display().to_string()),
        ),
        (
            "OutputPath".to_string(),
            json!(output_path.display().to_string()),
        ),
    ]);

    let outcome = with_before_commit_hook(
        move |_| fs::write(&owner_for_hook, &owner_bytes_for_hook).unwrap(),
        || compile_form(&args, &context),
    );

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome.errors.join("\n").contains("read guard"),
        "{outcome:?}"
    );
    assert_eq!(fs::read(&owner_path).unwrap(), concurrent_owner);
    assert!(!output_path.exists());
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_compile_rejects_supported_parent_owner_that_appears_after_probe() {
    let context = temp_context("compile-supported-parent-owner-appears");
    let definition_path = context.cwd.join("form.json");
    let owner_path = context.cwd.join("src/Catalogs/Goods.xml");
    let output_path = context
        .cwd
        .join("src/Catalogs/Goods/Forms/ItemForm/Ext/Form.xml");
    write_file(&definition_path, "{}");
    let supported_owner = empty_catalog_xml("\n", true).into_bytes();
    let owner_for_hook = owner_path.clone();
    let supported_for_hook = supported_owner.clone();
    let args = Map::from_iter([
        (
            "JsonPath".to_string(),
            json!(definition_path.display().to_string()),
        ),
        (
            "OutputPath".to_string(),
            json!(output_path.display().to_string()),
        ),
    ]);

    let outcome = with_form_compile_after_parent_owner_probe_hook(
        move |_| {
            fs::create_dir_all(owner_for_hook.parent().unwrap()).unwrap();
            fs::write(&owner_for_hook, &supported_for_hook).unwrap();
        },
        || compile_form(&args, &context),
    );

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome.errors.join("\n").contains("absence guard"),
        "{outcome:?}"
    );
    assert_eq!(fs::read(&owner_path).unwrap(), supported_owner);
    assert!(!output_path.exists());
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_compile_rejects_newer_parent_owner_that_appears_after_probe() {
    let context = temp_context("compile-parent-owner-appears");
    let definition_path = context.cwd.join("form.json");
    let owner_path = context.cwd.join("src/Catalogs/Goods.xml");
    let output_path = context
        .cwd
        .join("src/Catalogs/Goods/Forms/ItemForm/Ext/Form.xml");
    write_file(&definition_path, "{}");
    let newer_owner = empty_catalog_xml("\n", true)
        .replacen(r#"version="2.20""#, r#"version="2.21""#, 1)
        .into_bytes();
    let owner_for_hook = owner_path.clone();
    let newer_for_hook = newer_owner.clone();
    let args = Map::from_iter([
        (
            "JsonPath".to_string(),
            json!(definition_path.display().to_string()),
        ),
        (
            "OutputPath".to_string(),
            json!(output_path.display().to_string()),
        ),
    ]);

    let outcome = with_form_compile_after_parent_owner_probe_hook(
        move |_| {
            fs::create_dir_all(owner_for_hook.parent().unwrap()).unwrap();
            fs::write(&owner_for_hook, &newer_for_hook).unwrap();
        },
        || compile_form(&args, &context),
    );

    assert!(!outcome.ok, "{outcome:?}");
    assert!(outcome.errors.join("\n").contains("2.21"), "{outcome:?}");
    assert_eq!(fs::read(&owner_path).unwrap(), newer_owner);
    assert!(!output_path.exists());
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_compile_exact_binds_an_explicit_from_object_derivation_source() {
    let context = temp_context("compile-explicit-source-race");
    let object_path = context.cwd.join("src/Catalogs/Source.xml");
    let output_path = context
        .cwd
        .join("src/Catalogs/Target/Forms/ItemForm/Ext/Form.xml");
    let source = empty_catalog_xml("\n", true);
    write_file(&object_path, &source);
    let concurrent_source = source
        .replace(
            "</MetaDataObject>",
            "\t<!-- concurrent source change -->\n</MetaDataObject>",
        )
        .into_bytes();
    let source_for_hook = object_path.clone();
    let concurrent_for_hook = concurrent_source.clone();
    let args = Map::from_iter([
        ("FromObject".to_string(), json!(true)),
        (
            "ObjectPath".to_string(),
            json!(object_path.display().to_string()),
        ),
        (
            "OutputPath".to_string(),
            json!(output_path.display().to_string()),
        ),
    ]);

    let outcome = with_before_commit_hook(
        move |_| fs::write(&source_for_hook, &concurrent_for_hook).unwrap(),
        || compile_form(&args, &context),
    );

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome.errors.join("\n").contains("read guard"),
        "{outcome:?}"
    );
    assert_eq!(fs::read(&object_path).unwrap(), concurrent_source);
    assert!(!output_path.exists());
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_compile_rejects_platform_invalid_parent_owner_before_writing_or_registering() {
    let context = temp_context("compile-invalid-parent-owner");
    let definition_path = context.cwd.join("form.json");
    let owner_path = context.cwd.join("src/Catalogs/Goods.xml");
    let output_path = context
        .cwd
        .join("src/Catalogs/Goods/Forms/CompiledForm/Ext/Form.xml");
    write_file(&definition_path, "{}");
    let invalid_owner = empty_catalog_xml("\n", true).replace(
        "\t\t\t<DefaultListForm/>",
        "\t\t\t<DefaultListForm/>\n\t\t\t<IncludeHelpInContents>truthy</IncludeHelpInContents>",
    );
    write_file(&owner_path, &invalid_owner);
    let owner_before = fs::read(&owner_path).unwrap();
    let args = Map::from_iter([
        (
            "JsonPath".to_string(),
            json!(definition_path.display().to_string()),
        ),
        (
            "OutputPath".to_string(),
            json!(output_path.display().to_string()),
        ),
    ]);

    let outcome = compile_form(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("IncludeHelpInContents")
                    && error.contains("true or false")),
            "{outcome:?}"
        );
    assert_eq!(fs::read(&owner_path).unwrap(), owner_before);
    assert!(!output_path.exists());
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_compile_standalone_does_not_require_a_metadata_owner() {
    let context = temp_context("compile-standalone-without-owner");
    let definition_path = context.cwd.join("form.json");
    let output_path = context.cwd.join("StandaloneForm.xml");
    write_file(&definition_path, "{}");
    let args = Map::from_iter([
        (
            "JsonPath".to_string(),
            json!(definition_path.display().to_string()),
        ),
        (
            "OutputPath".to_string(),
            json!(output_path.display().to_string()),
        ),
    ]);

    let outcome = compile_form(&args, &context);

    assert!(outcome.ok, "{outcome:?}");
    assert!(output_path.is_file());
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_compile_rejects_unsafe_output_derived_form_name_without_writing() {
    for (case, relative_output) in [
        ("markup", "src/Catalogs/Goods/Forms/Bad&Name/Ext/Form.xml"),
        (
            "traversal",
            "src/Catalogs/Goods/Forms/../Escaped/Ext/Form.xml",
        ),
    ] {
        let context = temp_context(&format!("compile-unsafe-form-name-{case}"));
        let definition_path = context.cwd.join("form.json");
        let owner_path = context.cwd.join("src/Catalogs/Goods.xml");
        let output_path = context.cwd.join(relative_output);
        write_file(&definition_path, "{}");
        write_file(&owner_path, &empty_catalog_xml("\n", true));
        let owner_before = fs::read(&owner_path).unwrap();
        let args = Map::from_iter([
            (
                "JsonPath".to_string(),
                json!(definition_path.display().to_string()),
            ),
            (
                "OutputPath".to_string(),
                json!(output_path.display().to_string()),
            ),
        ]);

        let outcome = compile_form(&args, &context);

        assert!(!outcome.ok, "{case}: {outcome:?}");
        assert!(outcome.errors.iter().any(|error| {
            error.contains("OutputPath")
                && error.contains("XML NCName")
                && error.contains("single path component")
        }));
        assert_eq!(fs::read(&owner_path).unwrap(), owner_before, "{case}");
        assert!(!output_path.exists(), "{case}: {}", output_path.display());
        let _ = fs::remove_dir_all(&context.cwd);
    }
}

#[test]
fn form_compile_rejects_nested_binding_path_with_missing_attribute_before_write() {
    let context = temp_context("compile-missing-data-path-root");
    let definition_path = context.cwd.join("form.json");
    let output_path = context.cwd.join("generated/CorpusForm/Ext/Form.xml");
    write_file(
        &definition_path,
        &serde_json::to_string(&json!({
            "attributes": [
                {"name": "Object", "type": "CatalogObject.CorpusCatalog", "main": true},
                {"name": "Rows", "type": "ValueTable"}
            ],
            "elements": [{
                "group": "Body",
                "children": [{
                    "table": "Rows",
                    "path": "Rows",
                    "rowPictureDataPath": "Missing.Picture",
                    "columns": [{"input": "Value", "path": "Rows.Value"}]
                }]
            }]
        }))
        .unwrap(),
    );
    let args = Map::from_iter([
        (
            "JsonPath".to_string(),
            json!(definition_path.display().to_string()),
        ),
        (
            "OutputPath".to_string(),
            json!(output_path.display().to_string()),
        ),
    ]);

    let outcome = compile_form(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    let errors = outcome.errors.join("\n");
    assert!(errors.contains("Missing.Picture"), "{errors}");
    assert!(errors.contains("Missing"), "{errors}");
    assert!(!output_path.exists(), "{outcome:?}");
    assert!(!context.cwd.join("generated").exists(), "{outcome:?}");
    assert!(outcome.changes.is_empty(), "{outcome:?}");
    assert!(outcome.artifacts.is_empty(), "{outcome:?}");

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn html_document_field_edit_refuses_each_exhausted_allocation_without_writing() {
    let context = temp_context("html-exhausted-ids");
    let form_path = context.cwd.join("Form.xml");
    for (highest, allocation) in [
        (usize::MAX, "HTMLDocumentField"),
        (usize::MAX - 1, "ContextMenu"),
        (usize::MAX - 2, "ExtendedTooltip"),
    ] {
        // A known input kind isolates HTML emission from the imported-kind scan.
        let original =
            form_edit_remove_test_xml(&format!(r#"<InputField name="Imported" id="{highest}"/>"#));
        fs::write(&form_path, &original).unwrap();
        let args = Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            (
                "definition".to_string(),
                json!({"elements": [{"html": "AddedHtml"}]}),
            ),
        ]);
        let before = unica_workspace::test_support::tree_snapshot(&context.cwd);
        for edit in [preview_form_edit, edit_form] {
            let outcome = edit(&args, &context);
            assert!(!outcome.ok, "{allocation}: {outcome:?}");
            assert!(
                outcome
                    .errors
                    .iter()
                    .any(|error| error.contains("Form ID space exhausted")),
                "{allocation}: {outcome:?}"
            );
            assert!(outcome.changes.is_empty(), "{allocation}: {outcome:?}");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&context.cwd),
                before
            );
        }
    }
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn html_document_field_edit_reserves_imported_ids_in_managed_and_extension_forms() {
    let context = temp_context("html-imported-ids");
    let form_path = context.cwd.join("Form.xml");
    for extension in [false, true] {
        let highest = if extension { 1_000_000usize } else { 5 };
        let imported = format!(
            r#"<UsualGroup name="Container" id="1"><ExtendedTooltip name="ContainerTooltip" id="2"/><ChildItems><HTMLDocumentField name="Imported" id="{highest}"><ContextMenu name="ImportedMenu" id="3"/><ExtendedTooltip name="ImportedTooltip" id="4"/></HTMLDocumentField></ChildItems></UsualGroup>"#
        );
        let original = editable_form_xml(extension).replace(
            "\t<ChildItems>\n\t</ChildItems>",
            &format!("<ChildItems>{imported}</ChildItems>"),
        );
        fs::write(&form_path, &original).unwrap();
        let args = Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            (
                "definition".to_string(),
                json!({
                    "attributes": [{"name": "HtmlSource", "type": "string"}],
                    "elements": [{"html": "AddedHtml", "path": "HtmlSource"}]
                }),
            ),
        ]);
        let before = unica_workspace::test_support::tree_snapshot(&context.cwd);
        let preview = preview_form_edit(&args, &context);
        assert!(preview.ok, "extension={extension}: {preview:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&context.cwd),
            before
        );
        let applied = edit_form(&args, &context);
        assert!(applied.ok, "extension={extension}: {applied:?}");
        let updated = fs::read_to_string(&form_path).unwrap();
        let document = Document::parse(&updated).unwrap();
        let added = document
            .descendants()
            .find(|node| node.attribute("name") == Some("AddedHtml"))
            .unwrap();
        let ids = added
            .descendants()
            .filter_map(|node| node.attribute("id"))
            .map(|id| id.parse::<usize>().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(ids, [highest + 1, highest + 2, highest + 3]);
        let ui_ids = document
            .descendants()
            .filter(|node| {
                node.ancestors()
                    .any(|ancestor| ancestor.has_tag_name("ChildItems"))
            })
            .filter_map(|node| node.attribute("id"))
            .collect::<Vec<_>>();
        assert_eq!(ui_ids.len(), ui_ids.iter().collect::<HashSet<_>>().len());
        assert!(
            updated.contains(&imported),
            "imported subtree changed: {updated}"
        );
    }
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_compile_emits_root_events_from_json_map_and_passes_validation() {
    let context = temp_context("compile-root-events");
    let definition_path = context.cwd.join("form.json");
    let form_path = context.cwd.join("Form.xml");
    write_file(
        &definition_path,
        r#"{"events":{"OnCreateAtServer":"ПриСозданииНаСервере"}}"#,
    );
    let args = Map::from_iter([
        (
            "JsonPath".to_string(),
            json!(definition_path.display().to_string()),
        ),
        (
            "OutputPath".to_string(),
            json!(form_path.display().to_string()),
        ),
    ]);

    let outcome = compile_form(&args, &context);

    assert!(outcome.ok, "{outcome:?}");
    let xml = read_utf8_sig(&form_path).unwrap();
    assert!(xml.contains(r#"version="2.20""#), "{xml}");
    assert!(!xml.contains(r#"version="2.17""#), "{xml}");
    assert_eq!(xml.matches("<Events>").count(), 1, "{xml}");
    assert_eq!(xml.matches("name=\"OnCreateAtServer\"").count(), 1, "{xml}");
    assert!(
        xml.contains("<Event name=\"OnCreateAtServer\">ПриСозданииНаСервере</Event>"),
        "{xml}"
    );
    let validation_args = Map::from_iter([(
        "FormPath".to_string(),
        json!(form_path.display().to_string()),
    )]);
    let validation = validate_form(&validation_args, &context);
    assert!(validation.ok, "{validation:?}");

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_compile_rejects_invalid_root_events_before_writing() {
    let context = temp_context("compile-invalid-root-events");
    let definition_path = context.cwd.join("form.json");
    let form_path = context.cwd.join("Form.xml");
    let original = b"do-not-replace-invalid-form";
    let cases = [
        (
            "event outside root registry",
            json!({"events": {"Opening": "OnOpening"}}),
            "FORM_EVENT_NOT_ALLOWED",
        ),
        (
            "record event without main attribute",
            json!({"events": {"OnReadAtServer": "OnReadAtServer"}}),
            "FORM_EVENT_CONTEXT_UNKNOWN",
        ),
        (
            "record event with non-persistent main attribute",
            json!({
                "attributes": [{"name": "List", "type": "DynamicList", "main": true}],
                "events": {"OnReadAtServer": "OnReadAtServer"}
            }),
            "FORM_EVENT_NOT_ALLOWED",
        ),
        (
            "events payload is not a map",
            json!({"events": ["OnOpen"]}),
            "FORM_EVENT_NOT_ALLOWED",
        ),
        (
            "event handler is not a string",
            json!({"events": {"OnOpen": 42}}),
            "FORM_EVENT_EMPTY_HANDLER",
        ),
        (
            "map does not silently accept call type",
            json!({
                "events": {"OnOpen": {"handler": "OnOpen", "callType": "Before"}}
            }),
            "FORM_EVENT_NOT_ALLOWED",
        ),
    ];

    for (name, definition, expected_code) in cases {
        write_file(
            &definition_path,
            &serde_json::to_string(&definition).unwrap(),
        );
        fs::write(&form_path, original).unwrap();
        let args = Map::from_iter([
            (
                "JsonPath".to_string(),
                json!(definition_path.display().to_string()),
            ),
            (
                "OutputPath".to_string(),
                json!(form_path.display().to_string()),
            ),
        ]);

        let outcome = compile_form(&args, &context);

        assert!(!outcome.ok, "{name}: {outcome:?}");
        assert!(
            outcome.errors.join("\n").contains(expected_code),
            "{name}: {outcome:?}"
        );
        assert!(outcome.changes.is_empty(), "{name}: {outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{name}: {outcome:?}");
        assert!(outcome.stdout.is_none(), "{name}: {outcome:?}");
        assert_eq!(fs::read(&form_path).unwrap(), original, "{name}");
    }

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_compile_dry_run_rejects_invalid_root_event_without_writing() {
    let context = temp_context("compile-invalid-root-event-dry-run");
    let definition_path = context.cwd.join("form.json");
    let form_path = context.cwd.join("Form.xml");
    let original = b"do-not-replace-invalid-form";
    write_file(&definition_path, r#"{"events":{"Opening":"OnOpening"}}"#);
    fs::write(&form_path, original).unwrap();
    let args = Map::from_iter([
        (
            "JsonPath".to_string(),
            json!(definition_path.display().to_string()),
        ),
        (
            "OutputPath".to_string(),
            json!(form_path.display().to_string()),
        ),
    ]);

    let outcome = NativeOperationAdapter::invoke(
        "form-compile",
        "unica.form.compile",
        &args,
        &context,
        true,
        true,
    )
    .unwrap();

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| error.contains("FORM_EVENT_NOT_ALLOWED")),
        "{outcome:?}"
    );
    assert!(outcome.changes.is_empty(), "{outcome:?}");
    assert_eq!(fs::read(&form_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_compile_dry_run_plans_valid_root_event_without_writing() {
    let context = temp_context("compile-valid-root-event-dry-run");
    let definition_path = context.cwd.join("form.json");
    let form_path = context.cwd.join("Form.xml");
    let original = b"do-not-replace-valid-form-during-preview";
    write_file(
        &definition_path,
        r#"{"events":{"OnCreateAtServer":"OnCreateAtServer"}}"#,
    );
    fs::write(&form_path, original).unwrap();
    let args = Map::from_iter([
        (
            "JsonPath".to_string(),
            json!(definition_path.display().to_string()),
        ),
        (
            "OutputPath".to_string(),
            json!(form_path.display().to_string()),
        ),
    ]);

    let outcome = NativeOperationAdapter::invoke(
        "form-compile",
        "unica.form.compile",
        &args,
        &context,
        true,
        true,
    )
    .unwrap();

    assert!(outcome.ok, "{outcome:?}");
    assert_eq!(outcome.changes.len(), 1, "{outcome:?}");
    assert!(
        outcome.changes[0].contains("would update")
            && outcome.changes[0].contains(&form_path.display().to_string()),
        "{outcome:?}"
    );
    assert_eq!(fs::read(&form_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_emits_and_validates_new_table_and_column_events() {
    let context = temp_context("edit-new-element-events");
    let form_path = context.cwd.join("Form.xml");
    write_file(
        &form_path,
        &event_form_xml(Some("CatalogObject.Goods"), "", "", false),
    );
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({
                "attributes": [{"name": "Rows", "type": "ValueTable"}],
                "elements": [{
                    "table": "Rows",
                    "path": "Rows",
                    "commandBar": {"autofill": false},
                    "on": [{"event": "Selection", "handler": "RowsSelection"}],
                    "columns": [{
                        "labelField": "Description",
                        "on": [{
                            "event": "OnChange",
                            "handler": "DescriptionOnChange"
                        }]
                    }]
                }]
            }),
        ),
    ]);

    let outcome = edit_form(&args, &context);

    assert!(outcome.ok, "{outcome:?}");
    let updated = fs::read_to_string(&form_path).unwrap();
    assert!(
        updated.contains("<Event name=\"Selection\">RowsSelection</Event>"),
        "{updated}"
    );
    assert!(
        updated.contains("<Event name=\"OnChange\">DescriptionOnChange</Event>"),
        "{updated}"
    );
    let table_start = updated.find("<Table name=\"Rows\"").unwrap();
    let table_end = updated[table_start..].find("</Table>").unwrap() + table_start;
    let table_xml = &updated[table_start..table_end];
    let table_event_pos = table_xml.find("<Event name=\"Selection\"").unwrap();
    let child_items_pos = table_xml.find("<ChildItems>").unwrap();
    assert!(table_event_pos < child_items_pos, "{table_xml}");
    let validation = validate_form(&args, &context);
    assert!(validation.ok, "{validation:?}");

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_rejects_invalid_nested_new_element_events_atomically() {
    let cases = [
        json!({
            "elements": [{
                "table": "Rows",
                "columns": [{"labelField": "Description", "on": ["Opening"]}]
            }]
        }),
        json!({"elements": [{"name": "FallbackInput", "on": ["OnCurrentPageChange"]}]}),
        json!({"elements": [{"input": "Field", "button": "Button", "on": ["OnChange"]}]}),
        json!({"elements": [{"input": "Field", "on": {"event": "OnChange"}}]}),
        json!({"elements": [{"input": "Field", "handlers": "invalid", "on": ["OnChange"]}]}),
        json!({"elements": [{"input": "Field", "on": [{"event": "OnChange", "handler": 123}]}]}),
        json!({"elements": [{"input": "Field", "handlers": {"OnChange": 123}, "on": ["OnChange"]}]}),
        json!({"elements": [{"table": "Rows", "on": ["Selection"], "columns": []}]}),
    ];

    for (index, definition) in cases.into_iter().enumerate() {
        let context = temp_context(&format!("edit-invalid-new-event-{index}"));
        let form_path = context.cwd.join("Form.xml");
        let original = event_form_xml(Some("CatalogObject.Goods"), "", "", false).into_bytes();
        fs::write(&form_path, &original).unwrap();
        let args = Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            ("definition".to_string(), definition),
        ]);

        let outcome = edit_form(&args, &context);

        assert!(!outcome.ok, "case {index}: {outcome:?}");
        assert!(outcome.changes.is_empty(), "case {index}: {outcome:?}");
        assert_eq!(fs::read(&form_path).unwrap(), original, "case {index}");
        let _ = fs::remove_dir_all(&context.cwd);
    }
}

#[test]
fn edit_form_emits_constants_set_for_projected_object_event_context() {
    let context = temp_context("edit-projected-constants-set-context");
    let form_path = context.cwd.join("Form.xml");
    write_file(&form_path, &event_form_xml(None, "", "", false));
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({
                "attributes": [{
                    "name": "Constants",
                    "type": "ConstantsSet",
                    "main": true
                }],
                "formEvents": [{
                    "name": "OnReadAtServer",
                    "handler": "OnReadAtServer"
                }]
            }),
        ),
    ]);

    let outcome = edit_form(&args, &context);

    assert!(outcome.ok, "{outcome:?}");
    let updated = fs::read_to_string(&form_path).unwrap();
    assert!(
        updated.contains("<v8:Type>cfg:ConstantsSet</v8:Type>"),
        "{updated}"
    );
    assert!(!updated.contains("<v8:Type>ConstantsSet</v8:Type>"));
    let validation = validate_form(&args, &context);
    assert!(validation.ok, "{validation:?}");

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_rejects_empty_main_attribute_name_before_event_planning() {
    let context = temp_context("edit-empty-main-attribute-name");
    let form_path = context.cwd.join("Form.xml");
    let base_form = concat!(
        "\t<BaseForm version=\"2.20\">\n",
        "\t\t<Attributes>\n",
        "\t\t\t<Attribute name=\"BaseObject\" id=\"1\">\n",
        "\t\t\t\t<Type><v8:Type>cfg:CatalogObject.Base</v8:Type></Type>\n",
        "\t\t\t\t<MainAttribute>true</MainAttribute>\n",
        "\t\t\t</Attribute>\n",
        "\t\t</Attributes>\n",
        "\t</BaseForm>\n"
    );
    let original = event_form_xml(None, "", "", true)
        .replace("\t<BaseForm version=\"2.20\"/>\n", base_form)
        .into_bytes();
    fs::write(&form_path, &original).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({
                "attributes": [{
                    "name": "",
                    "type": "DataProcessorObject.Override",
                    "main": true
                }],
                "formEvents": [{
                    "name": "OnReadAtServer",
                    "handler": "OnReadAtServer"
                }]
            }),
        ),
    ]);

    let outcome = edit_form(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(outcome.changes.is_empty(), "{outcome:?}");
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| error.contains("Empty attribute name")),
        "{outcome:?}"
    );
    assert_eq!(fs::read(&form_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_projects_only_attributes_that_can_be_emitted() {
    let cases = [
        (
            json!({
                "attributes": [{
                    "name": "Object",
                    "type": "CatalogObject.Goods",
                    "main": true
                }],
                "formEvents": [{
                    "name": "OnReadAtServer",
                    "handler": "OnReadAtServer"
                }]
            }),
            true,
        ),
        (
            json!({
                "attributes": [{"type": "CatalogObject.Goods", "main": true}],
                "formEvents": [{
                    "name": "OnReadAtServer",
                    "handler": "OnReadAtServer"
                }]
            }),
            false,
        ),
    ];

    for (index, (definition, expected_ok)) in cases.into_iter().enumerate() {
        let context = temp_context(&format!("edit-projected-context-{index}"));
        let form_path = context.cwd.join("Form.xml");
        let original = event_form_xml(None, "", "", false).into_bytes();
        fs::write(&form_path, &original).unwrap();
        let args = Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            ("definition".to_string(), definition),
        ]);

        let outcome = edit_form(&args, &context);

        assert_eq!(outcome.ok, expected_ok, "case {index}: {outcome:?}");
        if expected_ok {
            let validation = validate_form(&args, &context);
            assert!(validation.ok, "case {index}: {validation:?}");
        } else {
            assert!(outcome
                .errors
                .iter()
                .any(|error| { error.contains("FORM_EVENT_CONTEXT_UNKNOWN") }));
            assert_eq!(fs::read(&form_path).unwrap(), original, "case {index}");
        }
        let _ = fs::remove_dir_all(&context.cwd);
    }

    let context = temp_context("edit-existing-unknown-main-context");
    let form_path = context.cwd.join("Form.xml");
    let original = event_form_xml(None, "", "", false)
        .replace(
            "\t<Attributes/>",
            concat!(
                "\t<Attributes>\n",
                "\t\t<Attribute name=\"Existing\" id=\"1\">\n",
                "\t\t\t<Type/>\n",
                "\t\t\t<MainAttribute>true</MainAttribute>\n",
                "\t\t</Attribute>\n",
                "\t</Attributes>"
            ),
        )
        .into_bytes();
    fs::write(&form_path, &original).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({
                "attributes": [{
                    "name": "Object",
                    "type": "CatalogObject.Goods",
                    "main": true
                }],
                "formEvents": [{
                    "name": "OnReadAtServer",
                    "handler": "OnReadAtServer"
                }]
            }),
        ),
    ]);

    let outcome = edit_form(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(outcome
        .errors
        .iter()
        .any(|error| error.contains("MainAttribute=true")));
    assert_eq!(fs::read(&form_path).unwrap(), original);
    let _ = fs::remove_dir_all(&context.cwd);

    let override_cases = [
        (
            "CatalogObject.Base",
            Some(json!("DynamicList")),
            Some("FORM_EVENT_NOT_ALLOWED"),
        ),
        ("DynamicList", Some(json!("CatalogObject.Override")), None),
        (
            "CatalogObject.Base",
            None,
            Some("FORM_EVENT_CONTEXT_UNKNOWN"),
        ),
        (
            "CatalogObject.Base",
            Some(Value::Null),
            Some("FORM_EVENT_CONTEXT_UNKNOWN"),
        ),
        (
            "CatalogObject.Base",
            Some(json!("")),
            Some("FORM_EVENT_CONTEXT_UNKNOWN"),
        ),
        (
            "CatalogObject.Base",
            Some(json!(42)),
            Some("FORM_EVENT_CONTEXT_UNKNOWN"),
        ),
    ];
    for (index, (base_type, added_type, expected_code)) in override_cases.into_iter().enumerate() {
        let context = temp_context(&format!("edit-base-context-override-{index}"));
        let form_path = context.cwd.join("Form.xml");
        let base_form = format!(
            concat!(
                "\t<BaseForm version=\"2.20\">\n",
                "\t\t<Attributes>\n",
                "\t\t\t<Attribute name=\"BaseObject\" id=\"1\">\n",
                "\t\t\t\t<Type><v8:Type>cfg:{base_type}</v8:Type></Type>\n",
                "\t\t\t\t<MainAttribute>true</MainAttribute>\n",
                "\t\t\t</Attribute>\n",
                "\t\t</Attributes>\n",
                "\t</BaseForm>\n"
            ),
            base_type = base_type
        );
        let original = event_form_xml(None, "", "", true)
            .replace("\t<BaseForm version=\"2.20\"/>\n", &base_form)
            .into_bytes();
        fs::write(&form_path, &original).unwrap();
        let mut projected_attribute = json!({
            "name": "Object",
            "main": true
        });
        if let Some(added_type) = added_type {
            projected_attribute["type"] = added_type;
        }
        let args = Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            (
                "definition".to_string(),
                json!({
                    "attributes": [projected_attribute],
                    "formEvents": [{
                        "name": "OnReadAtServer",
                        "handler": "OnReadAtServer",
                        "callType": "After"
                    }]
                }),
            ),
        ]);

        let outcome = edit_form(&args, &context);

        assert_eq!(
            outcome.ok,
            expected_code.is_none(),
            "case {index}: {outcome:?}"
        );
        if expected_code.is_none() {
            let validation = validate_form(&args, &context);
            assert!(validation.ok, "case {index}: {validation:?}");
        } else {
            let expected_code = expected_code.unwrap_or_default();
            assert!(
                outcome
                    .errors
                    .iter()
                    .any(|error| error.contains(expected_code)),
                "case {index}: {outcome:?}"
            );
            assert_eq!(fs::read(&form_path).unwrap(), original);
        }
        let _ = fs::remove_dir_all(&context.cwd);
    }
}

#[test]
fn edit_form_rejects_ambiguous_existing_element_target() {
    let context = temp_context("edit-ambiguous-event-target");
    let form_path = context.cwd.join("Form.xml");
    let children = concat!(
        "\t\t<InputField name=\"Duplicate\" id=\"1\"/>\n",
        "\t\t<InputField name=\"Duplicate\" id=\"2\"/>\n"
    );
    let original = event_form_xml(Some("CatalogObject.Goods"), "", children, false).into_bytes();
    fs::write(&form_path, &original).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({
                "elementEvents": [{
                    "element": "Duplicate",
                    "name": "OnChange",
                    "handler": "DuplicateOnChange"
                }]
            }),
        ),
    ]);

    let outcome = edit_form(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| error.contains("FORM_EVENT_DUPLICATE")),
        "{outcome:?}"
    );
    assert_eq!(fs::read(&form_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_event_success_preserves_source_byte_layout() {
    let context = temp_context("edit-event-byte-layout");
    let form_path = context.cwd.join("Form.xml");
    let text = event_form_xml(Some("CatalogObject.Goods"), "", "", false)
        .replace("encoding=\"utf-8\"", "encoding=\"UTF-8\"")
        .replace('\n', "\r\n")
        .trim_end_matches("\r\n")
        .to_string();
    let mut original = vec![0xef, 0xbb, 0xbf];
    original.extend_from_slice(text.as_bytes());
    fs::write(&form_path, &original).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({
                "formEvents": [{
                    "name": "OnCreateAtServer",
                    "handler": "OnCreateAtServer"
                }]
            }),
        ),
    ]);

    let outcome = edit_form(&args, &context);

    assert!(outcome.ok, "{outcome:?}");
    let updated = fs::read(&form_path).unwrap();
    assert!(updated.starts_with(&[0xef, 0xbb, 0xbf]));
    let content = &updated[3..];
    assert!(content.starts_with(b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n"));
    assert!(!content.ends_with(b"\n"));
    assert!(content
        .iter()
        .enumerate()
        .all(|(index, byte)| *byte != b'\n' || index > 0 && content[index - 1] == b'\r'));

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_rejects_self_closing_event_target_missing_required_companions() {
    let context = temp_context("edit-self-closing-event-targets");
    let form_path = context.cwd.join("Form.xml");
    let original = event_form_xml(
        Some("CatalogObject.Goods"),
        "\t<Events/>\n",
        "\t\t<InputField name=\"Name\" id=\"1\"/>\n",
        false,
    );
    write_file(&form_path, &original);
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({
                "formEvents": [{"name": "OnOpen", "handler": "OnOpen"}],
                "elementEvents": [{
                    "element": "Name",
                    "name": "OnChange",
                    "handler": "NameOnChange"
                }]
            }),
        ),
    ]);

    let outcome = edit_form(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(outcome.errors.iter().any(|error| {
        error.contains("InputField")
            && error.contains("Name")
            && error.contains("missing companion")
    }));
    assert!(outcome.changes.is_empty(), "{outcome:?}");
    assert_eq!(fs::read_to_string(&form_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_inserts_existing_pages_event_before_child_items() {
    let context = temp_context("edit-pages-event-order");
    let form_path = context.cwd.join("Form.xml");
    let children = concat!(
        "\t\t<Pages name=\"Tabs\" id=\"1\">\n",
        "\t\t\t<ExtendedTooltip name=\"TabsTooltip\" id=\"2\"/>\n",
        "\t\t\t<ChildItems>\n",
        "\t\t\t\t<Page name=\"MainPage\" id=\"3\">\n",
        "\t\t\t\t\t<ExtendedTooltip name=\"MainPageTooltip\" id=\"4\"/>\n",
        "\t\t\t\t</Page>\n",
        "\t\t\t</ChildItems>\n",
        "\t\t</Pages>\n"
    );
    write_file(
        &form_path,
        &event_form_xml(Some("CatalogObject.Goods"), "", children, false),
    );
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({
                "elementEvents": [{
                    "element": "Tabs",
                    "name": "OnCurrentPageChange",
                    "handler": "TabsOnCurrentPageChange"
                }]
            }),
        ),
    ]);

    let outcome = edit_form(&args, &context);

    assert!(outcome.ok, "{outcome:?}");
    let updated = fs::read_to_string(&form_path).unwrap();
    let event_pos = updated.find("<Events>").unwrap();
    let child_items_pos = updated.rfind("<ChildItems>").unwrap();
    assert!(event_pos < child_items_pos, "{updated}");
    let validation = validate_form(&args, &context);
    assert!(validation.ok, "{validation:?}");

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_inserts_existing_table_event_before_child_items() {
    let context = temp_context("edit-table-event-order");
    let form_path = context.cwd.join("Form.xml");
    let children = concat!(
        "\t\t<Table name=\"Rows\" id=\"1\">\n",
        "\t\t\t<DataPath>Object.Rows</DataPath>\n",
        "\t\t\t<ContextMenu name=\"RowsContextMenu\" id=\"2\"/>\n",
        "\t\t\t<AutoCommandBar name=\"RowsCommandBar\" id=\"3\"/>\n",
        "\t\t\t<ExtendedTooltip name=\"RowsTooltip\" id=\"4\"/>\n",
        "\t\t\t<SearchStringAddition name=\"RowsSearchString\" id=\"7\"/>\n",
        "\t\t\t<ViewStatusAddition name=\"RowsViewStatus\" id=\"8\"/>\n",
        "\t\t\t<SearchControlAddition name=\"RowsSearchControl\" id=\"9\"/>\n",
        "\t\t\t<ChildItems>\n",
        "\t\t\t\t<LabelField name=\"Description\" id=\"5\">\n",
        "\t\t\t\t\t<ContextMenu name=\"DescriptionContextMenu\" id=\"10\"/>\n",
        "\t\t\t\t\t<ExtendedTooltip name=\"DescriptionTooltip\" id=\"6\"/>\n",
        "\t\t\t\t</LabelField>\n",
        "\t\t\t</ChildItems>\n",
        "\t\t</Table>\n"
    );
    write_file(
        &form_path,
        &event_form_xml(Some("CatalogObject.Goods"), "", children, false),
    );
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({
                "elementEvents": [{
                    "element": "Rows",
                    "name": "Selection",
                    "handler": "RowsSelection"
                }]
            }),
        ),
    ]);

    let outcome = edit_form(&args, &context);

    assert!(outcome.ok, "{outcome:?}");
    let updated = fs::read_to_string(&form_path).unwrap();
    let table_start = updated.find("<Table name=\"Rows\"").unwrap();
    let table_end = updated[table_start..].find("</Table>").unwrap() + table_start;
    let table_xml = &updated[table_start..table_end];
    let event_pos = table_xml.find("<Event name=\"Selection\"").unwrap();
    let child_items_pos = table_xml.find("<ChildItems>").unwrap();
    assert!(event_pos < child_items_pos, "{table_xml}");

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_rejects_unbound_table_event_without_byte_changes() {
    let context = temp_context("edit-unbound-table-event");
    let form_path = context.cwd.join("Form.xml");
    let original = event_form_xml(
        Some("CatalogObject.Goods"),
        "",
        concat!(
            "\t\t<Table name=\"Rows\" id=\"1\">\n",
            "\t\t\t<ContextMenu name=\"RowsContextMenu\" id=\"2\"/>\n",
            "\t\t\t<AutoCommandBar name=\"RowsCommandBar\" id=\"3\"/>\n",
            "\t\t\t<ExtendedTooltip name=\"RowsTooltip\" id=\"4\"/>\n",
            "\t\t</Table>\n"
        ),
        false,
    )
    .into_bytes();
    fs::write(&form_path, &original).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({
                "elementEvents": [{
                    "element": "Rows",
                    "name": "Selection",
                    "handler": "RowsSelection"
                }]
            }),
        ),
    ]);

    let outcome = edit_form(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(outcome.changes.is_empty(), "{outcome:?}");
    assert!(
        outcome.errors.iter().any(|error| {
            error.contains("FORM_EVENT_NOT_ALLOWED") && error.contains("non-empty direct DataPath")
        }),
        "{:?}",
        outcome.errors
    );
    assert_eq!(fs::read(&form_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_rejects_mixed_event_batch_without_serializer_diff() {
    let context = temp_context("edit-events-rollback");
    let form_path = context.cwd.join("Form.xml");
    let original = event_form_xml(Some("DataProcessorObject.EventProbe"), "", "", false)
        .replace("encoding=\"utf-8\"", "encoding=\"UTF-8\"")
        .replace('\n', "\r\n")
        .trim_end_matches("\r\n")
        .to_string();
    write_file(&form_path, &original);
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            serde_json::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/unica_mcp_script_parity/form-edit/invalid-events.json"
            )))
            .unwrap(),
        ),
    ]);

    let outcome = edit_form(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| error.contains("FORM_EVENT_NOT_ALLOWED")),
        "{:?}",
        outcome.errors
    );
    assert!(outcome.changes.is_empty(), "{outcome:?}");
    assert_eq!(fs::read_to_string(&form_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
pub(crate) fn edit_form_identical_event_is_byte_and_identity_exact_idempotent_noop() {
    use unica_workspace::test_support::file_identity_for_test;

    let context = temp_context("edit-event-idempotent");
    let form_path = context.cwd.join("Form.xml");
    let original = event_form_xml(
        Some("CatalogObject.Goods"),
        r#"\t<Events>\n\t\t<Event name="OnCreateAtServer">OnCreateAtServer</Event>\n\t</Events>\n"#,
        "",
        false,
    )
    .replace("encoding=\"utf-8\"", "encoding=\"UTF-8\"")
    .replace('\n', "\r\n")
    .trim_end_matches("\r\n")
    .to_string();
    write_file(&form_path, &original);
    let identity = file_identity_for_test(&form_path).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({
                "formEvents": [
                    {"name": "OnCreateAtServer", "handler": "OnCreateAtServer"}
                ]
            }),
        ),
    ]);

    let outcome = edit_form(&args, &context);

    assert!(outcome.ok, "{outcome:?}");
    assert!(outcome.changes.is_empty(), "{outcome:?}");
    assert!(
        outcome.summary.contains("no-op")
            || outcome
                .stdout
                .as_deref()
                .is_some_and(|stdout| stdout.contains("no-op")),
        "{outcome:?}"
    );
    assert_eq!(fs::read_to_string(&form_path).unwrap(), original);
    assert_eq!(file_identity_for_test(&form_path).unwrap(), identity);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_rejects_conflicting_event_binding_without_byte_changes() {
    let context = temp_context("edit-event-conflict");
    let form_path = context.cwd.join("Form.xml");
    let original = event_form_xml(
        Some("CatalogObject.Goods"),
        r#"\t<Events>\n\t\t<Event name="OnCreateAtServer">ExistingHandler</Event>\n\t</Events>\n"#,
        "",
        false,
    )
    .into_bytes();
    fs::write(&form_path, &original).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({
                "formEvents": [
                    {"name": "OnCreateAtServer", "handler": "DifferentHandler"}
                ]
            }),
        ),
    ]);

    let outcome = edit_form(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(outcome.changes.is_empty(), "{outcome:?}");
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| error.contains("FORM_EVENT_BINDING_CONFLICT")),
        "{:?}",
        outcome.errors
    );
    assert_eq!(fs::read(&form_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_rejects_missing_element_event_target_atomically() {
    let context = temp_context("edit-event-missing-target");
    let form_path = context.cwd.join("Form.xml");
    let original = event_form_xml(Some("CatalogObject.Goods"), "", "", false).into_bytes();
    fs::write(&form_path, &original).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({
                "elementEvents": [{
                    "element": "MissingField",
                    "name": "OnChange",
                    "handler": "MissingFieldOnChange"
                }]
            }),
        ),
    ]);

    let outcome = edit_form(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(outcome.changes.is_empty(), "{outcome:?}");
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| error.contains("FORM_EVENT_TARGET_NOT_FOUND")),
        "{:?}",
        outcome.errors
    );
    assert_eq!(fs::read(&form_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn validate_form_enforces_event_call_type_scope_for_root_and_element_events() {
    let child_events = r#"\t\t<InputField name="Name" id="1">\n\t\t\t<DataPath>Object.Name</DataPath>\n\t\t\t<Events>\n\t\t\t\t<Event name="OnChange" callType="Override">NameOnChange</Event>\n\t\t\t</Events>\n\t\t\t<ContextMenu name="NameContextMenu" id="2"/>\n\t\t\t<ExtendedTooltip name="NameExtendedTooltip" id="3"/>\n\t\t</InputField>\n"#;
    let cases = [
        (
            false,
            r#"\t<Events>\n\t\t<Event name="OnOpen" callType="After">OnOpen</Event>\n\t</Events>\n"#,
            child_events,
            false,
            Some("FORM_EVENT_CALL_TYPE_NOT_ALLOWED"),
        ),
        (
            true,
            r#"\t<Events>\n\t\t<Event name="OnOpen" callType="Before">OnOpen</Event>\n\t</Events>\n"#,
            child_events,
            true,
            None,
        ),
        (
            true,
            r#"\t<Events>\n\t\t<Event name="OnOpen" callType="after">OnOpen</Event>\n\t</Events>\n"#,
            "",
            false,
            Some("FORM_EVENT_INVALID_CALL_TYPE"),
        ),
        (
            true,
            r#"\t<Events>\n\t\t<Event name="OnOpen" callType="">OnOpen</Event>\n\t</Events>\n"#,
            "",
            false,
            Some("FORM_EVENT_INVALID_CALL_TYPE"),
        ),
    ];

    for (extension, form_events, child_items, expected_ok, expected_code) in cases {
        let context = temp_context("validate-event-call-type");
        let form_path = context.cwd.join("Form.xml");
        write_file(
            &form_path,
            &event_form_xml(
                Some("CatalogObject.Goods"),
                form_events,
                child_items,
                extension,
            ),
        );
        let args = Map::from_iter([(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        )]);

        let outcome = validate_form(&args, &context);

        assert_eq!(outcome.ok, expected_ok, "{outcome:?}");
        if let Some(code) = expected_code {
            assert!(
                outcome.errors.iter().any(|error| error.contains(code)),
                "{:?}",
                outcome.errors
            );
        }
        let _ = fs::remove_dir_all(&context.cwd);
    }
}

#[test]
fn form_edit_dry_run_uses_event_planner_without_writing() {
    let cases = [
        ("CatalogObject.Goods", "OnCreateAtServer", true, None),
        (
            "DataProcessorObject.EventProbe",
            "OnReadAtServer",
            false,
            Some("FORM_EVENT_NOT_ALLOWED"),
        ),
    ];
    for (main_type, event, expected_ok, expected_code) in cases {
        let context = temp_context("edit-event-dry-run");
        let form_path = context.cwd.join("Form.xml");
        let original = event_form_xml(Some(main_type), "", "", false).into_bytes();
        fs::write(&form_path, &original).unwrap();
        let args = Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            (
                "definition".to_string(),
                json!({"formEvents": [{"name": event, "handler": event}]}),
            ),
        ]);

        let cancellation = CancellationToken::new();
        let outcome = NativeOperationAdapter::invoke_with_data(
            "form-edit",
            "unica.form.edit",
            &args,
            &context,
            true,
            true,
            NativeInvocationContext::new(
                &crate::infrastructure::support_state::WorkspaceSupportStateReader::new(&context),
                &cancellation,
                ProviderDeadline::new(Instant::now() + Duration::from_secs(5)),
            ),
        )
        .unwrap()
        .adapter;

        assert_eq!(outcome.ok, expected_ok, "{outcome:?}");
        if expected_ok {
            assert!(
                outcome
                    .changes
                    .iter()
                    .any(|change| change.contains("would update")),
                "{outcome:?}"
            );
        } else {
            assert!(outcome.changes.is_empty(), "{outcome:?}");
        }
        if let Some(code) = expected_code {
            assert!(
                outcome.errors.iter().any(|error| error.contains(code)),
                "{outcome:?}"
            );
        }
        assert_eq!(fs::read(&form_path).unwrap(), original);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    let context = temp_context("edit-element-dry-run-wording");
    let form_path = context.cwd.join("Form.xml");
    let original = event_form_xml(Some("CatalogObject.Goods"), "", "", false).into_bytes();
    fs::write(&form_path, &original).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({"elements": [{"input": "Name"}]}),
        ),
    ]);
    let execution = preview_with_data(&args, &context);
    let outcome = &execution.outcome;
    assert!(outcome.ok, "{outcome:?}");
    // A preview plans the element and writes nothing; the summary, not a
    // printed heading, is what marks it as a dry run.
    assert!(outcome.summary.starts_with("dry run:"), "{outcome:?}");
    let data = execution
        .data
        .as_ref()
        .expect("form.edit answers with data");
    assert!(!data.added_elements.is_empty(), "{data:?}");
    assert_eq!(fs::read(&form_path).unwrap(), original);
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_edit_dry_run_reports_table_properties_without_writing() {
    let context = temp_context("edit-table-properties-preview");
    let form_path = context.cwd.join("Form.xml");
    let original = form_edit_remove_test_xml("").into_bytes();
    fs::write(&form_path, &original).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({
                "elements": [{
                    "table": "Rows",
                    "representation": "List",
                    "autoInsertNewRow": true
                }]
            }),
        ),
    ]);

    let execution = preview_with_data(&args, &context);
    let outcome = &execution.outcome;

    assert!(outcome.ok, "{outcome:?}");
    let data = execution
        .data
        .as_ref()
        .expect("form.edit answers with data");
    let table = data
        .added_elements
        .iter()
        .find(|element| element.kind == "Table")
        .expect("the planned table is reported");
    assert_eq!(table.representation.as_deref(), Some("List"), "{table:?}");
    assert_eq!(table.auto_insert_new_row, Some(true), "{table:?}");
    assert_eq!(fs::read(&form_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_edit_rejects_non_boolean_auto_insert_new_row_without_writing() {
    let context = temp_context("edit-table-property-type");
    let form_path = context.cwd.join("Form.xml");
    let original = form_edit_remove_test_xml("").into_bytes();
    fs::write(&form_path, &original).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({
                "elements": [{
                    "table": "Rows",
                    "autoInsertNewRow": "true"
                }]
            }),
        ),
    ]);

    let outcome = edit_form(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| { error.contains("autoInsertNewRow") && error.contains("boolean") }),
        "{outcome:?}"
    );
    assert!(outcome.changes.is_empty(), "{outcome:?}");
    assert_eq!(fs::read(&form_path).unwrap(), original);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_edit_repeated_identical_table_is_byte_exact_idempotent_noop() {
    let context = temp_context("edit-table-idempotent");
    let form_path = context.cwd.join("Form.xml");
    fs::write(&form_path, form_edit_remove_test_xml("")).unwrap();
    let target_args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({
                "elements": [{
                    "group": "vertical",
                    "name": "Container"
                }]
            }),
        ),
    ]);
    let target = edit_form(&target_args, &context);
    assert!(target.ok, "{target:?}");
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({
                "into": "Container",
                "elements": [{
                    "table": "Rows",
                    "representation": "List",
                    "autoInsertNewRow": true,
                    "columns": [{
                        "input": "RowsItem"
                    }]
                }]
            }),
        ),
    ]);

    let first = edit_form(&args, &context);
    assert!(first.ok, "{first:?}");
    let after_first = fs::read(&form_path).unwrap();

    let second = edit_form(&args, &context);

    assert!(second.ok, "{second:?}");
    assert!(second.changes.is_empty(), "{second:?}");
    assert!(
        second.summary.contains("no-op")
            || second
                .stdout
                .as_deref()
                .is_some_and(|stdout| stdout.contains("no-op")),
        "{second:?}"
    );
    assert_eq!(fs::read(&form_path).unwrap(), after_first);

    let validation = validate_form(&form_path_args(&form_path), &context);
    assert!(validation.ok, "{validation:?}");

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_edit_rejects_conflicting_existing_table_without_writing() {
    let context = temp_context("edit-table-idempotent-conflict");
    let form_path = context.cwd.join("Form.xml");
    fs::write(&form_path, form_edit_remove_test_xml("")).unwrap();
    let list_args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({
                "elements": [{
                    "table": "Rows",
                    "representation": "List"
                }]
            }),
        ),
    ]);
    let first = edit_form(&list_args, &context);
    assert!(first.ok, "{first:?}");
    let after_first = fs::read(&form_path).unwrap();
    let tree_args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({
                "elements": [{
                    "table": "Rows",
                    "representation": "Tree"
                }]
            }),
        ),
    ]);

    let conflict = edit_form(&tree_args, &context);

    assert!(!conflict.ok, "{conflict:?}");
    assert!(
        conflict
            .errors
            .iter()
            .any(|error| error.contains("Element 'Rows' already exists in form")),
        "{conflict:?}"
    );
    assert!(conflict.changes.is_empty(), "{conflict:?}");
    assert_eq!(fs::read(&form_path).unwrap(), after_first);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn form_edit_accepts_platform_omitted_nil_row_filter_as_idempotent() {
    let context = temp_context("edit-table-idempotent-platform-default");
    let form_path = context.cwd.join("Form.xml");
    fs::write(&form_path, form_edit_remove_test_xml("")).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({
                "elements": [{
                    "table": "Rows",
                    "representation": "Tree",
                    "autoInsertNewRow": true
                }]
            }),
        ),
    ]);
    let first = edit_form(&args, &context);
    assert!(first.ok, "{first:?}");
    let platform_normalized = fs::read_to_string(&form_path)
        .unwrap()
        .replace("\t\t\t<RowFilter xsi:nil=\"true\"/>\n", "");
    assert!(
        !platform_normalized.contains("<RowFilter"),
        "{platform_normalized}"
    );
    fs::write(&form_path, &platform_normalized).unwrap();

    let second = edit_form(&args, &context);

    assert!(second.ok, "{second:?}");
    assert!(second.changes.is_empty(), "{second:?}");
    assert_eq!(fs::read_to_string(&form_path).unwrap(), platform_normalized);

    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_form_writes_call_type_only_for_extension_events() {
    let child_items = r#"\t\t<InputField name="Name" id="1">\n\t\t\t<DataPath>Object.Name</DataPath>\n\t\t\t<ContextMenu name="NameContextMenu" id="2"/>\n\t\t\t<ExtendedTooltip name="NameExtendedTooltip" id="3"/>\n\t\t</InputField>\n"#;
    let definition = json!({
        "formEvents": [{
            "name": "OnOpen",
            "handler": "OnOpenAfter",
            "callType": "After"
        }],
        "elementEvents": [{
            "element": "Name",
            "name": "OnChange",
            "handler": "NameOnChangeBefore",
            "callType": "Before"
        }]
    });

    for extension in [false, true] {
        let context = temp_context("edit-event-call-type");
        let form_path = context.cwd.join("Form.xml");
        let original =
            event_form_xml(Some("CatalogObject.Goods"), "", child_items, extension).into_bytes();
        fs::write(&form_path, &original).unwrap();
        let args = Map::from_iter([
            (
                "FormPath".to_string(),
                json!(form_path.display().to_string()),
            ),
            ("definition".to_string(), definition.clone()),
        ]);

        let outcome = edit_form(&args, &context);

        if extension {
            assert!(outcome.ok, "{outcome:?}");
            let updated = fs::read_to_string(&form_path).unwrap();
            assert!(updated.contains("name=\"OnOpen\" callType=\"After\""));
            assert!(updated.contains("name=\"OnChange\" callType=\"Before\""));
            let validation = validate_form(&args, &context);
            assert!(validation.ok, "{validation:?}");
        } else {
            assert!(!outcome.ok, "{outcome:?}");
            assert!(outcome
                .errors
                .iter()
                .any(|error| { error.contains("FORM_EVENT_CALL_TYPE_NOT_ALLOWED") }));
            assert_eq!(fs::read(&form_path).unwrap(), original);
        }
        let _ = fs::remove_dir_all(&context.cwd);
    }
}

#[test]
fn edit_form_rejects_borrowed_event_without_call_type_before_write() {
    let context = temp_context("edit-borrowed-event-missing-call-type");
    let form_path = context.cwd.join("Form.xml");
    let original = event_form_xml(Some("CatalogObject.Goods"), "", "", true).into_bytes();
    fs::write(&form_path, &original).unwrap();
    let args = Map::from_iter([
        (
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        ),
        (
            "definition".to_string(),
            json!({
                "formEvents": [{
                    "name": "OnOpen",
                    "handler": "OnOpenAfter"
                }]
            }),
        ),
    ]);

    let outcome = edit_form(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(outcome
        .errors
        .iter()
        .any(|error| error.contains("FORM_EVENT_CALL_TYPE_REQUIRED")));
    assert!(outcome.changes.is_empty(), "{outcome:?}");
    assert_eq!(fs::read(&form_path).unwrap(), original);
    let _ = fs::remove_dir_all(&context.cwd);
}

fn event_form_xml(
    main_type: Option<&str>,
    form_events: &str,
    child_items: &str,
    extension: bool,
) -> String {
    let base_form = if extension {
        "\t<BaseForm version=\"2.20\"/>\n"
    } else {
        ""
    };
    let attributes = main_type.map_or_else(
            || "\t<Attributes/>\n".to_string(),
            |main_type| {
                format!(
                    "\t<Attributes>\n\t\t<Attribute name=\"Object\" id=\"1\">\n\t\t\t<Type><v8:Type>cfg:{main_type}</v8:Type></Type>\n\t\t\t<MainAttribute>true</MainAttribute>\n\t\t</Attribute>\n\t</Attributes>\n"
                )
            },
        );
    format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<Form xmlns=\"{FORM_LOGFORM_NS}\" xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\" xmlns:v8=\"{FORM_V8_NS}\" version=\"2.20\">\n{base_form}\t<AutoCommandBar name=\"FormCommandBar\" id=\"-1\"/>\n{form_events}\t<ChildItems>\n{child_items}\t</ChildItems>\n{attributes}\t<Commands/>\n</Form>\n"
        )
}

fn form_edit_remove_test_xml(child_items: &str) -> String {
    format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<Form xmlns=\"{FORM_LOGFORM_NS}\" version=\"2.20\">\n\t<AutoCommandBar name=\"FormCommandBar\" id=\"-1\"/>\n\t<ChildItems>\n{child_items}\t</ChildItems>\n\t<Attributes/>\n\t<Commands/>\n</Form>\n"
        )
}

fn form_edit_remove_extension_test_xml(
    working_child_items: &str,
    base_child_items: &str,
) -> String {
    format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<Form xmlns=\"{FORM_LOGFORM_NS}\" version=\"2.20\">\n\t<AutoCommandBar name=\"FormCommandBar\" id=\"-1\"/>\n\t<ChildItems>\n{working_child_items}\t</ChildItems>\n\t<Attributes/>\n\t<Commands/>\n\t<BaseForm version=\"2.20\">\n\t\t<AutoCommandBar name=\"FormCommandBar\" id=\"-1\"/>\n\t\t<ChildItems>\n{base_child_items}\t\t</ChildItems>\n\t\t<Attributes/>\n\t\t<Commands/>\n\t</BaseForm>\n</Form>\n"
        )
}

fn editable_form_xml(extension: bool) -> &'static str {
    if extension {
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
	<BaseForm>Catalog.ParityCatalog.Form.ItemForm</BaseForm>
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1">
		<Autofill>true</Autofill>
	</AutoCommandBar>
	<ChildItems>
	</ChildItems>
	<Attributes>
		<Attribute name="Object" id="1">
			<Type><v8:Type>cfg:CatalogObject.ParityCatalog</v8:Type></Type>
		</Attribute>
	</Attributes>
	<Commands>
		<Command name="Refresh" id="2"><Action>Refresh</Action></Command>
	</Commands>
</Form>
"#
    } else {
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
	<AutoCommandBar name="ФормаКоманднаяПанель" id="-1">
		<Autofill>true</Autofill>
	</AutoCommandBar>
	<ChildItems>
	</ChildItems>
	<Attributes>
		<Attribute name="Object" id="1">
			<Type><v8:Type>cfg:CatalogObject.ParityCatalog</v8:Type></Type>
		</Attribute>
	</Attributes>
	<Commands>
		<Command name="Refresh" id="2"><Action>Refresh</Action></Command>
	</Commands>
</Form>
"#
    }
}
