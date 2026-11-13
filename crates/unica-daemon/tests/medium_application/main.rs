//! The application's public tools over the infrastructure ports, as the
//! daemon composes them: native mutators, their format and support gates,
//! publication and the typed results. Each test reads and writes a real
//! workspace through the product's ports, so it lives with the product; the
//! application's own contracts are tested on test ports in `unica-application`.

#[path = "../../../unica-coder/src/infrastructure/format_gate.rs"]
mod format_gate;

use format_gate::{support_test_catalog_xml, test_workspace_root};
use unica_application::application::*;
use unica_coder::infrastructure::application_ports::InfrastructureApplicationPorts;
use unica_workspace::compile_transaction::CompileTransaction;
use unica_workspace::single_file_publisher::{
    with_publication_lock_contention_signal, with_publication_lock_pause,
};
use unica_workspace::test_support::{
    create_file_link_fixture_for_test, file_identity_for_test, prepare_file_for_removal,
    set_unix_mode_for_test, unix_mode_for_test, FileLinkFixtureOutcome,
};

/// The application on the infrastructure ports, as the daemon composes it.
fn application() -> UnicaApplication {
    UnicaApplication::with_ports(Arc::new(InfrastructureApplicationPorts::new()))
}

#[test]
fn incompatible_format_blocks_before_native_handler() {
    format_gate::incompatible_format_blocks_before_native_handler();
}

#[test]
fn public_metadata_mutators_refuse_old_and_new_profiles_without_side_effects() {
    format_gate::public_metadata_mutators_refuse_old_and_new_profiles_without_side_effects();
}
use serde_json::{json, Value};
use std::path::PathBuf;

use serde_json::Map;

use std::sync::{mpsc, Arc, Barrier};
use std::thread;
use std::time::Duration;

fn normalized_path(path: &std::path::Path) -> std::path::PathBuf {
    unica_workspace::test_support::canonical_path(path)
}

fn call_public_tool_from_workspace(
    workspace: &std::path::Path,
    name: &str,
    args: &Map<String, Value>,
) -> Result<OperationResult, String> {
    let _cwd = unica_workspace::test_support::ProcessCwdGuard::enter(workspace)?;
    application().call_tool(name, args)
}

fn path_text(path: &std::path::Path) -> String {
    path.display().to_string().replace('\\', "/")
}

#[test]
fn entity_spelled_supported_format_is_invalid_at_the_public_boundary() {
    for (index, raw) in ["2.&#50;0", "&#x32;.20", "2.2&#48;"]
        .into_iter()
        .enumerate()
    {
        let (root, workspace, config_path) = cf_edit_mutation_workspace(
            &format!("unica-entity-spelled-format-{index}"),
            support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa")
                .replacen(r#"version="2.20""#, &format!(r#"version="{raw}""#), 1)
                .as_bytes(),
        );
        let before = std::fs::read(&config_path).unwrap();

        let mutation = application()
            .call_tool(
                "unica.cf.edit",
                &cf_edit_args(&workspace, "modify-property", "Version=2.0"),
            )
            .unwrap();

        assert!(!mutation.ok, "{raw}: {mutation:?}");
        let diagnostic = &mutation.diagnostics.as_ref().unwrap()["formatCompatibility"];
        assert_eq!(diagnostic["code"], "formatVersionInvalid", "{raw}");
        assert_eq!(diagnostic["actualFormat"], raw, "{raw}");
        assert_eq!(std::fs::read(&config_path).unwrap(), before, "{raw}");
        assert!(mutation.changes.is_empty(), "{raw}: {mutation:?}");
        assert!(mutation.artifacts.is_empty(), "{raw}: {mutation:?}");
        assert!(mutation.cache.events.is_empty(), "{raw}: {mutation:?}");
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn numeric_equivalent_noncanonical_format_warns_on_read_and_blocks_public_mutator() {
    for (index, raw) in ["2.20.0", "02.20", "2.020"].into_iter().enumerate() {
        let (root, workspace, config_path) = cf_edit_mutation_workspace(
            &format!("unica-noncanonical-format-{index}"),
            support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa")
                .replacen(r#"version="2.20""#, &format!(r#"version="{raw}""#), 1)
                .as_bytes(),
        );
        let before = std::fs::read(&config_path).unwrap();

        let mutation = application()
            .call_tool(
                "unica.cf.edit",
                &cf_edit_args(&workspace, "modify-property", "Version=2.0"),
            )
            .unwrap();

        assert!(!mutation.ok, "{raw}: {mutation:?}");
        let mutation_diagnostic = &mutation.diagnostics.as_ref().unwrap()["formatCompatibility"];
        assert_eq!(mutation_diagnostic["code"], "formatVersionInvalid", "{raw}");
        assert_eq!(mutation_diagnostic["actualFormat"], raw, "{raw}");
        assert_eq!(std::fs::read(&config_path).unwrap(), before, "{raw}");
        assert!(mutation.changes.is_empty(), "{raw}: {mutation:?}");
        assert!(mutation.artifacts.is_empty(), "{raw}: {mutation:?}");
        assert!(mutation.cache.events.is_empty(), "{raw}: {mutation:?}");
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn code_patch_public_result_is_typed_and_emits_only_applied_change_events() {
    let root = test_workspace_root("unica-code-patch-public-result");
    let workspace = root.join("workspace");
    let src = workspace.join("src");
    let module = src.join("CommonModules/Sample/Ext/Module.bsl");
    std::fs::create_dir_all(module.parent().unwrap()).unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(
        src.join("Configuration.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
    )
    .unwrap();
    std::fs::write(
        src.join("CommonModules/Sample.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule><Properties><Name>Sample</Name></Properties></CommonModule></MetaDataObject>"#,
    )
    .unwrap();
    std::fs::write(
        &module,
        "Procedure Run()\n    Message(\"ok\");\nEndProcedure\n",
    )
    .unwrap();
    let app = application();
    let mut args = json!({
        "cwd": workspace,
        "sourceSet": "main",
        "metadataPath": "CommonModule.Sample.Module",
        "operation": "insert",
        "selector": {"method": "Run"},
        "content": "Procedure Added()\nEndProcedure",
        "position": "after"
    })
    .as_object()
    .unwrap()
    .clone();

    let preview = app.call_tool("unica.code.patch", &args).unwrap();
    assert!(preview.ok, "{:?}", preview.errors);
    assert!(preview.stdout.is_none());
    assert!(preview.cache.events.is_empty());
    assert_eq!(preview.data.as_ref().unwrap()["sourceSet"], "main");
    assert_eq!(
        preview.data.as_ref().unwrap()["metadataPath"],
        "CommonModule.Sample.Module"
    );
    assert_eq!(preview.data.as_ref().unwrap()["targetKind"], "module");
    assert!(preview.data.as_ref().unwrap().get("path").is_none());
    assert!(preview.data.as_ref().unwrap()["affectedTarget"]
        .get("path")
        .is_none());
    assert_eq!(
        preview.data.as_ref().unwrap()["affectedTarget"]["owner"],
        "CommonModule.Sample"
    );
    assert_eq!(
        preview.data.as_ref().unwrap()["validation"]["status"],
        "passed"
    );
    let serialized = serde_json::to_value(&preview).unwrap();
    assert!(serialized["data"].is_object());
    assert!(serialized.get("stdout").is_none());
    assert!(!std::fs::read_to_string(&module)
        .unwrap()
        .contains("Procedure Added"));

    args.insert("dryRun".to_string(), json!(false));
    let applied = app.call_tool("unica.code.patch", &args).unwrap();
    assert!(applied.ok, "{:?}", applied.errors);
    assert_eq!(applied.cache.events, vec!["ModuleChanged"]);
    assert_eq!(applied.cache.mode, "applied");

    let repeated = app.call_tool("unica.code.patch", &args).unwrap();
    assert!(repeated.ok, "{:?}", repeated.errors);
    assert!(repeated.cache.events.is_empty());
    assert_eq!(repeated.data.as_ref().unwrap()["noOp"], true);

    let before_invalid = std::fs::read(&module).unwrap();
    args.insert(
        "selector".to_string(),
        json!({"anchor": "Message(\"ok\");"}),
    );
    args.insert("content".to_string(), json!("    If True Then"));
    let rejected = app.call_tool("unica.code.patch", &args).unwrap();
    assert!(!rejected.ok);
    assert!(rejected.cache.events.is_empty());
    assert_eq!(
        rejected.data.as_ref().unwrap()["validation"]["status"],
        "failed"
    );
    assert_eq!(std::fs::read(&module).unwrap(), before_invalid);

    let empty_module = src.join("CommonModules/Empty/Ext/Module.bsl");
    std::fs::create_dir_all(empty_module.parent().unwrap()).unwrap();
    std::fs::write(
        src.join("CommonModules/Empty.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule><Properties><Name>Empty</Name></Properties></CommonModule></MetaDataObject>"#,
    )
    .unwrap();
    std::fs::write(&empty_module, b"\xef\xbb\xbf").unwrap();
    // A module holding no method yet takes a selector-less insert: the end
    // of the module is the one place it already has.
    let mut first_body = json!({
        "cwd": workspace,
        "sourceSet": "main",
        "metadataPath": "CommonModule.Empty.Module",
        "operation": "insert",
        "content": "Procedure Run()\nEndProcedure"
    })
    .as_object()
    .unwrap()
    .clone();
    let first_preview = app.call_tool("unica.code.patch", &first_body).unwrap();
    assert!(first_preview.ok, "{:?}", first_preview.errors);
    assert!(first_preview.cache.events.is_empty());
    assert_eq!(
        first_preview.data.as_ref().unwrap()["validation"]["status"],
        "passed"
    );
    assert_eq!(std::fs::read(&empty_module).unwrap(), b"\xef\xbb\xbf");

    first_body.insert("dryRun".to_string(), json!(false));
    let written = app.call_tool("unica.code.patch", &first_body).unwrap();
    assert!(written.ok, "{:?}", written.errors);
    assert_eq!(written.cache.events, vec!["ModuleChanged"]);
    assert_eq!(
        std::fs::read(&empty_module).unwrap(),
        b"\xef\xbb\xbfProcedure Run()\nEndProcedure\n"
    );

    // Unlike a dedicated initialize operation, the repeat stays a proven
    // no-op instead of failing after the write already landed.
    let repeat = app.call_tool("unica.code.patch", &first_body).unwrap();
    assert!(repeat.ok, "{:?}", repeat.errors);
    assert!(repeat.cache.events.is_empty());
    assert_eq!(
        std::fs::read(&empty_module).unwrap(),
        b"\xef\xbb\xbfProcedure Run()\nEndProcedure\n"
    );

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn form_edit_remove_returns_typed_data() {
    let root = test_workspace_root("unica-form-edit-remove-typed-data");
    let workspace = root.join("workspace");
    let form_path = workspace.join("Form.xml");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        &form_path,
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
	<AutoCommandBar name="FormCommandBar" id="-1"/>
	<ChildItems>
		<Group name="First" id="1">
			<ChildItems>
				<InputField name="FirstInput" id="2">
					<ContextMenu name="FirstInputContextMenu" id="3"/>
				</InputField>
			</ChildItems>
		</Group>
		<InputField name="Second" id="4">
			<ContextMenu name="SecondContextMenu" id="5"/>
			<ExtendedTooltip name="SecondExtendedTooltip" id="6"/>
		</InputField>
	</ChildItems>
	<Attributes/>
	<Commands/>
</Form>
"#,
    )
    .unwrap();
    let original = std::fs::read(&form_path).unwrap();
    let app = application();
    let mut args = json!({
        "cwd": workspace,
        "FormPath": form_path,
        "definition": {
            "removeElements": [{"name": "Second"}, {"name": "First"}]
        }
    })
    .as_object()
    .unwrap()
    .clone();
    let expected_data = json!({
        "changed": true,
        "removed": [
            {"name": "Second", "kind": "InputField", "reason": "requested"},
            {"name": "SecondContextMenu", "kind": "ContextMenu", "reason": "contained"},
            {"name": "SecondExtendedTooltip", "kind": "ExtendedTooltip", "reason": "contained"},
            {"name": "First", "kind": "Group", "reason": "requested"},
            {"name": "FirstInput", "kind": "InputField", "reason": "contained"},
            {"name": "FirstInputContextMenu", "kind": "ContextMenu", "reason": "contained"}
        ],
        "addedElements": [],
        "addedAttributes": [],
        "addedCommands": [],
        "addedEvents": [],
        "validation": "passed"
    });

    let preview = app.call_tool("unica.form.edit", &args).unwrap();
    assert!(preview.ok, "{:?}", preview.errors);
    assert_eq!(preview.data, Some(expected_data.clone()));
    assert!(preview.stdout.is_none(), "{preview:?}");
    assert!(preview.cache.events.is_empty());
    assert_eq!(std::fs::read(&form_path).unwrap(), original);

    args.insert("dryRun".to_string(), json!(false));
    let applied = app.call_tool("unica.form.edit", &args).unwrap();
    assert!(applied.ok, "{:?}", applied.errors);
    assert_eq!(applied.data, Some(expected_data));
    assert_eq!(applied.cache.events, vec!["FormChanged"]);
    assert!(applied.stdout.is_none(), "{applied:?}");

    let non_removal_form_path = workspace.join("NonRemoval.xml");
    std::fs::write(
        &non_removal_form_path,
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
	<AutoCommandBar name="FormCommandBar" id="-1"/>
	<ChildItems/>
	<Attributes/>
	<Commands/>
</Form>
"#,
    )
    .unwrap();
    let non_removal_args = json!({
        "cwd": workspace,
        "dryRun": false,
        "FormPath": non_removal_form_path,
        "definition": {"elements": [{"input": "Added"}]}
    })
    .as_object()
    .unwrap()
    .clone();
    let non_removal = app.call_tool("unica.form.edit", &non_removal_args).unwrap();
    assert!(non_removal.ok, "{:?}", non_removal.errors);
    assert_eq!(
        non_removal.data,
        Some(json!({
            "changed": true,
            "removed": [],
            // The addition now shows up as data, not as a printed line.
            "addedElements": [{
                "kind": "InputField",
                "name": "Added",
                "path": null,
                "representation": null,
                "autoInsertNewRow": null
            }],
            "addedAttributes": [],
            "addedCommands": [],
            "addedEvents": [],
            "validation": "passed"
        }))
    );
    assert_eq!(non_removal.cache.events, vec!["FormChanged"]);

    let no_op_form_path = workspace.join("NoOp.xml");
    let no_op_original = r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
	<AutoCommandBar name="FormCommandBar" id="-1"/>
	<ChildItems/>
	<Attributes/>
	<Commands/>
</Form>
"#;
    std::fs::write(&no_op_form_path, no_op_original).unwrap();
    let no_op_args = json!({
        "cwd": workspace,
        "dryRun": false,
        "FormPath": no_op_form_path,
        "definition": {}
    })
    .as_object()
    .unwrap()
    .clone();
    let no_op = app.call_tool("unica.form.edit", &no_op_args).unwrap();
    assert!(no_op.ok, "{:?}", no_op.errors);
    assert_eq!(
        no_op.data,
        Some(json!({
            "changed": false,
            "removed": [],
            "addedElements": [],
            "addedAttributes": [],
            "addedCommands": [],
            "addedEvents": [],
            "validation": "passed"
        }))
    );
    assert!(no_op.cache.events.is_empty());
    assert_eq!(
        std::fs::read(&no_op_form_path).unwrap(),
        no_op_original.as_bytes()
    );

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn form_edit_preview_rejects_an_invalid_projected_form_at_the_public_boundary() {
    let root = test_workspace_root("unica-form-edit-project-validation");
    let workspace = root.join("workspace");
    let form_path = workspace.join("Form.xml");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        &form_path,
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
	<AutoCommandBar name="FormCommandBar" id="-1"/>
	<ChildItems>
		<InputField name="RemoveMe" id="1">
			<ContextMenu name="RemoveMeContextMenu" id="2"/>
			<ExtendedTooltip name="RemoveMeExtendedTooltip" id="3"/>
		</InputField>
		<InputField name="AlreadyInvalid" id="4"/>
	</ChildItems>
	<Attributes/>
	<Commands/>
</Form>
"#,
    )
    .unwrap();
    let original = std::fs::read(&form_path).unwrap();
    let args = json!({
        "cwd": workspace,
        "FormPath": form_path,
        "definition": {"removeElements": [{"name": "RemoveMe"}]}
    })
    .as_object()
    .unwrap()
    .clone();

    let result = application().call_tool("unica.form.edit", &args).unwrap();

    assert!(!result.ok, "{result:?}");
    assert!(result.data.is_none(), "{result:?}");
    assert!(result.cache.events.is_empty(), "{result:?}");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.contains("AlreadyInvalid") && error.contains("missing companion")),
        "{result:?}"
    );
    assert_eq!(std::fs::read(&form_path).unwrap(), original);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn form_edit_remove_json_path_uses_the_typed_public_contract() {
    let root = test_workspace_root("unica-form-edit-remove-json-path");
    let workspace = root.join("workspace");
    let form_path = workspace.join("Form.xml");
    let definition_path = workspace.join("remove.json");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        &form_path,
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
	<AutoCommandBar name="FormCommandBar" id="-1"/>
	<ChildItems>
		<InputField name="Target" id="1">
			<ContextMenu name="TargetContextMenu" id="2"/>
			<ExtendedTooltip name="TargetExtendedTooltip" id="3"/>
		</InputField>
	</ChildItems>
	<Attributes/>
	<Commands/>
</Form>
"#,
    )
    .unwrap();
    std::fs::write(
        &definition_path,
        r#"{"removeElements":[{"name":"Target"}]}"#,
    )
    .unwrap();
    let original_form = std::fs::read(&form_path).unwrap();
    let original_definition = std::fs::read(&definition_path).unwrap();
    let mut args = json!({
        "cwd": workspace,
        "FormPath": form_path,
        "JsonPath": definition_path
    })
    .as_object()
    .unwrap()
    .clone();
    let expected = json!({
        "changed": true,
        "removed": [
            {"name": "Target", "kind": "InputField", "reason": "requested"},
            {"name": "TargetContextMenu", "kind": "ContextMenu", "reason": "contained"},
            {"name": "TargetExtendedTooltip", "kind": "ExtendedTooltip", "reason": "contained"}
        ],
        "addedElements": [],
        "addedAttributes": [],
        "addedCommands": [],
        "addedEvents": [],
        "validation": "passed"
    });
    let app = application();

    let preview = app.call_tool("unica.form.edit", &args).unwrap();
    assert!(preview.ok, "{preview:?}");
    assert_eq!(preview.data, Some(expected.clone()));
    assert!(preview.cache.events.is_empty(), "{preview:?}");
    assert_eq!(std::fs::read(&form_path).unwrap(), original_form);

    args.insert("dryRun".to_string(), json!(false));
    let apply = app.call_tool("unica.form.edit", &args).unwrap();
    assert!(apply.ok, "{apply:?}");
    assert_eq!(apply.data, Some(expected));
    assert_eq!(apply.cache.events, vec!["FormChanged"]);
    assert!(!std::fs::read_to_string(&form_path)
        .unwrap()
        .contains("name=\"Target\""));
    assert_eq!(
        std::fs::read(&definition_path).unwrap(),
        original_definition
    );

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
pub(crate) fn code_patch_locked_support_blocks_preview_and_apply_before_handler() {
    let root = test_workspace_root("unica-code-patch-support-guard");
    let workspace = root.join("workspace");
    let src = workspace.join("src");
    let module = src.join("Catalogs/Items/Ext/ObjectModule.bsl");
    std::fs::create_dir_all(module.parent().unwrap()).unwrap();
    std::fs::create_dir_all(src.join("Ext")).unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(
        src.join("Configuration.xml"),
        support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
    )
    .unwrap();
    std::fs::write(
        src.join("Catalogs/Items.xml"),
        support_test_catalog_xml("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
    )
    .unwrap();
    std::fs::write(
        src.join("Ext/ParentConfigurations.bin"),
        support_test_parent_configurations_bin(
            "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
            "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
            "cccccccc-cccc-cccc-cccc-cccccccccccc",
        ),
    )
    .unwrap();
    let before = b"Procedure Run()\nEndProcedure\n";
    std::fs::write(&module, before).unwrap();
    let mut args = json!({
        "cwd": workspace,
        "sourceSet": "main",
        "metadataPath": "Catalog.Items.ObjectModule",
        "operation": "insert",
        "selector": {"method": "Run"},
        "content": "Procedure Added()\nEndProcedure",
        "position": "after"
    })
    .as_object()
    .unwrap()
    .clone();

    let mut results = Vec::new();
    for dry_run in [false, true] {
        args.insert("dryRun".to_string(), json!(dry_run));
        let result = application().call_tool("unica.code.patch", &args).unwrap();

        assert!(!result.ok, "dryRun={dry_run}: {result:?}");
        assert!(result.summary.contains("support guard"));
        assert!(result.errors.join("\n").contains("на замке"));
        assert!(result.data.is_none());
        assert!(result.cache.events.is_empty());
        assert_eq!(std::fs::read(&module).unwrap(), before);
        results.push(result);
    }
    assert_support_guard_block_parity(&results[0], &results[1]);

    std::fs::remove_dir_all(root).unwrap();
}

fn role_edit_application_workspace(
    label: &str,
    descriptor_version: &str,
    support_locked: bool,
) -> (PathBuf, PathBuf, Map<String, Value>) {
    let root = test_workspace_root(label);
    let workspace = root.join("workspace");
    let src = workspace.join("src");
    let rights = src.join("Roles/Demo/Ext/Rights.xml");
    std::fs::create_dir_all(rights.parent().unwrap()).unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(
        src.join("Configuration.xml"),
        format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{descriptor_version}"><Configuration uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"><Properties><Name>Main</Name></Properties><ChildObjects><Role>Demo</Role></ChildObjects></Configuration></MetaDataObject>"#
        ),
    )
    .unwrap();
    std::fs::write(
        src.join("Roles/Demo.xml"),
        format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{descriptor_version}"><Role uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"><Properties><Name>Demo</Name></Properties></Role></MetaDataObject>"#
        ),
    )
    .unwrap();
    std::fs::write(
        &rights,
        concat!(
            "<Rights xmlns=\"http://v8.1c.ru/8.2/roles\" ",
            "xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" ",
            "xsi:type=\"Rights\" version=\"2.20\">",
            "<setForNewObjects>false</setForNewObjects>",
            "<setForAttributesByDefault>true</setForAttributesByDefault>",
            "<independentRightsOfChildObjects>false</independentRightsOfChildObjects>",
            "<object><name>Catalog.Demo</name><right><name>Delete</name>",
            "<value>true</value></right></object></Rights>"
        ),
    )
    .unwrap();
    if support_locked {
        std::fs::create_dir_all(src.join("Ext")).unwrap();
        std::fs::write(
            src.join("Ext/ParentConfigurations.bin"),
            support_test_parent_configurations_bin(
                "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
                "cccccccc-cccc-cccc-cccc-cccccccccccc",
            ),
        )
        .unwrap();
    }
    let args = json!({
        "sourceSet": "main",
        "metadataPath": "Role.Demo",
        "operations": [{
            "op": "setRight",
            "objectName": "Catalog.Demo",
            "right": "Delete",
            "value": false
        }]
    })
    .as_object()
    .unwrap()
    .clone();
    (root, workspace, args)
}

fn assert_typed_role_edit_failure(
    result: &OperationResult,
    workspace: &std::path::Path,
    expected_metadata_path: &str,
    expected_code: &str,
) {
    assert!(!result.ok, "{result:?}");
    let value = serde_json::to_value(result).unwrap();
    assert_eq!(
        value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([
            "artifacts",
            "cache",
            "changes",
            "data",
            "errors",
            "ok",
            "summary",
            "warnings",
        ])
    );
    assert_eq!(value["artifacts"], json!([]));
    assert_eq!(value["cache"]["root"], "");
    assert_eq!(value["data"]["metadataPath"], expected_metadata_path);
    assert_eq!(value["data"]["changed"], false);
    assert_eq!(value["data"]["effects"], json!([]));
    assert_eq!(value["data"]["validation"], json!({"status":"failed"}));
    assert_eq!(value["data"]["diagnostics"][0]["code"], expected_code);
    assert!(jsonschema::validator_for(&role_edit_output_schema())
        .unwrap()
        .is_valid(&value));
    let encoded = serde_json::to_string(&value).unwrap();
    assert!(
        !encoded.contains(&workspace.display().to_string()),
        "{encoded}"
    );
    assert!(!encoded.contains("Rights.xml"), "{encoded}");
}

/// rule: INV.SOURCE.WRITABLE-PROFILE
/// Older than 2.20 and newer than 2.22 are refused without changing bytes.
#[test]
fn role_edit_application_projects_2_19_and_2_23_format_blocks_to_typed_data() {
    for (version, code) in [
        ("2.19", "formatMigrationAvailable"),
        ("2.23", "platformVersionUnsupported"),
    ] {
        let (root, workspace, args) = role_edit_application_workspace(
            &format!("unica-role-edit-format-{version}"),
            version,
            false,
        );
        let before = std::fs::read(workspace.join("src/Roles/Demo/Ext/Rights.xml")).unwrap();
        let result = call_public_tool_from_workspace(&workspace, "unica.role.edit", &args).unwrap();
        assert_typed_role_edit_failure(&result, &workspace, "Role.Demo", code);
        assert_eq!(
            std::fs::read(workspace.join("src/Roles/Demo/Ext/Rights.xml")).unwrap(),
            before
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

/// rule: INV.SOURCE.WRITABLE-PROFILE
/// A set in a writable format beyond 2.20 is edited in its own format.
#[test]
fn role_edit_application_edits_2_21_and_2_22_sets_in_their_own_format() {
    for version in ["2.21", "2.22"] {
        let (root, workspace, mut args) = role_edit_application_workspace(
            &format!("unica-role-edit-keeps-{version}"),
            version,
            false,
        );
        let rights = workspace.join("src/Roles/Demo/Ext/Rights.xml");
        let own_format = std::fs::read_to_string(&rights)
            .unwrap()
            .replace(r#"version="2.20""#, &format!(r#"version="{version}""#));
        std::fs::write(&rights, own_format).unwrap();
        args.insert("dryRun".to_string(), json!(false));
        let applied =
            call_public_tool_from_workspace(&workspace, "unica.role.edit", &args).unwrap();
        assert!(applied.ok, "{version}: {applied:?}");
        assert_eq!(applied.data.as_ref().unwrap()["changed"], true, "{version}");
        let edited = std::fs::read_to_string(&rights).unwrap();
        assert!(
            edited.contains(&format!(r#"version="{version}""#)) && !edited.contains("2.20"),
            "{version}: {edited}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn role_edit_application_uses_transactionally_recorded_cache_and_noop_does_not_republish() {
    let (root, workspace, mut args) =
        role_edit_application_workspace("unica-role-edit-recorded-cache", "2.20", false);
    args.insert("dryRun".to_string(), json!(false));
    let applied = call_public_tool_from_workspace(&workspace, "unica.role.edit", &args).unwrap();
    assert!(applied.ok, "{applied:?}");
    assert_eq!(applied.cache.root, "");
    assert_eq!(applied.cache.mode, "applied");
    assert_eq!(applied.cache.events, ["RoleChanged"]);
    assert!(applied
        .cache
        .invalidated
        .contains(&"rights_graph".to_string()));
    assert!(applied
        .cache
        .refreshed
        .contains(&"rights_graph".to_string()));
    assert_eq!(applied.data.as_ref().unwrap()["changed"], true);
    let state_path = workspace.join(".build/unica/state.json");
    let state_after_apply = std::fs::read(&state_path).unwrap();

    let repeated = call_public_tool_from_workspace(&workspace, "unica.role.edit", &args).unwrap();
    assert!(repeated.ok, "{repeated:?}");
    assert_eq!(repeated.cache.root, "");
    assert!(repeated.cache.events.is_empty());
    assert_eq!(repeated.data.as_ref().unwrap()["changed"], false);
    assert_eq!(std::fs::read(&state_path).unwrap(), state_after_apply);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn role_edit_application_projects_cache_read_failure_without_physical_paths() {
    let (root, workspace, args) =
        role_edit_application_workspace("unica-role-edit-cache-failure", "2.20", false);
    let rights = workspace.join("src/Roles/Demo/Ext/Rights.xml");
    let before = std::fs::read(&rights).unwrap();
    std::fs::create_dir_all(workspace.join(".build/unica/state.json")).unwrap();

    let result = call_public_tool_from_workspace(&workspace, "unica.role.edit", &args).unwrap();

    assert_typed_role_edit_failure(&result, &workspace, "Role.Demo", "cache_unavailable");
    assert_eq!(std::fs::read(&rights).unwrap(), before);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn role_edit_application_reports_one_stable_support_warning() {
    let (root, workspace, mut args) =
        role_edit_application_workspace("unica-role-edit-support-warn", "2.20", true);
    std::fs::write(
        workspace.join(".v8-project.json"),
        r#"{"editingAllowedCheck":"warn"}"#,
    )
    .unwrap();
    args.insert("dryRun".to_string(), json!(false));

    let result = call_public_tool_from_workspace(&workspace, "unica.role.edit", &args).unwrap();

    assert!(result.ok, "{result:?}");
    assert_eq!(
        result
            .warnings
            .iter()
            .filter(|warning| warning.starts_with("support_guard_warning:"))
            .count(),
        1,
        "{result:?}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn role_edit_application_projects_support_block_to_typed_data() {
    let (root, workspace, args) =
        role_edit_application_workspace("unica-role-edit-support", "2.20", true);
    let rights = workspace.join("src/Roles/Demo/Ext/Rights.xml");
    let before = std::fs::read(&rights).unwrap();
    let result = call_public_tool_from_workspace(&workspace, "unica.role.edit", &args).unwrap();
    assert_typed_role_edit_failure(&result, &workspace, "Role.Demo", "support_locked");
    assert_eq!(std::fs::read(&rights).unwrap(), before);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn role_edit_application_projects_missing_logical_target_to_typed_data() {
    let (root, workspace, mut args) =
        role_edit_application_workspace("unica-role-edit-missing", "2.20", false);
    args.insert("metadataPath".to_string(), json!("Role.Missing"));
    let result = call_public_tool_from_workspace(&workspace, "unica.role.edit", &args).unwrap();
    assert_typed_role_edit_failure(&result, &workspace, "Role.Missing", "target_not_found");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn mutating_cf_edit_blocks_locked_configuration_directory_target() {
    let root = std::env::temp_dir().join(format!(
        "unica-cf-guard-dir-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let workspace = root.join("workspace");
    let src = workspace.join("src");
    let ext = src.join("Ext");
    std::fs::create_dir_all(&ext).unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    let config_path = src.join("Configuration.xml");
    std::fs::write(
        &config_path,
        support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
    )
    .unwrap();
    std::fs::write(
        ext.join("ParentConfigurations.bin"),
        support_test_parent_configurations_bin(
            "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
            "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
            "cccccccc-cccc-cccc-cccc-cccccccccccc",
        ),
    )
    .unwrap();
    let before = std::fs::read_to_string(&config_path).unwrap();
    let mut args = Map::new();
    args.insert(
        "cwd".to_string(),
        Value::String(workspace.display().to_string()),
    );
    args.insert("dryRun".to_string(), Value::Bool(false));
    args.insert("ConfigPath".to_string(), Value::String("src".to_string()));
    args.insert(
        "Operation".to_string(),
        Value::String("modify-property".to_string()),
    );
    args.insert(
        "Value".to_string(),
        Value::String("Version=2.0".to_string()),
    );

    let result = application().call_tool("unica.cf.edit", &args).unwrap();

    assert!(!result.ok);
    assert!(result.summary.contains("support guard"));
    assert!(result.errors.join("\n").contains("на замке"));
    assert_eq!(std::fs::read_to_string(&config_path).unwrap(), before);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn cf_edit_normalizes_crlf_before_lxml_compatible_write() {
    let root = std::env::temp_dir().join(format!(
        "unica-cf-crlf-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let workspace = root.join("workspace");
    let src = workspace.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    let config_path = src.join("Configuration.xml");
    let crlf_config = support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa")
        .replace('\n', "\r\n");
    assert!(crlf_config.contains("\r\n"));
    std::fs::write(&config_path, crlf_config).unwrap();

    let mut args = Map::new();
    args.insert(
        "cwd".to_string(),
        Value::String(workspace.display().to_string()),
    );
    args.insert("dryRun".to_string(), Value::Bool(false));
    args.insert("ConfigPath".to_string(), Value::String("src".to_string()));
    args.insert(
        "Operation".to_string(),
        Value::String("modify-property".to_string()),
    );
    args.insert(
        "Value".to_string(),
        Value::String("Version=2.0".to_string()),
    );
    args.insert("NoValidate".to_string(), Value::Bool(true));

    let result = application().call_tool("unica.cf.edit", &args).unwrap();

    assert!(result.ok, "{result:?}");
    let after = std::fs::read_to_string(&config_path).unwrap();
    assert!(after.contains("<Version>2.0</Version>"));
    assert!(!after.contains("&#13;"));

    let _ = std::fs::remove_dir_all(root);
}

fn cf_edit_args(workspace: &std::path::Path, operation: &str, value: &str) -> Map<String, Value> {
    let mut args = Map::new();
    args.insert(
        "cwd".to_string(),
        Value::String(workspace.display().to_string()),
    );
    args.insert("dryRun".to_string(), Value::Bool(false));
    args.insert("ConfigPath".to_string(), Value::String("src".to_string()));
    args.insert(
        "Operation".to_string(),
        Value::String(operation.to_string()),
    );
    args.insert("Value".to_string(), Value::String(value.to_string()));
    args.insert("NoValidate".to_string(), Value::Bool(true));
    args
}

fn cf_edit_mutation_workspace(prefix: &str, configuration: &[u8]) -> (PathBuf, PathBuf, PathBuf) {
    let root = test_workspace_root(prefix);
    let workspace = root.join("workspace");
    let src = workspace.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    let config_path = src.join("Configuration.xml");
    std::fs::write(&config_path, configuration).unwrap();
    (root, workspace, config_path)
}

fn cf_edit_configuration_bytes() -> Vec<u8> {
    let text = support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
    let mut bytes = b"\xef\xbb\xbf".to_vec();
    bytes.extend_from_slice(text.as_bytes());
    bytes
}

fn cf_edit_home_page_bytes() -> Vec<u8> {
    br#"<?xml version="1.0" encoding="UTF-8"?>
<HomePageWorkArea xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20">
  <WorkingAreaTemplate>OneColumn</WorkingAreaTemplate>
</HomePageWorkArea>
"#
    .to_vec()
}

fn assert_no_cf_edit_stage_debris(config_path: &std::path::Path) {
    let parent = config_path.parent().unwrap();
    let debris = std::fs::read_dir(parent)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .filter(|name| name.to_string_lossy().contains(".unica-stage-"))
        .collect::<Vec<_>>();
    assert!(debris.is_empty(), "staging debris remains: {debris:?}");
}

#[test]
fn cf_edit_preserves_unix_mode_0600() {
    let before = cf_edit_configuration_bytes();
    let (root, workspace, config_path) =
        cf_edit_mutation_workspace("unica-cf-edit-mode-0600", &before);
    if !set_unix_mode_for_test(&config_path, 0o600).unwrap() {
        eprintln!("[SKIPPED FIXTURE] Unix permission modes are unsupported on this host");
        std::fs::remove_dir_all(root).unwrap();
        return;
    }

    let result = application()
        .call_tool(
            "unica.cf.edit",
            &cf_edit_args(&workspace, "modify-property", "Version=2.0"),
        )
        .unwrap();

    assert!(result.ok, "{result:?}");
    assert_eq!(unix_mode_for_test(&config_path).unwrap(), Some(0o600));
    assert_ne!(std::fs::read(&config_path).unwrap(), before);
    assert_no_cf_edit_stage_debris(&config_path);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cf_edit_rejects_readonly_configuration_unchanged() {
    let before = cf_edit_configuration_bytes();
    let (root, workspace, config_path) =
        cf_edit_mutation_workspace("unica-cf-edit-readonly", &before);
    let exact_unix_mode = set_unix_mode_for_test(&config_path, 0o400).unwrap();
    if !exact_unix_mode {
        let mut permissions = std::fs::metadata(&config_path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&config_path, permissions).unwrap();
    }
    let mode_before = unix_mode_for_test(&config_path).unwrap();
    assert!(std::fs::metadata(&config_path)
        .unwrap()
        .permissions()
        .readonly());
    if exact_unix_mode {
        assert_eq!(mode_before, Some(0o400));
    } else {
        assert_eq!(mode_before, None);
    }

    let result = application()
        .call_tool(
            "unica.cf.edit",
            &cf_edit_args(&workspace, "modify-property", "Version=2.0"),
        )
        .unwrap();

    assert!(!result.ok, "{result:?}");
    assert!(result.errors.join("\n").contains("read-only"), "{result:?}");
    assert_eq!(std::fs::read(&config_path).unwrap(), before);
    assert!(std::fs::metadata(&config_path)
        .unwrap()
        .permissions()
        .readonly());
    assert_eq!(unix_mode_for_test(&config_path).unwrap(), mode_before);
    assert_no_cf_edit_stage_debris(&config_path);
    prepare_file_for_removal(&config_path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cf_edit_rejects_symlink_configuration_without_touching_referent() {
    let before = cf_edit_configuration_bytes();
    let root = test_workspace_root("unica-cf-edit-symlink");
    let workspace = root.join("workspace");
    let src = workspace.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    let referent = root.join("real-Configuration.xml");
    let config_path = src.join("Configuration.xml");
    std::fs::write(&referent, &before).unwrap();
    let outcome = create_file_link_fixture_for_test(&referent, &config_path)
        .expect("unexpected file-link creation error must fail the fixture test");
    match outcome {
        FileLinkFixtureOutcome::Created => {}
        FileLinkFixtureOutcome::Unsupported => {
            eprintln!("[SKIPPED FIXTURE] file links are unsupported on this host");
            std::fs::remove_dir_all(root).unwrap();
            return;
        }
        FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
            eprintln!("[SKIPPED FIXTURE] Windows file-link privilege is unavailable");
            std::fs::remove_dir_all(root).unwrap();
            return;
        }
    }
    let link_before = std::fs::read_link(&config_path).unwrap();

    let result = application()
        .call_tool(
            "unica.cf.edit",
            &cf_edit_args(&workspace, "modify-property", "Version=2.0"),
        )
        .unwrap();

    assert!(!result.ok, "{result:?}");
    assert!(
        result.errors.join("\n").contains("link or reparse point"),
        "{result:?}"
    );
    assert_eq!(std::fs::read_link(&config_path).unwrap(), link_before);
    assert_eq!(std::fs::read(&referent).unwrap(), before);
    assert_no_cf_edit_stage_debris(&config_path);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cf_edit_rejects_hard_linked_configuration_unchanged() {
    let before = cf_edit_configuration_bytes();
    let (root, workspace, config_path) =
        cf_edit_mutation_workspace("unica-cf-edit-hard-link", &before);
    let alias = config_path
        .parent()
        .unwrap()
        .join("Configuration.alias.xml");
    std::fs::hard_link(&config_path, &alias).unwrap();

    let result = application()
        .call_tool(
            "unica.cf.edit",
            &cf_edit_args(&workspace, "modify-property", "Version=2.0"),
        )
        .unwrap();

    assert!(!result.ok, "{result:?}");
    assert!(
        result.errors.join("\n").contains("hard links"),
        "{result:?}"
    );
    assert_eq!(std::fs::read(&config_path).unwrap(), before);
    assert_eq!(std::fs::read(&alias).unwrap(), before);
    assert_no_cf_edit_stage_debris(&config_path);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
pub(crate) fn cf_edit_equal_serialized_result_is_a_public_noop_and_preserves_identity() {
    let before = cf_edit_configuration_bytes();
    let (root, workspace, config_path) =
        cf_edit_mutation_workspace("unica-cf-edit-equal-noop", &before);
    let identity = file_identity_for_test(&config_path).unwrap();

    let result = application()
        .call_tool(
            "unica.cf.edit",
            &cf_edit_args(&workspace, "modify-property", "Version=1.0"),
        )
        .unwrap();

    assert!(result.ok, "{result:?}");
    assert!(result.changes.is_empty(), "{result:?}");
    assert!(result.cache.events.is_empty(), "{result:?}");
    let data = result.data.as_ref().expect("cf.edit answers with data");
    assert_eq!(data["configUpdated"], serde_json::json!(false), "{data:?}");
    assert_eq!(std::fs::read(&config_path).unwrap(), before);
    assert_eq!(file_identity_for_test(&config_path).unwrap(), identity);
    assert_no_cf_edit_stage_debris(&config_path);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn compile_transaction_and_cf_edit_share_target_lock() {
    let before = cf_edit_configuration_bytes();
    let (root, workspace, config_path) =
        cf_edit_mutation_workspace("unica-compile-cf-edit-lock", &before);
    let mut transaction = CompileTransaction::new();
    transaction
        .register_canonical_child(
            &config_path,
            "Role",
            "Reader",
            unica_source_designer::registration::cf_edit_add_child_object_text,
        )
        .expect("compile transaction must plan a registration");

    let acquired = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let acquired_in_compile = Arc::clone(&acquired);
    let release_in_compile = Arc::clone(&release);
    let compile_thread = thread::spawn(move || {
        with_publication_lock_pause(acquired_in_compile, release_in_compile, || {
            transaction.commit()
        })
    });
    acquired.wait();

    let (contended_sender, contended_receiver) = mpsc::channel();
    let workspace_in_edit = workspace.clone();
    let edit_thread = thread::spawn(move || {
        with_publication_lock_contention_signal(contended_sender, || {
            application()
                .call_tool(
                    "unica.cf.edit",
                    &cf_edit_args(&workspace_in_edit, "modify-property", "Version=1.0"),
                )
                .unwrap()
        })
    });

    let contention = contended_receiver.recv_timeout(Duration::from_secs(2));
    release.wait();
    let compile_result = compile_thread
        .join()
        .expect("compile transaction thread must not panic");
    let edit_result = edit_thread.join().expect("cf-edit thread must not panic");

    contention.expect("cf-edit must contend on the shared publisher lock");
    compile_result.expect("compile transaction must commit");
    assert!(!edit_result.ok, "{edit_result:?}");
    assert!(
        edit_result
            .errors
            .join("\n")
            .contains("differs from the expected preimage"),
        "{edit_result:?}"
    );
    let after = std::fs::read(&config_path).unwrap();
    assert_ne!(after, before);
    assert!(
        String::from_utf8_lossy(&after).contains("<Role>Reader</Role>"),
        "{}",
        String::from_utf8_lossy(&after)
    );
    assert_no_cf_edit_stage_debris(&config_path);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cf_edit_definition_file_rejects_invalid_child_object_before_sidecar_writes() {
    let mut violations = Vec::new();

    for (sidecar_operation, sidecar_value, sidecar_name, child_operation, child_value, error) in [
        (
            "set-panels",
            json!({"top": ["open"]}),
            "ClientApplicationInterface.xml",
            "add-childObject",
            "SyntheticMetadata.Unknown",
            "Unknown type 'SyntheticMetadata'",
        ),
        (
            "set-panels",
            json!({"top": ["open"]}),
            "ClientApplicationInterface.xml",
            "remove-childObject",
            "SyntheticMetadata.Unknown",
            "Unknown type 'SyntheticMetadata'",
        ),
        (
            "set-home-page",
            json!({"template": "OneColumn", "left": ["CommonForm.Demo"]}),
            "HomePageWorkArea.xml",
            "add-childObject",
            "SyntheticMetadata.Unknown",
            "Unknown type 'SyntheticMetadata'",
        ),
        (
            "set-home-page",
            json!({"template": "OneColumn", "left": ["CommonForm.Demo"]}),
            "HomePageWorkArea.xml",
            "remove-childObject",
            "SyntheticMetadata.Unknown",
            "Unknown type 'SyntheticMetadata'",
        ),
        (
            "set-panels",
            json!({"top": ["open"]}),
            "ClientApplicationInterface.xml",
            "add-childObject",
            "Catalog.",
            "Invalid format 'Catalog.', expected 'Type.Name'",
        ),
        (
            "set-panels",
            json!({"top": ["open"]}),
            "ClientApplicationInterface.xml",
            "remove-childObject",
            "Catalog.",
            "Invalid format 'Catalog.', expected 'Type.Name'",
        ),
    ] {
        let (root, workspace, _) = support_test_workspace(
            &format!("unica-cf-edit-unknown-kind-atomic-{sidecar_operation}-{child_operation}"),
            String::new(),
        );
        let config_path = workspace.join("src/Configuration.xml");
        let definition_path = workspace.join(format!("{sidecar_operation}-{child_operation}.json"));
        std::fs::write(
            &definition_path,
            serde_json::to_string(&json!([
                {"operation": sidecar_operation, "value": sidecar_value},
                {"operation": child_operation, "value": child_value}
            ]))
            .unwrap(),
        )
        .unwrap();
        let config_before = std::fs::read(&config_path).unwrap();
        let definition_before = std::fs::read(&definition_path).unwrap();
        let sidecar_path = workspace.join("src/Ext").join(sidecar_name);
        let sidecar_before = if sidecar_name == "HomePageWorkArea.xml" {
            cf_edit_home_page_bytes()
        } else {
            b"sidecar content before failed batch".to_vec()
        };
        std::fs::write(&sidecar_path, &sidecar_before).unwrap();

        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert("ConfigPath".to_string(), Value::String("src".to_string()));
        args.insert(
            "DefinitionFile".to_string(),
            Value::String(definition_path.display().to_string()),
        );
        args.insert("NoValidate".to_string(), Value::Bool(true));

        let result = application().call_tool("unica.cf.edit", &args).unwrap();

        let case = format!("{sidecar_operation} -> {child_operation} {child_value}");
        if result.ok {
            violations.push(format!("{case}: batch unexpectedly succeeded"));
        }
        if !result.errors.join("\n").contains(error) {
            violations.push(format!("{case}: wrong error: {result:?}"));
        }
        if std::fs::read(&config_path).unwrap() != config_before {
            violations.push(format!("{case}: Configuration.xml changed"));
        }
        if std::fs::read(&definition_path).unwrap() != definition_before {
            violations.push(format!("{case}: definition file changed"));
        }
        if std::fs::read(&sidecar_path).unwrap() != sidecar_before {
            violations.push(format!(
                "{case}: failed batch changed {}",
                sidecar_path.display()
            ));
        }

        let _ = std::fs::remove_dir_all(root);
    }

    assert!(
        violations.is_empty(),
        "failed batches must leave all affected files byte-identical: {violations:#?}"
    );
}

#[test]
fn cf_edit_definition_file_late_failure_is_atomic_for_external_files() {
    for preexisting_sidecars in [false, true] {
        let before = cf_edit_configuration_bytes();
        let case = if preexisting_sidecars {
            "existing-sidecars"
        } else {
            "new-sidecars"
        };
        let (root, workspace, config_path) =
            cf_edit_mutation_workspace(&format!("unica-cf-edit-late-failure-{case}"), &before);
        let definition_path = workspace.join("late-failure.json");
        std::fs::write(
            &definition_path,
            serde_json::to_vec(&json!([
                {"operation": "set-panels", "value": {"top": ["open"]}},
                {"operation": "set-home-page", "value": {"template": "OneColumn"}},
                {"operation": "modify-property", "value": "MissingEquals"}
            ]))
            .unwrap(),
        )
        .unwrap();
        let definition_before = std::fs::read(&definition_path).unwrap();
        let ext = workspace.join("src/Ext");
        let panels = ext.join("ClientApplicationInterface.xml");
        let home_page = ext.join("HomePageWorkArea.xml");
        let existing_panels = b"panel bytes before failed batch";
        let existing_home_page = cf_edit_home_page_bytes();
        if preexisting_sidecars {
            std::fs::create_dir_all(&ext).unwrap();
            std::fs::write(&panels, existing_panels).unwrap();
            std::fs::write(&home_page, &existing_home_page).unwrap();
        }

        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert("ConfigPath".to_string(), Value::String("src".to_string()));
        args.insert(
            "DefinitionFile".to_string(),
            Value::String(definition_path.display().to_string()),
        );
        args.insert("NoValidate".to_string(), Value::Bool(true));

        let result = application().call_tool("unica.cf.edit", &args).unwrap();

        assert!(!result.ok, "{case}: {result:?}");
        assert!(
            result.errors.join("\n").contains("Invalid property format"),
            "{case}: {result:?}"
        );
        assert_eq!(std::fs::read(&config_path).unwrap(), before, "{case}");
        assert_eq!(
            std::fs::read(&definition_path).unwrap(),
            definition_before,
            "{case}"
        );
        if preexisting_sidecars {
            assert_eq!(std::fs::read(&panels).unwrap(), existing_panels, "{case}");
            assert_eq!(
                std::fs::read(&home_page).unwrap(),
                existing_home_page,
                "{case}"
            );
        } else {
            assert!(!ext.exists(), "{case}: {} was created", ext.display());
        }

        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn cf_edit_external_files_publish_for_single_and_combined_batches() {
    for (case, operations, expected_files) in [
        (
            "panels",
            json!([
                {"operation": "set-panels", "value": {"top": ["open"]}}
            ]),
            vec!["ClientApplicationInterface.xml"],
        ),
        (
            "home-page",
            json!([
                {"operation": "set-home-page", "value": {"template": "OneColumn"}}
            ]),
            vec!["HomePageWorkArea.xml"],
        ),
        (
            "combined",
            json!([
                {"operation": "set-panels", "value": {"top": ["open"]}},
                {"operation": "set-home-page", "value": {"template": "OneColumn"}}
            ]),
            vec!["ClientApplicationInterface.xml", "HomePageWorkArea.xml"],
        ),
    ] {
        let before = cf_edit_configuration_bytes();
        let (root, workspace, config_path) =
            cf_edit_mutation_workspace(&format!("unica-cf-edit-external-success-{case}"), &before);
        let definition_path = workspace.join(format!("{case}.json"));
        std::fs::write(&definition_path, serde_json::to_vec(&operations).unwrap()).unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert("ConfigPath".to_string(), Value::String("src".to_string()));
        args.insert(
            "DefinitionFile".to_string(),
            Value::String(definition_path.display().to_string()),
        );
        args.insert("NoValidate".to_string(), Value::Bool(true));

        let result = application().call_tool("unica.cf.edit", &args).unwrap();

        assert!(result.ok, "{case}: {result:?}");
        assert_eq!(std::fs::read(&config_path).unwrap(), before, "{case}");
        assert_eq!(
            result.changes.len(),
            expected_files.len(),
            "{case}: {result:?}"
        );
        assert_eq!(
            result.artifacts.len(),
            expected_files.len() + 1,
            "{case}: {result:?}"
        );
        for file_name in expected_files {
            let path = workspace.join("src/Ext").join(file_name);
            let bytes = std::fs::read(&path).unwrap();
            assert!(
                bytes.starts_with(b"\xef\xbb\xbf"),
                "{case}: {}",
                path.display()
            );
            roxmltree::Document::parse(std::str::from_utf8(&bytes[3..]).unwrap())
                .unwrap_or_else(|error| panic!("{case}: {}: {error}", path.display()));
            assert!(
                result
                    .changes
                    .iter()
                    .map(|change| change.replace('\\', "/"))
                    .any(|change| change == format!("updated {}", path_text(&path))),
                "{case}: {result:?}"
            );
        }

        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn cf_edit_combined_batch_replaces_external_files_and_updates_configuration() {
    let before = cf_edit_configuration_bytes();
    let (root, workspace, config_path) =
        cf_edit_mutation_workspace("unica-cf-edit-external-replace-combined", &before);
    let ext = workspace.join("src/Ext");
    std::fs::create_dir_all(&ext).unwrap();
    let panels = ext.join("ClientApplicationInterface.xml");
    let home_page = ext.join("HomePageWorkArea.xml");
    let panels_before = b"old panel bytes";
    let home_page_before = cf_edit_home_page_bytes();
    std::fs::write(&panels, panels_before).unwrap();
    std::fs::write(&home_page, &home_page_before).unwrap();
    let definition_path = workspace.join("combined-replace.json");
    std::fs::write(
        &definition_path,
        serde_json::to_vec(&json!([
            {"operation": "modify-property", "value": "Version=2.0"},
            {"operation": "set-panels", "value": {"top": ["open"]}},
            {"operation": "set-home-page", "value": {"template": "OneColumn"}}
        ]))
        .unwrap(),
    )
    .unwrap();
    let mut args = Map::new();
    args.insert(
        "cwd".to_string(),
        Value::String(workspace.display().to_string()),
    );
    args.insert("dryRun".to_string(), Value::Bool(false));
    args.insert("ConfigPath".to_string(), Value::String("src".to_string()));
    args.insert(
        "DefinitionFile".to_string(),
        Value::String(definition_path.display().to_string()),
    );
    args.insert("NoValidate".to_string(), Value::Bool(true));

    let result = application().call_tool("unica.cf.edit", &args).unwrap();

    assert!(result.ok, "{result:?}");
    assert_eq!(result.changes.len(), 3, "{result:?}");
    let config_after = std::fs::read(&config_path).unwrap();
    assert_ne!(config_after, before);
    assert!(String::from_utf8_lossy(&config_after).contains("<Version>2.0</Version>"));
    for (path, old_bytes) in [
        (&panels, panels_before.as_slice()),
        (&home_page, home_page_before.as_slice()),
    ] {
        let bytes = std::fs::read(path).unwrap();
        assert_ne!(bytes, old_bytes, "{}", path.display());
        assert!(bytes.starts_with(b"\xef\xbb\xbf"), "{}", path.display());
        roxmltree::Document::parse(std::str::from_utf8(&bytes[3..]).unwrap()).unwrap();
    }

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cf_edit_late_config_publication_failure_leaves_external_files_absent() {
    let before = cf_edit_configuration_bytes();
    let (root, workspace, config_path) =
        cf_edit_mutation_workspace("unica-cf-edit-external-config-failure", &before);
    let alias = workspace.join("Configuration.alias.xml");
    std::fs::hard_link(&config_path, &alias).unwrap();
    let definition_path = workspace.join("config-failure.json");
    std::fs::write(
        &definition_path,
        serde_json::to_vec(&json!([
            {"operation": "set-panels", "value": {"top": ["open"]}},
            {"operation": "set-home-page", "value": {"template": "OneColumn"}},
            {"operation": "modify-property", "value": "Version=2.0"}
        ]))
        .unwrap(),
    )
    .unwrap();
    let mut args = Map::new();
    args.insert(
        "cwd".to_string(),
        Value::String(workspace.display().to_string()),
    );
    args.insert("dryRun".to_string(), Value::Bool(false));
    args.insert("ConfigPath".to_string(), Value::String("src".to_string()));
    args.insert(
        "DefinitionFile".to_string(),
        Value::String(definition_path.display().to_string()),
    );
    args.insert("NoValidate".to_string(), Value::Bool(true));

    let result = application().call_tool("unica.cf.edit", &args).unwrap();

    assert!(!result.ok, "{result:?}");
    assert!(
        result.errors.join("\n").contains("hard links"),
        "{result:?}"
    );
    assert_eq!(std::fs::read(&config_path).unwrap(), before);
    assert_eq!(std::fs::read(&alias).unwrap(), before);
    assert!(!workspace.join("src/Ext").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cf_edit_definition_file_keeps_valid_ordered_child_object_batch() {
    let (root, workspace, _) =
        support_test_workspace("unica-cf-edit-known-kind-batch", String::new());
    let definition_path = workspace.join("ordered-batch.json");
    std::fs::write(
        &definition_path,
        serde_json::to_string(&json!([
            {"operation": "set-panels", "value": {"top": ["open"]}},
            {"operation": "remove-childObject", "value": "Catalog.Items"},
            {"operation": "add-childObject", "value": "Catalog.Items"}
        ]))
        .unwrap(),
    )
    .unwrap();
    let mut args = Map::new();
    args.insert(
        "cwd".to_string(),
        Value::String(workspace.display().to_string()),
    );
    args.insert("dryRun".to_string(), Value::Bool(false));
    args.insert("ConfigPath".to_string(), Value::String("src".to_string()));
    args.insert(
        "DefinitionFile".to_string(),
        Value::String(definition_path.display().to_string()),
    );
    args.insert("NoValidate".to_string(), Value::Bool(true));

    let result = application().call_tool("unica.cf.edit", &args).unwrap();

    assert!(result.ok, "{result:?}");
    assert!(workspace
        .join("src/Ext/ClientApplicationInterface.xml")
        .is_file());
    assert!(
        std::fs::read_to_string(workspace.join("src/Configuration.xml"))
            .unwrap()
            .contains("<Catalog>Items</Catalog>")
    );

    let _ = std::fs::remove_dir_all(root);
}

fn cf_edit_issue55_config_xml(child_indent: &str) -> String {
    format!(
        concat!(
            "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">\r\n",
            "\t<Configuration uuid=\"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\">\r\n",
            "\t\t<InternalInfo/>\r\n",
            "\t\t<Properties>\r\n",
            "\t\t\t<Name>Issue55</Name>\r\n",
            "\t\t\t<DefaultLanguage>Russian</DefaultLanguage>\r\n",
            "\t\t</Properties>\r\n",
            "\t\t<ChildObjects>\r\n",
            "{0}<Language>Russian</Language>\r\n",
            "{0}<StyleItem>НепринятаяВерсия</StyleItem>\r\n",
            "{0}<StyleItem>НеПринятыеКИсполнениюЗадачи</StyleItem>\r\n",
            "{0}<StyleItem>НерабочийПериодПроизводственногоКалендаряФон</StyleItem>\r\n",
            "{0}<CommonPicture>Минимум</CommonPicture>\r\n",
            "{0}<CommonPicture>МЧДАктивна</CommonPicture>\r\n",
            "{0}<Catalog>Валюты</Catalog>\r\n",
            "{0}<Catalog>ВариантыОтветовАнкет</Catalog>\r\n",
            "\t\t</ChildObjects>\r\n",
            "\t</Configuration>\r\n",
            "</MetaDataObject>\r\n"
        ),
        child_indent
    )
}

fn bot_configuration_xml(include_bot: bool) -> String {
    let children = if include_bot {
        concat!(
            "\t\t\t<Language>Русский</Language>\n",
            "\t\t\t<CommonModule>Core</CommonModule>\n",
            "\t\t\t<CommonAttribute>Shared</CommonAttribute>\n",
            "\t\t\t<Bot>Assistant</Bot>"
        )
    } else {
        concat!(
            "\t\t\t<Language>Русский</Language>\n",
            "\t\t\t<CommonModule>Core</CommonModule>\n",
            "\t\t\t<CommonAttribute>Shared</CommonAttribute>"
        )
    };
    include_str!("../../../../tests/fixtures/unica_mcp_script_parity/cf-validate/Configuration.xml")
        .replace("\r\n", "\n")
        .replace("version=\"2.17\"", "version=\"2.20\"")
        .replace("\t\t\t<Language>Русский</Language>", children)
}

fn bot_cf_workspace(prefix: &str, include_bot: bool) -> (PathBuf, PathBuf, PathBuf) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("{prefix}-{nanos}"));
    let workspace = root.join("workspace");
    let src = workspace.join("src");
    for directory in ["Languages", "CommonModules", "Bots", "CommonAttributes"] {
        std::fs::create_dir_all(src.join(directory)).unwrap();
    }
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    let config_path = src.join("Configuration.xml");
    std::fs::write(
        &config_path,
        format!("\u{feff}{}", bot_configuration_xml(include_bot)),
    )
    .unwrap();
    std::fs::write(
        src.join("Languages/Русский.xml"),
        include_str!(
            "../../../../tests/fixtures/unica_mcp_script_parity/cf-validate/Languages/Русский.xml"
        ),
    )
    .unwrap();
    if include_bot {
        std::fs::write(src.join("Bots/Assistant.xml"), "<MetaDataObject/>").unwrap();
    }
    (root, workspace, config_path)
}

#[test]
fn cf_edit_adds_removes_and_noops_bot_through_registry() {
    let (root, workspace, config_path) = bot_cf_workspace("unica-cf-bot-edit", false);
    let src = workspace.join("src");
    std::fs::write(src.join("Bots/Assistant.xml"), "<MetaDataObject/>").unwrap();
    let before = std::fs::read_to_string(&config_path).unwrap();

    let add = application()
        .call_tool(
            "unica.cf.edit",
            &cf_edit_args(&workspace, "add-childObject", "Bot.Assistant"),
        )
        .unwrap();
    assert!(add.ok, "{add:?}");
    let after_add = std::fs::read_to_string(&config_path).unwrap();
    // The platform writes Bot after DefinedType, so after CommonAttribute.
    assert!(
        after_add
            .find("<CommonAttribute>Shared</CommonAttribute>")
            .unwrap()
            < after_add.find("<Bot>Assistant</Bot>").unwrap()
    );

    let duplicate = application()
        .call_tool(
            "unica.cf.edit",
            &cf_edit_args(&workspace, "add-childObject", "Bot.Assistant"),
        )
        .unwrap();
    assert!(duplicate.ok, "{duplicate:?}");
    assert!(duplicate.changes.is_empty(), "{duplicate:?}");
    assert!(duplicate.cache.events.is_empty(), "{duplicate:?}");
    assert_eq!(std::fs::read_to_string(&config_path).unwrap(), after_add);

    let remove = application()
        .call_tool(
            "unica.cf.edit",
            &cf_edit_args(&workspace, "remove-childObject", "Bot.Assistant"),
        )
        .unwrap();
    assert!(remove.ok, "{remove:?}");
    assert_eq!(std::fs::read_to_string(&config_path).unwrap(), before);

    let missing = application()
        .call_tool(
            "unica.cf.edit",
            &cf_edit_args(&workspace, "add-childObject", "Bot.Missing"),
        )
        .unwrap();
    assert!(!missing.ok, "{missing:?}");
    let missing_errors = missing.errors.join("\n");
    assert!(missing_errors.contains("Bots/Missing.xml"), "{missing:?}");
    assert!(!missing_errors.contains("use meta-compile"), "{missing:?}");
    assert_eq!(std::fs::read_to_string(&config_path).unwrap(), before);

    let unknown = application()
        .call_tool(
            "unica.cf.edit",
            &cf_edit_args(
                &workspace,
                "remove-childObject",
                "SyntheticMetadata.Unknown",
            ),
        )
        .unwrap();
    assert!(!unknown.ok, "{unknown:?}");
    assert!(
        unknown
            .errors
            .join("\n")
            .contains("Unknown type 'SyntheticMetadata'"),
        "{unknown:?}"
    );
    assert_eq!(std::fs::read_to_string(&config_path).unwrap(), before);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn cf_edit_add_child_object_does_not_escape_structural_crlf() {
    let root = std::env::temp_dir().join(format!(
        "unica-cf-child-crlf-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let workspace = root.join("workspace");
    let src = workspace.join("src");
    let catalogs = src.join("Catalogs");
    std::fs::create_dir_all(&catalogs).unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    let config_path = src.join("Configuration.xml");
    let crlf_config = support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa")
        .replace('\n', "\r\n");
    std::fs::write(&config_path, crlf_config).unwrap();
    std::fs::write(
        catalogs.join("Extra.xml"),
        support_test_catalog_xml("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
    )
    .unwrap();

    let mut args = Map::new();
    args.insert(
        "cwd".to_string(),
        Value::String(workspace.display().to_string()),
    );
    args.insert("dryRun".to_string(), Value::Bool(false));
    args.insert("ConfigPath".to_string(), Value::String("src".to_string()));
    args.insert(
        "Operation".to_string(),
        Value::String("add-childObject".to_string()),
    );
    args.insert(
        "Value".to_string(),
        Value::String("Catalog.Extra".to_string()),
    );
    args.insert("NoValidate".to_string(), Value::Bool(true));

    let result = application().call_tool("unica.cf.edit", &args).unwrap();

    assert!(result.ok, "{result:?}");
    let after_bytes = std::fs::read(&config_path).unwrap();
    let after = String::from_utf8(after_bytes.clone()).unwrap();
    assert!(after.starts_with('\u{feff}'));
    assert!(after.contains("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
    assert!(after.contains("<Catalog>Extra</Catalog>"));
    assert!(!after.contains("&#13;"), "{after}");
    assert!(
        after_bytes
            .iter()
            .enumerate()
            .filter(|(_, byte)| **byte == b'\n')
            .all(|(index, _)| index > 0 && after_bytes[index - 1] == b'\r'),
        "{after}"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn cf_edit_remove_add_child_object_preserves_neighboring_childobjects() {
    let root = std::env::temp_dir().join(format!(
        "unica-cf-issue55-roundtrip-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let workspace = root.join("workspace");
    let src = workspace.join("src");
    let catalogs = src.join("Catalogs");
    std::fs::create_dir_all(&catalogs).unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    let config_path = src.join("Configuration.xml");
    let before = cf_edit_issue55_config_xml("\t\t\t\t\t");
    std::fs::write(&config_path, before.as_bytes()).unwrap();
    std::fs::write(
        catalogs.join("Валюты.xml"),
        support_test_catalog_xml("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
    )
    .unwrap();

    let remove = application()
        .call_tool(
            "unica.cf.edit",
            &cf_edit_args(&workspace, "remove-childObject", "Catalog.Валюты"),
        )
        .unwrap();
    assert!(remove.ok, "{remove:?}");

    let add = application()
        .call_tool(
            "unica.cf.edit",
            &cf_edit_args(&workspace, "add-childObject", "Catalog.Валюты"),
        )
        .unwrap();
    assert!(add.ok, "{add:?}");

    let after = std::fs::read_to_string(&config_path).unwrap();
    assert_eq!(after, before);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn cf_edit_child_object_roundtrip_preserves_trailing_blank_lines() {
    fn trailer_after_root(text: &str) -> &str {
        let marker = "</MetaDataObject>";
        let root_end = text.rfind(marker).unwrap() + marker.len();
        &text[root_end..]
    }

    let root = std::env::temp_dir().join(format!(
        "unica-cf-issue55-trailer-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let workspace = root.join("workspace");
    let src = workspace.join("src");
    let catalogs = src.join("Catalogs");
    std::fs::create_dir_all(&catalogs).unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    let config_path = src.join("Configuration.xml");
    let before = format!("{}\r\n\r\n", cf_edit_issue55_config_xml("\t\t\t\t\t"));
    assert_eq!(trailer_after_root(&before), "\r\n\r\n\r\n");
    std::fs::write(&config_path, before.as_bytes()).unwrap();
    std::fs::write(
        catalogs.join("Валюты.xml"),
        support_test_catalog_xml("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
    )
    .unwrap();

    let remove = application()
        .call_tool(
            "unica.cf.edit",
            &cf_edit_args(&workspace, "remove-childObject", "Catalog.Валюты"),
        )
        .unwrap();
    assert!(remove.ok, "{remove:?}");
    let after_remove = std::fs::read_to_string(&config_path).unwrap();
    assert_eq!(trailer_after_root(&after_remove), "\r\n\r\n\r\n");

    let add = application()
        .call_tool(
            "unica.cf.edit",
            &cf_edit_args(&workspace, "add-childObject", "Catalog.Валюты"),
        )
        .unwrap();
    assert!(add.ok, "{add:?}");

    let after = std::fs::read_to_string(&config_path).unwrap();
    assert_eq!(after, before);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn cf_edit_duplicate_add_child_object_does_not_rewrite_configuration() {
    let root = std::env::temp_dir().join(format!(
        "unica-cf-issue55-noop-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let workspace = root.join("workspace");
    let src = workspace.join("src");
    let catalogs = src.join("Catalogs");
    std::fs::create_dir_all(&catalogs).unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    let config_path = src.join("Configuration.xml");
    let before = cf_edit_issue55_config_xml("\t\t\t\t\t");
    std::fs::write(&config_path, before.as_bytes()).unwrap();
    std::fs::write(
        catalogs.join("Валюты.xml"),
        support_test_catalog_xml("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
    )
    .unwrap();

    let result = application()
        .call_tool(
            "unica.cf.edit",
            &cf_edit_args(&workspace, "add-childObject", "Catalog.Валюты"),
        )
        .unwrap();

    assert!(result.ok, "{result:?}");
    assert!(result.changes.is_empty(), "{result:?}");
    assert!(result.cache.events.is_empty(), "{result:?}");
    let data = result.data.as_ref().expect("cf.edit answers with data");
    assert_eq!(data["configUpdated"], serde_json::json!(false), "{data:?}");
    let skipped = data["operations"]
        .as_array()
        .expect("operations is a list")
        .iter()
        .find(|item| item["target"] == serde_json::json!("Catalog.Валюты"))
        .expect("the duplicate add is reported");
    assert_eq!(skipped["applied"], serde_json::json!(false), "{skipped:?}");
    assert_eq!(std::fs::read_to_string(&config_path).unwrap(), before);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn declared_existing_form_output_rejects_wrong_root_before_handler() {
    let root = test_workspace_root("unica-form-existing-wrong-root");
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let definition = workspace.join("form.json");
    std::fs::write(&definition, "{}").unwrap();
    let output = workspace.join("Form.xml");
    std::fs::write(&output, b"<garbage/>").unwrap();
    let before = std::fs::read(&output).unwrap();
    let args = Map::from_iter([
        (
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        ),
        (
            "JsonPath".to_string(),
            Value::String(definition.display().to_string()),
        ),
        (
            "OutputPath".to_string(),
            Value::String(output.display().to_string()),
        ),
        ("dryRun".to_string(), Value::Bool(false)),
    ]);

    let result = application()
        .call_tool("unica.form.compile", &args)
        .unwrap();

    assert!(!result.ok, "{result:?}");
    let diagnostic = &result.diagnostics.as_ref().unwrap()["formatCompatibility"];
    assert_eq!(diagnostic["code"], "formatVersionInvalid", "{result:?}");
    assert_eq!(diagnostic["compatibility"], "invalid", "{result:?}");
    assert_eq!(std::fs::read(&output).unwrap(), before);
    assert!(result.changes.is_empty(), "{result:?}");
    assert!(result.artifacts.is_empty(), "{result:?}");
    assert!(result.cache.events.is_empty(), "{result:?}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn declared_form_output_with_nonstandard_suffix_still_blocks_newer_owner() {
    for (index, file_name) in ["Form.XML", "Form"].into_iter().enumerate() {
        let root = test_workspace_root(&format!("unica-form-existing-newer-nonstandard-{index}"));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let definition = workspace.join("form.json");
        std::fs::write(&definition, "{}").unwrap();
        let output = workspace.join(file_name);
        std::fs::write(
            &output,
            br#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.23"/>"#,
        )
        .unwrap();
        let before = std::fs::read(&output).unwrap();
        let args = Map::from_iter([
            (
                "cwd".to_string(),
                Value::String(workspace.display().to_string()),
            ),
            (
                "JsonPath".to_string(),
                Value::String(definition.display().to_string()),
            ),
            (
                "OutputPath".to_string(),
                Value::String(output.display().to_string()),
            ),
            ("dryRun".to_string(), Value::Bool(false)),
        ]);

        let result = application()
            .call_tool("unica.form.compile", &args)
            .unwrap();

        assert!(!result.ok, "{file_name}: {result:?}");
        let diagnostic = &result.diagnostics.as_ref().unwrap()["formatCompatibility"];
        assert_eq!(
            diagnostic["code"], "platformVersionUnsupported",
            "{file_name}: {result:?}"
        );
        assert_eq!(
            diagnostic["actualFormat"], "2.23",
            "{file_name}: {result:?}"
        );
        assert_eq!(std::fs::read(&output).unwrap(), before, "{file_name}");
        assert!(result.changes.is_empty(), "{file_name}: {result:?}");
        assert!(result.artifacts.is_empty(), "{file_name}: {result:?}");
        assert!(result.cache.events.is_empty(), "{file_name}: {result:?}");
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn cf_edit_validation_dependencies_block_incompatible_home_page_file() {
    let newer_home_page = String::from_utf8(cf_edit_home_page_bytes())
        .unwrap()
        .replacen(r#"version="2.20""#, r#"version="2.23""#, 1)
        .into_bytes();
    let cases = [
        (
            "modify-newer",
            "modify-property",
            "Version=2.0",
            newer_home_page.as_slice(),
        ),
        (
            "modify-malformed",
            "modify-property",
            "Version=2.0",
            b"<not-valid-xml".as_slice(),
        ),
        (
            "panels-newer",
            "set-panels",
            r#"{"top":["open"]}"#,
            newer_home_page.as_slice(),
        ),
        (
            "panels-malformed",
            "set-panels",
            r#"{"top":["open"]}"#,
            b"<not-valid-xml".as_slice(),
        ),
    ];
    for (label, operation, value, home_page_bytes) in cases {
        let (root, workspace, config_path) = cf_edit_mutation_workspace(
            &format!("unica-cf-edit-unrelated-home-page-{label}"),
            &cf_edit_configuration_bytes(),
        );
        let home_page_path = config_path
            .parent()
            .unwrap()
            .join("Ext/HomePageWorkArea.xml");
        std::fs::create_dir_all(home_page_path.parent().unwrap()).unwrap();
        std::fs::write(&home_page_path, home_page_bytes).unwrap();
        let home_page_before = std::fs::read(&home_page_path).unwrap();
        let config_before = std::fs::read(&config_path).unwrap();
        let panels_path = config_path
            .parent()
            .unwrap()
            .join("Ext/ClientApplicationInterface.xml");

        let result = application()
            .call_tool("unica.cf.edit", &cf_edit_args(&workspace, operation, value))
            .unwrap();

        assert!(!result.ok, "{label}: {result:?}");
        let diagnostic = &result.diagnostics.as_ref().unwrap()["formatCompatibility"];
        let expected_code = if label.ends_with("newer") {
            "platformVersionUnsupported"
        } else {
            "formatVersionInvalid"
        };
        assert_eq!(diagnostic["code"], expected_code, "{label}: {result:?}");
        assert_eq!(
            std::fs::read(&home_page_path).unwrap(),
            home_page_before,
            "{label}"
        );
        assert_eq!(
            std::fs::read(&config_path).unwrap(),
            config_before,
            "{label}"
        );
        assert!(!panels_path.exists(), "{label}: {result:?}");
        assert!(result.changes.is_empty(), "{label}: {result:?}");
        assert!(result.artifacts.is_empty(), "{label}: {result:?}");
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn cf_edit_add_child_object_prioritizes_newer_existing_target_descriptor() {
    let (root, workspace, config_path) = cf_edit_mutation_workspace(
        "unica-cf-edit-add-child-newer-target",
        &cf_edit_configuration_bytes(),
    );
    let older_configuration = std::fs::read_to_string(&config_path).unwrap().replacen(
        r#"version="2.20""#,
        r#"version="2.19""#,
        1,
    );
    std::fs::write(&config_path, older_configuration).unwrap();
    let target_path = config_path.parent().unwrap().join("Catalogs/Future.xml");
    std::fs::create_dir_all(target_path.parent().unwrap()).unwrap();
    let newer_target = support_test_catalog_xml("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb")
        .replacen(r#"version="2.20""#, r#"version="2.23""#, 1)
        .replacen("<Name>Items</Name>", "<Name>Future</Name>", 1);
    std::fs::write(&target_path, newer_target).unwrap();
    let config_before = std::fs::read(&config_path).unwrap();
    let target_before = std::fs::read(&target_path).unwrap();

    let result = application()
        .call_tool(
            "unica.cf.edit",
            &cf_edit_args(&workspace, "add-childObject", "Catalog.Future"),
        )
        .unwrap();

    assert!(!result.ok, "{result:?}");
    let diagnostic = &result.diagnostics.as_ref().unwrap()["formatCompatibility"];
    assert_eq!(diagnostic["code"], "platformVersionUnsupported");
    assert_eq!(diagnostic["actualFormat"], "2.23");
    assert_eq!(diagnostic["compatibility"], "newer");
    assert_eq!(
        diagnostic["root"],
        normalized_path(&target_path).display().to_string(),
        "{result:?}"
    );
    let errors = result.errors.join("\n");
    assert!(errors.contains("1С 8.5"), "{result:?}");
    assert!(!errors.contains("старше поддерживаемого"), "{result:?}");
    assert_eq!(std::fs::read(&config_path).unwrap(), config_before);
    assert_eq!(std::fs::read(&target_path).unwrap(), target_before);
    assert!(result.changes.is_empty(), "{result:?}");
    assert!(result.artifacts.is_empty(), "{result:?}");
    assert!(result.cache.events.is_empty(), "{result:?}");
    std::fs::remove_dir_all(root).unwrap();
}

fn support_test_configuration_xml(uuid: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
  <Configuration uuid="{uuid}">
<InternalInfo/>
<Properties>
  <Name>Demo</Name>
  <Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Demo</v8:content></v8:item></Synonym>
  <Version>1.0</Version>
  <Vendor>Vendor</Vendor>
  <CompatibilityMode>Version8_3_24</CompatibilityMode>
  <DefaultRunMode>ManagedApplication</DefaultRunMode>
  <ScriptVariant>Russian</ScriptVariant>
  <DefaultLanguage>Russian</DefaultLanguage>
  <DataLockControlMode>Managed</DataLockControlMode>
  <ModalityUseMode>DontUse</ModalityUseMode>
  <InterfaceCompatibilityMode>Taxi</InterfaceCompatibilityMode>
</Properties>
<ChildObjects><Language>Russian</Language><Catalog>Items</Catalog></ChildObjects>
  </Configuration>
</MetaDataObject>"#
    )
}

fn write_support_test_language(src: &std::path::Path) {
    let languages = src.join("Languages");
    std::fs::create_dir_all(&languages).unwrap();
    std::fs::write(
        languages.join("Russian.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
  <Language uuid="eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee">
<Properties>
  <Name>Russian</Name>
  <Synonym/>
  <Comment/>
  <LanguageCode>ru</LanguageCode>
</Properties>
  </Language>
</MetaDataObject>"#,
    )
    .unwrap();
}

fn assert_support_guard_block_parity(applied: &OperationResult, preview: &OperationResult) {
    assert_eq!(preview.summary, format!("dry run: {}", applied.summary));
    assert_eq!(preview.ok, applied.ok);
    assert_eq!(preview.changes, applied.changes);
    assert_eq!(preview.warnings, applied.warnings);
    assert_eq!(preview.errors, applied.errors);
    assert_eq!(preview.artifacts, applied.artifacts);
    assert_eq!(preview.stdout, applied.stdout);
    assert_eq!(preview.stderr, applied.stderr);
    assert_eq!(preview.command, applied.command);
    assert_eq!(preview.diagnostics, applied.diagnostics);
    assert_eq!(preview.data, applied.data);
    assert_eq!(preview.job, applied.job);
    assert_eq!(preview.cache.mode, applied.cache.mode);
    assert_eq!(preview.cache.root, applied.cache.root);
    assert_eq!(preview.cache.workspace_epoch, applied.cache.workspace_epoch);
    assert_eq!(preview.cache.events, applied.cache.events);
    assert_eq!(preview.cache.invalidated, applied.cache.invalidated);
    assert_eq!(preview.cache.refreshed, applied.cache.refreshed);
    assert_eq!(preview.cache.lazy_rebuilt, applied.cache.lazy_rebuilt);
    assert_eq!(preview.cache.stale, applied.cache.stale);
    assert_eq!(preview.cache.fresh, applied.cache.fresh);
}

fn support_test_workspace(
    prefix: &str,
    parent_configurations_bin: String,
) -> (PathBuf, PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let workspace = root.join("workspace");
    let src = workspace.join("src");
    let ext = src.join("Ext");
    let catalogs = src.join("Catalogs");
    std::fs::create_dir_all(&ext).unwrap();
    std::fs::create_dir_all(&catalogs).unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(
        src.join("Configuration.xml"),
        support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
    )
    .unwrap();
    write_support_test_language(&src);
    std::fs::write(
        catalogs.join("Items.xml"),
        support_test_catalog_xml("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
    )
    .unwrap();
    let bin_path = ext.join("ParentConfigurations.bin");
    std::fs::write(&bin_path, parent_configurations_bin).unwrap();
    (root, workspace, bin_path)
}

fn support_test_parent_configurations_bin(
    config_uuid: &str,
    locked_uuid: &str,
    removed_uuid: &str,
) -> String {
    format!(
        "\u{feff}{{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",\"VendorConf\",3,1,0,{config_uuid},{config_uuid},0,0,{locked_uuid},{locked_uuid},2,0,{removed_uuid},{removed_uuid}}}"
    )
}

#[test]
fn native_xml_metadata_tools_reject_edt_source_set_targets() {
    let root = std::env::temp_dir().join(format!(
        "unica-xml-tool-edt-guard-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let workspace = root.join("workspace");
    std::fs::create_dir_all(workspace.join("src/Configuration")).unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: EDT\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(workspace.join("src/.project"), "<projectDescription/>").unwrap();
    std::fs::write(
        workspace.join("src/Configuration/Configuration.mdo"),
        "<mdclass:Configuration/>",
    )
    .unwrap();
    let mut args = Map::new();
    args.insert(
        "cwd".to_string(),
        Value::String(workspace.display().to_string()),
    );
    args.insert(
        "TemplatePath".to_string(),
        Value::String("src/Reports/Sales/Templates/Sheet/Ext/Template.xml".to_string()),
    );

    let error = match application().call_tool("unica.mxl.info", &args) {
        Ok(result) => panic!("expected EDT source-set guard, got {}", result.summary),
        Err(error) => error,
    };

    assert!(error.contains("sourceFormat=edt"));
    assert!(error.contains("platform_xml"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn native_xml_metadata_tools_reject_ambiguous_source_set_targets() {
    let root = std::env::temp_dir().join(format!(
        "unica-xml-tool-ambiguous-guard-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let workspace = root.join("workspace");
    std::fs::create_dir_all(workspace.join("src")).unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(workspace.join("src/.project"), "<projectDescription/>").unwrap();
    std::fs::write(workspace.join("src/Configuration.xml"), "not XML").unwrap();
    let args = Map::from_iter([
        (
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        ),
        (
            "TemplatePath".to_string(),
            Value::String("src/Reports/Sales/Templates/Sheet/Ext/Template.xml".to_string()),
        ),
    ]);

    let error = application()
        .call_tool("unica.mxl.info", &args)
        .expect_err("ambiguous source format must fail before XML parsing");

    assert!(error.contains("invalid/ambiguous format"), "{error}");
    assert!(error.contains("platform_xml"), "{error}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn read_only_native_outfile_is_rejected_before_any_write() {
    let root = std::env::temp_dir().join(format!(
        "unica-read-outfile-write-guard-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let workspace = root.join("workspace");
    let outside = root.join("outside").join("report.txt");
    std::fs::create_dir_all(workspace.join("src")).unwrap();
    std::fs::create_dir_all(outside.parent().unwrap()).unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(
        workspace.join("src/Configuration.xml"),
        support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
    )
    .unwrap();
    let mut args = Map::new();
    args.insert(
        "cwd".to_string(),
        Value::String(workspace.display().to_string()),
    );
    args.insert(
        "TemplatePath".to_string(),
        Value::String("src/Reports/Sales/Templates/Sheet/Ext/Template.xml".to_string()),
    );
    args.insert(
        "OutFile".to_string(),
        Value::String(outside.display().to_string()),
    );

    let error = match application().call_tool("unica.mxl.info", &args) {
        Ok(result) => panic!(
            "expected OutFile contract rejection, got {}",
            result.summary
        ),
        Err(error) => error,
    };

    assert!(
        error.contains("does not accept argument `OutFile`"),
        "{error}"
    );
    assert!(!outside.exists());

    let _ = std::fs::remove_dir_all(root);
}
#[test]
fn cfe_borrow_rejects_edt_config_source_set_target() {
    let root = std::env::temp_dir().join(format!(
        "unica-cfe-borrow-edt-guard-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let workspace = root.join("workspace");
    std::fs::create_dir_all(workspace.join("cfg/Configuration")).unwrap();
    std::fs::create_dir_all(workspace.join("ext")).unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: EDT\nsource-set:\n  - name: cfg\n    type: CONFIGURATION\n    path: cfg\n  - name: ext\n    type: EXTENSION\n    path: ext\n",
    )
    .unwrap();
    std::fs::write(workspace.join("cfg/.project"), "<projectDescription/>").unwrap();
    std::fs::write(
        workspace.join("cfg/Configuration/Configuration.mdo"),
        "<mdclass:Configuration/>",
    )
    .unwrap();
    std::fs::write(
        workspace.join("ext/Configuration.xml"),
        support_test_configuration_xml("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
    )
    .unwrap();
    let mut args = Map::new();
    args.insert(
        "cwd".to_string(),
        Value::String(workspace.display().to_string()),
    );
    args.insert("dryRun".to_string(), Value::Bool(false));
    args.insert(
        "ExtensionPath".to_string(),
        Value::String("ext/Configuration.xml".to_string()),
    );
    args.insert(
        "ConfigPath".to_string(),
        Value::String("cfg/Configuration.xml".to_string()),
    );
    args.insert(
        "Object".to_string(),
        Value::String("Catalog.Items".to_string()),
    );

    let error = match application().call_tool("unica.cfe.borrow", &args) {
        Ok(result) => panic!("expected EDT source-set guard, got {}", result.summary),
        Err(error) => error,
    };

    assert!(error.contains("source-set `cfg`"), "{error}");
    assert!(error.contains("sourceFormat=edt"), "{error}");

    let _ = std::fs::remove_dir_all(root);
}

/// The published `OperationResult` is where a consumer reads the effect of
/// a borrow, so `data.mutation`, `changes` and the workspace itself must
/// agree on which files were created and which were replaced.
#[test]
fn cfe_borrow_result_mutation_changes_and_workspace_agree() {
    let root = std::env::temp_dir().join(format!(
        "unica-cfe-borrow-mutation-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let workspace = root.join("workspace");
    std::fs::create_dir_all(workspace.join("src/Catalogs")).unwrap();
    std::fs::create_dir_all(workspace.join("ext")).unwrap();
    std::fs::write(
        workspace.join("src/Configuration.xml"),
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">\n\t<Configuration uuid=\"55555555-5555-5555-5555-555555555555\"/>\n</MetaDataObject>\n",
    )
    .unwrap();
    std::fs::write(
        workspace.join("src/Catalogs/Items.xml"),
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">\n\t<Catalog uuid=\"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\">\n\t\t<Properties><Name>Items</Name></Properties>\n\t\t<ChildObjects/>\n\t</Catalog>\n</MetaDataObject>\n",
    )
    .unwrap();
    let extension_owner = workspace.join("ext/Configuration.xml");
    std::fs::write(
        &extension_owner,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">\n\t<Configuration uuid=\"66666666-6666-6666-6666-666666666666\">\n\t\t<InternalInfo/>\n\t\t<Properties>\n\t\t\t<ObjectBelonging>Adopted</ObjectBelonging>\n\t\t\t<Name>SmokeExtension</Name>\n\t\t\t<ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose>\n\t\t\t<NamePrefix>SE_</NamePrefix>\n\t\t</Properties>\n\t\t<ChildObjects/>\n\t</Configuration>\n</MetaDataObject>\n",
    )
    .unwrap();
    let borrowed = workspace.join("ext/Catalogs/Items.xml");
    assert!(!borrowed.exists());

    let mut args = Map::new();
    args.insert(
        "cwd".to_string(),
        Value::String(workspace.display().to_string()),
    );
    args.insert("dryRun".to_string(), Value::Bool(false));
    args.insert(
        "ExtensionPath".to_string(),
        Value::String("ext".to_string()),
    );
    args.insert("ConfigPath".to_string(), Value::String("src".to_string()));
    args.insert(
        "Object".to_string(),
        Value::String("Catalog.Items".to_string()),
    );

    let result = application()
        .call_tool("unica.cfe.borrow", &args)
        .expect("borrow must succeed");

    assert!(result.ok, "{:?}", result.errors);
    let mutation = &result
        .data
        .as_ref()
        .expect("cfe.borrow publishes typed data")["mutation"];
    let created = mutation["created"]
        .as_array()
        .expect("created is an array")
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    let updated = mutation["updated"]
        .as_array()
        .expect("updated is an array")
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect::<Vec<_>>();

    assert!(borrowed.exists(), "the borrow must have created the object");
    assert!(
        created
            .iter()
            .any(|path| normalized_path(std::path::Path::new(path)) == normalized_path(&borrowed)),
        "{created:?}"
    );
    assert!(
        updated.iter().any(|path| {
            normalized_path(std::path::Path::new(path)) == normalized_path(&extension_owner)
        }),
        "{updated:?}"
    );
    for path in &created {
        assert!(
            !updated.iter().any(|updated_path| {
                normalized_path(std::path::Path::new(updated_path))
                    == normalized_path(std::path::Path::new(path))
            }),
            "{path} is created and updated"
        );
    }
    let expected_changes = created
        .iter()
        .map(|path| format!("created {path}"))
        .chain(updated.iter().map(|path| format!("updated {path}")))
        .collect::<Vec<_>>();
    assert_eq!(result.changes, expected_changes);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mutating_native_operation_rejects_output_escape_before_backend_execution() {
    let root = std::env::temp_dir().join(format!(
        "unica-app-path-policy-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut args = Map::new();
    args.insert(
        "cwd".to_string(),
        Value::String(workspace.display().to_string()),
    );
    args.insert("dryRun".to_string(), Value::Bool(false));
    args.insert(
        "OutputPath".to_string(),
        Value::String("../outside/Form.xml".to_string()),
    );

    let error = match application().call_tool("unica.form.compile", &args) {
        Ok(result) => panic!("expected path policy error, got {}", result.summary),
        Err(error) => error,
    };

    assert!(error.contains("outside workspace root"));
    assert!(!root.join("outside").exists());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn form_compile_dry_run_rejects_output_escape_like_apply() {
    let root = test_workspace_root("unica-form-compile-preview-path-policy");
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let json_path = workspace.join("form.json");
    std::fs::write(&json_path, "{}").unwrap();
    let mut args = Map::new();
    args.insert(
        "cwd".to_string(),
        Value::String(workspace.display().to_string()),
    );
    args.insert("dryRun".to_string(), Value::Bool(true));
    args.insert(
        "JsonPath".to_string(),
        Value::String(json_path.display().to_string()),
    );
    args.insert(
        "OutputPath".to_string(),
        Value::String("../outside.xml".to_string()),
    );

    let error = application()
        .call_tool("unica.form.compile", &args)
        .expect_err("form preview must enforce the same output path policy as apply");

    assert!(error.contains("outside workspace root"), "{error}");
    assert!(!root.join("outside.xml").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn form_compile_dry_run_rejects_edt_source_set_like_apply() {
    let root = test_workspace_root("unica-form-compile-preview-edt-guard");
    let workspace = root.join("workspace");
    std::fs::create_dir_all(workspace.join("src/Configuration")).unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: EDT\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(workspace.join("src/.project"), "<projectDescription/>").unwrap();
    std::fs::write(
        workspace.join("src/Configuration/Configuration.mdo"),
        "<mdclass:Configuration/>",
    )
    .unwrap();
    let json_path = workspace.join("form.json");
    std::fs::write(&json_path, "{}").unwrap();
    let mut args = Map::new();
    args.insert(
        "cwd".to_string(),
        Value::String(workspace.display().to_string()),
    );
    args.insert("dryRun".to_string(), Value::Bool(true));
    args.insert(
        "JsonPath".to_string(),
        Value::String(json_path.display().to_string()),
    );
    args.insert(
        "OutputPath".to_string(),
        Value::String("src/Form.xml".to_string()),
    );

    let error = application()
        .call_tool("unica.form.compile", &args)
        .expect_err("form preview must enforce the same source-format guard as apply");

    assert!(error.contains("sourceFormat=edt"), "{error}");
    assert!(error.contains("platform_xml"), "{error}");
    assert!(!workspace.join("src/Form.xml").exists());
    let _ = std::fs::remove_dir_all(root);
}

fn metadata_test_object(value: Value) -> Map<String, Value> {
    value
        .as_object()
        .expect("test arguments are an object")
        .clone()
}

#[test]
fn preview_effects_follow_operation_order() {
    let workspace = std::env::temp_dir().join(format!(
        "unica-meta-effect-order-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    // Файловая фикстура пустой конфигурации 8.3.27 вместо снятого cf.init.
    fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
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
    copy_tree(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/formats/platform_xml/2.20/empty_configuration"),
        &workspace,
    );
    let cwd = unica_workspace::test_support::ProcessCwdGuard::enter(&workspace).unwrap();
    let application = application();
    for name in ["Owner", "Subject"] {
        let added = application
            .call_tool(
                "unica.meta.add",
                &metadata_test_object(json!({
                    "sourceSet": "main",
                    "kind": "Catalog",
                    "name": name,
                    "dryRun": false
                })),
            )
            .unwrap();
        assert!(added.ok, "{name}: {:?}", added.errors);
    }
    let operations = json!([
        {"op": "setProperties", "values": {"Comment": "ordered"}},
        {"op": "add", "collection": "attributes", "elements": [{"name": "Transient"}]},
        {"op": "update", "collection": "attributes", "elements": [{"name": "Transient", "newName": "Updated"}]},
        {"op": "remove", "collection": "attributes", "names": ["Updated"]},
        {
            "op": "add",
            "collection": "attributes",
            "elements": [{
                "name": "SearchKey",
                "type": {"variants": [{
                    "kind": "string",
                    "length": 20,
                    "allowedLength": "variable"
                }]},
                "fillValue": {"kind": "string", "value": "ready"}
            }]
        },
        {"op": "editRelations", "relation": "owners", "mode": "replace", "targets": [{"metadataPath": "Catalog.Owner"}]},
        {"op": "editRelations", "relation": "inputByString", "mode": "replace", "targets": [{"fieldPath": "Catalog.Subject.Attribute.SearchKey"}]}
    ]);
    let preview_call = json!({
        "sourceSet": "main",
        "metadataPath": "Catalog.Subject",
        "operations": operations,
        "dryRun": true
    });
    let missing_final_field_call = json!({
        "sourceSet": "main",
        "metadataPath": "Catalog.Subject",
        "operations": [{
            "op": "editRelations",
            "relation": "inputByString",
            "mode": "replace",
            "targets": [{
                "fieldPath": "Catalog.Subject.Attribute.Missing"
            }]
        }],
        "dryRun": true
    });
    let edit_schema = unica_metadata::request::metadata_input_schema(
        unica_metadata::request::MetadataOperation::Edit,
    );
    let edit_validator = jsonschema::validator_for(&edit_schema).unwrap();
    assert!(edit_validator.is_valid(&preview_call));
    assert!(edit_validator.is_valid(&missing_final_field_call));
    let preview = application
        .call_tool("unica.meta.edit", &metadata_test_object(preview_call))
        .unwrap();
    let missing_final_field = application
        .call_tool(
            "unica.meta.edit",
            &metadata_test_object(missing_final_field_call),
        )
        .unwrap();
    let template = application
        .call_tool(
            "unica.meta.add",
            &metadata_test_object(json!({
                "sourceSet": "main",
                "kind": "Catalog",
                "name": "TemplateOnly",
                "dryRun": true
            })),
        )
        .unwrap();
    drop(cwd);
    let _ = std::fs::remove_dir_all(&workspace);

    assert!(preview.ok, "{:?}", preview.errors);
    assert!(!missing_final_field.ok);
    assert!(missing_final_field
        .errors
        .iter()
        .any(|error| error.contains("does not exist in the owner post-image")));
    assert_eq!(
        missing_final_field.diagnostics.unwrap()[0]["field"],
        "operations[0].targets[0]"
    );
    let effects = preview.data.unwrap()["effects"].clone();
    assert_eq!(
        effects
            .as_array()
            .unwrap()
            .iter()
            .map(|effect| effect["operation"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![
            "setProperties",
            "add",
            "update",
            "remove",
            "add",
            "editRelations",
            "editRelations",
        ]
    );
    assert!(effects
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .all(|(index, effect)| effect["operationIndex"] == index as u64));
    assert_eq!(
        template.data.unwrap()["effects"][0]["operation"],
        "createTemplate"
    );
}
