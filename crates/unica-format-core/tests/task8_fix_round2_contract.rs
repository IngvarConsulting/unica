use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use unica_format_core::commands::*;

fn text<T>(value: &str, constructor: impl FnOnce(String) -> Result<T, SemanticValueError>) -> T {
    constructor(value.to_string()).unwrap()
}

fn metadata_definition() -> MetadataDefinition {
    MetadataDefinition::new(
        MetadataCommonDefinition::new(text("Items", MetadataChildName::new)),
        MetadataKindDefinition::new(MetadataKind::Catalog, Vec::new()).unwrap(),
    )
}

fn samples() -> Vec<WriterCommand> {
    let object = || text("Catalog.Items", MetadataObjectReference::new);
    let form_owner = || text("Catalog.Items", FormOwnerReference::new);
    let template_owner = || text("Catalog.Items", TemplateOwnerReference::new);
    let interface_item = || {
        InterfaceItemReference::new(
            InterfaceItemKind::Command,
            text("Catalog.Items.Command.Open", InterfaceItemName::new),
        )
    };

    vec![
        WriterCommand::ConfigurationInitialize(ConfigurationInitialize::new(text(
            "Sales",
            ConfigurationName::new,
        ))),
        WriterCommand::ConfigurationEdit(ConfigurationEdit::mutate(
            ConfigurationMutation::SetProperty(
                ConfigurationPropertyPatch::new(
                    ConfigurationProperty::Comment,
                    ConfigurationPropertyValue::Text(text(
                        "semantic comment",
                        ConfigurationTextValue::new,
                    )),
                )
                .unwrap(),
            ),
        )),
        WriterCommand::ExtensionInitialize(ExtensionInitialize::new(text(
            "SalesPatch",
            ExtensionName::new,
        ))),
        WriterCommand::ExtensionBorrow(ExtensionBorrow::new(object())),
        WriterCommand::ExtensionPatchMethod(ExtensionPatchMethod::new(
            ExtensionModuleTarget::Object {
                owner: text("Catalog.Items", MetadataObjectReference::new),
                role: ExtensionObjectModuleRole::Object,
            },
            text("BeforeWrite", MethodName::new),
            InterceptorKind::Before,
            ExecutionContext::Server,
            false,
        )),
        WriterCommand::ExternalProcessorInitialize(ExternalArtifactInitialize::new(text(
            "Importer",
            ExternalArtifactName::new,
        ))),
        WriterCommand::ExternalReportInitialize(ExternalArtifactInitialize::new(text(
            "Balances",
            ExternalArtifactName::new,
        ))),
        WriterCommand::MetadataCreate(MetadataCreate::new(metadata_definition())),
        WriterCommand::MetadataEdit(MetadataEdit::new(
            object(),
            MetadataPatch::SetProperties(MetadataPropertyChanges::one(
                MetadataPropertyPatch::new(
                    MetadataObjectProperty::Comment,
                    MetadataPropertyValue::Comment(text("updated", CommentText::new)),
                )
                .unwrap(),
            )),
        )),
        WriterCommand::MetadataRemove(MetadataRemove::new(object(), false)),
        WriterCommand::FormCreate(FormCreate::new(
            form_owner(),
            text("ObjectForm", FormName::new),
        )),
        WriterCommand::FormCompile(FormCompile::new(ManagedFormDefinition::empty(), false)),
        WriterCommand::FormEdit(
            FormEdit::new(
                vec![FormPatch::RemoveElement(text(
                    "Obsolete",
                    FormElementName::new,
                ))],
                false,
            )
            .unwrap(),
        ),
        WriterCommand::FormRemove(FormRemove::new(
            form_owner(),
            text("ObjectForm", FormName::new),
        )),
        WriterCommand::TemplateCreate(TemplateCreate::new(
            template_owner(),
            text("Main", TemplateName::new),
            TemplateKind::Spreadsheet,
        )),
        WriterCommand::TemplateRemove(TemplateRemove::new(
            template_owner(),
            text("Main", TemplateName::new),
        )),
        WriterCommand::HelpCreate(HelpCreate::new(
            text("Catalog.Items", HelpOwnerReference::new),
            Some(text("en", LanguageCode::new)),
        )),
        WriterCommand::InterfaceEdit(InterfaceEdit::Hide(interface_item())),
        WriterCommand::RoleCreate(RoleCreate::from_definition(RoleDefinition::new(text(
            "Reader",
            RoleName::new,
        )))),
        WriterCommand::SubsystemCreate(SubsystemCreate::from_definition(SubsystemDefinition::new(
            text("Sales", SubsystemName::new),
        ))),
        WriterCommand::SubsystemEdit(SubsystemEdit::AddChild(text(
            "SalesReports",
            SubsystemName::new,
        ))),
        WriterCommand::SupportEdit(SupportEdit::ObjectRule(SupportObjectRule::Editable)),
        WriterCommand::DataCompositionCreate(DataCompositionCreate::new(
            DataCompositionDefinition::new(vec![DataCompositionDataSet::Query(
                DataCompositionQueryDataSet::new(
                    text("MainData", DataSetName::new),
                    text("SELECT 1 AS Value", DataCompositionQueryText::new),
                ),
            )])
            .unwrap(),
        )),
        WriterCommand::DataCompositionEdit(DataCompositionEdit::new(
            DataCompositionMutation::Clear {
                target: DataCompositionClearTarget::Selection,
                scope: DataCompositionScope::Root,
            },
        )),
        WriterCommand::SpreadsheetCreate(SpreadsheetCreate::new(
            SpreadsheetDocument::new(vec![SpreadsheetArea::new(
                text("Main", SpreadsheetAreaName::new),
                vec![SpreadsheetRow::new(vec![SpreadsheetCell::new(
                    1,
                    SpreadsheetCellValue::Text(text("value", SpreadsheetCellText::new)),
                )
                .unwrap()])],
            )
            .unwrap()])
            .unwrap(),
        )),
    ]
}

fn assert_wire<T: Serialize + DeserializeOwned + Clone + Eq>() {}

#[derive(Clone, Debug)]
enum WireStep {
    Key(String),
    Index(usize),
}

fn collect_object_paths(value: &Value, path: &mut Vec<WireStep>, output: &mut Vec<Vec<WireStep>>) {
    match value {
        Value::Object(values) => {
            output.push(path.clone());
            for (key, value) in values {
                path.push(WireStep::Key(key.clone()));
                collect_object_paths(value, path, output);
                path.pop();
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                path.push(WireStep::Index(index));
                collect_object_paths(value, path, output);
                path.pop();
            }
        }
        _ => {}
    }
}

fn object_at_mut<'a>(mut value: &'a mut Value, path: &[WireStep]) -> &'a mut Value {
    for step in path {
        value = match step {
            WireStep::Key(key) => value.get_mut(key).expect("object path key"),
            WireStep::Index(index) => value.get_mut(*index).expect("object path index"),
        };
    }
    value
}

fn assert_payload_wire<T>(kind: WriterCommandKind, payload: T)
where
    T: Serialize + DeserializeOwned + Clone + Eq + std::fmt::Debug,
{
    let encoded = serde_json::to_value(&payload).unwrap();
    let decoded: T = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(decoded, payload, "{kind:?}: {encoded}");

    let mut object_paths = Vec::new();
    collect_object_paths(&encoded, &mut Vec::new(), &mut object_paths);
    for path in object_paths {
        let mut adversarial = encoded.clone();
        object_at_mut(&mut adversarial, &path)
            .as_object_mut()
            .expect("collected object path")
            .insert("__unknown".to_string(), Value::Bool(true));
        assert!(
            serde_json::from_value::<T>(adversarial).is_err(),
            "{kind:?} payload accepted an unknown field at nested path {path:?}"
        );
    }
}

#[test]
fn every_command_payload_round_trips_and_rejects_unknown_fields() {
    assert_wire::<WriterCommand>();
    assert_wire::<ConfigurationInitialize>();
    assert_wire::<ConfigurationEdit>();
    assert_wire::<ExtensionInitialize>();
    assert_wire::<ExtensionBorrow>();
    assert_wire::<ExtensionPatchMethod>();
    assert_wire::<ExternalArtifactInitialize>();
    assert_wire::<MetadataCreate>();
    assert_wire::<MetadataEdit>();
    assert_wire::<MetadataRemove>();
    assert_wire::<FormCreate>();
    assert_wire::<FormCompile>();
    assert_wire::<FormEdit>();
    assert_wire::<FormRemove>();
    assert_wire::<TemplateCreate>();
    assert_wire::<TemplateRemove>();
    assert_wire::<HelpCreate>();
    assert_wire::<InterfaceEdit>();
    assert_wire::<RoleCreate>();
    assert_wire::<SubsystemCreate>();
    assert_wire::<SubsystemEdit>();
    assert_wire::<SupportEdit>();
    assert_wire::<DataCompositionCreate>();
    assert_wire::<DataCompositionEdit>();
    assert_wire::<SpreadsheetCreate>();
    assert_wire::<ConfigurationPropertyPatch>();
    assert_wire::<MetadataDefinition>();
    assert_wire::<MetadataPatch>();
    assert_wire::<MetadataPropertyChanges>();
    assert_wire::<MetadataPropertiesToClear>();
    assert_wire::<MetadataChildPatch>();
    assert_wire::<ManagedFormDefinition>();
    assert_wire::<FormPatch>();
    assert_wire::<CommandInterfaceDefinition>();
    assert_wire::<SubsystemPropertyPatch>();
    assert_wire::<RoleDefinition>();
    assert_wire::<DataCompositionDefinition>();
    assert_wire::<DataCompositionMutation>();
    assert_wire::<DataCompositionParameterChange>();
    assert_wire::<DataCompositionParameterOrder>();
    assert_wire::<SpreadsheetDocument>();
    assert_wire::<SpreadsheetCellContent>();
    let commands = samples();
    assert_eq!(commands.len(), WriterCommandKind::ALL.len());

    for command in commands {
        let encoded = serde_json::to_value(&command).unwrap();
        let decoded: WriterCommand = serde_json::from_value(encoded.clone()).unwrap();
        assert_eq!(decoded, command, "{encoded}");

        let mut object_paths = Vec::new();
        collect_object_paths(&encoded, &mut Vec::new(), &mut object_paths);
        for path in object_paths {
            let mut adversarial = encoded.clone();
            object_at_mut(&mut adversarial, &path)
                .as_object_mut()
                .expect("collected object path")
                .insert("__unknown".to_string(), Value::Bool(true));
            assert!(
                serde_json::from_value::<WriterCommand>(adversarial).is_err(),
                "{:?} accepted an unknown field at nested path {:?}",
                command.kind(),
                path
            );
        }
    }
}

#[test]
fn every_concrete_command_payload_type_round_trips_independently() {
    let commands = samples();
    assert_eq!(commands.len(), WriterCommandKind::ALL.len());

    for command in commands {
        match command {
            WriterCommand::ConfigurationInitialize(payload) => {
                assert_payload_wire(WriterCommandKind::ConfigurationInitialize, payload)
            }
            WriterCommand::ConfigurationEdit(payload) => {
                assert_payload_wire(WriterCommandKind::ConfigurationEdit, payload)
            }
            WriterCommand::ExtensionInitialize(payload) => {
                assert_payload_wire(WriterCommandKind::ExtensionInitialize, payload)
            }
            WriterCommand::ExtensionBorrow(payload) => {
                assert_payload_wire(WriterCommandKind::ExtensionBorrow, payload)
            }
            WriterCommand::ExtensionPatchMethod(payload) => {
                assert_payload_wire(WriterCommandKind::ExtensionPatchMethod, payload)
            }
            WriterCommand::ExternalProcessorInitialize(payload) => {
                assert_payload_wire(WriterCommandKind::ExternalProcessorInitialize, payload)
            }
            WriterCommand::ExternalReportInitialize(payload) => {
                assert_payload_wire(WriterCommandKind::ExternalReportInitialize, payload)
            }
            WriterCommand::MetadataCreate(payload) => {
                assert_payload_wire(WriterCommandKind::MetadataCreate, payload)
            }
            WriterCommand::MetadataEdit(payload) => {
                assert_payload_wire(WriterCommandKind::MetadataEdit, payload)
            }
            WriterCommand::MetadataRemove(payload) => {
                assert_payload_wire(WriterCommandKind::MetadataRemove, payload)
            }
            WriterCommand::FormCreate(payload) => {
                assert_payload_wire(WriterCommandKind::FormCreate, payload)
            }
            WriterCommand::FormCompile(payload) => {
                assert_payload_wire(WriterCommandKind::FormCompile, payload)
            }
            WriterCommand::FormEdit(payload) => {
                assert_payload_wire(WriterCommandKind::FormEdit, payload)
            }
            WriterCommand::FormRemove(payload) => {
                assert_payload_wire(WriterCommandKind::FormRemove, payload)
            }
            WriterCommand::TemplateCreate(payload) => {
                assert_payload_wire(WriterCommandKind::TemplateCreate, payload)
            }
            WriterCommand::TemplateRemove(payload) => {
                assert_payload_wire(WriterCommandKind::TemplateRemove, payload)
            }
            WriterCommand::HelpCreate(payload) => {
                assert_payload_wire(WriterCommandKind::HelpCreate, payload)
            }
            WriterCommand::InterfaceEdit(payload) => {
                assert_payload_wire(WriterCommandKind::InterfaceEdit, payload)
            }
            WriterCommand::RoleCreate(payload) => {
                assert_payload_wire(WriterCommandKind::RoleCreate, payload)
            }
            WriterCommand::SubsystemCreate(payload) => {
                assert_payload_wire(WriterCommandKind::SubsystemCreate, payload)
            }
            WriterCommand::SubsystemEdit(payload) => {
                assert_payload_wire(WriterCommandKind::SubsystemEdit, payload)
            }
            WriterCommand::SupportEdit(payload) => {
                assert_payload_wire(WriterCommandKind::SupportEdit, payload)
            }
            WriterCommand::DataCompositionCreate(payload) => {
                assert_payload_wire(WriterCommandKind::DataCompositionCreate, payload)
            }
            WriterCommand::DataCompositionEdit(payload) => {
                assert_payload_wire(WriterCommandKind::DataCompositionEdit, payload)
            }
            WriterCommand::SpreadsheetCreate(payload) => {
                assert_payload_wire(WriterCommandKind::SpreadsheetCreate, payload)
            }
        }
    }
}

#[test]
fn command_wire_rejects_unknown_variants_and_invalid_combinations() {
    assert!(serde_json::from_value::<WriterCommand>(json!({
        "command": "nativeOperation",
        "payload": {}
    }))
    .is_err());

    let mut invalid_name = serde_json::to_value(WriterCommand::ConfigurationInitialize(
        ConfigurationInitialize::new(text("Sales", ConfigurationName::new)),
    ))
    .unwrap();
    invalid_name["payload"]["name"] = Value::String(" \n".to_string());
    assert!(serde_json::from_value::<WriterCommand>(invalid_name).is_err());

    let empty_configuration_patch = json!({
        "command": "configurationEdit",
        "payload": {
            "source": "patch",
            "patch": {"operations": []}
        }
    });
    assert!(serde_json::from_value::<WriterCommand>(empty_configuration_patch).is_err());

    let empty_form_patch = json!({
        "command": "formEdit",
        "payload": {
            "patches": [],
            "skipValidation": false
        }
    });
    assert!(serde_json::from_value::<WriterCommand>(empty_form_patch).is_err());

    let unknown_module_target = json!({
        "command": "extensionPatchMethod",
        "payload": {
            "module": {"kind": "filesystemPath", "path": "Catalogs/Items.xml"},
            "method": "Run",
            "interceptor": "before",
            "context": "server",
            "callable": "procedure"
        }
    });
    assert!(serde_json::from_value::<WriterCommand>(unknown_module_target).is_err());

    let empty_data_sets = json!({
        "command": "dataCompositionCreate",
        "payload": {
            "definition": {
                "dataSources": [],
                "dataSets": [],
                "dataSetLinks": [],
                "calculatedFields": [],
                "totals": [],
                "parameters": [],
                "variants": []
            }
        }
    });
    assert!(serde_json::from_value::<WriterCommand>(empty_data_sets).is_err());

    let empty_spreadsheet = json!({
        "command": "spreadsheetCreate",
        "payload": {
            "document": {
                "fonts": [],
                "styles": [],
                "areas": [],
                "columnWidths": []
            }
        }
    });
    assert!(serde_json::from_value::<WriterCommand>(empty_spreadsheet).is_err());

    let zero_spreadsheet_coordinate = json!({
        "command": "spreadsheetCreate",
        "payload": {
            "document": {
                "fonts": [],
                "styles": [],
                "areas": [{
                    "name": "Main",
                    "rows": [{
                        "cells": [{
                            "column": 0,
                            "value": {"kind": "text", "value": "invalid"},
                            "columnSpan": 1,
                            "rowSpan": 1
                        }],
                        "height": null
                    }]
                }],
                "columnWidths": []
            }
        }
    });
    assert!(serde_json::from_value::<WriterCommand>(zero_spreadsheet_coordinate).is_err());

    for unknown_variant in [
        json!({"operation": "native", "payload": {}}),
        json!({"operation": "rawXml", "payload": {}}),
        json!({"operation": "toolName", "payload": {}}),
    ] {
        assert!(serde_json::from_value::<MetadataPatch>(unknown_variant.clone()).is_err());
        assert!(serde_json::from_value::<FormPatch>(unknown_variant.clone()).is_err());
        assert!(serde_json::from_value::<DataCompositionMutation>(unknown_variant).is_err());
    }
}

#[test]
fn nested_patch_payloads_reject_invalid_property_value_combinations() {
    assert!(serde_json::from_value::<ConfigurationPropertyPatch>(json!({
        "property": "defaultLanguage",
        "value": {"kind": "boolean", "value": true}
    }))
    .is_err());

    assert!(serde_json::from_value::<SubsystemPropertyPatch>(json!({
        "operation": "setCommandInterfaceVisibility",
        "value": "not-a-boolean"
    }))
    .is_err());

    assert!(
        serde_json::from_value::<DataCompositionParameterChange>(json!({
            "operation": "setHidden",
            "value": "not-a-boolean"
        }))
        .is_err()
    );
}

#[test]
fn nested_payloads_reject_empty_or_zero_semantic_operations() {
    assert!(serde_json::from_value::<MetadataPatch>(json!({
        "operation": "setProperties",
        "payload": []
    }))
    .is_err());
    assert!(serde_json::from_value::<MetadataPatch>(json!({
        "operation": "clearProperties",
        "payload": []
    }))
    .is_err());
    assert!(serde_json::from_value::<MetadataChildPatch>(json!({
        "target": {"kind": "attribute", "name": "Code", "parent": null},
        "changes": []
    }))
    .is_err());
    assert!(serde_json::from_value::<DataCompositionMutation>(json!({
        "operation": "reorderParameters",
        "payload": []
    }))
    .is_err());
    assert!(serde_json::from_value::<SpreadsheetCellContent>(json!({
        "text": null,
        "parameter": null,
        "template": null,
        "detail": null
    }))
    .is_err());
    assert!(serde_json::from_value::<ConfigurationHomePageEntry>(json!({
        "form": "Catalog.Items.Form.Object",
        "height": 0,
        "visible": true,
        "roleVisibility": []
    }))
    .is_err());
}
