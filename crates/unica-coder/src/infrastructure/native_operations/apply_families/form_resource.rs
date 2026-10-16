use super::metadata::{owner_registration_image, qualified_target};
use super::validate_platform_xml_binding;
use crate::domain::address::{member_address, NodeKind, QualifiedAddress};
use crate::domain::apply::{FormItemRole, OperationDescriptor, OperationFamily};
use crate::domain::events::{DomainEvent, DomainEventKind};
use crate::domain::role_operation::RoleOperation;
use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use crate::domain::support_operation::SupportOperation;
use crate::infrastructure::logical_event_source::attached_resource_relative;
use crate::infrastructure::native_operations::apply::{
    empty_apply_family_batch, hidden_apply_family_unimplemented, ApplyPlanError,
    ApplyPlanErrorKind, ApplyStagedState,
};
use crate::infrastructure::native_operations::apply_families::request::{
    IndexedPlanOperation, ProvisionalApplyEffect,
};
use crate::infrastructure::native_operations::form::FormInfoElement;
use crate::infrastructure::native_operations::interface::{
    interface_order_membership, interface_text_direct_item_values, interface_text_do_group_order,
    interface_text_do_hide, interface_text_do_order, interface_text_do_place,
    interface_text_do_show, interface_text_do_subsystem_order, interface_text_group_command_order,
    normalize_interface_command_name, InterfaceEditCounters,
};
use crate::infrastructure::native_operations::support::{SupportCapability, SupportObjectRule};
use crate::infrastructure::workspace_actor::{FormResourceApplyAuthority, ProviderRootBinding};
use serde_json::{json, Map, Value};
use std::path::PathBuf;

#[derive(Debug, Clone)]
enum FormEditStep {
    Definition(Value),
    Move {
        request: super::super::form_move::FormItemMove,
        location: String,
    },
}

const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";

#[derive(Debug, Clone)]
enum SubsystemEdit {
    ContentAdd(Vec<String>),
    ContentRemove(Vec<String>),
    ChildAdd(Vec<String>),
    ChildRemove(Vec<String>),
}

/// Что именно правится в командном интерфейсе. Каждый вариант — одна секция
/// файла: писатель не трогает соседние и тем сохраняет то, чего читатель не
/// показывает, — в первую очередь значения видимости по ролям.
#[derive(Debug, Clone)]
enum InterfaceEdit {
    Visibility(Vec<(String, bool)>),
    Placement(Vec<(String, Option<String>, Option<String>)>),
    CommandOrder {
        group: String,
        commands: Vec<String>,
    },
    GroupOrder(Vec<String>),
    SubsystemOrder(Vec<String>),
}

/// Новая запись прав роли: объект и его начальные права.
#[derive(Debug, Clone)]
struct RightEntry {
    object: String,
    rights: Vec<(String, bool)>,
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
    /// `right.set` на существующей записи прав `Role.R.Right.<Вид_Имя>`.
    RightSet {
        role: MetadataAddress,
        /// Квалифицированный адрес роли для текста отказа.
        role_at: String,
        object: String,
        right: String,
        value: Option<bool>,
        rls: Option<String>,
    },
    /// `right.add` на роли: новые записи прав с начальным набором прав.
    RightAdd {
        role: MetadataAddress,
        role_at: String,
        entries: Vec<RightEntry>,
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
    /// `form.add` on the owner: managed forms from the platform shapes.
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
        subject: FormEditSubject,
        definitions: Vec<FormEditStep>,
    },
    Unsupported,
}

/// Узел формы, который задаёт адрес правки (#1355): цепочка элементов от
/// формы, пустая у самой формы. Узел проверяется по прочитанной форме до
/// записи: отсутствующий даёт `not_found`, а не правку другого узла.
#[derive(Debug, Clone)]
enum FormEditSubject {
    /// Форма или элемент, на который пишется событие или реквизит.
    Node(Vec<String>),
    /// Форма или элемент-контейнер, куда добавляются элементы.
    Container(Vec<String>),
    /// Удаляемый элемент.
    Removed(Vec<String>),
}

impl FormEditSubject {
    fn items(&self) -> &[String] {
        match self {
            Self::Node(items) | Self::Container(items) | Self::Removed(items) => items,
        }
    }
}

/// Элемент формы по цепочке имён от корня, как её адресует `view`.
pub(super) fn form_item_at<'a>(
    elements: &'a [FormInfoElement],
    chain: &[String],
) -> Option<&'a FormInfoElement> {
    let (first, rest) = chain.split_first()?;
    let item = elements.iter().find(|element| &element.name == first)?;
    if rest.is_empty() {
        Some(item)
    } else {
        form_item_at(&item.children, rest)
    }
}

/// Путь элемента с именем `name` от корня формы: один, ни одного или
/// несколько элементов с этим именем.
pub(super) fn form_item_path(
    elements: &[FormInfoElement],
    name: &str,
) -> crate::domain::apply::NamedLookup {
    fn collect(
        elements: &[FormInfoElement],
        name: &str,
        prefix: &mut Vec<String>,
        found: &mut Vec<Vec<String>>,
    ) {
        for element in elements {
            prefix.push(element.name.clone());
            if element.name == name {
                found.push(prefix.clone());
            }
            collect(&element.children, name, prefix, found);
            prefix.pop();
        }
    }
    let mut found = Vec::new();
    collect(elements, name, &mut Vec::new(), &mut found);
    match found.len() {
        0 => crate::domain::apply::NamedLookup::Missing,
        1 => crate::domain::apply::NamedLookup::Found(found.remove(0)),
        _ => crate::domain::apply::NamedLookup::Ambiguous(found),
    }
}

/// Дерево элементов формы, как его читает `view`.
pub(super) fn form_items_of(text: &str) -> Result<Vec<FormInfoElement>, String> {
    crate::infrastructure::native_operations::form::parse_form_event_evidence_xml(text)
        .map(|evidence| evidence.elements)
}

/// Узел правки есть в форме и подходит операции; сосед `after` нового
/// элемента лежит в том же контейнере.
fn check_form_edit_subject(
    xml_text: &str,
    subject: &FormEditSubject,
    definitions: &[FormEditStep],
    op_index: usize,
) -> Result<(), ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    let elements = form_items_of(xml_text).map_err(|error| {
        ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, error).at_path(at_path.clone())
    })?;
    let chain = subject.items();
    let siblings = match chain {
        [] => &elements[..],
        chain => {
            let item = form_item_at(&elements, chain).ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::NotFound,
                    format!(
                        "form item `Item.{}` was not found; read the form items to address an existing one",
                        chain.join(".Item.")
                    ),
                )
                .at_path(at_path.clone())
            })?;
            let role = FormItemRole::from_tag(&item.tag);
            match (subject, role) {
                (FormEditSubject::Container(_), FormItemRole::Leaf) => {
                    return Err(bad(
                        op_index,
                        "at",
                        format!(
                            "form item `{}` {} does not hold elements; item.add targets the form or a container item",
                            item.name, item.tag
                        ),
                    ))
                }
                (FormEditSubject::Removed(_), FormItemRole::AutoCommandBar) => {
                    return Err(bad(
                        op_index,
                        "at",
                        format!(
                            "`{}` is the automatic command bar of its owner and is removed only with it",
                            item.name
                        ),
                    ))
                }
                _ => {}
            }
            &item.children[..]
        }
    };
    if matches!(subject, FormEditSubject::Container(_)) {
        // Элементы, добавленные раньше в том же `items`, к моменту
        // следующего уже стоят в контейнере и тоже служат соседями.
        let mut added: Vec<&str> = Vec::new();
        for (index, step) in definitions.iter().enumerate() {
            let FormEditStep::Definition(definition) = step else {
                continue;
            };
            if let Some(after) = definition.get("after").and_then(Value::as_str) {
                if !siblings.iter().any(|sibling| sibling.name == after) && !added.contains(&after)
                {
                    return Err(bad(
                        op_index,
                        &format!("items[{index}].after"),
                        format!(
                            "`after` names a sibling inside the addressed container; `{after}` is not there"
                        ),
                    ));
                }
            }
            added.extend(
                definition
                    .get("elements")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|element| element.get("name").and_then(Value::as_str)),
            );
        }
    }
    Ok(())
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
) -> Result<FormResourcePlanKind, ApplyPlanError> {
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
                let command =
                    required_string(item, "command", op_index, &format!("items[{index}]"))?;
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
                let command =
                    required_string(item, "command", op_index, &format!("items[{index}]"))?;
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
    Ok(FormResourcePlanKind::CommandInterface {
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
    // Узел создаёт операция родителя: `at` всегда называет существующий
    // узел, а имя нового — аргумент.
    if !matches!(target.segments(), [root] if root.kind() == NodeKind::Configuration) {
        return Err(bad(
            op_index,
            "at",
            format!(
                "`{}` creation targets the configuration root; name the new object in `values.name`",
                kind.as_str()
            ),
        ));
    }
    Ok(name)
}

/// Разбор без семейства, выбранного вызывающим: общее имя (`attribute.add`
/// формы и объекта, #1465) выбирает запись реестра по адресу вызова.
#[cfg(test)]
pub(crate) fn parse_form_resource_plan_operation(
    operation: &str,
    args: &Value,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<FormResourcePlanOperation, ApplyPlanError> {
    let at = args
        .get("at")
        .and_then(Value::as_str)
        .and_then(|raw| QualifiedAddress::resolve_input(raw, &[binding.source_set_name()]).ok());
    let descriptor = super::resolved_descriptor(operation, at.as_ref(), binding);
    parse_form_resource_plan_operation_in(descriptor, operation, args, op_index, binding)
}

/// Разбор операции семейством, которое выбрал разбор `apply` по записи
/// реестра для адреса вызова.
pub(crate) fn parse_form_resource_plan_operation_in(
    descriptor: Option<OperationDescriptor>,
    operation: &str,
    args: &Value,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<FormResourcePlanOperation, ApplyPlanError> {
    validate_platform_xml_binding(binding, operation, op_index)?;
    let object = args.as_object().ok_or_else(|| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "operation args must be an object",
        )
        .at_path(format!("ops[{op_index}].args"))
    })?;
    let kind = match descriptor.map(OperationDescriptor::family) {
        Some(OperationFamily::Role) => {
            match descriptor.and_then(OperationDescriptor::role_operation) {
                Some(operation) => parse_role_operation(operation, object, op_index, binding)?,
                None => return Err(hidden_apply_family_unimplemented(op_index)),
            }
        }
        Some(OperationFamily::Subsystem) => {
            parse_subsystem_operation(operation, object, op_index, binding)?
        }
        Some(OperationFamily::Interface) => {
            parse_subsystem_operation(operation, object, op_index, binding)?
        }
        Some(OperationFamily::Support) => {
            match descriptor.and_then(OperationDescriptor::support_operation) {
                Some(operation) => parse_support_operation(operation, object, op_index, binding)?,
                None => return Err(hidden_apply_family_unimplemented(op_index)),
            }
        }
        Some(OperationFamily::Form) => parse_form_operation(operation, object, op_index, binding)?,
        _ => FormResourcePlanKind::Unsupported,
    };
    Ok(FormResourcePlanOperation {
        operation: operation.to_string(),
        kind,
    })
}

/// Вид владельца записи прав: вид метаданных или корень конфигурации.
/// Имя записи `<Вид>_<Имя>` делится по первому `_`, и вид без `_` отличает
/// каноническое имя от короткого псевдонима чтения.
fn role_right_owner_kind(kind: &str) -> bool {
    kind == NodeKind::Configuration.as_str()
        || crate::infrastructure::metadata_kinds::metadata_kind(kind).is_some()
}

/// Роль адреса: `Role.R` для `right.add`, `Role.R.Right.<Вид_Имя>` для
/// `right.set`.
fn role_of(
    target: &QualifiedAddress,
    entry: bool,
    op_index: usize,
) -> Result<MetadataAddress, ApplyPlanError> {
    let shape = if entry {
        "right.set addresses a rights entry: `Role.Name.Right.<Kind>_<Name>`"
    } else {
        "right.add targets one role: `Role.Name`"
    };
    let role_name = match (target.segments(), entry) {
        ([role], false) if role.kind() == NodeKind::Role => role.name(),
        ([role, right], true)
            if role.kind() == NodeKind::Role && right.kind() == NodeKind::Right =>
        {
            role.name()
        }
        _ => None,
    }
    .ok_or_else(|| bad(op_index, "at", shape))?;
    MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!("Role.{role_name}"),
    )
    .map_err(|error| bad(op_index, "at", error.to_string()))
}

fn parse_role_operation(
    operation: RoleOperation,
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<FormResourcePlanKind, ApplyPlanError> {
    let target = qualified_target(args, op_index, binding)?;
    // Опубликованный контракт и разбор планировщиком принимают одну форму
    // аргументов (#1354): после адреса схема проверяется до ручного разбора.
    // Нормализованный JSON не подменяет исходные аргументы планировщика.
    operation
        .contract()
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
    match operation {
        RoleOperation::Create => {
            let values = required_values(args, op_index, &["name"])?;
            let name = creation_name(&target, NodeKind::Role, values, op_index)?;
            Ok(FormResourcePlanKind::CreateRole { name })
        }
        RoleOperation::RightAdd => {
            let role = role_of(&target, false, op_index)?;
            let items = args
                .get("items")
                .and_then(Value::as_array)
                .filter(|items| !items.is_empty())
                .ok_or_else(|| bad(op_index, "items", "`items` must list at least one object"))?;
            let mut entries = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                let location = format!("items[{index}]");
                let item = item
                    .as_object()
                    .ok_or_else(|| bad(op_index, &location, "each item must be an object"))?;
                if let Some(field) = item
                    .keys()
                    .find(|field| !matches!(field.as_str(), "object" | "rights"))
                {
                    return Err(bad(
                        op_index,
                        &format!("{location}.{field}"),
                        format!("an item accepts only `object` and `rights`, not `{field}`"),
                    ));
                }
                let object = required_string(item, "object", op_index, &location)?.to_string();
                // Запись адресуется `Right.<Вид>_<Имя>`; у вложенного объекта
                // (`Kind.X.Command.Y`) остальные точки имени экранируются.
                match object.split_once('.') {
                    Some((kind, name))
                        if role_right_owner_kind(kind)
                            && !name.is_empty()
                            && name.split('.').all(|part| !part.is_empty()) => {}
                    _ => {
                        return Err(bad(
                            op_index,
                            &format!("{location}.object"),
                            "`object` names a metadata object as `Kind.Name` or a nested one as `Kind.Name.Command.C`",
                        ))
                    }
                }
                // Вложенный объект роли — закрытый набор раскладок, которые
                // платформа пишет в Rights.xml: команда, реквизит, стандартный
                // реквизит, табличная часть и её реквизиты, измерение, ресурс,
                // операция веб-сервиса, метод шаблона URL, канал сервиса
                // интеграции, реквизит адресации задачи, вложенная подсистема.
                // Опечатка вида (`Comand`) или вложенный объект у корня
                // конфигурации иначе ушли бы в Rights.xml записью, которой
                // у платформы нет.
                let nested = object.split('.').count() > 2;
                if nested && !crate::domain::role::nested_right_shape_is_modelled(&object) {
                    return Err(bad(
                        op_index,
                        &format!("{location}.object"),
                        format!(
                            "`{object}` is not a nested object a role lists: use Kind.Name.Command|Attribute|StandardAttribute|TabularSection|Dimension|Resource|Operation|AddressingAttribute|IntegrationServiceChannel.X, Kind.Name.TabularSection.T.Attribute|StandardAttribute.X, HTTPService.S.URLTemplate.T.Method.M or Subsystem.S.Subsystem.X"
                        ),
                    ));
                }
                // Созданную запись `right.set` правит по адресу
                // `Right.<Вид>_<Имя>`; адрес, который грамматика не разбирает,
                // оставил бы запись без адреса.
                let entry_name = object.replacen('.', "_", 1);
                if QualifiedAddress::parse(&member_address(&format!("{target}.Right"), &entry_name))
                    .ok()
                    .and_then(|address| {
                        address
                            .segments()
                            .last()
                            .and_then(|s| s.name().map(str::to_string))
                    })
                    .as_deref()
                    != Some(entry_name.as_str())
                {
                    return Err(bad(
                        op_index,
                        &format!("{location}.object"),
                        "the object has no addressable `Right.<Kind>_<Name>` entry",
                    ));
                }
                if entries
                    .iter()
                    .any(|entry: &RightEntry| entry.object == object)
                {
                    return Err(bad(
                        op_index,
                        &format!("{location}.object"),
                        format!("`{object}` is listed twice"),
                    ));
                }
                let rights = item
                    .get("rights")
                    .and_then(Value::as_object)
                    .filter(|rights| !rights.is_empty())
                    .ok_or_else(|| {
                        bad(
                            op_index,
                            &format!("{location}.rights"),
                            "`rights` maps at least one right name to true/false",
                        )
                    })?
                    .iter()
                    .map(|(right, value)| match value {
                        Value::Bool(value) if !right.trim().is_empty() => {
                            Ok((right.trim().to_string(), *value))
                        }
                        _ => Err(bad(
                            op_index,
                            &format!("{location}.rights.{right}"),
                            "a right takes true or false",
                        )),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if nested {
                    // Право вложенного объекта — из его закрытого набора
                    // (View/Edit, Use у операций и каналов), а не права объекта.
                    if let Some((right, message)) = rights.iter().find_map(|(right, _)| {
                        crate::domain::role::validate_nested_right(&object, right)
                            .err()
                            .map(|message| (right, message))
                    }) {
                        return Err(bad(
                            op_index,
                            &format!("{location}.rights.{right}"),
                            message,
                        ));
                    }
                }
                entries.push(RightEntry { object, rights });
            }
            Ok(FormResourcePlanKind::RightAdd {
                role,
                role_at: target.to_string(),
                entries,
            })
        }
        RoleOperation::RightSet => {
            let role = role_of(&target, true, op_index)?;
            // Объект записи задаёт адрес `Role.R.Right.<Вид_Имя>` (#1354).
            let object = target
                .segments()
                .get(1)
                .and_then(|segment| segment.name())
                .and_then(|name| name.split_once('_'))
                .filter(|(kind, name)| role_right_owner_kind(kind) && !name.is_empty())
                .map(|(kind, name)| format!("{kind}.{name}"))
                .ok_or_else(|| {
                    bad(
                        op_index,
                        "at",
                        "right.set addresses a rights entry by its canonical name: `Role.Name.Right.<Kind>_<Name>`",
                    )
                })?;
            let values = required_values(args, op_index, &["right", "value", "rls"])?;
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
            Ok(FormResourcePlanKind::RightSet {
                role_at: format!("{}:{}", target.source_set(), role.as_str()),
                role,
                object,
                right,
                value,
                rls,
            })
        }
    }
}

fn parse_subsystem_operation(
    operation: &str,
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<FormResourcePlanKind, ApplyPlanError> {
    let target = qualified_target(args, op_index, binding)?;
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
        let name = creation_name(&target, NodeKind::Subsystem, values, op_index)?;
        return Ok(FormResourcePlanKind::CreateSubsystem { name });
    }
    let edit = match operation {
        "content.add" => SubsystemEdit::ContentAdd(listed_names(args, "object", op_index)?),
        "content.remove" => SubsystemEdit::ContentRemove(listed_names(args, "object", op_index)?),
        "childSubsystem.add" => SubsystemEdit::ChildAdd(listed_names(args, "name", op_index)?),
        "childSubsystem.remove" => {
            // The child travels only in the address
            // (`...Subsystem.Parent.Subsystem.Child`, #1353).
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
    operation: SupportOperation,
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<FormResourcePlanKind, ApplyPlanError> {
    let target = qualified_target(args, op_index, binding)?;
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
            Ok(FormResourcePlanKind::SupportCapability(if enabled {
                SupportCapability::On
            } else {
                SupportCapability::Off
            }))
        }
        SupportOperation::RuleSet => {
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
            Ok(FormResourcePlanKind::SupportRule { target, rule })
        }
    }
}

/// Узел формы по адресу `Owner.Name[.Form.F[.Item.A[.Item.B…]][.Event.E]]`.
struct FormTarget {
    owner: MetadataAddress,
    form: Option<String>,
    /// Цепочка элементов от формы: `Item.A.Item.B` даёт `[A, B]`.
    items: Vec<String>,
    /// Событие формы или последнего элемента цепочки.
    event: Option<String>,
}

const FORM_ADDRESS_SHAPE: &str =
    "form operations address `Owner.Name`, `Owner.Name.Form.F`, `Owner.Name.Form.F.Item.A[.Item.B…]` or `….Event.E` of the form or its item";

fn form_target(target: &QualifiedAddress, op_index: usize) -> Result<FormTarget, ApplyPlanError> {
    let segments = target.segments();
    let [owner, rest @ ..] = segments else {
        return Err(bad(op_index, "at", FORM_ADDRESS_SHAPE));
    };
    if owner.kind() == NodeKind::Configuration || owner.kind() == NodeKind::Form {
        return Err(bad(op_index, "at", FORM_ADDRESS_SHAPE));
    }
    let owner_name = owner
        .name()
        .ok_or_else(|| bad(op_index, "at", "the form owner must be named"))?;
    let address = MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!("{}.{owner_name}", owner.kind().as_str()),
    )
    .map_err(|error| bad(op_index, "at", error.to_string()))?;
    let named = |segment: &crate::domain::address::AddressSegment, what: &str| {
        segment
            .name()
            .map(str::to_string)
            .ok_or_else(|| bad(op_index, "at", format!("the {what} must be named")))
    };
    let (form, items, event) = match rest {
        [] => (None, Vec::new(), None),
        [form, tail @ ..] if form.kind() == NodeKind::Form => {
            let (items, event) = match tail {
                [items @ .., last] if last.kind() == NodeKind::Event => {
                    (items, Some(named(last, "form event")?))
                }
                items => (items, None),
            };
            let items = items
                .iter()
                .map(|item| {
                    if item.kind() == NodeKind::Item {
                        named(item, "form item")
                    } else {
                        Err(bad(op_index, "at", FORM_ADDRESS_SHAPE))
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            (Some(named(form, "form")?), items, event)
        }
        _ => return Err(bad(op_index, "at", FORM_ADDRESS_SHAPE)),
    };
    Ok(FormTarget {
        owner: address,
        form,
        items,
        event,
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
/// The item has passed the closed `item.add` contract: its keys are the
/// ones the writer of its kind writes.
fn legacy_element_definition(
    item: &Map<String, Value>,
    op_index: usize,
    location: &str,
) -> Result<(Value, Option<String>), ApplyPlanError> {
    let name = required_string(item, "name", op_index, location)?.to_string();
    let kind = item
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("InputField");
    let mut definition = Map::new();
    definition.insert("name".to_string(), Value::String(name.clone()));
    let orientation = || {
        Value::String(
            item.get("orientation")
                .and_then(Value::as_str)
                .unwrap_or("vertical")
                .to_string(),
        )
    };
    match kind {
        "InputField" | "Input" | "Field" => {}
        "HTMLDocumentField" => {
            definition.insert("html".to_string(), Value::String(name.clone()));
        }
        "TextDocumentField" => {
            definition.insert("textDoc".to_string(), Value::String(name.clone()));
        }
        "Group" | "UsualGroup" => {
            definition.insert("group".to_string(), orientation());
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
            // Ориентацию страницы пишет её `group`; прежде `orientation`
            // у страницы молча пропадал (#1447).
            if item.contains_key("orientation") {
                definition.insert("group".to_string(), orientation());
            }
        }
        "CommandBar" => {
            definition.insert("cmdBar".to_string(), Value::String(name.clone()));
        }
        other => {
            return Err(bad(
                op_index,
                &format!("{location}.type"),
                format!(
                    "unknown form element type `{other}`; use InputField, Group, Table, Button, Label, LabelField, CheckBox, Pages, Page, CommandBar, HTMLDocumentField or TextDocumentField"
                ),
            ))
        }
    }
    let mut after = None;
    for (key, value) in item {
        match key.as_str() {
            "name" | "type" | "orientation" => {}
            "after" => after = value.as_str().map(str::to_string),
            _ => {
                definition.insert(key.clone(), value.clone());
            }
        }
    }
    Ok((Value::Object(definition), after))
}

/// Ключи, которые `item.add` снял по Р2 (#1447), и внутренние различители
/// вида: отказ с продолжением до проверки контракта, чтобы агент получил
/// следующий вызов, а не общий «неизвестный аргумент».
fn refuse_nested_element_content(
    at: &str,
    item: &Map<String, Value>,
    op_index: usize,
    location: &str,
) -> Result<(), ApplyPlanError> {
    use crate::domain::operation_contract::{FORM_ELEMENT_EVENT_KEYS, FORM_ELEMENT_NESTED_KEYS};
    let name = item
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("<Element>");
    let container = format!("{at}.Item.{}", crate::domain::address::escape_name(name));
    let contract = serde_json::json!({
        "tool": "unica.view",
        "args": {"at": format!("{at}.Can.item.add")},
        "reason": "контракт item.add: один элемент на запись, ключи по виду",
    });
    if item.contains_key("html") {
        return Err(bad(
            op_index,
            &format!("{location}.html"),
            "html is an internal discriminator; use name and type: HTMLDocumentField in item.add",
        )
        .with_next(contract));
    }
    if item.contains_key("textDoc") {
        return Err(bad(
            op_index,
            &format!("{location}.textDoc"),
            "textDoc is an internal discriminator; use name and type: TextDocumentField in item.add",
        )
        .with_next(contract));
    }
    if let Some(key) = FORM_ELEMENT_NESTED_KEYS
        .iter()
        .find(|key| item.contains_key(**key))
    {
        return Err(bad(
            op_index,
            &format!("{location}.{key}"),
            format!(
                "item.add adds one element without nested content; drop `{key}` and add each nested element with a separate item.add at `{container}` once this element exists"
            ),
        )
        .with_next(contract));
    }
    if let Some(key) = FORM_ELEMENT_EVENT_KEYS
        .iter()
        .find(|key| item.contains_key(**key))
    {
        return Err(bad(
            op_index,
            &format!("{location}.{key}"),
            format!(
                "item.add does not bind events; drop `{key}` and bind each handler with event.bind at `{container}.Event.<Event>` once this element exists, naming the handler explicitly"
            ),
        )
        .with_next(contract));
    }
    Ok(())
}

/// Перекрёстные правила ключей элемента, которых схема не выражает.
fn check_element_cross_fields(
    item: &Map<String, Value>,
    op_index: usize,
    location: &str,
) -> Result<(), ApplyPlanError> {
    use crate::domain::operation_contract::FORM_DYNAMIC_LIST_KEYS;
    if item.get("_dynList").and_then(Value::as_bool) != Some(true) {
        if let Some(key) = FORM_DYNAMIC_LIST_KEYS
            .iter()
            .find(|key| item.contains_key(**key))
        {
            return Err(bad(
                op_index,
                &format!("{location}.{key}"),
                format!("`{key}` is a dynamic-list table property: it is written only with `_dynList: true`"),
            ));
        }
    }
    let commands = ["command", "commandName", "stdCommand"]
        .into_iter()
        .filter(|key| item.contains_key(*key))
        .collect::<Vec<_>>();
    if let [_, second, ..] = commands.as_slice() {
        return Err(bad(
            op_index,
            &format!("{location}.{second}"),
            format!(
                "a button runs one command: give one of `command`, `commandName`, `stdCommand`, not {}",
                commands
                    .iter()
                    .map(|key| format!("`{key}`"))
                    .collect::<Vec<_>>()
                    .join(" and ")
            ),
        ));
    }
    Ok(())
}

fn parse_form_operation(
    operation: &str,
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<FormResourcePlanKind, ApplyPlanError> {
    let address = qualified_target(args, op_index, binding)?;
    let target = form_target(&address, op_index)?;
    // Узел, на котором исполняется операция, — ровно её `at`: элемент и
    // событие задаются адресом, а операция другого узла отказывает, а не
    // пишет молча в форму, элемент или владельца (#1351, #1355).
    let placed = match operation {
        "form.add" | "form.set" => target.form.is_none(),
        "item.add" => target.form.is_some() && target.event.is_none(),
        "item.remove" | "item.move" => !target.items.is_empty() && target.event.is_none(),
        "event.bind" => target.event.is_some(),
        _ => target.form.is_some() && target.items.is_empty() && target.event.is_none(),
    };
    if !placed {
        let shape = match operation {
            "form.add" | "form.set" => "the form owner `Owner.Name`, not a form",
            "item.add" => {
                "the form `Owner.Name.Form.F` or its item `….Item.A[.Item.B…]` that receives the elements"
            }
            "item.remove" | "item.move" => {
                "the existing item `Owner.Name.Form.F.Item.A[.Item.B…]`"
            }
            "event.bind" => "the event `Owner.Name.Form.F[.Item.A…].Event.E`",
            _ => "the form `Owner.Name.Form.F`",
        };
        return Err(bad(op_index, "at", format!("{operation} targets {shape}")));
    }
    if operation == "item.add" {
        if let Some(items) = args.get("items").and_then(Value::as_array) {
            for (index, item) in items.iter().enumerate() {
                if let Some(item) = item.as_object() {
                    refuse_nested_element_content(
                        &address.to_string(),
                        item,
                        op_index,
                        &format!("items[{index}]"),
                    )?;
                }
            }
        }
    }
    // Опубликованный контракт и разбор планировщиком принимают одну форму
    // аргументов: ключ, которого писатель не пишет, отказывает до записи,
    // а не теряется молча (#1447).
    if let Some(contract) = crate::domain::operation_contract::OperationContract::form(operation) {
        let mut checked = args.clone();
        // `at` операции уже разобран в адрес; контракт сверяет остальное.
        checked.insert("at".to_string(), Value::String(address.to_string()));
        contract
            .normalize(&Value::Object(checked), &format!("ops[{op_index}].args"))
            .map_err(|error| {
                let contract_at = format!("{address}.Can.{operation}");
                ApplyPlanError::new(
                    ApplyPlanErrorKind::BadValue,
                    format!(
                        "{}: {}; `{operation}` writes only the arguments listed at `{contract_at}`",
                        error.path, error.message
                    ),
                )
                .at_path(error.path)
                .with_next(serde_json::json!({
                    "tool": "unica.view",
                    "args": {"at": contract_at},
                    "reason": "аргументы, которые операция записывает на этом узле",
                }))
            })?;
    }
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
        "item.add" => {
            let form = form_of(&target)?;
            let items = args
                .get("items")
                .and_then(Value::as_array)
                .ok_or_else(|| bad(op_index, "items", "`items` must be an array"))?;
            // Контейнер — последний элемент адреса; у самой формы — корень.
            // Сосед `after` лежит в том же контейнере и задаёт место сам.
            let container = target.items.last().cloned();
            let mut definitions = Vec::new();
            for (index, item) in items.iter().enumerate() {
                let location = format!("items[{index}]");
                let item = item
                    .as_object()
                    .ok_or_else(|| bad(op_index, &location, "each item must be an object"))?;
                check_element_cross_fields(item, op_index, &location)?;
                let (definition, after) = legacy_element_definition(item, op_index, &location)?;
                let mut wrapper = Map::new();
                wrapper.insert("elements".to_string(), Value::Array(vec![definition]));
                match (after, &container) {
                    (Some(after), _) => {
                        wrapper.insert("after".to_string(), Value::String(after));
                    }
                    (None, Some(container)) => {
                        wrapper.insert("into".to_string(), Value::String(container.clone()));
                    }
                    (None, None) => {}
                }
                definitions.push(FormEditStep::Definition(Value::Object(wrapper)));
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
                subject: FormEditSubject::Container(target.items),
                definitions,
            })
        }
        "item.move" => {
            let form = form_of(&target)?;
            let normalized = crate::domain::operation_contract::OperationContract::form_move()
                .normalize(
                    &Value::Object(args.clone()),
                    &format!("ops[{op_index}].args"),
                )
                .map_err(|error| {
                    ApplyPlanError::new(ApplyPlanErrorKind::BadValue, error.message)
                        .at_path(error.path)
                })?;
            let values = &normalized["values"];
            let into = match values.get("into") {
                Some(Value::Null) => super::super::form_move::FormMoveDestination::Root,
                Some(Value::String(name)) => {
                    super::super::form_move::FormMoveDestination::Named(name.clone())
                }
                _ => unreachable!("the item.move contract normalizes an explicit into"),
            };
            let request = super::super::form_move::FormItemMove {
                name: target.items.last().expect("move addresses an item").clone(),
                source_path: Some(target.items.clone()),
                into,
                after: values
                    .get("after")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            };
            Ok(FormResourcePlanKind::FormEdit {
                owner: target.owner,
                form,
                subject: FormEditSubject::Node(target.items),
                definitions: vec![FormEditStep::Move {
                    request,
                    location: format!("ops[{op_index}].args.values"),
                }],
            })
        }
        "item.remove" => {
            let form = form_of(&target)?;
            // Элемент назван адресом: список в аргументах называл бы другие
            // узлы, и молча его не отбрасываем.
            if let Some(key) = ["items", "values"]
                .into_iter()
                .find(|key| args.contains_key(*key))
            {
                return Err(bad(
                    op_index,
                    key,
                    "item.remove removes the item named by its address; drop the argument",
                ));
            }
            let name = target
                .items
                .last()
                .cloned()
                .expect("item.remove target carries an item");
            Ok(FormResourcePlanKind::FormEdit {
                owner: target.owner,
                form,
                subject: FormEditSubject::Removed(target.items),
                definitions: vec![FormEditStep::Definition(
                    serde_json::json!({"removeElements": [{"name": name}]}),
                )],
            })
        }
        "attribute.add" | "command.add" => {
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
                if operation == "attribute.add" {
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
            let key = if operation == "attribute.add" {
                "attributes"
            } else {
                "commands"
            };
            let mut wrapper = Map::new();
            wrapper.insert(key.to_string(), Value::Array(entries));
            Ok(FormResourcePlanKind::FormEdit {
                owner: target.owner,
                form,
                subject: FormEditSubject::Node(Vec::new()),
                definitions: vec![FormEditStep::Definition(Value::Object(wrapper))],
            })
        }
        "event.bind" => {
            let form = form_of(&target)?;
            // Событие и его элемент задаются адресом: `values` несут только
            // обработчик и `callType`.
            let values = required_values(args, op_index, &["handler", "callType"])?;
            let handler = required_string(values, "handler", op_index, "values")?;
            let event = target
                .event
                .as_deref()
                .expect("event.bind target carries an event");
            // Имя события в адресе — имя реестра, как его публикует `view`;
            // псевдонимов у адреса нет.
            let mut entry = Map::new();
            entry.insert("name".to_string(), Value::String(event.to_string()));
            entry.insert("handler".to_string(), Value::String(handler.to_string()));
            if let Some(call_type) = values.get("callType").and_then(Value::as_str) {
                entry.insert("callType".to_string(), Value::String(call_type.to_string()));
            }
            let definition = match target.items.last() {
                Some(element) => {
                    entry.insert("element".to_string(), Value::String(element.clone()));
                    serde_json::json!({"elementEvents": [Value::Object(entry)]})
                }
                None => serde_json::json!({"formEvents": [Value::Object(entry)]}),
            };
            Ok(FormResourcePlanKind::FormEdit {
                owner: target.owner,
                form,
                subject: FormEditSubject::Node(target.items),
                definitions: vec![FormEditStep::Definition(definition)],
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
        let lang =
            super::metadata::staged_text_language(staged, authority.source_kind(), op_index)?;
        let descriptor = form_add_metadata_xml(
            form_name,
            form_name,
            &object_type,
            format_version,
            &uuid,
            &lang,
        );
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
    let payload_relative = forms_dir.join(form);
    let files = staged
        .enumerate_tree(&payload_relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
    for relative in files {
        let preimage = read_required(staged, &relative, "a form payload file", &at_path)?;
        staged
            .remove(&relative, &preimage)
            .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
        touched.push(relative);
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

#[allow(clippy::too_many_arguments)]
fn stage_form_edit(
    staged: &mut ApplyStagedState,
    authority: &FormResourceApplyAuthority<'_>,
    owner: &MetadataAddress,
    form: &str,
    subject: &FormEditSubject,
    definitions: &[FormEditStep],
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
    check_form_edit_subject(&xml_text, subject, definitions, op_index)?;
    let absolute = authority.source_root().join(&relative);
    let args_path = format!("ops[{op_index}].args");
    // Plain-string titles, tooltips and input hints are written in the
    // default language of the source set (#1257). It is read only when a
    // definition writes such text: the definition is first applied without a
    // language and, if it needs one, applied again from the same image.
    let mut lang: Option<String> = None;
    for step in definitions {
        let definition = match step {
            FormEditStep::Definition(definition) => definition,
            FormEditStep::Move { request, location } => {
                xml_text =
                    super::super::form_move::move_item(&xml_text, request).map_err(|error| {
                        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, error.message).at_path(
                            if error.field == "name" {
                                at_path.clone()
                            } else {
                                format!("{location}.{}", error.field)
                            },
                        )
                    })?;
                continue;
            }
        };
        let apply = |xml_text: &mut String, lang: Option<&str>| {
            crate::infrastructure::native_operations::form::form_edit_apply_definition(
                xml_text, definition, &absolute, root_start, lang,
            )
        };
        let before = xml_text.clone();
        let result = match apply(&mut xml_text, lang.as_deref()) {
            Err(message)
                if lang.is_none()
                    && message
                        == crate::infrastructure::native_operations::form::FORM_TEXT_LANGUAGE_REQUIRED =>
            {
                xml_text = before;
                let resolved = super::metadata::staged_text_language(
                    staged,
                    authority.source_kind(),
                    op_index,
                )?;
                apply(&mut xml_text, Some(&resolved)).map(|_| lang = Some(resolved))
            }
            result => result.map(|_| ()),
        };
        result.map_err(|message| {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(args_path.clone())
        })?;
    }
    // Перенос сохраняет исходное состояние BOM. Остальные определения
    // используют прежнюю нормализацию писателя формы.
    let is_move = definitions
        .iter()
        .any(|step| matches!(step, FormEditStep::Move { .. }));
    let postimage = if is_move && !preimage.starts_with(UTF8_BOM) {
        xml_text.as_bytes().to_vec()
    } else {
        with_bom(&xml_text)
    };
    if !is_move && postimage == preimage {
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
    if postimage == preimage {
        return Ok(());
    }
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
/// Читает права роли, применяет правку и стадирует результат с проверкой
/// документа до и после. Неизменный документ не пишется.
fn stage_role_rights(
    staged: &mut ApplyStagedState,
    authority: &FormResourceApplyAuthority<'_>,
    role: &MetadataAddress,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
    edit: impl FnOnce(String) -> Result<String, ApplyPlanError>,
) -> Result<(), ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    let relative = attached_resource_relative(role, "Rights.xml", authority.source_kind())
        .map_err(|message| {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(at_path.clone())
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
        ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message).at_path(at_path.clone())
    };
    let (bom, body) = crate::infrastructure::native_operations::role::decode_role_xml(&preimage)
        .map_err(invalid)?;
    crate::infrastructure::native_operations::role::validate_role_rights_document(
        &body,
        crate::infrastructure::native_operations::role::RoleValueScope::SourceImage,
    )
    .map_err(invalid)?;
    let body = edit(body)?;
    crate::infrastructure::native_operations::role::validate_role_rights_document(
        &body,
        crate::infrastructure::native_operations::role::RoleValueScope::SourceImage,
    )
    .map_err(|message| {
        ApplyPlanError::new(ApplyPlanErrorKind::Postcondition, message)
            .at_path(format!("ops[{op_index}].args"))
    })?;
    let postimage = crate::infrastructure::native_operations::role::encode_role_xml(bom, &body);
    if postimage == preimage {
        return Ok(());
    }
    staged
        .replace(&relative, &preimage, postimage)
        .map_err(|error| ApplyPlanError::staging(error, at_path))?;
    provisional.push(ProvisionalApplyEffect::single(
        relative,
        DomainEvent::new(DomainEventKind::RoleChanged, role.as_str().to_string()),
        op_index,
    ));
    Ok(())
}

/// Одно право объекта роли через общий редактор прав.
fn set_role_right(
    body: &str,
    object: &str,
    right: &str,
    value: bool,
    op_index: usize,
) -> Result<String, ApplyPlanError> {
    let edit = crate::domain::role::RoleEditOperation {
        object_name: object.to_string(),
        right: right.to_string(),
        value,
    };
    crate::infrastructure::native_operations::role::apply_role_edit_operation(body, &edit, op_index)
        .map(|(updated, _effect)| updated)
        .map_err(|message| ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message))
}

/// Есть ли запись прав на объект в стадированном документе прав роли.
/// `None` — документ не читается.
pub(super) fn staged_rights_entry_exists(bytes: &[u8], object: &str) -> Option<bool> {
    let (_, body) = crate::infrastructure::native_operations::role::decode_role_xml(bytes).ok()?;
    rights_object_exists(&body, object).ok()
}

/// Есть ли у роли запись прав на объект.
fn rights_object_exists(body: &str, object: &str) -> Result<bool, ApplyPlanError> {
    Ok(rights_entry_size(body, object)?.is_some())
}

/// Число прав в записи роли на объект; `None` — записи нет.
fn rights_entry_size(body: &str, object: &str) -> Result<Option<usize>, ApplyPlanError> {
    let document = roxmltree::Document::parse(body).map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "Rights.xml is not well-formed XML",
        )
    })?;
    let named = |node: &roxmltree::Node<'_, '_>, tag: &str| {
        node.is_element() && node.tag_name().name() == tag
    };
    Ok(document
        .root_element()
        .children()
        .filter(|node| named(node, "object"))
        .find(|node| {
            node.children()
                .any(|child| named(&child, "name") && child.text().map(str::trim) == Some(object))
        })
        .map(|node| {
            node.children()
                .filter(|child| named(child, "right"))
                .count()
        }))
}

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
        picture_load_transparent: None,
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
                let lang = super::metadata::staged_text_language(
                    &mut staged,
                    authority.source_kind(),
                    op_index,
                )?;
                let descriptor = crate::infrastructure::native_operations::role::role_metadata_xml(
                    name,
                    name,
                    "",
                    authority.expected_format(),
                    &uuid,
                    &lang,
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
                role_at,
                object,
                right,
                value,
                rls,
            } => {
                let value_path = format!("ops[{op_index}].args.values");
                stage_role_rights(
                    &mut staged,
                    &authority,
                    role,
                    op_index,
                    &mut provisional,
                    |mut body| {
                        // `right.set` пишет только в существующую запись:
                        // опечатка в адресе не создаёт объект роли (#1354).
                        if !rights_object_exists(&body, object)? {
                            return Err(ApplyPlanError::new(
                                ApplyPlanErrorKind::NotFound,
                                format!(
                                    "the role has no rights entry for `{object}`; create it with right.add at `{role_at}`"
                                ),
                            )
                            .at_path(format!("ops[{op_index}].args.at")));
                        }
                        let granted = value.or(if rls.is_some() { Some(true) } else { None });
                        if let Some(granted) = granted {
                            body = set_role_right(&body, object, right, granted, op_index)
                                .map_err(|error| error.at_path(value_path.clone()))?;
                        }
                        if let Some(condition) = rls {
                            body = set_rights_restriction(&body, object, right, condition)
                                .map_err(|message| {
                                    ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message)
                                        .at_path(format!("{value_path}.rls"))
                                })?;
                        }
                        Ok(body)
                    },
                )?;
            }
            FormResourcePlanKind::RightAdd {
                role,
                role_at,
                entries,
            } => {
                stage_role_rights(
                    &mut staged,
                    &authority,
                    role,
                    op_index,
                    &mut provisional,
                    |mut body| {
                        for (index, entry) in entries.iter().enumerate() {
                            let location = format!("ops[{op_index}].args.items[{index}]");
                            // Новая запись не перекрывает существующую: её
                            // права меняет `right.set` по её адресу.
                            if rights_object_exists(&body, &entry.object)? {
                                return Err(ApplyPlanError::new(
                                    ApplyPlanErrorKind::BadValue,
                                    format!(
                                        "the role already has a rights entry for `{}`; change it with right.set at `{}`",
                                        entry.object,
                                        member_address(
                                            &format!("{role_at}.Right"),
                                            &entry.object.replacen('.', "_", 1)
                                        )
                                    ),
                                )
                                .at_path(format!("{location}.object")));
                            }
                            body =
                                ensure_rights_object(&body, &entry.object).map_err(|message| {
                                    ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message)
                                        .at_path(location.clone())
                                })?;
                            for (right, granted) in &entry.rights {
                                body =
                                    set_role_right(&body, &entry.object, right, *granted, op_index)
                                        .map_err(|error| {
                                            error.at_path(format!("{location}.rights.{right}"))
                                        })?;
                            }
                            // Права, равные умолчанию роли, платформа не хранит:
                            // такая запись осталась бы пустой и не появилась бы.
                            if rights_entry_size(&body, &entry.object)?.unwrap_or(0) == 0 {
                                return Err(ApplyPlanError::new(
                                    ApplyPlanErrorKind::BadValue,
                                    format!(
                                        "every right of `{}` equals the role default for new objects; the entry would stay empty",
                                        entry.object
                                    ),
                                )
                                .at_path(format!("{location}.rights")));
                            }
                        }
                        Ok(body)
                    },
                )?;
            }
            FormResourcePlanKind::CommandInterface {
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
                            &interface_text_direct_item_values(
                                &text,
                                "SubsystemsOrder",
                                "Subsystem",
                            ),
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
                let postimage =
                    crate::infrastructure::native_operations::common::patch_subsystem_lists(
                        &text, &model, original,
                    )
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
                subject,
                definitions,
            } => {
                stage_form_edit(
                    &mut staged,
                    &authority,
                    owner,
                    form,
                    subject,
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
                prove_support_form_owner(
                    &mut staged,
                    target,
                    &descriptor_relative,
                    &descriptor,
                    &authority,
                    &at_path,
                )?;
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

// Вложенный адрес сначала доказывает регистрацию и идентичность описания,
// только затем его UUID выбирает запись поддержки. Все чтения входят
// в барьер исходного состояния подготовленного применения.
fn prove_support_form_owner(
    staged: &mut ApplyStagedState,
    target: &MetadataAddress,
    relative: &std::path::Path,
    descriptor: &[u8],
    authority: &FormResourceApplyAuthority<'_>,
    at_path: &str,
) -> Result<(), ApplyPlanError> {
    use crate::infrastructure::platform_xml_owner::{
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
    let owner_path = crate::infrastructure::logical_event_source::metadata_descriptor_relative(
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
    let uuid =
        crate::infrastructure::native_operations::common::support_root_uuid_from_bytes(descriptor)
            .ok_or_else(invalid)?;
    uuid::Uuid::parse_str(&uuid).map_err(|_| invalid())?;
    Ok(())
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

    #[test]
    fn a_dynamic_list_table_keeps_its_discriminator() {
        let item = json!({
            "name": "Список",
            "type": "Table",
            "path": "Список",
            "_dynList": true,
            "autoRefresh": true,
        });
        let (definition, _) =
            super::legacy_element_definition(item.as_object().unwrap(), 0, "ops[0].args.items[0]")
                .expect("a dynamic-list table is a valid element definition");
        assert_eq!(definition["_dynList"], true);
        assert_eq!(definition["autoRefresh"], true);
    }
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

    /// Р2 (#1447): `item.add` добавляет один элемент. Вложенное дерево,
    /// companion-содержимое и события отказывают `bad_value` по точному
    /// пути до записи, а `next` ведёт к контракту `item.add`.
    #[test]
    fn item_add_refuses_nested_content_and_events_with_the_next_call() {
        let fixture = ApplySeamFixture::new();
        let at = "main:Document.First.Form.ФормаДокумента";
        for key in crate::domain::operation_contract::FORM_ELEMENT_NESTED_KEYS
            .iter()
            .chain(crate::domain::operation_contract::FORM_ELEMENT_EVENT_KEYS)
            .chain(&["html", "textDoc"])
        {
            let mut item = json!({"name": "Parent", "type": "Group"});
            item[*key] = json!([{"name": "Child", "type": "HTMLDocumentField"}]);
            let error = parse_form_resource_plan_operation(
                "item.add",
                &json!({"at": at, "items": [item]}),
                0,
                &fixture.binding,
            )
            .unwrap_err();
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{key}");
            let path = format!("ops[0].args.items[0].{key}");
            assert_eq!(error.path(), Some(path.as_str()), "{key}");
            assert_eq!(
                error.next().unwrap()["args"]["at"],
                json!(format!("{at}.Can.item.add")),
                "{key}"
            );
            let follow_up = match *key {
                "html" => "HTMLDocumentField".to_string(),
                "textDoc" => "TextDocumentField".to_string(),
                _ => format!("{at}.Item.Parent"),
            };
            assert!(error.to_string().contains(&follow_up), "{key}: {error:?}");
        }
    }

    /// Перекрёстные правила, которых схема не выражает: ключи динамического
    /// списка без `_dynList` и две команды у кнопки пропали бы молча.
    #[test]
    fn item_add_refuses_keys_the_writer_would_drop_by_a_neighbour() {
        let fixture = ApplySeamFixture::new();
        for (item, path) in [
            (
                json!({"name": "T", "type": "Table", "autoRefresh": true}),
                "ops[0].args.items[0].autoRefresh",
            ),
            (
                json!({"name": "B", "type": "Button", "command": "A", "stdCommand": "Close"}),
                "ops[0].args.items[0].stdCommand",
            ),
        ] {
            let error = parse_form_resource_plan_operation(
                "item.add",
                &json!({"at": "main:Document.First.Form.ФормаДокумента", "items": [item]}),
                0,
                &fixture.binding,
            )
            .unwrap_err();
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
            assert_eq!(error.path(), Some(path));
        }
    }

    /// Значения-образцы ключа по его схеме: перебираются, пока одно не
    /// изменит записанный XML.
    fn sample_values(schema: &serde_json::Value) -> Vec<serde_json::Value> {
        if let Some(values) = schema["enum"].as_array() {
            return values.clone();
        }
        if let Some(choices) = schema["anyOf"].as_array() {
            return choices.iter().flat_map(sample_values).collect();
        }
        match schema["type"].as_str() {
            Some("boolean") => vec![json!(true), json!(false)],
            Some("integer") | Some("number") => vec![json!(7)],
            Some("string") => vec![json!("Sample")],
            Some("object") => {
                let mut sample = serde_json::Map::new();
                match schema["properties"].as_object() {
                    Some(properties) => {
                        for (key, child) in properties {
                            sample.insert(key.clone(), sample_values(child)[0].clone());
                        }
                    }
                    None => {
                        sample.insert("en".into(), json!("Sample"));
                    }
                }
                vec![serde_json::Value::Object(sample)]
            }
            other => panic!("no sample for schema type {other:?}"),
        }
    }

    /// Контракт `item.add` и писатель согласны по каждому виду: каждый
    /// опубликованный ключ принимают контракт и планировщик, и хотя бы одно
    /// его значение меняет записанный XML вида (#1447). Ключ, который
    /// писатель вида не пишет, в контракт не попадает.
    #[test]
    fn every_published_element_key_reaches_the_xml_of_its_kind() {
        use crate::infrastructure::native_operations::form::{emit_form_element, FormIdAllocator};
        let fixture = ApplySeamFixture::new();
        let contract =
            crate::domain::operation_contract::OperationContract::form("item.add").unwrap();
        let variants = contract.schema["properties"]["items"]["items"]["anyOf"]
            .as_array()
            .unwrap()
            .clone();
        let emit = |item: &serde_json::Value| {
            let (definition, _) =
                super::legacy_element_definition(item.as_object().unwrap(), 0, "items[0]").unwrap();
            let mut lines = Vec::new();
            emit_form_element(
                &mut lines,
                &definition,
                "\t",
                &mut FormIdAllocator::new(),
                Some("en"),
            )
            .unwrap_or_else(|error| panic!("{item}: {error}"));
            lines.join("\n")
        };
        for kind in crate::domain::operation_contract::FORM_ELEMENT_KINDS {
            let variant = variants
                .iter()
                .find(|variant| variant["properties"]["type"]["enum"][0] == json!(kind.kind))
                .unwrap_or_else(|| panic!("{} has a contract variant", kind.kind));
            let published = variant["properties"].as_object().unwrap();
            assert_eq!(
                published.len(),
                kind.keys.len() + 3,
                "{}: name, type, after and the kind keys",
                kind.kind
            );
            for key in kind.keys {
                let mut baseline = json!({"name": "Sample", "type": kind.kind});
                if kind.kind == "TextDocumentField" {
                    baseline["path"] = json!("Protocol");
                }
                if crate::domain::operation_contract::FORM_DYNAMIC_LIST_KEYS.contains(key) {
                    baseline["_dynList"] = json!(true);
                }
                let before = emit(&baseline);
                let written = sample_values(&published[*key]).into_iter().any(|value| {
                    let mut item = baseline.clone();
                    item[*key] = value;
                    parse_form_resource_plan_operation(
                        "item.add",
                        &json!({"at": "main:Document.First.Form.ФормаДокумента", "items": [item.clone()]}),
                        0,
                        &fixture.binding,
                    )
                    .unwrap_or_else(|error| panic!("{} {key}: {error:?}", kind.kind));
                    emit(&item) != before
                });
                assert!(written, "{}: `{key}` never reaches the XML", kind.kind);
            }
        }
    }

    /// Перечисления контракта элемента совпадают со справочником писателя.
    #[test]
    fn element_enums_match_the_writer_enum_contract() {
        use crate::domain::operation_contract::form_element_enum;
        use crate::infrastructure::native_operations::form::form_8_3_27_enum_values_for_test as writer;
        for (kind, key, xml) in [
            ("InputField", "titleLocation", "FormElementTitleLocation"),
            ("CheckBox", "checkBoxType", "CheckBoxType"),
            ("InputField", "horizontalAlign", "HorizontalAlign"),
            ("Group", "orientation", "FormChildrenGroup"),
            ("Group", "behavior", "UsualGroupBehavior"),
            ("Group", "representation", "UsualGroupRepresentation"),
            ("Table", "representation", "TableRepresentation"),
            ("Button", "representation", "ButtonRepresentation"),
            ("Group", "currentRowUse", "CurrentRowUse"),
            ("Pages", "pagesRepresentation", "PagesRepresentation"),
            ("Table", "commandBarLocation", "CommandBarLocation"),
            ("Table", "initialTreeView", "InitialTreeView"),
            ("Table", "choiceFoldersAndItems", "ChoiceFoldersAndItems"),
            ("Table", "updateOnDataChange", "UpdateOnDataChange"),
            (
                "Button",
                "locationInCommandBar",
                "ButtonLocationInCommandBar",
            ),
        ] {
            let mut published = form_element_enum(kind, key).unwrap().to_vec();
            let mut expected = writer(xml).unwrap().to_vec();
            published.sort_unstable();
            expected.sort_unstable();
            assert_eq!(published, expected, "{kind} {key}");
        }
    }

    /// Реквизит формы принимает только то, что пишет его писатель (#1447);
    /// проверка заполнения пишется тегом `<FillCheck>`, который знает 8.3.27.
    #[test]
    fn form_attribute_add_refuses_unwritten_keys_and_writes_fill_check() {
        let fixture = ApplySeamFixture::new();
        let at = "main:Document.First.Form.ФормаДокумента";
        for (key, value) in [
            ("view", json!(false)),
            ("edit", json!(false)),
            ("functionalOptions", json!(["FunctionalOption.X"])),
            ("useAlways", json!(["Field"])),
            ("save", json!(true)),
            ("fillCheck", json!(true)),
            ("valueType", json!("String")),
            ("settings", json!({})),
            ("chart", json!({})),
            ("planner", json!({})),
            ("additionalColumns", json!([])),
            ("title", json!({"ru": "Заголовок"})),
            ("fillChecking", json!("DontCheck")),
        ] {
            let error = parse_form_resource_plan_operation(
                "attribute.add",
                &json!({"at": at, "items": [{"name": "A", "type": "String", key: value}]}),
                0,
                &fixture.binding,
            )
            .unwrap_err();
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{key}");
            let path = format!("ops[0].args.items[0].{key}");
            assert_eq!(error.path(), Some(path.as_str()), "{key}");
        }
        let mut lines = Vec::new();
        crate::infrastructure::native_operations::form::emit_form_edit_attribute_item(
            &mut lines,
            json!({"name": "A", "type": "xs:string", "savedData": true, "fillChecking": "ShowError"})
                .as_object()
                .unwrap(),
            "A",
            1,
            "\t\t",
            Some("ru"),
        )
        .unwrap();
        let text = lines.join("\n");
        assert!(!text.contains("FillChecking"), "{text}");
        let saved = text.find("<SavedData>true</SavedData>").unwrap();
        let fill = text.find("<FillCheck>ShowError</FillCheck>").unwrap();
        assert!(saved < fill, "{text}");
    }

    #[test]
    fn role_create_and_form_add_write_the_configuration_default_language() {
        let fixture = ApplySeamFixture::new();
        fixture.use_english_only_language();
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
                "role.create",
                json!({"at": "main:Configuration", "values": {"name": "Manager"}}),
                0,
            ),
            parse(
                "form.add",
                json!({"at": "main:Document.First", "items": [{"name": "Main", "type": "ObjectForm"}]}),
                1,
            ),
        ];
        let (staged, _) = plan_form_resource_batch(staged, authority, &operations)
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        for relative in ["Roles/Manager.xml", "Documents/First/Forms/Main.xml"] {
            let text = staged_text(&staged, relative);
            assert!(text.contains("<v8:lang>en</v8:lang>"), "{relative}\n{text}");
            assert!(
                !text.contains("<v8:lang>ru</v8:lang>"),
                "{relative}\n{text}"
            );
        }
    }

    /// #1257: titles, tooltips and input hints of new form elements,
    /// attributes and commands are written in the default language of the
    /// source set; an explicit language-to-text value is written as given.
    #[test]
    fn form_item_titles_and_tooltips_write_the_configuration_default_language() {
        let fixture = ApplySeamFixture::new();
        fixture.use_english_only_language();
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
        let form = "main:Document.First.Form.Main";
        let operations = [
            parse(
                "form.add",
                json!({"at": "main:Document.First", "items": [{"name": "Main", "type": "ObjectForm"}]}),
                0,
            ),
            parse(
                "item.add",
                json!({"at": form, "items": [
                    {"name": "Header", "type": "Group", "title": "Header"},
                    {"name": "Total", "path": "Объект.Total", "title": "Total",
                     "tooltip": "Total hint", "inputHint": "Type the total"},
                    {"name": "Count", "path": "Объект.Total",
                     "tooltip": {"en": "Count hint", "de": "Anzahl"}},
                    {"name": "Note", "type": "Label", "title": "Note"}
                ]}),
                1,
            ),
            parse(
                "attribute.add",
                json!({"at": form, "items": [{"name": "Comment", "type": "String", "title": "Comment"}]}),
                2,
            ),
            parse(
                "command.add",
                json!({"at": form, "items": [{"name": "Refresh", "title": "Refresh"}]}),
                3,
            ),
        ];
        let (staged, _) = plan_form_resource_batch(staged, authority, &operations)
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let text = staged_text(&staged, "Documents/First/Forms/Main/Ext/Form.xml");
        assert!(!text.contains("<v8:lang>ru</v8:lang>"), "{text}");
        let document = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).unwrap();
        let translations = document
            .descendants()
            .filter(|node| node.tag_name().name() == "item")
            .filter_map(|item| {
                let part = |name: &str| {
                    item.children()
                        .find(|child| child.tag_name().name() == name)
                        .and_then(|child| child.text())
                };
                Some((part("lang")?.to_string(), part("content")?.to_string()))
            })
            .collect::<Vec<_>>();
        for content in [
            "Header",
            "Total",
            "Total hint",
            "Type the total",
            "Count hint",
            "Note",
            "Comment",
            "Refresh",
        ] {
            assert!(
                translations.contains(&("en".to_string(), content.to_string())),
                "`{content}` must be written in en: {translations:?}\n{text}"
            );
        }
        assert!(
            translations.contains(&("de".to_string(), "Anzahl".to_string())),
            "an explicit translation is written as given\n{text}"
        );
    }

    /// Only a plain-string text needs the default language: in a source set
    /// whose language cannot be read, an element without text, an explicit
    /// translation and a removal pass, a plain-string title is refused.
    #[test]
    fn form_edits_without_plain_text_do_not_read_the_default_language() {
        let fixture = ApplySeamFixture::new();
        let form = "main:Document.First.Form.Main";
        let plan = |operations: &[(&str, serde_json::Value)]| {
            let admission = fixture.admission();
            let staged = admission.staged_state().unwrap();
            let authority = admission
                .form_resource_planning_authority(&fixture.binding)
                .unwrap();
            let operations = operations
                .iter()
                .enumerate()
                .map(|(index, (op, args))| {
                    IndexedPlanOperation::new(
                        index,
                        parse_form_resource_plan_operation(op, args, index, &fixture.binding)
                            .unwrap_or_else(|error| panic!("{op}: {error:?}")),
                    )
                })
                .collect::<Vec<_>>();
            plan_form_resource_batch(staged, authority, &operations).map(|(staged, _)| staged)
        };
        let staged = plan(&[
            (
                "form.add",
                json!({"at": "main:Document.First", "items": [{"name": "Main", "type": "ObjectForm"}]}),
            ),
            (
                "item.add",
                json!({"at": form, "items": [{"name": "Header", "type": "Group", "title": "Header"}]}),
            ),
        ])
        .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        for relative in [
            "Documents/First.xml",
            "Documents/First/Forms/Main.xml",
            "Documents/First/Forms/Main/Ext/Form.xml",
            "Documents/First/Forms/Main/Ext/Form/Module.bsl",
        ] {
            let path = fixture.source_dir().join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, staged_text(&staged, relative)).unwrap();
        }
        std::fs::remove_file(fixture.source_dir().join("Languages/Русский.xml")).unwrap();

        let error = plan(&[(
            "item.add",
            json!({"at": form, "items": [{"name": "Footer", "type": "Group", "title": "Footer"}]}),
        )])
        .expect_err("a new title needs the default language");
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidSource, "{error:?}");

        let staged = plan(&[
            (
                "item.add",
                json!({"at": form, "items": [{"name": "Plain", "path": "Объект.Total"}]}),
            ),
            (
                "item.add",
                json!({"at": form, "items": [{"name": "Hinted", "path": "Объект.Total",
                    "tooltip": {"en": "Hint"}}]}),
            ),
        ])
        .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let text = staged_text(&staged, "Documents/First/Forms/Main/Ext/Form.xml");
        assert!(text.contains("name=\"Plain\""), "{text}");
        assert!(text.contains("<v8:lang>en</v8:lang>"), "{text}");
        assert!(!text.contains("<v8:lang></v8:lang>"), "{text}");

        let staged = plan(&[("item.remove", json!({"at": format!("{form}.Item.Header")}))])
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let text = staged_text(&staged, "Documents/First/Forms/Main/Ext/Form.xml");
        assert!(!text.contains("name=\"Header\""), "{text}");
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
                "item.add",
                json!({
                    "at": "main:Document.First.Form.ФормаДокумента",
                    "items": [{"name": "ГруппаИтог", "type": "UsualGroup"}]
                }),
                1,
            ),
            // Группа, добавленная предыдущей операцией, уже адресуется.
            parse(
                "item.add",
                json!({
                    "at": "main:Document.First.Form.ФормаДокумента.Item.ГруппаИтог",
                    "items": [{"name": "ПолеИтог", "path": "Объект.Total", "title": "Итог"}]
                }),
                2,
            ),
            parse(
                "event.bind",
                json!({
                    "at": "main:Document.First.Form.ФормаДокумента.Event.OnOpen",
                    "values": {"handler": "ПриОткрытии"}
                }),
                3,
            ),
            parse(
                "event.bind",
                json!({
                    "at": "main:Document.First.Form.ФормаДокумента.Item.ГруппаИтог.Item.ПолеИтог.Event.OnChange",
                    "values": {"handler": "ПолеИтогПриИзменении"}
                }),
                4,
            ),
        ];
        let (staged, effects) = plan_form_resource_batch(staged, authority, &operations)
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        assert_eq!(effects.len(), 5);
        let form = staged_text(&staged, "Documents/First/Forms/ФормаДокумента/Ext/Form.xml");
        let elements = super::form_items_of(&form).unwrap();
        let field = super::form_item_at(
            &elements,
            &["ГруппаИтог".to_string(), "ПолеИтог".to_string()],
        )
        .expect("the field lands inside the group named by the address");
        assert_eq!(field.events.len(), 1, "{form}");
        assert_eq!(field.events[0].name, "OnChange");
        assert_eq!(field.events[0].handler, "ПолеИтогПриИзменении");
        assert!(form.contains("<DataPath>Объект.Total</DataPath>"), "{form}");
        assert!(
            form.contains("<Event name=\"OnOpen\">ПриОткрытии</Event>"),
            "{form}"
        );
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
    fn item_add_writes_a_bound_text_document_field_and_refuses_invalid_bindings() {
        let fixture = ApplySeamFixture::new();
        let form = "main:Document.First.Form.Main";
        let parse = |op: &str, args: serde_json::Value, index: usize| {
            parse_form_resource_plan_operation(op, &args, index, &fixture.binding)
                .map(|operation| IndexedPlanOperation::new(index, operation))
        };
        for item in [
            json!({"name":"ProtocolField","type":"TextDocumentField"}),
            json!({"name":"ProtocolField","type":"TextDocumentField","path":""}),
        ] {
            let error = parse("item.add", json!({"at": form, "items": [item]}), 0).unwrap_err();
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
            assert_eq!(error.path(), Some("ops[0].args.items[0].path"));
        }

        let plan = |attribute_type: Option<&str>| {
            let admission = fixture.admission();
            let staged = admission.staged_state().unwrap();
            let authority = admission
                .form_resource_planning_authority(&fixture.binding)
                .unwrap();
            let mut operations = vec![parse(
                "form.add",
                json!({"at":"main:Document.First","items":[{"name":"Main","type":"ObjectForm"}]}),
                0,
            )
            .unwrap()];
            if let Some(attribute_type) = attribute_type {
                operations.push(
                    parse(
                        "attribute.add",
                        json!({"at":form,"items":[{"name":"Protocol","type":attribute_type}]}),
                        1,
                    )
                    .unwrap(),
                );
            }
            let item_index = operations.len();
            operations.push(
                parse(
                    "item.add",
                    json!({"at":form,"items":[
                        {
                            "name":"Протокол1",
                            "type":"TextDocumentField",
                            "path":"Protocol",
                            "titleLocation":"None"
                        },
                        {
                            "name":"Протокол2",
                            "type":"TextDocumentField",
                            "path":"Protocol",
                            "visible":false,
                            "enabled":false,
                            "readOnly":true,
                            "title":"Протокол",
                            "titleLocation":"Top",
                            "tooltip":"Подсказка",
                            "tooltipRepresentation":"Button",
                            "width":35,
                            "autoMaxWidth":false,
                            "height":10,
                            "autoMaxHeight":false
                        }
                    ]}),
                    item_index,
                )
                .unwrap(),
            );
            plan_form_resource_batch(staged, authority, &operations).map(|(staged, _)| staged)
        };

        let before = crate::test_support::tree_snapshot(&fixture.source_dir());
        for attribute_type in [None, Some("String")] {
            let error = plan(attribute_type).unwrap_err();
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{error:?}");
            assert!(
                error
                    .to_string()
                    .contains("requires a TextDocument attribute"),
                "{error:?}"
            );
            assert_eq!(
                crate::test_support::tree_snapshot(&fixture.source_dir()),
                before,
                "planning an invalid field must not write the source tree"
            );
        }

        let staged = plan(Some("d5p1:TextDocument"))
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let text = staged_text(&staged, "Documents/First/Forms/Main/Ext/Form.xml");
        let document = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).unwrap();
        let platform_export = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/platform_8_3_27/text-document-field/Form.xml"
        ));
        let platform = roxmltree::Document::parse(platform_export).unwrap();
        for name in ["Протокол1", "Протокол2"] {
            let child_tags = |root: roxmltree::Node<'_, '_>| {
                root.descendants()
                    .find(|node| {
                        node.has_tag_name("TextDocumentField")
                            && node.attribute("name") == Some(name)
                    })
                    .unwrap()
                    .children()
                    .filter(|node| node.is_element())
                    .map(|node| node.tag_name().name().to_owned())
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                child_tags(document.root()),
                child_tags(platform.root()),
                "{name}"
            );
        }
        assert_eq!(document.root_element().attribute("version"), Some("2.20"));
        let field = document
            .descendants()
            .find(|node| {
                node.has_tag_name("TextDocumentField")
                    && node.attribute("name") == Some("Протокол2")
            })
            .unwrap();
        assert!(field.children().all(|node| !node.has_tag_name("Height")));
        assert_eq!(
            field
                .children()
                .find(|node| node.has_tag_name("DataPath"))
                .and_then(|node| node.text()),
            Some("Protocol")
        );
        let type_node = document
            .descendants()
            .find(|node| node.text() == Some("d5p1:TextDocument"))
            .unwrap();
        assert_eq!(
            type_node.lookup_namespace_uri(Some("d5p1")),
            Some("http://v8.1c.ru/8.1/data/txtedt")
        );
        assert!(!text.contains("<HTMLDocumentField name=\"Протокол2\""));
        assert!(!text.contains("<InputField name=\"Протокол2\""));
    }

    /// Узел правки формы — ровно адрес (#1355): цепочка `Item` сверяется
    /// с формой, отсутствующий элемент даёт `not_found`, в поле не
    /// добавить, сосед `after` берётся только из адресованного контейнера,
    /// а удаление и событие касаются только названного элемента.
    #[test]
    fn a_form_edit_writes_only_the_addressed_item() {
        let fixture = ApplySeamFixture::new();
        let admission = fixture.admission();
        let form = "main:Document.First.Form.Main";
        let parse = |op: &str, args: serde_json::Value, index: usize| {
            parse_form_resource_plan_operation(op, &args, index, &fixture.binding)
                .map(|operation| IndexedPlanOperation::new(index, operation))
        };
        let base = || {
            vec![
                parse(
                    "form.add",
                    json!({"at": "main:Document.First", "items": [{"name": "Main", "type": "ObjectForm"}]}),
                    0,
                )
                .unwrap(),
                parse(
                    "item.add",
                    json!({"at": form, "items": [
                        {"name": "Header", "type": "Group"},
                        {"name": "Footer", "type": "Group"}
                    ]}),
                    1,
                )
                .unwrap(),
                parse(
                    "item.add",
                    json!({"at": format!("{form}.Item.Header"), "items": [
                        {"name": "Total", "path": "Объект.Total"},
                        {"name": "Count", "path": "Объект.Total"}
                    ]}),
                    2,
                )
                .unwrap(),
            ]
        };
        let plan = |op: &str, args: serde_json::Value| {
            let mut operations = base();
            operations.push(parse(op, args, 3)?);
            let authority = admission
                .form_resource_planning_authority(&fixture.binding)
                .unwrap();
            plan_form_resource_batch(admission.staged_state().unwrap(), authority, &operations).map(
                |(staged, _)| {
                    let text = staged_text(&staged, "Documents/First/Forms/Main/Ext/Form.xml");
                    super::form_items_of(&text).unwrap()
                },
            )
        };
        let names = |elements: &[super::FormInfoElement]| {
            elements
                .iter()
                .map(|element| element.name.clone())
                .collect::<Vec<_>>()
        };
        let chain = |names: &[&str]| {
            names
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>()
        };
        for (op, args, kind, path) in [
            (
                "item.add",
                json!({"at": format!("{form}.Item.Footer.Item.Total"), "items": [{"name": "X", "path": "Объект.Total"}]}),
                ApplyPlanErrorKind::NotFound,
                "ops[3].args.at",
            ),
            (
                "item.add",
                json!({"at": format!("{form}.Item.Header.Item.Total"), "items": [{"name": "X", "path": "Объект.Total"}]}),
                ApplyPlanErrorKind::BadValue,
                "ops[3].args.at",
            ),
            (
                "item.add",
                json!({"at": format!("{form}.Item.Footer"), "items": [{"name": "X", "path": "Объект.Total", "after": "Total"}]}),
                ApplyPlanErrorKind::BadValue,
                "ops[3].args.items[0].after",
            ),
            (
                "item.remove",
                json!({"at": format!("{form}.Item.Total")}),
                ApplyPlanErrorKind::NotFound,
                "ops[3].args.at",
            ),
            (
                "event.bind",
                json!({"at": format!("{form}.Item.Footer.Item.Total.Event.OnChange"), "values": {"handler": "H"}}),
                ApplyPlanErrorKind::NotFound,
                "ops[3].args.at",
            ),
        ] {
            let Err(error) = plan(op, args.clone()) else {
                panic!("{op} {args} is refused");
            };
            assert_eq!(error.kind(), kind, "{op} {args}: {error}");
            assert_eq!(error.path(), Some(path), "{op} {args}: {error}");
        }
        // Сосед в том же контейнере задаёт место.
        let elements = plan(
            "item.add",
            json!({"at": format!("{form}.Item.Header"), "items": [{"name": "X", "path": "Объект.Total", "after": "Total"}]}),
        )
        .unwrap();
        let header = super::form_item_at(&elements, &chain(&["Header"])).unwrap();
        assert_eq!(names(&header.children), ["Total", "X", "Count"]);
        // Удаление вложенного элемента оставляет контейнер и соседа.
        let elements = plan(
            "item.remove",
            json!({"at": format!("{form}.Item.Header.Item.Total")}),
        )
        .unwrap();
        // Автоматическая командная панель новой формы — тоже элемент `Item`.
        assert_eq!(
            names(&elements),
            ["ФормаКоманднаяПанель", "Header", "Footer"]
        );
        let header = super::form_item_at(&elements, &chain(&["Header"])).unwrap();
        assert_eq!(names(&header.children), ["Count"]);
        // Событие ложится только на названный элемент.
        let elements = plan(
            "event.bind",
            json!({"at": format!("{form}.Item.Header.Item.Count.Event.OnChange"), "values": {"handler": "CountOnChange"}}),
        )
        .unwrap();
        let count = super::form_item_at(&elements, &chain(&["Header", "Count"])).unwrap();
        assert_eq!(count.events.len(), 1);
        assert_eq!(count.events[0].handler, "CountOnChange");
        let total = super::form_item_at(&elements, &chain(&["Header", "Total"])).unwrap();
        assert!(total.events.is_empty());
        // Снятые аргументы разбор семейства отвергает и в обход реестра.
        for (op, args, path) in [
            (
                "item.add",
                json!({"at": form, "items": [{"name": "X", "into": "Header"}]}),
                "ops[3].args.items[0].into",
            ),
            (
                "item.remove",
                json!({"at": format!("{form}.Item.Header"), "items": [{"name": "Total"}]}),
                "ops[3].args.items",
            ),
            (
                "event.bind",
                json!({"at": format!("{form}.Event.OnOpen"), "values": {"handler": "H", "event": "OnClose"}}),
                "ops[3].args.values.event",
            ),
            (
                "event.bind",
                json!({"at": form, "values": {"handler": "H"}}),
                "ops[3].args.at",
            ),
        ] {
            let Err(error) = parse(op, args.clone(), 3) else {
                panic!("{op} {args} is refused");
            };
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{op} {args}");
            assert_eq!(error.path(), Some(path), "{op} {args}: {error}");
        }
    }

    /// Сосед `after` может быть добавлен раньше в том же `items` (#1443).
    #[test]
    fn item_add_after_names_an_item_added_earlier_in_the_same_list() {
        let fixture = ApplySeamFixture::new();
        let admission = fixture.admission();
        let form = "main:Document.First.Form.Main";
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
                json!({"at": "main:Document.First", "items": [{"name": "Main", "type": "ObjectForm"}]}),
                0,
            ),
            parse(
                "item.add",
                json!({"at": form, "items": [{"name": "Header", "type": "Group"}]}),
                1,
            ),
            parse(
                "item.add",
                json!({"at": format!("{form}.Item.Header"), "items": [
                    {"name": "A", "path": "Объект.Total"},
                    {"name": "B", "path": "Объект.Total", "after": "A"}
                ]}),
                2,
            ),
        ];
        let authority = admission
            .form_resource_planning_authority(&fixture.binding)
            .unwrap();
        let (staged, _) =
            plan_form_resource_batch(admission.staged_state().unwrap(), authority, &operations)
                .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let text = staged_text(&staged, "Documents/First/Forms/Main/Ext/Form.xml");
        let elements = super::form_items_of(&text).unwrap();
        let header = super::form_item_at(&elements, &["Header".to_string()]).unwrap();
        let names = header
            .children
            .iter()
            .map(|element| element.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["A", "B"], "{text}");
    }

    /// События из каталога 8.3.27, зависящие от вида владельца формы и пути
    /// данных таблицы, принимаются и при добавлении элемента, и итоговой
    /// проверкой формы (#1443).
    #[test]
    fn owner_and_data_path_specific_events_pass_planning_and_final_validation() {
        let fixture = ApplySeamFixture::new();
        let admission = fixture.admission();
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
                json!({"at": "main:Document.First", "items": [
                    {"name": "Main", "type": "ObjectForm"},
                    {"name": "List", "type": "ListForm"}
                ]}),
                0,
            ),
            // `ValueChoice` формы есть только в каталоге владельца-документа.
            parse(
                "event.bind",
                json!({
                    "at": "main:Document.First.Form.Main.Event.ValueChoice",
                    "values": {"handler": "ОбработкаВыбораЗначения"}
                }),
                1,
            ),
            // `URLListGetProcessing` таблицы есть только у источника
            // динамического списка, его выбирает путь данных таблицы.
            parse(
                "item.add",
                json!({
                    "at": "main:Document.First.Form.List",
                    "items": [{
                        "name": "ВтороеПредставление",
                        "type": "Table",
                        "path": "Список"
                    }]
                }),
                2,
            ),
            // Событие нового элемента привязывает `event.bind` на его адресе
            // (Р2, #1447).
            parse(
                "event.bind",
                json!({
                    "at": "main:Document.First.Form.List.Item.ВтороеПредставление.Event.URLListGetProcessing",
                    "values": {"handler": "ВтороеПредставлениеОбработкаПолученияСпискаURL"}
                }),
                3,
            ),
        ];
        let authority = admission
            .form_resource_planning_authority(&fixture.binding)
            .unwrap();
        let (staged, _) =
            plan_form_resource_batch(admission.staged_state().unwrap(), authority, &operations)
                .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let main = staged_text(&staged, "Documents/First/Forms/Main/Ext/Form.xml");
        assert!(
            main.contains("<Event name=\"ValueChoice\">ОбработкаВыбораЗначения</Event>"),
            "{main}"
        );
        let list = staged_text(&staged, "Documents/First/Forms/List/Ext/Form.xml");
        assert!(
            list.contains("<Event name=\"URLListGetProcessing\">"),
            "{list}"
        );
    }

    /// Главное обещание семейства: писатель правит только `Common`, а
    /// значения видимости по ролям — соседние элементы того же блока —
    /// остаются как были. Читатель их не показывает, поэтому наивный писатель,
    /// переписывающий блок по прочитанному, снёс бы их молча.
    #[test]
    fn command_visibility_edits_common_and_leaves_role_values_alone() {
        let fixture = ApplySeamFixture::new();
        let interface_dir = fixture.source_dir().join("Subsystems/Sales/Ext");
        std::fs::create_dir_all(&interface_dir).unwrap();
        std::fs::write(
            fixture.source_dir().join("Subsystems/Sales.xml"),
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><Subsystem uuid=\"44444444-4444-4444-8444-444444444444\"><Properties><Name>Sales</Name></Properties><ChildObjects/></Subsystem></MetaDataObject>",
        )
        .unwrap();
        std::fs::write(
            interface_dir.join("CommandInterface.xml"),
            concat!(
                "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n",
                "<CommandInterface xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" version=\"2.20\">\n",
                "\t<CommandsVisibility>\n",
                "\t\t<Command name=\"Catalog.First.Command.Open\">\n",
                "\t\t\t<Visibility>\n",
                "\t\t\t\t<xr:Common>true</xr:Common>\n",
                "\t\t\t\t<xr:Value name=\"Role.Кладовщик\">false</xr:Value>\n",
                "\t\t\t</Visibility>\n",
                "\t\t</Command>\n",
                "\t</CommandsVisibility>\n",
                "\t<GroupsOrder>\n",
                "\t\t<Group>NavigationPanelImportant</Group>\n",
                "\t\t<Group>NavigationPanelSeeAlso</Group>\n",
                "\t</GroupsOrder>\n",
                "</CommandInterface>\n"
            ),
        )
        .unwrap();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .form_resource_planning_authority(&fixture.binding)
            .unwrap();
        let parse = |op: &str, args: serde_json::Value, index: usize| {
            IndexedPlanOperation::new(
                index,
                parse_form_resource_plan_operation(op, &args, index, &fixture.binding).unwrap(),
            )
        };
        let operations = [
            parse(
                "commandVisibility.set",
                json!({
                    "at": "main:Subsystem.Sales.Interface",
                    "items": [{"command": "Catalog.First.Command.Open", "visible": false}]
                }),
                0,
            ),
            parse(
                "groupOrder.set",
                json!({
                    "at": "main:Subsystem.Sales.Interface",
                    "values": {"groups": ["NavigationPanelSeeAlso", "NavigationPanelImportant"]}
                }),
                1,
            ),
        ];

        let (staged, effects) = plan_form_resource_batch(staged, authority, &operations)
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));

        assert_eq!(effects.len(), 2);
        let text = staged_text(&staged, "Subsystems/Sales/Ext/CommandInterface.xml");
        assert!(text.contains("<xr:Common>false</xr:Common>"), "{text}");
        // Роль на месте — и это единственная причина, по которой правка
        // общего значения вообще безопасна.
        assert!(
            text.contains("<xr:Value name=\"Role.Кладовщик\">false</xr:Value>"),
            "{text}"
        );
        let seen = text.find("NavigationPanelSeeAlso").expect("group order");
        let important = text
            .find("<Group>NavigationPanelImportant</Group>")
            .expect("group");
        assert!(seen < important, "группы переставлены: {text}");
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
        let parse = |op: &str, at: &str, args: serde_json::Value, index: usize| {
            let mut args = args;
            args["at"] = json!(at);
            IndexedPlanOperation::new(
                index,
                parse_form_resource_plan_operation(op, &args, index, &fixture.binding).unwrap(),
            )
        };
        // Запись прав создаёт `right.add` на роли, а `right.set` меняет
        // существующую запись по её адресу (#1354).
        let operations = [
            parse(
                "right.set",
                "main:Role.Reader.Right.Document_First",
                json!({"values": {"right": "Update", "value": true}}),
                0,
            ),
            parse(
                "right.add",
                "main:Role.Reader",
                json!({"items": [{"object": "Document.Second", "rights": {"Read": true}}]}),
                1,
            ),
            parse(
                "right.set",
                "main:Role.Reader.Right.Document_Second",
                json!({"values": {"right": "Read", "rls": "ГДЕ Автор = &Пользователь"}}),
                2,
            ),
            // Вложенный объект — одна запись с экранированными точками
            // в имени: созданную запись правит `right.set` по её адресу.
            parse(
                "right.add",
                "main:Role.Reader",
                json!({"items": [{"object": "DataProcessor.D.Command.C", "rights": {"View": true}}]}),
                3,
            ),
            parse(
                "right.set",
                "main:Role.Reader.Right.DataProcessor_D\\.Command\\.C",
                json!({"values": {"right": "View", "value": false}}),
                4,
            ),
        ];
        let (staged, effects) = plan_form_resource_batch(staged, authority, &operations)
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        assert_eq!(effects.len(), 5);
        let text = staged_text(&staged, "Roles/Reader/Ext/Rights.xml");
        assert!(text.starts_with('\u{feff}'), "byte-order mark survives");
        assert!(text.contains("<name>Update</name>"), "{text}");
        assert!(text.contains("<name>Document.Second</name>"), "{text}");
        let nested = text
            .split("<object>")
            .find(|entry| entry.contains("<name>DataProcessor.D.Command.C</name>"))
            .unwrap_or_else(|| panic!("nested entry is written: {text}"));
        assert!(
            nested.contains("<name>View</name>") && nested.contains("<value>false</value>"),
            "{nested}"
        );
        assert!(
            text.contains("<condition>ГДЕ Автор = &amp;Пользователь</condition>"),
            "{text}"
        );

        // Опечатка в адресе записи не создаёт объект роли, а повторное
        // `right.add` не перекрывает существующую запись.
        for (op, at, args, kind, path) in [
            (
                "right.set",
                "main:Role.Reader.Right.Document_Frist",
                json!({"values": {"right": "Read", "value": true}}),
                ApplyPlanErrorKind::NotFound,
                "ops[0].args.at",
            ),
            (
                "right.add",
                "main:Role.Reader",
                json!({"items": [{"object": "Document.First", "rights": {"Read": false}}]}),
                ApplyPlanErrorKind::BadValue,
                "ops[0].args.items[0].object",
            ),
            (
                "right.add",
                "main:Role.Reader",
                json!({"items": [{"object": "Document.Second", "rights": {"Read": false}}]}),
                ApplyPlanErrorKind::BadValue,
                "ops[0].args.items[0].rights",
            ),
        ] {
            let admission = fixture.admission();
            let staged = admission.staged_state().unwrap();
            let authority = admission
                .form_resource_planning_authority(&fixture.binding)
                .unwrap();
            let Err(error) = plan_form_resource_batch(staged, authority, &[parse(op, at, args, 0)])
            else {
                panic!("{op} at {at}: the existing entry decides");
            };
            assert_eq!(error.kind(), kind, "{op}");
            assert_eq!(error.path(), Some(path), "{op}");
        }
        // Объект записи назван адресом: прежний аргумент и адрес роли
        // разбор отвергает.
        for (op, at, args, path) in [
            (
                "right.set",
                "main:Role.Reader.Right.Document_First",
                json!({"values": {"object": "Document.First", "right": "Read", "value": true}}),
                "ops[0].args.values.object",
            ),
            (
                "right.set",
                "main:Role.Reader",
                json!({"values": {"right": "Read", "value": true}}),
                "ops[0].args.at",
            ),
            (
                "right.add",
                "main:Role.Reader",
                json!({"items": [{"object": "Document.Third", "rights": {}}]}),
                "ops[0].args.items[0].rights",
            ),
            (
                "right.add",
                "main:Role.Reader",
                json!({"items": [{"object": "Third", "rights": {"Read": true}}]}),
                "ops[0].args.items[0].object",
            ),
            (
                "right.add",
                "main:Role.Reader",
                json!({"items": [{"object": "Document.A:B", "rights": {"Read": true}}]}),
                "ops[0].args.items[0].object",
            ),
            (
                "right.add",
                "main:Role.Reader",
                json!({"items": [{"object": "DataProcessor.D..C", "rights": {"Use": true}}]}),
                "ops[0].args.items[0].object",
            ),
            // Короткий псевдоним чтения — не каноническое имя записи.
            (
                "right.set",
                "main:Role.Reader.Right.Товары_Архив",
                json!({"values": {"right": "Read", "value": true}}),
                "ops[0].args.at",
            ),
        ] {
            let mut args = args;
            args["at"] = json!(at);
            let error = parse_form_resource_plan_operation(op, &args, 0, &fixture.binding)
                .expect_err("the subject comes from the address");
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{op} {path}");
            assert_eq!(error.path(), Some(path), "{op}");
        }
    }

    /// Вложенный объект `right.add` — из закрытого набора раскладок
    /// Rights.xml. Опечатка вида и вложенный объект корня конфигурации
    /// отклоняются до записи, право вложенного объекта — из его набора.
    #[test]
    fn right_add_accepts_only_nested_objects_a_role_lists() {
        let fixture = ApplySeamFixture::new();
        for object in [
            "Catalog.Items.Command.Open",
            "Catalog.Items.Attribute.Price",
            "Catalog.Items.StandardAttribute.Code",
            "Document.Order.TabularSection.Goods",
            "Document.Order.TabularSection.Goods.Attribute.Qty",
            "Document.Order.TabularSection.Goods.StandardAttribute.LineNumber",
            "InformationRegister.Prices.Dimension.Item",
            "AccumulationRegister.Stock.Resource.Qty",
            "Task.Job.AddressingAttribute.Assignee",
            "Subsystem.Sales.Subsystem.Orders.Subsystem.Returns",
        ] {
            let args = json!({"at": "main:Role.Reader", "items": [{"object": object, "rights": {"View": true}}]});
            parse_form_resource_plan_operation("right.add", &args, 0, &fixture.binding)
                .unwrap_or_else(|error| panic!("{object}: {error:?}"));
        }
        for object in [
            "WebService.Exchange.Operation.Send",
            "HTTPService.Api.URLTemplate.Items.Method.Get",
            "IntegrationService.Bus.IntegrationServiceChannel.Out",
        ] {
            let args = json!({"at": "main:Role.Reader", "items": [{"object": object, "rights": {"Use": true}}]});
            parse_form_resource_plan_operation("right.add", &args, 0, &fixture.binding)
                .unwrap_or_else(|error| panic!("{object}: {error:?}"));
        }
        for (object, rights, path) in [
            (
                "Catalog.Items.Comand.Open",
                json!({"Read": true}),
                "ops[0].args.items[0].object",
            ),
            (
                "Configuration.X.Command.Y",
                json!({"View": true}),
                "ops[0].args.items[0].object",
            ),
            (
                "Constant.Rate.Command.Open",
                json!({"View": true}),
                "ops[0].args.items[0].object",
            ),
            (
                "Catalog.Items.Command.Open.Attribute.X",
                json!({"View": true}),
                "ops[0].args.items[0].object",
            ),
            (
                "Catalog.Items.Command.Open",
                json!({"Read": true}),
                "ops[0].args.items[0].rights.Read",
            ),
        ] {
            let args =
                json!({"at": "main:Role.Reader", "items": [{"object": object, "rights": rights}]});
            let error = parse_form_resource_plan_operation("right.add", &args, 0, &fixture.binding)
                .expect_err(object);
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{object}");
            assert_eq!(error.path(), Some(path), "{object}");
        }
    }

    #[test]
    fn role_target_refusals_precede_argument_contract_refusals() {
        let fixture = ApplySeamFixture::new();
        for (operation, args) in [
            ("role.create", json!({"values":{"name":42},"unknown":true})),
            ("right.add", json!({"items":"wrong","unknown":true})),
            ("right.set", json!({"values":{"right":42},"unknown":true})),
        ] {
            let error = parse_form_resource_plan_operation(operation, &args, 0, &fixture.binding)
                .expect_err("missing target must be refused before malformed arguments");
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{operation}");
            assert_eq!(error.path(), Some("ops[0].args.at"), "{operation}");
        }
    }

    #[test]
    fn role_create_contract_preserves_typed_names_and_rejects_unknown_args() {
        let fixture = ApplySeamFixture::new();
        let contract = crate::domain::role_operation::RoleOperation::Create.contract();
        for (input, expected) in [
            ("Reader", "Reader"),
            ("МенеджерПродаж", "МенеджерПродаж"),
            ("_Reader2", "_Reader2"),
            (" Reader ", "Reader"),
            ("\tЧитатель\r\n", "Читатель"),
            ("\u{a0}Ёж\u{a0}", "Ёж"),
        ] {
            let args = json!({"at":"main:Configuration","values":{"name":input}});
            assert!(
                contract.normalize(&args, "ops[0].args").is_ok(),
                "{input:?}"
            );
            let parsed =
                parse_form_resource_plan_operation("role.create", &args, 0, &fixture.binding)
                    .unwrap_or_else(|error| panic!("{input:?}: {error:?}"));
            let super::FormResourcePlanKind::CreateRole { name } = parsed.kind else {
                panic!("role.create must produce a typed role creation");
            };
            assert_eq!(name, expected);
        }
        for (args, expected_path) in [
            (json!({"values":{}}), "ops[0].args.values.name"),
            (json!({"values":{"name":" "}}), "ops[0].args.values.name"),
            (
                json!({"values":{"name":"Role.Reader"}}),
                "ops[0].args.values.name",
            ),
            (json!({"values":{"name":1}}), "ops[0].args.values.name"),
            (json!({"values":{"name":null}}), "ops[0].args.values.name"),
            (json!({"values":null}), "ops[0].args.values"),
            (json!({"values":[]}), "ops[0].args.values"),
            (json!({"values":"Reader"}), "ops[0].args.values"),
            (
                json!({"values":{"name":"Reader","synonym":"Ignored"}}),
                "ops[0].args.values.synonym",
            ),
            // Previously ignored top-level fields are intentionally closed by #1468.
            (
                json!({"values":{"name":"Reader"},"ignored":true}),
                "ops[0].args.ignored",
            ),
        ] {
            let mut args = args;
            args["at"] = json!("main:Configuration");
            let schema_error = contract.normalize(&args, "ops[0].args").unwrap_err();
            assert_eq!(schema_error.path, expected_path, "{args}");
            let error =
                parse_form_resource_plan_operation("role.create", &args, 0, &fixture.binding)
                    .expect_err("closed typed creation arguments");
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{args}");
            assert_eq!(error.path(), Some(expected_path), "{args}");
        }
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
        let stub = stub
            .replace("<Properties>", "<InternalInfo><xr:PropertyState><xr:Property>CommandInterface</xr:Property><xr:State>Extended</xr:State></xr:PropertyState></InternalInfo><Properties><ObjectBelonging>Adopted</ObjectBelonging><ExtendedConfigurationObject>66666666-6666-4666-8666-666666666666</ExtendedConfigurationObject>")
            .replace("<Synonym/>", "<Synonym><v8:item><v8:lang>en</v8:lang><v8:content>Sales original</v8:content></v8:item></Synonym>")
            .replace("<Comment/>", "<Comment>Keep&#13;this</Comment>");
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
        let expected = format!("\u{feff}{stub}")
            .replace(
                "<Content/>",
                "<Content><xr:Item xsi:type=\"xr:MDObjectRef\">Document.First</xr:Item></Content>",
            )
            .replace(
                "<ChildObjects/>",
                "<ChildObjects><Subsystem>Orders</Subsystem></ChildObjects>",
            );
        assert_eq!(
            text, expected,
            "only the requested lists change; all other source bytes survive"
        );
        assert_eq!(
            std::fs::read_to_string(fixture.source_dir().join("Subsystems/Sales.xml")).unwrap(),
            format!("\u{feff}{stub}"),
            "planning never writes the source"
        );
        let child = staged_text(&staged, "Subsystems/Sales/Subsystems/Orders.xml");
        assert!(child.contains("<Name>Orders</Name>"), "{child}");
    }

    #[test]
    fn subsystem_membership_batch_noop_keeps_the_exact_original_image() {
        for empty in ["<Content/>", "<Content></Content>", "<Content > </Content>"] {
            let fixture = ApplySeamFixture::new();
            std::fs::create_dir_all(fixture.source_dir().join("Subsystems")).unwrap();
            let original =
                super::subsystem_stub_xml("Sales", "2.20", "55555555-5555-4555-8555-555555555555")
                    .replace("<Content/>", empty)
                    .replace('\n', "\r\n");
            let path = fixture.source_dir().join("Subsystems/Sales.xml");
            std::fs::write(&path, original.as_bytes()).unwrap();
            let admission = fixture.admission();
            let staged = admission.staged_state().unwrap();
            let authority = admission
                .form_resource_planning_authority(&fixture.binding)
                .unwrap();
            let operations: Vec<_> = ["content.add", "content.remove"].into_iter().enumerate().map(|(index, op)| {
                let args = json!({"at": "main:Subsystem.Sales", "items": [{"object": "Document.First"}]});
                IndexedPlanOperation::new(index, parse_form_resource_plan_operation(op, &args, index, &fixture.binding).unwrap())
            }).collect();
            let (mut staged, _) = plan_form_resource_batch(staged, authority, &operations).unwrap();
            assert!(staged.planned_changes().is_empty(), "{empty}");
            assert_eq!(
                staged
                    .read(std::path::Path::new("Subsystems/Sales.xml"))
                    .unwrap()
                    .unwrap(),
                original.as_bytes()
            );
            assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes());
        }
    }

    #[test]
    fn subsystem_membership_restores_original_lists_across_family_batches() {
        let fixture = ApplySeamFixture::new();
        std::fs::create_dir_all(fixture.source_dir().join("Subsystems")).unwrap();
        let original =
            super::subsystem_stub_xml("Sales", "2.20", "55555555-5555-4555-8555-555555555555");
        std::fs::write(fixture.source_dir().join("Subsystems/Sales.xml"), &original).unwrap();
        let admission = fixture.admission();
        let mut staged = admission.staged_state().unwrap();
        let relative = std::path::Path::new("Subsystems/Sales.xml");
        for (index, op) in ["content.add", "content.remove"].into_iter().enumerate() {
            let authority = admission
                .form_resource_planning_authority(&fixture.binding)
                .unwrap();
            let args =
                json!({"at": "main:Subsystem.Sales", "items": [{"object": "Document.First"}]});
            let operation = IndexedPlanOperation::new(
                index,
                parse_form_resource_plan_operation(op, &args, index, &fixture.binding).unwrap(),
            );
            (staged, _) = plan_form_resource_batch(staged, authority, &[operation]).unwrap();
            if index == 0 {
                // An intervening family can edit the same descriptor. Restoration
                // must be scoped to membership and keep that staged property.
                let before = staged.read(relative).unwrap().unwrap();
                let after = String::from_utf8(before.clone())
                    .unwrap()
                    .replace("<Comment/>", "<Comment>Changed</Comment>");
                staged
                    .replace(relative, &before, after.into_bytes())
                    .unwrap();
            }
        }
        assert_eq!(
            staged.read(relative).unwrap().unwrap(),
            original
                .replace("<Comment/>", "<Comment>Changed</Comment>")
                .as_bytes()
        );
    }

    #[test]
    fn subsystem_membership_after_props_set_keeps_valid_namespace_bindings() {
        use super::super::metadata::{parse_metadata_plan_operation, plan_metadata_batch};
        let fixture = ApplySeamFixture::new();
        std::fs::create_dir_all(fixture.source_dir().join("Subsystems")).unwrap();
        let source = "<m:MetaDataObject xmlns:m=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:r=\"http://v8.1c.ru/8.3/xcf/readable\" version=\"2.20\"><m:Subsystem uuid=\"55555555-5555-4555-8555-555555555555\"><m:Properties><m:Name>Sales</m:Name><m:Comment/><m:Content/></m:Properties><m:ChildObjects/></m:Subsystem></m:MetaDataObject>";
        std::fs::write(fixture.source_dir().join("Subsystems/Sales.xml"), source).unwrap();
        let admission = fixture.admission();
        let mut staged = admission.staged_state().unwrap();
        for (index, op) in ["content.add", "content.remove"].into_iter().enumerate() {
            let authority = admission
                .form_resource_planning_authority(&fixture.binding)
                .unwrap();
            let args =
                json!({"at": "main:Subsystem.Sales", "items": [{"object": "Document.First"}]});
            let operation = IndexedPlanOperation::new(
                index * 2,
                parse_form_resource_plan_operation(op, &args, index * 2, &fixture.binding).unwrap(),
            );
            (staged, _) = plan_form_resource_batch(staged, authority, &[operation]).unwrap();
            if index == 0 {
                let authority = admission
                    .metadata_planning_authority(&fixture.binding)
                    .unwrap();
                let args = json!({"at": "main:Subsystem.Sales", "values": {"Comment": "Changed"}});
                let operation = IndexedPlanOperation::new(
                    1,
                    parse_metadata_plan_operation("props.set", &args, 1, &fixture.binding).unwrap(),
                );
                (staged, _) = plan_metadata_batch(staged, authority, &[operation]).unwrap();
            }
        }
        let actual = staged_text(&staged, "Subsystems/Sales.xml");
        let doc = roxmltree::Document::parse(actual.trim_start_matches('\u{feff}')).unwrap();
        assert!(doc.descendants().any(|n| n
            .has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "Comment"))
            && n.text() == Some("Changed")));
        assert!(!doc
            .descendants()
            .any(|n| n.has_tag_name(("http://v8.1c.ru/8.3/xcf/readable", "Item"))));
    }

    #[test]
    fn support_contracts_and_productive_parser_agree_on_closed_arguments() {
        use crate::domain::apply::OperationRegistry;
        let fixture = ApplySeamFixture::new();
        let capability = [
            (json!({"enabled": true}), true),
            (json!({"enabled": false}), true),
            (json!({"rule": "on"}), true),
            (json!({"rule": "off"}), true),
            (json!({"rule": "EditableSupportEnabled"}), true),
            (json!({"rule": "EditableSupportDisabled"}), true),
            (json!({"rule": "enabled"}), true),
            (json!({"rule": "disabled"}), true),
            (json!({}), false),
            (json!({"enabled": "true"}), false),
            (json!({"enabled": null}), false),
            (json!({"rule": true}), false),
            (json!({"rule": "editable"}), false),
            (json!({"enabled": true, "rule": "on"}), false),
            (json!({"enabled": true, "ignored": 1}), false),
        ];
        let rule = [
            (json!({"rule": "locked"}), true),
            (json!({"rule": "editable"}), true),
            (json!({"rule": "off-support"}), true),
            (json!({"rule": "NotEditable"}), true),
            (json!({"rule": "EditableWithSupport"}), true),
            (json!({"rule": "NotSupported"}), true),
            (json!({}), false),
            (json!({"rule": "on"}), false),
            (json!({"rule": true}), false),
            (json!({"rule": null}), false),
            (json!({"rule": ""}), false),
            (json!({"rule": "editable", "ignored": 1}), false),
        ];
        for (operation, at, cases) in [
            (
                "supportCapability.set",
                "main:Configuration",
                capability.as_slice(),
            ),
            ("supportRule.set", "main:Document.First", rule.as_slice()),
            (
                "supportRule.set",
                "main:Document.First.Form.Card",
                rule.as_slice(),
            ),
        ] {
            let descriptor = OperationRegistry::closed().lookup(operation).unwrap();
            let contract = descriptor
                .argument_contract()
                .expect("implemented support operation must publish its argument contract");
            assert_eq!(contract.schema["additionalProperties"], false);
            assert!(contract.example_args.get("at").is_none());
            let mut unknown_argument = contract.example_args.clone();
            unknown_argument["ignored"] = json!(true);
            for args in [
                json!({"values": null}),
                json!({"values": []}),
                json!({"values": "editable"}),
                unknown_argument,
            ] {
                let contract_error = contract.normalize(&args, "args").unwrap_err();
                let has_unknown_argument = args.get("ignored").is_some();
                if has_unknown_argument {
                    assert_eq!(contract_error.path, "args.ignored");
                }
                let mut args = args;
                args["at"] = json!(at);
                let error =
                    parse_form_resource_plan_operation(operation, &args, 3, &fixture.binding)
                        .unwrap_err();
                assert_eq!(
                    error.kind(),
                    ApplyPlanErrorKind::BadValue,
                    "{operation}: {args}"
                );
                if has_unknown_argument {
                    assert_eq!(error.path(), Some("ops[3].args.ignored"));
                }
            }
            for (values, accepted) in cases {
                let args = json!({"values": values});
                assert_eq!(
                    contract.normalize(&args, "args").is_ok(),
                    *accepted,
                    "contract: {operation} {args}"
                );
                let mut args = args;
                args["at"] = json!(at);
                let parsed =
                    parse_form_resource_plan_operation(operation, &args, 3, &fixture.binding);
                assert_eq!(
                    parsed.is_ok(),
                    *accepted,
                    "parser: {operation} {args}: {parsed:?}"
                );
                if let Err(error) = parsed {
                    assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
                }
            }
        }
    }

    #[test]
    fn support_aliases_select_the_exact_capability_and_object_rule() {
        use super::{FormResourcePlanKind, SupportCapability, SupportObjectRule};
        let fixture = ApplySeamFixture::new();
        for (values, expected) in [
            (json!({"enabled": true}), SupportCapability::On),
            (json!({"enabled": false}), SupportCapability::Off),
            (json!({"rule": "on"}), SupportCapability::On),
            (json!({"rule": "off"}), SupportCapability::Off),
            (
                json!({"rule": "EditableSupportEnabled"}),
                SupportCapability::On,
            ),
            (
                json!({"rule": "EditableSupportDisabled"}),
                SupportCapability::Off,
            ),
            (json!({"rule": "enabled"}), SupportCapability::On),
            (json!({"rule": "disabled"}), SupportCapability::Off),
        ] {
            let args = json!({"at": "main:Configuration", "values": values});
            let parsed = parse_form_resource_plan_operation(
                "supportCapability.set",
                &args,
                3,
                &fixture.binding,
            )
            .unwrap();
            match parsed.kind {
                FormResourcePlanKind::SupportCapability(actual) => {
                    assert_eq!(actual, expected, "{args}")
                }
                other => panic!("support capability selected {other:?}: {args}"),
            }
        }
        for (rule, expected) in [
            ("locked", SupportObjectRule::Locked),
            ("editable", SupportObjectRule::Editable),
            ("off-support", SupportObjectRule::OffSupport),
            ("NotEditable", SupportObjectRule::Locked),
            ("EditableWithSupport", SupportObjectRule::Editable),
            ("NotSupported", SupportObjectRule::OffSupport),
        ] {
            for (at, expected_target) in [
                ("main:Document.First", "Document.First"),
                ("main:Document.First.Form.Card", "Document.First.Form.Card"),
            ] {
                let args = json!({"at": at, "values": {"rule": rule}});
                let parsed = parse_form_resource_plan_operation(
                    "supportRule.set",
                    &args,
                    3,
                    &fixture.binding,
                )
                .unwrap();
                match parsed.kind {
                    FormResourcePlanKind::SupportRule {
                        target,
                        rule: actual,
                    } => {
                        assert_eq!(actual, expected, "{args}");
                        assert_eq!(target.as_str(), expected_target, "{args}");
                    }
                    other => panic!("support rule selected {other:?}: {args}"),
                }
            }
        }
    }

    #[test]
    fn support_target_refusals_precede_argument_contract_refusals() {
        let fixture = ApplySeamFixture::new();
        for operation in ["supportCapability.set", "supportRule.set"] {
            for target in [None, Some(json!(42)), Some(json!(null))] {
                let mut args = json!({"values": {"rule": true}, "unknown": true});
                if let Some(target) = target {
                    args["at"] = target;
                }
                let error =
                    parse_form_resource_plan_operation(operation, &args, 3, &fixture.binding)
                        .expect_err("an invalid target precedes malformed arguments");
                assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{operation}");
                assert_eq!(error.path(), Some("ops[3].args.at"), "{operation}");
                assert_eq!(
                    error.to_string(),
                    "`at` must be a logical address",
                    "{operation}"
                );
            }
        }
        for (operation, args, path) in [
            (
                "supportCapability.set",
                json!({"at":"main:Document.First", "values":{"enabled":"wrong"}}),
                "ops[3].args.values",
            ),
            (
                "supportRule.set",
                json!({"at":"main:Configuration", "values":{"rule":true}}),
                "ops[3].args.values.rule",
            ),
        ] {
            let error = parse_form_resource_plan_operation(operation, &args, 3, &fixture.binding)
                .expect_err("a logical target still checks its arguments before its owner kind");
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{operation}");
            assert_eq!(error.path(), Some(path), "{operation}: {error}");
        }
    }

    #[test]
    fn support_rule_form_requires_registered_identity_and_changes_only_its_record() {
        let fixture = ApplySeamFixture::new();
        let source = fixture.source_dir();
        let form_path = source.join("Documents/First/Forms/Card.xml");
        std::fs::create_dir_all(form_path.parent().unwrap()).unwrap();
        let form = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form uuid="bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"><Properties><Name>Card</Name></Properties></Form></MetaDataObject>"#;
        let owner_path = source.join("Documents/First.xml");
        let owner = std::fs::read_to_string(&owner_path).unwrap();
        let registered = owner.replacen("<ChildObjects>", "<ChildObjects><Form>Card</Form>", 1);
        let marker = "\u{feff}{6,0,1,dddddddd-dddd-4ddd-8ddd-dddddddddddd,0,eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee,\"1.0\",\"Vendor\",\"VendorConf\",3,1,0,aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa,aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa,1,0,11111111-1111-4111-8111-111111111111,11111111-1111-4111-8111-111111111111,0,0,bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb,bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb,0,0,cccccccc-cccc-4ccc-8ccc-cccccccccccc,cccccccc-cccc-4ccc-8ccc-cccccccccccc}";
        std::fs::create_dir_all(source.join("Ext")).unwrap();
        std::fs::write(source.join("Ext/ParentConfigurations.bin"), marker).unwrap();
        let operation = parse_form_resource_plan_operation(
            "supportRule.set",
            &json!({"at": "main:Document.First.Form.Card", "values": {"rule": "editable"}}),
            0,
            &fixture.binding,
        )
        .unwrap();
        for (owner_image, form_image, succeeds) in [
            (registered.clone(), Some(form.to_string()), true),
            (owner.clone(), Some(form.to_string()), false),
            (registered.clone(), None, false),
            (
                registered.clone(),
                Some(form.replace(
                    "<MetaDataObject ",
                    "<MetaDataObject uuid=\"11111111-1111-4111-8111-111111111111\" ",
                )),
                false,
            ),
            (
                registered.clone(),
                Some(form.replace("<Name>Card</Name>", "<Name>Other</Name>")),
                false,
            ),
            (
                registered.clone(),
                Some(form.replace("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb", "invalid")),
                false,
            ),
            (
                registered.clone(),
                Some(form.replace(" uuid=\"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb\"", "")),
                false,
            ),
        ] {
            std::fs::write(&owner_path, owner_image).unwrap();
            match form_image {
                Some(text) => std::fs::write(&form_path, text).unwrap(),
                None => std::fs::remove_file(&form_path).unwrap(),
            }
            let admission = fixture.admission();
            let result = plan_form_resource_batch(
                admission.staged_state().unwrap(),
                admission
                    .form_resource_planning_authority(&fixture.binding)
                    .unwrap(),
                &[IndexedPlanOperation::new(0, operation.clone())],
            );
            assert_eq!(result.is_ok(), succeeds, "{result:?}");
            if let Ok((staged, _)) = result {
                let expected = marker.replace("0,0,bbbbbbbb", "1,0,bbbbbbbb");
                assert_eq!(
                    staged_text(&staged, "Ext/ParentConfigurations.bin")
                        .trim_start_matches('\u{feff}'),
                    expected.trim_start_matches('\u{feff}')
                );
                let batch = admission
                    .prepare_with_effects(staged, Default::default())
                    .unwrap();
                fixture.actor().preview_prepared_apply(&batch).unwrap();
                let token = fixture
                    .actor()
                    .save_prepared_apply(
                        batch,
                        crate::domain::invocation::DomainResult::success("support preview"),
                    )
                    .unwrap();
                std::fs::write(
                    &form_path,
                    form.replace("<Name>Card</Name>", "<Name>Changed</Name>"),
                )
                .unwrap();
                let result = fixture
                    .actor()
                    .execute_saved_apply(
                        &token,
                        crate::domain::code_intelligence::ProviderDeadline::from_budget(
                            std::time::Duration::from_secs(5),
                        ),
                        &crate::domain::cancellation::CancellationToken::new(),
                        |mut result, publication| {
                            let error = publication
                                .expect_err("changed form descriptor must refuse publication");
                            result.ok = false;
                            result.summary = error.to_string();
                            result
                        },
                    )
                    .unwrap();
                assert!(!result.ok);
                assert!(result.summary.contains("Card.xml"), "{}", result.summary);
            }
            assert_eq!(
                std::fs::read_to_string(source.join("Ext/ParentConfigurations.bin")).unwrap(),
                marker
            );
        }
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

    /// Имя дочерней подсистемы задаёт адрес: аргумент, который его
    /// повторял, отказывает и в разборе планировщиком (#1353). Форму создаёт
    /// `form.add` на владельце: снятую `form.create` планировщик не знает.
    #[test]
    fn form_and_child_subsystem_names_come_only_from_the_address() {
        let fixture = ApplySeamFixture::new();
        assert!(matches!(
            parse_form_resource_plan_operation(
                "form.create",
                &json!({"at": "main:Catalog.Items.Form.Choice", "values": {"type": "ChoiceForm"}}),
                0,
                &fixture.binding,
            )
            .map(|operation| operation.kind),
            Ok(super::FormResourcePlanKind::Unsupported)
        ));
        for (op, args, path) in [
            (
                "childSubsystem.remove",
                json!({"at": "main:Subsystem.Sales", "items": [{"name": "Orders"}]}),
                "ops[0].args.items",
            ),
            (
                "childSubsystem.remove",
                json!({"at": "main:Subsystem.Sales.Subsystem.Orders", "values": {"name": "Orders"}}),
                "ops[0].args.values",
            ),
            (
                "childSubsystem.remove",
                json!({"at": "main:Subsystem.Sales"}),
                "ops[0].args.at",
            ),
        ] {
            let error = parse_form_resource_plan_operation(op, &args, 0, &fixture.binding)
                .expect_err("a name outside the address is refused");
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{op} {path}");
            assert_eq!(error.path(), Some(path), "{op}");
        }
        assert!(parse_form_resource_plan_operation(
            "childSubsystem.remove",
            &json!({"at": "main:Subsystem.Sales.Subsystem.Orders"}),
            0,
            &fixture.binding,
        )
        .is_ok());
    }

    /// `command.add` принимает ровно ключи, которые писатель кладёт
    /// в `<Command>`, и передаёт их писателю; остальное отказывает
    /// `bad_value` до записи, а не теряется молча (#1439).
    #[test]
    fn form_command_add_accepts_only_the_keys_its_writer_writes() {
        let fixture = ApplySeamFixture::new();
        let at = "main:Catalog.Items.Form.Item";
        let item = json!({
            "name": "Refresh",
            "title": "Refresh",
            "tooltip": "Refresh the list",
            "shortcut": "F5",
            "picture": "StdPicture.Refresh",
            "representation": "TextPicture",
        });
        let plan = parse_form_resource_plan_operation(
            "command.add",
            &json!({"at": at, "items": [item.clone()]}),
            0,
            &fixture.binding,
        )
        .expect("every published key is accepted")
        .kind;
        let super::FormResourcePlanKind::FormEdit { definitions, .. } = plan else {
            panic!("command.add plans a form edit");
        };
        let mut expected = item.as_object().unwrap().clone();
        expected.insert("action".into(), json!("Refresh"));
        let [super::FormEditStep::Definition(definition)] = definitions.as_slice() else {
            panic!("expected one command definition");
        };
        assert_eq!(definition, &json!({"commands": [expected]}));

        for (field, value) in [
            ("use", json!(false)),
            ("functionalOptions", json!(["FunctionalOption.X"])),
            ("currentRowUse", json!("DontUse")),
            ("modifiesSavedData", json!(true)),
            ("table", json!("Список")),
            ("loadTransparent", json!(false)),
            ("representation", json!("PictureAndText")),
            ("representation", json!("text")),
            ("picture", json!({"src": "StdPicture.Refresh"})),
            ("picture", json!("abs:picture.png")),
            ("title", json!({"ru": "Обновить"})),
            ("title", json!("")),
            ("callType", json!("Before")),
            (
                "actions",
                json!([{"callType": "Before", "handler": "RefreshBefore"}]),
            ),
            ("tooltip", json!({"ru": "Обновить"})),
            ("tooltip", json!("")),
            ("shortcut", json!("")),
        ] {
            let error = parse_form_resource_plan_operation(
                "command.add",
                &json!({"at": at, "items": [{"name": "Refresh", field: value}]}),
                0,
                &fixture.binding,
            )
            .expect_err("a key the writer does not write is refused");
            assert_eq!(
                error.kind(),
                ApplyPlanErrorKind::BadValue,
                "{field} {value}"
            );
            let path = format!("ops[0].args.items[0].{field}");
            assert_eq!(error.path(), Some(path.as_str()), "{field} {value}");
        }
    }

    #[test]
    fn form_operations_refuse_a_root_target_and_name_the_argument() {
        let fixture = ApplySeamFixture::new();
        let error = parse_form_resource_plan_operation(
            "form.add",
            &json!({"at": "main:Configuration", "items": [{"name": "Форма"}]}),
            0,
            &fixture.binding,
        )
        .unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
        assert_eq!(error.path(), Some("ops[0].args.at"));
    }
}
