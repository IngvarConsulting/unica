use super::validate_platform_xml_binding;
use crate::application::metadata::{parse_metadata_request, MetadataOperation, MetadataRequest};
use crate::domain::address::{NodeKind, QualifiedAddress};
use crate::domain::apply::OperationDescriptor;
use crate::domain::events::{DomainEvent, DomainEventKind};
use crate::domain::metadata::{
    MetaCollection, MetaDiagnosticCode, MetaEditOperation, MetadataKind,
};
use crate::domain::metadata_operation::MetadataOperation as RegisteredMetadataOperation;
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
    apply_typed_operations_to_image_with_seed, canonical_typed_command_name,
    meta_default_text_language, meta_edit_object_identity, meta_mltext_property_replacement,
    predefined_item_id, predefined_post_image, typed_command_module_path,
    typed_operations_write_multilingual_text, EXTERNAL_SOURCE_TEXT_LANGUAGE,
};
use crate::infrastructure::native_operations::text_snapshot::{
    LineEnding, LineEndingProfile, SourceTextSnapshot,
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
        /// The member is named by the operation address, not by an
        /// argument: a missing member is the operation's own node, and the
        /// writer's own field names map back to the caller's arguments.
        addressed: Option<AddressedFields>,
    },
    /// Predefined items live in `Ext/Predefined.xml` next to the owner, not
    /// in its descriptor; `set` and `remove` name the item by address, and
    /// the item id is read from the staged file.
    Predefined {
        owner: MetadataAddress,
        kind: MetadataKind,
        change: PredefinedChange,
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

/// Where the caller wrote the fields of an addressed typed edit: the typed
/// writer names them in its own request (`elements[0]`, `names[0]`, `scope`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AddressedFields {
    /// New members in `items`.
    Items,
    /// Changed fields of the member in `values`.
    Values,
    /// No arguments beyond the address.
    None,
}

impl AddressedFields {
    /// The writer field `field` (without its `operations[0].` wrapper) as the
    /// caller's argument path; `None` when it is the member selector itself,
    /// which the address names.
    fn caller_field(self, field: &str) -> Option<String> {
        // In `items` the caller wrote every name, the first included; only a
        // member that the address names has its selector dropped.
        let addressed_selector = field == "elements[0].name" && !matches!(self, Self::Items);
        if field == "names[0]" || addressed_selector || field.starts_with("scope") {
            return None;
        }
        Some(match self {
            Self::Items => field.replacen("elements", "items", 1),
            Self::Values => field.replacen("elements[0]", "values", 1),
            Self::None => field.to_string(),
        })
    }
}

#[derive(Debug, Clone)]
enum PredefinedChange {
    Add(MetaEditOperation),
    Set {
        name: String,
        values: Map<String, Value>,
    },
    Remove {
        name: String,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct MetadataPlanOperation {
    kind: MetadataPlanKind,
}

#[cfg(test)]
pub(crate) fn parse_metadata_plan_operation(
    operation: &str,
    args: &Value,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanOperation, ApplyPlanError> {
    let at = args
        .get("at")
        .and_then(Value::as_str)
        .and_then(|raw| QualifiedAddress::resolve_input(raw, &[binding.source_set_name()]).ok());
    let descriptor = super::resolved_descriptor(operation, at.as_ref(), binding);
    parse_metadata_plan_operation_in(descriptor, operation, args, op_index, binding)
}

pub(crate) fn parse_metadata_plan_operation_in(
    descriptor: Option<OperationDescriptor>,
    operation: &str,
    args: &Value,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanOperation, ApplyPlanError> {
    validate_platform_xml_binding(binding, operation, op_index)?;
    let object = args.as_object().ok_or_else(|| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "operation args must be an object",
        )
        .at_path(format!("ops[{op_index}].args"))
    })?;
    if let Some(operation) = descriptor.and_then(OperationDescriptor::metadata_operation) {
        let kind = match operation {
            RegisteredMetadataOperation::AttributeAdd => {
                parse_attribute_add(object, op_index, binding)?
            }
            RegisteredMetadataOperation::AttributeSet => {
                parse_attribute_set(object, op_index, binding)?
            }
            RegisteredMetadataOperation::AttributeRemove => {
                parse_attribute_remove(object, op_index, binding)?
            }
            RegisteredMetadataOperation::CommandAdd => {
                parse_member_add(operation.name(), "commands", object, op_index, binding)?
            }
            RegisteredMetadataOperation::CommandSet => parse_addressed_member_change(
                operation.name(),
                NodeKind::Command,
                MetaCollection::Commands,
                AddressedMemberChange::Set,
                object,
                op_index,
                binding,
            )?,
            RegisteredMetadataOperation::CommandRemove => parse_addressed_member_change(
                operation.name(),
                NodeKind::Command,
                MetaCollection::Commands,
                AddressedMemberChange::Remove,
                object,
                op_index,
                binding,
            )?,
        };
        return Ok(MetadataPlanOperation { kind });
    }
    let kind = match operation {
        "props.set" => parse_props_set(object, op_index, binding)?,
        "tabularSection.add" => {
            parse_member_add(operation, "tabularSections", object, op_index, binding)?
        }
        "tabularSection.set"
        | "tabularSection.remove"
        | "dimension.set"
        | "dimension.remove"
        | "resource.set"
        | "resource.remove"
        | "enumValue.set"
        | "enumValue.remove"
        | "column.set"
        | "column.remove" => parse_addressed_member(operation, object, op_index, binding)?,
        "dimension.add" => parse_member_add(operation, "dimensions", object, op_index, binding)?,
        "resource.add" => parse_member_add(operation, "resources", object, op_index, binding)?,
        "enumValue.add" => parse_member_add(operation, "enumValues", object, op_index, binding)?,
        "column.add" => parse_member_add(operation, "columns", object, op_index, binding)?,
        "template.add" => parse_template_add(object, op_index, binding)?,
        "template.set" => parse_template_set(object, op_index, binding)?,
        "template.remove" => parse_template_remove(object, op_index, binding)?,
        "predefinedItem.add" => parse_predefined_add(object, op_index, binding)?,
        "predefinedItem.set" => parse_predefined_set(object, op_index, binding)?,
        "predefinedItem.remove" => parse_predefined_remove(object, op_index, binding)?,
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

/// `template.set`/`template.remove` name the template by its address
/// `Owner.Name.Template.T`; the owner alone no longer selects a template.
fn template_target(
    target: &QualifiedAddress,
    op_index: usize,
) -> Result<(MetadataAddress, String), ApplyPlanError> {
    let (owner, _, name) = metadata_member_target(target, NodeKind::Template, op_index)?;
    Ok((owner, name))
}

fn parse_template_set(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    reject_unknown_args("template.set", args, &["at", "values"], op_index)?;
    let target = qualified_target(args, op_index, binding)?;
    let (owner, name) = template_target(&target, op_index)?;
    let values = required_object(args, "values", op_index)?.clone();
    if let Some(field) = values
        .keys()
        .find(|field| !["synonym", "comment", "templateType"].contains(&field.as_str()))
    {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!(
                "template.set values accept `synonym`, `comment` and `templateType`, not `{field}`; the template is named by its address"
            ),
        )
        .at_path(format!("ops[{op_index}].args.values.{field}")));
    }
    if values.is_empty() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "template.set values must change at least one property",
        )
        .at_path(format!("ops[{op_index}].args.values")));
    }
    for (field, value) in &values {
        let location = format!("ops[{op_index}].args.values.{field}");
        let Some(text) = value.as_str() else {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::BadValue,
                format!("`{field}` must be a string"),
            )
            .at_path(location));
        };
        if field == "templateType" {
            crate::domain::metadata::MetaTemplateKind::parse(text).map_err(|diagnostic| {
                ApplyPlanError::new(ApplyPlanErrorKind::BadValue, diagnostic.message)
                    .at_path(location.clone())
            })?;
        }
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
    reject_unknown_args("template.remove", args, &["at"], op_index)?;
    let target = qualified_target(args, op_index, binding)?;
    let (owner, name) = template_target(&target, op_index)?;
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
        line_range: None,
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
    Ok(MetadataPlanKind::Edit {
        target,
        operation,
        addressed: None,
    })
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

/// `attribute.add` on the owner adds header attributes; on
/// `Owner.Name.TabularSection.T` it adds attributes of that tabular section.
fn parse_attribute_add(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    reject_unknown_args("attribute.add", args, &["at", "items"], op_index)?;
    let target = qualified_target(args, op_index, binding)?;
    let (owner, kind, section) = match target.segments() {
        [_] => {
            let (owner, kind) = metadata_owner(&target, op_index)?;
            (owner, kind, None)
        }
        [_, section] if section.kind() == NodeKind::TabularSection => {
            let (owner, kind, name) =
                metadata_member_target(&target, NodeKind::TabularSection, op_index)?;
            (owner, kind, Some(name))
        }
        _ => {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::BadValue,
                "attribute.add target must be a metadata object or its `TabularSection.<Name>`",
            )
            .at_path(format!("ops[{op_index}].args.at")))
        }
    };
    let items = required_array(args, "items", op_index)?.to_vec();
    let mut legacy = json!({"op": "add", "collection": "attributes", "elements": items});
    let in_section = section.is_some();
    if let Some(section) = section {
        legacy["scope"] = json!({"tabularSection": section});
    }
    let edit = parse_legacy_edit(
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
    )?;
    // Табличная часть названа адресом: её отсутствие — «не найден» по `at`.
    Ok(if in_section {
        addressed_edit(edit, AddressedFields::Items)
    } else {
        edit
    })
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
    Ok(addressed_edit(
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
        )?,
        AddressedFields::Values,
    ))
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
    Ok(addressed_edit(
        parse_legacy_edit(
            binding.source_set_name(),
            owner,
            kind,
            legacy,
            |field| format!("ops[{op_index}].args.{field}"),
            op_index,
        )?,
        AddressedFields::None,
    ))
}

/// One member-collection add: `at` names the owner, `items` carry the new
/// elements exactly as the typed metadata contract defines them.
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

/// The node kind and typed collection of an addressed member operation.
fn addressed_member(operation: &str) -> (NodeKind, MetaCollection) {
    match operation.split_once('.').map(|(family, _)| family) {
        Some("tabularSection") => (NodeKind::TabularSection, MetaCollection::TabularSections),
        Some("dimension") => (NodeKind::Dimension, MetaCollection::Dimensions),
        Some("resource") => (NodeKind::Resource, MetaCollection::Resources),
        Some("enumValue") => (NodeKind::EnumValue, MetaCollection::EnumValues),
        Some("column") => (NodeKind::Column, MetaCollection::Columns),
        _ => unreachable!("only addressed member operations reach this parser"),
    }
}

/// `<member>.set` and `<member>.remove` act on `Owner.Name.<Kind>.<Member>`:
/// the address names the member, `set` carries only the changed fields in
/// `values` (a rename is `newName`), and `remove` carries no arguments.
fn parse_addressed_member(
    operation: &str,
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    let (member_kind, collection) = addressed_member(operation);
    let change = if operation.ends_with(".remove") {
        AddressedMemberChange::Remove
    } else {
        AddressedMemberChange::Set
    };
    parse_addressed_member_change(
        operation,
        member_kind,
        collection,
        change,
        args,
        op_index,
        binding,
    )
}

#[derive(Debug, Clone, Copy)]
enum AddressedMemberChange {
    Set,
    Remove,
}

fn parse_addressed_member_change(
    operation: &str,
    member_kind: NodeKind,
    collection: MetaCollection,
    change: AddressedMemberChange,
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    let allowed: &[&str] = match change {
        AddressedMemberChange::Set => &["at", "values"],
        AddressedMemberChange::Remove => &["at"],
    };
    reject_unknown_args(operation, args, allowed, op_index)?;
    let target = qualified_target(args, op_index, binding)?;
    let (owner, kind, name) = metadata_member_target(&target, member_kind, op_index)?;
    if matches!(change, AddressedMemberChange::Remove) {
        return Ok(addressed_edit(
            parse_legacy_edit(
                binding.source_set_name(),
                owner,
                kind,
                json!({"op": "remove", "collection": collection.as_str(), "names": [name]}),
                |field| format!("ops[{op_index}].args.{field}"),
                op_index,
            )?,
            AddressedFields::None,
        ));
    }
    let mut values = required_object(args, "values", op_index)?.clone();
    if values.contains_key("name") {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!("`{operation}` names the member by its address; rename it with `newName`"),
        )
        .at_path(format!("ops[{op_index}].args.values.name")));
    }
    if values.is_empty() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!("`{operation}` values must change at least one field"),
        )
        .at_path(format!("ops[{op_index}].args.values")));
    }
    values.insert("name".to_string(), Value::String(name));
    Ok(addressed_edit(
        parse_legacy_edit(
            binding.source_set_name(),
            owner,
            kind,
            json!({"op": "update", "collection": collection.as_str(), "elements": [values]}),
            |field| {
                format!(
                    "ops[{op_index}].args.{}",
                    field.replacen("elements[0]", "values", 1)
                )
            },
            op_index,
        )?,
        AddressedFields::Values,
    ))
}

/// Marks a typed edit whose member is named by the operation address.
fn addressed_edit(kind: MetadataPlanKind, fields: AddressedFields) -> MetadataPlanKind {
    match kind {
        MetadataPlanKind::Edit {
            target, operation, ..
        } => MetadataPlanKind::Edit {
            target,
            operation,
            addressed: Some(fields),
        },
        other => other,
    }
}

/// `Owner.Name.<Kind>.<Member>`: the owner, its kind and the member name.
fn metadata_member_target(
    target: &QualifiedAddress,
    member_kind: NodeKind,
    op_index: usize,
) -> Result<(MetadataAddress, MetadataKind, String), ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    let [owner, member] = target.segments() else {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!(
                "operation target must be `Owner.Name.{}.<Name>`",
                member_kind.as_str()
            ),
        )
        .at_path(at_path));
    };
    if member.kind() != member_kind {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!(
                "operation target must end in a `{}` leaf",
                member_kind.as_str()
            ),
        )
        .at_path(at_path));
    }
    let name = member.name().ok_or_else(|| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!("the `{}` leaf must be named", member_kind.as_str()),
        )
        .at_path(at_path.clone())
    })?;
    let owner_target = QualifiedAddress {
        line_range: None,
        source_set: target.source_set().to_string(),
        segments: vec![owner.clone()],
    };
    let (owner, kind) = metadata_owner(&owner_target, op_index)?;
    Ok((owner, kind, name.to_string()))
}

/// The typed operation inside a parsed edit.
fn edit_operation(kind: MetadataPlanKind) -> MetaEditOperation {
    match kind {
        MetadataPlanKind::Edit { operation, .. } => operation,
        _ => unreachable!("the typed edit parser returns an edit"),
    }
}

/// `predefinedItem.add` on the owner: new items carry their `id` and `name`.
fn parse_predefined_add(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    reject_unknown_args("predefinedItem.add", args, &["at", "items"], op_index)?;
    let target = qualified_target(args, op_index, binding)?;
    let (owner, kind) = metadata_owner(&target, op_index)?;
    let items = required_array(args, "items", op_index)?.to_vec();
    let operation = edit_operation(parse_legacy_edit(
        binding.source_set_name(),
        owner.clone(),
        kind,
        json!({"op": "add", "collection": "predefinedItems", "elements": items}),
        |field| {
            format!(
                "ops[{op_index}].args.{}",
                field.replacen("elements", "items", 1)
            )
        },
        op_index,
    )?);
    Ok(MetadataPlanKind::Predefined {
        owner,
        kind,
        change: PredefinedChange::Add(operation),
    })
}

/// `predefinedItem.set` on `Owner.Name.PredefinedItem.<Name>`: `values`
/// carry the changed fields; a rename is `newName`. The id is not an input.
fn parse_predefined_set(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    reject_unknown_args("predefinedItem.set", args, &["at", "values"], op_index)?;
    let target = qualified_target(args, op_index, binding)?;
    let (owner, kind, name) = metadata_member_target(&target, NodeKind::PredefinedItem, op_index)?;
    let values = required_object(args, "values", op_index)?.clone();
    for (field, message) in [
        (
            "id",
            "the predefined item is named by its address, not by `id`",
        ),
        (
            "name",
            "the predefined item is named by its address; rename it with `newName`",
        ),
    ] {
        if values.contains_key(field) {
            return Err(ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message)
                .at_path(format!("ops[{op_index}].args.values.{field}")));
        }
    }
    if values.is_empty() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "`predefinedItem.set` values must change at least one field",
        )
        .at_path(format!("ops[{op_index}].args.values")));
    }
    // The typed contract is checked now, with a placeholder id: argument
    // errors do not wait for the staged file that resolves the real one.
    predefined_update(
        binding.source_set_name(),
        &owner,
        kind,
        PREDEFINED_PLACEHOLDER_ID,
        &values,
        op_index,
    )?;
    Ok(MetadataPlanKind::Predefined {
        owner,
        kind,
        change: PredefinedChange::Set { name, values },
    })
}

fn parse_predefined_remove(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    reject_unknown_args("predefinedItem.remove", args, &["at"], op_index)?;
    let target = qualified_target(args, op_index, binding)?;
    let (owner, kind, name) = metadata_member_target(&target, NodeKind::PredefinedItem, op_index)?;
    Ok(MetadataPlanKind::Predefined {
        owner,
        kind,
        change: PredefinedChange::Remove { name },
    })
}

const PREDEFINED_PLACEHOLDER_ID: &str = "00000000-0000-4000-8000-000000000000";

/// The typed update of one predefined item: `values` with `newName` mapped to
/// the item's `name` field, the item selected by `id`.
fn predefined_update(
    source_set: &str,
    owner: &MetadataAddress,
    kind: MetadataKind,
    id: &str,
    values: &Map<String, Value>,
    op_index: usize,
) -> Result<MetaEditOperation, ApplyPlanError> {
    let mut element = values.clone();
    if let Some(new_name) = element.remove("newName") {
        element.insert("name".to_string(), new_name);
    }
    element.insert("id".to_string(), Value::String(id.to_string()));
    Ok(edit_operation(parse_legacy_edit(
        source_set,
        owner.clone(),
        kind,
        json!({"op": "update", "collection": "predefinedItems", "elements": [element]}),
        |field| {
            let field = field.replacen("elements[0]", "values", 1);
            let field = if field == "values.name" {
                "values.newName".to_string()
            } else {
                field
            };
            format!("ops[{op_index}].args.{field}")
        },
        op_index,
    )?))
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
        let (target, edit, addressed) = match &operation.operation().kind {
            MetadataPlanKind::Edit {
                target,
                operation: edit,
                addressed,
            } => (target, edit, *addressed),
            MetadataPlanKind::Predefined {
                owner,
                kind,
                change,
            } => {
                stage_predefined(
                    &mut staged,
                    &authority,
                    owner,
                    *kind,
                    change,
                    op_index,
                    &mut provisional,
                )?;
                continue;
            }
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
        let source = std::str::from_utf8(body).map_err(|_| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                "metadata descriptor is not UTF-8",
            )
            .at_path(format!("ops[{op_index}].args.at"))
        })?;
        let mut postimage = source.to_string();
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
            // The member named by the address is the operation's own node:
            // when the writer cannot find it, the refusal points at `at`. Other
            // fields map back to where the caller wrote them.
            let path = match (addressed, diagnostic.field) {
                (Some(fields), field) => {
                    let field = field.map(|field| {
                        field
                            .strip_prefix("operations[0].")
                            .map(str::to_string)
                            .unwrap_or(field)
                    });
                    match field.as_deref().map(|field| fields.caller_field(field)) {
                        None | Some(None) if kind == ApplyPlanErrorKind::NotFound => {
                            format!("ops[{op_index}].args.at")
                        }
                        Some(Some(field)) => format!("ops[{op_index}].args.{field}"),
                        _ => format!("ops[{op_index}].args"),
                    }
                }
                (None, None) => format!("ops[{op_index}].args"),
                (None, Some(field)) => format!("ops[{op_index}].args.{field}"),
            };
            ApplyPlanError::new(kind, diagnostic.message).at_path(path)
        })?;
        stage_command_module_lifecycle(
            &mut staged,
            &authority,
            target,
            &relative,
            source,
            edit,
            op_index,
            &mut provisional,
        )?;
        // The typed writer inserts `\n`-separated lines. A descriptor with one
        // observed line ending keeps it (a platform export writes CRLF); a
        // mixed or line-free one is left as written, never normalized
        // (INV.SOURCE.OBSERVED-BYTES).
        let postimage = match SourceTextSnapshot::from_bytes(&preimage)
            .map(|snapshot| snapshot.line_endings())
        {
            Ok(LineEndingProfile::Uniform(LineEnding::CrLf)) => {
                postimage.replace("\r\n", "\n").replace('\n', "\r\n")
            }
            Ok(LineEndingProfile::Uniform(LineEnding::Cr)) => postimage.replace('\n', "\r"),
            _ => postimage,
        };
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

#[allow(clippy::too_many_arguments)]
fn stage_command_module_lifecycle(
    staged: &mut ApplyStagedState,
    authority: &MetadataApplyAuthority<'_>,
    owner: &MetadataAddress,
    descriptor_relative: &Path,
    owner_preimage: &str,
    edit: &MetaEditOperation,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let canonical_name = |name: &str| {
        canonical_typed_command_name(owner_preimage, name)
            .map_err(|failure| meta_failure_to_plan_error(failure, op_index))
    };
    let changes = match edit {
        MetaEditOperation::Update {
            collection: MetaCollection::Commands,
            elements,
            ..
        } => elements
            .iter()
            .filter_map(|element| {
                element.new_name.as_ref().map(|new_name| {
                    canonical_name(&element.name).map(|old_name| (old_name, Some(new_name.clone())))
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
        MetaEditOperation::Remove {
            collection: MetaCollection::Commands,
            names,
            ..
        } => names
            .iter()
            .map(|name| canonical_name(name).map(|name| (name, None)))
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Ok(()),
    };
    let module_event = |name: &str| {
        DomainEvent::new(
            DomainEventKind::ModuleChanged,
            format!(
                "{}:{}.Command.{name}.Module.Command",
                authority.source_set_name(),
                owner.as_str()
            ),
        )
    };
    for (old_name, new_name) in changes {
        if new_name.as_deref() == Some(old_name.as_str()) {
            continue;
        }
        let at_path = format!("ops[{op_index}].args.at");
        let old_relative = typed_command_module_path(descriptor_relative, &old_name);
        let module = staged
            .read(&old_relative)
            .map_err(|error| ApplyPlanError::staging(error, &at_path))?;
        if let Some(bytes) = &module {
            let effect_path = staged
                .retained_relative_path(&old_relative)
                .map_err(|error| ApplyPlanError::staging(error, &at_path))?;
            staged
                .remove(&old_relative, bytes)
                .map_err(|error| ApplyPlanError::staging(error, &at_path))?;
            provisional.push(ProvisionalApplyEffect::single(
                effect_path,
                module_event(&old_name),
                op_index,
            ));
        }
        if let Some(new_name) = new_name {
            let new_relative = typed_command_module_path(descriptor_relative, &new_name);
            let new_name_path = format!("ops[{op_index}].args.values.newName");
            // Probe after the staged removal: aliases under the retained
            // filesystem's name policy share that absent overlay entry.
            // A distinct orphan module must still refuse even when this
            // command had no module to move.
            if staged
                .read(&new_relative)
                .map_err(|error| ApplyPlanError::staging(error, &new_name_path))?
                .is_some()
            {
                return Err(ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidState,
                    format!(
                        "the command module destination already exists: {}",
                        new_relative.to_string_lossy().replace('\\', "/")
                    ),
                )
                .at_path(new_name_path));
            }
            if let Some(bytes) = module {
                staged
                    .create(&new_relative, bytes)
                    .map_err(|error| ApplyPlanError::staging(error, &new_name_path))?;
                staged
                    .require_exact_path_spelling(&new_relative)
                    .map_err(|error| ApplyPlanError::staging(error, &new_name_path))?;
                let effect_path = staged
                    .retained_relative_path(&new_relative)
                    .map_err(|error| ApplyPlanError::staging(error, new_name_path))?;
                provisional.push(ProvisionalApplyEffect::single(
                    effect_path,
                    module_event(&new_name),
                    op_index,
                ));
                if staged
                    .read(&old_relative)
                    .map_err(|error| ApplyPlanError::staging(error, &at_path))?
                    .is_some()
                {
                    // A case alias can change the logical module address
                    // without moving bytes on this filesystem. The owner's
                    // final delta still invalidates the BSL index.
                    provisional.push(ProvisionalApplyEffect::single(
                        descriptor_relative,
                        module_event(&new_name),
                        op_index,
                    ));
                }
            }
        }
    }
    Ok(())
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
                // An earlier operation of this batch may have staged its own
                // change of the guarded prerequisite (a register adds itself
                // to the registrar a journal then guards). The guard binds the
                // on-disk image the template read, so it holds while that
                // image is still the one this plan retained (#1332).
                let unchanged_on_disk =
                    current.is_some() && staged.original_image(&relative) == Some(expected);
                if current.as_deref() != Some(expected) && !unchanged_on_disk {
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
                let preimage = file.preimage.as_deref().unwrap_or(&file.bytes);
                let current = staged
                    .read(&relative)
                    .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
                // The template derived its replacement from the file on disk.
                // An earlier operation of this batch may already have staged
                // its own change of the same prerequisite (#1332): two new
                // registers share one registrar document. Re-derive the edit
                // from that staged image, but only while the on-disk image the
                // template read is still the one this plan retained.
                let replacement = match (current.as_deref(), &file.edit) {
                    (Some(current), _) if current == preimage => file.bytes.clone(),
                    (Some(current), Some(edit))
                        if staged.original_image(&relative) == Some(preimage) =>
                    {
                        edit.apply(current).map_err(|_| {
                            ApplyPlanError::new(
                                ApplyPlanErrorKind::InvalidState,
                                format!(
                                    "prerequisite `{}` cannot accept the change of the new object",
                                    relative.display()
                                ),
                            )
                            .at_path(at_path.clone())
                        })?
                    }
                    _ => {
                        return Err(ApplyPlanError::new(
                            ApplyPlanErrorKind::InvalidState,
                            format!(
                                "prerequisite `{}` changed while planning the new object",
                                relative.display()
                            ),
                        )
                        .at_path(at_path));
                    }
                };
                let current = current.expect("a matched prerequisite image is present");
                staged
                    .replace(&relative, &current, replacement)
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
/// closed at two tabs) in the platform slot of its kind (#1385): before the
/// first entry of a kind the export writes later, else after the last
/// sibling of the same tag when there is one.
fn register_owner_child_ref(text: &str, tag: &str, name: &str) -> Option<String> {
    let entry = format!("\t\t\t<{tag}>{name}</{tag}>\n");
    let mut updated = text.to_string();
    if let Some(offset) =
        crate::infrastructure::native_operations::meta::platform_child_object_insertion_offset(
            text, tag,
        )
    {
        updated.insert_str(offset, &entry);
        return Some(updated);
    }
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

/// Stage one predefined-item change in the owner's `Ext/Predefined.xml`.
///
/// The file is read from the staged state, so an item added earlier in the
/// batch is visible to a later `set` or `remove` of the same batch. `set` and
/// `remove` find the item by the name in their address; the platform keeps
/// predefined names unique within the owner, nested items included.
fn stage_predefined(
    staged: &mut ApplyStagedState,
    authority: &MetadataApplyAuthority<'_>,
    owner: &MetadataAddress,
    kind: MetadataKind,
    change: &PredefinedChange,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    let (owner_relative, _, owner_source) =
        owner_descriptor_and_text(staged, authority, owner, op_index)?;
    // Коды предопределённых проверяются по `CodeType` владельца; описание
    // без него (заимствованный справочник платформа выгружает так) не даёт
    // вида кодов, и запись отказывает тем же текстом, что и чтение.
    crate::infrastructure::native_operations::meta::require_predefined_code_type(
        kind,
        &owner_source,
    )
    .map_err(|message| {
        ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message).at_path(at_path.clone())
    })?;
    let relative = owner_relative.with_extension("").join("Ext/Predefined.xml");
    let original = staged
        .read(&relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
    let item_id = |name: &str| -> Result<String, ApplyPlanError> {
        let found = match &original {
            Some(bytes) => predefined_item_id(bytes, kind, name).map_err(|message| {
                ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message)
                    .at_path(at_path.clone())
            })?,
            None => None,
        };
        found.ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                format!("`{}` has no predefined item `{name}`", owner.as_str()),
            )
            .at_path(at_path.clone())
        })
    };
    let operation = match change {
        PredefinedChange::Add(operation) => operation.clone(),
        PredefinedChange::Set { name, values } => predefined_update(
            authority.source_set_name(),
            owner,
            kind,
            &item_id(name)?,
            values,
            op_index,
        )?,
        PredefinedChange::Remove { name } => edit_operation(parse_legacy_edit(
            authority.source_set_name(),
            owner.clone(),
            kind,
            json!({"op": "remove", "collection": "predefinedItems", "ids": [item_id(name)?]}),
            |field| format!("ops[{op_index}].args.{field}"),
            op_index,
        )?),
    };
    let post_image =
        predefined_post_image(kind, owner, &owner_source, original.as_deref(), &operation)
            .map_err(|failure| {
                // A name or id already taken is the caller's to change: the
                // refusal asks for another value rather than for a human.
                let duplicate = failure
                    .diagnostics
                    .first()
                    .filter(|diagnostic| diagnostic.code == MetaDiagnosticCode::AlreadyExists)
                    .map(|diagnostic| diagnostic.message.clone());
                let error = meta_failure_to_plan_error(failure, op_index);
                let error = match (duplicate, error.path().map(str::to_string)) {
                    (Some(message), Some(path)) => {
                        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(path)
                    }
                    _ => error,
                };
                // The typed writer names fields of its own request (`elements[0]`,
                // `ids[0]`); the caller wrote `items`, `values` or the address.
                let path = error
                    .path()
                    .map(|path| match change {
                        PredefinedChange::Add(_) => {
                            path.replacen("args.values.elements[", "args.items[", 1)
                        }
                        PredefinedChange::Set { .. } => path
                            .replacen("args.values.elements[0].name", "args.values.newName", 1)
                            .replacen("args.values.elements[0].id", "args.at", 1)
                            .replacen("args.values.elements[0]", "args.values", 1),
                        PredefinedChange::Remove { .. } => {
                            path.replacen("args.values.ids[0]", "args.at", 1)
                        }
                    })
                    .unwrap_or_else(|| format!("ops[{op_index}].args"));
                error.at_path(path)
            })?;
    let changed = match (&original, post_image) {
        (None, None) => false,
        (None, Some(post)) => {
            staged
                .create(&relative, post)
                .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
            true
        }
        (Some(pre), Some(post)) if *pre != post => {
            staged
                .replace(&relative, pre, post)
                .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
            true
        }
        (Some(_), Some(_)) => false,
        (Some(pre), None) => {
            staged
                .remove(&relative, pre)
                .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
            true
        }
    };
    if changed {
        provisional.push(ProvisionalApplyEffect::single(
            relative,
            DomainEvent::new(DomainEventKind::MetadataChanged, owner.as_str().to_string()),
            op_index,
        ));
    }
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
    fn the_first_new_item_name_stays_the_callers_field() {
        use super::AddressedFields;
        assert_eq!(
            AddressedFields::Items
                .caller_field("elements[0].name")
                .as_deref(),
            Some("items[0].name")
        );
        assert_eq!(
            AddressedFields::Items
                .caller_field("elements[1].name")
                .as_deref(),
            Some("items[1].name")
        );
        assert_eq!(
            AddressedFields::Values.caller_field("elements[0].name"),
            None
        );
        assert_eq!(AddressedFields::None.caller_field("elements[0].name"), None);
        assert_eq!(AddressedFields::Items.caller_field("names[0]"), None);
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
    fn typed_member_additions_keep_the_owner_collection_and_section() {
        use super::{AddressedFields, MetadataPlanKind};
        use crate::domain::metadata::{MetaCollection, MetaEditOperation};
        let fixture = MetadataFixture::new();
        for (operation, at, collection, section) in [
            (
                "attribute.add",
                "Document.Order",
                MetaCollection::Attributes,
                None,
            ),
            (
                "attribute.add",
                "main:Document.Order.TabularSection.Lines",
                MetaCollection::Attributes,
                Some("Lines"),
            ),
            (
                "command.add",
                "Document.Order",
                MetaCollection::Commands,
                None,
            ),
        ] {
            let parsed = parse_metadata_plan_operation(
                operation,
                &json!({"at":at,"items":[{"name":"Added"}]}),
                3,
                &fixture.binding,
            )
            .unwrap();
            let MetadataPlanKind::Edit {
                target,
                operation:
                    MetaEditOperation::Add {
                        collection: actual,
                        scope,
                        elements,
                    },
                addressed,
            } = parsed.kind
            else {
                panic!("{operation} must plan a typed metadata addition");
            };
            assert_eq!(target.as_str(), "Document.Order");
            assert_eq!(actual, collection);
            assert_eq!(
                scope.as_ref().map(|scope| scope.tabular_section.as_str()),
                section
            );
            assert_eq!(elements.len(), 1);
            assert_eq!(elements[0].name, "Added");
            assert_eq!(addressed, section.map(|_| AddressedFields::Items));
        }
    }

    #[test]
    fn typed_metadata_additions_keep_writer_parser_refusal_order() {
        let fixture = MetadataFixture::new();
        for operation in ["attribute.add", "command.add"] {
            let error = parse_metadata_plan_operation(
                operation,
                &json!({"at":null,"items":false,"unknown":true}),
                3,
                &fixture.binding,
            )
            .unwrap_err();
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
            assert_eq!(error.path(), Some("ops[3].args.unknown"));
            assert_eq!(
                error.to_string(),
                format!(
                    "operation does not accept argument `unknown`; `{operation}` expects `items`"
                )
            );
            let error = parse_metadata_plan_operation(
                operation,
                &json!({"at":42,"items":false}),
                3,
                &fixture.binding,
            )
            .unwrap_err();
            assert_eq!(error.path(), Some("ops[3].args.at"));
            assert_eq!(error.to_string(), "`at` must be a logical address");
            let error = parse_metadata_plan_operation(
                operation,
                &json!({"at":"main:Document.Order","items":[{"name":" spaced "}]}),
                3,
                &fixture.binding,
            )
            .unwrap_err();
            assert_eq!(error.path(), Some("ops[3].args.items[0].name"));
        }
    }

    #[test]
    fn typed_attribute_changes_keep_the_member_owner_scope_and_caller_fields() {
        use super::{AddressedFields, MetadataPlanKind};
        use crate::domain::metadata::{MetaCollection, MetaEditOperation};
        let fixture = MetadataFixture::new();
        for (at, section) in [
            ("Document.Order.Attribute.Total", None),
            (
                "main:Document.Order.TabularSection.Lines.Attribute.Total",
                Some("Lines"),
            ),
        ] {
            let parsed = parse_metadata_plan_operation(
                "attribute.set",
                &json!({"at":at,"values":{"newName":"Changed","comment":"kept"}}),
                3,
                &fixture.binding,
            )
            .unwrap();
            let MetadataPlanKind::Edit {
                target,
                operation:
                    MetaEditOperation::Update {
                        collection,
                        scope,
                        elements,
                    },
                addressed,
            } = parsed.kind
            else {
                panic!("attribute.set must plan a typed update");
            };
            assert_eq!(target.as_str(), "Document.Order");
            assert_eq!(collection, MetaCollection::Attributes);
            assert_eq!(
                scope.as_ref().map(|scope| scope.tabular_section.as_str()),
                section
            );
            assert_eq!(elements.len(), 1);
            assert_eq!(elements[0].name, "Total");
            assert_eq!(elements[0].new_name.as_deref(), Some("Changed"));
            assert_eq!(elements[0].comment.as_deref(), Some("kept"));
            assert_eq!(addressed, Some(AddressedFields::Values));

            let parsed = parse_metadata_plan_operation(
                "attribute.remove",
                &json!({"at":at}),
                3,
                &fixture.binding,
            )
            .unwrap();
            let MetadataPlanKind::Edit {
                target,
                operation:
                    MetaEditOperation::Remove {
                        collection,
                        scope,
                        names,
                    },
                addressed,
            } = parsed.kind
            else {
                panic!("attribute.remove must plan a typed removal");
            };
            assert_eq!(target.as_str(), "Document.Order");
            assert_eq!(collection, MetaCollection::Attributes);
            assert_eq!(
                scope.as_ref().map(|scope| scope.tabular_section.as_str()),
                section
            );
            assert_eq!(names, ["Total"]);
            assert_eq!(addressed, Some(AddressedFields::None));
        }
    }

    #[test]
    fn typed_attribute_changes_keep_writer_parser_refusal_order() {
        let fixture = MetadataFixture::new();
        for (operation, args, path) in [
            (
                "attribute.set",
                json!({"at":null,"values":false,"unknown":true}),
                "unknown",
            ),
            (
                "attribute.remove",
                json!({"at":null,"unknown":true}),
                "unknown",
            ),
            ("attribute.set", json!({"at":42,"values":false}), "at"),
            ("attribute.remove", json!({"at":42}), "at"),
            (
                "attribute.set",
                json!({"at":"main:Document.Order","values":false}),
                "at",
            ),
            (
                "attribute.remove",
                json!({"at":"main:Document.Order"}),
                "at",
            ),
            (
                "attribute.set",
                json!({"at":"main:Document.Order.Attribute.Total","values":false}),
                "values",
            ),
            (
                "attribute.set",
                json!({"at":"main:Document.Order.Attribute.Total","values":{"name":42,"newName":false}}),
                "values.name",
            ),
            (
                "attribute.set",
                json!({"at":"main:Document.Order.Attribute.Total","values":{"newName":false,"unknown":true}}),
                "values.unknown",
            ),
        ] {
            let error =
                parse_metadata_plan_operation(operation, &args, 3, &fixture.binding).unwrap_err();
            assert_eq!(
                error.kind(),
                ApplyPlanErrorKind::BadValue,
                "{operation}: {args}"
            );
            assert_eq!(
                error.path(),
                Some(format!("ops[3].args.{path}").as_str()),
                "{operation}: {args}: {error:?}"
            );
        }
    }

    #[test]
    fn typed_command_changes_keep_the_addressed_collection_action_and_fields() {
        use super::{AddressedFields, MetadataPlanKind};
        use crate::domain::metadata::{MetaCollection, MetaEditOperation};
        let fixture = MetadataFixture::new();
        for at in [
            "Document.Order.Command.Print",
            "main:Document.Order.Command.Print",
        ] {
            let parsed = parse_metadata_plan_operation(
                "command.set",
                &json!({"at":at,"values":{
                    "newName":"PrintCopy","synonym":"Print copy","comment":"kept",
                    "position":{"before":"Other"}
                }}),
                3,
                &fixture.binding,
            )
            .unwrap();
            let MetadataPlanKind::Edit {
                target,
                operation:
                    MetaEditOperation::Update {
                        collection,
                        scope,
                        elements,
                    },
                addressed,
            } = parsed.kind
            else {
                panic!("command.set must plan a typed update");
            };
            assert_eq!(target.as_str(), "Document.Order");
            assert_eq!(collection, MetaCollection::Commands);
            assert!(scope.is_none());
            assert_eq!(elements.len(), 1);
            assert_eq!(elements[0].name, "Print");
            assert_eq!(elements[0].new_name.as_deref(), Some("PrintCopy"));
            assert_eq!(elements[0].synonym.as_deref(), Some("Print copy"));
            assert_eq!(elements[0].comment.as_deref(), Some("kept"));
            assert_eq!(
                elements[0].position.as_ref().unwrap().before.as_deref(),
                Some("Other")
            );
            assert_eq!(addressed, Some(AddressedFields::Values));

            let parsed = parse_metadata_plan_operation(
                "command.remove",
                &json!({"at":at}),
                3,
                &fixture.binding,
            )
            .unwrap();
            let MetadataPlanKind::Edit {
                target,
                operation:
                    MetaEditOperation::Remove {
                        collection,
                        scope,
                        names,
                    },
                addressed,
            } = parsed.kind
            else {
                panic!("command.remove must plan a typed removal");
            };
            assert_eq!(target.as_str(), "Document.Order");
            assert_eq!(collection, MetaCollection::Commands);
            assert!(scope.is_none());
            assert_eq!(names, ["Print"]);
            assert_eq!(addressed, Some(AddressedFields::None));
        }
    }

    #[test]
    fn typed_command_changes_keep_writer_parser_refusal_order() {
        use serde_json::Map;
        let fixture = MetadataFixture::new();
        for (operation, args, path) in [
            ("command.set", json!(false), ""),
            (
                "command.set",
                json!({"at":null,"values":false,"unknown":true}),
                ".unknown",
            ),
            (
                "command.remove",
                json!({"at":null,"unknown":true}),
                ".unknown",
            ),
            (
                "command.remove",
                json!({"at":null,"values":false}),
                ".values",
            ),
            ("command.set", json!({"at":42,"values":false}), ".at"),
            ("command.remove", json!({"at":42}), ".at"),
            (
                "command.set",
                json!({"at":"main:Document.Order","values":false}),
                ".at",
            ),
            ("command.remove", json!({"at":"main:Document.Order"}), ".at"),
            (
                "command.set",
                json!({"at":"main:Document.Order.Attribute.Total","values":false}),
                ".at",
            ),
            (
                "command.set",
                json!({"at":"main:Document.Order.Form.Main.Command.Print","values":false}),
                ".at",
            ),
            (
                "command.remove",
                json!({"at":"main:Document.Order.Command"}),
                ".at",
            ),
            (
                "command.set",
                json!({"at":"main:Document.Order.Command.Print","values":false}),
                ".values",
            ),
            (
                "command.set",
                json!({"at":"main:Document.Order.Command.Print","values":{}}),
                ".values",
            ),
            (
                "command.set",
                json!({"at":"main:Document.Order.Command.Print","values":{"name":42,"newName":false}}),
                ".values.name",
            ),
            (
                "command.set",
                json!({"at":"main:Document.Order.Command.Print","values":{"newName":false,"unknown":true}}),
                ".values.unknown",
            ),
        ] {
            let error =
                parse_metadata_plan_operation(operation, &args, 3, &fixture.binding).unwrap_err();
            assert_eq!(
                error.kind(),
                ApplyPlanErrorKind::BadValue,
                "{operation}: {args}"
            );
            assert_eq!(
                error.path(),
                Some(format!("ops[3].args{path}").as_str()),
                "{operation}: {args}: {error:?}"
            );
        }
        for (field, value) in [
            ("newName", json!("1Print")),
            ("title", json!("Form-only")),
            ("action", json!("Print")),
            ("type", json!({"variants":[{"kind":"boolean"}]})),
            ("required", json!(true)),
            ("fillValue", json!({"kind":"boolean","value":true})),
            ("indexing", json!("Index")),
            ("attributes", json!([{"name":"Nested"}])),
        ] {
            let values = Map::from_iter([(field.to_string(), value)]);
            let error = parse_metadata_plan_operation(
                "command.set",
                &json!({"at":"main:Document.Order.Command.Print","values":values}),
                3,
                &fixture.binding,
            )
            .unwrap_err();
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{field}");
            assert_eq!(
                error.path(),
                Some(format!("ops[3].args.values.{field}").as_str()),
                "{field}: {error:?}"
            );
        }
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

    /// #1332: registers of one batch share the registrar document. Each
    /// operation adds its record to the image the previous one staged
    /// instead of refusing because the disk preimage no longer matches.
    #[test]
    fn object_create_batches_registers_that_share_one_registrar_document() {
        let order = ORDER_XML.replace(
            "<BasedOn/></Properties>",
            "<BasedOn/><RegisterRecords/></Properties>",
        );
        let configuration = RU_CONFIGURATION_XML.replace(
            "<Document>Order</Document>",
            "<Document>Order</Document><ChartOfCalculationTypes>Kinds</ChartOfCalculationTypes>",
        );
        let chart = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><ChartOfCalculationTypes uuid="44444444-4444-4444-8444-444444444444"><Properties><Name>Kinds</Name></Properties><ChildObjects/></ChartOfCalculationTypes></MetaDataObject>"#;
        let fixture = MetadataFixture::with_files(&[
            ("Configuration.xml", &configuration),
            ("Languages/Русский.xml", &language_xml("Русский", "ru")),
            ("Documents/Order.xml", &order),
            ("ChartsOfCalculationTypes/Kinds.xml", chart),
        ]);
        let registers = [
            ("AccumulationRegister", "Продажи"),
            ("AccumulationRegister", "Остатки"),
            ("AccumulationRegister", "Обороты"),
            ("CalculationRegister", "Начисления"),
            ("CalculationRegister", "Премии"),
        ];
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let operations = registers
            .iter()
            .enumerate()
            .map(|(index, (kind, name))| {
                fixture.parse(
                    "object.create",
                    json!({"at": "main:Configuration", "values": {"kind": kind, "name": name}}),
                    index,
                )
            })
            .collect::<Vec<_>>();

        let (staged, effects) = plan_metadata_batch(staged, authority, &operations)
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));

        assert_eq!(effects.len(), registers.len());
        let changes = staged.planned_changes();
        let document = changes
            .iter()
            .find(|change| change.relative_path == Path::new("Documents/Order.xml"))
            .expect("the registrar document is staged once");
        assert_eq!(
            document.original,
            StagedFileState::Bytes(order.clone().into_bytes())
        );
        let StagedFileState::Bytes(bytes) = &document.current else {
            panic!("registrar keeps bytes");
        };
        let records = registers
            .iter()
            .map(|(kind, name)| {
                format!("<xr:Item xsi:type=\"xr:MDObjectRef\">{kind}.{name}</xr:Item>")
            })
            .collect::<String>();
        assert_eq!(
            String::from_utf8(bytes.clone()).unwrap(),
            order.replace(
                "<RegisterRecords/>",
                &format!("<RegisterRecords>{records}</RegisterRecords>")
            )
        );
        for (kind, name) in registers {
            let directory = if kind == "AccumulationRegister" {
                "AccumulationRegisters"
            } else {
                "CalculationRegisters"
            };
            let descriptor = PathBuf::from(directory).join(format!("{name}.xml"));
            assert!(
                changes
                    .iter()
                    .any(|change| change.relative_path == descriptor
                        && change.kind == StagedChangeKind::Create),
                "{descriptor:?} is not staged"
            );
        }
        assert_eq!(fixture.disk_bytes(), order.as_bytes());
    }

    /// A journal guards the registrar document that a register of the same
    /// batch has already changed; the order of the two must not matter.
    #[test]
    fn object_create_guards_a_registrar_changed_earlier_in_the_batch() {
        let order = ORDER_XML.replace(
            "<BasedOn/></Properties>",
            "<BasedOn/><RegisterRecords/></Properties>",
        );
        for objects in [
            [
                ("AccumulationRegister", "Продажи"),
                ("DocumentJournal", "Журнал"),
            ],
            [
                ("DocumentJournal", "Журнал"),
                ("AccumulationRegister", "Продажи"),
            ],
        ] {
            let fixture = MetadataFixture::with_files(&[
                ("Configuration.xml", RU_CONFIGURATION_XML),
                ("Languages/Русский.xml", &language_xml("Русский", "ru")),
                ("Documents/Order.xml", &order),
            ]);
            let admission = fixture.admission();
            let staged = admission.staged_state().unwrap();
            let authority = admission
                .metadata_planning_authority(&fixture.binding)
                .unwrap();
            let operations = objects
                .iter()
                .enumerate()
                .map(|(index, (kind, name))| {
                    fixture.parse(
                        "object.create",
                        json!({"at": "main:Configuration", "values": {"kind": kind, "name": name}}),
                        index,
                    )
                })
                .collect::<Vec<_>>();

            let (staged, effects) = plan_metadata_batch(staged, authority, &operations)
                .unwrap_or_else(|error| panic!("{objects:?}: {error:?} at {:?}", error.path()));

            assert_eq!(effects.len(), 2, "{objects:?}");
            let changes = staged.planned_changes();
            for descriptor in [
                "AccumulationRegisters/Продажи.xml",
                "DocumentJournals/Журнал.xml",
            ] {
                assert!(
                    changes
                        .iter()
                        .any(|change| change.relative_path == Path::new(descriptor)),
                    "{objects:?}: {descriptor} is not staged"
                );
            }
            let document = changes
                .iter()
                .find(|change| change.relative_path == Path::new("Documents/Order.xml"))
                .expect("the registrar document is staged");
            let StagedFileState::Bytes(bytes) = &document.current else {
                panic!("registrar keeps bytes");
            };
            assert!(
                String::from_utf8_lossy(bytes).contains("AccumulationRegister.Продажи"),
                "{objects:?}"
            );
            assert_eq!(fixture.disk_bytes(), order.as_bytes());
        }
    }

    /// #1385: on a platform-shaped CRLF document the new header attribute
    /// lands before the tabular sections and forms, the new template before
    /// the commands, and every inserted line keeps the descriptor's CRLF.
    #[test]
    fn member_and_template_adds_take_their_platform_slot_and_line_ending() {
        let order = ORDER_XML
            .replace(
                "\t\t<ChildObjects/>\n",
                concat!(
                    "\t\t<ChildObjects>\n",
                    "\t\t\t<Attribute uuid=\"22222222-2222-4222-8222-222222222221\">\n",
                    "\t\t\t\t<Properties><Name>Number2</Name><Synonym/><Comment/></Properties>\n",
                    "\t\t\t</Attribute>\n",
                    "\t\t\t<Form>Main</Form>\n",
                    "\t\t\t<TabularSection uuid=\"22222222-2222-4222-8222-222222222222\">\n",
                    "\t\t\t\t<Properties><Name>Rows</Name><Synonym/><Comment/></Properties>\n",
                    "\t\t\t\t<ChildObjects/>\n",
                    "\t\t\t</TabularSection>\n",
                    "\t\t\t<Command uuid=\"22222222-2222-4222-8222-222222222223\">\n",
                    "\t\t\t\t<Properties><Name>Open</Name><Synonym/><Comment/></Properties>\n",
                    "\t\t\t</Command>\n",
                    "\t\t</ChildObjects>\n",
                ),
            )
            .replace('\n', "\r\n");
        let fixture = MetadataFixture::with_files(&[
            ("Configuration.xml", RU_CONFIGURATION_XML),
            ("Languages/Русский.xml", &language_xml("Русский", "ru")),
            ("Documents/Order.xml", &order),
        ]);
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let attribute = fixture.parse(
            "attribute.add",
            json!({
                "at": "main:Document.Order",
                "items": [{
                    "name": "Comment2",
                    "type": {"variants": [{"kind": "string", "length": 25, "allowedLength": "variable"}]}
                }]
            }),
            0,
        );
        let template = fixture.parse(
            "template.add",
            json!({
                "at": "main:Document.Order",
                "items": [{"name": "Print", "templateType": "SpreadsheetDocument"}]
            }),
            1,
        );
        let (staged, _) = plan_metadata_batch(staged, authority, &[attribute, template])
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let descriptor = staged
            .planned_changes()
            .into_iter()
            .find(|change| change.relative_path == Path::new("Documents/Order.xml"))
            .expect("the owner descriptor is staged");
        let StagedFileState::Bytes(bytes) = descriptor.current else {
            panic!("descriptor keeps bytes");
        };
        let text = String::from_utf8(bytes).unwrap();
        let bare_lf = text
            .match_indices('\n')
            .filter(|(index, _)| *index == 0 || text.as_bytes()[index - 1] != b'\r')
            .count();
        assert_eq!(bare_lf, 0, "inserted lines must keep the CRLF line ending");
        let document = roxmltree::Document::parse(&text).unwrap();
        let children = document
            .descendants()
            .find(|node| node.has_tag_name("ChildObjects"))
            .unwrap()
            .children()
            .filter(|node| node.is_element())
            .map(|node| {
                let name = node
                    .descendants()
                    .find(|inner| inner.has_tag_name("Name"))
                    .and_then(|inner| inner.text())
                    .or_else(|| node.text())
                    .unwrap_or_default();
                format!("{}:{name}", node.tag_name().name())
            })
            .collect::<Vec<_>>();
        assert_eq!(
            children,
            [
                "Attribute:Number2",
                "Attribute:Comment2",
                "Form:Main",
                "TabularSection:Rows",
                "Template:Print",
                "Command:Open",
            ]
        );
    }

    /// A descriptor with mixed line endings is not normalized by a member
    /// add: no line ending is chosen for its author (INV.SOURCE.OBSERVED-BYTES).
    #[test]
    fn member_add_leaves_a_mixed_line_ending_descriptor_unnormalized() {
        let order = ORDER_XML.replace('\n', "\r\n").replacen(
            "<Name>Order</Name>",
            "<Name>Order</Name>\n",
            1,
        );
        let fixture = MetadataFixture::with_files(&[
            ("Configuration.xml", RU_CONFIGURATION_XML),
            ("Languages/Русский.xml", &language_xml("Русский", "ru")),
            ("Documents/Order.xml", &order),
        ]);
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let attribute = fixture.parse(
            "attribute.add",
            json!({"at": "main:Document.Order", "items": [{"name": "Comment2"}]}),
            0,
        );
        let (staged, _) = plan_metadata_batch(staged, authority, &[attribute])
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let descriptor = staged
            .planned_changes()
            .into_iter()
            .find(|change| change.relative_path == Path::new("Documents/Order.xml"))
            .expect("the owner descriptor is staged");
        let StagedFileState::Bytes(bytes) = descriptor.current else {
            panic!("descriptor keeps bytes");
        };
        let text = String::from_utf8(bytes).unwrap();
        let untouched = order.split("<ChildObjects/>").next().unwrap();
        assert!(
            text.starts_with(untouched),
            "the lines before the change keep their bytes"
        );
        assert!(text.contains("<Name>Comment2</Name>"));
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

    /// Табличная часть адресуется (#1352): реквизит, добавленный на адресе
    /// новой табличной части в том же пакете, попадает внутрь неё, а не в
    /// шапку; соседние реквизиты и сгенерированные типы части остаются.
    /// Переименование члена — `newName` на его адресе.
    #[test]
    fn a_tabular_section_takes_attributes_at_its_own_address_in_one_batch() {
        let fixture = MetadataFixture::new();
        let staged = plan_all(
            &fixture,
            &[
                (
                    "attribute.add",
                    json!({"at": "main:Document.Order", "items": [{"name": "Total"}]}),
                ),
                (
                    "tabularSection.add",
                    json!({"at": "main:Document.Order", "items": [{"name": "Lines", "attributes": [{"name": "Qty"}]}]}),
                ),
                (
                    "attribute.add",
                    json!({"at": "main:Document.Order.TabularSection.Lines", "items": [{"name": "Price"}]}),
                ),
                (
                    "tabularSection.set",
                    json!({"at": "main:Document.Order.TabularSection.Lines", "values": {"synonym": "Строки"}}),
                ),
            ],
        )
        .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let texts = staged_texts(&staged);
        let order = &texts[Path::new("Documents/Order.xml")];
        let section_start = order.find("<TabularSection").expect("the section is added");
        let section_end = order.find("</TabularSection>").expect("the section closes");
        let section = &order[section_start..section_end];
        for attribute in ["<Name>Qty</Name>", "<Name>Price</Name>"] {
            assert!(
                section.contains(attribute),
                "{attribute} in the section: {order}"
            );
        }
        // Без позиции новый реквизит встаёт после существующих, как в шапке.
        assert!(
            section.find("<Name>Qty</Name>") < section.find("<Name>Price</Name>"),
            "{section}"
        );
        let header = format!("{}{}", &order[..section_start], &order[section_end..]);
        assert!(header.contains("<Name>Total</Name>"), "{order}");
        assert!(
            !header.contains("<Name>Price</Name>"),
            "the section attribute is not a header attribute: {order}"
        );
        // Сгенерированные типы новой части по одному и не размножены
        // последующими правками.
        for generated in [
            "DocumentTabularSection.Order.Lines",
            "DocumentTabularSectionRow.Order.Lines",
        ] {
            assert_eq!(
                order.matches(&format!("name=\"{generated}\"")).count(),
                1,
                "{generated}: {order}"
            );
        }
        assert!(section.contains("Строки"), "{section}");
    }

    /// Член называется адресом: `values.name` и адрес владельца — ошибки
    /// вызова с точным путём; удаление не принимает аргументов.
    #[test]
    fn addressed_member_operations_refuse_a_name_argument_and_the_owner_address() {
        let fixture = MetadataFixture::new();
        for (operation, args, path) in [
            (
                "tabularSection.set",
                json!({"at": "main:Document.Order.TabularSection.Lines", "values": {"name": "Lines", "synonym": "x"}}),
                "ops[0].args.values.name",
            ),
            (
                "tabularSection.set",
                json!({"at": "main:Document.Order", "values": {"synonym": "x"}}),
                "ops[0].args.at",
            ),
            (
                "tabularSection.set",
                json!({"at": "main:Document.Order.TabularSection.Lines", "values": {}}),
                "ops[0].args.values",
            ),
            (
                "tabularSection.remove",
                json!({"at": "main:Document.Order.TabularSection.Lines", "values": {"name": "Lines"}}),
                "ops[0].args.values",
            ),
            (
                "dimension.set",
                json!({"at": "main:Document.Order.TabularSection.Lines", "values": {"synonym": "x"}}),
                "ops[0].args.at",
            ),
            (
                "command.remove",
                json!({"at": "main:Document.Order.Command"}),
                "ops[0].args.at",
            ),
            (
                "template.set",
                json!({"at": "main:Document.Order", "values": {"name": "Print", "synonym": "x"}}),
                "ops[0].args.at",
            ),
            (
                "template.set",
                json!({"at": "main:Document.Order.Template.Print", "values": {"name": "Print"}}),
                "ops[0].args.values.name",
            ),
            (
                "template.remove",
                json!({"at": "main:Document.Order.Template.Print", "values": {"name": "Print"}}),
                "ops[0].args.values",
            ),
            (
                "attribute.add",
                json!({"at": "main:Document.Order", "scope": {"tabularSection": "Lines"}, "items": [{"name": "Price"}]}),
                "ops[0].args.scope",
            ),
            (
                "attribute.add",
                json!({"at": "main:Document.Order.TabularSection.Lines.Attribute.Qty", "items": [{"name": "Price"}]}),
                "ops[0].args.at",
            ),
            (
                "predefinedItem.set",
                json!({"at": "main:Document.Order.PredefinedItem.Main", "values": {"id": "a1b2c3d4-1111-4111-8111-111111111111", "description": "x"}}),
                "ops[0].args.values.id",
            ),
            (
                "predefinedItem.set",
                json!({"at": "main:Document.Order.PredefinedItem.Main", "values": {"name": "Other"}}),
                "ops[0].args.values.name",
            ),
            (
                "predefinedItem.remove",
                json!({"at": "main:Document.Order", "values": {"id": "a1b2c3d4-1111-4111-8111-111111111111"}}),
                "ops[0].args.values",
            ),
        ] {
            let error = parse_metadata_plan_operation(operation, &args, 0, &fixture.binding)
                .expect_err(operation);
            assert_eq!(
                error.kind(),
                ApplyPlanErrorKind::BadValue,
                "{operation} {args}"
            );
            assert_eq!(error.path(), Some(path), "{operation} {args}: {error:?}");
        }
    }

    /// Изменение отсутствующего члена — «не найден» по адресу операции:
    /// `next` отказа ведёт к владельцу, где член создают.
    #[test]
    fn a_missing_addressed_member_is_not_found_at_the_operation_address() {
        let fixture = MetadataFixture::new();
        for (operation, args) in [
            (
                "tabularSection.set",
                json!({"at": "main:Document.Order.TabularSection.Absent", "values": {"synonym": "x"}}),
            ),
            (
                "tabularSection.remove",
                json!({"at": "main:Document.Order.TabularSection.Absent"}),
            ),
            (
                "attribute.set",
                json!({"at": "main:Document.Order.Attribute.Absent", "values": {"synonym": "x"}}),
            ),
            (
                "attribute.add",
                json!({"at": "main:Document.Order.TabularSection.Absent", "items": [{"name": "Price"}]}),
            ),
        ] {
            let error = plan_all(&fixture, &[(operation, args)]).expect_err(operation);
            assert_eq!(error.kind(), ApplyPlanErrorKind::NotFound, "{operation}");
            assert_eq!(error.path(), Some("ops[0].args.at"), "{operation}");
        }
        // Отсутствует не сам член, а названный в аргументах сосед: отказ
        // указывает на аргумент, а не на адрес.
        let error = plan_all(
            &fixture,
            &[
                (
                    "tabularSection.add",
                    json!({"at": "main:Document.Order", "items": [{"name": "Lines"}]}),
                ),
                (
                    "tabularSection.set",
                    json!({"at": "main:Document.Order.TabularSection.Lines", "values": {"position": {"before": "Absent"}}}),
                ),
            ],
        )
        .expect_err("a missing anchor refuses");
        assert_eq!(error.kind(), ApplyPlanErrorKind::NotFound);
        assert_eq!(error.path(), Some("ops[1].args.values.position"));
    }

    /// Опубликованный контракт операции члена и разбор планировщиком
    /// согласны: структурно неверный вызов отвергают оба, верный принимают
    /// оба. Контракт берётся на конкретном узле, как его отдаёт `view`
    /// адресом `<узел>.Can.<op>`: поля, зависящие от вида владельца и табличной части,
    /// сужены до узла. Каждое семейство проверяется на нескольких видах
    /// владельца. Перекрёстные правила полей (значение заполнения требует
    /// тип) схема не выражает — такие образцы здесь не берутся.
    #[test]
    fn member_operation_contracts_and_the_planner_agree_on_structure() {
        use crate::domain::address::QualifiedAddress;
        use crate::domain::apply::{OperationRegistry, OperationSite, SourceSetTraits};
        use crate::domain::metadata::{metadata_collection_spec, MetaCollection};
        use crate::domain::platform_profile::PlatformProfile;
        let fixture = MetadataFixture::new();
        let traits =
            SourceSetTraits::read(SourceSetKind::Configuration, PlatformProfile::v8_3_27());
        let string_type =
            json!({"variants": [{"kind": "string", "length": 10, "allowedLength": "variable"}]});
        let string_fill = json!({"kind": "string", "value": "x"});
        let id = "a1b2c3d4-1111-4111-8111-111111111111";
        let registry = OperationRegistry::closed();
        let mut checked = 0;
        // Образцы, чья допустимость зависит от узла: обе стороны обязаны
        // согласиться, и по матрице встречаются оба исхода.
        let mut site_dependent = std::collections::BTreeSet::new();
        for descriptor in registry.descriptors() {
            let op = descriptor.name();
            // Реквизит и команда формы носят то же имя (#1465): здесь сверяются
            // члены метаданных.
            if op == "template.add"
                || descriptor.argument_contract().is_none()
                || descriptor.family() != crate::domain::apply::OperationFamily::Metadata
            {
                continue;
            }
            let (family, verb) = op.split_once('.').unwrap();
            let owners: &[&str] = match family {
                "attribute" => &[
                    "main:Document.Order",
                    "main:Catalog.Items",
                    "main:Report.Sales",
                ],
                "tabularSection" | "command" => &["main:Document.Order", "main:Catalog.Items"],
                "dimension" | "resource" => &[
                    "main:InformationRegister.Prices",
                    "main:AccumulationRegister.Stock",
                ],
                "enumValue" => &["main:Enum.Kinds"],
                "column" => &["main:DocumentJournal.All"],
                "template" => &["main:Document.Order", "main:Report.Sales"],
                "predefinedItem" => &[
                    "main:Catalog.Items",
                    "main:ChartOfAccounts.Main",
                    "main:ChartOfCharacteristicTypes.Props",
                    "main:ChartOfCalculationTypes.Calc",
                ],
                _ => continue,
            };
            let leaf = match family {
                "attribute" => "Attribute",
                "tabularSection" => "TabularSection",
                "dimension" => "Dimension",
                "resource" => "Resource",
                "enumValue" => "EnumValue",
                "column" => "Column",
                "command" => "Command",
                "template" => "Template",
                _ => "PredefinedItem",
            };
            let collection = match family {
                "attribute" => Some(MetaCollection::Attributes),
                "tabularSection" => Some(MetaCollection::TabularSections),
                "dimension" => Some(MetaCollection::Dimensions),
                "resource" => Some(MetaCollection::Resources),
                "enumValue" => Some(MetaCollection::EnumValues),
                "column" => Some(MetaCollection::Columns),
                "command" => Some(MetaCollection::Commands),
                _ => None,
            };
            let spec = collection.map(metadata_collection_spec);
            let typed = spec.is_some_and(|spec| spec.allows_type);
            // `Some(verdict)` — исход задан; `None` — зависит от узла.
            let mut samples: Vec<(serde_json::Value, Option<bool>)> = Vec::new();
            match (family, verb) {
                ("predefinedItem", "add") => samples.extend([
                    (json!({"items": [{"id": id, "name": "Main"}]}), Some(true)),
                    (json!({"items": [{"id": id, "name": "Main", "code": "1"}]}), Some(true)),
                    (json!({"items": [{"name": "Main"}]}), Some(false)),
                    (json!({"items": [{"id": "not-a-uuid", "name": "Main"}]}), Some(false)),
                    (json!({"items": [{"id": id, "name": "1Main"}]}), Some(false)),
                    (json!({"items": [{"id": id, "name": "Main", "unknown": 1}]}), Some(false)),
                    (json!({"items": []}), Some(false)),
                    (json!({}), Some(false)),
                ]),
                ("predefinedItem", "set") => samples.extend([
                    (json!({"values": {"description": "x"}}), Some(true)),
                    (json!({"values": {"newName": "Other"}}), Some(true)),
                    (json!({"values": {"newName": "1Other"}}), Some(false)),
                    (json!({"values": {"newName": ""}}), Some(false)),
                    (json!({"values": {"id": id}}), Some(false)),
                    (json!({"values": {"name": "Other"}}), Some(false)),
                    (json!({"values": {}}), Some(false)),
                    (json!({}), Some(false)),
                ]),
                ("template", "set") => samples.extend([
                    (json!({"values": {"comment": "x"}}), Some(true)),
                    (json!({"values": {"name": "Print"}}), Some(false)),
                    (json!({"values": {}}), Some(false)),
                    (json!({"values": {"templateType": "Picture"}}), Some(false)),
                    (json!({"values": {"synonym": 1}}), Some(false)),
                ]),
                (_, "add") => samples.extend([
                    (json!({"items": [{"name": "Member", "comment": "x"}]}), Some(true)),
                    (json!({"items": [{"name": "Member", "type": string_type}]}), Some(typed)),
                    (
                        json!({"items": [{"name": "Member", "indexing": "Index"}]}),
                        // Индексируются только типизированные поля.
                        Some(matches!(family, "attribute" | "dimension" | "resource" | "column")),
                    ),
                    (
                        json!({"items": [{"name": "Member", "required": true}]}),
                        Some(spec.is_some_and(|spec| spec.allows_required)),
                    ),
                    (
                        json!({"items": [{"name": "Member", "type": string_type, "fillValue": string_fill}]}),
                        None,
                    ),
                    (
                        json!({"items": [{"name": "Member", "templateType": "SpreadsheetDocument"}]}),
                        Some(false),
                    ),
                    (json!({"items": [{"name": "1Member"}]}), Some(false)),
                    (json!({"items": [{"name": "Член-1"}]}), Some(false)),
                    (json!({"items": [{"comment": "x"}]}), Some(false)),
                    (json!({"items": [{"name": "Member", "unknown": 1}]}), Some(false)),
                    (json!({"items": [{"name": "Member", "synonym": 1}]}), Some(false)),
                    (json!({"items": []}), Some(false)),
                    (json!({"items": {}}), Some(false)),
                    (json!({"values": {}}), Some(false)),
                    (json!({}), Some(false)),
                ]),
                (_, "set") => samples.extend([
                    (json!({"values": {"newName": "Other"}}), Some(true)),
                    (json!({"values": {"comment": "x", "synonym": "y"}}), Some(true)),
                    (json!({"values": {"type": string_type}}), Some(typed)),
                    (
                        json!({"values": {"required": true}}),
                        Some(spec.is_some_and(|spec| spec.allows_required)),
                    ),
                    (
                        json!({"values": {"type": string_type, "fillValue": string_fill}}),
                        None,
                    ),
                    (json!({"values": {"name": "Member"}}), Some(false)),
                    (json!({"values": {"newName": "1Other"}}), Some(false)),
                    (json!({"values": {"synonym": 1}}), Some(false)),
                    (json!({"values": {}}), Some(false)),
                    (json!({"values": "x"}), Some(false)),
                    (json!({}), Some(false)),
                ]),
                (_, "remove") => samples.extend([
                    (json!({}), Some(true)),
                    (json!({"values": {"name": "Member"}}), Some(false)),
                    (json!({"items": []}), Some(false)),
                ]),
                _ => {}
            }
            if family == "predefinedItem" && verb != "remove" {
                let container = if verb == "add" { "items" } else { "values" };
                for (field, value) in [
                    ("isFolder", json!(false)),
                    ("type", string_type.clone()),
                    ("accountType", json!("Active")),
                    ("offBalance", json!(false)),
                    ("order", json!("1")),
                    ("accountingFlags", json!({})),
                    ("extDimensionTypes", json!([])),
                    ("actionPeriodIsBase", json!(true)),
                ] {
                    let mut item = if verb == "add" {
                        json!({"id": id, "name": "Main"})
                    } else {
                        json!({})
                    };
                    item[field] = value;
                    let args = if verb == "add" {
                        json!({container: [item]})
                    } else {
                        json!({container: item})
                    };
                    samples.push((args, None));
                }
            }
            for owner in owners {
                let mut targets = vec![if verb == "add" {
                    owner.to_string()
                } else {
                    format!("{owner}.{leaf}.Member")
                }];
                if family == "attribute" {
                    targets.push(if verb == "add" {
                        format!("{owner}.TabularSection.Lines")
                    } else {
                        format!("{owner}.TabularSection.Lines.Attribute.Member")
                    });
                }
                for target in &targets {
                    let address = QualifiedAddress::parse(target).unwrap();
                    let site = OperationSite::new(&address, traits).unwrap();
                    assert!(descriptor.applies_at(&site), "{op} at {target}");
                    let contract = descriptor
                        .argument_contract_at(&site)
                        .unwrap_or_else(|| panic!("{op} at {target} has a contract"));
                    let validator = jsonschema::validator_for(&contract.schema)
                        .unwrap_or_else(|error| panic!("{op}: {error}"));
                    let mut all = vec![(contract.example_args.clone(), Some(true))];
                    all.extend(samples.iter().cloned());
                    for (index, (args, expected)) in all.iter().enumerate() {
                        let mut args = args.clone();
                        args["at"] = json!(target);
                        let schema = validator.is_valid(&args);
                        let planner = parse_metadata_plan_operation(op, &args, 0, &fixture.binding)
                            .map(|_| ());
                        assert_eq!(
                            schema,
                            planner.is_ok(),
                            "{op} at {target} with {args}: schema={schema}, planner={planner:?}"
                        );
                        match expected {
                            Some(expected) => assert_eq!(
                                schema, *expected,
                                "{op} at {target} with {args}: planner={planner:?}"
                            ),
                            None => {
                                site_dependent.insert((op, index, schema));
                            }
                        }
                    }
                    checked += 1;
                }
            }
        }
        // Члены: 7 семейств × 3 операции по своим владельцам, табличная
        // часть у реквизита; макет и предопределённый элемент.
        assert_eq!(checked, 64);
        for op in [
            "attribute.add",
            "attribute.set",
            "predefinedItem.add",
            "predefinedItem.set",
        ] {
            for verdict in [true, false] {
                assert!(
                    site_dependent
                        .iter()
                        .any(|(name, _, schema)| *name == op && *schema == verdict),
                    "{op}: the node-dependent samples never answered {verdict}"
                );
            }
        }
    }

    /// Предопределённые элементы пишутся в `Ext/Predefined.xml` владельца:
    /// добавление создаёт файл, изменение и удаление находят элемент по
    /// имени из адреса, в том числе добавленный ранее в том же пакете.
    #[test]
    fn predefined_items_are_staged_in_the_owner_predefined_file_by_name() {
        const ITEMS_XML: &str = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Catalog uuid="99999999-9999-4999-8999-999999999999">
		<Properties><Name>Items</Name><CodeLength>9</CodeLength><CodeType>String</CodeType></Properties>
		<ChildObjects/>
	</Catalog>
</MetaDataObject>
"#;
        let fixture = MetadataFixture::with_files(&[
            ("Configuration.xml", RU_CONFIGURATION_XML),
            ("Languages/Русский.xml", &language_xml("Русский", "ru")),
            ("Documents/Order.xml", ORDER_XML),
            ("Catalogs/Items.xml", ITEMS_XML),
        ]);
        let staged = plan_all(
            &fixture,
            &[
                (
                    "predefinedItem.add",
                    json!({"at": "main:Catalog.Items", "items": [
                        {"id": "a1b2c3d4-1111-4111-8111-111111111111", "name": "Main"},
                        {"id": "a1b2c3d4-2222-4222-8222-222222222222", "name": "Spare"}
                    ]}),
                ),
                (
                    "predefinedItem.set",
                    json!({"at": "main:Catalog.Items.PredefinedItem.Main", "values": {"description": "Основной", "newName": "Primary"}}),
                ),
                (
                    "predefinedItem.remove",
                    json!({"at": "main:Catalog.Items.PredefinedItem.Spare"}),
                ),
            ],
        )
        .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let texts = staged_texts(&staged);
        let predefined = &texts[Path::new("Catalogs/Items/Ext/Predefined.xml")];
        assert!(predefined.contains("<Name>Primary</Name>"), "{predefined}");
        assert!(
            predefined.contains("<Description>Основной</Description>"),
            "{predefined}"
        );
        assert!(!predefined.contains("Spare"), "{predefined}");
        let missing = plan_all(
            &fixture,
            &[(
                "predefinedItem.remove",
                json!({"at": "main:Catalog.Items.PredefinedItem.Absent"}),
            )],
        )
        .expect_err("an absent item is not found");
        assert_eq!(missing.kind(), ApplyPlanErrorKind::NotFound);
        assert_eq!(missing.path(), Some("ops[0].args.at"));
        // Имя — адрес элемента: занятое имя не принимается ни при
        // добавлении (на любой глубине и без учёта регистра), ни при
        // переименовании.
        for (operations, path) in [
            (
                vec![(
                    "predefinedItem.add",
                    json!({"at": "main:Catalog.Items", "items": [
                        {"id": "a1b2c3d4-1111-4111-8111-111111111111", "name": "Дубль"},
                        {"id": "a1b2c3d4-2222-4222-8222-222222222222", "name": "дубль"}
                    ]}),
                )],
                "ops[0].args.items[1].name",
            ),
            (
                vec![
                    (
                        "predefinedItem.add",
                        json!({"at": "main:Catalog.Items", "items": [
                            {"id": "a1b2c3d4-1111-4111-8111-111111111111", "name": "Один"},
                            {"id": "a1b2c3d4-2222-4222-8222-222222222222", "name": "Два"}
                        ]}),
                    ),
                    (
                        "predefinedItem.set",
                        json!({"at": "main:Catalog.Items.PredefinedItem.Два", "values": {"newName": "Один"}}),
                    ),
                ],
                "ops[1].args.values.newName",
            ),
        ] {
            let error = plan_all(&fixture, &operations).expect_err(path);
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{path}");
            assert_eq!(error.path(), Some(path));
        }
        // Два одноимённых элемента в исходниках: адрес ничего не выбирает,
        // и первый попавшийся не подставляется.
        assert!(crate::infrastructure::native_operations::meta::predefined_item_id(
            br#"<PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="00000000-0000-4000-8000-000000000001"><Name>Same</Name><Code/><Description/><IsFolder>true</IsFolder><ChildItems><Item id="00000000-0000-4000-8000-000000000002"><Name>same</Name><Code/><Description/><IsFolder>false</IsFolder></Item></ChildItems></Item></PredefinedData>"#,
            crate::domain::metadata::MetadataKind::Catalog,
            "Same",
        )
        .is_err());
    }

    #[test]
    fn member_collection_removal_takes_the_member_from_its_address() {
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
            json!({"at": "main:Document.Order.TabularSection.Строки"}),
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
        plan_all_with_effects(fixture, operations).map(|(staged, _)| staged)
    }

    fn plan_all_with_effects(
        fixture: &MetadataFixture,
        operations: &[(&str, serde_json::Value)],
    ) -> Result<
        (
            crate::infrastructure::native_operations::apply::ApplyStagedState,
            crate::infrastructure::native_operations::apply::PlannedApplyEffects,
        ),
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
        let (staged, provisional) = plan_metadata_batch(staged, authority, &parsed)?;
        let effects =
            crate::infrastructure::native_operations::apply_families::request::reconcile_effects(
                &staged,
                provisional,
            );
        Ok((staged, effects))
    }

    const COMMAND_MODULE: &str =
        "\u{feff}Procedure Refresh() Export\r\n\tMessage(\"unchanged\");\r\nEndProcedure\r\n";
    const COMMAND_MODULE_RELATIVE: &str = "Documents/Order/Commands/Refresh/Ext/CommandModule.bsl";

    fn command_fixture() -> MetadataFixture {
        let order = ORDER_XML.replace(
            "<ChildObjects/>",
            concat!(
                "<ChildObjects>",
                "<Command uuid=\"22222222-2222-4222-8222-222222222222\"><Properties><Name>Refresh</Name><Synonym/><Comment>keep</Comment><Group/><Picture/><ToolTip/></Properties></Command>",
                "<Command uuid=\"33333333-3333-4333-8333-333333333333\"><Properties><Name>Inspect</Name><Synonym/><Comment/><Group/></Properties></Command>",
                "<Command uuid=\"44444444-4444-4444-8444-444444444444\"><Properties><Name>Neighbor</Name><Synonym/><Comment>neighbor</Comment><Group/></Properties></Command>",
                "</ChildObjects>"
            ),
        );
        MetadataFixture::with_files(&[
            ("Configuration.xml", RU_CONFIGURATION_XML),
            ("Languages/Русский.xml", &language_xml("Русский", "ru")),
            ("Documents/Order.xml", &order),
            (COMMAND_MODULE_RELATIVE, COMMAND_MODULE),
            (
                "Documents/Order/Commands/Refresh/Ext/unrelated.txt",
                "opaque neighbor",
            ),
            (
                "Documents/Order/Commands/Neighbor/Ext/CommandModule.bsl",
                "Procedure Neighbor()\nEndProcedure\n",
            ),
        ])
    }

    #[test]
    fn command_rename_stages_exact_module_bytes_and_preserves_inline_identity_and_neighbors() {
        let fixture = command_fixture();
        let original = fixture.disk_bytes();
        let staged = plan_all(
            &fixture,
            &[("command.set", json!({"at": "main:Document.Order.Command.refresh", "values": {"newName": "Updated"}}))],
        ).unwrap();
        let changes = staged.planned_changes();
        assert_eq!(
            changes.len(),
            3,
            "rename must stage owner, old module and new module"
        );
        let texts = staged_texts(&staged);
        assert_eq!(
            texts[Path::new("Documents/Order/Commands/Updated/Ext/CommandModule.bsl")],
            COMMAND_MODULE
        );
        assert!(changes.iter().any(|change| change.relative_path
            == Path::new(COMMAND_MODULE_RELATIVE)
            && change.kind == StagedChangeKind::Remove));
        let owner = &texts[Path::new("Documents/Order.xml")];
        assert_eq!(
            owner.as_bytes(),
            String::from_utf8(original.clone())
                .unwrap()
                .replace("<Name>Refresh</Name>", "<Name>Updated</Name>")
                .as_bytes()
        );
        assert_eq!(fixture.disk_bytes(), original);
        assert_eq!(
            std::fs::read(fixture.binding.source_root().join(COMMAND_MODULE_RELATIVE)).unwrap(),
            COMMAND_MODULE.as_bytes()
        );
        assert_eq!(
            std::fs::read(
                fixture
                    .binding
                    .source_root()
                    .join("Documents/Order/Commands/Refresh/Ext/unrelated.txt")
            )
            .unwrap(),
            b"opaque neighbor"
        );
        assert!(!texts.contains_key(Path::new(
            "Documents/Order/Commands/Updated/Ext/unrelated.txt"
        )));
    }

    #[test]
    fn command_remove_stages_only_the_owner_and_its_known_module() {
        let fixture = command_fixture();
        let staged = plan_all(
            &fixture,
            &[(
                "command.remove",
                json!({"at": "main:Document.Order.Command.rEfReSh"}),
            )],
        )
        .unwrap();
        let changes = staged.planned_changes();
        assert_eq!(changes.len(), 2);
        assert!(changes.iter().any(|change| change.relative_path
            == Path::new(COMMAND_MODULE_RELATIVE)
            && change.kind == StagedChangeKind::Remove));
        let texts = staged_texts(&staged);
        let owner = &texts[Path::new("Documents/Order.xml")];
        assert!(!owner.contains("<Name>Refresh</Name>"));
        assert!(owner.contains("<Name>Neighbor</Name>"));
        assert_eq!(
            std::fs::read(
                fixture
                    .binding
                    .source_root()
                    .join("Documents/Order/Commands/Refresh/Ext/unrelated.txt")
            )
            .unwrap(),
            b"opaque neighbor"
        );
    }

    #[test]
    fn command_property_updates_do_not_read_or_stage_the_module() {
        let fixture = command_fixture();
        let module = fixture.binding.source_root().join(COMMAND_MODULE_RELATIVE);
        std::fs::remove_file(&module).unwrap();
        std::fs::create_dir(&module).unwrap();
        let (staged, effects) = plan_all_with_effects(&fixture, &[(
            "command.set",
            json!({"at": "main:Document.Order.Command.Refresh", "values": {"comment": "updated"}}),
        )]).unwrap();
        assert_eq!(staged.planned_changes().len(), 1);
        assert!(staged_texts(&staged)[Path::new("Documents/Order.xml")]
            .contains("<Comment>updated</Comment>"));
        assert!(
            !crate::domain::cache::CacheImpact::from_events(effects.events())
                .invalidated
                .contains("bsl_index")
        );
        assert!(module.is_dir());
    }

    #[test]
    fn command_without_a_module_can_be_renamed_or_removed_without_materialization() {
        for (operation, values) in [
            ("command.set", json!({"newName": "Updated"})),
            ("command.remove", serde_json::Value::Null),
        ] {
            let fixture = command_fixture();
            let mut args = json!({"at": "main:Document.Order.Command.Inspect"});
            if !values.is_null() {
                args["values"] = values;
            }
            let (staged, effects) = plan_all_with_effects(&fixture, &[(operation, args)]).unwrap();
            assert_eq!(staged.planned_changes().len(), 1);
            assert!(
                !crate::domain::cache::CacheImpact::from_events(effects.events())
                    .invalidated
                    .contains("bsl_index")
            );
            assert!(staged.planned_changes().iter().all(|change| change
                .relative_path
                .extension()
                .is_some_and(|extension| extension == "xml")));
        }
    }

    #[test]
    fn command_rename_refuses_an_orphan_destination_even_without_a_source_module() {
        for name in ["Refresh", "Inspect"] {
            let fixture = command_fixture();
            let original = fixture.disk_bytes();
            let blocked_relative = "Documents/Order/Commands/Blocked/Ext/CommandModule.bsl";
            let blocked = fixture.binding.source_root().join(blocked_relative);
            std::fs::create_dir_all(blocked.parent().unwrap()).unwrap();
            std::fs::write(&blocked, b"foreign module").unwrap();
            let error = plan_all(
                &fixture,
                &[(
                    "command.set",
                    json!({
                        "at": format!("main:Document.Order.Command.{name}"),
                        "values": {"newName": "Blocked"}
                    }),
                )],
            )
            .unwrap_err();
            assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);
            assert_eq!(error.path(), Some("ops[0].args.values.newName"));
            assert_eq!(
                error.to_string(),
                format!("the command module destination already exists: {blocked_relative}")
            );
            assert_eq!(fixture.disk_bytes(), original);
            assert_eq!(std::fs::read(&blocked).unwrap(), b"foreign module");
            assert_eq!(
                std::fs::read(fixture.binding.source_root().join(COMMAND_MODULE_RELATIVE)).unwrap(),
                COMMAND_MODULE.as_bytes()
            );
        }
    }

    #[test]
    fn command_lifecycle_chains_use_staged_modules_and_reconcile_final_cache_effects() {
        let rename = |from: &str, to: &str| {
            (
                "command.set",
                json!({"at": format!("main:Document.Order.Command.{from}"), "values": {"newName": to}}),
            )
        };
        let remove = |name: &str| {
            (
                "command.remove",
                json!({"at": format!("main:Document.Order.Command.{name}")}),
            )
        };
        for (operations, expected_module, expected_changes) in [
            (
                vec![rename("Refresh", "Step"), rename("Step", "Final")],
                Some("Final"),
                3,
            ),
            (vec![rename("Refresh", "Step"), remove("Step")], None, 2),
            (
                vec![rename("Refresh", "Step"), rename("Step", "Refresh")],
                Some("Refresh"),
                0,
            ),
            (
                vec![
                    remove("Refresh"),
                    (
                        "command.add",
                        json!({"at": "main:Document.Order", "items": [{"name": "Refresh"}]}),
                    ),
                    rename("Refresh", "Final"),
                ],
                None,
                2,
            ),
        ] {
            let fixture = command_fixture();
            let (mut staged, effects) = plan_all_with_effects(&fixture, &operations).unwrap();
            assert_eq!(
                staged.planned_changes().len(),
                expected_changes,
                "{operations:?}"
            );
            for name in ["Refresh", "Step", "Final"] {
                let relative = format!("Documents/Order/Commands/{name}/Ext/CommandModule.bsl");
                let module = staged.read(Path::new(&relative)).unwrap();
                assert_eq!(
                    module.as_deref(),
                    (expected_module == Some(name)).then_some(COMMAND_MODULE.as_bytes()),
                    "{operations:?}, {name}"
                );
            }
            let impact = crate::domain::cache::CacheImpact::from_events(effects.events());
            assert_eq!(
                impact.invalidated.contains("bsl_index"),
                expected_changes != 0
            );
            assert!(effects
                .events()
                .iter()
                .filter(|event| event.kind == crate::domain::events::DomainEventKind::ModuleChanged)
                .all(|event| !event.artifact.contains(".Command.Step.")));
            if expected_changes == 0 {
                assert!(effects.events().is_empty());
            }
        }
    }

    #[test]
    fn command_lifecycle_rejects_a_stale_module_before_finalization() {
        for (operation, args) in [
            (
                "command.set",
                json!({"at": "main:Document.Order.Command.Refresh", "values": {"newName": "Updated"}}),
            ),
            (
                "command.remove",
                json!({"at": "main:Document.Order.Command.Refresh"}),
            ),
        ] {
            let fixture = command_fixture();
            let original = fixture.disk_bytes();
            let staged = plan_all(&fixture, &[(operation, args)]).unwrap();
            let module = fixture.binding.source_root().join(COMMAND_MODULE_RELATIVE);
            std::fs::write(&module, b"concurrent bytes").unwrap();
            let error = staged.finalize().unwrap_err();
            assert_eq!(error.kind(), crate::infrastructure::native_operations::apply::ApplyStagingErrorKind::ConcurrentRevision);
            assert_eq!(fixture.disk_bytes(), original);
            assert_eq!(std::fs::read(&module).unwrap(), b"concurrent bytes");
            assert!(!fixture
                .binding
                .source_root()
                .join("Documents/Order/Commands/Updated")
                .exists());
        }
    }

    #[test]
    fn command_rename_carries_the_previous_staged_module_postimage() {
        let fixture = command_fixture();
        let admission = fixture.admission();
        let mut staged = admission.staged_state().unwrap();
        let edited_module = b"Procedure EditedEarlier() Export\nEndProcedure\n";
        staged
            .replace(
                COMMAND_MODULE_RELATIVE,
                COMMAND_MODULE.as_bytes(),
                edited_module.to_vec(),
            )
            .unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let operations = [fixture.parse(
            "command.set",
            json!({"at": "main:Document.Order.Command.Refresh", "values": {"newName": "Updated"}}),
            1,
        )];
        let (mut staged, _) = plan_metadata_batch(staged, authority, &operations).unwrap();
        assert_eq!(
            staged
                .read(Path::new(
                    "Documents/Order/Commands/Updated/Ext/CommandModule.bsl"
                ))
                .unwrap()
                .as_deref(),
            Some(edited_module.as_slice())
        );
        assert_eq!(
            staged.read(Path::new(COMMAND_MODULE_RELATIVE)).unwrap(),
            None
        );
        assert_eq!(
            std::fs::read(fixture.binding.source_root().join(COMMAND_MODULE_RELATIVE)).unwrap(),
            COMMAND_MODULE.as_bytes()
        );
    }

    #[test]
    fn command_case_alias_rename_invalidates_the_index_and_a_roundtrip_cancels_it() {
        let rename_lower = (
            "command.set",
            json!({"at": "main:Document.Order.Command.refresh", "values": {"newName": "refresh", "comment": "keep"}}),
        );
        let fixture = command_fixture();
        let (mut staged, effects) =
            plan_all_with_effects(&fixture, std::slice::from_ref(&rename_lower)).unwrap();
        assert_eq!(
            staged
                .read(Path::new(
                    "Documents/Order/Commands/refresh/Ext/CommandModule.bsl"
                ))
                .unwrap()
                .as_deref(),
            Some(COMMAND_MODULE.as_bytes())
        );
        assert!(
            crate::domain::cache::CacheImpact::from_events(effects.events())
                .invalidated
                .contains("bsl_index")
        );
        assert!(staged_texts(&staged)[Path::new("Documents/Order.xml")]
            .contains("<Name>refresh</Name>"));
        let (staged, effects) = plan_all_with_effects(&fixture, &[
            rename_lower,
            ("command.set", json!({"at": "main:Document.Order.Command.Refresh", "values": {"newName": "Refresh", "comment": "keep"}})),
        ]).unwrap();
        assert!(staged.planned_changes().is_empty());
        assert!(effects.events().is_empty());
        staged.finalize().unwrap();
    }

    #[test]
    fn command_case_correction_to_the_existing_physical_spelling_publishes_and_invalidates() {
        let fixture = command_fixture();
        let lowercase_module = fixture
            .binding
            .source_root()
            .join("Documents/Order/Commands/refresh/Ext/CommandModule.bsl");
        let case_alias = lowercase_module.is_file();
        let lowercase_owner = String::from_utf8(fixture.disk_bytes())
            .unwrap()
            .replace("<Name>Refresh</Name>", "<Name>refresh</Name>");
        std::fs::write(&fixture.descriptor, lowercase_owner).unwrap();
        let admission = fixture
            .actor
            .admit_apply(
                &fixture.binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let operations = [fixture.parse(
            "command.set",
            json!({"at": "main:Document.Order.Command.Refresh", "values": {"newName": "Refresh", "comment": "keep"}}),
            0,
        )];
        let planned = plan_metadata_batch(staged, authority, &operations);
        if case_alias {
            let (staged, provisional) = planned.unwrap();
            let effects = crate::infrastructure::native_operations::apply_families::request::reconcile_effects(&staged, provisional);
            let prepared = admission.prepare_with_effects(staged, effects).unwrap();
            let result = fixture.actor.publish_prepared_apply(prepared).unwrap();
            assert!(result
                .effects()
                .cache()
                .invalidated
                .iter()
                .any(|name| name == "bsl_index"));
            assert!(String::from_utf8(fixture.disk_bytes())
                .unwrap()
                .contains("<Name>Refresh</Name>"));
            assert_eq!(
                std::fs::read(fixture.binding.source_root().join(COMMAND_MODULE_RELATIVE)).unwrap(),
                COMMAND_MODULE.as_bytes()
            );
        } else {
            assert_eq!(
                planned.unwrap_err().kind(),
                ApplyPlanErrorKind::InvalidState
            );
        }
    }

    #[test]
    fn command_case_alias_rename_invalidates_after_an_earlier_aliased_read() {
        let fixture = command_fixture();
        let admission = fixture.admission();
        let mut staged = admission.staged_state().unwrap();
        staged
            .read(Path::new(
                "Documents/Order/Commands/rEfReSh/Ext/CommandModule.bsl",
            ))
            .unwrap();
        let authority = admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap();
        let operations = [fixture.parse(
            "command.set",
            json!({"at": "main:Document.Order.Command.refresh", "values": {"newName": "refresh", "comment": "keep"}}),
            1,
        )];
        let (staged, provisional) = plan_metadata_batch(staged, authority, &operations).unwrap();
        let effects =
            crate::infrastructure::native_operations::apply_families::request::reconcile_effects(
                &staged,
                provisional,
            );
        assert!(
            crate::domain::cache::CacheImpact::from_events(effects.events())
                .invalidated
                .contains("bsl_index")
        );
    }

    #[test]
    fn command_lifecycle_effects_follow_the_retained_spelling_of_earlier_reads() {
        for (operation, args) in [
            (
                "command.set",
                json!({"at": "main:Document.Order.Command.Refresh", "values": {"newName": "Updated"}}),
            ),
            (
                "command.remove",
                json!({"at": "main:Document.Order.Command.Refresh"}),
            ),
        ] {
            let fixture = command_fixture();
            let admission = fixture.admission();
            let mut staged = admission.staged_state().unwrap();
            for relative in [
                "Documents/Order/Commands/rEfReSh/Ext/CommandModule.bsl",
                "Documents/Order/Commands/uPdAtEd/Ext/CommandModule.bsl",
            ] {
                staged.read(Path::new(relative)).unwrap();
            }
            let authority = admission
                .metadata_planning_authority(&fixture.binding)
                .unwrap();
            let operations = [fixture.parse(operation, args, 1)];
            let (staged, provisional) =
                plan_metadata_batch(staged, authority, &operations).unwrap();
            let effects = crate::infrastructure::native_operations::apply_families::request::reconcile_effects(&staged, provisional);
            assert!(
                crate::domain::cache::CacheImpact::from_events(effects.events())
                    .invalidated
                    .contains("bsl_index"),
                "{operation}"
            );
        }
    }

    #[test]
    fn command_lifecycle_refuses_links_at_the_source_or_destination_module_route() {
        use crate::infrastructure::platform::testing::{
            create_directory_link_fixture_for_test, create_file_link_fixture_for_test,
            FileLinkFixtureOutcome,
        };
        for (source, directory) in [(true, false), (false, false), (true, true), (false, true)] {
            let fixture = command_fixture();
            let original = fixture.disk_bytes();
            let outside = tempfile::tempdir().unwrap();
            let foreign = outside.path().join("CommandModule.bsl");
            std::fs::write(&foreign, b"foreign module").unwrap();
            let name = if source { "Refresh" } else { "Updated" };
            let ext = fixture
                .binding
                .source_root()
                .join(format!("Documents/Order/Commands/{name}/Ext"));
            if source {
                if directory {
                    std::fs::rename(&ext, ext.with_file_name("ExtSaved")).unwrap();
                } else {
                    std::fs::remove_file(ext.join("CommandModule.bsl")).unwrap();
                }
            } else {
                std::fs::create_dir_all(ext.parent().unwrap()).unwrap();
                if !directory {
                    std::fs::create_dir(&ext).unwrap();
                }
            }
            let outcome = if directory {
                create_directory_link_fixture_for_test(outside.path(), &ext)
            } else {
                create_file_link_fixture_for_test(&foreign, ext.join("CommandModule.bsl"))
            }
            .unwrap();
            if outcome != FileLinkFixtureOutcome::Created {
                continue;
            }
            let error = plan_all(&fixture, &[("command.set", json!({"at": "main:Document.Order.Command.Refresh", "values": {"newName": "Updated"}}))]).unwrap_err();
            use crate::infrastructure::native_operations::apply::ApplyStagingErrorKind;
            assert!(
                matches!(
                    error.kind(),
                    ApplyPlanErrorKind::Staging(
                        ApplyStagingErrorKind::ContainmentIdentity
                            | ApplyStagingErrorKind::UnsupportedProvider
                    )
                ),
                "{source}, {directory}: {error}"
            );
            assert!(error.to_string().contains("link"), "{error}");
            assert_eq!(
                error.path(),
                Some(if source {
                    "ops[0].args.at"
                } else {
                    "ops[0].args.values.newName"
                })
            );
            assert_eq!(fixture.disk_bytes(), original);
            assert_eq!(std::fs::read(&foreign).unwrap(), b"foreign module");
        }
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
                    json!({"at": "main:Document.Order.TabularSection.Lines", "items": [{"name": "Price"}]}),
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
                    json!({"at": "main:Enum.Kinds.EnumValue.Retail", "values": {"synonym": "Розничная"}}),
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
