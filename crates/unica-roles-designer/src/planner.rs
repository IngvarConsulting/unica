//! Role operation decoding and planning over an existing source proposal.
use serde_json::{Map, Value};
use std::path::PathBuf;
use unica_address::address::{member_address, NodeKind, QualifiedAddress};
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_roles::operations::RoleOperation;
use unica_source_contract::events::{DomainEvent, DomainEventKind};
use unica_source_contract::proposal::input::{bad, creation_name, required_values};
use unica_source_contract::proposal::{
    ApplyPlanError, ApplyPlanErrorKind, ApplyProposal, ApplySourceAuthority, ProvisionalApplyEffect,
};
use unica_source_designer::layout::attached_resource_relative;
/// Новая запись прав роли: объект и его начальные права.
#[derive(Debug, Clone)]
pub struct RightEntry {
    object: String,
    rights: Vec<(String, bool)>,
}
#[derive(Debug, Clone)]
pub enum RolePlan {
    /// `role.create`: descriptor, empty rights document and registration.
    CreateRole { name: String },
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
}

/// Вид владельца записи прав: вид метаданных или корень конфигурации.
/// Имя записи `<Вид>_<Имя>` делится по первому `_`, и вид без `_` отличает
/// каноническое имя от короткого псевдонима чтения.
fn role_right_owner_kind(kind: &str) -> bool {
    kind == NodeKind::Configuration.as_str()
        || unica_source_designer::layout::metadata_kind(kind).is_some()
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
pub fn parse_role_operation(
    operation: RoleOperation,
    args: &Map<String, Value>,
    op_index: usize,
    source_set_name: &str,
) -> Result<RolePlan, ApplyPlanError> {
    let target =
        unica_source_contract::proposal::input::qualified_target(args, op_index, source_set_name)?;
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
            let name = creation_name(
                &target,
                NodeKind::Role,
                values,
                op_index,
                unica_metadata::canonical_creation_identifier,
            )?;
            Ok(RolePlan::CreateRole { name })
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
                let object = unica_source_contract::proposal::input::required_string(
                    item, "object", op_index, &location,
                )?
                .to_string();
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
                if nested && !unica_roles::nested_right_shape_is_modelled(&object) {
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
                        unica_roles::validate_nested_right(&object, right)
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
            Ok(RolePlan::RightAdd {
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
            let right = unica_source_contract::proposal::input::required_string(
                values, "right", op_index, "values",
            )?
            .to_string();
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
            Ok(RolePlan::RightSet {
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
/// Ensures `<object><name>X</name></object>` exists in the rights document.
/// Читает права роли, применяет правку и стадирует результат с проверкой
/// документа до и после. Неизменный документ не пишется.
fn stage_role_rights(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
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
    let (bom, body) = crate::writer::decode_role_xml(&preimage).map_err(invalid)?;
    crate::writer::validate_role_rights_document(&body, crate::writer::RoleValueScope::SourceImage)
        .map_err(invalid)?;
    let body = edit(body)?;
    crate::writer::validate_role_rights_document(&body, crate::writer::RoleValueScope::SourceImage)
        .map_err(|message| {
            ApplyPlanError::new(ApplyPlanErrorKind::Postcondition, message)
                .at_path(format!("ops[{op_index}].args"))
        })?;
    let postimage = crate::writer::encode_role_xml(bom, &body);
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
    let edit = unica_roles::RoleEditOperation {
        object_name: object.to_string(),
        right: right.to_string(),
        value,
    };
    crate::writer::apply_role_edit_operation(body, &edit, op_index)
        .map(|(updated, _effect)| updated)
        .map_err(|message| ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message))
}
/// Есть ли запись прав на объект в стадированном документе прав роли.
/// `None` — документ не читается.
pub fn staged_rights_entry_exists(bytes: &[u8], object: &str) -> Option<bool> {
    let (_, body) = crate::writer::decode_role_xml(bytes).ok()?;
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
        unica_xml_format::text::escape_xml_text(object)
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
    let escaped = unica_xml_format::text::escape_xml_text(condition);
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

pub fn stage_role_plan(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    plan: &RolePlan,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    match plan {
        RolePlan::CreateRole { name } => {
            let name_path = format!("ops[{op_index}].args.values.name");
            let uuid = unica_source_designer::registration::seeded_uuid(&format!(
                "{}\0Role.{name}",
                authority.source_set_name()
            ));
            let lang = unica_source_designer::language::staged_text_language(
                staged,
                authority.source_kind(),
                op_index,
            )?;
            let descriptor = crate::descriptor::role_metadata_xml(
                name,
                name,
                "",
                authority.expected_format(),
                &uuid,
                &lang,
            );
            let mut touched = Vec::new();
            let descriptor_relative = PathBuf::from("Roles").join(format!("{name}.xml"));
            unica_source_designer::registration::stage_new_file(
                staged,
                &descriptor_relative,
                descriptor.as_bytes(),
                &name_path,
            )?;
            touched.push(descriptor_relative);
            let rights_relative = PathBuf::from("Roles").join(name).join("Ext/Rights.xml");
            unica_source_designer::registration::stage_new_file(
                staged,
                &rights_relative,
                minimal_rights_xml().as_bytes(),
                &name_path,
            )?;
            touched.push(rights_relative);
            touched.push(unica_source_designer::registration::register_child(
                staged, "Role", name, op_index,
            )?);
            provisional.push(ProvisionalApplyEffect::spanning(
                touched,
                DomainEvent::new(DomainEventKind::RoleChanged, format!("Role.{name}")),
                op_index,
            ));
        }
        RolePlan::RightSet {
            role,
            role_at,
            object,
            right,
            value,
            rls,
        } => {
            let value_path = format!("ops[{op_index}].args.values");
            stage_role_rights(
                staged,
                authority,
                role,
                op_index,
                provisional,
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
                        body = set_rights_restriction(&body, object, right, condition).map_err(
                            |message| {
                                ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message)
                                    .at_path(format!("{value_path}.rls"))
                            },
                        )?;
                    }
                    Ok(body)
                },
            )?;
        }
        RolePlan::RightAdd {
            role,
            role_at,
            entries,
        } => {
            stage_role_rights(
                staged,
                authority,
                role,
                op_index,
                provisional,
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
                        body = ensure_rights_object(&body, &entry.object).map_err(|message| {
                            ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message)
                                .at_path(location.clone())
                        })?;
                        for (right, granted) in &entry.rights {
                            body = set_role_right(&body, &entry.object, right, *granted, op_index)
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
    }
    Ok(())
}
