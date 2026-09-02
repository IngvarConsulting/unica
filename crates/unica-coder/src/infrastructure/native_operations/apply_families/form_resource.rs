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
                other => return Err(bad(
                    op_index,
                    "values.rule",
                    format!(
                        "unknown support rule `{other}`; use `locked`, `editable` or `off-support`"
                    ),
                )),
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
    fn form_resource_apply_seam_routes_actor_authorized_batch_to_stable_unsupported() {
        let fixture = ApplySeamFixture::new();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .form_resource_planning_authority(&fixture.binding)
            .unwrap();
        let operation = parse_form_resource_plan_operation(
            "form.create",
            &json!({"at": "main:Configuration"}),
            0,
            &fixture.binding,
        )
        .unwrap();

        let operation = IndexedPlanOperation::new(0, operation);
        let error = plan_form_resource_batch(staged, authority, &[operation]).unwrap_err();

        assert_eq!(error.kind(), ApplyPlanErrorKind::ProviderUnavailable);
        assert_eq!(error.path(), Some("ops[0].op"));
        assert_eq!(
            error.to_string(),
            "hidden v0.13 apply family is not implemented"
        );
    }
}
