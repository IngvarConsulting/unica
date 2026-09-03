use super::metadata::{owner_registration_image, qualified_target};
use super::validate_platform_xml_binding;
use crate::domain::address::{NodeKind, QualifiedAddress};
use crate::domain::apply::OperationFamily;
use crate::domain::events::{DomainEvent, DomainEventKind};
use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use crate::infrastructure::logical_event_source::attached_resource_relative;
use crate::infrastructure::native_operations::apply::{
    empty_apply_family_batch, hidden_apply_family_unimplemented, ApplyPlanError,
    ApplyPlanErrorKind, ApplyStagedState,
};
use crate::infrastructure::native_operations::apply_families::request::{
    IndexedPlanOperation, ProvisionalApplyEffect,
};
use crate::infrastructure::native_operations::support::{SupportCapability, SupportObjectRule};
use crate::infrastructure::workspace_actor::{FormResourceApplyAuthority, ProviderRootBinding};
use serde_json::{Map, Value};
use std::path::PathBuf;

const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";

#[derive(Debug, Clone)]
enum SubsystemEdit {
    ContentAdd(Vec<String>),
    ContentRemove(Vec<String>),
    ChildAdd(Vec<String>),
    ChildRemove(Vec<String>),
}

#[derive(Debug, Clone)]
enum FormResourcePlanKind {
    /// `role.create`: descriptor, empty rights document and registration.
    CreateRole {
        name: String,
    },
    /// `subsystem.create`: descriptor and registration.
    CreateSubsystem {
        name: String,
    },
    RightSet {
        role: MetadataAddress,
        object: String,
        right: String,
        value: Option<bool>,
        rls: Option<String>,
    },
    Subsystem {
        address: String,
        relative: PathBuf,
        edit: SubsystemEdit,
    },
    SupportCapability(SupportCapability),
    SupportRule {
        target: MetadataAddress,
        rule: SupportObjectRule,
    },
    /// `form.add` / `form.create`: a managed form from the platform shapes.
    FormAdd {
        owner: MetadataAddress,
        forms: Vec<(String, String)>,
    },
    /// `form.set`: default-form slots of the owner.
    FormSet {
        owner: MetadataAddress,
        defaults: Vec<(String, String)>,
    },
    FormRemove {
        owner: MetadataAddress,
        form: String,
    },
    /// Definition-driven edits of one form's Form.xml.
    FormEdit {
        owner: MetadataAddress,
        form: String,
        definitions: Vec<Value>,
    },
    Unsupported,
}

#[derive(Debug, Clone)]
pub(crate) struct FormResourcePlanOperation {
    operation: String,
    kind: FormResourcePlanKind,
}

impl FormResourcePlanOperation {
    pub(crate) fn operation(&self) -> &str {
        &self.operation
    }
}

fn bad(op_index: usize, field: &str, message: impl Into<String>) -> ApplyPlanError {
    ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message.into())
        .at_path(format!("ops[{op_index}].args.{field}"))
}

fn required_values<'a>(
    args: &'a Map<String, Value>,
    op_index: usize,
    allowed: &[&str],
) -> Result<&'a Map<String, Value>, ApplyPlanError> {
    let values = args
        .get("values")
        .and_then(Value::as_object)
        .ok_or_else(|| bad(op_index, "values", "`values` must be an object"))?;
    if let Some(field) = values
        .keys()
        .find(|field| !allowed.contains(&field.as_str()))
    {
        return Err(bad(
            op_index,
            &format!("values.{field}"),
            format!(
                "values accept only {}, not `{field}`",
                allowed
                    .iter()
                    .map(|name| format!("`{name}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }
    Ok(values)
}

fn required_string<'a>(
    values: &'a Map<String, Value>,
    field: &str,
    op_index: usize,
    location: &str,
) -> Result<&'a str, ApplyPlanError> {
    values
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            bad(
                op_index,
                &format!("{location}.{field}"),
                format!("`{field}` is required and must be a non-empty string"),
            )
        })
}

/// Names listed in `items` (objects with `key`, or bare strings), or a single
/// `values.<key>`; the closed argument shapes for list-like operations.
fn listed_names(
    args: &Map<String, Value>,
    key: &str,
    op_index: usize,
) -> Result<Vec<String>, ApplyPlanError> {
    if let Some(items) = args.get("items") {
        let items = items
            .as_array()
            .ok_or_else(|| bad(op_index, "items", "`items` must be an array"))?;
        let mut names = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            let name = match item {
                Value::String(name) => name.trim().to_string(),
                Value::Object(object) => {
                    required_string(object, key, op_index, &format!("items[{index}]"))?.to_string()
                }
                _ => {
                    return Err(bad(
                        op_index,
                        &format!("items[{index}]"),
                        format!("each item must be an object with `{key}` or a string"),
                    ))
                }
            };
            if name.is_empty() {
                return Err(bad(
                    op_index,
                    &format!("items[{index}].{key}"),
                    "name is empty",
                ));
            }
            names.push(name);
        }
        if names.is_empty() {
            return Err(bad(
                op_index,
                "items",
                "`items` must list at least one entry",
            ));
        }
        return Ok(names);
    }
    if let Some(values) = args.get("values").and_then(Value::as_object) {
        return Ok(vec![
            required_string(values, key, op_index, "values")?.to_string()
        ]);
    }
    Err(bad(
        op_index,
        "items",
        format!("`items` (objects with `{key}`) is required"),
    ))
}

/// `Subsystems/A/Subsystems/B.xml` for `Subsystem.A.Subsystem.B`.
fn subsystem_descriptor_relative(
    target: &QualifiedAddress,
    op_index: usize,
) -> Result<(String, PathBuf), ApplyPlanError> {
    let mut relative = PathBuf::new();
    let mut address = Vec::new();
    let segments = target.segments();
    if segments.is_empty() {
        return Err(bad(op_index, "at", "the target must be a subsystem"));
    }
    for (index, segment) in segments.iter().enumerate() {
        if segment.kind() != NodeKind::Subsystem {
            return Err(bad(
                op_index,
                "at",
                "the target must be a subsystem: `Subsystem.Name[.Subsystem.Child]`",
            ));
        }
        let name = segment
            .name()
            .ok_or_else(|| bad(op_index, "at", "the subsystem must be named"))?;
        relative.push("Subsystems");
        if index + 1 == segments.len() {
            relative.push(format!("{name}.xml"));
        } else {
            relative.push(name);
        }
        address.push(format!("Subsystem.{name}"));
    }
    Ok((address.join("."), relative))
}

fn creation_name(
    target: &QualifiedAddress,
    kind: NodeKind,
    values: &Map<String, Value>,
    op_index: usize,
) -> Result<String, ApplyPlanError> {
    let name = required_string(values, "name", op_index, "values")?.to_string();
    if !crate::infrastructure::native_operations::common::is_1c_identifier(&name) {
        return Err(bad(
            op_index,
            "values.name",
            format!("`{name}` is not a valid 1C identifier"),
        ));
    }
    match target.segments() {
        [root] if root.kind() == NodeKind::Configuration => {}
        [only] if only.kind() == kind && only.name() == Some(name.as_str()) => {}
        _ => {
            return Err(bad(
                op_index,
                "at",
                format!(
                    "creation targets the configuration root (or `{}.{name}` itself)",
                    kind.as_str()
                ),
            ))
        }
    }
    Ok(name)
}

pub(crate) fn parse_form_resource_plan_operation(
    operation: &str,
    args: &Value,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<FormResourcePlanOperation, ApplyPlanError> {
    validate_platform_xml_binding(binding, op_index)?;
    let object = args.as_object().ok_or_else(|| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "operation args must be an object",
        )
        .at_path(format!("ops[{op_index}].args"))
    })?;
    let kind = match crate::application::v13::apply::dispatch_family(operation) {
        Some(OperationFamily::Role) => parse_role_operation(operation, object, op_index, binding)?,
        Some(OperationFamily::Subsystem) => {
            parse_subsystem_operation(operation, object, op_index, binding)?
        }
        Some(OperationFamily::Support) => {
            parse_support_operation(operation, object, op_index, binding)?
        }
        Some(OperationFamily::Form) => parse_form_operation(operation, object, op_index, binding)?,
        _ => FormResourcePlanKind::Unsupported,
    };
    Ok(FormResourcePlanOperation {
        operation: operation.to_string(),
        kind,
    })
}

fn parse_role_operation(
    operation: &str,
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<FormResourcePlanKind, ApplyPlanError> {
    let target = qualified_target(args, op_index, binding)?;
    match operation {
        "role.create" => {
            let values = required_values(args, op_index, &["name"])?;
            let name = creation_name(&target, NodeKind::Role, values, op_index)?;
            Ok(FormResourcePlanKind::CreateRole { name })
        }
        "right.set" => {
            let [role] = target.segments() else {
                return Err(bad(
                    op_index,
                    "at",
                    "right.set targets one role: `Role.Name`",
                ));
            };
            if role.kind() != NodeKind::Role {
                return Err(bad(
                    op_index,
                    "at",
                    "right.set targets one role: `Role.Name`",
                ));
            }
            let role_name = role
                .name()
                .ok_or_else(|| bad(op_index, "at", "the role must be named"))?;
            let values = required_values(args, op_index, &["object", "right", "value", "rls"])?;
            let object = required_string(values, "object", op_index, "values")?.to_string();
            let right = required_string(values, "right", op_index, "values")?.to_string();
            let value = match values.get("value") {
                None => None,
                Some(Value::Bool(value)) => Some(*value),
                Some(_) => return Err(bad(op_index, "values.value", "`value` must be a boolean")),
            };
            let rls = match values.get("rls") {
                None => None,
                Some(Value::String(condition)) if !condition.trim().is_empty() => {
                    Some(condition.trim().to_string())
                }
                Some(_) => {
                    return Err(bad(
                        op_index,
                        "values.rls",
                        "`rls` must be a non-empty restriction condition",
                    ))
                }
            };
            if value.is_none() && rls.is_none() {
                return Err(bad(
                    op_index,
                    "values",
                    "right.set needs `value` (true/false) and/or `rls` (a restriction condition)",
                ));
            }
            let role = MetadataAddress::parse(
                PLATFORM_XML_8_3_27_FORMAT_2_20,
                &format!("Role.{role_name}"),
            )
            .map_err(|error| bad(op_index, "at", error.to_string()))?;
            Ok(FormResourcePlanKind::RightSet {
                role,
                object,
                right,
                value,
                rls,
            })
        }
        _ => Ok(FormResourcePlanKind::Unsupported),
    }
}

fn parse_subsystem_operation(
    operation: &str,
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<FormResourcePlanKind, ApplyPlanError> {
    let target = qualified_target(args, op_index, binding)?;
    if operation == "subsystem.create" {
        let values = required_values(args, op_index, &["name"])?;
        let name = creation_name(&target, NodeKind::Subsystem, values, op_index)?;
        return Ok(FormResourcePlanKind::CreateSubsystem { name });
    }
    let edit = match operation {
        "content.add" => SubsystemEdit::ContentAdd(listed_names(args, "object", op_index)?),
        "content.remove" => SubsystemEdit::ContentRemove(listed_names(args, "object", op_index)?),
        "childSubsystem.add" => SubsystemEdit::ChildAdd(listed_names(args, "name", op_index)?),
        "childSubsystem.remove" => {
            // The child may travel in the address (`...Subsystem.Parent.Subsystem.Child`)
            // or in `items`.
            if args.contains_key("items") || args.contains_key("values") {
                SubsystemEdit::ChildRemove(listed_names(args, "name", op_index)?)
            } else {
                let segments = target.segments();
                let [.., parent_segment, child] = segments else {
                    return Err(bad(
                        op_index,
                        "at",
                        "childSubsystem.remove names the child in `items` or addresses it: `Subsystem.Parent.Subsystem.Child`",
                    ));
                };
                let _ = parent_segment;
                let child_name = child
                    .name()
                    .ok_or_else(|| bad(op_index, "at", "the child subsystem must be named"))?
                    .to_string();
                let parent = QualifiedAddress::resolve_input(
                    &format!(
                        "{}:{}",
                        target.source_set(),
                        segments[..segments.len() - 1]
                            .iter()
                            .map(|segment| {
                                format!(
                                    "{}.{}",
                                    segment.kind().as_str(),
                                    segment.name().unwrap_or("")
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(".")
                    ),
                    &[binding.source_set_name()],
                )
                .map_err(|error| bad(op_index, "at", error.to_string()))?;
                let (address, relative) = subsystem_descriptor_relative(&parent, op_index)?;
                return Ok(FormResourcePlanKind::Subsystem {
                    address,
                    relative,
                    edit: SubsystemEdit::ChildRemove(vec![child_name]),
                });
            }
        }
        _ => return Ok(FormResourcePlanKind::Unsupported),
    };
    let (address, relative) = subsystem_descriptor_relative(&target, op_index)?;
    Ok(FormResourcePlanKind::Subsystem {
        address,
        relative,
        edit,
    })
}

fn parse_support_operation(
    operation: &str,
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<FormResourcePlanKind, ApplyPlanError> {
    let target = qualified_target(args, op_index, binding)?;
    match operation {
        "supportCapability.set" => {
            let values = required_values(args, op_index, &["enabled", "rule"])?;
            let enabled =
                match (values.get("enabled"), values.get("rule")) {
                    (Some(Value::Bool(enabled)), None) => *enabled,
                    (None, Some(Value::String(rule))) => match rule.as_str() {
                        "on" | "EditableSupportEnabled" | "enabled" => true,
                        "off" | "EditableSupportDisabled" | "disabled" => false,
                        other => {
                            return Err(bad(
                                op_index,
                                "values.rule",
                                format!("unknown support capability `{other}`; use `on` or `off`"),
                            ))
                        }
                    },
                    _ => return Err(bad(
                        op_index,
                        "values",
                        "supportCapability.set takes `enabled` (boolean) or `rule` (`on`/`off`)",
                    )),
                };
            Ok(FormResourcePlanKind::SupportCapability(if enabled {
                SupportCapability::On
            } else {
                SupportCapability::Off
            }))
        }
        "supportRule.set" => {
            let values = required_values(args, op_index, &["rule"])?;
            let rule = required_string(values, "rule", op_index, "values")?;
            let rule = match rule {
                "NotEditable" | "locked" => SupportObjectRule::Locked,
                "EditableWithSupport" | "editable" => SupportObjectRule::Editable,
                "NotSupported" | "off-support" => SupportObjectRule::OffSupport,
                other => {
                    return Err(bad(
                        op_index,
                        "values.rule",
                        format!(
                        "unknown support rule `{other}`; use `locked`, `editable` or `off-support`"
                    ),
                    ))
                }
            };
            let [owner] = target.segments() else {
                return Err(bad(
                    op_index,
                    "at",
                    "supportRule.set targets one top-level metadata object",
                ));
            };
            let name = owner
                .name()
                .ok_or_else(|| bad(op_index, "at", "the object must be named"))?;
            let target = MetadataAddress::parse(
                PLATFORM_XML_8_3_27_FORMAT_2_20,
                &format!("{}.{name}", owner.kind().as_str()),
            )
            .map_err(|error| bad(op_index, "at", error.to_string()))?;
            Ok(FormResourcePlanKind::SupportRule { target, rule })
        }
        _ => Ok(FormResourcePlanKind::Unsupported),
    }
}

/// The owner object of a form address (`Owner.Name[.Form.F[.Item.X]]`).
struct FormTarget {
    owner: MetadataAddress,
    owner_kind: String,
    owner_name: String,
    form: Option<String>,
    item: Option<String>,
}

fn form_target(target: &QualifiedAddress, op_index: usize) -> Result<FormTarget, ApplyPlanError> {
    let segments = target.segments();
    let [owner, rest @ ..] = segments else {
        return Err(bad(
            op_index,
            "at",
            "form operations address a metadata owner or its form",
        ));
    };
    if owner.kind() == NodeKind::Configuration || owner.kind() == NodeKind::Form {
        return Err(bad(
            op_index,
            "at",
            "form operations address `Owner.Name`, `Owner.Name.Form.F` or `Owner.Name.Form.F.Item.X`",
        ));
    }
    let owner_name = owner
        .name()
        .ok_or_else(|| bad(op_index, "at", "the form owner must be named"))?
        .to_string();
    let owner_kind = owner.kind().as_str().to_string();
    let address = MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!("{owner_kind}.{owner_name}"),
    )
    .map_err(|error| bad(op_index, "at", error.to_string()))?;
    let mut form = None;
    let mut item = None;
    match rest {
        [] => {}
        [form_segment] if form_segment.kind() == NodeKind::Form => {
            form = Some(
                form_segment
                    .name()
                    .ok_or_else(|| bad(op_index, "at", "the form must be named"))?
                    .to_string(),
            );
        }
        [form_segment, item_segment]
            if form_segment.kind() == NodeKind::Form && item_segment.kind() == NodeKind::Item =>
        {
            form = Some(
                form_segment
                    .name()
                    .ok_or_else(|| bad(op_index, "at", "the form must be named"))?
                    .to_string(),
            );
            item = Some(
                item_segment
                    .name()
                    .ok_or_else(|| bad(op_index, "at", "the form item must be named"))?
                    .to_string(),
            );
        }
        _ => {
            return Err(bad(
                op_index,
                "at",
                "form operations address `Owner.Name`, `Owner.Name.Form.F` or `Owner.Name.Form.F.Item.X`",
            ))
        }
    }
    Ok(FormTarget {
        owner: address,
        owner_kind,
        owner_name,
        form,
        item,
    })
}

fn form_purpose(
    kind: Option<&str>,
    op_index: usize,
    location: &str,
) -> Result<String, ApplyPlanError> {
    Ok(match kind.map(str::trim).filter(|kind| !kind.is_empty()) {
        None | Some("ObjectForm") | Some("Object") | Some("object") => "Object".to_string(),
        Some("ListForm") | Some("List") | Some("list") => "List".to_string(),
        Some("ChoiceForm") | Some("Choice") | Some("choice") => "Choice".to_string(),
        Some("RecordForm") | Some("RecordSetForm") | Some("Record") | Some("record") => {
            "Record".to_string()
        }
        Some("SettingsForm") | Some("Settings") | Some("FolderForm") | Some("Generic") => {
            "Object".to_string()
        }
        Some(other) => {
            return Err(bad(
                op_index,
                &format!("{location}.type"),
                format!(
                    "unknown form type `{other}`; use ObjectForm, ListForm, ChoiceForm, RecordSetForm or SettingsForm"
                ),
            ))
        }
    })
}

fn default_form_property(key: &str) -> Option<&'static str> {
    Some(match key {
        "defaultObjectForm" => "DefaultObjectForm",
        "defaultListForm" => "DefaultListForm",
        "defaultChoiceForm" => "DefaultChoiceForm",
        "defaultFolderForm" => "DefaultFolderForm",
        "defaultFolderChoiceForm" => "DefaultFolderChoiceForm",
        "defaultRecordForm" => "DefaultRecordForm",
        "defaultForm" => "DefaultForm",
        "defaultSettingsForm" => "DefaultSettingsForm",
        "defaultVariantForm" => "DefaultVariantForm",
        "defaultSaveForm" => "DefaultSaveForm",
        "auxiliaryObjectForm" => "AuxiliaryObjectForm",
        "auxiliaryListForm" => "AuxiliaryListForm",
        "auxiliaryChoiceForm" => "AuxiliaryChoiceForm",
        _ => return None,
    })
}

fn form_attribute_type(type_name: &str) -> String {
    match type_name {
        "Boolean" | "boolean" | "Булево" => "xs:boolean".to_string(),
        "String" | "string" | "Строка" => "xs:string".to_string(),
        "Number" | "number" | "Число" => "xs:decimal".to_string(),
        "Date" | "date" | "Дата" => "xs:dateTime".to_string(),
        other => other.to_string(),
    }
}

/// The legacy element definition for one typed v0.13 item: the discriminator
/// key named after the element type, every other property passed through.
fn legacy_element_definition(
    item: &Map<String, Value>,
    op_index: usize,
    location: &str,
) -> Result<(Value, Option<String>, Option<String>), ApplyPlanError> {
    let name = required_string(item, "name", op_index, location)?.to_string();
    let kind = item
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("InputField");
    let mut definition = Map::new();
    definition.insert("name".to_string(), Value::String(name.clone()));
    match kind {
        "InputField" | "Input" | "Field" => {}
        "Group" | "UsualGroup" => {
            let orientation = item
                .get("orientation")
                .and_then(Value::as_str)
                .unwrap_or("vertical");
            definition.insert("group".to_string(), Value::String(orientation.to_string()));
        }
        "Table" => {
            definition.insert("table".to_string(), Value::String(name.clone()));
        }
        "Button" => {
            definition.insert("button".to_string(), Value::String(name.clone()));
        }
        "Label" | "LabelDecoration" => {
            definition.insert("label".to_string(), Value::String(name.clone()));
        }
        "LabelField" => {
            definition.insert("labelField".to_string(), Value::String(name.clone()));
        }
        "CheckBox" | "CheckBoxField" => {
            definition.insert("check".to_string(), Value::String(name.clone()));
        }
        "Pages" => {
            definition.insert("pages".to_string(), Value::String(name.clone()));
        }
        "Page" => {
            definition.insert("page".to_string(), Value::String(name.clone()));
        }
        "CommandBar" => {
            definition.insert("cmdBar".to_string(), Value::String(name.clone()));
        }
        other => {
            return Err(bad(
                op_index,
                &format!("{location}.type"),
                format!(
                    "unknown form element type `{other}`; use InputField, Group, Table, Button, Label, LabelField, CheckBox, Pages, Page or CommandBar"
                ),
            ))
        }
    }
    let mut into = None;
    let mut after = None;
    for (key, value) in item {
        match key.as_str() {
            "name" | "type" | "orientation" => {}
            "after" => after = value.as_str().map(str::to_string),
            "into" => into = value.as_str().map(str::to_string),
            _ => {
                definition.insert(key.clone(), value.clone());
            }
        }
    }
    Ok((Value::Object(definition), into, after))
}

/// The registry spells form events in English; Russian names travel in from
/// developers and are mapped here, unknown names pass through to the matrix.
fn canonical_form_event(name: &str) -> &str {
    match name {
        "ПриОткрытии" => "OnOpen",
        "ПриПовторномОткрытии" => "OnReopen",
        "ПриЗакрытии" => "OnClose",
        "ПередЗакрытием" => "BeforeClose",
        "ПриСозданииНаСервере" => "OnCreateAtServer",
        "ПриЧтенииНаСервере" => "OnReadAtServer",
        "ПередЗаписью" => "BeforeWrite",
        "ПередЗаписьюНаСервере" => "BeforeWriteAtServer",
        "ПриЗаписиНаСервере" => "OnWriteAtServer",
        "ПослеЗаписи" => "AfterWrite",
        "ПослеЗаписиНаСервере" => "AfterWriteAtServer",
        "ОбработкаВыбора" => "ChoiceProcessing",
        "ОбработкаОповещения" => "NotificationProcessing",
        "ОбработкаПроверкиЗаполненияНаСервере" => {
            "FillCheckProcessingAtServer"
        }
        "ПриЗагрузкеДанныхИзНастроекНаСервере" => {
            "OnLoadDataFromSettingsAtServer"
        }
        "ПередЗагрузкойДанныхИзНастроекНаСервере" => {
            "BeforeLoadDataFromSettingsAtServer"
        }
        "ПриСохраненииДанныхВНастройкахНаСервере" => {
            "OnSaveDataInSettingsAtServer"
        }
        "ПриИзменении" => "OnChange",
        "НачалоВыбора" => "StartChoice",
        "Очистка" => "Clearing",
        "АвтоПодбор" => "AutoComplete",
        "ОкончаниеВводаТекста" => "TextEditEnd",
        "Нажатие" => "Click",
        "Выбор" => "Selection",
        "ПриАктивизацииСтроки" => "OnActivateRow",
        "ПередНачаломДобавления" => "BeforeAddRow",
        "ПередУдалением" => "BeforeDeleteRow",
        "ПередНачаломИзменения" => "BeforeRowChange",
        "ПриНачалеРедактирования" => "OnStartEdit",
        "ПриОкончанииРедактирования" => "OnEditEnd",
        "ПередОкончаниемРедактирования" => "BeforeEditEnd",
        "ПриАктивизацииЯчейки" => "OnActivateCell",
        "ПриАктивизацииПоля" => "OnActivateField",
        "ПередРазворачиванием" => "BeforeExpand",
        "ПриПолученииДанныхНаСервере" => "OnGetDataAtServer",
        "ПередЗагрузкойПользовательскихНастроекНаСервере" => {
            "BeforeLoadUserSettingsAtServer"
        }
        "ПриЗагрузкеПользовательскихНастроекНаСервере" => {
            "OnLoadUserSettingsAtServer"
        }
        "ПриОбновленииСоставаПользовательскихНастроекНаСервере" => {
            "OnUpdateUserSettingSetAtServer"
        }
        other => other,
    }
}

fn parse_form_operation(
    operation: &str,
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<FormResourcePlanKind, ApplyPlanError> {
    let address = qualified_target(args, op_index, binding)?;
    let target = form_target(&address, op_index)?;
    let items_path = format!("ops[{op_index}].args.items");
    let form_of = |target: &FormTarget| -> Result<String, ApplyPlanError> {
        target.form.clone().ok_or_else(|| {
            bad(
                op_index,
                "at",
                "this operation addresses one form: `Owner.Name.Form.F`",
            )
        })
    };
    match operation {
        "form.add" => {
            if target.form.is_some() {
                return Err(bad(
                    op_index,
                    "at",
                    "form.add targets the owner; name the forms in `items`",
                ));
            }
            let items = args
                .get("items")
                .and_then(Value::as_array)
                .ok_or_else(|| bad(op_index, "items", "`items` must be an array"))?;
            let mut forms = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                let location = format!("items[{index}]");
                let item = item
                    .as_object()
                    .ok_or_else(|| bad(op_index, &location, "each item must be an object"))?;
                let name = required_string(item, "name", op_index, &location)?.to_string();
                let purpose = form_purpose(
                    item.get("type").and_then(Value::as_str),
                    op_index,
                    &location,
                )?;
                forms.push((name, purpose));
            }
            if forms.is_empty() {
                return Err(bad(
                    op_index,
                    "items",
                    "`items` must list at least one form",
                ));
            }
            Ok(FormResourcePlanKind::FormAdd {
                owner: target.owner,
                forms,
            })
        }
        "form.create" => {
            let form = form_of(&target)?;
            let values = required_values(args, op_index, &["name", "type"])?;
            if let Some(name) = values.get("name").and_then(Value::as_str) {
                if name != form {
                    return Err(bad(
                        op_index,
                        "values.name",
                        "`values.name` must match the form named in `at`",
                    ));
                }
            }
            let purpose = form_purpose(
                values.get("type").and_then(Value::as_str),
                op_index,
                "values",
            )?;
            Ok(FormResourcePlanKind::FormAdd {
                owner: target.owner,
                forms: vec![(form, purpose)],
            })
        }
        "form.set" => {
            let values = args
                .get("values")
                .and_then(Value::as_object)
                .ok_or_else(|| bad(op_index, "values", "`values` must be an object"))?;
            let mut defaults = Vec::new();
            for (key, value) in values {
                let property = default_form_property(key).ok_or_else(|| {
                    bad(
                        op_index,
                        &format!("values.{key}"),
                        format!("unknown form slot `{key}`; use defaultObjectForm, defaultListForm, defaultChoiceForm, defaultRecordForm, defaultForm, defaultSettingsForm, ..."),
                    )
                })?;
                let form_name = value
                    .as_str()
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .ok_or_else(|| {
                        bad(
                            op_index,
                            &format!("values.{key}"),
                            "the form slot takes a form name",
                        )
                    })?;
                let form_name = form_name
                    .rsplit(".Form.")
                    .next()
                    .unwrap_or(form_name)
                    .to_string();
                defaults.push((property.to_string(), form_name));
            }
            if defaults.is_empty() {
                return Err(bad(
                    op_index,
                    "values",
                    "form.set needs at least one form slot",
                ));
            }
            Ok(FormResourcePlanKind::FormSet {
                owner: target.owner,
                defaults,
            })
        }
        "form.remove" => {
            let form = form_of(&target)?;
            Ok(FormResourcePlanKind::FormRemove {
                owner: target.owner,
                form,
            })
        }
        "element.add" => {
            let form = form_of(&target)?;
            let items = args
                .get("items")
                .and_then(Value::as_array)
                .ok_or_else(|| bad(op_index, "items", "`items` must be an array"))?;
            let mut definitions = Vec::new();
            for (index, item) in items.iter().enumerate() {
                let location = format!("items[{index}]");
                let item = item
                    .as_object()
                    .ok_or_else(|| bad(op_index, &location, "each item must be an object"))?;
                let (definition, into, after) =
                    legacy_element_definition(item, op_index, &location)?;
                let mut wrapper = Map::new();
                wrapper.insert("elements".to_string(), Value::Array(vec![definition]));
                if let Some(into) = into {
                    wrapper.insert("into".to_string(), Value::String(into));
                }
                if let Some(after) = after {
                    wrapper.insert("after".to_string(), Value::String(after));
                }
                definitions.push(Value::Object(wrapper));
            }
            if definitions.is_empty() {
                return Err(bad(
                    op_index,
                    &items_path,
                    "`items` must list at least one element",
                ));
            }
            Ok(FormResourcePlanKind::FormEdit {
                owner: target.owner,
                form,
                definitions,
            })
        }
        "element.remove" => {
            let form = form_of(&target)?;
            let names = match &target.item {
                Some(item) => vec![item.clone()],
                None => listed_names(args, "name", op_index).map_err(|_| {
                    bad(
                        op_index,
                        "at",
                        "element.remove names the element in the address (`...Form.F.Item.X`) or in `items`",
                    )
                })?,
            };
            let definition = serde_json::json!({
                "removeElements": names
                    .into_iter()
                    .map(|name| serde_json::json!({"name": name}))
                    .collect::<Vec<_>>()
            });
            Ok(FormResourcePlanKind::FormEdit {
                owner: target.owner,
                form,
                definitions: vec![definition],
            })
        }
        "formAttribute.add" | "formCommand.add" => {
            let form = form_of(&target)?;
            let items = args
                .get("items")
                .and_then(Value::as_array)
                .ok_or_else(|| bad(op_index, "items", "`items` must be an array"))?;
            let mut entries = Vec::new();
            for (index, item) in items.iter().enumerate() {
                let location = format!("items[{index}]");
                let item = item
                    .as_object()
                    .ok_or_else(|| bad(op_index, &location, "each item must be an object"))?;
                let name = required_string(item, "name", op_index, &location)?.to_string();
                let mut entry = item.clone();
                entry.insert("name".to_string(), Value::String(name.clone()));
                if operation == "formAttribute.add" {
                    if let Some(type_name) = item.get("type").and_then(Value::as_str) {
                        entry.insert(
                            "type".to_string(),
                            Value::String(form_attribute_type(type_name)),
                        );
                    }
                } else if !entry.contains_key("action") {
                    entry.insert("action".to_string(), Value::String(name));
                }
                entries.push(Value::Object(entry));
            }
            if entries.is_empty() {
                return Err(bad(
                    op_index,
                    &items_path,
                    "`items` must list at least one entry",
                ));
            }
            let key = if operation == "formAttribute.add" {
                "attributes"
            } else {
                "commands"
            };
            let mut wrapper = Map::new();
            wrapper.insert(key.to_string(), Value::Array(entries));
            Ok(FormResourcePlanKind::FormEdit {
                owner: target.owner,
                form,
                definitions: vec![Value::Object(wrapper)],
            })
        }
        "event.bind" => {
            let form = form_of(&target)?;
            let values =
                required_values(args, op_index, &["event", "handler", "element", "callType"])?;
            let event = required_string(values, "event", op_index, "values")?;
            let handler = required_string(values, "handler", op_index, "values")?;
            let mut entry = Map::new();
            entry.insert(
                "name".to_string(),
                Value::String(canonical_form_event(event).to_string()),
            );
            entry.insert("handler".to_string(), Value::String(handler.to_string()));
            if let Some(call_type) = values.get("callType").and_then(Value::as_str) {
                entry.insert("callType".to_string(), Value::String(call_type.to_string()));
            }
            let element = values
                .get("element")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| target.item.clone());
            let definition = match element {
                Some(element) => {
                    entry.insert("element".to_string(), Value::String(element));
                    serde_json::json!({"elementEvents": [Value::Object(entry)]})
                }
                None => serde_json::json!({"formEvents": [Value::Object(entry)]}),
            };
            Ok(FormResourcePlanKind::FormEdit {
                owner: target.owner,
                form,
                definitions: vec![definition],
            })
        }
        _ => Ok(FormResourcePlanKind::Unsupported),
    }
}

/// `Kind.Name` → `Kinds/Name.xml`, the owner descriptor of a form.
fn owner_descriptor_relative(
    owner: &MetadataAddress,
    authority: &FormResourceApplyAuthority<'_>,
    op_index: usize,
) -> Result<PathBuf, ApplyPlanError> {
    crate::infrastructure::logical_event_source::metadata_descriptor_relative(
        owner,
        authority.source_kind(),
    )
    .map_err(|message| bad(op_index, "at", message))
}

fn read_required(
    staged: &mut ApplyStagedState,
    relative: &std::path::Path,
    what: &str,
    at_path: &str,
) -> Result<Vec<u8>, ApplyPlanError> {
    staged
        .read(relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.to_string()))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                format!("{what} was not found"),
            )
            .at_path(at_path.to_string())
        })
}

fn utf8_body(bytes: &[u8], what: &str, at_path: &str) -> Result<String, ApplyPlanError> {
    String::from_utf8(bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes).to_vec()).map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            format!("{what} is not UTF-8"),
        )
        .at_path(at_path.to_string())
    })
}

fn with_bom(text: &str) -> Vec<u8> {
    let mut bytes = UTF8_BOM.to_vec();
    bytes.extend_from_slice(text.as_bytes());
    bytes
}

/// Clears every default-form slot of the owner that points at `form`.
fn clear_default_form_slots(owner_text: &str, reference: &str) -> String {
    let mut text = owner_text.to_string();
    for property in [
        "DefaultObjectForm",
        "DefaultListForm",
        "DefaultChoiceForm",
        "DefaultFolderForm",
        "DefaultFolderChoiceForm",
        "DefaultRecordForm",
        "DefaultForm",
        "DefaultSettingsForm",
        "DefaultVariantForm",
        "DefaultSaveForm",
        "AuxiliaryObjectForm",
        "AuxiliaryListForm",
        "AuxiliaryChoiceForm",
    ] {
        let filled = format!("<{property}>{reference}</{property}>");
        if text.contains(&filled) {
            text = text.replace(&filled, &format!("<{property}/>"));
        }
    }
    text
}

fn stage_form_add(
    staged: &mut ApplyStagedState,
    authority: &FormResourceApplyAuthority<'_>,
    owner: &MetadataAddress,
    forms: &[(String, String)],
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    use crate::infrastructure::native_operations::form::{
        form_add_content_xml, form_add_metadata_xml, form_add_module_bsl, form_default_property,
        replace_form_default_property,
    };
    let at_path = format!("ops[{op_index}].args.at");
    let owner_relative = owner_descriptor_relative(owner, authority, op_index)?;
    let owner_preimage = read_required(
        staged,
        &owner_relative,
        "the form owner descriptor",
        &at_path,
    )?;
    let owner_source = utf8_body(&owner_preimage, "the form owner descriptor", &at_path)?;
    let (object_type, object_name) =
        crate::infrastructure::native_operations::common::detect_form_add_object(&owner_source)
            .map_err(|message| {
                ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(at_path.clone())
            })?;
    let mut owner_text = owner_source.clone();
    let format_version = authority.expected_format();
    let forms_dir = owner_relative.with_extension("").join("Forms");
    let mut touched = Vec::new();
    for (index, (form_name, purpose)) in forms.iter().enumerate() {
        let name_path = format!("ops[{op_index}].args.items[{index}].name");
        if !crate::infrastructure::native_operations::common::is_1c_identifier(form_name) {
            return Err(bad(
                op_index,
                &format!("items[{index}].name"),
                format!("`{form_name}` is not a valid 1C identifier"),
            ));
        }
        crate::infrastructure::native_operations::common::validate_form_purpose(
            &object_type,
            purpose,
        )
        .map_err(|message| bad(op_index, &format!("items[{index}].type"), message))?;
        if owner_text.contains(&format!("<Form>{form_name}</Form>")) {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidState,
                format!("form `{form_name}` already exists on `{}`", owner.as_str()),
            )
            .at_path(name_path));
        }
        let uuid = seeded_uuid(&format!(
            "{}\0{}.Form.{form_name}",
            authority.source_set_name(),
            owner.as_str()
        ));
        let descriptor =
            form_add_metadata_xml(form_name, form_name, &object_type, format_version, &uuid);
        let content = form_add_content_xml(&object_type, &object_name, purpose, format_version)
            .map_err(|message| bad(op_index, &format!("items[{index}].type"), message))?;
        let descriptor_relative = forms_dir.join(format!("{form_name}.xml"));
        let content_relative = forms_dir.join(form_name).join("Ext/Form.xml");
        let module_relative = forms_dir.join(form_name).join("Ext/Form/Module.bsl");
        stage_new_file(
            staged,
            &descriptor_relative,
            descriptor.as_bytes(),
            &name_path,
        )?;
        stage_new_file(staged, &content_relative, content.as_bytes(), &name_path)?;
        stage_new_file(
            staged,
            &module_relative,
            form_add_module_bsl().as_bytes(),
            &name_path,
        )?;
        touched.push(descriptor_relative);
        touched.push(content_relative);
        touched.push(module_relative);
        owner_text = crate::infrastructure::native_operations::common::register_form_in_object_text(
            &owner_text,
            form_name,
        );
        let property = form_default_property(&object_type, purpose);
        let (updated, _) = replace_form_default_property(
            &owner_text,
            property,
            &format!("{object_type}.{object_name}.Form.{form_name}"),
            false,
        );
        owner_text = updated;
    }
    let owner_postimage = with_bom(
        &crate::infrastructure::native_operations::common::lxml_tree_serialized_text_like_source_preserving_final_newline(
            &owner_text,
            &owner_source,
        ),
    );
    if owner_postimage != owner_preimage {
        staged
            .replace(&owner_relative, &owner_preimage, owner_postimage)
            .map_err(|error| ApplyPlanError::staging(error, at_path))?;
        touched.push(owner_relative);
    }
    provisional.push(ProvisionalApplyEffect::spanning(
        touched,
        DomainEvent::new(DomainEventKind::FormChanged, owner.as_str().to_string()),
        op_index,
    ));
    Ok(())
}

fn stage_form_set(
    staged: &mut ApplyStagedState,
    authority: &FormResourceApplyAuthority<'_>,
    owner: &MetadataAddress,
    defaults: &[(String, String)],
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    let owner_relative = owner_descriptor_relative(owner, authority, op_index)?;
    let owner_preimage = read_required(
        staged,
        &owner_relative,
        "the form owner descriptor",
        &at_path,
    )?;
    let owner_source = utf8_body(&owner_preimage, "the form owner descriptor", &at_path)?;
    let mut owner_text = owner_source.clone();
    for (property, form_name) in defaults {
        if !owner_text.contains(&format!("<Form>{form_name}</Form>")) {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                format!("form `{form_name}` is not a form of `{}`", owner.as_str()),
            )
            .at_path(format!("ops[{op_index}].args.values")));
        }
        if !owner_text.contains(&format!("<{property}>"))
            && !owner_text.contains(&format!("<{property}/>"))
        {
            return Err(bad(
                op_index,
                "values",
                format!("`{}` has no `{property}` slot", owner.as_str()),
            ));
        }
        let (updated, _) =
            crate::infrastructure::native_operations::form::replace_form_default_property(
                &owner_text,
                property,
                &format!("{}.Form.{form_name}", owner.as_str()),
                true,
            );
        owner_text = updated;
    }
    let owner_postimage = with_bom(
        &crate::infrastructure::native_operations::common::lxml_tree_serialized_text_like_source_preserving_final_newline(
            &owner_text,
            &owner_source,
        ),
    );
    if owner_postimage == owner_preimage {
        return Ok(());
    }
    staged
        .replace(&owner_relative, &owner_preimage, owner_postimage)
        .map_err(|error| ApplyPlanError::staging(error, at_path))?;
    provisional.push(ProvisionalApplyEffect::single(
        owner_relative,
        DomainEvent::new(DomainEventKind::MetadataChanged, owner.as_str().to_string()),
        op_index,
    ));
    Ok(())
}

fn stage_form_remove(
    staged: &mut ApplyStagedState,
    authority: &FormResourceApplyAuthority<'_>,
    owner: &MetadataAddress,
    form: &str,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    super::metadata::require_untouched_staged_state(staged, "form.remove", &at_path)?;
    let owner_relative = owner_descriptor_relative(owner, authority, op_index)?;
    let owner_preimage = read_required(
        staged,
        &owner_relative,
        "the form owner descriptor",
        &at_path,
    )?;
    let owner_source = utf8_body(&owner_preimage, "the form owner descriptor", &at_path)?;
    let forms_dir = owner_relative.with_extension("").join("Forms");
    let descriptor_relative = forms_dir.join(format!("{form}.xml"));
    let descriptor_preimage = read_required(
        staged,
        &descriptor_relative,
        "the form descriptor",
        &at_path,
    )?;
    let mut touched = Vec::new();
    let root = authority.source_root();
    let payload_dir = root.join(&forms_dir).join(form);
    if payload_dir.is_dir() {
        let traversal =
            crate::infrastructure::native_operations::meta::remove::metadata_files_recursive(
                &payload_dir,
            )
            .map_err(|error| {
                ApplyPlanError::new(ApplyPlanErrorKind::ProviderUnavailable, error)
                    .at_path(at_path.clone())
            })?;
        for file in &traversal.files {
            let relative = super::metadata::staged_relative(root, file, op_index)?;
            let preimage = read_required(staged, &relative, "a form payload file", &at_path)?;
            staged
                .remove(&relative, &preimage)
                .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
            touched.push(relative);
        }
    }
    staged
        .remove(&descriptor_relative, &descriptor_preimage)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
    touched.push(descriptor_relative);
    let (deregistered, removed) =
        crate::infrastructure::native_operations::meta::remove::remove_metadata_child_text_with_flag(
            &owner_source,
            "Form",
            form,
        );
    if !removed {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            format!("`{}` does not register form `{form}`", owner.as_str()),
        )
        .at_path(at_path));
    }
    let owner_text =
        clear_default_form_slots(&deregistered, &format!("{}.Form.{form}", owner.as_str()));
    let owner_postimage =
        super::metadata::preserve_descriptor_image(&owner_preimage, &owner_source, &owner_text);
    staged
        .replace(&owner_relative, &owner_preimage, owner_postimage)
        .map_err(|error| ApplyPlanError::staging(error, at_path))?;
    touched.push(owner_relative);
    provisional.push(ProvisionalApplyEffect::spanning(
        touched,
        DomainEvent::new(
            DomainEventKind::FormChanged,
            format!("{}.Form.{form}", owner.as_str()),
        ),
        op_index,
    ));
    Ok(())
}

fn stage_form_edit(
    staged: &mut ApplyStagedState,
    authority: &FormResourceApplyAuthority<'_>,
    owner: &MetadataAddress,
    form: &str,
    definitions: &[Value],
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    let owner_relative = owner_descriptor_relative(owner, authority, op_index)?;
    let relative = owner_relative
        .with_extension("")
        .join("Forms")
        .join(form)
        .join("Ext/Form.xml");
    let preimage = read_required(staged, &relative, "the form (Ext/Form.xml)", &at_path)?;
    let mut xml_text = utf8_body(&preimage, "the form", &at_path)?;
    let root_start = {
        let document = roxmltree::Document::parse(&xml_text).map_err(|error| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                format!("the form is not well-formed XML: {error}"),
            )
            .at_path(at_path.clone())
        })?;
        let root = document.root_element();
        crate::infrastructure::native_operations::form::require_form_root(root).map_err(
            |error| {
                ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, error)
                    .at_path(at_path.clone())
            },
        )?;
        root.range().start
    };
    let absolute = authority.source_root().join(&relative);
    let args_path = format!("ops[{op_index}].args");
    for definition in definitions {
        crate::infrastructure::native_operations::form::form_edit_apply_definition(
            &mut xml_text,
            definition,
            &absolute,
            root_start,
        )
        .map_err(|message| {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(args_path.clone())
        })?;
    }
    let postimage = with_bom(&xml_text);
    if postimage == preimage {
        return Ok(());
    }
    let validation = crate::infrastructure::native_operations::form::validate_form_with_source(
        &Map::new(),
        authority.workspace_context(),
        Some((&absolute, &xml_text)),
    );
    crate::infrastructure::native_operations::form::form_edit_require_valid(validation).map_err(
        |message| {
            ApplyPlanError::new(ApplyPlanErrorKind::Postcondition, message).at_path(args_path)
        },
    )?;
    staged
        .replace(&relative, &preimage, postimage)
        .map_err(|error| ApplyPlanError::staging(error, at_path))?;
    provisional.push(ProvisionalApplyEffect::single(
        relative,
        DomainEvent::new(
            DomainEventKind::FormChanged,
            format!("{}.Form.{form}", owner.as_str()),
        ),
        op_index,
    ));
    Ok(())
}

fn minimal_rights_xml() -> String {
    concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<Rights xmlns=\"http://v8.1c.ru/8.2/roles\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"Rights\" version=\"2.20\">\n",
        "\t<setForNewObjects>false</setForNewObjects>\n",
        "\t<setForAttributesByDefault>true</setForAttributesByDefault>\n",
        "\t<independentRightsOfChildObjects>false</independentRightsOfChildObjects>\n",
        "</Rights>\n",
    )
    .to_string()
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Ensures `<object><name>X</name></object>` exists in the rights document.
fn ensure_rights_object(body: &str, object: &str) -> Result<String, String> {
    let document = roxmltree::Document::parse(body)
        .map_err(|_| "Rights.xml is not well-formed XML".to_string())?;
    let root = document.root_element();
    let exists = root
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "object")
        .any(|node| {
            node.children().any(|child| {
                child.is_element()
                    && child.tag_name().name() == "name"
                    && child.text().map(str::trim) == Some(object)
            })
        });
    if exists {
        return Ok(body.to_string());
    }
    // Insert before the first restriction template, else before the root close.
    let insert_at = root
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == "restrictionTemplate")
        .map(|node| node.range().start)
        .or_else(|| body.rfind("</Rights>"))
        .ok_or_else(|| "Rights.xml has no closing root element".to_string())?;
    let line_start = body[..insert_at]
        .rfind('\n')
        .map_or(insert_at, |index| index + 1);
    let indent = body[line_start..insert_at]
        .chars()
        .take_while(|ch| *ch == '\t' || *ch == ' ')
        .collect::<String>();
    let unit = if indent.starts_with(' ') {
        " ".repeat(4)
    } else {
        "\t".to_string()
    };
    let block = format!(
        "{indent}<object>\n{indent}{unit}<name>{}</name>\n{indent}</object>\n",
        xml_escape(object)
    );
    let mut updated = body.to_string();
    updated.insert_str(line_start, &block);
    Ok(updated)
}

/// Sets or replaces the restriction condition of one right, creating the
/// right (granted) when the object does not list it yet.
fn set_rights_restriction(
    body: &str,
    object: &str,
    right: &str,
    condition: &str,
) -> Result<String, String> {
    let document = roxmltree::Document::parse(body)
        .map_err(|_| "Rights.xml is not well-formed XML".to_string())?;
    let root = document.root_element();
    let object_node = root
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "object")
        .find(|node| {
            node.children().any(|child| {
                child.is_element()
                    && child.tag_name().name() == "name"
                    && child.text().map(str::trim) == Some(object)
            })
        })
        .ok_or_else(|| format!("role object `{object}` was not found"))?;
    let right_node = object_node
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "right")
        .find(|node| {
            node.children().any(|child| {
                child.is_element()
                    && child.tag_name().name() == "name"
                    && child.text().map(str::trim) == Some(right)
            })
        })
        .ok_or_else(|| format!("right `{right}` of `{object}` is not set; set its value first"))?;
    let escaped = xml_escape(condition);
    if let Some(existing) = right_node
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == "restrictionByCondition")
    {
        let condition_node = existing
            .children()
            .find(|node| node.is_element() && node.tag_name().name() == "condition");
        let range = match condition_node {
            Some(node) => node.range(),
            None => existing.range(),
        };
        let indent = body[..range.start]
            .rfind('\n')
            .map(|index| {
                body[index + 1..range.start]
                    .chars()
                    .take_while(|ch| *ch == '\t' || *ch == ' ')
                    .collect::<String>()
            })
            .unwrap_or_default();
        let replacement = if condition_node.is_some() {
            format!("<condition>{escaped}</condition>")
        } else {
            format!(
                "<restrictionByCondition>\n{indent}\t<condition>{escaped}</condition>\n{indent}</restrictionByCondition>"
            )
        };
        let mut updated = body.to_string();
        updated.replace_range(range, &replacement);
        return Ok(updated);
    }
    let value_node = right_node
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == "value")
        .ok_or_else(|| format!("right `{right}` of `{object}` has no value element"))?;
    let range = value_node.range();
    let indent = body[..range.start]
        .rfind('\n')
        .map(|index| {
            body[index + 1..range.start]
                .chars()
                .take_while(|ch| *ch == '\t' || *ch == ' ')
                .collect::<String>()
        })
        .unwrap_or_default();
    let unit = if indent.starts_with(' ') {
        " ".repeat(4)
    } else {
        "\t".to_string()
    };
    let block = format!(
        "\n{indent}<restrictionByCondition>\n{indent}{unit}<condition>{escaped}</condition>\n{indent}</restrictionByCondition>"
    );
    let mut updated = body.to_string();
    updated.insert_str(range.end, &block);
    Ok(updated)
}

/// A version-4 shaped uuid derived from the plan identity, so a preview and
/// its publication produce the same descriptor bytes.
fn seeded_uuid(seed: &str) -> String {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(format!("unica-v13-form-resource-uuid-v1\0{seed}"));
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes).to_string()
}

fn subsystem_stub_xml(name: &str, format_version: &str, uuid: &str) -> String {
    let model = crate::infrastructure::native_operations::subsystem::SubsystemEditModel {
        version: format_version.to_string(),
        uuid: uuid.to_string(),
        name: name.to_string(),
        synonym: String::new(),
        comment: String::new(),
        include_help: "true".to_string(),
        include_ci: "true".to_string(),
        use_one_command: "false".to_string(),
        explanation: String::new(),
        picture: String::new(),
        content: Vec::new(),
        children: Vec::new(),
    };
    crate::infrastructure::native_operations::common::emit_subsystem_edit_model(&model)
}

fn stage_new_file(
    staged: &mut ApplyStagedState,
    relative: &std::path::Path,
    text: &[u8],
    at_path: &str,
) -> Result<(), ApplyPlanError> {
    let existing = staged
        .read(relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.to_string()))?;
    if existing.is_some() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            format!("`{}` already exists", relative.display()),
        )
        .at_path(at_path.to_string()));
    }
    let mut bytes = UTF8_BOM.to_vec();
    bytes.extend_from_slice(text.strip_prefix(UTF8_BOM).unwrap_or(text));
    staged
        .create(relative, bytes)
        .map_err(|error| ApplyPlanError::staging(error, at_path.to_string()))
}

/// Adds `<Kind>Name</Kind>` to the configuration's child objects.
fn register_child(
    staged: &mut ApplyStagedState,
    kind: &str,
    name: &str,
    op_index: usize,
) -> Result<PathBuf, ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    let owner = PathBuf::from("Configuration.xml");
    let preimage = staged
        .read(&owner)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                "the configuration descriptor was not found",
            )
            .at_path(at_path.clone())
        })?;
    let Some(postimage) = owner_registration_image(&preimage, kind, name, true, op_index)? else {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            format!("the configuration already registers `{kind}.{name}`"),
        )
        .at_path(format!("ops[{op_index}].args.values.name")));
    };
    staged
        .replace(&owner, &preimage, postimage)
        .map_err(|error| ApplyPlanError::staging(error, at_path))?;
    Ok(owner)
}

pub(crate) fn plan_form_resource_batch(
    staged: ApplyStagedState,
    authority: FormResourceApplyAuthority<'_>,
    operations: &[IndexedPlanOperation<FormResourcePlanOperation>],
) -> Result<(ApplyStagedState, Vec<ProvisionalApplyEffect>), ApplyPlanError> {
    if operations.is_empty() {
        return Err(empty_apply_family_batch());
    }
    if !authority.owns_staged_state(&staged) {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            "form/resource planner authority does not own the staged state",
        )
        .at_path("ops"));
    }
    let mut staged = staged;
    let mut provisional = Vec::new();
    for operation in operations {
        let op_index = operation.index();
        match &operation.operation().kind {
            FormResourcePlanKind::Unsupported => {
                return Err(hidden_apply_family_unimplemented(op_index));
            }
            FormResourcePlanKind::CreateRole { name } => {
                let name_path = format!("ops[{op_index}].args.values.name");
                let uuid = seeded_uuid(&format!("{}\0Role.{name}", authority.source_set_name()));
                let descriptor = crate::infrastructure::native_operations::role::role_metadata_xml(
                    name,
                    name,
                    "",
                    authority.expected_format(),
                    &uuid,
                );
                let mut touched = Vec::new();
                let descriptor_relative = PathBuf::from("Roles").join(format!("{name}.xml"));
                stage_new_file(
                    &mut staged,
                    &descriptor_relative,
                    descriptor.as_bytes(),
                    &name_path,
                )?;
                touched.push(descriptor_relative);
                let rights_relative = PathBuf::from("Roles").join(name).join("Ext/Rights.xml");
                stage_new_file(
                    &mut staged,
                    &rights_relative,
                    minimal_rights_xml().as_bytes(),
                    &name_path,
                )?;
                touched.push(rights_relative);
                touched.push(register_child(&mut staged, "Role", name, op_index)?);
                provisional.push(ProvisionalApplyEffect::spanning(
                    touched,
                    DomainEvent::new(DomainEventKind::RoleChanged, format!("Role.{name}")),
                    op_index,
                ));
            }
            FormResourcePlanKind::CreateSubsystem { name } => {
                let name_path = format!("ops[{op_index}].args.values.name");
                let descriptor = subsystem_stub_xml(
                    name,
                    authority.expected_format(),
                    &seeded_uuid(&format!(
                        "{}\0Subsystem.{name}",
                        authority.source_set_name()
                    )),
                );
                let mut touched = Vec::new();
                let descriptor_relative = PathBuf::from("Subsystems").join(format!("{name}.xml"));
                stage_new_file(
                    &mut staged,
                    &descriptor_relative,
                    descriptor.as_bytes(),
                    &name_path,
                )?;
                touched.push(descriptor_relative);
                touched.push(register_child(&mut staged, "Subsystem", name, op_index)?);
                provisional.push(ProvisionalApplyEffect::spanning(
                    touched,
                    DomainEvent::new(
                        DomainEventKind::SubsystemChanged,
                        format!("Subsystem.{name}"),
                    ),
                    op_index,
                ));
            }
            FormResourcePlanKind::RightSet {
                role,
                object,
                right,
                value,
                rls,
            } => {
                let at_path = format!("ops[{op_index}].args.at");
                let relative =
                    attached_resource_relative(role, "Rights.xml", authority.source_kind())
                        .map_err(|message| {
                            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message)
                                .at_path(at_path.clone())
                        })?;
                let preimage = staged
                    .read(&relative)
                    .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
                    .ok_or_else(|| {
                        ApplyPlanError::new(
                            ApplyPlanErrorKind::NotFound,
                            "the role rights document (Ext/Rights.xml) was not found",
                        )
                        .at_path(at_path.clone())
                    })?;
                let invalid = |message: String| {
                    ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message)
                        .at_path(at_path.clone())
                };
                let (bom, mut body) =
                    crate::infrastructure::native_operations::role::decode_role_xml(&preimage)
                        .map_err(invalid)?;
                crate::infrastructure::native_operations::role::validate_role_rights_document(
                    &body,
                    crate::infrastructure::native_operations::role::RoleValueScope::SourceImage,
                )
                .map_err(invalid)?;
                body = ensure_rights_object(&body, object).map_err(invalid)?;
                let value_path = format!("ops[{op_index}].args.values");
                let granted = value.or(if rls.is_some() { Some(true) } else { None });
                if let Some(granted) = granted {
                    let edit = crate::domain::role::RoleEditOperation {
                        object_name: object.clone(),
                        right: right.clone(),
                        value: granted,
                    };
                    let (updated, _effect) =
                        crate::infrastructure::native_operations::role::apply_role_edit_operation(
                            &body, &edit, op_index,
                        )
                        .map_err(|message| {
                            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message)
                                .at_path(value_path.clone())
                        })?;
                    body = updated;
                }
                if let Some(condition) = rls {
                    body = set_rights_restriction(&body, object, right, condition).map_err(
                        |message| {
                            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message)
                                .at_path(format!("{value_path}.rls"))
                        },
                    )?;
                }
                crate::infrastructure::native_operations::role::validate_role_rights_document(
                    &body,
                    crate::infrastructure::native_operations::role::RoleValueScope::SourceImage,
                )
                .map_err(|message| {
                    ApplyPlanError::new(ApplyPlanErrorKind::Postcondition, message)
                        .at_path(value_path.clone())
                })?;
                let postimage =
                    crate::infrastructure::native_operations::role::encode_role_xml(bom, &body);
                if postimage == preimage {
                    continue;
                }
                staged
                    .replace(&relative, &preimage, postimage)
                    .map_err(|error| ApplyPlanError::staging(error, at_path))?;
                provisional.push(ProvisionalApplyEffect::single(
                    relative,
                    DomainEvent::new(DomainEventKind::RoleChanged, role.as_str().to_string()),
                    op_index,
                ));
            }
            FormResourcePlanKind::Subsystem {
                address,
                relative,
                edit,
            } => {
                let at_path = format!("ops[{op_index}].args.at");
                let preimage = staged
                    .read(relative)
                    .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
                    .ok_or_else(|| {
                        ApplyPlanError::new(
                            ApplyPlanErrorKind::NotFound,
                            "the subsystem descriptor was not found",
                        )
                        .at_path(at_path.clone())
                    })?;
                let text = String::from_utf8(preimage.clone()).map_err(|_| {
                    ApplyPlanError::new(
                        ApplyPlanErrorKind::InvalidSource,
                        "the subsystem descriptor is not UTF-8",
                    )
                    .at_path(at_path.clone())
                })?;
                let mut model =
                    crate::infrastructure::native_operations::common::parse_subsystem_edit_model(
                        &text,
                        &relative.display().to_string(),
                    )
                    .map_err(|message| {
                        ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message)
                            .at_path(at_path.clone())
                    })?;
                let mut touched = Vec::new();
                match edit {
                    SubsystemEdit::ContentAdd(names) => {
                        for name in names {
                            if !model.content.iter().any(|existing| existing == name) {
                                model.content.push(name.clone());
                            }
                        }
                    }
                    SubsystemEdit::ContentRemove(names) => {
                        model.content.retain(|existing| !names.contains(existing));
                    }
                    SubsystemEdit::ChildAdd(names) => {
                        let children_dir = relative.with_extension("").join("Subsystems");
                        for name in names {
                            if !crate::infrastructure::native_operations::common::is_1c_identifier(
                                name,
                            ) {
                                return Err(bad(
                                    op_index,
                                    "items",
                                    format!("`{name}` is not a valid 1C identifier"),
                                ));
                            }
                            if !model.children.iter().any(|existing| existing == name) {
                                model.children.push(name.clone());
                            }
                            let child_relative = children_dir.join(format!("{name}.xml"));
                            let existing = staged
                                .read(&child_relative)
                                .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
                            if existing.is_none() {
                                let stub = subsystem_stub_xml(
                                    name,
                                    &model.version,
                                    &seeded_uuid(&format!(
                                        "{}\0{address}.Subsystem.{name}",
                                        authority.source_set_name()
                                    )),
                                );
                                stage_new_file(
                                    &mut staged,
                                    &child_relative,
                                    stub.as_bytes(),
                                    &at_path,
                                )?;
                                touched.push(child_relative);
                            }
                        }
                    }
                    SubsystemEdit::ChildRemove(names) => {
                        let children_dir = relative.with_extension("").join("Subsystems");
                        model.children.retain(|existing| !names.contains(existing));
                        for name in names {
                            let child_relative = children_dir.join(format!("{name}.xml"));
                            if let Some(child_preimage) = staged
                                .read(&child_relative)
                                .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
                            {
                                staged.remove(&child_relative, &child_preimage).map_err(
                                    |error| ApplyPlanError::staging(error, at_path.clone()),
                                )?;
                                touched.push(child_relative);
                            }
                        }
                    }
                }
                let mut postimage = UTF8_BOM.to_vec();
                postimage.extend_from_slice(
                    crate::infrastructure::native_operations::common::emit_subsystem_edit_model(
                        &model,
                    )
                    .as_bytes(),
                );
                if postimage != preimage {
                    staged
                        .replace(relative, &preimage, postimage)
                        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
                    touched.push(relative.clone());
                }
                if !touched.is_empty() {
                    provisional.push(ProvisionalApplyEffect::spanning(
                        touched,
                        DomainEvent::new(DomainEventKind::SubsystemChanged, address.clone()),
                        op_index,
                    ));
                }
            }
            FormResourcePlanKind::FormAdd { owner, forms } => {
                stage_form_add(
                    &mut staged,
                    &authority,
                    owner,
                    forms,
                    op_index,
                    &mut provisional,
                )?;
            }
            FormResourcePlanKind::FormSet { owner, defaults } => {
                stage_form_set(
                    &mut staged,
                    &authority,
                    owner,
                    defaults,
                    op_index,
                    &mut provisional,
                )?;
            }
            FormResourcePlanKind::FormRemove { owner, form } => {
                stage_form_remove(
                    &mut staged,
                    &authority,
                    owner,
                    form,
                    op_index,
                    &mut provisional,
                )?;
            }
            FormResourcePlanKind::FormEdit {
                owner,
                form,
                definitions,
            } => {
                stage_form_edit(
                    &mut staged,
                    &authority,
                    owner,
                    form,
                    definitions,
                    op_index,
                    &mut provisional,
                )?;
            }
            FormResourcePlanKind::SupportCapability(capability) => {
                let at_path = format!("ops[{op_index}].args.at");
                let relative = PathBuf::from("Ext/ParentConfigurations.bin");
                let (preimage, text) = read_support_state(&mut staged, &relative, &at_path)?;
                let (global_flag, _) =
                    crate::infrastructure::native_operations::common::parse_support_header(&text)
                        .expect("read_support_state proved the header");
                // Already in the requested state: nothing to switch, and the
                // object rules must not be reset as a side effect.
                if (global_flag == 0) == capability.enabled() {
                    continue;
                }
                let root = authority.source_root();
                let (_, updated, _) =
                    crate::infrastructure::native_operations::support::plan_capability(
                        &root.join(&relative),
                        &text,
                        *capability,
                        root,
                    )
                    .map_err(|message| {
                        ApplyPlanError::new(ApplyPlanErrorKind::InvalidState, message)
                            .at_path(at_path.clone())
                    })?;
                stage_support_state(
                    &mut staged,
                    &relative,
                    &preimage,
                    &updated,
                    op_index,
                    &at_path,
                    &mut provisional,
                )?;
            }
            FormResourcePlanKind::SupportRule { target, rule } => {
                let at_path = format!("ops[{op_index}].args.at");
                let relative = PathBuf::from("Ext/ParentConfigurations.bin");
                let (preimage, text) = read_support_state(&mut staged, &relative, &at_path)?;
                let descriptor_relative =
                    crate::infrastructure::logical_event_source::metadata_descriptor_relative(
                        target,
                        authority.source_kind(),
                    )
                    .map_err(|message| {
                        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message)
                            .at_path(at_path.clone())
                    })?;
                let descriptor = staged
                    .read(&descriptor_relative)
                    .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
                    .ok_or_else(|| {
                        ApplyPlanError::new(
                            ApplyPlanErrorKind::NotFound,
                            "metadata descriptor was not found",
                        )
                        .at_path(at_path.clone())
                    })?;
                let uuid =
                    crate::infrastructure::native_operations::common::support_root_uuid_from_bytes(
                        &descriptor,
                    )
                    .ok_or_else(|| {
                        ApplyPlanError::new(
                            ApplyPlanErrorKind::InvalidSource,
                            "the metadata descriptor carries no object uuid",
                        )
                        .at_path(at_path.clone())
                    })?;
                let root = authority.source_root();
                let (_, updated, _) =
                    crate::infrastructure::native_operations::support::plan_object_rule(
                        &root.join(&relative),
                        &text,
                        &uuid,
                        *rule,
                        &root.join(&descriptor_relative),
                    )
                    .map_err(|message| {
                        ApplyPlanError::new(ApplyPlanErrorKind::InvalidState, message)
                            .at_path(at_path.clone())
                    })?;
                stage_support_state(
                    &mut staged,
                    &relative,
                    &preimage,
                    &updated,
                    op_index,
                    &at_path,
                    &mut provisional,
                )?;
            }
        }
    }
    Ok((staged, provisional))
}

fn read_support_state(
    staged: &mut ApplyStagedState,
    relative: &std::path::Path,
    at_path: &str,
) -> Result<(Vec<u8>, String), ApplyPlanError> {
    let preimage = staged
        .read(relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.to_string()))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidState,
                "the configuration is not on vendor support (no Ext/ParentConfigurations.bin); there is no support state to switch",
            )
            .at_path(at_path.to_string())
        })?;
    let text =
        crate::infrastructure::native_operations::support::decode_parent_configurations(&preimage)
            .map_err(|message| {
                ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message)
                    .at_path(at_path.to_string())
            })?;
    if crate::infrastructure::native_operations::common::parse_support_header(&text).is_none() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "Ext/ParentConfigurations.bin has an unknown format",
        )
        .at_path(at_path.to_string()));
    }
    Ok((preimage, text))
}

fn stage_support_state(
    staged: &mut ApplyStagedState,
    relative: &std::path::Path,
    preimage: &[u8],
    updated: &str,
    op_index: usize,
    at_path: &str,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let postimage =
        crate::infrastructure::native_operations::support::parent_configurations_bytes(updated);
    if postimage == preimage {
        return Ok(());
    }
    staged
        .replace(relative, preimage, postimage)
        .map_err(|error| ApplyPlanError::staging(error, at_path.to_string()))?;
    provisional.push(ProvisionalApplyEffect::single(
        relative.to_path_buf(),
        DomainEvent::new(
            DomainEventKind::ConfigXmlChanged,
            "Configuration".to_string(),
        ),
        op_index,
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_form_resource_plan_operation, plan_form_resource_batch};
    use crate::infrastructure::native_operations::apply::ApplyPlanErrorKind;
    use crate::infrastructure::native_operations::apply_families::request::IndexedPlanOperation;
    use crate::infrastructure::native_operations::apply_families::tests::ApplySeamFixture;
    use serde_json::json;
    use std::path::Path;

    const RIGHTS: &str = concat!(
        "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<Rights xmlns=\"http://v8.1c.ru/8.2/roles\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"Rights\" version=\"2.20\">\n",
        "\t<setForNewObjects>false</setForNewObjects>\n",
        "\t<setForAttributesByDefault>true</setForAttributesByDefault>\n",
        "\t<independentRightsOfChildObjects>false</independentRightsOfChildObjects>\n",
        "\t<object>\n\t\t<name>Document.First</name>\n\t\t<right>\n\t\t\t<name>Read</name>\n\t\t\t<value>true</value>\n\t\t</right>\n\t</object>\n",
        "</Rights>\n",
    );

    fn staged_text(
        staged: &crate::infrastructure::native_operations::apply::ApplyStagedState,
        relative: &str,
    ) -> String {
        let change = staged
            .planned_changes()
            .into_iter()
            .find(|change| change.relative_path == Path::new(relative))
            .unwrap_or_else(|| panic!("`{relative}` is staged"));
        let crate::infrastructure::native_operations::apply::StagedFileState::Bytes(bytes) =
            change.current
        else {
            panic!("`{relative}` keeps bytes");
        };
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn form_add_then_element_and_event_edits_compose_in_one_batch() {
        let fixture = ApplySeamFixture::new();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .form_resource_planning_authority(&fixture.binding)
            .unwrap();
        let parse = |op: &str, args: serde_json::Value, index: usize| {
            IndexedPlanOperation::new(
                index,
                parse_form_resource_plan_operation(op, &args, index, &fixture.binding)
                    .unwrap_or_else(|error| panic!("{op}: {error:?}")),
            )
        };
        let operations = [
            parse(
                "form.add",
                json!({"at": "main:Document.First", "items": [{"name": "ФормаДокумента", "type": "ObjectForm"}]}),
                0,
            ),
            parse(
                "element.add",
                json!({
                    "at": "main:Document.First.Form.ФормаДокумента",
                    "items": [{"name": "ПолеИтог", "path": "Объект.Total", "title": "Итог"}]
                }),
                1,
            ),
            parse(
                "event.bind",
                json!({
                    "at": "main:Document.First.Form.ФормаДокумента",
                    "values": {"event": "ПриОткрытии", "handler": "ПриОткрытии"}
                }),
                2,
            ),
        ];
        let (staged, effects) = plan_form_resource_batch(staged, authority, &operations)
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        assert_eq!(effects.len(), 3);
        let form = staged_text(&staged, "Documents/First/Forms/ФормаДокумента/Ext/Form.xml");
        assert!(form.contains("<InputField name=\"ПолеИтог\""), "{form}");
        assert!(form.contains("<DataPath>Объект.Total</DataPath>"), "{form}");
        assert!(form.contains("ПриОткрытии"), "{form}");
        let owner = staged_text(&staged, "Documents/First.xml");
        assert!(owner.contains("<Form>ФормаДокумента</Form>"), "{owner}");
        assert!(
            staged
                .planned_changes()
                .iter()
                .any(|change| change.relative_path
                    == Path::new("Documents/First/Forms/ФормаДокумента/Ext/Form/Module.bsl")),
            "the form module is staged"
        );
    }

    #[test]
    fn right_set_grants_lists_and_restricts_rights_on_the_staged_document() {
        let fixture = ApplySeamFixture::new();
        let role_dir = fixture.source_dir().join("Roles/Reader/Ext");
        std::fs::create_dir_all(&role_dir).unwrap();
        std::fs::write(
            fixture.source_dir().join("Roles/Reader.xml"),
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><Role uuid=\"33333333-3333-4333-8333-333333333333\"><Properties><Name>Reader</Name></Properties></Role></MetaDataObject>",
        )
        .unwrap();
        std::fs::write(role_dir.join("Rights.xml"), RIGHTS).unwrap();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .form_resource_planning_authority(&fixture.binding)
            .unwrap();
        let parse = |op: &str, values: serde_json::Value, index: usize| {
            IndexedPlanOperation::new(
                index,
                parse_form_resource_plan_operation(
                    op,
                    &json!({"at": "main:Role.Reader", "values": values}),
                    index,
                    &fixture.binding,
                )
                .unwrap(),
            )
        };
        let operations = [
            parse(
                "right.set",
                json!({"object": "Document.First", "right": "Update", "value": true}),
                0,
            ),
            parse(
                "right.set",
                json!({"object": "Document.Second", "right": "Read", "value": true}),
                1,
            ),
            parse(
                "right.set",
                json!({"object": "Document.Second", "right": "Read", "rls": "ГДЕ Автор = &Пользователь"}),
                2,
            ),
        ];
        let (staged, effects) = plan_form_resource_batch(staged, authority, &operations)
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        assert_eq!(effects.len(), 3);
        let text = staged_text(&staged, "Roles/Reader/Ext/Rights.xml");
        assert!(text.starts_with('\u{feff}'), "byte-order mark survives");
        assert!(text.contains("<name>Update</name>"), "{text}");
        assert!(text.contains("<name>Document.Second</name>"), "{text}");
        assert!(
            text.contains("<condition>ГДЕ Автор = &amp;Пользователь</condition>"),
            "{text}"
        );
    }

    #[test]
    fn role_create_stages_descriptor_rights_and_registration_deterministically() {
        let fixture = ApplySeamFixture::new();
        let plan = |fixture: &ApplySeamFixture| {
            let admission = fixture.admission();
            let staged = admission.staged_state().unwrap();
            let authority = admission
                .form_resource_planning_authority(&fixture.binding)
                .unwrap();
            let operation = parse_form_resource_plan_operation(
                "role.create",
                &json!({"at": "main:Configuration", "values": {"name": "Manager"}}),
                0,
                &fixture.binding,
            )
            .unwrap();
            let (staged, _) = plan_form_resource_batch(
                staged,
                authority,
                &[IndexedPlanOperation::new(0, operation)],
            )
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
            let mut paths = staged
                .planned_changes()
                .into_iter()
                .map(|change| change.relative_path)
                .collect::<Vec<_>>();
            paths.sort();
            (
                paths,
                staged_text(&staged, "Roles/Manager.xml"),
                staged_text(&staged, "Configuration.xml"),
            )
        };
        let (paths, descriptor, owner) = plan(&fixture);
        assert_eq!(
            paths,
            vec![
                std::path::PathBuf::from("Configuration.xml"),
                std::path::PathBuf::from("Roles/Manager/Ext/Rights.xml"),
                std::path::PathBuf::from("Roles/Manager.xml"),
            ]
        );
        assert!(owner.contains("<Role>Manager</Role>"), "{owner}");
        assert!(descriptor.contains("<Name>Manager</Name>"), "{descriptor}");
        let (_, descriptor_again, _) = plan(&fixture);
        assert_eq!(
            descriptor, descriptor_again,
            "the preview and the publication agree"
        );
    }

    #[test]
    fn subsystem_edits_update_content_and_children_of_the_staged_descriptor() {
        let fixture = ApplySeamFixture::new();
        std::fs::create_dir_all(fixture.source_dir().join("Subsystems")).unwrap();
        let stub =
            super::subsystem_stub_xml("Sales", "2.20", "55555555-5555-4555-8555-555555555555");
        std::fs::write(
            fixture.source_dir().join("Subsystems/Sales.xml"),
            format!("\u{feff}{stub}"),
        )
        .unwrap();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .form_resource_planning_authority(&fixture.binding)
            .unwrap();
        let parse = |op: &str, args: serde_json::Value, index: usize| {
            let mut args = args;
            args["at"] = json!("main:Subsystem.Sales");
            IndexedPlanOperation::new(
                index,
                parse_form_resource_plan_operation(op, &args, index, &fixture.binding).unwrap(),
            )
        };
        let operations = [
            parse(
                "content.add",
                json!({"items": [{"object": "Document.First"}]}),
                0,
            ),
            parse(
                "childSubsystem.add",
                json!({"items": [{"name": "Orders"}]}),
                1,
            ),
        ];
        let (staged, effects) = plan_form_resource_batch(staged, authority, &operations)
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        assert_eq!(effects.len(), 2);
        let text = staged_text(&staged, "Subsystems/Sales.xml");
        assert!(text.contains("Document.First"), "{text}");
        assert!(text.contains("<Subsystem>Orders</Subsystem>"), "{text}");
        let child = staged_text(&staged, "Subsystems/Sales/Subsystems/Orders.xml");
        assert!(child.contains("<Name>Orders</Name>"), "{child}");
    }

    #[test]
    fn support_rule_set_rewrites_the_object_record_and_refuses_without_support_state() {
        let fixture = ApplySeamFixture::new();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .form_resource_planning_authority(&fixture.binding)
            .unwrap();
        let operation = parse_form_resource_plan_operation(
            "supportRule.set",
            &json!({"at": "main:Document.First", "values": {"rule": "editable"}}),
            0,
            &fixture.binding,
        )
        .unwrap();
        let error = plan_form_resource_batch(
            staged,
            authority,
            &[IndexedPlanOperation::new(0, operation.clone())],
        )
        .unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);

        std::fs::create_dir_all(fixture.source_dir().join("Ext")).unwrap();
        std::fs::write(
            fixture.source_dir().join("Ext/ParentConfigurations.bin"),
            "\u{feff}{6,0,1,dddddddd-dddd-4ddd-8ddd-dddddddddddd,0,eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee,\"1.0\",\"Vendor\",\"VendorConf\",3,1,0,aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa,aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa,0,0,11111111-1111-4111-8111-111111111111,11111111-1111-4111-8111-111111111111}",
        )
        .unwrap();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .form_resource_planning_authority(&fixture.binding)
            .unwrap();
        let (staged, effects) = plan_form_resource_batch(
            staged,
            authority,
            &[IndexedPlanOperation::new(0, operation)],
        )
        .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        assert_eq!(effects.len(), 1);
        let text = staged_text(&staged, "Ext/ParentConfigurations.bin");
        assert!(
            text.contains("1,0,11111111-1111-4111-8111-111111111111"),
            "the object rule flips to editable: {text}"
        );
    }

    #[test]
    fn form_operations_refuse_a_root_target_and_name_the_argument() {
        let fixture = ApplySeamFixture::new();
        let error = parse_form_resource_plan_operation(
            "form.create",
            &json!({"at": "main:Configuration", "values": {"name": "Форма"}}),
            0,
            &fixture.binding,
        )
        .unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
        assert_eq!(error.path(), Some("ops[0].args.at"));
    }
}
