//! Existing metadata resource parsers and staging algorithms over the caller's source proposal.
use crate::interface::{
    interface_order_membership, interface_text_direct_item_values, interface_text_do_group_order,
    interface_text_do_hide, interface_text_do_order, interface_text_do_place,
    interface_text_do_show, interface_text_do_subsystem_order, interface_text_group_command_order,
    normalize_interface_command_name,
};
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_metadata::interface::InterfaceEditCounters;
use unica_metadata::support::SupportOperation;
use unica_metadata::support_edit::{SupportCapability, SupportObjectRule};
use unica_source_contract::events::{DomainEvent, DomainEventKind};
use unica_source_contract::proposal::input::{bad, creation_name, required_values};
use unica_source_contract::proposal::{
    hidden_apply_family_unimplemented, ApplyPlanError, ApplyPlanErrorKind, ApplyProposal,
    ApplySourceAuthority, ProvisionalApplyEffect,
};
use unica_source_designer::registration::read_required;

#[derive(Debug, Clone)]
pub enum SubsystemEdit {
    ContentAdd(Vec<String>),
    ContentRemove(Vec<String>),
    ChildAdd(Vec<String>),
    ChildRemove(Vec<String>),
}

/// Что именно правится в командном интерфейсе. Каждый вариант — одна секция
/// файла: писатель не трогает соседние и тем сохраняет то, чего читатель не
/// показывает, — в первую очередь значения видимости по ролям.
#[derive(Debug, Clone)]
pub enum InterfaceEdit {
    Visibility(Vec<(String, bool)>),
    Placement(Vec<(String, Option<String>, Option<String>)>),
    CommandOrder {
        group: String,
        commands: Vec<String>,
    },
    GroupOrder(Vec<String>),
    SubsystemOrder(Vec<String>),
}

#[derive(Debug, Clone)]
pub enum MetadataResourcePlan {
    /// `subsystem.create`: descriptor and registration.
    CreateSubsystem {
        name: String,
    },
    Subsystem {
        address: String,
        relative: PathBuf,
        edit: SubsystemEdit,
    },
    /// Правка `CommandInterface.xml`: видимость, размещение и три порядка.
    CommandInterface {
        address: String,
        relative: PathBuf,
        edit: InterfaceEdit,
    },
    SupportCapability(SupportCapability),
    SupportRule {
        target: MetadataAddress,
        rule: SupportObjectRule,
    },

    Unsupported,
}

/// Разбор операций командного интерфейса.
///
/// Адрес несёт интерфейс, а команда лежит в аргументах: предмет операции —
/// пара «интерфейс и ссылка на команду», а адрес может нести только одно.
/// Приписать факт подсистемы адресу команды, живущей в другом месте, было бы
/// неверно. Порядок команд принадлежит группе, и группа — узел интерфейса:
/// `commandOrder.set` адресует `…Interface.Group.<g>`, имя с точкой
/// экранируется (`CommandGroup\.X`).
fn parse_command_interface_operation(
    section: &str,
    target: &QualifiedAddress,
    args: &Map<String, Value>,
    op_index: usize,
) -> Result<MetadataResourcePlan, ApplyPlanError> {
    let (segments, group) = match target.segments() {
        [interface @ .., group] if group.kind() == NodeKind::Group => match group.name() {
            Some(name) => (interface, Some(name)),
            // Ветвь групп — коллекция, а не группа: порядок пишется в группу.
            None => {
                return Err(bad(
                    op_index,
                    "at",
                    format!("`{section}.set` does not target the group collection; name the group: `Subsystem.Name.Interface.Group.G`"),
                ))
            }
        },
        segments => (segments, None),
    };
    if (section == "commandOrder") != group.is_some() {
        return Err(bad(
            op_index,
            "at",
            if section == "commandOrder" {
                "`commandOrder.set` targets a command group: `Subsystem.Name.Interface.Group.G`"
                    .to_string()
            } else {
                format!("`{section}.set` does not target a command group")
            },
        ));
    }
    let interface = QualifiedAddress {
        line_range: None,
        source_set: target.source_set().to_string(),
        segments: segments.to_vec(),
    };
    let target = &interface;
    let (address, relative) = match segments {
        // Порядок подсистем верхнего уровня живёт в корневом документе.
        [root] if root.kind() == NodeKind::Configuration => {
            if section != "subsystemOrder" {
                return Err(bad(
                    op_index,
                    "at",
                    format!("`{section}.set` targets a subsystem command interface: `Subsystem.Name.Interface`"),
                ));
            }
            (
                target.to_string(),
                PathBuf::from("Ext").join("CommandInterface.xml"),
            )
        }
        // Владелец — вся цепочка подсистем: `Subsystem.A.Subsystem.B.Interface`
        // живёт в `Subsystems/A/Subsystems/B/Ext`, и одноимённая корневая `B`
        // целью не становится.
        [owners @ .., facet]
            if facet.kind() == NodeKind::Interface
                && !owners.is_empty()
                && owners
                    .iter()
                    .all(|owner| owner.kind() == NodeKind::Subsystem) =>
        {
            if section == "subsystemOrder" {
                return Err(bad(
                    op_index,
                    "at",
                    "`subsystemOrder.set` targets the configuration root: `<set>:Configuration`",
                ));
            }
            let mut relative = PathBuf::new();
            for owner in owners {
                let name = owner
                    .name()
                    .ok_or_else(|| bad(op_index, "at", "the subsystem must be named"))?;
                relative.push("Subsystems");
                relative.push(name);
            }
            relative.push("Ext");
            relative.push("CommandInterface.xml");
            (target.to_string(), relative)
        }
        _ => return Err(bad(
            op_index,
            "at",
            "a command-interface operation targets `Subsystem.Name[.Subsystem.Child].Interface`",
        )),
    };
    let edit = match section {
        "commandVisibility" => {
            let items = interface_items(args, op_index)?;
            let mut values = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                let item = item.as_object().ok_or_else(|| {
                    bad(
                        op_index,
                        &format!("items[{index}]"),
                        "each item is an object",
                    )
                })?;
                let command = unica_source_contract::proposal::input::required_string(
                    item,
                    "command",
                    op_index,
                    &format!("items[{index}]"),
                )?;
                let visible = match item.get("visible") {
                    Some(Value::Bool(visible)) => visible,
                    _ => {
                        return Err(bad(
                            op_index,
                            &format!("items[{index}].visible"),
                            "`visible` must be a boolean",
                        ))
                    }
                };
                values.push((command.to_string(), *visible));
            }
            InterfaceEdit::Visibility(values)
        }
        "commandPlacement" => {
            let items = interface_items(args, op_index)?;
            let mut values = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                let item = item.as_object().ok_or_else(|| {
                    bad(
                        op_index,
                        &format!("items[{index}]"),
                        "each item is an object",
                    )
                })?;
                let command = unica_source_contract::proposal::input::required_string(
                    item,
                    "command",
                    op_index,
                    &format!("items[{index}]"),
                )?;
                let group = item
                    .get("group")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let placement = item
                    .get("placement")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if group.is_none() && placement.is_none() {
                    return Err(bad(
                        op_index,
                        &format!("items[{index}]"),
                        "placement needs `group` and/or `placement`",
                    ));
                }
                values.push((command.to_string(), group, placement));
            }
            InterfaceEdit::Placement(values)
        }
        "commandOrder" => {
            let values = required_values(args, op_index, &["commands"])?;
            // Порядок команд принадлежит группе адреса: без неё нечего
            // сравнивать с сохранённым составом.
            InterfaceEdit::CommandOrder {
                group: group
                    .expect("a command order target names its group")
                    .to_string(),
                commands: listed_strings(values, "commands", op_index)?,
            }
        }
        "groupOrder" => {
            let values = required_values(args, op_index, &["groups"])?;
            InterfaceEdit::GroupOrder(listed_strings(values, "groups", op_index)?)
        }
        _ => {
            let values = required_values(args, op_index, &["subsystems"])?;
            InterfaceEdit::SubsystemOrder(listed_strings(values, "subsystems", op_index)?)
        }
    };
    Ok(MetadataResourcePlan::CommandInterface {
        address,
        relative,
        edit,
    })
}

fn interface_items(
    args: &Map<String, Value>,
    op_index: usize,
) -> Result<&Vec<Value>, ApplyPlanError> {
    args.get("items")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
        .ok_or_else(|| bad(op_index, "items", "`items` must be a non-empty array"))
}

/// Полная последовательность, а не добавка: порядок — отношение между всеми
/// элементами, и частичный список некуда вставить.
fn listed_strings(
    values: &Map<String, Value>,
    key: &str,
    op_index: usize,
) -> Result<Vec<String>, ApplyPlanError> {
    let array = values.get(key).and_then(Value::as_array).ok_or_else(|| {
        bad(
            op_index,
            &format!("values.{key}"),
            format!("`{key}` must be an array"),
        )
    })?;
    let mut names = Vec::with_capacity(array.len());
    for (index, item) in array.iter().enumerate() {
        let name = item
            .as_str()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                bad(
                    op_index,
                    &format!("values.{key}[{index}]"),
                    "each entry is a non-empty name",
                )
            })?;
        names.push(name.to_string());
    }
    if names.is_empty() {
        return Err(bad(
            op_index,
            &format!("values.{key}"),
            "the order is a complete sequence and cannot be empty",
        ));
    }
    Ok(names)
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
                Value::Object(object) => unica_source_contract::proposal::input::required_string(
                    object,
                    key,
                    op_index,
                    &format!("items[{index}]"),
                )?
                .to_string(),
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
            unica_source_contract::proposal::input::required_string(
                values, key, op_index, "values",
            )?
            .to_string(),
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

pub fn parse_subsystem_operation(
    operation: &str,
    args: &Map<String, Value>,
    op_index: usize,
    source_set_name: &str,
) -> Result<MetadataResourcePlan, ApplyPlanError> {
    let target =
        unica_source_contract::proposal::input::qualified_target(args, op_index, source_set_name)?;
    let contract = unica_metadata::subsystem_operations::SubsystemOperation::from_name(operation)
        .map(|operation| operation.contract())
        .or_else(|| {
            unica_metadata::interface_operations::InterfaceOperation::from_name(operation)
                .map(|operation| operation.contract())
        })
        .ok_or_else(|| hidden_apply_family_unimplemented(op_index))?;
    // The contract excludes this pair too; the refusal names the key.
    if args.contains_key("items") && args.contains_key("values") {
        return Err(bad(
            op_index,
            "values",
            "pass either `items` or `values`, not both",
        ));
    }
    if operation == "childSubsystem.remove" {
        if let Some(key) = ["items", "values"]
            .into_iter()
            .find(|key| args.contains_key(*key))
        {
            return Err(bad(
                    op_index,
                    key,
                    "childSubsystem.remove takes the child from its address: `Subsystem.Parent.Subsystem.Child`",
                ));
        }
    }
    contract
        .normalize(
            &Value::Object(args.clone()),
            &format!("ops[{op_index}].args"),
        )
        .map_err(|error| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::BadValue,
                format!("{}: {}", error.path, error.message),
            )
            .at_path(error.path)
        })?;
    if let Some(section) = operation.strip_suffix(".set").filter(|section| {
        matches!(
            *section,
            "commandVisibility" | "commandPlacement" | "commandOrder" | "groupOrder"
        ) || (*section == "subsystemOrder")
    }) {
        return parse_command_interface_operation(section, &target, args, op_index);
    }
    if operation == "subsystem.create" {
        let values = required_values(args, op_index, &["name"])?;
        let name = creation_name(
            &target,
            NodeKind::Subsystem,
            values,
            op_index,
            unica_metadata::canonical_creation_identifier,
        )?;
        return Ok(MetadataResourcePlan::CreateSubsystem { name });
    }
    let edit = match operation {
        "content.add" => SubsystemEdit::ContentAdd(listed_names(args, "object", op_index)?),
        "content.remove" => SubsystemEdit::ContentRemove(listed_names(args, "object", op_index)?),
        "childSubsystem.add" => SubsystemEdit::ChildAdd(listed_names(args, "name", op_index)?),
        "childSubsystem.remove" => {
            // The child travels only in the address
            // (`...Subsystem.Parent.Subsystem.Child`, #1353).
            let segments = target.segments();
            let [_, .., child] = segments else {
                return Err(bad(
                    op_index,
                    "at",
                    "childSubsystem.remove addresses the child: `Subsystem.Parent.Subsystem.Child`",
                ));
            };
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
                &[source_set_name],
            )
            .map_err(|error| bad(op_index, "at", error.to_string()))?;
            let (address, relative) = subsystem_descriptor_relative(&parent, op_index)?;
            return Ok(MetadataResourcePlan::Subsystem {
                address,
                relative,
                edit: SubsystemEdit::ChildRemove(vec![child_name]),
            });
        }
        _ => return Ok(MetadataResourcePlan::Unsupported),
    };
    let (address, relative) = subsystem_descriptor_relative(&target, op_index)?;
    Ok(MetadataResourcePlan::Subsystem {
        address,
        relative,
        edit,
    })
}

pub fn parse_support_operation(
    operation: SupportOperation,
    args: &Map<String, Value>,
    op_index: usize,
    source_set_name: &str,
) -> Result<MetadataResourcePlan, ApplyPlanError> {
    let target =
        unica_source_contract::proposal::input::qualified_target(args, op_index, source_set_name)?;
    operation
        .contract()
        .normalize(
            &Value::Object(args.clone()),
            &format!("ops[{op_index}].args"),
        )
        .map_err(|error| {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, error.message).at_path(error.path)
        })?;

    match operation {
        SupportOperation::CapabilitySet => {
            // Возможность изменения — свойство всей конфигурации; вызов на
            // объекте писал бы не в тот узел.
            if !matches!(target.segments(), [root] if root.kind() == NodeKind::Configuration) {
                return Err(bad(
                    op_index,
                    "at",
                    "supportCapability.set targets the configuration root",
                ));
            }
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
            Ok(MetadataResourcePlan::SupportCapability(if enabled {
                SupportCapability::On
            } else {
                SupportCapability::Off
            }))
        }
        SupportOperation::RuleSet => {
            let values = required_values(args, op_index, &["rule"])?;
            let rule = unica_source_contract::proposal::input::required_string(
                values, "rule", op_index, "values",
            )?;
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
            let raw = match target.segments() {
                [owner] if owner.name().is_some() => {
                    format!("{}.{}", owner.kind().as_str(), owner.name().unwrap())
                }
                [owner, form]
                    if owner.name().is_some()
                        && form.kind() == NodeKind::Form
                        && form.name().is_some() =>
                {
                    format!(
                        "{}.{}.Form.{}",
                        owner.kind().as_str(),
                        owner.name().unwrap(),
                        form.name().unwrap()
                    )
                }
                _ => {
                    return Err(bad(
                        op_index,
                        "at",
                        "supportRule.set targets a top-level metadata object or its form",
                    ))
                }
            };
            let target = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &raw)
                .map_err(|error| bad(op_index, "at", error.to_string()))?;
            Ok(MetadataResourcePlan::SupportRule { target, rule })
        }
    }
}

fn subsystem_stub_xml(name: &str, format_version: &str, uuid: &str) -> String {
    let model = unica_metadata::subsystem::SubsystemEditModel {
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
        picture_load_transparent: None,
        content: Vec::new(),
        children: Vec::new(),
    };
    crate::subsystem::emit_subsystem_edit_model(&model)
}

// Вложенный адрес сначала доказывает регистрацию и идентичность описания,
// только затем его UUID выбирает запись поддержки. Все чтения входят
// в барьер исходного состояния подготовленного применения.
fn prove_support_form_owner(
    staged: &mut ApplyProposal<'_>,
    target: &MetadataAddress,
    relative: &std::path::Path,
    descriptor: &[u8],
    authority: &dyn ApplySourceAuthority,
    at_path: &str,
) -> Result<(), ApplyPlanError> {
    use unica_source_designer::owner::{
        prove_already_read_metadata_owner, prove_already_read_source_set_owner,
    };
    let parts: Vec<_> = target.as_str().split('.').collect();
    let [kind, name, "Form", form] = parts.as_slice() else {
        return Ok(());
    };
    let invalid = || {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "form support target does not match its registered metadata descriptor",
        )
        .at_path(at_path)
    };
    let root_path = std::path::Path::new("Configuration.xml");
    let root = read_required(staged, root_path, "configuration descriptor", at_path)?;
    let root = prove_already_read_source_set_owner(root_path, &root, authority.source_kind())
        .map_err(|_| invalid())?;
    if !root.registers(kind, name) {
        return Err(invalid());
    }
    let owner = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &format!("{kind}.{name}"))
        .map_err(|_| invalid())?;
    let owner_path = unica_source_designer::layout::metadata_descriptor_relative(
        &owner,
        authority.source_kind(),
    )
    .map_err(|_| invalid())?;
    let owner_bytes = read_required(staged, &owner_path, "form owner descriptor", at_path)?;
    let owner =
        prove_already_read_metadata_owner(&owner_path, &owner_bytes).map_err(|_| invalid())?;
    let child = prove_already_read_metadata_owner(relative, descriptor).map_err(|_| invalid())?;
    if owner.artifact_kind() != *kind
        || owner.artifact_name() != Some(*name)
        || !owner.registers("Form", form)
        || child.artifact_kind() != "Form"
        || child.artifact_name() != Some(*form)
        || root.version() != Some(authority.expected_format())
        || owner.version() != Some(authority.expected_format())
        || child.version() != Some(authority.expected_format())
    {
        return Err(invalid());
    }
    // Контейнер не является владельцем формы. Его UUID отклоняется, чтобы
    // общий читатель поддержки не выбрал запись другого объекта.
    let text = std::str::from_utf8(descriptor).map_err(|_| invalid())?;
    let document =
        roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).map_err(|_| invalid())?;
    if document.root_element().attribute("uuid").is_some() {
        return Err(invalid());
    }
    let uuid = crate::support::support_root_uuid_from_bytes(descriptor).ok_or_else(invalid)?;
    uuid::Uuid::parse_str(&uuid).map_err(|_| invalid())?;
    Ok(())
}

fn read_support_state(
    staged: &mut ApplyProposal<'_>,
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
        crate::support_codec::decode_parent_configurations(&preimage).map_err(|message| {
            ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message)
                .at_path(at_path.to_string())
        })?;
    if crate::support::parse_support_header(&text).is_none() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "Ext/ParentConfigurations.bin has an unknown format",
        )
        .at_path(at_path.to_string()));
    }
    Ok((preimage, text))
}

fn stage_support_state(
    staged: &mut ApplyProposal<'_>,
    relative: &std::path::Path,
    preimage: &[u8],
    updated: &str,
    op_index: usize,
    at_path: &str,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let postimage = crate::support_codec::parent_configurations_bytes(updated);
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

pub fn stage_metadata_resource_plan(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    plan: &MetadataResourcePlan,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    match plan {
        MetadataResourcePlan::CreateSubsystem { name } => {
            let name_path = format!("ops[{op_index}].args.values.name");
            let descriptor = subsystem_stub_xml(
                name,
                authority.expected_format(),
                &unica_source_designer::registration::seeded_uuid(&format!(
                    "{}\0Subsystem.{name}",
                    authority.source_set_name()
                )),
            );
            let mut touched = Vec::new();
            let descriptor_relative = PathBuf::from("Subsystems").join(format!("{name}.xml"));
            unica_source_designer::registration::stage_new_file(
                staged,
                &descriptor_relative,
                descriptor.as_bytes(),
                &name_path,
            )?;
            touched.push(descriptor_relative);
            touched.push(unica_source_designer::registration::register_child(
                staged,
                "Subsystem",
                name,
                op_index,
            )?);
            provisional.push(ProvisionalApplyEffect::spanning(
                touched,
                DomainEvent::new(
                    DomainEventKind::SubsystemChanged,
                    format!("Subsystem.{name}"),
                ),
                op_index,
            ));
        }

        MetadataResourcePlan::CommandInterface {
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
                        "the command interface document was not found",
                    )
                    .at_path(at_path.clone())
                })?;
            let mut text = String::from_utf8(preimage.clone()).map_err(|_| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidSource,
                    "the command interface document is not UTF-8",
                )
                .at_path(at_path.clone())
            })?;
            let mut counters = InterfaceEditCounters::default();
            let mut log = String::new();
            let fail = |message: String| {
                ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message)
                    .at_path(at_path.clone())
            };
            let order_refusal = |message: String, key: &str| {
                ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message)
                    .at_path(format!("ops[{op_index}].args.values.{key}"))
            };
            match edit {
                InterfaceEdit::Visibility(values) => {
                    // Правится только `Common`; значения по ролям —
                    // соседние элементы того же блока — остаются как были.
                    let (shown, hidden): (Vec<_>, Vec<_>) =
                        values.iter().partition(|(_, visible)| *visible);
                    let names = |group: Vec<&(String, bool)>| {
                        group
                            .into_iter()
                            .map(|(command, _)| command.clone())
                            .collect::<Vec<_>>()
                    };
                    let hidden = names(hidden);
                    if !hidden.is_empty() {
                        interface_text_do_hide(&mut text, hidden, &mut counters, &mut log)
                            .map_err(fail)?;
                    }
                    let shown = names(shown);
                    if !shown.is_empty() {
                        interface_text_do_show(&mut text, shown, &mut counters, &mut log)
                            .map_err(fail)?;
                    }
                }
                InterfaceEdit::Placement(values) => {
                    for (command, group, placement) in values {
                        let mut request = Map::new();
                        request.insert("command".to_string(), json!(command));
                        if let Some(group) = group {
                            request.insert("group".to_string(), json!(group));
                        }
                        if let Some(placement) = placement {
                            request.insert("placement".to_string(), json!(placement));
                        }
                        interface_text_do_place(
                            &mut text,
                            &Value::Object(request),
                            &mut counters,
                            &mut log,
                        )
                        .map_err(fail)?;
                    }
                }
                InterfaceEdit::CommandOrder { group, commands } => {
                    // Группа — узел адреса, а `*.set` правит только
                    // существующий узел: группа объявлена в `GroupsOrder`,
                    // как её видит чтение. Порядок не создаёт группу.
                    if !interface_text_direct_item_values(&text, "GroupsOrder", "Group")
                        .iter()
                        .any(|declared| declared == group)
                    {
                        return Err(ApplyPlanError::new(
                                ApplyPlanErrorKind::NotFound,
                                format!(
                                    "the command interface declares no group `{group}`; read its groups at `…Interface.Group`"
                                ),
                            )
                            .at_path(at_path.clone()));
                    }
                    let commands = commands
                        .iter()
                        .map(|command| normalize_interface_command_name(command, &mut log))
                        .collect::<Vec<_>>();
                    interface_order_membership(
                        &interface_text_group_command_order(&text, group),
                        &commands,
                        &format!("commands of group `{group}`"),
                    )
                    .map_err(|message| order_refusal(message, "commands"))?;
                    interface_text_do_order(
                        &mut text,
                        &json!({"group": group, "commands": commands}),
                        &mut counters,
                        &mut log,
                    )
                    .map_err(fail)?;
                }
                InterfaceEdit::GroupOrder(groups) => {
                    interface_order_membership(
                        &interface_text_direct_item_values(&text, "GroupsOrder", "Group"),
                        groups,
                        "groups",
                    )
                    .map_err(|message| order_refusal(message, "groups"))?;
                    interface_text_do_group_order(
                        &mut text,
                        &json!(groups),
                        &mut counters,
                        &mut log,
                    )
                    .map_err(fail)?;
                }
                InterfaceEdit::SubsystemOrder(subsystems) => {
                    interface_order_membership(
                        &interface_text_direct_item_values(&text, "SubsystemsOrder", "Subsystem"),
                        subsystems,
                        "subsystems",
                    )
                    .map_err(|message| order_refusal(message, "subsystems"))?;
                    interface_text_do_subsystem_order(
                        &mut text,
                        &json!(subsystems),
                        &mut counters,
                        &mut log,
                    )
                    .map_err(fail)?;
                }
            }
            let postimage = text.into_bytes();
            if postimage != preimage {
                staged
                    .replace(relative, &preimage, postimage)
                    .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
                provisional.push(ProvisionalApplyEffect::spanning(
                    vec![relative.clone()],
                    DomainEvent::new(DomainEventKind::SubsystemChanged, address.clone()),
                    op_index,
                ));
            }
        }
        MetadataResourcePlan::Subsystem {
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
            let mut model = crate::subsystem::parse_subsystem_edit_model(
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
                        if !unica_metadata::identifier::is_1c_identifier(name) {
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
                                &unica_source_designer::registration::seeded_uuid(&format!(
                                    "{}\0{address}.Subsystem.{name}",
                                    authority.source_set_name()
                                )),
                            );
                            unica_source_designer::registration::stage_new_file(
                                staged,
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
                            staged
                                .remove(&child_relative, &child_preimage)
                                .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
                            touched.push(child_relative);
                        }
                    }
                }
            }
            let original = staged
                .original_image(relative)
                .map(std::str::from_utf8)
                .transpose()
                .map_err(|_| {
                    ApplyPlanError::new(
                        ApplyPlanErrorKind::InvalidSource,
                        "the original subsystem descriptor is not UTF-8",
                    )
                    .at_path(at_path.clone())
                })?;
            let postimage = crate::subsystem::patch_subsystem_lists(&text, &model, original)
                .map_err(|message| {
                    ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message)
                        .at_path(at_path.clone())
                })?
                .into_bytes();
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

        MetadataResourcePlan::SupportCapability(capability) => {
            let at_path = format!("ops[{op_index}].args.at");
            let relative = PathBuf::from("Ext/ParentConfigurations.bin");
            let (preimage, text) = read_support_state(staged, &relative, &at_path)?;
            let (global_flag, _) = crate::support::parse_support_header(&text)
                .expect("read_support_state proved the header");
            // Already in the requested state: nothing to switch, and the
            // object rules must not be reset as a side effect.
            if (global_flag == 0) == capability.enabled() {
                return Ok(());
            }
            let root = authority.source_root();
            let (updated, _) = crate::support_codec::plan_capability_data(
                &root.join(&relative),
                &text,
                *capability,
            )
            .map_err(|message| {
                ApplyPlanError::new(ApplyPlanErrorKind::InvalidState, message)
                    .at_path(at_path.clone())
            })?;
            stage_support_state(
                staged,
                &relative,
                &preimage,
                &updated,
                op_index,
                &at_path,
                provisional,
            )?;
        }
        MetadataResourcePlan::SupportRule { target, rule } => {
            let at_path = format!("ops[{op_index}].args.at");
            let relative = PathBuf::from("Ext/ParentConfigurations.bin");
            let (preimage, text) = read_support_state(staged, &relative, &at_path)?;
            let descriptor_relative = unica_source_designer::layout::metadata_descriptor_relative(
                target,
                authority.source_kind(),
            )
            .map_err(|message| {
                ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(at_path.clone())
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
            prove_support_form_owner(
                staged,
                target,
                &descriptor_relative,
                &descriptor,
                authority,
                &at_path,
            )?;
            let uuid =
                crate::support::support_root_uuid_from_bytes(&descriptor).ok_or_else(|| {
                    ApplyPlanError::new(
                        ApplyPlanErrorKind::InvalidSource,
                        "the metadata descriptor carries no object uuid",
                    )
                    .at_path(at_path.clone())
                })?;
            let root = authority.source_root();
            let (updated, _) = crate::support_codec::plan_object_rule_data(
                &root.join(&relative),
                &text,
                &uuid,
                *rule,
            )
            .map_err(|message| {
                ApplyPlanError::new(ApplyPlanErrorKind::InvalidState, message)
                    .at_path(at_path.clone())
            })?;
            stage_support_state(
                staged,
                &relative,
                &preimage,
                &updated,
                op_index,
                &at_path,
                provisional,
            )?;
        }
        MetadataResourcePlan::Unsupported => {
            return Err(hidden_apply_family_unimplemented(op_index))
        }
    }
    Ok(())
}

#[cfg(feature = "test-support")]
pub mod test_support {
    /// Existing canonical fixture emitter; delegates to the same production algorithm.
    pub fn subsystem_stub_xml(name: &str, format_version: &str, uuid: &str) -> String {
        super::subsystem_stub_xml(name, format_version, uuid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_remove_retired_shapes_keep_address_guidance() {
        for (retired, key) in [
            (json!({"items":[]}), "items"),
            (json!({"values":{}}), "values"),
            (json!({"items":null}), "items"),
            (json!({"values":null}), "values"),
            (json!({"items":[{}]}), "items"),
            (json!({"values":{"name":null}}), "values"),
        ] {
            let mut args = retired.as_object().unwrap().clone();
            args.insert("at".into(), json!("main:Subsystem.Parent.Subsystem.Child"));
            let error =
                parse_subsystem_operation("childSubsystem.remove", &args, 2, "main").unwrap_err();
            assert_eq!(error.path(), Some(format!("ops[2].args.{key}").as_str()));
            assert!(
                error
                    .to_string()
                    .contains("Subsystem.Parent.Subsystem.Child"),
                "{retired}: {}",
                error
            );
        }
        let args = json!({"at":"main:Subsystem.Parent.Subsystem.Child","items":[],"values":{}});
        let error = parse_subsystem_operation(
            "childSubsystem.remove",
            args.as_object().unwrap(),
            2,
            "main",
        )
        .unwrap_err();
        assert_eq!(error.path(), Some("ops[2].args.values"));
        assert!(error
            .to_string()
            .contains("pass either `items` or `values`, not both"));
    }
}
