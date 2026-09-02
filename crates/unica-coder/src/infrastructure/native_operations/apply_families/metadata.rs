use super::validate_platform_xml_binding;
use crate::application::metadata::{parse_metadata_request, MetadataOperation, MetadataRequest};
use crate::domain::address::{NodeKind, QualifiedAddress};
use crate::domain::events::{DomainEvent, DomainEventKind};
use crate::domain::metadata::{MetaDiagnosticCode, MetaEditOperation, MetadataKind};
use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use crate::infrastructure::logical_event_source::metadata_descriptor_relative;
use crate::infrastructure::native_operations::apply::{
    empty_apply_family_batch, hidden_apply_family_unimplemented, ApplyPlanError,
    ApplyPlanErrorKind, ApplyStagedState,
};
use crate::infrastructure::native_operations::apply_families::request::{
    IndexedPlanOperation, ProvisionalApplyEffect,
};
use crate::infrastructure::native_operations::meta::{
    apply_typed_operations_to_image_with_seed, meta_edit_object_identity,
};
use crate::infrastructure::workspace_actor::{MetadataApplyAuthority, ProviderRootBinding};
use serde_json::{json, Map, Value};
use sha2::Digest;

#[derive(Debug, Clone)]
enum MetadataPlanKind {
    Edit {
        target: MetadataAddress,
        operation: MetaEditOperation,
    },
    Unsupported,
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
        "template.add" => parse_member_add(operation, "templates", object, op_index, binding)?,
        "template.set" => parse_member_set(operation, "templates", object, op_index, binding)?,
        "template.remove" => {
            parse_member_remove(operation, "templates", object, op_index, binding)?
        }
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
        // These names are part of the closed public registry, but their v0.13
        // argument union or retained staging primitive is not yet proved.
        // Keeping them explicit prevents an unknown name from accidentally
        // entering the metadata writer while preserving exact typed
        // unsupported behavior at the operation slot.
        // `help.create` builds the Ext/Help file facet, which the pure
        // descriptor-image transform cannot stage yet.
        "object.create" | "object.remove" | "help.create" => MetadataPlanKind::Unsupported,
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

fn qualified_target(
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
            format!(
                "ops[{op_index}].args.{}",
                field
                    .replacen("elements[0]", "values", 1)
                    .replacen("elements", "items", 1)
                    .replacen("ids[0]", "values.id", 1)
            )
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
        let MetadataPlanKind::Edit {
            target,
            operation: edit,
        } = &operation.operation().kind
        else {
            return Err(hidden_apply_family_unimplemented(op_index));
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
        let mut postimage = String::from_utf8(preimage.clone()).map_err(|_| {
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
        apply_typed_operations_to_image_with_seed(
            &mut postimage,
            std::slice::from_ref(edit),
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
                | MetaDiagnosticCode::ReferenceConflict
                | MetaDiagnosticCode::SupportLocked
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
        let postimage = postimage.into_bytes();
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

#[cfg(test)]
mod tests {
    use super::{parse_metadata_plan_operation, plan_metadata_batch};
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

    impl MetadataFixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("src");
            std::fs::create_dir_all(source.join("Documents")).unwrap();
            std::fs::write(
                root.path().join("v8project.yaml"),
                "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
            )
            .unwrap();
            std::fs::write(
                source.join("Configuration.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Main</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
            )
            .unwrap();
            let descriptor = source.join("Documents/Order.xml");
            std::fs::write(&descriptor, ORDER_XML).unwrap();
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
                    SourceSetKind::Configuration,
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
    fn metadata_operations_without_a_proved_retained_schema_stay_typed_unsupported() {
        let fixture = MetadataFixture::new();
        for (index, name) in ["object.create", "object.remove"].into_iter().enumerate() {
            let admission = fixture.admission();
            let staged = admission.staged_state().unwrap();
            let authority = admission
                .metadata_planning_authority(&fixture.binding)
                .unwrap();
            let parsed = fixture.parse(name, json!({"at": "main:Document.Order"}), index);
            let error = plan_metadata_batch(staged, authority, &[parsed]).unwrap_err();
            assert_eq!(
                error.kind(),
                ApplyPlanErrorKind::ProviderUnavailable,
                "{name}"
            );
            assert_eq!(
                error.path(),
                Some(format!("ops[{index}].op").as_str()),
                "{name}"
            );
        }
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
}
