//! Canonical metadata operation decoding from immutable admitted source facts.
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_metadata::apply_operations::MetadataOperation as RegisteredMetadataOperation;
use unica_metadata::contracts::{
    metadata_help_create_input_schema, metadata_object_create_input_schema,
    metadata_object_remove_input_schema, metadata_relation_input_schema,
};
use unica_metadata::descriptor_operations::{PropertiesOperation, PropertiesSite};
use unica_metadata::{MetaCollection, MetaEditOperation, MetadataKind};
use unica_operation_contract::ArgumentShape;
use unica_operations::{OperationDescriptor, OperationRegistry};
use unica_source_contract::proposal::{
    hidden_apply_family_unimplemented, ApplyPlanError, ApplyPlanErrorKind,
};
pub struct MetadataDecodeContext<'a> {
    pub source_set_name: &'a str,
    /// The addressed source set's kind selects the root property profile.
    pub source_kind: unica_source_contract::SourceSetKind,
    pub registry: OperationRegistry,
}
impl MetadataDecodeContext<'_> {
    fn source_set_name(&self) -> &str {
        self.source_set_name
    }
}
#[derive(Debug, Clone)]
pub enum MetadataPlanKind {
    ExchangeContent {
        owner: MetadataAddress,
        items: Vec<unica_metadata::MetaExchangeContentItem>,
    },
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
        items: Vec<(String, unica_metadata::MetaTemplateKind)>,
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
    /// Role and subsystem descriptors keep their own typed patches while
    /// sharing the existing staged-state and publication path.
    DescriptorProperties {
        name: String,
        relative: PathBuf,
        changes: DescriptorPropertyChanges,
    },
}
#[derive(Debug, Clone)]
pub enum DescriptorPropertyChanges {
    Role(unica_metadata::property_input::RolePropertyChanges),
    Subsystem(unica_metadata::property_input::SubsystemPropertyChanges),
    Configuration(unica_metadata::property_input::ConfigurationPropertyChanges),
}
/// Where the caller wrote the fields of an addressed typed edit: the typed
/// writer names them in its own request (`elements[0]`, `names[0]`, `scope`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressedFields {
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
    pub fn caller_field(self, field: &str) -> Option<String> {
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
pub enum PredefinedChange {
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
pub struct MetadataPlanOperation {
    pub kind: MetadataPlanKind,
}

impl MetadataPlanOperation {
    pub fn exchange_content_owner(&self) -> Option<&MetadataAddress> {
        match &self.kind {
            MetadataPlanKind::ExchangeContent { owner, .. } => Some(owner),
            _ => None,
        }
    }
}
pub fn parse_metadata_plan_operation_in(
    descriptor: Option<OperationDescriptor>,
    _operation: &str,
    args: &Value,
    op_index: usize,
    binding: &MetadataDecodeContext<'_>,
) -> Result<MetadataPlanOperation, ApplyPlanError> {
    let object = args.as_object().ok_or_else(|| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "operation args must be an object",
        )
        .at_path(format!("ops[{op_index}].args"))
    })?;
    if let Some(operation) =
        descriptor.and_then(
            OperationDescriptor::operation::<
                unica_metadata::descriptor_operations::PropertiesOperation,
            >,
        )
    {
        let kind = parse_properties(operation, object, op_index, binding)?;
        return Ok(MetadataPlanOperation { kind });
    }
    if let Some(operation) = descriptor.and_then(
        OperationDescriptor::operation::<unica_metadata::apply_operations::MetadataOperation>,
    ) {
        let member_contract = operation
            .member_collection()
            .map(|_| operation.contract_at(None));
        let member_shape = member_contract
            .as_ref()
            .map(|contract| ArgumentShape::new(&contract.schema));
        // Inline members retain their shared envelope checks. Other Metadata
        // operations visit their argument shape in their own established order.
        if let Some(shape) = member_shape {
            reject_unknown_args_with(
                operation.name(),
                object,
                |key| shape.property(key).is_some(),
                op_index,
                binding.registry,
            )?;
        }
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
            RegisteredMetadataOperation::AttributeCopy => {
                let contract = operation.contract_at(None);
                let shape = ArgumentShape::new(&contract.schema);
                reject_unknown_args_with(
                    operation.name(),
                    object,
                    |key| shape.property(key).is_some(),
                    op_index,
                    binding.registry,
                )?;
                parse_attribute_copy(object, op_index, binding)?
            }
            RegisteredMetadataOperation::TabularSectionAdd => parse_member_add(
                operation
                    .member_collection()
                    .expect("inline member collection"),
                object,
                op_index,
                binding,
            )?,
            RegisteredMetadataOperation::TabularSectionSet => parse_addressed_member_change(
                operation.name(),
                NodeKind::TabularSection,
                operation
                    .member_collection()
                    .expect("inline member collection"),
                AddressedMemberChange::Set,
                object,
                op_index,
                binding,
            )?,
            RegisteredMetadataOperation::TabularSectionRemove => parse_addressed_member_change(
                operation.name(),
                NodeKind::TabularSection,
                operation
                    .member_collection()
                    .expect("inline member collection"),
                AddressedMemberChange::Remove,
                object,
                op_index,
                binding,
            )?,
            RegisteredMetadataOperation::DimensionAdd => parse_member_add(
                operation
                    .member_collection()
                    .expect("inline member collection"),
                object,
                op_index,
                binding,
            )?,
            RegisteredMetadataOperation::DimensionSet => parse_addressed_member_change(
                operation.name(),
                NodeKind::Dimension,
                operation
                    .member_collection()
                    .expect("inline member collection"),
                AddressedMemberChange::Set,
                object,
                op_index,
                binding,
            )?,
            RegisteredMetadataOperation::DimensionRemove => parse_addressed_member_change(
                operation.name(),
                NodeKind::Dimension,
                operation
                    .member_collection()
                    .expect("inline member collection"),
                AddressedMemberChange::Remove,
                object,
                op_index,
                binding,
            )?,
            RegisteredMetadataOperation::ResourceAdd => parse_member_add(
                operation
                    .member_collection()
                    .expect("inline member collection"),
                object,
                op_index,
                binding,
            )?,
            RegisteredMetadataOperation::ResourceSet => parse_addressed_member_change(
                operation.name(),
                NodeKind::Resource,
                operation
                    .member_collection()
                    .expect("inline member collection"),
                AddressedMemberChange::Set,
                object,
                op_index,
                binding,
            )?,
            RegisteredMetadataOperation::ResourceRemove => parse_addressed_member_change(
                operation.name(),
                NodeKind::Resource,
                operation
                    .member_collection()
                    .expect("inline member collection"),
                AddressedMemberChange::Remove,
                object,
                op_index,
                binding,
            )?,
            RegisteredMetadataOperation::EnumValueAdd => parse_member_add(
                operation
                    .member_collection()
                    .expect("inline member collection"),
                object,
                op_index,
                binding,
            )?,
            RegisteredMetadataOperation::EnumValueSet => parse_addressed_member_change(
                operation.name(),
                NodeKind::EnumValue,
                operation
                    .member_collection()
                    .expect("inline member collection"),
                AddressedMemberChange::Set,
                object,
                op_index,
                binding,
            )?,
            RegisteredMetadataOperation::EnumValueRemove => parse_addressed_member_change(
                operation.name(),
                NodeKind::EnumValue,
                operation
                    .member_collection()
                    .expect("inline member collection"),
                AddressedMemberChange::Remove,
                object,
                op_index,
                binding,
            )?,
            RegisteredMetadataOperation::ColumnAdd => parse_member_add(
                operation
                    .member_collection()
                    .expect("inline member collection"),
                object,
                op_index,
                binding,
            )?,
            RegisteredMetadataOperation::ColumnSet => parse_addressed_member_change(
                operation.name(),
                NodeKind::Column,
                operation
                    .member_collection()
                    .expect("inline member collection"),
                AddressedMemberChange::Set,
                object,
                op_index,
                binding,
            )?,
            RegisteredMetadataOperation::ColumnRemove => parse_addressed_member_change(
                operation.name(),
                NodeKind::Column,
                operation
                    .member_collection()
                    .expect("inline member collection"),
                AddressedMemberChange::Remove,
                object,
                op_index,
                binding,
            )?,
            RegisteredMetadataOperation::CommandAdd => parse_member_add(
                operation
                    .member_collection()
                    .expect("inline member collection"),
                object,
                op_index,
                binding,
            )?,
            RegisteredMetadataOperation::CommandSet => parse_addressed_member_change(
                operation.name(),
                NodeKind::Command,
                operation
                    .member_collection()
                    .expect("inline member collection"),
                AddressedMemberChange::Set,
                object,
                op_index,
                binding,
            )?,
            RegisteredMetadataOperation::CommandRemove => parse_addressed_member_change(
                operation.name(),
                NodeKind::Command,
                operation
                    .member_collection()
                    .expect("inline member collection"),
                AddressedMemberChange::Remove,
                object,
                op_index,
                binding,
            )?,
            RegisteredMetadataOperation::ObjectBorrow => {
                // The family entry dispatches Borrow to its existing carrier
                // before this lower MetadataPlan parser is called.
                return Err(hidden_apply_family_unimplemented(op_index));
            }
            RegisteredMetadataOperation::ObjectCreate => {
                parse_object_create(object, op_index, binding)?
            }
            RegisteredMetadataOperation::ObjectRemove => {
                parse_object_remove(object, op_index, binding)?
            }
            RegisteredMetadataOperation::TemplateAdd => {
                parse_template_add(object, op_index, binding)?
            }
            RegisteredMetadataOperation::TemplateSet => {
                parse_template_set(object, op_index, binding)?
            }
            RegisteredMetadataOperation::TemplateRemove => {
                parse_template_remove(object, op_index, binding)?
            }
            RegisteredMetadataOperation::PredefinedItemAdd => {
                parse_predefined_add(object, op_index, binding)?
            }
            RegisteredMetadataOperation::PredefinedItemSet => {
                parse_predefined_set(object, op_index, binding)?
            }
            RegisteredMetadataOperation::PredefinedItemRemove => {
                parse_predefined_remove(object, op_index, binding)?
            }
            RegisteredMetadataOperation::RelationAdd
            | RegisteredMetadataOperation::RelationRemove
            | RegisteredMetadataOperation::RelationReplace => {
                parse_relation(operation, object, op_index, binding)?
            }
            RegisteredMetadataOperation::HelpCreate => {
                parse_help_create(object, op_index, binding)?
            }
            RegisteredMetadataOperation::ContentSet => {
                parse_exchange_content(object, op_index, binding)?
            }
        };
        // Convert the target and payload first to retain their diagnostics.
        // Only the shared envelope's requiredness and container types remain;
        // nested fields were visited by the staged Metadata argument reader.
        if let Some(shape) = member_shape {
            for key in shape.required_names() {
                if !object.contains_key(key) {
                    return Err(ApplyPlanError::new(
                        ApplyPlanErrorKind::BadValue,
                        format!("`{key}` is required"),
                    )
                    .at_path(format!("ops[{op_index}].args.{key}")));
                }
            }
            for (key, value) in object {
                shape
                    .property(key)
                    .expect("known operation argument")
                    .validate_type(value, &format!("ops[{op_index}].args.{key}"))
                    .map_err(|error| {
                        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, error.message)
                            .at_path(error.path)
                    })?;
            }
        }
        return Ok(MetadataPlanOperation { kind });
    }
    Err(hidden_apply_family_unimplemented(op_index))
}
pub fn reject_unknown_args(
    operation: &str,
    args: &Map<String, Value>,
    allowed: &[&str],
    op_index: usize,
    registry: unica_operations::OperationRegistry,
) -> Result<(), ApplyPlanError> {
    reject_unknown_args_with(
        operation,
        args,
        |key| allowed.contains(&key),
        op_index,
        registry,
    )
}
pub fn reject_unknown_args_with(
    operation: &str,
    args: &Map<String, Value>,
    accepts: impl Fn(&str) -> bool,
    op_index: usize,
    registry: unica_operations::OperationRegistry,
) -> Result<(), ApplyPlanError> {
    if let Some(field) = args.keys().find(|field| !accepts(field)) {
        // The refusal names the expected skeleton so the caller's first retry
        // does not have to discover the argument shape by another refusal.
        let expected = registry
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
pub fn reject_unknown_input_args(
    operation: &str,
    args: &Map<String, Value>,
    schema: &Value,
    op_index: usize,
    registry: unica_operations::OperationRegistry,
) -> Result<(), ApplyPlanError> {
    let shape = ArgumentShape::new(schema);
    reject_unknown_args_with(
        operation,
        args,
        |key| shape.property(key).is_some(),
        op_index,
        registry,
    )
}
/// Keep conversion diagnostics at their existing stage, then consult the
/// shared field type. Compound children are visited by their typed reader.
pub fn check_input_field(
    shape: ArgumentShape<'_>,
    name: &str,
    value: &Value,
    path: &str,
) -> Result<(), ApplyPlanError> {
    shape
        .property(name)
        .expect("known Metadata input field")
        .validate_field(value, path)
        .map_err(|error| {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, error.message).at_path(error.path)
        })
}
pub fn required_object<'a>(
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
pub fn required_array<'a>(
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
pub fn parse_template_add(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &MetadataDecodeContext<'_>,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    let normalized = unica_metadata::contracts::metadata_template_add()
        .normalize(
            &Value::Object(args.clone()),
            &format!("ops[{op_index}].args"),
        )
        .map_err(|error| {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, error.message).at_path(error.path)
        })?;
    let args = normalized.as_object().expect("object contract");
    let target = unica_source_contract::proposal::input::qualified_target(
        args,
        op_index,
        binding.source_set_name(),
    )?;
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
        if !unica_metadata::identifier::is_1c_identifier(name) {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::BadValue,
                format!("`{name}` is not a valid 1C identifier"),
            )
            .at_path(format!("{location}.name")));
        }
        let kind = match item.get("templateType").and_then(Value::as_str) {
            None => unica_metadata::MetaTemplateKind::SpreadsheetDocument,
            Some(raw) => unica_metadata::MetaTemplateKind::parse(raw).map_err(|diagnostic| {
                ApplyPlanError::new(ApplyPlanErrorKind::BadValue, diagnostic.message)
                    .at_path(format!("{location}.templateType"))
            })?,
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
pub fn template_target(
    target: &QualifiedAddress,
    op_index: usize,
) -> Result<(MetadataAddress, String), ApplyPlanError> {
    let (owner, _, name) = metadata_member_target(target, NodeKind::Template, op_index)?;
    Ok((owner, name))
}
pub fn parse_template_set(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &MetadataDecodeContext<'_>,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    let contract = RegisteredMetadataOperation::TemplateSet.contract_at(None);
    reject_unknown_input_args(
        "template.set",
        args,
        &contract.schema,
        op_index,
        binding.registry,
    )?;
    let target = unica_source_contract::proposal::input::qualified_target(
        args,
        op_index,
        binding.source_set_name(),
    )?;
    let (owner, name) = template_target(&target, op_index)?;
    let values = required_object(args, "values", op_index)?.clone();
    let shape = ArgumentShape::new(&contract.schema);
    check_input_field(
        shape,
        "values",
        &args["values"],
        &format!("ops[{op_index}].args.values"),
    )?;
    let values_shape = shape.property("values").expect("template.set values");
    if let Some(field) = values
        .keys()
        .find(|field| values_shape.property(field).is_none())
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
            .at_path(location.clone()));
        };
        if field == "templateType" {
            unica_metadata::MetaTemplateKind::parse(text).map_err(|diagnostic| {
                ApplyPlanError::new(ApplyPlanErrorKind::BadValue, diagnostic.message)
                    .at_path(location.clone())
            })?;
        }
        check_input_field(values_shape, field, value, &location)?;
    }
    Ok(MetadataPlanKind::TemplateSet {
        owner,
        name,
        values,
    })
}
pub fn parse_template_remove(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &MetadataDecodeContext<'_>,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    let contract = RegisteredMetadataOperation::TemplateRemove.contract_at(None);
    reject_unknown_input_args(
        "template.remove",
        args,
        &contract.schema,
        op_index,
        binding.registry,
    )?;
    let target = unica_source_contract::proposal::input::qualified_target(
        args,
        op_index,
        binding.source_set_name(),
    )?;
    let (owner, name) = template_target(&target, op_index)?;
    Ok(MetadataPlanKind::TemplateRemove { owner, name })
}
pub fn parse_object_create(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &MetadataDecodeContext<'_>,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    let schema = metadata_object_create_input_schema();
    let shape = ArgumentShape::new(schema);
    reject_unknown_input_args("object.create", args, schema, op_index, binding.registry)?;
    let target = unica_source_contract::proposal::input::qualified_target(
        args,
        op_index,
        binding.source_set_name(),
    )?;
    if !matches!(target.segments(), [root] if root.kind() == NodeKind::Configuration) {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "object.create targets the configuration root; name the new object in `values`",
        )
        .at_path(format!("ops[{op_index}].args.at")));
    }
    let values = required_object(args, "values", op_index)?;
    check_input_field(
        shape,
        "values",
        &args["values"],
        &format!("ops[{op_index}].args.values"),
    )?;
    let values_shape = shape.property("values").expect("object.create values");
    if let Some(field) = values
        .keys()
        .find(|field| values_shape.property(field).is_none())
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
    check_input_field(
        values_shape,
        "kind",
        &values["kind"],
        &format!("ops[{op_index}].args.values.kind"),
    )?;
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
    check_input_field(
        values_shape,
        "name",
        &values["name"],
        &format!("ops[{op_index}].args.values.name"),
    )?;
    Ok(MetadataPlanKind::Create {
        kind,
        name: name.to_string(),
    })
}
pub fn parse_object_remove(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &MetadataDecodeContext<'_>,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    let schema = metadata_object_remove_input_schema();
    reject_unknown_input_args("object.remove", args, schema, op_index, binding.registry)?;
    let target = unica_source_contract::proposal::input::qualified_target(
        args,
        op_index,
        binding.source_set_name(),
    )?;
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
            .at_path(format!("ops[{op_index}].args.force")));
        }
    };
    if let Some(value) = args.get("force") {
        check_input_field(
            ArgumentShape::new(schema),
            "force",
            value,
            &format!("ops[{op_index}].args.force"),
        )?;
    }
    Ok(MetadataPlanKind::Remove {
        target: address,
        kind,
        name,
        force,
    })
}
pub fn parse_help_create(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &MetadataDecodeContext<'_>,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    let schema = metadata_help_create_input_schema();
    let shape = ArgumentShape::new(schema);
    reject_unknown_input_args("help.create", args, schema, op_index, binding.registry)?;
    let target = unica_source_contract::proposal::input::qualified_target(
        args,
        op_index,
        binding.source_set_name(),
    )?;
    let (address, _) = metadata_owner(&target, op_index)?;
    let name = target.segments()[0]
        .name()
        .expect("metadata_owner proved the segment is named")
        .to_string();
    let values = required_object(args, "values", op_index)?;
    check_input_field(
        shape,
        "values",
        &args["values"],
        &format!("ops[{op_index}].args.values"),
    )?;
    let values_shape = shape.property("values").expect("help.create values");
    if let Some(field) = values
        .keys()
        .find(|field| values_shape.property(field).is_none())
    {
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
    check_input_field(
        values_shape,
        "lang",
        &values["lang"],
        &format!("ops[{op_index}].args.values.lang"),
    )?;
    Ok(MetadataPlanKind::HelpCreate {
        target: address,
        name,
        lang: lang.to_string(),
    })
}
pub fn metadata_owner(
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
pub fn attribute_owner_and_name(
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
            .at_path(format!("ops[{op_index}].args.at")));
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
pub fn parse_legacy_edit(
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
    let request = unica_metadata::request::parse_metadata_request(
        unica_metadata::request::MetadataOperation::Edit,
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
    let unica_metadata::request::MetadataRequest::Edit(request) = request else {
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
pub fn parse_properties(
    operation: PropertiesOperation,
    args: &Map<String, Value>,
    op_index: usize,
    binding: &MetadataDecodeContext<'_>,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    let target = unica_source_contract::proposal::input::qualified_target(
        args,
        op_index,
        binding.source_set_name(),
    )?;
    let site = if matches!(
        target.segments(),
        [root] if root.kind() == NodeKind::Configuration && root.name().is_none()
    ) {
        PropertiesSite::root_of(binding.source_kind)
    } else {
        PropertiesSite::from_address(&target)
    }
    .ok_or_else(|| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "properties require the configuration root, a named top-level metadata object, role or subsystem",
        )
        .at_path(format!("ops[{op_index}].args.at"))
    })?;
    // ApplyRequest has already resolved the operation address. It is an
    // internal carrier key, not part of the published property arguments.
    let mut payload = args.clone();
    payload.remove("at");
    let contract = operation.contract(site);
    reject_unknown_input_args(
        operation.name(),
        &payload,
        &contract.schema,
        op_index,
        binding.registry,
    )?;
    let intent = unica_metadata::property_input::decode_properties(
        operation,
        site,
        &Value::Object(payload),
        &format!("ops[{op_index}].args"),
    )
    .map_err(|error| {
        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, error.message).at_path(error.path)
    })?;
    let (directory, changes) = match intent {
        unica_metadata::property_input::PropertiesIntent::Metadata(values) => {
            let (target, _) = metadata_owner(&target, op_index)?;
            return Ok(MetadataPlanKind::Edit {
                target,
                operation: MetaEditOperation::SetProperties { values },
                addressed: None,
            });
        }
        unica_metadata::property_input::PropertiesIntent::Role(values) => {
            ("Roles", DescriptorPropertyChanges::Role(values))
        }
        unica_metadata::property_input::PropertiesIntent::Subsystem(values) => {
            ("Subsystems", DescriptorPropertyChanges::Subsystem(values))
        }
        unica_metadata::property_input::PropertiesIntent::Configuration(values) => {
            return Ok(MetadataPlanKind::DescriptorProperties {
                relative: PathBuf::from("Configuration.xml"),
                name: "Configuration".to_string(),
                changes: DescriptorPropertyChanges::Configuration(values),
            });
        }
    };
    let name = target.segments()[0]
        .name()
        .expect("proven named property owner")
        .to_string();
    Ok(MetadataPlanKind::DescriptorProperties {
        relative: PathBuf::from(directory).join(format!("{name}.xml")),
        name,
        changes,
    })
}
/// `attribute.add` on the owner adds header attributes; on
/// `Owner.Name.TabularSection.T` it adds attributes of that tabular section.
pub fn parse_attribute_add(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &MetadataDecodeContext<'_>,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    let target = unica_source_contract::proposal::input::qualified_target(
        args,
        op_index,
        binding.source_set_name(),
    )?;
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
            .at_path(format!("ops[{op_index}].args.at")));
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
pub fn parse_attribute_set(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &MetadataDecodeContext<'_>,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    let target = unica_source_contract::proposal::input::qualified_target(
        args,
        op_index,
        binding.source_set_name(),
    )?;
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
pub fn parse_attribute_remove(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &MetadataDecodeContext<'_>,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    let target = unica_source_contract::proposal::input::qualified_target(
        args,
        op_index,
        binding.source_set_name(),
    )?;
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
/// `attribute.copy` on `Catalog.C.TabularSection.T.Attribute.A`: the source
/// is the address, `values.name` the copy's name.
pub fn parse_attribute_copy(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &MetadataDecodeContext<'_>,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    let target = unica_source_contract::proposal::input::qualified_target(
        args,
        op_index,
        binding.source_set_name(),
    )?;
    let (owner, kind, name, scope) = attribute_owner_and_name(&target, op_index)?;
    let Some(section) = scope.filter(|_| kind == MetadataKind::Catalog) else {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "attribute.copy copies an attribute of a catalog tabular section: `Catalog.C.TabularSection.T.Attribute.A`",
        )
        .at_path(format!("ops[{op_index}].args.at")));
    };
    let values = required_object(args, "values", op_index)?;
    if let Some(field) = values.keys().find(|field| *field != "name") {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "attribute.copy takes only the copy's name; the rest comes from the source",
        )
        .at_path(format!("ops[{op_index}].args.values.{field}")));
    }
    let new_name = values
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| unica_metadata::identifier::is_1c_identifier(name))
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::BadValue,
                "values.name must be a valid 1C identifier",
            )
            .at_path(format!("ops[{op_index}].args.values.name"))
        })?;
    Ok(MetadataPlanKind::Edit {
        target: owner,
        operation: MetaEditOperation::Copy {
            collection: MetaCollection::Attributes,
            scope: Some(unica_metadata::MetaScope {
                tabular_section: section,
            }),
            name,
            new_name: new_name.to_string(),
        },
        addressed: Some(AddressedFields::Values),
    })
}
/// One member-collection add: `at` names the owner, `items` carry the new
/// elements exactly as the typed metadata contract defines them.
pub fn parse_member_add(
    collection: MetaCollection,
    args: &Map<String, Value>,
    op_index: usize,
    binding: &MetadataDecodeContext<'_>,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    let target = unica_source_contract::proposal::input::qualified_target(
        args,
        op_index,
        binding.source_set_name(),
    )?;
    let (owner, kind) = metadata_owner(&target, op_index)?;
    let items = required_array(args, "items", op_index)?.to_vec();
    parse_legacy_edit(
        binding.source_set_name(),
        owner,
        kind,
        json!({"op": "add", "collection": collection.as_str(), "elements": items}),
        |field| {
            format!(
                "ops[{op_index}].args.{}",
                field.replacen("elements", "items", 1)
            )
        },
        op_index,
    )
}
#[derive(Debug, Clone, Copy)]
pub enum AddressedMemberChange {
    Set,
    Remove,
}
pub fn parse_addressed_member_change(
    operation: &str,
    member_kind: NodeKind,
    collection: MetaCollection,
    change: AddressedMemberChange,
    args: &Map<String, Value>,
    op_index: usize,
    binding: &MetadataDecodeContext<'_>,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    let target = unica_source_contract::proposal::input::qualified_target(
        args,
        op_index,
        binding.source_set_name(),
    )?;
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
pub fn addressed_edit(kind: MetadataPlanKind, fields: AddressedFields) -> MetadataPlanKind {
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
pub fn metadata_member_target(
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
pub fn edit_operation(kind: MetadataPlanKind) -> MetaEditOperation {
    match kind {
        MetadataPlanKind::Edit { operation, .. } => operation,
        _ => unreachable!("the typed edit parser returns an edit"),
    }
}
/// `predefinedItem.add` on the owner: new items carry their `id` and `name`.
pub fn parse_predefined_add(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &MetadataDecodeContext<'_>,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    let contract = RegisteredMetadataOperation::PredefinedItemAdd.contract_at(None);
    reject_unknown_input_args(
        "predefinedItem.add",
        args,
        &contract.schema,
        op_index,
        binding.registry,
    )?;
    let target = unica_source_contract::proposal::input::qualified_target(
        args,
        op_index,
        binding.source_set_name(),
    )?;
    let (owner, kind) = metadata_owner(&target, op_index)?;
    let items = required_array(args, "items", op_index)?.to_vec();
    check_input_field(
        ArgumentShape::new(&contract.schema),
        "items",
        &args["items"],
        &format!("ops[{op_index}].args.items"),
    )?;
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
pub fn parse_predefined_set(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &MetadataDecodeContext<'_>,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    let contract = RegisteredMetadataOperation::PredefinedItemSet.contract_at(None);
    reject_unknown_input_args(
        "predefinedItem.set",
        args,
        &contract.schema,
        op_index,
        binding.registry,
    )?;
    let target = unica_source_contract::proposal::input::qualified_target(
        args,
        op_index,
        binding.source_set_name(),
    )?;
    let (owner, kind, name) = metadata_member_target(&target, NodeKind::PredefinedItem, op_index)?;
    let values = required_object(args, "values", op_index)?.clone();
    // Only the already-converted container is checked here. Selector presence
    // errors still precede the old typed inner reader and its unknown keys.
    check_input_field(
        ArgumentShape::new(&contract.schema),
        "values",
        &args["values"],
        &format!("ops[{op_index}].args.values"),
    )?;
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
pub fn parse_predefined_remove(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &MetadataDecodeContext<'_>,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    let contract = RegisteredMetadataOperation::PredefinedItemRemove.contract_at(None);
    reject_unknown_input_args(
        "predefinedItem.remove",
        args,
        &contract.schema,
        op_index,
        binding.registry,
    )?;
    let target = unica_source_contract::proposal::input::qualified_target(
        args,
        op_index,
        binding.source_set_name(),
    )?;
    let (owner, kind, name) = metadata_member_target(&target, NodeKind::PredefinedItem, op_index)?;
    Ok(MetadataPlanKind::Predefined {
        owner,
        kind,
        change: PredefinedChange::Remove { name },
    })
}
/// The typed update of one predefined item: `values` with `newName` mapped to
/// the item's `name` field, the item selected by `id`.
pub fn predefined_update(
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
/// Object kinds whose changes an ExchangePlan composition can record.
const EXCHANGE_CONTENT_KINDS: &[NodeKind] = &[
    NodeKind::Catalog,
    NodeKind::Document,
    NodeKind::Constant,
    NodeKind::InformationRegister,
    NodeKind::AccumulationRegister,
    NodeKind::ChartOfCharacteristicTypes,
    NodeKind::BusinessProcess,
    NodeKind::Task,
    NodeKind::ChartOfAccounts,
    NodeKind::ChartOfCalculationTypes,
    NodeKind::AccountingRegister,
    NodeKind::CalculationRegister,
];

pub fn parse_exchange_content(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &MetadataDecodeContext<'_>,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    use unica_metadata::{MetaExchangeAutoRecord, MetaExchangeContentItem};
    reject_unknown_args(
        "content.set",
        args,
        &["at", "items"],
        op_index,
        binding.registry,
    )?;
    let (owner, kind) = metadata_owner(
        &unica_source_contract::proposal::input::qualified_target(
            args,
            op_index,
            binding.source_set_name(),
        )?,
        op_index,
    )?;
    let bad = |message: &str| {
        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message)
            .at_path(format!("ops[{op_index}].args.items"))
    };
    if kind != MetadataKind::ExchangePlan {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "content.set requires an ExchangePlan",
        )
        .at_path(format!("ops[{op_index}].args.at")));
    }
    let values = args
        .get("items")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
        .ok_or_else(|| bad("items must be a nonempty array"))?;
    let mut items = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (index, item) in values.iter().enumerate() {
        let item_path = format!("ops[{op_index}].args.items[{index}]");
        let bad_field = |field: &str, message: &str| {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(
                if field.is_empty() {
                    item_path.clone()
                } else {
                    format!("{item_path}.{field}")
                },
            )
        };
        let object = item
            .as_object()
            .ok_or_else(|| bad_field("", "content item must be an object"))?;
        if let Some(key) = object
            .keys()
            .find(|k| !matches!(k.as_str(), "at" | "autoRecord"))
        {
            return Err(bad_field(
                key,
                "content item expects only at and autoRecord",
            ));
        }
        let target = unica_source_contract::proposal::input::qualified_target(
            object,
            op_index,
            binding.source_set_name(),
        )
        .map_err(|e| e.at_path(format!("{item_path}.at")))?;
        let [segment] = target.segments() else {
            return Err(bad_field("at", "content target must be a top-level object"));
        };
        let name = segment
            .name()
            .ok_or_else(|| bad_field("at", "content target must be a named object"))?;
        if !EXCHANGE_CONTENT_KINDS.contains(&segment.kind()) {
            return Err(bad_field(
                "at",
                "content target must be a Catalog, Document, Constant, ChartOfCharacteristicTypes, ChartOfAccounts, ChartOfCalculationTypes, InformationRegister, AccumulationRegister, AccountingRegister, CalculationRegister, BusinessProcess or Task",
            ));
        }
        let target = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            &format!("{}.{name}", segment.kind().as_str()),
        )
        .map_err(|_| bad_field("at", "content target must be a metadata object"))?;
        if !seen.insert(target.as_str().to_lowercase()) {
            return Err(bad_field("at", "content target is repeated in items"));
        }
        let auto_record = match object.get("autoRecord").and_then(Value::as_str) {
            Some("Allow") => MetaExchangeAutoRecord::Allow,
            Some("Deny") => MetaExchangeAutoRecord::Deny,
            _ => return Err(bad_field("autoRecord", "autoRecord must be Allow or Deny")),
        };
        items.push(MetaExchangeContentItem {
            target,
            auto_record,
        });
    }
    Ok(MetadataPlanKind::ExchangeContent { owner, items })
}

/// One relation edit: `values` carries the closed `relation` name and its
/// `targets`; the mode comes from the operation name itself.
pub fn parse_relation(
    operation: RegisteredMetadataOperation,
    args: &Map<String, Value>,
    op_index: usize,
    binding: &MetadataDecodeContext<'_>,
) -> Result<MetadataPlanKind, ApplyPlanError> {
    let mode = operation
        .relation_mode()
        .expect("registered relation operation");
    let operation = operation.name();
    let schema = metadata_relation_input_schema();
    let shape = ArgumentShape::new(schema);
    reject_unknown_input_args(operation, args, schema, op_index, binding.registry)?;
    let target = unica_source_contract::proposal::input::qualified_target(
        args,
        op_index,
        binding.source_set_name(),
    )?;
    let (owner, kind) = metadata_owner(&target, op_index)?;
    let values = required_object(args, "values", op_index)?;
    check_input_field(
        shape,
        "values",
        &args["values"],
        &format!("ops[{op_index}].args.values"),
    )?;
    let values_shape = shape.property("values").expect("relation values");
    let relation = values.get("relation").cloned().unwrap_or(Value::Null);
    let targets = values.get("targets").cloned().unwrap_or(Value::Null);
    if values
        .keys()
        .any(|key| values_shape.property(key).is_none())
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
        json!({"op": "editRelations", "relation": relation, "mode": mode.as_str(), "targets": targets}),
        |field| format!("ops[{op_index}].args.values.{field}"),
        op_index,
    )
}

const PREDEFINED_PLACEHOLDER_ID: &str = "00000000-0000-4000-8000-000000000000";
