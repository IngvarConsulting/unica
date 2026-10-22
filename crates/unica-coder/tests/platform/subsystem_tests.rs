use super::*;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use unica_source_contract::workspace::WorkspaceContext;
use unica_workspace::single_file_publisher::with_before_commit_hook;

fn temp_context(name: &str) -> WorkspaceContext {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("unica-subsystem-{name}-{nanos}"));
    fs::create_dir_all(&root).unwrap();
    WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".build").join("unica"),
        workspace_epoch: 1,
    }
}

fn edit_definition_args(
    context: &WorkspaceContext,
    subsystem_path: &Path,
    definition: Value,
) -> Map<String, Value> {
    let definition_path = context.cwd.join("subsystem-edit-definition.json");
    fs::write(&definition_path, definition.to_string()).unwrap();
    let mut args = Map::new();
    args.insert(
        "SubsystemPath".to_string(),
        Value::String(subsystem_path.display().to_string()),
    );
    args.insert(
        "DefinitionFile".to_string(),
        Value::String(definition_path.display().to_string()),
    );
    args
}

/// Platform XML of a fresh top-level subsystem with platform defaults, as
/// the 8.3.27 dump writes it.
fn subsystem_fixture_xml(name: &str) -> String {
    let mut synonym = String::new();
    for (index, character) in name.chars().enumerate() {
        if index > 0 && character.is_uppercase() {
            synonym.push(' ');
            synonym.extend(character.to_lowercase());
        } else {
            synonym.push(character);
        }
    }
    format!(
            concat!(
                "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
                "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:app=\"http://v8.1c.ru/8.2/managed-application/core\" xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\" xmlns:cmi=\"http://v8.1c.ru/8.2/managed-application/cmi\" xmlns:ent=\"http://v8.1c.ru/8.1/data/enterprise\" xmlns:lf=\"http://v8.1c.ru/8.2/managed-application/logform\" xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\" xmlns:xen=\"http://v8.1c.ru/8.3/xcf/enums\" xmlns:xpr=\"http://v8.1c.ru/8.3/xcf/predef\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"2.20\">\n",
                "\t<Subsystem uuid=\"11111111-2222-4333-8444-555555555555\">\n",
                "\t\t<Properties>\n",
                "\t\t\t<Name>{name}</Name>\n",
                "\t\t\t<Synonym>\n",
                "\t\t\t\t<v8:item>\n",
                "\t\t\t\t\t<v8:lang>ru</v8:lang>\n",
                "\t\t\t\t\t<v8:content>{synonym}</v8:content>\n",
                "\t\t\t\t</v8:item>\n",
                "\t\t\t</Synonym>\n",
                "\t\t\t<Comment/>\n",
                "\t\t\t<IncludeHelpInContents>true</IncludeHelpInContents>\n",
                "\t\t\t<IncludeInCommandInterface>true</IncludeInCommandInterface>\n",
                "\t\t\t<UseOneCommand>false</UseOneCommand>\n",
                "\t\t\t<Explanation/>\n",
                "\t\t\t<Picture/>\n",
                "\t\t\t<Content/>\n",
                "\t\t</Properties>\n",
                "\t\t<ChildObjects/>\n",
                "\t</Subsystem>\n",
                "</MetaDataObject>\n"
            ),
            name = name,
            synonym = synonym,
        )
}

#[test]
fn subsystem_native_text_observations_keep_empty_absent_and_invalid_distinct() {
    for (comment_xml, explanation_xml, comment, explanation, errors) in [
        ("<Comment/>", "<Explanation/>", Some(""), Some(""), 0),
        ("", "", None, None, 0),
        ("<Comment>точный текст</Comment>", "<Explanation><v8:item><v8:lang>en</v8:lang><v8:content>fallback</v8:content></v8:item></Explanation>", Some("точный текст"), Some("fallback"), 0),
        ("<Comment>а<![CDATA[б]]>в</Comment>", "<Explanation><v8:item><v8:lang>ru</v8:lang><v8:content>а<![CDATA[б]]>в</v8:content></v8:item></Explanation>", Some("абв"), Some("абв"), 0),
        ("<Comment><Bad/></Comment>", "<Explanation>text</Explanation>", None, None, 2),
        ("<Comment/><Comment/>", "<Explanation/><Explanation/>", None, None, 2),
    ] {
        let xml = subsystem_fixture_xml("Example").replace("<Comment/>", comment_xml).replace("<Explanation/>", explanation_xml);
        let (data, _) = parse_subsystem_info_xml(Path::new("Example.xml"), &xml, false).unwrap();
        assert_eq!(data.comment.as_deref(), comment, "{xml}");
        assert_eq!(data.explanation.as_deref(), explanation, "{xml}");
        assert_eq!(data.property_limits.len(), errors, "{xml}");
        assert_eq!(data.name, "Example");
    }
}

/// Writes a top-level subsystem under `root` and, when `root` holds a
/// `Configuration.xml`, registers it there in canonical position.
fn write_subsystem_fixture(root: &Path, name: &str) -> PathBuf {
    let subsystem = root.join("Subsystems").join(format!("{name}.xml"));
    fs::create_dir_all(subsystem.parent().unwrap()).unwrap();
    fs::write(&subsystem, subsystem_fixture_xml(name)).unwrap();
    let configuration = root.join("Configuration.xml");
    if configuration.is_file() {
        let text = fs::read_to_string(&configuration).unwrap();
        let registered = text.replacen(
            "\t\t\t<StyleItem>",
            &format!("\t\t\t<Subsystem>{name}</Subsystem>\r\n\t\t\t<StyleItem>"),
            1,
        );
        assert_ne!(
            registered, text,
            "fixture configuration has no StyleItem anchor"
        );
        fs::write(&configuration, registered).unwrap();
    }
    subsystem
}

fn create_edit_fixture(context: &WorkspaceContext, name: &str) -> PathBuf {
    write_subsystem_fixture(&context.cwd, name)
}

fn configuration_bytes() -> Vec<u8> {
    let text = concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" version=\"2.20\">\r\n",
            "\t<Configuration uuid=\"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\">\r\n",
            "\t\t<InternalInfo>\r\n",
            "\t\t\t<xr:ContainedObject><xr:ClassId>9cd510cd-abfc-11d4-9434-004095e12fc7</xr:ClassId><xr:ObjectId>00000000-0000-0000-0000-000000000002</xr:ObjectId></xr:ContainedObject>\r\n",
            "\t\t\t<xr:ContainedObject><xr:ClassId>9fcd25a0-4822-11d4-9414-008048da11f9</xr:ClassId><xr:ObjectId>00000000-0000-0000-0000-000000000003</xr:ObjectId></xr:ContainedObject>\r\n",
            "\t\t\t<xr:ContainedObject><xr:ClassId>e3687481-0a87-462c-a166-9f34594f9bba</xr:ClassId><xr:ObjectId>00000000-0000-0000-0000-000000000004</xr:ObjectId></xr:ContainedObject>\r\n",
            "\t\t\t<xr:ContainedObject><xr:ClassId>9de14907-ec23-4a07-96f0-85521cb6b53b</xr:ClassId><xr:ObjectId>00000000-0000-0000-0000-000000000005</xr:ObjectId></xr:ContainedObject>\r\n",
            "\t\t\t<xr:ContainedObject><xr:ClassId>51f2d5d8-ea4d-4064-8892-82951750031e</xr:ClassId><xr:ObjectId>00000000-0000-0000-0000-000000000006</xr:ObjectId></xr:ContainedObject>\r\n",
            "\t\t\t<xr:ContainedObject><xr:ClassId>e68182ea-4237-4383-967f-90c1e3370bc7</xr:ClassId><xr:ObjectId>00000000-0000-0000-0000-000000000007</xr:ObjectId></xr:ContainedObject>\r\n",
            "\t\t\t<xr:ContainedObject><xr:ClassId>fb282519-d103-4dd3-bc12-cb271d631dfc</xr:ClassId><xr:ObjectId>00000000-0000-0000-0000-000000000008</xr:ObjectId></xr:ContainedObject>\r\n",
            "\t\t</InternalInfo>\r\n",
            "\t\t<Properties><Name>Demo</Name><ConfigurationExtensionCompatibilityMode>Version8_3_27</ConfigurationExtensionCompatibilityMode><DefaultLanguage>Language.Russian</DefaultLanguage></Properties>\r\n",
            "\t\t<ChildObjects>\r\n",
            "\t\t\t<Language>Russian</Language>\r\n",
            "\t\t\t<StyleItem>Accent</StyleItem>\r\n",
            "\t\t</ChildObjects>\r\n",
            "\t</Configuration>\r\n",
            "</MetaDataObject><!-- registrar-tail -->\r\n\r\n"
        );
    let mut bytes = b"\xef\xbb\xbf".to_vec();
    bytes.extend_from_slice(text.as_bytes());
    bytes
}

fn write_configuration(root: &Path) -> Vec<u8> {
    let bytes = configuration_bytes();
    fs::create_dir_all(root.join("Languages")).unwrap();
    fs::write(root.join("Languages/Russian.xml"), b"language marker").unwrap();
    fs::write(root.join("Configuration.xml"), &bytes).unwrap();
    bytes
}

fn write_command_interface(subsystem_xml: &Path, version: &str) -> PathBuf {
    let path = subsystem_dir_for_xml(subsystem_xml)
        .join("Ext")
        .join("CommandInterface.xml");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        format!(
            r#"<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="{version}"/>"#
        ),
    )
    .unwrap();
    path
}

#[test]
fn subsystem_validate_reports_validation_failures_in_errors() {
    let context = temp_context("validate-errors");
    let subsystem_path = context.cwd.join("missing-subsystem.xml");
    let outcome = validate_subsystem(
        &Map::from_iter([(
            "SubsystemPath".to_string(),
            Value::String(subsystem_path.display().to_string()),
        )]),
        &context,
    );

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome.errors.join("\n").contains("not found"),
        "{outcome:?}"
    );
    fs::remove_dir_all(&context.cwd).unwrap();
}

#[test]
fn subsystem_edit_rolls_back_if_format_owner_changes_during_publication() {
    let context = temp_context("edit-format-owner-race");
    let source = context.cwd.join("src");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        context.cwd.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    write_configuration(&source);
    let subsystem = write_subsystem_fixture(&source, "Editable");
    let subsystem_before = fs::read(&subsystem).unwrap();
    let owner = source.join("Configuration.xml");
    let mut concurrent_owner = fs::read(&owner).unwrap();
    concurrent_owner.extend_from_slice(b" ");
    let owner_for_hook = owner.clone();
    let concurrent_for_hook = concurrent_owner.clone();
    let args = edit_definition_args(
        &context,
        &subsystem,
        json!({
            "operation": "set-property",
            "value": {
                "name": "IncludeHelpInContents",
                "value": false
            }
        }),
    );

    let outcome = with_before_commit_hook(
        move |_| fs::write(&owner_for_hook, &concurrent_for_hook).unwrap(),
        || edit_subsystem(&args, &context),
    );

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome.errors.join("\n").contains("read guard"),
        "{outcome:?}"
    );
    assert_eq!(fs::read(&owner).unwrap(), concurrent_owner);
    assert_eq!(fs::read(&subsystem).unwrap(), subsystem_before);
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
pub(crate) fn subsystem_edit_exact_binds_a_reused_existing_child() {
    let context = temp_context("edit-reused-child-race");
    let subsystem = create_edit_fixture(&context, "Parent");
    let subsystem_before = fs::read(&subsystem).unwrap();
    let child_path = context
        .cwd
        .join("Subsystems/Parent/Subsystems/ExistingChild.xml");
    fs::create_dir_all(child_path.parent().unwrap()).unwrap();
    let child_before = unica_xml_format::text::utf8_bom_bytes(&child_subsystem_stub_xml(
        "ExistingChild",
        "2.20",
        crate::infrastructure::source_ids::fresh_uuid,
    ));
    fs::write(&child_path, &child_before).unwrap();
    let mut concurrent_child = child_before.clone();
    concurrent_child.extend_from_slice(b"<!-- concurrent -->");
    let child_for_hook = child_path.clone();
    let concurrent_for_hook = concurrent_child.clone();
    let args = edit_definition_args(
        &context,
        &subsystem,
        json!({
            "operation": "add-child",
            "value": "ExistingChild"
        }),
    );

    let outcome = with_before_commit_hook(
        move |_| fs::write(&child_for_hook, &concurrent_for_hook).unwrap(),
        || edit_subsystem(&args, &context),
    );

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome.errors.join("\n").contains("read guard"),
        "{outcome:?}"
    );
    assert_eq!(fs::read(&child_path).unwrap(), concurrent_child);
    assert_eq!(fs::read(&subsystem).unwrap(), subsystem_before);
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn subsystem_edit_rejects_newer_direct_command_interface_without_mutation() {
    let context = temp_context("edit-newer-direct-ci");
    let subsystem = create_edit_fixture(&context, "Editable");
    let subsystem_before = fs::read(&subsystem).unwrap();
    let command_interface = write_command_interface(&subsystem, "2.21");
    let command_interface_before = fs::read(&command_interface).unwrap();
    let args = edit_definition_args(
        &context,
        &subsystem,
        json!({
            "operation": "set-property",
            "value": {
                "name": "IncludeHelpInContents",
                "value": false
            }
        }),
    );

    let outcome = edit_subsystem(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome
            .errors
            .join("\n")
            .contains("newer than supported 2.20"),
        "{outcome:?}"
    );
    assert_eq!(fs::read(&subsystem).unwrap(), subsystem_before);
    assert_eq!(
        fs::read(&command_interface).unwrap(),
        command_interface_before
    );
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn subsystem_edit_ignores_newer_command_interface_of_unrelated_neighbor() {
    let context = temp_context("edit-unrelated-newer-ci");
    let subsystem = create_edit_fixture(&context, "Editable");
    let neighbor = context.cwd.join("Subsystems/Neighbor.xml");
    fs::write(
        &neighbor,
        unica_xml_format::text::utf8_bom_bytes(&child_subsystem_stub_xml(
            "Neighbor",
            "2.20",
            crate::infrastructure::source_ids::fresh_uuid,
        )),
    )
    .unwrap();
    let neighbor_command_interface = write_command_interface(&neighbor, "2.21");
    let neighbor_before = fs::read(&neighbor).unwrap();
    let neighbor_command_interface_before = fs::read(&neighbor_command_interface).unwrap();
    let args = edit_definition_args(
        &context,
        &subsystem,
        json!({
            "operation": "set-property",
            "value": {
                "name": "IncludeHelpInContents",
                "value": false
            }
        }),
    );

    let outcome = edit_subsystem(&args, &context);

    assert!(outcome.ok, "{outcome:?}");
    assert_eq!(fs::read(&neighbor).unwrap(), neighbor_before);
    assert_eq!(
        fs::read(&neighbor_command_interface).unwrap(),
        neighbor_command_interface_before
    );
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_subsystem_rejects_traversal_in_child_name_before_writing() {
    let context = temp_context("edit-reject-child-traversal");
    let subsystem_path = create_edit_fixture(&context, "EditableSubsystem");
    let parent_before = fs::read(&subsystem_path).unwrap();
    let args = edit_definition_args(
        &context,
        &subsystem_path,
        json!({
            "operation": "add-child",
            "value": "../EscapedChild"
        }),
    );

    let outcome = edit_subsystem(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(outcome.errors.join("\n").contains("XML NCName"));
    assert_eq!(fs::read(&subsystem_path).unwrap(), parent_before);
    assert!(!context.cwd.join("Subsystems/EditableSubsystem").exists());
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_subsystem_rejects_non_canonical_boolean_properties_before_writing() {
    for property in [
        "IncludeHelpInContents",
        "IncludeInCommandInterface",
        "UseOneCommand",
    ] {
        let context = temp_context(&format!("edit-reject-boolean-{property}"));
        let subsystem_path = create_edit_fixture(&context, "EditableSubsystem");
        let parent_before = fs::read(&subsystem_path).unwrap();
        let args = edit_definition_args(
            &context,
            &subsystem_path,
            json!({
                "operation": "set-property",
                "value": {
                    "name": property,
                    "value": "banana"
                }
            }),
        );

        let outcome = edit_subsystem(&args, &context);

        assert!(!outcome.ok, "{property}: {outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("true or false"),
            "{property}: {:?}",
            outcome.errors
        );
        assert_eq!(fs::read(&subsystem_path).unwrap(), parent_before);
        let _ = fs::remove_dir_all(&context.cwd);
    }
}

#[test]
fn edit_subsystem_late_unknown_operation_leaves_child_stub_absent() {
    let context = temp_context("edit-late-unknown-operation");
    let subsystem_path = create_edit_fixture(&context, "EditableSubsystem");
    let parent_before = fs::read(&subsystem_path).unwrap();
    let child_path = context
        .cwd
        .join("Subsystems/EditableSubsystem/Subsystems/PlannedChild.xml");
    let args = edit_definition_args(
        &context,
        &subsystem_path,
        json!([
            {
                "operation": "add-child",
                "value": "PlannedChild"
            },
            {
                "operation": "bogus",
                "value": ""
            }
        ]),
    );

    let outcome = edit_subsystem(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(outcome.errors.join("\n").contains("Unknown operation"));
    assert_eq!(fs::read(&subsystem_path).unwrap(), parent_before);
    assert!(!child_path.exists());
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_subsystem_plans_child_stubs_from_final_batch_state() {
    let context = temp_context("edit-final-batch-state");
    let subsystem_path = create_edit_fixture(&context, "EditableSubsystem");
    let child_path = context
        .cwd
        .join("Subsystems/EditableSubsystem/Subsystems/TransientChild.xml");
    let args = edit_definition_args(
        &context,
        &subsystem_path,
        json!([
            {
                "operation": "add-child",
                "value": "TransientChild"
            },
            {
                "operation": "remove-child",
                "value": "TransientChild"
            }
        ]),
    );

    let outcome = edit_subsystem(&args, &context);

    assert!(outcome.ok, "{:?}", outcome.errors);
    assert!(!child_path.exists());
    let parent = fs::read_to_string(&subsystem_path).unwrap();
    assert!(!parent.contains("<Subsystem>TransientChild</Subsystem>"));
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_subsystem_does_not_register_or_clobber_invalid_existing_child_target() {
    let context = temp_context("edit-invalid-existing-child");
    let subsystem_path = create_edit_fixture(&context, "EditableSubsystem");
    let parent_before = fs::read(&subsystem_path).unwrap();
    let child_path = context
        .cwd
        .join("Subsystems/EditableSubsystem/Subsystems/ExistingChild.xml");
    fs::create_dir_all(child_path.parent().unwrap()).unwrap();
    let child_before = b"sentinel-partial-child".to_vec();
    fs::write(&child_path, &child_before).unwrap();
    let args = edit_definition_args(
        &context,
        &subsystem_path,
        json!({
            "operation": "add-child",
            "value": "ExistingChild"
        }),
    );

    let outcome = edit_subsystem(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome.errors.join("\n").contains("semantic validation"),
        "{:?}",
        outcome.errors
    );
    assert!(
        outcome.errors.join("\n").contains("XML parse error"),
        "{:?}",
        outcome.errors
    );
    assert_eq!(fs::read(&subsystem_path).unwrap(), parent_before);
    assert_eq!(fs::read(&child_path).unwrap(), child_before);
    let _ = fs::remove_dir_all(&context.cwd);
}

#[test]
fn edit_subsystem_rolls_back_parent_and_child_stub_after_post_write_failure() {
    use unica_workspace::compile_transaction::{with_commit_failpoint, CommitFailpoint};

    let context = temp_context("edit-post-write-rollback");
    let subsystem_path = create_edit_fixture(&context, "EditableSubsystem");
    let parent_before = fs::read(&subsystem_path).unwrap();
    let child_path = context
        .cwd
        .join("Subsystems/EditableSubsystem/Subsystems/PlannedChild.xml");
    let args = edit_definition_args(
        &context,
        &subsystem_path,
        json!({
            "operation": "add-child",
            "value": "PlannedChild"
        }),
    );

    let outcome = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
        edit_subsystem(&args, &context)
    });

    assert!(!outcome.ok, "{outcome:?}");
    assert!(outcome.errors.join("\n").contains("post-write validation"));
    assert_eq!(fs::read(&subsystem_path).unwrap(), parent_before);
    assert!(!child_path.exists());
    assert!(!context.cwd.join("Subsystems/EditableSubsystem").exists());
    let _ = fs::remove_dir_all(&context.cwd);
}
