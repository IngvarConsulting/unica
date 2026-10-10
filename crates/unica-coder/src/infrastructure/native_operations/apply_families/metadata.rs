use super::validate_platform_xml_binding;
use crate::application::metadata::{parse_metadata_request, MetadataOperation, MetadataRequest};
use crate::domain::address::{NodeKind, QualifiedAddress};
use crate::domain::events::{DomainEvent, DomainEventKind};
use crate::domain::metadata::{MetaDiagnosticCode, MetaEditOperation, MetadataKind};
use crate::domain::project_sources::SourceSetKind;
use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use crate::infrastructure::logical_event_source::metadata_descriptor_relative;
use crate::infrastructure::metadata_kinds::metadata_layout;
use crate::infrastructure::native_operations::apply::{
    empty_apply_family_batch, hidden_apply_family_unimplemented, ApplyPlanError,
    ApplyPlanErrorKind, ApplyStagedState,
};
use crate::infrastructure::native_operations::apply_families::request::{
    IndexedPlanOperation, ProvisionalApplyEffect,
};
use crate::infrastructure::native_operations::meta::{
    apply_typed_operations_to_image_with_seed, meta_default_text_language,
    meta_edit_object_identity, meta_mltext_property_replacement,
    typed_operations_write_multilingual_text, EXTERNAL_SOURCE_TEXT_LANGUAGE,
};
use crate::infrastructure::workspace_actor::{MetadataApplyAuthority, ProviderRootBinding};
use serde_json::{json, Map, Value};
use sha2::Digest;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
enum MetadataPlanKind {
    Edit {
        target: MetadataAddress,
        operation: MetaEditOperation,
    },
    Create {
        kind: MetadataKind,
        name: String,
    },
    Remove {
        target: MetadataAddress,
        kind: MetadataKind,
        name: String,
        /// Remove even when other source files still refer to the object;
        /// the plan then carries a typed warning naming those files.
        force: bool,
    },
    HelpCreate {
        target: MetadataAddress,
        name: String,
        lang: String,
    },
    /// Templates live in their own descriptors under `Templates/`, registered
    /// on the owner by name; the typed collection writer does not know that.
    TemplateAdd {
        owner: MetadataAddress,
        items: Vec<(String, crate::domain::metadata::MetaTemplateKind)>,
    },
    TemplateSet {
        owner: MetadataAddress,
        name: String,
        values: Map<String, Value>,
    },
    TemplateRemove {
        owner: MetadataAddress,
        name: String,
    },
    /// `props.set` on a role or subsystem: descriptor kinds outside the typed
    /// metadata writer, edited through their own small property sets.
    SimpleProps {
        kind: NodeKind,
        name: String,
        relative: PathBuf,
        values: Map<String, Value>,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct MetadataPlanOperation {
    kind: MetadataPlanKind,
}

pub(crate) fn parse_metadata_plan_operation(
    operation: &str,
    args: &Value,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanOperation, ApplyPlanError> {
    validate_platform_xml_binding(binding, op_index)?;
    let object = args.as_object().ok_or_else(|| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "operation args must be an object",
        )
        .at_path(format!("ops[{op_index}].args"))
    })?;
    let kind = match operation {
        "props.set" => parse_props_set(object, op_index, binding)?,
        "attribute.add" => parse_attribute_add(object, op_index, binding)?,
        "attribute.set" => parse_attribute_set(object, op_index, binding)?,
        "attribute.remove" => parse_attribute_remove(object, op_index, binding)?,
        "tabularSection.add" => {
            parse_member_add(operation, "tabularSections", object, op_index, binding)?
        }
        "tabularSection.set" => {
            parse_member_set(operation, "tabularSections", object, op_index, binding)?
        }
        "tabularSection.remove" => {
            parse_member_remove(operation, "tabularSections", object, op_index, binding)?
        }
        "dimension.add" => parse_member_add(operation, "dimensions", object, op_index, binding)?,
        "dimension.set" => parse_member_set(operation, "dimensions", object, op_index, binding)?,
        "dimension.remove" => {
            parse_member_remove(operation, "dimensions", object, op_index, binding)?
        }
        "resource.add" => parse_member_add(operation, "resources", object, op_index, binding)?,
        "resource.set" => parse_member_set(operation, "resources", object, op_index, binding)?,
        "resource.remove" => {
            parse_member_remove(operation, "resources", object, op_index, binding)?
        }
        "enumValue.add" => parse_member_add(operation, "enumValues", object, op_index, binding)?,
        "enumValue.set" => parse_member_set(operation, "enumValues", object, op_index, binding)?,
        "enumValue.remove" => {
            parse_member_remove(operation, "enumValues", object, op_index, binding)?
        }
        "column.add" => parse_member_add(operation, "columns", object, op_index, binding)?,
        "column.set" => parse_member_set(operation, "columns", object, op_index, binding)?,
        "column.remove" => parse_member_remove(operation, "columns", object, op_index, binding)?,
        "template.add" => parse_template_add(object, op_index, binding)?,
        "template.set" => parse_template_set(object, op_index, binding)?,
        "template.remove" => parse_template_remove(object, op_index, binding)?,
        "command.add" => parse_member_add(operation, "commands", object, op_index, binding)?,
        "command.set" => parse_member_set(operation, "commands", object, op_index, binding)?,
        "command.remove" => parse_member_remove(operation, "commands", object, op_index, binding)?,
        "predefinedItem.add" => parse_predefined(operation, "add", object, op_index, binding)?,
        "predefinedItem.set" => parse_predefined(operation, "update", object, op_index, binding)?,
        "predefinedItem.remove" => {
            parse_predefined(operation, "remove", object, op_index, binding)?
        }
        "relation.add" => parse_relation(operation, "add", object, op_index, binding)?,
        "relation.replace" => parse_relation(operation, "replace", object, op_index, binding)?,
        "relation.remove" => parse_relation(operation, "remove", object, op_index, binding)?,
        "object.create" => parse_object_create(object, op_index, binding)?,
        "object.remove" => parse_object_remove(object, op_index, binding)?,
        "help.create" => parse_help_create(object, op_index, binding)?,
        _ => return Err(hidden_apply_family_unimplemented(op_index)),
    };
    Ok(MetadataPlanOperation { kind })
}

fn reject_unknown_args(
    operation: &str,
    args: &Map<String, Value>,
    allowed: &[&str],
    op_index: usize,
) -> Result<(), ApplyPlanError> {
    if let Some(field) = args.keys().find(|field| !allowed.contains(&field.as_str())) {
        // The refusal names the expected skeleton so the caller's first retry
        // does not have to discover the argument shape by another refusal.
        let expected = crate::domain::apply::OperationRegistry::closed()
            .lookup(operation)
            .map(|descriptor| format!("; `{operation}` expects `{}`", descriptor.skeleton_key()))
            .unwrap_or_default();
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!("operation does not accept argument `{field}`{expected}"),
        )
        .at_path(format!("ops[{op_index}].args.{field}")));
    }
    Ok(())
}

fn required_object<'a>(
    args: &'a Map<String, Value>,
    name: &str,
    op_index: usize,
) -> Result<&'a Map<String, Value>, ApplyPlanError> {
    args.get(name).and_then(Value::as_object).ok_or_else(|| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!("`{name}` must be an object"),
        )
        .at_path(format!("ops[{op_index}].args.{name}"))
    })
}

fn required_array<'a>(
    args: &'a Map<String, Value>,
    name: &str,
    op_index: usize,
) -> Result<&'a [Value], ApplyPlanError> {
    args.get(name)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::BadValue,
                format!("`{name}` must be an array"),
            )
            .at_path(format!("ops[{op_index}].args.{name}"))
        })
}

fn parse_template_add(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    let normalized = crate::domain::operation_contract::OperationContract::mxl("template.add")
        .expect("template contract")
        .normalize(
            &Value::Object(args.clone()),
            &format!("ops[{op_index}].args"),
        )
        .map_err(|error| {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, error.message).at_path(error.path)
        })?;
    let args = normalized.as_object().expect("object contract");
    let target = qualified_target(args, op_index, binding)?;
    let (owner, _) = metadata_owner(&target, op_index)?;
    let items = required_array(args, "items", op_index)?;
    let mut planned = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let location = format!("ops[{op_index}].args.items[{index}]");
        let item = item.as_object().ok_or_else(|| {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, "each item must be an object")
                .at_path(location.clone())
        })?;
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                ApplyPlanError::new(ApplyPlanErrorKind::BadValue, "`name` is required")
                    .at_path(format!("{location}.name"))
            })?;
        if !crate::infrastructure::native_operations::common::is_1c_identifier(name) {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::BadValue,
                format!("`{name}` is not a valid 1C identifier"),
            )
            .at_path(format!("{location}.name")));
        }
        let kind = match item.get("templateType").and_then(Value::as_str) {
            None => crate::domain::metadata::MetaTemplateKind::SpreadsheetDocument,
            Some(raw) => {
                crate::domain::metadata::MetaTemplateKind::parse(raw).map_err(|diagnostic| {
                    ApplyPlanError::new(ApplyPlanErrorKind::BadValue, diagnostic.message)
                        .at_path(format!("{location}.templateType"))
                })?
            }
        };
        planned.push((name.to_string(), kind));
    }
    if planned.is_empty() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "`items` must list at least one template",
        )
        .at_path(format!("ops[{op_index}].args.items")));
    }
    Ok(MetadataPlanKind::TemplateAdd {
        owner,
        items: planned,
    })
}

fn template_name_from(
    target: &QualifiedAddress,
    values: Option<&Map<String, Value>>,
    op_index: usize,
) -> Result<(MetadataAddress, String), ApplyPlanError> {
    // `Owner.Name.Template.T` names the template in the address; `values.name`
    // is the other accepted spelling.
    match target.segments() {
        [owner, template] if template.kind() == NodeKind::Template => {
            let owner_address = MetadataAddress::parse(
                PLATFORM_XML_8_3_27_FORMAT_2_20,
                &format!(
                    "{}.{}",
                    owner.kind().as_str(),
                    owner.name().unwrap_or_default()
                ),
            )
            .map_err(|error| {
                ApplyPlanError::new(ApplyPlanErrorKind::BadValue, error.to_string())
                    .at_path(format!("ops[{op_index}].args.at"))
            })?;
            let name = template
                .name()
                .ok_or_else(|| {
                    ApplyPlanError::new(ApplyPlanErrorKind::BadValue, "the template must be named")
                        .at_path(format!("ops[{op_index}].args.at"))
                })?
                .to_string();
            Ok((owner_address, name))
        }
        _ => {
            let (owner, _) = metadata_owner(target, op_index)?;
            let name = values
                .and_then(|values| values.get("name"))
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| {
                    ApplyPlanError::new(
                        ApplyPlanErrorKind::BadValue,
                        "name the template in the address (`Owner.Name.Template.T`) or in `values.name`",
                    )
                    .at_path(format!("ops[{op_index}].args.values.name"))
                })?
                .to_string();
            Ok((owner, name))
        }
    }
}

fn parse_template_set(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    reject_unknown_args("template.set", args, &["at", "values"], op_index)?;
    let target = qualified_target(args, op_index, binding)?;
    let values = required_object(args, "values", op_index)?.clone();
    let (owner, name) = template_name_from(&target, Some(&values), op_index)?;
    if let Some(field) = values
        .keys()
        .find(|field| !["name", "synonym", "comment", "templateType"].contains(&field.as_str()))
    {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!(
                "template.set values accept `synonym`, `comment` and `templateType`, not `{field}`"
            ),
        )
        .at_path(format!("ops[{op_index}].args.values.{field}")));
    }
    Ok(MetadataPlanKind::TemplateSet {
        owner,
        name,
        values,
    })
}

fn parse_template_remove(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    reject_unknown_args("template.remove", args, &["at", "values"], op_index)?;
    let target = qualified_target(args, op_index, binding)?;
    let values = args.get("values").and_then(Value::as_object);
    let (owner, name) = template_name_from(&target, values, op_index)?;
    Ok(MetadataPlanKind::TemplateRemove { owner, name })
}

fn parse_object_create(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    reject_unknown_args("object.create", args, &["at", "values"], op_index)?;
    let target = qualified_target(args, op_index, binding)?;
    if !matches!(target.segments(), [root] if root.kind() == NodeKind::Configuration) {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "object.create targets the configuration root; name the new object in `values`",
        )
        .at_path(format!("ops[{op_index}].args.at")));
    }
    let values = required_object(args, "values", op_index)?;
    if let Some(field) = values
        .keys()
        .find(|field| !["kind", "name"].contains(&field.as_str()))
    {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!("object.create values accept only `kind` and `name`, not `{field}`"),
        )
        .at_path(format!("ops[{op_index}].args.values.{field}")));
    }
    let kind_text = values.get("kind").and_then(Value::as_str).ok_or_else(|| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "object.create requires `values.kind`, a top-level metadata kind",
        )
        .at_path(format!("ops[{op_index}].args.values.kind"))
    })?;
    let kind = MetadataKind::parse(kind_text).map_err(|diagnostic| {
        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, diagnostic.message)
            .at_path(format!("ops[{op_index}].args.values.kind"))
    })?;
    let name = values
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::BadValue,
                "object.create requires `values.name`, a 1C identifier",
            )
            .at_path(format!("ops[{op_index}].args.values.name"))
        })?;
    Ok(MetadataPlanKind::Create {
        kind,
        name: name.to_string(),
    })
}

fn parse_object_remove(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    reject_unknown_args("object.remove", args, &["at", "force"], op_index)?;
    let target = qualified_target(args, op_index, binding)?;
    let (address, kind) = metadata_owner(&target, op_index)?;
    let name = target.segments()[0]
        .name()
        .expect("metadata_owner proved the segment is named")
        .to_string();
    let force = match args.get("force") {
        None => false,
        Some(Value::Bool(force)) => *force,
        Some(_) => {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::BadValue,
                "`force` must be a boolean",
            )
            .at_path(format!("ops[{op_index}].args.force")))
        }
    };
    Ok(MetadataPlanKind::Remove {
        target: address,
        kind,
        name,
        force,
    })
}

fn parse_help_create(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    reject_unknown_args("help.create", args, &["at", "values"], op_index)?;
    let target = qualified_target(args, op_index, binding)?;
    let (address, _) = metadata_owner(&target, op_index)?;
    let name = target.segments()[0]
        .name()
        .expect("metadata_owner proved the segment is named")
        .to_string();
    let values = required_object(args, "values", op_index)?;
    if let Some(field) = values.keys().find(|field| field.as_str() != "lang") {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!("help.create values accept only `lang`, not `{field}`"),
        )
        .at_path(format!("ops[{op_index}].args.values.{field}")));
    }
    let lang = values
        .get("lang")
        .and_then(Value::as_str)
        .filter(|lang| !lang.is_empty() && lang.chars().all(|ch| ch.is_ascii_alphanumeric()))
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::BadValue,
                "help.create requires `values.lang`, a language code such as `ru`",
            )
            .at_path(format!("ops[{op_index}].args.values.lang"))
        })?;
    Ok(MetadataPlanKind::HelpCreate {
        target: address,
        name,
        lang: lang.to_string(),
    })
}

pub(super) fn qualified_target(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<QualifiedAddress, ApplyPlanError> {
    let raw = args.get("at").and_then(Value::as_str).ok_or_else(|| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "`at` must be a logical address",
        )
        .at_path(format!("ops[{op_index}].args.at"))
    })?;
    let target =
        QualifiedAddress::resolve_input(raw, &[binding.source_set_name()]).map_err(|error| {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, error.to_string())
                .at_path(format!("ops[{op_index}].args.at"))
        })?;
    if target.source_set() != binding.source_set_name() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "operation target belongs to another admitted source set",
        )
        .at_path(format!("ops[{op_index}].args.at")));
    }
    Ok(target)
}

fn metadata_owner(
    target: &QualifiedAddress,
    op_index: usize,
) -> Result<(MetadataAddress, MetadataKind), ApplyPlanError> {
    let [owner] = target.segments() else {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "operation target must identify one metadata object",
        )
        .at_path(format!("ops[{op_index}].args.at")));
    };
    let name = owner.name().ok_or_else(|| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "operation target must identify a named metadata object",
        )
        .at_path(format!("ops[{op_index}].args.at"))
    })?;
    let kind = MetadataKind::parse(owner.kind().as_str()).map_err(|diagnostic| {
        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, diagnostic.message)
            .at_path(format!("ops[{op_index}].args.at"))
    })?;
    let address = MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!("{}.{name}", owner.kind().as_str()),
    )
    .map_err(|error| {
        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, error.to_string())
            .at_path(format!("ops[{op_index}].args.at"))
    })?;
    Ok((address, kind))
}

fn attribute_owner_and_name(
    target: &QualifiedAddress,
    op_index: usize,
) -> Result<(MetadataAddress, MetadataKind, String, Option<String>), ApplyPlanError> {
    // Two shapes are addressable: `Owner.Attribute.X` and the tabular-section
    // member `Owner.TabularSection.TS.Attribute.X`.
    let (owner, section, attribute) = match target.segments() {
        [owner, attribute] => (owner, None, attribute),
        [owner, section, attribute] if section.kind() == NodeKind::TabularSection => {
            let section_name = section.name().ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::BadValue,
                    "tabular-section scope must be named",
                )
                .at_path(format!("ops[{op_index}].args.at"))
            })?;
            (owner, Some(section_name.to_string()), attribute)
        }
        _ => {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::BadValue,
                "attribute target must identify one exact Attribute leaf",
            )
            .at_path(format!("ops[{op_index}].args.at")))
        }
    };
    if attribute.kind() != NodeKind::Attribute {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "attribute target must end in an Attribute leaf",
        )
        .at_path(format!("ops[{op_index}].args.at")));
    }
    let attribute_name = attribute.name().ok_or_else(|| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "attribute target must have a name",
        )
        .at_path(format!("ops[{op_index}].args.at"))
    })?;
    let owner_target = QualifiedAddress {
        source_set: target.source_set().to_string(),
        segments: vec![owner.clone()],
    };
    let (owner, kind) = metadata_owner(&owner_target, op_index)?;
    Ok((owner, kind, attribute_name.to_string(), section))
}

fn parse_legacy_edit(
    source_set: &str,
    target: MetadataAddress,
    kind: MetadataKind,
    legacy_operation: Value,
    canonical_field: impl Fn(&str) -> String,
    _op_index: usize,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    let input = json!({
        "sourceSet": source_set,
        "metadataPath": target.as_str(),
        "operations": [legacy_operation],
        "dryRun": true
    });
    let request = parse_metadata_request(
        MetadataOperation::Edit,
        input
            .as_object()
            .expect("metadata edit wrapper is an object"),
    )
    .map_err(|failure| {
        let diagnostic = failure
            .diagnostics
            .into_iter()
            .next()
            .expect("metadata parser failures contain a diagnostic");
        let field = diagnostic.field.as_deref().unwrap_or("args");
        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, diagnostic.message)
            .at_path(canonical_field(field))
    })?;
    let MetadataRequest::Edit(request) = request else {
        unreachable!("edit parser returns an edit request")
    };
    let operation = request
        .operations
        .into_iter()
        .next()
        .expect("edit wrapper contains one operation");
    // Keep an explicit owner-kind proof next to the reused parser: this also
    // guards future parser refactors from accepting a different target kind.
    let _ = kind;
    Ok(MetadataPlanKind::Edit { target, operation })
}

fn parse_props_set(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    reject_unknown_args("props.set", args, &["at", "values"], op_index)?;
    let target = qualified_target(args, op_index, binding)?;
    if let [only] = target.segments() {
        if matches!(only.kind(), NodeKind::Role | NodeKind::Subsystem) {
            let name = only
                .name()
                .ok_or_else(|| {
                    ApplyPlanError::new(ApplyPlanErrorKind::BadValue, "the target must be named")
                        .at_path(format!("ops[{op_index}].args.at"))
                })?
                .to_string();
            let relative = if only.kind() == NodeKind::Role {
                PathBuf::from("Roles").join(format!("{name}.xml"))
            } else {
                PathBuf::from("Subsystems").join(format!("{name}.xml"))
            };
            let values = required_object(args, "values", op_index)?.clone();
            return Ok(MetadataPlanKind::SimpleProps {
                kind: only.kind(),
                name,
                relative,
                values,
            });
        }
    }
    let (owner, kind) = metadata_owner(&target, op_index)?;
    let values = required_object(args, "values", op_index)?.clone();
    parse_legacy_edit(
        binding.source_set_name(),
        owner,
        kind,
        json!({"op": "setProperties", "values": values}),
        |field| format!("ops[{op_index}].args.{field}"),
        op_index,
    )
}

fn parse_attribute_add(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    reject_unknown_args("attribute.add", args, &["at", "items", "scope"], op_index)?;
    let target = qualified_target(args, op_index, binding)?;
    let (owner, kind) = metadata_owner(&target, op_index)?;
    let items = required_array(args, "items", op_index)?.to_vec();
    let mut legacy = json!({"op": "add", "collection": "attributes", "elements": items});
    if let Some(scope) = args.get("scope") {
        legacy["scope"] = scope.clone();
    }
    parse_legacy_edit(
        binding.source_set_name(),
        owner,
        kind,
        legacy,
        |field| {
            format!(
                "ops[{op_index}].args.{}",
                field.replacen("elements", "items", 1)
            )
        },
        op_index,
    )
}

fn parse_attribute_set(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    reject_unknown_args("attribute.set", args, &["at", "values"], op_index)?;
    let target = qualified_target(args, op_index, binding)?;
    let (owner, kind, name, scope) = attribute_owner_and_name(&target, op_index)?;
    let mut values = required_object(args, "values", op_index)?.clone();
    if values
        .insert("name".to_string(), Value::String(name))
        .is_some()
    {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "attribute.set values must not repeat the target name",
        )
        .at_path(format!("ops[{op_index}].args.values.name")));
    }
    let mut legacy = json!({"op": "update", "collection": "attributes", "elements": [values]});
    if let Some(section) = scope {
        legacy["scope"] = json!({"tabularSection": section});
    }
    parse_legacy_edit(
        binding.source_set_name(),
        owner,
        kind,
        legacy,
        |field| {
            let field = field
                .strip_prefix("elements[0].")
                .unwrap_or(field)
                .to_string();
            format!("ops[{op_index}].args.values.{field}")
        },
        op_index,
    )
}

fn parse_attribute_remove(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    reject_unknown_args("attribute.remove", args, &["at"], op_index)?;
    let target = qualified_target(args, op_index, binding)?;
    let (owner, kind, name, scope) = attribute_owner_and_name(&target, op_index)?;
    let mut legacy = json!({"op": "remove", "collection": "attributes", "names": [name]});
    if let Some(section) = scope {
        legacy["scope"] = json!({"tabularSection": section});
    }
    parse_legacy_edit(
        binding.source_set_name(),
        owner,
        kind,
        legacy,
        |field| format!("ops[{op_index}].args.{field}"),
        op_index,
    )
}

/// One member-collection add: `at` names the owner, `items` carry the new
/// elements exactly as the typed metadata contract defines them. Attributes
/// additionally accept `scope` for a tabular section; other collections have
/// no nested scope in the platform model.
fn parse_member_add(
    operation: &str,
    collection: &str,
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    reject_unknown_args(operation, args, &["at", "items"], op_index)?;
    let target = qualified_target(args, op_index, binding)?;
    let (owner, kind) = metadata_owner(&target, op_index)?;
    let items = required_array(args, "items", op_index)?.to_vec();
    parse_legacy_edit(
        binding.source_set_name(),
        owner,
        kind,
        json!({"op": "add", "collection": collection, "elements": items}),
        |field| {
            format!(
                "ops[{op_index}].args.{}",
                field.replacen("elements", "items", 1)
            )
        },
        op_index,
    )
}

/// One member-collection update: `values` carries the member `name` plus the
/// changed fields of the typed update contract.
fn parse_member_set(
    operation: &str,
    collection: &str,
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    reject_unknown_args(operation, args, &["at", "values"], op_index)?;
    let target = qualified_target(args, op_index, binding)?;
    let (owner, kind) = metadata_owner(&target, op_index)?;
    let values = required_object(args, "values", op_index)?.clone();
    parse_legacy_edit(
        binding.source_set_name(),
        owner,
        kind,
        json!({"op": "update", "collection": collection, "elements": [values]}),
        |field| {
            format!(
                "ops[{op_index}].args.{}",
                field.replacen("elements[0]", "values", 1)
            )
        },
        op_index,
    )
}

/// One member-collection removal: `values.name` names the member to remove.
fn parse_member_remove(
    operation: &str,
    collection: &str,
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    reject_unknown_args(operation, args, &["at", "values"], op_index)?;
    let target = qualified_target(args, op_index, binding)?;
    let (owner, kind) = metadata_owner(&target, op_index)?;
    let values = required_object(args, "values", op_index)?;
    let name = values.get("name").and_then(Value::as_str).ok_or_else(|| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!("`{operation}` expects `values` with the member `name`"),
        )
        .at_path(format!("ops[{op_index}].args.values.name"))
    })?;
    if values.len() != 1 {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!("`{operation}` removal accepts only the member `name`"),
        )
        .at_path(format!("ops[{op_index}].args.values")));
    }
    parse_legacy_edit(
        binding.source_set_name(),
        owner,
        kind,
        json!({"op": "remove", "collection": collection, "names": [name]}),
        |field| format!("ops[{op_index}].args.{field}"),
        op_index,
    )
}

/// Predefined items ride their own typed element schema: add takes `items`,
/// update takes `values`, and removal takes `values.id`.
fn parse_predefined(
    operation: &str,
    mode: &str,
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    let target = qualified_target(args, op_index, binding)?;
    let (owner, kind) = metadata_owner(&target, op_index)?;
    let legacy = match mode {
        "add" => {
            reject_unknown_args(operation, args, &["at", "items"], op_index)?;
            let items = required_array(args, "items", op_index)?.to_vec();
            json!({"op": "add", "collection": "predefinedItems", "elements": items})
        }
        "update" => {
            reject_unknown_args(operation, args, &["at", "values"], op_index)?;
            let values = required_object(args, "values", op_index)?.clone();
            json!({"op": "update", "collection": "predefinedItems", "elements": [values]})
        }
        _ => {
            reject_unknown_args(operation, args, &["at", "values"], op_index)?;
            let values = required_object(args, "values", op_index)?;
            let id = values.get("id").and_then(Value::as_str).ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::BadValue,
                    format!("`{operation}` expects `values` with the predefined item `id`"),
                )
                .at_path(format!("ops[{op_index}].args.values.id"))
            })?;
            json!({"op": "remove", "collection": "predefinedItems", "ids": [id]})
        }
    };
    parse_legacy_edit(
        binding.source_set_name(),
        owner,
        kind,
        legacy,
        |field| {
            // `add` carries `items[i]`; `set` carries one object in `values`;
            // `remove` names the item through `values.id`.
            let rewritten = match mode {
                "add" => field.replacen("elements", "items", 1),
                "update" => field.replacen("elements[0]", "values", 1),
                _ => field.replacen("ids[0]", "values.id", 1),
            };
            format!("ops[{op_index}].args.{rewritten}")
        },
        op_index,
    )
}

/// One relation edit: `values` carries the closed `relation` name and its
/// `targets`; the mode comes from the operation name itself.
fn parse_relation(
    operation: &str,
    mode: &str,
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    reject_unknown_args(operation, args, &["at", "values"], op_index)?;
    let target = qualified_target(args, op_index, binding)?;
    let (owner, kind) = metadata_owner(&target, op_index)?;
    let values = required_object(args, "values", op_index)?;
    let relation = values.get("relation").cloned().unwrap_or(Value::Null);
    let targets = values.get("targets").cloned().unwrap_or(Value::Null);
    if values
        .keys()
        .any(|key| !matches!(key.as_str(), "relation" | "targets"))
    {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!("`{operation}` expects `values` with `relation` and `targets`"),
        )
        .at_path(format!("ops[{op_index}].args.values")));
    }
    parse_legacy_edit(
        binding.source_set_name(),
        owner,
        kind,
        json!({"op": "editRelations", "relation": relation, "mode": mode, "targets": targets}),
        |field| format!("ops[{op_index}].args.values.{field}"),
        op_index,
    )
}

pub(crate) fn plan_metadata_batch(
    staged: ApplyStagedState,
    authority: MetadataApplyAuthority<'_>,
    operations: &[IndexedPlanOperation<MetadataPlanOperation>],
) -> Result<(ApplyStagedState, Vec<ProvisionalApplyEffect>), ApplyPlanError> {
    if operations.is_empty() {
        return Err(empty_apply_family_batch());
    }
    if !authority.owns_staged_state(&staged) {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            "metadata planner authority does not own the staged state",
        )
        .at_path("ops"));
    }
    let mut staged = staged;
    let mut provisional = Vec::new();
    for operation in operations {
        let op_index = operation.index();
        let (target, edit) = match &operation.operation().kind {
            MetadataPlanKind::Edit {
                target,
                operation: edit,
            } => (target, edit),
            MetadataPlanKind::Create { kind, name } => {
                stage_object_create(
                    &mut staged,
                    authority.source_kind(),
                    authority.workspace_context(),
                    authority.source_set_name(),
                    authority.source_root(),
                    *kind,
                    name,
                    op_index,
                    &mut provisional,
                )?;
                continue;
            }
            MetadataPlanKind::Remove {
                target,
                kind,
                name,
                force,
            } => {
                stage_object_remove(
                    &mut staged,
                    &authority,
                    target,
                    *kind,
                    name,
                    *force,
                    op_index,
                    &mut provisional,
                )?;
                continue;
            }
            MetadataPlanKind::TemplateAdd { owner, items } => {
                stage_template_add(
                    &mut staged,
                    &authority,
                    owner,
                    items,
                    op_index,
                    &mut provisional,
                )?;
                continue;
            }
            MetadataPlanKind::TemplateSet {
                owner,
                name,
                values,
            } => {
                stage_template_set(
                    &mut staged,
                    &authority,
                    owner,
                    name,
                    values,
                    op_index,
                    &mut provisional,
                )?;
                continue;
            }
            MetadataPlanKind::TemplateRemove { owner, name } => {
                stage_template_remove(
                    &mut staged,
                    &authority,
                    owner,
                    name,
                    op_index,
                    &mut provisional,
                )?;
                continue;
            }
            MetadataPlanKind::SimpleProps {
                kind,
                name,
                relative,
                values,
            } => {
                stage_simple_props(
                    &mut staged,
                    authority.source_kind(),
                    *kind,
                    name,
                    relative,
                    values,
                    op_index,
                    &mut provisional,
                )?;
                continue;
            }
            MetadataPlanKind::HelpCreate { target, name, lang } => {
                stage_help_create(
                    &mut staged,
                    &authority,
                    target,
                    name,
                    lang,
                    op_index,
                    &mut provisional,
                )?;
                continue;
            }
        };
        let relative =
            metadata_descriptor_relative(target, authority.source_kind()).map_err(|message| {
                ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message)
                    .at_path(format!("ops[{op_index}].args.at"))
            })?;
        let preimage = staged
            .read(&relative)
            .map_err(|error| ApplyPlanError::staging(error, format!("ops[{op_index}].args.at")))?
            .ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::NotFound,
                    "metadata descriptor was not found",
                )
                .at_path(format!("ops[{op_index}].args.at"))
            })?;
        // The typed image transform addresses byte offsets of the parsed
        // document, so the byte-order mark stays outside the text it edits
        // and is restored on the way out.
        let (bom, body) = match preimage.strip_prefix(b"\xef\xbb\xbf") {
            Some(body) => (&b"\xef\xbb\xbf"[..], body),
            None => (&b""[..], preimage.as_slice()),
        };
        let mut postimage = String::from_utf8(body.to_vec()).map_err(|_| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                "metadata descriptor is not UTF-8",
            )
            .at_path(format!("ops[{op_index}].args.at"))
        })?;
        let (actual_kind, actual_name) =
            meta_edit_object_identity(&postimage).map_err(|message| {
                ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message)
                    .at_path(format!("ops[{op_index}].args.at"))
            })?;
        let expected = target.as_str().split('.').collect::<Vec<_>>();
        if expected.as_slice() != [actual_kind.as_str(), actual_name.as_str()] {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                "metadata descriptor identity does not match its logical target",
            )
            .at_path(format!("ops[{op_index}].args.at")));
        }
        let uuid_seed = format!(
            "{}\0{}\0{}\0{:?}\0{:x}",
            authority.source_set_name(),
            target.as_str(),
            op_index,
            edit,
            sha2::Sha256::digest(&preimage)
        );
        let lang = if typed_operations_write_multilingual_text(std::slice::from_ref(edit)) {
            staged_text_language(&mut staged, authority.source_kind(), op_index)?
        } else {
            String::new()
        };
        apply_typed_operations_to_image_with_seed(
            &mut postimage,
            std::slice::from_ref(edit),
            &lang,
            uuid_seed.as_bytes(),
        )
        .map_err(|failure| {
            let diagnostic = failure
                .diagnostics
                .into_iter()
                .next()
                .expect("typed metadata failures contain a diagnostic");
            let kind = match diagnostic.code {
                MetaDiagnosticCode::InvalidArguments
                | MetaDiagnosticCode::UnsupportedKind
                | MetaDiagnosticCode::CapabilityUnavailable => ApplyPlanErrorKind::BadValue,
                MetaDiagnosticCode::TargetNotFound => ApplyPlanErrorKind::NotFound,
                MetaDiagnosticCode::ValidationFailed => ApplyPlanErrorKind::Postcondition,
                MetaDiagnosticCode::AlreadyExists
                | MetaDiagnosticCode::SupportLocked
                | MetaDiagnosticCode::ValidationWarning
                | MetaDiagnosticCode::RedundantListPresentation
                | MetaDiagnosticCode::CommandTextRecommendedLimit
                | MetaDiagnosticCode::CommandTextUpperLimit
                | MetaDiagnosticCode::ConcurrentModification
                | MetaDiagnosticCode::ProviderUnavailable
                | MetaDiagnosticCode::RollbackFailed => ApplyPlanErrorKind::ProviderUnavailable,
            };
            let path = diagnostic.field.map_or_else(
                || format!("ops[{op_index}].args"),
                |field| format!("ops[{op_index}].args.{field}"),
            );
            ApplyPlanError::new(kind, diagnostic.message).at_path(path)
        })?;
        let mut postimage = {
            let mut bytes = Vec::with_capacity(bom.len() + postimage.len());
            bytes.extend_from_slice(bom);
            bytes.extend_from_slice(postimage.as_bytes());
            bytes
        };
        postimage.shrink_to_fit();
        if postimage != preimage {
            staged
                .replace(&relative, &preimage, postimage)
                .map_err(|error| {
                    ApplyPlanError::staging(error, format!("ops[{op_index}].args.at"))
                })?;
            provisional.push(ProvisionalApplyEffect::single(
                relative,
                DomainEvent::new(
                    DomainEventKind::MetadataChanged,
                    target.as_str().to_string(),
                ),
                op_index,
            ));
        }
    }
    Ok((staged, provisional))
}

pub(super) fn meta_failure_to_plan_error(
    failure: crate::application::metadata::MetaFailure,
    op_index: usize,
) -> ApplyPlanError {
    let diagnostic = failure
        .diagnostics
        .into_iter()
        .next()
        .expect("typed metadata failures contain a diagnostic");
    let kind = match diagnostic.code {
        MetaDiagnosticCode::InvalidArguments
        | MetaDiagnosticCode::UnsupportedKind
        | MetaDiagnosticCode::CapabilityUnavailable => ApplyPlanErrorKind::BadValue,
        MetaDiagnosticCode::TargetNotFound => ApplyPlanErrorKind::NotFound,
        MetaDiagnosticCode::ValidationFailed => ApplyPlanErrorKind::Postcondition,
        MetaDiagnosticCode::AlreadyExists | MetaDiagnosticCode::SupportLocked => {
            ApplyPlanErrorKind::InvalidState
        }
        MetaDiagnosticCode::ValidationWarning
        | MetaDiagnosticCode::RedundantListPresentation
        | MetaDiagnosticCode::CommandTextRecommendedLimit
        | MetaDiagnosticCode::CommandTextUpperLimit
        | MetaDiagnosticCode::ConcurrentModification
        | MetaDiagnosticCode::ProviderUnavailable
        | MetaDiagnosticCode::RollbackFailed => ApplyPlanErrorKind::ProviderUnavailable,
    };
    let path = diagnostic.field.map_or_else(
        || format!("ops[{op_index}].args"),
        |field| format!("ops[{op_index}].args.values.{field}"),
    );
    ApplyPlanError::new(kind, diagnostic.message).at_path(path)
}

pub(super) fn staged_relative(
    root: &std::path::Path,
    absolute: &std::path::Path,
    op_index: usize,
) -> Result<PathBuf, ApplyPlanError> {
    absolute
        .strip_prefix(root)
        .map(std::path::Path::to_path_buf)
        .map_err(|_| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::ProviderUnavailable,
                "planned metadata file lies outside the admitted source root",
            )
            .at_path(format!("ops[{op_index}].args.at"))
        })
}

/// The owner descriptor (`Configuration.xml`) with `<Kind>Name</Kind>` added
/// to or removed from `ChildObjects`, keeping the byte-order mark and the
/// line endings of the original image.
pub(super) fn owner_registration_image(
    owner: &[u8],
    kind: &str,
    name: &str,
    register: bool,
    op_index: usize,
) -> Result<Option<Vec<u8>>, ApplyPlanError> {
    use crate::infrastructure::native_operations::compile_transaction::{
        preserve_inserted_line_endings, split_utf8_bom_prefix,
    };
    let (bom, payload) = split_utf8_bom_prefix(owner);
    let source = std::str::from_utf8(payload).map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "the configuration descriptor is not UTF-8",
        )
        .at_path(format!("ops[{op_index}].args.at"))
    })?;
    let (updated, changed) = if register {
        let mut updated = source.to_string();
        let changed = crate::infrastructure::native_operations::cf::cf_edit_add_child_object_text(
            &mut updated,
            kind,
            name,
        )
        .map_err(|error| {
            ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, error)
                .at_path(format!("ops[{op_index}].args.at"))
        })?;
        (updated, changed)
    } else {
        crate::infrastructure::native_operations::meta::remove::remove_metadata_child_text_with_flag(
            source, kind, name,
        )
    };
    if !changed {
        return Ok(None);
    }
    let updated = preserve_inserted_line_endings(source, &updated);
    let mut image = Vec::with_capacity(bom.len() + updated.len());
    image.extend_from_slice(bom);
    image.extend_from_slice(updated.as_bytes());
    Ok(Some(image))
}

/// Stages a new top-level object from the platform template catalog: its
/// descriptor, modules and auxiliary files plus the owner registration.
/// Shared by the metadata family (`object.create`) and the families whose
/// creation operations are the same template (`role.create`,
/// `subsystem.create`).
#[allow(clippy::too_many_arguments)]
pub(super) fn stage_object_create(
    staged: &mut ApplyStagedState,
    source_kind: SourceSetKind,
    context: &crate::domain::workspace::WorkspaceContext,
    source_set_name: &str,
    root: &std::path::Path,
    kind: MetadataKind,
    name: &str,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    use crate::infrastructure::native_operations::meta::template_catalog::{
        MetadataTemplateCatalog, MetadataTemplateFileMode, MetadataTemplateOperationOverrides,
        PlatformMetadataTemplateCatalog,
    };
    let source = crate::infrastructure::platform_xml_source_targets::resolve_metadata_add_source(
        context,
        source_set_name,
    )
    .map_err(|failure| meta_failure_to_plan_error(failure, op_index))?;
    // The new descriptor's synonym is written in the source set's default
    // language, read from the staged image like every other edit (#909).
    let language = staged_text_language(staged, source_kind, op_index)?;
    let post_image = PlatformMetadataTemplateCatalog
        .minimal_object(
            &source,
            kind,
            name,
            MetadataTemplateOperationOverrides {
                source: false,
                handler: false,
            },
            source_set_name,
            context,
            &language,
        )
        .map_err(|failure| meta_failure_to_plan_error(failure, op_index))?;
    let owner_relative = staged_relative(root, &source.owner_path, op_index)?;
    let descriptor_relative =
        PathBuf::from(metadata_layout(kind).directory).join(format!("{name}.xml"));
    let existing = staged.read(&descriptor_relative).map_err(|error| {
        ApplyPlanError::staging(error, format!("ops[{op_index}].args.values.name"))
    })?;
    if existing.is_some() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            format!("metadata object `{}.{name}` already exists", kind.as_str()),
        )
        .at_path(format!("ops[{op_index}].args.values.name")));
    }
    let mut touched = Vec::new();
    for file in &post_image.files {
        let relative = file.relative_path.clone();
        let at_path = format!("ops[{op_index}].args.values.name");
        match file.mode {
            MetadataTemplateFileMode::Create => {
                staged
                    .create(&relative, file.bytes.clone())
                    .map_err(|error| ApplyPlanError::staging(error, at_path))?;
                touched.push(relative);
            }
            MetadataTemplateFileMode::Guard => {
                let current = staged
                    .read(&relative)
                    .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
                let expected = file.preimage.as_deref().unwrap_or(&file.bytes);
                if current.as_deref() != Some(expected) {
                    return Err(ApplyPlanError::new(
                        ApplyPlanErrorKind::InvalidState,
                        format!(
                            "prerequisite `{}` changed while planning the new object",
                            relative.display()
                        ),
                    )
                    .at_path(at_path));
                }
            }
            MetadataTemplateFileMode::Replace => {
                let expected = file.preimage.as_deref().unwrap_or(&file.bytes).to_vec();
                staged
                    .replace(&relative, &expected, file.bytes.clone())
                    .map_err(|error| ApplyPlanError::staging(error, at_path))?;
                touched.push(relative);
            }
        }
    }
    let owner_preimage = staged
        .read(&owner_relative)
        .map_err(|error| ApplyPlanError::staging(error, format!("ops[{op_index}].args.at")))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                "the configuration descriptor was not found",
            )
            .at_path(format!("ops[{op_index}].args.at"))
        })?;
    let Some(owner_postimage) =
        owner_registration_image(&owner_preimage, kind.as_str(), name, true, op_index)?
    else {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            format!(
                "the configuration already registers `{}.{name}`",
                kind.as_str()
            ),
        )
        .at_path(format!("ops[{op_index}].args.values.name")));
    };
    staged
        .replace(&owner_relative, &owner_preimage, owner_postimage)
        .map_err(|error| ApplyPlanError::staging(error, format!("ops[{op_index}].args.at")))?;
    touched.push(owner_relative);
    provisional.push(ProvisionalApplyEffect::spanning(
        touched,
        DomainEvent::new(
            DomainEventKind::MetadataChanged,
            post_image.metadata_path.as_str().to_string(),
        ),
        op_index,
    ));
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn stage_object_remove(
    staged: &mut ApplyStagedState,
    authority: &MetadataApplyAuthority<'_>,
    target: &MetadataAddress,
    kind: MetadataKind,
    name: &str,
    force: bool,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    use crate::infrastructure::native_operations::meta::remove::{
        meta_remove_search_patterns, meta_remove_should_skip_file,
        plan_meta_remove_subsystem_replacements, META_REMOVE_REFERENCE_FILE_MAX_BYTES,
    };
    let at_path = format!("ops[{op_index}].args.at");
    require_untouched_staged_state(staged, "object.remove", &at_path)?;
    let root = authority.source_root();
    let layout = metadata_layout(kind);
    let descriptor_relative = PathBuf::from(layout.directory).join(format!("{name}.xml"));
    let descriptor_preimage = staged
        .read(&descriptor_relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                "metadata descriptor was not found",
            )
            .at_path(at_path.clone())
        })?;
    let descriptor_absolute = root.join(&descriptor_relative);
    let object_dir = root.join(layout.directory).join(name);
    let files = staged
        .enumerate_tree(Path::new(""))
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
    let patterns = meta_remove_search_patterns(kind.as_str(), name, layout.directory);
    let type_name_ref = format!("{}.{name}", kind.as_str());
    let mut references = Vec::new();
    for relative in &files {
        let extension = relative
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("");
        if !extension.eq_ignore_ascii_case("xml") && !extension.eq_ignore_ascii_case("bsl") {
            continue;
        }
        let absolute = root.join(relative);
        if meta_remove_should_skip_file(
            &absolute,
            root,
            &descriptor_absolute,
            &object_dir,
            true,
            true,
        ) {
            continue;
        }
        let bytes = staged
            .read_guarded_bounded(relative, META_REMOVE_REFERENCE_FILE_MAX_BYTES)
            .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
            .ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidState,
                    "reference input disappeared",
                )
            })?;
        let text = std::str::from_utf8(&bytes).map_err(|error| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                format!("reference input is not UTF-8: {error}"),
            )
        })?;
        if patterns.iter().any(|pattern| text.contains(pattern))
            || (extension.eq_ignore_ascii_case("xml") && text.contains(&type_name_ref))
        {
            references.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
    let mut kept_references = None;
    if !references.is_empty() {
        let shown = references
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        if !force {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidState,
                format!(
                    "`{}` is still referenced by {} source file(s): {shown}; remove the references first",
                    target.as_str(),
                    references.len()
                ),
            )
            .at_path(at_path));
        }
        // A forced removal leaves the referrers as they are and says so:
        // the caller asked for the object to go, the dangling references
        // are the caller's next task.
        kept_references = Some(serde_json::json!({
            "code": "references_kept",
            "at": at_path,
            "count": references.len(),
            "files": references.iter().take(5).cloned().collect::<Vec<_>>(),
            "message": format!(
                "`{}` was removed with force; {} source file(s) still refer to it: {shown}",
                target.as_str(),
                references.len()
            ),
        }));
    }
    let mut touched = Vec::new();
    for relative in files
        .iter()
        .filter(|path| root.join(path).starts_with(&object_dir))
    {
        let preimage = staged
            .read(relative)
            .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
            .ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidState,
                    "payload input disappeared",
                )
            })?;
        staged
            .remove(relative, &preimage)
            .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
        touched.push(relative.clone());
    }
    let subsystems_dir = root.join("Subsystems");
    if subsystems_dir.is_dir() {
        let mut replacements = Vec::new();
        let mut reads = Vec::new();
        plan_meta_remove_subsystem_replacements(
            &subsystems_dir,
            target.as_str(),
            &mut replacements,
            &mut reads,
        )
        .map_err(|error| {
            ApplyPlanError::new(ApplyPlanErrorKind::ProviderUnavailable, error)
                .at_path(at_path.clone())
        })?;
        for read in reads {
            let relative = staged_relative(root, &read.path, op_index)?;
            let observed = staged
                .read(&relative)
                .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
            if observed.as_deref() != Some(read.raw.as_slice()) {
                return Err(ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidState,
                    "subsystem input changed while planning",
                ));
            }
        }
        for replacement in replacements {
            let relative = staged_relative(root, &replacement.path, op_index)?;
            staged
                .replace(&relative, &replacement.original, replacement.replacement)
                .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
            touched.push(relative);
        }
    }
    let owner_relative = PathBuf::from("Configuration.xml");
    let owner_preimage = staged
        .read(&owner_relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                "the configuration descriptor was not found",
            )
            .at_path(at_path.clone())
        })?;
    let Some(owner_postimage) =
        owner_registration_image(&owner_preimage, kind.as_str(), name, false, op_index)?
    else {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "metadata object is not registered by its owner",
        )
        .at_path(at_path));
    };
    staged
        .replace(&owner_relative, &owner_preimage, owner_postimage)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
    touched.push(owner_relative);
    staged
        .remove(&descriptor_relative, &descriptor_preimage)
        .map_err(|error| ApplyPlanError::staging(error, at_path))?;
    touched.push(descriptor_relative);
    let effect = ProvisionalApplyEffect::spanning(
        touched,
        DomainEvent::new(
            DomainEventKind::MetadataChanged,
            target.as_str().to_string(),
        ),
        op_index,
    );
    provisional.push(match kept_references {
        Some(warning) => effect.with_warning(warning),
        None => effect,
    });
    Ok(())
}

/// The descriptor image after a text edit: the observed byte-order mark and
/// the source's line-ending profile are kept, as the typed engine does.
pub(super) fn preserve_descriptor_image(preimage: &[u8], source: &str, updated: &str) -> Vec<u8> {
    use crate::infrastructure::native_operations::compile_transaction::{
        preserve_inserted_line_endings, split_utf8_bom_prefix,
    };
    let (bom, _) = split_utf8_bom_prefix(preimage);
    let updated = preserve_inserted_line_endings(source, updated);
    let mut image = Vec::with_capacity(bom.len() + updated.len());
    image.extend_from_slice(bom);
    image.extend_from_slice(updated.as_bytes());
    image
}

/// Sets the `lang` item of a multilingual property of the descriptor's own
/// `<Properties>`, keeping the other languages byte for byte; an empty
/// element becomes a one-item block. `None` when the descriptor is not XML
/// or has no such property.
pub(super) fn set_ml_property(
    text: &str,
    tag: &str,
    indent: &str,
    lang: &str,
    value: &str,
) -> Option<String> {
    edit_descriptor_property(text, tag, |source, property| {
        meta_mltext_property_replacement(source, property, indent, lang, value)
    })
}

/// Sets a plain-text property of the descriptor's own `<Properties>`,
/// spelled with the element name the source uses.
fn set_plain_property(text: &str, tag: &str, value: &str) -> Option<String> {
    edit_descriptor_property(text, tag, |source, property| {
        let open = &source[property.range().start + 1..];
        let name = &open[..open
            .find(|ch: char| ch.is_whitespace() || ch == '>' || ch == '/')
            .unwrap_or(open.len())];
        if value.is_empty() {
            format!("<{name}/>")
        } else {
            format!(
                "<{name}>{}</{name}>",
                value
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
            )
        }
    })
}

/// Rewrites the property `tag` of the first `<Properties>` of the document,
/// which is the descriptor object's own; `replace` builds the new element
/// from the source text and the property node.
fn edit_descriptor_property(
    text: &str,
    tag: &str,
    replace: impl FnOnce(&str, roxmltree::Node<'_, '_>) -> String,
) -> Option<String> {
    let document = roxmltree::Document::parse(text).ok()?;
    let property = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "Properties")?
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == tag)?;
    let replacement = replace(text, property);
    let mut updated = text.to_string();
    updated.replace_range(property.range(), &replacement);
    Some(updated)
}

/// `<Tag>` as a multilingual text block in `lang`, or the empty element.
fn ml_text_block(indent: &str, tag: &str, lang: &str, text: &str) -> String {
    if text.is_empty() {
        return format!("{indent}<{tag}/>");
    }
    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        "{indent}<{tag}>\n{indent}\t<v8:item>\n{indent}\t\t<v8:lang>{lang}</v8:lang>\n{indent}\t\t<v8:content>{escaped}</v8:content>\n{indent}\t</v8:item>\n{indent}</{tag}>"
    )
}

/// `LanguageCode` of the default language of the edited source set: new
/// multilingual values are written in it (#909). It is read from the staged
/// `Configuration.xml` and `Languages/<Name>.xml`, so an extension uses its
/// own adopted languages. External processors and reports have no language
/// profile; see [`EXTERNAL_SOURCE_TEXT_LANGUAGE`].
pub(super) fn staged_text_language(
    staged: &mut ApplyStagedState,
    source_kind: SourceSetKind,
    op_index: usize,
) -> Result<String, ApplyPlanError> {
    if matches!(
        source_kind,
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
    ) {
        return Ok(EXTERNAL_SOURCE_TEXT_LANGUAGE.to_string());
    }
    let at_path = format!("ops[{op_index}].args.at");
    let unavailable = |message: String| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            format!("the configuration default language is unavailable: {message}"),
        )
        .at_path(at_path.clone())
    };
    let configuration = staged
        .read(Path::new("Configuration.xml"))
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
        .ok_or_else(|| unavailable("the source set has no Configuration.xml".to_string()))?;
    let mut staging_error = None;
    let language = meta_default_text_language(&configuration, |relative| {
        staged.read(relative).map_err(|error| {
            let message = error.to_string();
            staging_error = Some(error);
            message
        })
    });
    if let Some(error) = staging_error {
        return Err(ApplyPlanError::staging(error, at_path));
    }
    language.map_err(unavailable)
}

/// Replaces the first `<Tag>…</Tag>` or `<Tag/>` inside `<Properties>`.
fn replace_property_block(text: &str, tag: &str, replacement: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let empty = format!("<{tag}/>");
    let properties_start = text.find("<Properties>")?;
    let properties_end = text[properties_start..].find("</Properties>")? + properties_start;
    let scope = &text[properties_start..properties_end];
    let (start, end) = if let Some(index) = scope.find(&empty) {
        (index, index + empty.len())
    } else {
        let index = scope.find(&open)?;
        let close_index = scope[index..].find(&close)? + index + close.len();
        (index, close_index)
    };
    let mut updated = text.to_string();
    updated.replace_range(
        properties_start + start..properties_start + end,
        replacement.trim_start(),
    );
    Some(updated)
}

#[allow(clippy::too_many_arguments)]
fn stage_simple_props(
    staged: &mut ApplyStagedState,
    source_kind: SourceSetKind,
    kind: NodeKind,
    name: &str,
    relative: &std::path::Path,
    values: &Map<String, Value>,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    let preimage = staged
        .read(relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                "metadata descriptor was not found",
            )
            .at_path(at_path.clone())
        })?;
    let (_, body) = match preimage.strip_prefix(b"\xef\xbb\xbf") {
        Some(body) => (&b"\xef\xbb\xbf"[..], body),
        None => (&b""[..], preimage.as_slice()),
    };
    let source = String::from_utf8(body.to_vec()).map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "metadata descriptor is not UTF-8",
        )
        .at_path(at_path.clone())
    })?;
    let text_value = |key: &str| -> Result<Option<String>, ApplyPlanError> {
        let Some(value) = values.get(key) else {
            return Ok(None);
        };
        value
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::BadValue,
                    format!("property `{key}` takes a string"),
                )
                .at_path(format!("ops[{op_index}].args.values.{key}"))
            })
            .map(Some)
    };
    let postimage_text = match kind {
        NodeKind::Role => {
            for key in values.keys() {
                if !matches!(key.as_str(), "synonym" | "Synonym" | "comment" | "Comment") {
                    return Err(ApplyPlanError::new(
                        ApplyPlanErrorKind::BadValue,
                        format!("role properties accept `synonym` and `comment`, not `{key}`"),
                    )
                    .at_path(format!("ops[{op_index}].args.values.{key}")));
                }
            }
            let mut text = source.clone();
            let indent = "\t\t\t";
            if let Some(synonym) = text_value("synonym")?.or(text_value("Synonym")?) {
                let lang = staged_text_language(staged, source_kind, op_index)?;
                text = set_ml_property(&text, "Synonym", indent, &lang, &synonym).ok_or_else(
                    || {
                        ApplyPlanError::new(
                            ApplyPlanErrorKind::InvalidSource,
                            "the role descriptor has no Synonym property",
                        )
                        .at_path(at_path.clone())
                    },
                )?;
            }
            if let Some(comment) = text_value("comment")?.or(text_value("Comment")?) {
                let block = if comment.is_empty() {
                    format!("{indent}<Comment/>")
                } else {
                    format!(
                        "{indent}<Comment>{}</Comment>",
                        comment
                            .replace('&', "&amp;")
                            .replace('<', "&lt;")
                            .replace('>', "&gt;")
                    )
                };
                text = replace_property_block(&text, "Comment", &block).ok_or_else(|| {
                    ApplyPlanError::new(
                        ApplyPlanErrorKind::InvalidSource,
                        "the role descriptor has no Comment property",
                    )
                    .at_path(at_path.clone())
                })?;
            }
            text
        }
        NodeKind::Subsystem => {
            // Parsing proves the descriptor is a subsystem. The properties are
            // then edited in place: regenerating the descriptor from the parsed
            // model kept one language of Synonym and Explanation and dropped
            // the others (#909).
            crate::infrastructure::native_operations::common::parse_subsystem_edit_model(
                &source,
                &relative.display().to_string(),
            )
            .map_err(|message| {
                ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message)
                    .at_path(at_path.clone())
            })?;
            let mut text = source.clone();
            let indent = "\t\t\t";
            let mut lang = None;
            let missing = |tag: &str| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidSource,
                    format!("the subsystem descriptor has no {tag} property"),
                )
                .at_path(at_path.clone())
            };
            for (key, value) in values {
                let bool_text = || -> Result<String, ApplyPlanError> {
                    match value {
                        Value::Bool(flag) => Ok(flag.to_string()),
                        Value::String(text) if text == "true" || text == "false" => {
                            Ok(text.clone())
                        }
                        _ => Err(ApplyPlanError::new(
                            ApplyPlanErrorKind::BadValue,
                            format!("property `{key}` takes a boolean"),
                        )
                        .at_path(format!("ops[{op_index}].args.values.{key}"))),
                    }
                };
                let flag_tag = match key.as_str() {
                    "synonym" | "Synonym" | "explanation" | "Explanation" => {
                        let tag = if key.eq_ignore_ascii_case("synonym") {
                            "Synonym"
                        } else {
                            "Explanation"
                        };
                        let value = text_value(key)?.unwrap_or_default();
                        if lang.is_none() {
                            lang = Some(staged_text_language(staged, source_kind, op_index)?);
                        }
                        let lang = lang.as_deref().expect("the language was just resolved");
                        text = set_ml_property(&text, tag, indent, lang, &value)
                            .ok_or_else(|| missing(tag))?;
                        continue;
                    }
                    "comment" | "Comment" => {
                        let comment = text_value(key)?.unwrap_or_default();
                        text = set_plain_property(&text, "Comment", &comment)
                            .ok_or_else(|| missing("Comment"))?;
                        continue;
                    }
                    "includeHelpInContents" | "IncludeHelpInContents" => "IncludeHelpInContents",
                    "includeInCommandInterface" | "IncludeInCommandInterface" => {
                        "IncludeInCommandInterface"
                    }
                    "useOneCommand" | "UseOneCommand" => "UseOneCommand",
                    other => {
                        return Err(ApplyPlanError::new(
                            ApplyPlanErrorKind::BadValue,
                            format!(
                                "subsystem properties accept synonym, comment, explanation, includeHelpInContents, includeInCommandInterface and useOneCommand, not `{other}`"
                            ),
                        )
                        .at_path(format!("ops[{op_index}].args.values.{other}")))
                    }
                };
                text = set_plain_property(&text, flag_tag, &bool_text()?)
                    .ok_or_else(|| missing(flag_tag))?;
            }
            text
        }
        _ => unreachable!("simple props are parsed for roles and subsystems only"),
    };
    let postimage = preserve_descriptor_image(&preimage, &source, &postimage_text);
    if postimage == preimage {
        return Ok(());
    }
    staged
        .replace(relative, &preimage, postimage)
        .map_err(|error| ApplyPlanError::staging(error, at_path))?;
    provisional.push(ProvisionalApplyEffect::single(
        relative.to_path_buf(),
        DomainEvent::new(
            if kind == NodeKind::Role {
                DomainEventKind::RoleChanged
            } else {
                DomainEventKind::SubsystemChanged
            },
            format!("{}.{name}", kind.as_str()),
        ),
        op_index,
    ));
    Ok(())
}

fn template_descriptor_xml(
    name: &str,
    kind: crate::domain::metadata::MetaTemplateKind,
    format_version: &str,
    uuid: &str,
    lang: &str,
) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MetaDataObject {} version=\"{format_version}\">\n\t<Template uuid=\"{uuid}\">\n\t\t<Properties>\n\t\t\t<Name>{name}</Name>\n{}\n\t\t\t<Comment/>\n\t\t\t<TemplateType>{}</TemplateType>\n\t\t</Properties>\n\t</Template>\n</MetaDataObject>\n",
        crate::infrastructure::native_operations::template::full_md_namespace_declarations(),
        ml_text_block("\t\t\t", "Synonym", lang, &crate::infrastructure::native_operations::common::split_camel_case(name)),
        kind.as_str()
    )
}

fn empty_data_composition_schema_xml() -> String {
    concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<DataCompositionSchema xmlns=\"http://v8.1c.ru/8.1/data-composition-system/schema\" xmlns:dcscom=\"http://v8.1c.ru/8.1/data-composition-system/common\" xmlns:dcscor=\"http://v8.1c.ru/8.1/data-composition-system/core\" xmlns:dcsset=\"http://v8.1c.ru/8.1/data-composition-system/settings\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n",
        "\t<dataSource>\n\t\t<name>ИсточникДанных1</name>\n\t\t<dataSourceType>Local</dataSourceType>\n\t</dataSource>\n",
        "\t<settingsVariant>\n\t\t<dcsset:name>Основной</dcsset:name>\n\t\t<dcsset:presentation xsi:type=\"xs:string\">Основной</dcsset:presentation>\n\t\t<dcsset:settings xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\">\n\t\t\t<dcsset:selection/>\n\t\t\t<dcsset:item xsi:type=\"dcsset:StructureItemGroup\">\n\t\t\t\t<dcsset:order/>\n\t\t\t\t<dcsset:selection>\n\t\t\t\t\t<dcsset:item xsi:type=\"dcsset:SelectedItemAuto\"/>\n\t\t\t\t</dcsset:selection>\n\t\t\t</dcsset:item>\n\t\t</dcsset:settings>\n\t</settingsVariant>\n",
        "</DataCompositionSchema>\n",
    )
    .to_string()
}

/// Adds `<Tag>Name</Tag>` to the owner's outer `ChildObjects` (the block
/// closed at two tabs), after the last sibling of the same tag when there
/// is one.
fn register_owner_child_ref(text: &str, tag: &str, name: &str) -> Option<String> {
    let entry = format!("\t\t\t<{tag}>{name}</{tag}>\n");
    let mut updated = text.to_string();
    if let Some(index) = text.rfind(&format!("\n\t\t\t<{tag}>")) {
        let line_end = text[index + 1..].find('\n')? + index + 2;
        updated.insert_str(line_end, &entry);
        return Some(updated);
    }
    if let Some(close) = text.rfind("\n\t\t</ChildObjects>") {
        updated.insert_str(close + 1, &entry);
        return Some(updated);
    }
    // Compact layouts: an empty `<ChildObjects/>` or a close on the same
    // line as its content.
    if let Some(empty) = text.rfind("<ChildObjects/>") {
        updated.replace_range(
            empty..empty + "<ChildObjects/>".len(),
            &format!("<ChildObjects>\n{entry}\t\t</ChildObjects>"),
        );
        return Some(updated);
    }
    let close = text.rfind("</ChildObjects>")?;
    updated.insert_str(close, &format!("\n{entry}\t\t"));
    Some(updated)
}

fn owner_descriptor_and_text(
    staged: &mut ApplyStagedState,
    authority: &MetadataApplyAuthority<'_>,
    owner: &MetadataAddress,
    op_index: usize,
) -> Result<(PathBuf, Vec<u8>, String), ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    let relative =
        metadata_descriptor_relative(owner, authority.source_kind()).map_err(|message| {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(at_path.clone())
        })?;
    let preimage = staged
        .read(&relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                "metadata descriptor was not found",
            )
            .at_path(at_path.clone())
        })?;
    let text = String::from_utf8(
        preimage
            .strip_prefix(b"\xef\xbb\xbf")
            .unwrap_or(&preimage)
            .to_vec(),
    )
    .map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "metadata descriptor is not UTF-8",
        )
        .at_path(at_path)
    })?;
    Ok((relative, preimage, text))
}

fn bom_bytes(text: &str) -> Vec<u8> {
    let mut bytes = b"\xef\xbb\xbf".to_vec();
    bytes.extend_from_slice(text.as_bytes());
    bytes
}

fn stage_template_add(
    staged: &mut ApplyStagedState,
    authority: &MetadataApplyAuthority<'_>,
    owner: &MetadataAddress,
    items: &[(String, crate::domain::metadata::MetaTemplateKind)],
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    use crate::domain::metadata::MetaTemplateKind;
    let (owner_relative, owner_preimage, owner_source) =
        owner_descriptor_and_text(staged, authority, owner, op_index)?;
    let mut owner_text = owner_source.clone();
    let templates_dir = owner_relative.with_extension("").join("Templates");
    let lang = staged_text_language(staged, authority.source_kind(), op_index)?;
    let mut touched = Vec::new();
    for (index, (name, kind)) in items.iter().enumerate() {
        let name_path = format!("ops[{op_index}].args.items[{index}].name");
        if owner_text.contains(&format!("<Template>{name}</Template>")) {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidState,
                format!("template `{name}` already exists on `{}`", owner.as_str()),
            )
            .at_path(name_path));
        }
        let content = match kind {
            MetaTemplateKind::SpreadsheetDocument => {
                crate::infrastructure::native_operations::mxl::empty_spreadsheet_document_xml()
            }
            MetaTemplateKind::DataCompositionSchema => empty_data_composition_schema_xml(),
            MetaTemplateKind::TextDocument | MetaTemplateKind::HtmlDocument | MetaTemplateKind::BinaryData => {
                return Err(ApplyPlanError::new(
                    ApplyPlanErrorKind::BadValue,
                    format!(
                        "template type `{}` needs content the planner cannot synthesize; create a SpreadsheetDocument or DataCompositionSchema template",
                        kind.as_str()
                    ),
                )
                .at_path(format!("ops[{op_index}].args.items[{index}].templateType")))
            }
        };
        let uuid = {
            use sha2::Digest;
            let digest = sha2::Sha256::digest(format!(
                "unica-v13-template-uuid-v1\0{}\0{}.Template.{name}",
                authority.source_set_name(),
                owner.as_str()
            ));
            let mut bytes = [0_u8; 16];
            bytes.copy_from_slice(&digest[..16]);
            bytes[6] = (bytes[6] & 0x0f) | 0x40;
            bytes[8] = (bytes[8] & 0x3f) | 0x80;
            uuid::Uuid::from_bytes(bytes).to_string()
        };
        let descriptor =
            template_descriptor_xml(name, *kind, authority.expected_format(), &uuid, &lang);
        let descriptor_relative = templates_dir.join(format!("{name}.xml"));
        let content_relative = templates_dir.join(name).join("Ext/Template.xml");
        for (relative, text) in [
            (&descriptor_relative, &descriptor),
            (&content_relative, &content),
        ] {
            if staged
                .read(relative)
                .map_err(|error| ApplyPlanError::staging(error, name_path.clone()))?
                .is_some()
            {
                return Err(ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidState,
                    format!("`{}` already exists", relative.display()),
                )
                .at_path(name_path));
            }
            staged
                .create(relative, bom_bytes(text))
                .map_err(|error| ApplyPlanError::staging(error, name_path.clone()))?;
            touched.push(relative.clone());
        }
        owner_text = register_owner_child_ref(&owner_text, "Template", name).ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                "the owner descriptor has no ChildObjects block to register the template",
            )
            .at_path(format!("ops[{op_index}].args.at"))
        })?;
    }
    let owner_postimage = preserve_descriptor_image(&owner_preimage, &owner_source, &owner_text);
    staged
        .replace(&owner_relative, &owner_preimage, owner_postimage)
        .map_err(|error| ApplyPlanError::staging(error, format!("ops[{op_index}].args.at")))?;
    touched.push(owner_relative);
    provisional.push(ProvisionalApplyEffect::spanning(
        touched,
        DomainEvent::new(DomainEventKind::TemplateChanged, owner.as_str().to_string()),
        op_index,
    ));
    Ok(())
}

fn stage_template_set(
    staged: &mut ApplyStagedState,
    authority: &MetadataApplyAuthority<'_>,
    owner: &MetadataAddress,
    name: &str,
    values: &Map<String, Value>,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    let (owner_relative, _, owner_source) =
        owner_descriptor_and_text(staged, authority, owner, op_index)?;
    if !owner_source.contains(&format!("<Template>{name}</Template>")) {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::NotFound,
            format!("`{}` has no template `{name}`", owner.as_str()),
        )
        .at_path(at_path));
    }
    let relative = owner_relative
        .with_extension("")
        .join("Templates")
        .join(format!("{name}.xml"));
    let preimage = staged
        .read(&relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                "the template descriptor was not found",
            )
            .at_path(at_path.clone())
        })?;
    let (_, body) = match preimage.strip_prefix(b"\xef\xbb\xbf") {
        Some(body) => (&b"\xef\xbb\xbf"[..], body),
        None => (&b""[..], preimage.as_slice()),
    };
    let mut text = String::from_utf8(body.to_vec()).map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "the template descriptor is not UTF-8",
        )
        .at_path(at_path.clone())
    })?;
    let source_text = text.clone();
    let string_value = |key: &str| -> Result<Option<String>, ApplyPlanError> {
        match values.get(key) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(_) => Err(ApplyPlanError::new(
                ApplyPlanErrorKind::BadValue,
                format!("`{key}` must be a string"),
            )
            .at_path(format!("ops[{op_index}].args.values.{key}"))),
        }
    };
    let indent = "\t\t\t";
    if let Some(synonym) = string_value("synonym")? {
        let lang = staged_text_language(staged, authority.source_kind(), op_index)?;
        text = set_ml_property(&text, "Synonym", indent, &lang, &synonym).ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                "the template descriptor has no Synonym property",
            )
            .at_path(at_path.clone())
        })?;
    }
    if let Some(comment) = string_value("comment")? {
        let block = if comment.is_empty() {
            format!("{indent}<Comment/>")
        } else {
            format!(
                "{indent}<Comment>{}</Comment>",
                comment
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
            )
        };
        text = replace_property_block(&text, "Comment", &block).ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                "the template descriptor has no Comment property",
            )
            .at_path(at_path.clone())
        })?;
    }
    if let Some(kind) = string_value("templateType")? {
        let kind =
            crate::domain::metadata::MetaTemplateKind::parse(&kind).map_err(|diagnostic| {
                ApplyPlanError::new(ApplyPlanErrorKind::BadValue, diagnostic.message)
                    .at_path(format!("ops[{op_index}].args.values.templateType"))
            })?;
        text = replace_property_block(
            &text,
            "TemplateType",
            &format!("{indent}<TemplateType>{}</TemplateType>", kind.as_str()),
        )
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                "the template descriptor has no TemplateType property",
            )
            .at_path(at_path.clone())
        })?;
    }
    let postimage = preserve_descriptor_image(&preimage, &source_text, &text);
    if postimage == preimage {
        return Ok(());
    }
    staged
        .replace(&relative, &preimage, postimage)
        .map_err(|error| ApplyPlanError::staging(error, at_path))?;
    provisional.push(ProvisionalApplyEffect::single(
        relative,
        DomainEvent::new(
            DomainEventKind::TemplateChanged,
            format!("{}.Template.{name}", owner.as_str()),
        ),
        op_index,
    ));
    Ok(())
}

fn stage_template_remove(
    staged: &mut ApplyStagedState,
    authority: &MetadataApplyAuthority<'_>,
    owner: &MetadataAddress,
    name: &str,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    require_untouched_staged_state(staged, "template.remove", &at_path)?;
    let (owner_relative, owner_preimage, owner_source) =
        owner_descriptor_and_text(staged, authority, owner, op_index)?;
    let (deregistered, removed) =
        crate::infrastructure::native_operations::meta::remove::remove_metadata_child_text_with_flag(
            &owner_source,
            "Template",
            name,
        );
    if !removed {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::NotFound,
            format!("`{}` has no template `{name}`", owner.as_str()),
        )
        .at_path(at_path));
    }
    let reference = format!("{}.Template.{name}", owner.as_str());
    let mut owner_text = deregistered;
    for property in ["MainDataCompositionSchema", "DefaultTemplate"] {
        let filled = format!("<{property}>{reference}</{property}>");
        if owner_text.contains(&filled) {
            owner_text = owner_text.replace(&filled, &format!("<{property}/>"));
        }
    }
    let mut touched = Vec::new();
    let templates_dir = owner_relative.with_extension("").join("Templates");
    let descriptor_relative = templates_dir.join(format!("{name}.xml"));
    if let Some(preimage) = staged
        .read(&descriptor_relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
    {
        staged
            .remove(&descriptor_relative, &preimage)
            .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
        touched.push(descriptor_relative);
    }
    let payload_relative = templates_dir.join(name);
    let files = staged
        .enumerate_tree(&payload_relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
    for relative in files {
        if let Some(preimage) = staged
            .read(&relative)
            .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
        {
            staged
                .remove(&relative, &preimage)
                .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
            touched.push(relative);
        }
    }
    staged
        .replace(
            &owner_relative,
            &owner_preimage,
            preserve_descriptor_image(&owner_preimage, &owner_source, &owner_text),
        )
        .map_err(|error| ApplyPlanError::staging(error, at_path))?;
    touched.push(owner_relative);
    provisional.push(ProvisionalApplyEffect::spanning(
        touched,
        DomainEvent::new(DomainEventKind::TemplateChanged, reference),
        op_index,
    ));
    Ok(())
}

/// Removal planners list payload files and scan references on the physical
/// source root, so they only see the truth when nothing was staged before
/// them. A batch that edited files first must run the removal in its own
/// apply call; refusing here keeps a stale scan from committing dangling
/// references or leaving staged payload behind.
pub(super) fn require_untouched_staged_state(
    staged: &ApplyStagedState,
    operation: &str,
    at_path: &str,
) -> Result<(), ApplyPlanError> {
    if staged.planned_changes().is_empty() {
        return Ok(());
    }
    Err(ApplyPlanError::new(
        ApplyPlanErrorKind::InvalidState,
        format!(
            "`{operation}` inspects the source root and must run in its own apply call before other changes"
        ),
    )
    .at_path(at_path.to_string()))
}

fn stage_help_create(
    staged: &mut ApplyStagedState,
    authority: &MetadataApplyAuthority<'_>,
    target: &MetadataAddress,
    name: &str,
    lang: &str,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    // The help facet planner inspects the owner's forms on the physical root.
    require_untouched_staged_state(staged, "help.create", &at_path)?;
    let root = authority.source_root();
    let descriptor_relative = metadata_descriptor_relative(target, authority.source_kind())
        .map_err(|message| {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(at_path.clone())
        })?;
    if staged
        .read(&descriptor_relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
        .is_none()
    {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::NotFound,
            "metadata descriptor was not found",
        )
        .at_path(at_path));
    }
    let changes = crate::infrastructure::native_operations::meta::plan_help_facet_files(
        &root.join(&descriptor_relative),
        target,
        name,
        lang,
    )
    .map_err(|failure| meta_failure_to_plan_error(failure, op_index))?;
    let mut touched = Vec::new();
    for (path, preimage, postimage) in changes {
        let relative = staged_relative(root, &path, op_index)?;
        match (preimage, postimage) {
            (None, Some(bytes)) => staged
                .create(&relative, bytes)
                .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?,
            (Some(expected), Some(bytes)) => staged
                .replace(&relative, &expected, bytes)
                .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?,
            (Some(expected), None) => staged
                .remove(&relative, &expected)
                .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?,
            (None, None) => continue,
        }
        touched.push(relative);
    }
    provisional.push(ProvisionalApplyEffect::spanning(
        touched,
        DomainEvent::new(
            DomainEventKind::MetadataChanged,
            target.as_str().to_string(),
        ),
        op_index,
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        parse_metadata_plan_operation, plan_metadata_batch, preserve_descriptor_image,
        set_ml_property,
    };
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::ProviderDeadline;
    use crate::domain::project_sources::{SourceFormat, SourceProfile, SourceSetKind};
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::native_operations::apply::{
        ApplyPlanErrorKind, StagedChangeKind, StagedFileState,
    };
    use crate::infrastructure::native_operations::apply_families::request::IndexedPlanOperation;
    use crate::infrastructure::workspace_actor::{
        ApplyAdmission, ProviderRootBinding, WorkspaceActor, WorkspaceIdentity,
        WorkspaceSourceSetInput,
    };
    use serde_json::json;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    const ORDER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
	<Document uuid="11111111-1111-4111-8111-111111111111">
		<Properties><Name>Order</Name><Synonym/><Comment/><BasedOn/></Properties>
		<ChildObjects/>
	</Document>
</MetaDataObject>
"#;

    struct MetadataFixture {
        _root: tempfile::TempDir,
        actor: Arc<WorkspaceActor>,
        binding: ProviderRootBinding,
        descriptor: PathBuf,
    }

    const RU_CONFIGURATION_XML: &str = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Main</Name><DefaultLanguage>Language.Русский</DefaultLanguage></Properties><ChildObjects><Language>Русский</Language><Document>Order</Document></ChildObjects></Configuration></MetaDataObject>"#;

    fn language_xml(name: &str, code: &str) -> String {
        format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="33333333-3333-4333-8333-333333333333"><Properties><Name>{name}</Name><Comment/><LanguageCode>{code}</LanguageCode></Properties></Language></MetaDataObject>"#
        )
    }

    impl MetadataFixture {
        fn new() -> Self {
            Self::with_files(&[
                ("Configuration.xml", RU_CONFIGURATION_XML),
                ("Languages/Русский.xml", &language_xml("Русский", "ru")),
                ("Documents/Order.xml", ORDER_XML),
            ])
        }

        /// A `main` configuration source set made of `files` (relative
        /// path, text); `descriptor` is `Documents/Order.xml`.
        fn with_files(files: &[(&str, &str)]) -> Self {
            Self::with_kind_and_files(SourceSetKind::Configuration, files)
        }

        fn with_kind_and_files(kind: SourceSetKind, files: &[(&str, &str)]) -> Self {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("src");
            std::fs::create_dir_all(&source).unwrap();
            let project_kind = match kind {
                SourceSetKind::Configuration => "CONFIGURATION",
                SourceSetKind::Extension => "EXTENSION",
                other => panic!("unsupported fixture kind {other:?}"),
            };
            std::fs::write(
                root.path().join("v8project.yaml"),
                format!("format: DESIGNER\nsource-set:\n  - name: main\n    type: {project_kind}\n    path: src\n"),
            )
            .unwrap();
            for (relative, text) in files {
                let path = source.join(relative);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, text).unwrap();
            }
            let descriptor = source.join("Documents/Order.xml");
            let workspace_root = std::fs::canonicalize(root.path()).unwrap();
            let source = std::fs::canonicalize(source).unwrap();
            let context = WorkspaceContext {
                cwd: workspace_root.clone(),
                workspace_root: workspace_root.clone(),
                cache_root: workspace_root.join(".build/unica"),
                workspace_epoch: 1,
            };
            let identity = WorkspaceIdentity::new(
                &context,
                [WorkspaceSourceSetInput::new(
                    "main",
                    &source,
                    kind,
                    SourceFormat::PlatformXml,
                    SourceProfile::platform_xml_8_3_27_format_2_20(),
                )],
                "metadata-family-planner-test",
            )
            .unwrap();
            let actor = Arc::new(WorkspaceActor::new(identity, context).unwrap());
            let binding = actor.bind_provider_root("main", &source).unwrap();
            Self {
                _root: root,
                actor,
                binding,
                descriptor,
            }
        }

        fn admission(&self) -> ApplyAdmission {
            self.actor
                .admit_apply(
                    &self.binding,
                    None,
                    true,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap()
        }

        fn parse(
            &self,
            operation: &str,
            args: serde_json::Value,
            index: usize,
        ) -> IndexedPlanOperation<super::MetadataPlanOperation> {
            IndexedPlanOperation::new(
                index,
                parse_metadata_plan_operation(operation, &args, index, &self.binding).unwrap(),
            )
        }

        fn disk_bytes(&self) -> Vec<u8> {
            std::fs::read(&self.descriptor).unwrap()
        }
    }

    #[test]
    fn metadata_parser_rejects_unknown_and_misplaced_fields_at_the_exact_operation_path() {
        let fixture = MetadataFixture::new();
        let unknown = parse_metadata_plan_operation(
            "props.set",
            &json!({
                "at": "main:Document.Order",
                "values": {"Comment": "typed"},
                "command": "forbidden"
            }),
            3,
            &fixture.binding,
        )
        .unwrap_err();
        assert_eq!(unknown.kind(), ApplyPlanErrorKind::BadValue);
        assert_eq!(unknown.path(), Some("ops[3].args.command"));

        let misplaced = parse_metadata_plan_operation(
            "attribute.remove",
            &json!({
                "at": "main:Document.Order",
                "items": ["Total"]
            }),
            4,
            &fixture.binding,
        )
        .unwrap_err();
        assert_eq!(misplaced.kind(), ApplyPlanErrorKind::BadValue);
        assert_eq!(misplaced.path(), Some("ops[4].args.items"));
    }

    #[test]
    fn object_create_stages_the_template_files_and_registers_the_child() {
        let fixture = MetadataFixture::new();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let parsed = fixture.parse(
            "object.create",
            json!({"at": "main:Configuration", "values": {"kind": "Catalog", "name": "Товары"}}),
            0,
        );
        let (staged, effects) = plan_metadata_batch(staged, authority, &[parsed])
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        assert_eq!(effects.len(), 1);
        let changes = staged.planned_changes();
        let descriptor = changes
            .iter()
            .find(|change| change.relative_path == Path::new("Catalogs/Товары.xml"))
            .expect("the new descriptor is staged");
        assert_eq!(descriptor.kind, StagedChangeKind::Create);
        let owner = changes
            .iter()
            .find(|change| change.relative_path == Path::new("Configuration.xml"))
            .expect("the owner registration is staged");
        assert_eq!(owner.kind, StagedChangeKind::Replace);
        let StagedFileState::Bytes(owner_bytes) = &owner.current else {
            panic!("owner keeps bytes");
        };
        let owner_text = String::from_utf8(owner_bytes.clone()).unwrap();
        assert!(
            owner_text.contains("<Catalog>Товары</Catalog>"),
            "{owner_text}"
        );
        assert!(
            owner_text.contains("<Document>Order</Document>"),
            "{owner_text}"
        );
    }

    #[test]
    fn object_create_then_attribute_add_compose_in_one_batch() {
        let fixture = MetadataFixture::new();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let create = fixture.parse(
            "object.create",
            json!({"at": "main:Configuration", "values": {"kind": "Catalog", "name": "Товары"}}),
            0,
        );
        let add = fixture.parse(
            "attribute.add",
            json!({
                "at": "main:Catalog.Товары",
                "items": [{
                    "name": "Артикул",
                    "type": {"variants": [{"kind": "string", "length": 25, "allowedLength": "variable"}]}
                }]
            }),
            1,
        );
        let (staged, effects) = plan_metadata_batch(staged, authority, &[create, add])
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        assert_eq!(effects.len(), 2);
        let descriptor = staged
            .planned_changes()
            .into_iter()
            .find(|change| change.relative_path == Path::new("Catalogs/Товары.xml"))
            .expect("the new descriptor is staged");
        let StagedFileState::Bytes(bytes) = descriptor.current else {
            panic!("descriptor keeps bytes");
        };
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("<Name>Артикул</Name>"), "{text}");
    }

    #[test]
    fn object_create_refuses_an_existing_object_and_a_non_root_target() {
        let fixture = MetadataFixture::new();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let parsed = fixture.parse(
            "object.create",
            json!({"at": "main:Configuration", "values": {"kind": "Document", "name": "Order"}}),
            0,
        );
        let error = plan_metadata_batch(staged, authority, &[parsed]).unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);
        assert_eq!(error.path(), Some("ops[0].args.values.name"));

        let misplaced = parse_metadata_plan_operation(
            "object.create",
            &json!({"at": "main:Document.Order", "values": {"kind": "Catalog", "name": "X"}}),
            1,
            &fixture.binding,
        )
        .unwrap_err();
        assert_eq!(misplaced.kind(), ApplyPlanErrorKind::BadValue);
        assert_eq!(misplaced.path(), Some("ops[1].args.at"));
    }

    #[test]
    fn object_remove_stages_descriptor_removal_and_owner_deregistration() {
        let fixture = MetadataFixture::new();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let parsed = fixture.parse("object.remove", json!({"at": "main:Document.Order"}), 0);
        let (staged, effects) = plan_metadata_batch(staged, authority, &[parsed])
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        assert_eq!(effects.len(), 1);
        let changes = staged.planned_changes();
        let descriptor = changes
            .iter()
            .find(|change| change.relative_path == Path::new("Documents/Order.xml"))
            .expect("the descriptor removal is staged");
        assert_eq!(descriptor.kind, StagedChangeKind::Remove);
        let owner = changes
            .iter()
            .find(|change| change.relative_path == Path::new("Configuration.xml"))
            .expect("the owner deregistration is staged");
        let StagedFileState::Bytes(owner_bytes) = &owner.current else {
            panic!("owner keeps bytes");
        };
        let owner_text = String::from_utf8(owner_bytes.clone()).unwrap();
        assert!(
            !owner_text.contains("<Document>Order</Document>"),
            "{owner_text}"
        );
    }

    #[test]
    fn object_remove_retains_unchanged_reference_and_subsystem_inputs() {
        for subsystem in [false, true] {
            let fixture = MetadataFixture::new();
            let root = fixture.descriptor.parent().unwrap().parent().unwrap();
            let input = if subsystem {
                root.join("Subsystems/Main.xml")
            } else {
                root.join("CommonModules/Helper/Ext/Module.bsl")
            };
            std::fs::create_dir_all(input.parent().unwrap()).unwrap();
            let original = if subsystem {
                b"<Subsystem><Content/></Subsystem>".as_slice()
            } else {
                b"Procedure Helper()\nEndProcedure".as_slice()
            };
            std::fs::write(&input, original).unwrap();
            let admission = fixture.admission();
            let authority = admission
                .metadata_planning_authority(&fixture.binding)
                .unwrap();
            let operation = fixture.parse("object.remove", json!({"at": "main:Document.Order"}), 0);
            let (staged, _) =
                plan_metadata_batch(admission.staged_state().unwrap(), authority, &[operation])
                    .unwrap();
            let prepared = admission.prepare(staged).unwrap();
            let changed = if subsystem {
                b"<Subsystem><Content><Item>Document.Order</Item></Content></Subsystem>".as_slice()
            } else {
                b"Procedure Helper()\n x = Documents.Order;\nEndProcedure".as_slice()
            };
            std::fs::write(&input, changed).unwrap();
            assert!(fixture.actor.publish_prepared_apply(prepared).is_err());
            assert_eq!(fixture.disk_bytes(), ORDER_XML.as_bytes());
            assert_eq!(std::fs::read(&input).unwrap(), changed);
        }
    }

    #[test]
    fn help_create_stages_the_embedded_help_facet() {
        let fixture = MetadataFixture::new();
        std::fs::create_dir_all(fixture.descriptor.with_extension("").join("Ext")).unwrap();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let parsed = fixture.parse(
            "help.create",
            json!({"at": "main:Document.Order", "values": {"lang": "ru"}}),
            0,
        );
        let (staged, effects) = plan_metadata_batch(staged, authority, &[parsed])
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        assert_eq!(effects.len(), 1);
        let mut created = staged
            .planned_changes()
            .into_iter()
            .filter(|change| change.kind == StagedChangeKind::Create)
            .map(|change| change.relative_path)
            .collect::<Vec<_>>();
        created.sort();
        // Component order: the `Help` directory sorts before `Help.xml`.
        assert_eq!(
            created,
            vec![
                PathBuf::from("Documents/Order/Ext/Help/ru.html"),
                PathBuf::from("Documents/Order/Ext/Help.xml"),
            ]
        );
    }

    #[test]
    fn member_collections_relations_and_help_plan_through_the_typed_engine() {
        let fixture = MetadataFixture::new();
        let cases = [
            (
                "tabularSection.add",
                json!({"at": "main:Document.Order", "items": [{
                    "name": "Строки",
                    "attributes": [{
                        "name": "Сумма",
                        "type": {"variants": [{"kind": "number", "digits": 10, "fraction": 2, "sign": "any"}]}
                    }]
                }]}),
                "<TabularSection",
            ),
            (
                "command.add",
                json!({"at": "main:Document.Order", "items": [{"name": "ПечатьАкта"}]}),
                "<Command",
            ),
            (
                "template.add",
                json!({"at": "main:Document.Order", "items": [{
                    "name": "ПФ_MXL_Акт",
                    "templateType": "SpreadsheetDocument"
                }]}),
                "<Template",
            ),
            (
                "relation.add",
                json!({"at": "main:Document.Order", "values": {
                    "relation": "basedOn",
                    "targets": [{"metadataPath": "Document.Order"}]
                }}),
                "<BasedOn",
            ),
        ];
        for (name, args, marker) in cases {
            let admission = fixture.admission();
            let staged = admission.staged_state().unwrap();
            let authority = admission
                .metadata_planning_authority(&fixture.binding)
                .unwrap();
            let parsed = fixture.parse(name, args, 0);
            let (staged, effects) = plan_metadata_batch(staged, authority, &[parsed])
                .unwrap_or_else(|error| panic!("{name}: {error:?} at {:?}", error.path()));
            assert_eq!(effects.len(), 1, "{name}");
            let changed = staged
                .planned_changes()
                .iter()
                .map(|change| match &change.current {
                    StagedFileState::Bytes(bytes) => String::from_utf8_lossy(bytes).into_owned(),
                    StagedFileState::Absent => String::new(),
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                changed.contains(marker),
                "{name}: staged postimage misses {marker}"
            );
        }
    }

    #[test]
    fn member_collection_removal_takes_the_member_name_from_values() {
        let fixture = MetadataFixture::new();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let add = fixture.parse(
            "tabularSection.add",
            json!({"at": "main:Document.Order", "items": [{"name": "Строки"}]}),
            0,
        );
        let remove = fixture.parse(
            "tabularSection.remove",
            json!({"at": "main:Document.Order", "values": {"name": "Строки"}}),
            1,
        );
        let (staged, effects) = plan_metadata_batch(staged, authority, &[add, remove]).unwrap();
        assert_eq!(effects.len(), 2);
        let postimage = staged
            .planned_changes()
            .iter()
            .map(|change| match &change.current {
                StagedFileState::Bytes(bytes) => String::from_utf8_lossy(bytes).into_owned(),
                StagedFileState::Absent => String::new(),
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !postimage.contains("Строки"),
            "removal after addition leaves no member behind"
        );
    }

    #[test]
    fn metadata_planner_preserves_operation_order_in_one_staged_postimage_without_disk_mutation() {
        let fixture = MetadataFixture::new();
        let disk_preimage = fixture.disk_bytes();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let operations = [
            fixture.parse(
                "props.set",
                json!({"at": "main:Document.Order", "values": {"Comment": "first"}}),
                0,
            ),
            fixture.parse(
                "attribute.add",
                json!({"at": "main:Document.Order", "items": [{"name": "Total"}]}),
                1,
            ),
            fixture.parse(
                "attribute.set",
                json!({
                    "at": "main:Document.Order.Attribute.Total",
                    "values": {"comment": "ordered"}
                }),
                2,
            ),
        ];

        let (staged, effects) = plan_metadata_batch(staged, authority, &operations).unwrap();
        let changes = staged.planned_changes();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].relative_path, Path::new("Documents/Order.xml"));
        assert_eq!(changes[0].kind, StagedChangeKind::Replace);
        assert_eq!(
            changes[0].original,
            StagedFileState::Bytes(disk_preimage.clone())
        );
        let StagedFileState::Bytes(postimage) = &changes[0].current else {
            panic!("metadata edit must stage a descriptor postimage")
        };
        let postimage = String::from_utf8(postimage.clone()).unwrap();
        assert!(postimage.contains("<Comment>first</Comment>"));
        assert!(postimage.contains("<Name>Total</Name>"));
        assert!(postimage.contains("<Comment>ordered</Comment>"));
        assert_eq!(fixture.disk_bytes(), disk_preimage);
        assert_eq!(effects.len(), 3);
        assert_eq!(effects[0].event().artifact, "Document.Order");
    }

    /// Every staged file of a plan, as text by relative path.
    fn staged_texts(
        staged: &crate::infrastructure::native_operations::apply::ApplyStagedState,
    ) -> std::collections::BTreeMap<PathBuf, String> {
        staged
            .planned_changes()
            .into_iter()
            .filter_map(|change| match change.current {
                StagedFileState::Bytes(bytes) => {
                    Some((change.relative_path, String::from_utf8(bytes).unwrap()))
                }
                StagedFileState::Absent => None,
            })
            .collect()
    }

    /// `(lang, content)` of the multilingual property `tag` of the element
    /// whose `Name` is `name`.
    fn ml_values(text: &str, name: &str, tag: &str) -> Vec<(String, String)> {
        const CORE: &str = "http://v8.1c.ru/8.1/data/core";
        let document = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).unwrap();
        let properties = document
            .descendants()
            .filter(|node| node.has_tag_name("Properties"))
            .find(|node| {
                node.children()
                    .any(|child| child.has_tag_name("Name") && child.text() == Some(name))
            })
            .unwrap_or_else(|| panic!("{name} is not described\n{text}"));
        let property = properties
            .children()
            .find(|child| child.is_element() && child.tag_name().name() == tag)
            .unwrap_or_else(|| panic!("{name} has no {tag}\n{text}"));
        property
            .children()
            .filter(|item| item.has_tag_name((CORE, "item")))
            .map(|item| {
                let child = |local: &str| {
                    item.children()
                        .find(|child| child.has_tag_name((CORE, local)))
                        .and_then(|child| child.text())
                        .unwrap_or_default()
                        .to_string()
                };
                (child("lang"), child("content"))
            })
            .collect()
    }

    fn plan_all(
        fixture: &MetadataFixture,
        operations: &[(&str, serde_json::Value)],
    ) -> Result<
        crate::infrastructure::native_operations::apply::ApplyStagedState,
        crate::infrastructure::native_operations::apply::ApplyPlanError,
    > {
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let parsed = operations
            .iter()
            .enumerate()
            .map(|(index, (name, args))| fixture.parse(name, args.clone(), index))
            .collect::<Vec<_>>();
        plan_metadata_batch(staged, authority, &parsed).map(|(staged, _)| staged)
    }

    const KINDS_ENUM_XML: &str = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
	<Enum uuid="66666666-6666-4666-8666-666666666666">
		<Properties>
			<Name>Kinds</Name>
			<Synonym/>
			<Comment/>
		</Properties>
		<ChildObjects/>
	</Enum>
</MetaDataObject>
"#;

    const READER_ROLE_XML: &str = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
	<Role uuid="77777777-7777-4777-8777-777777777777">
		<Properties>
			<Name>Reader</Name>
			<Synonym/>
			<Comment/>
		</Properties>
	</Role>
</MetaDataObject>
"#;

    const SALES_SUBSYSTEM_XML: &str = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20">
	<Subsystem uuid="88888888-8888-4888-8888-888888888888">
		<Properties>
			<Name>Sales</Name>
			<Synonym/>
			<Comment/>
			<IncludeHelpInContents>true</IncludeHelpInContents>
			<IncludeInCommandInterface>true</IncludeInCommandInterface>
			<UseOneCommand>false</UseOneCommand>
			<Explanation/>
			<Picture/>
			<Content/>
		</Properties>
		<ChildObjects/>
	</Subsystem>
</MetaDataObject>
"#;

    #[test]
    fn new_multilingual_values_take_the_configuration_default_language() {
        let fixture = MetadataFixture::with_files(&[
            (
                "Configuration.xml",
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Main</Name><DefaultLanguage>Language.English</DefaultLanguage></Properties><ChildObjects><Language>English</Language><Document>Order</Document><Enum>Kinds</Enum><Role>Reader</Role><Subsystem>Sales</Subsystem></ChildObjects></Configuration></MetaDataObject>"#,
            ),
            ("Languages/English.xml", &language_xml("English", "en")),
            ("Documents/Order.xml", ORDER_XML),
            ("Enums/Kinds.xml", KINDS_ENUM_XML),
            ("Roles/Reader.xml", READER_ROLE_XML),
            ("Subsystems/Sales.xml", SALES_SUBSYSTEM_XML),
        ]);
        let staged = plan_all(
            &fixture,
            &[
                (
                    "props.set",
                    json!({"at": "main:Document.Order", "values": {"Synonym": "Sales order"}}),
                ),
                (
                    "attribute.add",
                    json!({"at": "main:Document.Order", "items": [{"name": "Total"}, {"name": "Note"}]}),
                ),
                (
                    "attribute.set",
                    json!({"at": "main:Document.Order.Attribute.Total", "values": {"synonym": "Grand total"}}),
                ),
                (
                    "tabularSection.add",
                    json!({"at": "main:Document.Order", "items": [{"name": "Lines", "attributes": [{"name": "Qty"}]}]}),
                ),
                (
                    "attribute.add",
                    json!({"at": "main:Document.Order", "scope": {"tabularSection": "Lines"}, "items": [{"name": "Price"}]}),
                ),
                (
                    "command.add",
                    json!({"at": "main:Document.Order", "items": [{"name": "Print"}]}),
                ),
                (
                    "template.add",
                    json!({"at": "main:Document.Order", "items": [{"name": "Layout", "templateType": "SpreadsheetDocument"}]}),
                ),
                (
                    "enumValue.add",
                    json!({"at": "main:Enum.Kinds", "items": [{"name": "Retail"}]}),
                ),
                (
                    "props.set",
                    json!({"at": "main:Role.Reader", "values": {"synonym": "Reader"}}),
                ),
                (
                    "props.set",
                    json!({"at": "main:Subsystem.Sales", "values": {"synonym": "Sales", "explanation": "Sales area"}}),
                ),
            ],
        )
        .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let texts = staged_texts(&staged);
        let text = |relative: &str| {
            texts
                .get(Path::new(relative))
                .unwrap_or_else(|| panic!("{relative} is not staged"))
                .as_str()
        };
        let english = |content: &str| vec![("en".to_string(), content.to_string())];
        let order = text("Documents/Order.xml");
        assert_eq!(ml_values(order, "Order", "Synonym"), english("Sales order"));
        assert_eq!(ml_values(order, "Total", "Synonym"), english("Grand total"));
        assert_eq!(ml_values(order, "Note", "Synonym"), english("Note"));
        assert_eq!(ml_values(order, "Lines", "Synonym"), english("Lines"));
        assert_eq!(ml_values(order, "Qty", "Synonym"), english("Qty"));
        assert_eq!(ml_values(order, "Price", "Synonym"), english("Price"));
        assert_eq!(ml_values(order, "Print", "Synonym"), english("Print"));
        assert_eq!(
            ml_values(
                text("Documents/Order/Templates/Layout.xml"),
                "Layout",
                "Synonym"
            ),
            english("Layout")
        );
        assert_eq!(
            ml_values(text("Enums/Kinds.xml"), "Retail", "Synonym"),
            english("Retail")
        );
        assert_eq!(
            ml_values(text("Roles/Reader.xml"), "Reader", "Synonym"),
            english("Reader")
        );
        let sales = text("Subsystems/Sales.xml");
        assert_eq!(ml_values(sales, "Sales", "Synonym"), english("Sales"));
        assert_eq!(
            ml_values(sales, "Sales", "Explanation"),
            english("Sales area")
        );
        for (relative, text) in &texts {
            assert!(
                !text.contains("<v8:lang>ru</v8:lang>"),
                "{} invents a ru value\n{text}",
                relative.display()
            );
        }
    }

    const ENGLISH_ONLY_CONFIGURATION_XML: &str = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Main</Name><DefaultLanguage>Language.English</DefaultLanguage></Properties><ChildObjects><Language>English</Language><Document>Order</Document></ChildObjects></Configuration></MetaDataObject>"#;

    #[test]
    fn object_create_writes_the_configuration_default_language() {
        let fixture = MetadataFixture::with_files(&[
            ("Configuration.xml", ENGLISH_ONLY_CONFIGURATION_XML),
            ("Languages/English.xml", &language_xml("English", "en")),
            ("Documents/Order.xml", ORDER_XML),
        ]);
        let kinds = [
            ("Catalog", "Catalogs", "Products"),
            ("Document", "Documents", "Invoice"),
            ("Enum", "Enums", "Statuses"),
            ("InformationRegister", "InformationRegisters", "Prices"),
            ("CommonModule", "CommonModules", "Helpers"),
            ("Report", "Reports", "Sales"),
        ];
        let operations = kinds
            .iter()
            .map(|(kind, _, name)| {
                (
                    "object.create",
                    json!({"at": "main:Configuration", "values": {"kind": kind, "name": name}}),
                )
            })
            .collect::<Vec<_>>();
        let staged = plan_all(&fixture, &operations)
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let texts = staged_texts(&staged);
        for (kind, directory, name) in kinds {
            let relative = format!("{directory}/{name}.xml");
            let text = texts
                .get(Path::new(&relative))
                .unwrap_or_else(|| panic!("{kind}: {relative} is not staged"));
            assert_eq!(
                ml_values(text, name, "Synonym"),
                vec![("en".to_string(), name.to_string())],
                "{kind}"
            );
        }
        for (relative, text) in &texts {
            assert!(
                !text.contains("<v8:lang>ru</v8:lang>"),
                "{} invents a ru value\n{text}",
                relative.display()
            );
        }

        let refused = MetadataFixture::with_files(&[
            (
                "Configuration.xml",
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Main</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
            ),
            ("Documents/Order.xml", ORDER_XML),
        ]);
        let error = plan_all(
            &refused,
            &[(
                "object.create",
                json!({"at": "main:Configuration", "values": {"kind": "Catalog", "name": "Products"}}),
            )],
        )
        .unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidSource);
        assert_eq!(error.path(), Some("ops[0].args.at"));
    }

    #[test]
    fn an_extension_writes_new_values_in_its_own_default_language() {
        // The extension's own Configuration.xml and adopted language decide;
        // nothing is read from the configuration it extends.
        let configuration = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="99999999-9999-4999-8999-999999999999"><Properties><ObjectBelonging>Adopted</ObjectBelonging><Name>Ext</Name><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose><NamePrefix>Ext_</NamePrefix><DefaultLanguage>Language.English</DefaultLanguage></Properties><ChildObjects><Language>English</Language><Document>Order</Document></ChildObjects></Configuration></MetaDataObject>"#;
        let language = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="33333333-3333-4333-8333-333333333333"><InternalInfo/><Properties><ObjectBelonging>Adopted</ObjectBelonging><Name>English</Name><Comment/><ExtendedConfigurationObject>44444444-4444-4444-8444-444444444444</ExtendedConfigurationObject><LanguageCode>en</LanguageCode></Properties></Language></MetaDataObject>"#;
        let fixture = MetadataFixture::with_kind_and_files(
            SourceSetKind::Extension,
            &[
                ("Configuration.xml", configuration),
                ("Languages/English.xml", language),
                ("Documents/Order.xml", ORDER_XML),
            ],
        );
        let staged = plan_all(
            &fixture,
            &[
                (
                    "object.create",
                    json!({"at": "main:Configuration", "values": {"kind": "Catalog", "name": "Ext_Products"}}),
                ),
                (
                    "attribute.add",
                    json!({"at": "main:Document.Order", "items": [{"name": "Total"}]}),
                ),
            ],
        )
        .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let texts = staged_texts(&staged);
        assert_eq!(
            ml_values(
                &texts[Path::new("Catalogs/Ext_Products.xml")],
                "Ext_Products",
                "Synonym"
            ),
            vec![("en".to_string(), "Ext_products".to_string())]
        );
        assert_eq!(
            ml_values(&texts[Path::new("Documents/Order.xml")], "Total", "Synonym"),
            vec![("en".to_string(), "Total".to_string())]
        );
    }

    #[test]
    fn editing_a_multilingual_value_changes_only_the_default_language_item() {
        const WORKSPACE: &str = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/acceptance/workspace/src"
        );
        let read =
            |relative: &str| std::fs::read_to_string(Path::new(WORKSPACE).join(relative)).unwrap();
        let language_aware = read("Enums/LanguageAware.xml");
        let item = |lang: &str, content: &str, indent: &str| {
            format!(
                "{indent}<v8:item>\n{indent}\t<v8:lang>{lang}</v8:lang>\n{indent}\t<v8:content>{content}</v8:content>\n{indent}</v8:item>\n"
            )
        };
        let bilingual = |name: &str, ru: &str, en: &str| {
            let indent = "\t\t\t\t\t\t\t";
            format!(
                "<Properties><Name>{name}</Name><Synonym>\n{}{}\t\t\t\t\t\t</Synonym><Comment/>",
                item("ru", ru, indent),
                item("en", en, indent)
            )
        };
        let order = ORDER_XML.replace(
            "<ChildObjects/>\n\t</Document>",
            &format!(
                "<ChildObjects><Attribute uuid=\"22222222-2222-4222-8222-222222222222\">{}</Properties></Attribute><TabularSection uuid=\"33333333-3333-4333-8333-333333333334\"><Properties><Name>Lines</Name><Synonym/><Comment/></Properties><ChildObjects><Attribute uuid=\"44444444-4444-4444-8444-444444444445\">{}</Properties></Attribute></ChildObjects></TabularSection></ChildObjects>\n\t</Document>",
                bilingual("Total", "Итого", "Total"),
                bilingual("Qty", "Количество", "Quantity"),
            ),
        );
        let kinds = KINDS_ENUM_XML.replace(
            "<ChildObjects/>",
            &format!(
                "<ChildObjects><EnumValue uuid=\"55555555-5555-4555-8555-555555555557\">{}</Properties></EnumValue></ChildObjects>",
                bilingual("Retail", "Розница", "Retail")
            ),
        );
        let block = |tag: &str, ru: &str, en: &str| {
            format!(
                "<{tag}>\n{}{}\t\t\t</{tag}>",
                item("ru", ru, "\t\t\t\t"),
                item("en", en, "\t\t\t\t")
            )
        };
        let sales = SALES_SUBSYSTEM_XML
            .replace("<Synonym/>", &block("Synonym", "Продажи", "Sales"))
            .replace(
                "<Explanation/>",
                &block("Explanation", "Раздел продаж", "Sales area"),
            )
            .replace(
                "<Content/>",
                "<Content>\n\t\t\t\t<xr:Item xsi:type=\"xr:MDObjectRef\">Document.Order</xr:Item>\n\t\t\t</Content>",
            )
            .replace(
                "xmlns:xr=",
                "xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:xr=",
            );
        let fixture = MetadataFixture::with_files(&[
            ("Configuration.xml", &read("Configuration.xml")),
            ("Languages/Русский.xml", &read("Languages/Русский.xml")),
            ("Languages/English.xml", &read("Languages/English.xml")),
            ("Enums/LanguageAware.xml", &language_aware),
            ("Enums/Kinds.xml", &kinds),
            ("Documents/Order.xml", &order),
            ("Subsystems/Sales.xml", &sales),
        ]);
        let staged = plan_all(
            &fixture,
            &[
                (
                    "props.set",
                    json!({"at": "main:Enum.LanguageAware", "values": {"Synonym": "Короткое & новое"}}),
                ),
                (
                    "attribute.set",
                    json!({"at": "main:Document.Order.Attribute.Total", "values": {"synonym": "Всего"}}),
                ),
                (
                    "attribute.set",
                    json!({"at": "main:Document.Order.TabularSection.Lines.Attribute.Qty", "values": {"synonym": "Кол-во"}}),
                ),
                (
                    "enumValue.set",
                    json!({"at": "main:Enum.Kinds", "values": {"name": "Retail", "synonym": "Розничная"}}),
                ),
                (
                    "props.set",
                    json!({"at": "main:Subsystem.Sales", "values": {
                        "synonym": "Продажи и маркетинг",
                        "explanation": "Новый раздел",
                        "comment": "Отдел"
                    }}),
                ),
            ],
        )
        .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let texts = staged_texts(&staged);
        // The default language is Русский (`ru`): its content is the only
        // change, and the English items stay byte for byte.
        assert_eq!(
            texts[Path::new("Enums/LanguageAware.xml")],
            language_aware.replace(
                "Очень длинное наименование перечисления для интерфейса команд",
                "Короткое &amp; новое"
            )
        );
        assert_eq!(
            texts[Path::new("Documents/Order.xml")],
            order
                .replace("Итого", "Всего")
                .replace("Количество", "Кол-во")
        );
        assert_eq!(
            texts[Path::new("Enums/Kinds.xml")],
            kinds.replace("Розница", "Розничная")
        );
        // The subsystem is edited in place: English, Picture and Content
        // keep their bytes.
        assert_eq!(
            texts[Path::new("Subsystems/Sales.xml")],
            sales
                .replace(
                    "<v8:content>Продажи</v8:content>",
                    "<v8:content>Продажи и маркетинг</v8:content>"
                )
                .replace("Раздел продаж", "Новый раздел")
                .replace("<Comment/>", "<Comment>Отдел</Comment>")
        );
    }

    #[test]
    fn a_configuration_without_a_default_language_refuses_new_text_but_not_other_edits() {
        let fixture = MetadataFixture::with_files(&[
            (
                "Configuration.xml",
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Main</Name></Properties><ChildObjects><Document>Order</Document></ChildObjects></Configuration></MetaDataObject>"#,
            ),
            ("Documents/Order.xml", ORDER_XML),
        ]);
        let error = plan_all(
            &fixture,
            &[(
                "attribute.add",
                json!({"at": "main:Document.Order", "items": [{"name": "Total"}]}),
            )],
        )
        .unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidSource);
        assert_eq!(error.path(), Some("ops[0].args.at"));
        assert_eq!(fixture.disk_bytes(), ORDER_XML.as_bytes());

        plan_all(
            &fixture,
            &[(
                "props.set",
                json!({"at": "main:Document.Order", "values": {"Comment": "typed"}}),
            )],
        )
        .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
    }

    #[test]
    fn multilingual_property_edits_keep_the_other_languages() {
        let text = "<Properties xmlns:v8=\"http://v8.1c.ru/8.1/data/core\">\n\t\t\t<Name>SalesReader</Name>\n\t\t\t<Synonym>\n\t\t\t\t<v8:item>\n\t\t\t\t\t<v8:lang>en</v8:lang>\n\t\t\t\t\t<v8:content>Sales reader</v8:content>\n\t\t\t\t</v8:item>\n\t\t\t\t<v8:item>\n\t\t\t\t\t<v8:lang>ru</v8:lang>\n\t\t\t\t\t<v8:content>Старое</v8:content>\n\t\t\t\t</v8:item>\n\t\t\t</Synonym>\n\t\t\t<Comment/>\n\t\t</Properties>";
        let updated = set_ml_property(text, "Synonym", "\t\t\t", "ru", "Новое & лучшее").unwrap();
        assert!(updated.contains("<v8:content>Sales reader</v8:content>"));
        assert!(updated.contains("<v8:content>Новое &amp; лучшее</v8:content>"));
        assert!(!updated.contains("Старое"));
        assert_eq!(updated.matches("<v8:item>").count(), 2);

        let english_only = text.replace(
            "\t\t\t\t<v8:item>\n\t\t\t\t\t<v8:lang>ru</v8:lang>\n\t\t\t\t\t<v8:content>Старое</v8:content>\n\t\t\t\t</v8:item>\n",
            "",
        );
        let updated = set_ml_property(&english_only, "Synonym", "\t\t\t", "ru", "Русское").unwrap();
        assert!(updated.contains("<v8:content>Sales reader</v8:content>"));
        assert!(
            updated.contains("<v8:lang>ru</v8:lang>\n\t\t\t\t\t<v8:content>Русское</v8:content>")
        );
        assert_eq!(updated.matches("<v8:item>").count(), 2);

        let empty = "<Properties xmlns:v8=\"http://v8.1c.ru/8.1/data/core\">\n\t\t\t<Name>X</Name>\n\t\t\t<Synonym/>\n\t\t</Properties>";
        let updated = set_ml_property(empty, "Synonym", "\t\t\t", "ru", "Икс").unwrap();
        assert!(updated.contains("<Synonym>\n\t\t\t\t<v8:item>"));
        assert!(updated.contains("<v8:content>Икс</v8:content>"));
        assert!(!updated.contains("<Synonym/>"));
    }

    #[test]
    fn descriptor_images_keep_the_observed_bom_and_line_endings() {
        let source = "<a>\r\n\t<b/>\r\n</a>\r\n";
        let preimage = [b"\xef\xbb\xbf".as_slice(), source.as_bytes()].concat();
        let image =
            preserve_descriptor_image(&preimage, source, "<a>\r\n\t<b/>\n\t<c/>\r\n</a>\r\n");
        assert!(image.starts_with(b"\xef\xbb\xbf"));
        assert_eq!(&image[3..], b"<a>\r\n\t<b/>\r\n\t<c/>\r\n</a>\r\n");
        let plain = preserve_descriptor_image(source.as_bytes(), source, "<a>\n</a>\n");
        assert!(!plain.starts_with(b"\xef\xbb\xbf"));
    }
}
