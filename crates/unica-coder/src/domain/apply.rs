use crate::domain::address::{NodeKind, QualifiedAddress};
use crate::domain::metadata::{metadata_kind_collections, MetaCollection, MetadataKind};
use crate::domain::node_view::OperationRef;
use serde_json::{Map, Value};
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ApplyRequest {
    at: QualifiedAddress,
    ops: NonEmptyVec<ApplyOp>,
    dry_run: bool,
    if_rev: Option<String>,
}

impl ApplyRequest {
    pub(crate) fn parse(
        input: &Map<String, Value>,
        available_source_sets: &[&str],
    ) -> Result<Self, ApplyValidationError> {
        for key in input.keys() {
            if !["at", "ops", "dryRun", "ifRev"].contains(&key.as_str()) {
                return Err(ApplyValidationError::bad_value(
                    key,
                    format!("unknown apply argument `{key}`"),
                ));
            }
        }
        let at = parse_address(input.get("at"), "at", available_source_sets)?;
        let raw_ops = input
            .get("ops")
            .and_then(Value::as_array)
            .ok_or_else(|| ApplyValidationError::bad_value("ops", "ops must be an array"))?;
        if raw_ops.is_empty() {
            return Err(ApplyValidationError::bad_value(
                "ops",
                "ops must contain at least one operation",
            ));
        }

        let mut operations = Vec::with_capacity(raw_ops.len());
        for (index, raw_operation) in raw_ops.iter().enumerate() {
            let operation_location = format!("ops[{index}]");
            let object = raw_operation.as_object().ok_or_else(|| {
                ApplyValidationError::bad_value(&operation_location, "operation must be an object")
            })?;
            for key in object.keys() {
                if !["op", "args"].contains(&key.as_str()) {
                    return Err(ApplyValidationError::bad_value(
                        format!("{operation_location}.{key}"),
                        format!("unknown operation member `{key}`"),
                    ));
                }
            }
            let op_location = format!("{operation_location}.op");
            let name = object
                .get("op")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    ApplyValidationError::bad_value(&op_location, "op must be non-empty text")
                })?
                .to_string();
            let args_location = format!("{operation_location}.args");
            let mut args = match object.get("args") {
                None => Map::new(),
                Some(Value::Object(args)) => args.clone(),
                Some(_) => {
                    return Err(ApplyValidationError::bad_value(
                        &args_location,
                        "args must be an object",
                    ))
                }
            };
            let target_location = format!("{args_location}.at");
            let operation_at = match args.get("at") {
                None => at.clone(),
                Some(value) => parse_address(Some(value), &target_location, available_source_sets)?,
            };
            if operation_at.source_set() != at.source_set()
                || operation_at.segments().len() < at.segments().len()
                || operation_at.segments()[..at.segments().len()] != *at.segments()
            {
                return Err(ApplyValidationError::bad_value(
                    &target_location,
                    "operation target must be the same logical node or its descendant in the top-level source set",
                ));
            }
            args.insert("at".to_string(), Value::String(operation_at.to_string()));
            operations.push(ApplyOp {
                name,
                args,
                at: operation_at,
            });
        }

        let dry_run = match input.get("dryRun") {
            None => false,
            Some(Value::Bool(value)) => *value,
            Some(_) => {
                return Err(ApplyValidationError::bad_value(
                    "dryRun",
                    "dryRun must be a boolean",
                ))
            }
        };
        let if_rev = match input.get("ifRev") {
            None => None,
            Some(Value::String(value)) if !value.is_empty() => Some(value.clone()),
            Some(_) => {
                return Err(ApplyValidationError::bad_value(
                    "ifRev",
                    "ifRev must be non-empty text",
                ))
            }
        };
        Ok(Self {
            at,
            ops: NonEmptyVec::new(operations)
                .expect("the request parser rejects an empty operation array"),
            dry_run,
            if_rev,
        })
    }

    pub(crate) fn at(&self) -> &QualifiedAddress {
        &self.at
    }

    pub(crate) fn ops(&self) -> &[ApplyOp] {
        self.ops.as_slice()
    }

    pub(crate) const fn dry_run(&self) -> bool {
        self.dry_run
    }

    pub(crate) fn if_rev(&self) -> Option<&str> {
        self.if_rev.as_deref()
    }
}

fn parse_address(
    value: Option<&Value>,
    location: &str,
    available_source_sets: &[&str],
) -> Result<QualifiedAddress, ApplyValidationError> {
    let raw = value
        .and_then(Value::as_str)
        .ok_or_else(|| ApplyValidationError::bad_value(location, "logical address must be text"))?;
    QualifiedAddress::resolve_input(raw, available_source_sets)
        .map_err(|error| ApplyValidationError::bad_value(location, error.to_string()))
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ApplyOp {
    name: String,
    args: Map<String, Value>,
    at: QualifiedAddress,
}

impl ApplyOp {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn args(&self) -> &Map<String, Value> {
        &self.args
    }

    pub(crate) fn at(&self) -> &QualifiedAddress {
        &self.at
    }
}

#[derive(Debug, Clone, PartialEq)]
struct NonEmptyVec<T>(Vec<T>);

impl<T> NonEmptyVec<T> {
    fn new(values: Vec<T>) -> Option<Self> {
        (!values.is_empty()).then_some(Self(values))
    }

    fn as_slice(&self) -> &[T] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApplyValidationError {
    code: &'static str,
    location: String,
    message: String,
}

impl ApplyValidationError {
    fn bad_value(location: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: "bad_value",
            location: location.into(),
            message: message.into(),
        }
    }

    pub(crate) const fn code(&self) -> &'static str {
        self.code
    }

    pub(crate) fn location(&self) -> &str {
        &self.location
    }
}

impl fmt::Display for ApplyValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.location, self.message)
    }
}

impl std::error::Error for ApplyValidationError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OperationFamily {
    Metadata,
    Properties,
    Form,
    Role,
    Dcs,
    Mxl,
    Xdto,
    Subsystem,
    Support,
    Code,
    Event,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OperationApplicability {
    Metadata,
    MetadataOrSubsystem,
    MetadataAttributes,
    MetadataTabularSections,
    MetadataDimensions,
    MetadataResources,
    MetadataEnumValues,
    MetadataColumns,
    MetadataForms,
    MetadataFormsOrForm,
    MetadataTemplates,
    MetadataCommands,
    MetadataPredefinedItems,
    Form,
    Role,
    RoleOrRoot,
    Dcs,
    Mxl,
    Xdto,
    Subsystem,
    Support,
    Code,
    Event,
}

impl OperationApplicability {
    fn matches(self, kind: NodeKind) -> bool {
        let metadata = kind.is_metadata_kind() || kind == NodeKind::Configuration;
        match self {
            Self::Metadata => metadata,
            Self::MetadataOrSubsystem => metadata || kind == NodeKind::Subsystem,
            Self::MetadataAttributes => {
                metadata_collection_matches(kind, MetaCollection::Attributes)
            }
            Self::MetadataTabularSections => {
                metadata_collection_matches(kind, MetaCollection::TabularSections)
            }
            Self::MetadataDimensions => {
                metadata_collection_matches(kind, MetaCollection::Dimensions)
            }
            Self::MetadataResources => metadata_collection_matches(kind, MetaCollection::Resources),
            Self::MetadataEnumValues => {
                metadata_collection_matches(kind, MetaCollection::EnumValues)
            }
            Self::MetadataColumns => metadata_collection_matches(kind, MetaCollection::Columns),
            Self::MetadataForms => metadata_collection_matches(kind, MetaCollection::Forms),
            Self::MetadataFormsOrForm => {
                metadata_collection_matches(kind, MetaCollection::Forms) || form_kind(kind)
            }
            Self::MetadataTemplates => metadata_collection_matches(kind, MetaCollection::Templates),
            Self::MetadataCommands => metadata_collection_matches(kind, MetaCollection::Commands),
            Self::MetadataPredefinedItems => {
                metadata_collection_matches(kind, MetaCollection::PredefinedItems)
            }
            Self::Form => form_kind(kind),
            Self::Role => matches!(kind, NodeKind::Role | NodeKind::Right | NodeKind::Rls),
            // Creation names the new role at the root or at its own address.
            Self::RoleOrRoot => matches!(kind, NodeKind::Configuration | NodeKind::Role),
            Self::Dcs => matches!(
                kind,
                NodeKind::DataSet
                    | NodeKind::Field
                    | NodeKind::Query
                    | NodeKind::Calculation
                    | NodeKind::Setting
                    | NodeKind::Parameter
            ),
            Self::Mxl => matches!(kind, NodeKind::Template | NodeKind::Area),
            Self::Xdto => matches!(
                kind,
                NodeKind::XdtoPackage | NodeKind::Namespace | NodeKind::Type | NodeKind::Property
            ),
            Self::Subsystem => matches!(kind, NodeKind::Configuration | NodeKind::Subsystem),
            Self::Support => metadata || kind == NodeKind::Subsystem,
            // A common module is its own module terminal: the read projection
            // already shows it as kind `Module`, and the code planner writes it
            // through the same capability as an owner's named module.
            Self::Code => matches!(
                kind,
                NodeKind::Module
                    | NodeKind::Method
                    | NodeKind::Body
                    | NodeKind::Region
                    | NodeKind::CommonModule
            ),
            Self::Event => kind == NodeKind::Event,
        }
    }
}

fn metadata_collection_matches(kind: NodeKind, collection: MetaCollection) -> bool {
    MetadataKind::parse(kind.as_str())
        .ok()
        .is_some_and(|kind| metadata_kind_collections(kind).contains(&collection))
}

fn form_kind(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Form
            | NodeKind::Item
            | NodeKind::Column
            | NodeKind::Attribute
            | NodeKind::Command
            | NodeKind::Event
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OperationDescriptor {
    name: &'static str,
    family: OperationFamily,
    applicability: OperationApplicability,
    skeleton: OperationSkeleton,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OperationSkeleton {
    Items,
    Values,
    Text,
    Target,
}

impl OperationSkeleton {
    /// The single root argument key the operation expects; the published
    /// `can` dictionary and refusal texts both speak this name.
    pub(crate) const fn key(self) -> &'static str {
        match self {
            Self::Items => "items",
            Self::Values => "values",
            Self::Text => "text",
            Self::Target => "at",
        }
    }

    fn args(self) -> Map<String, Value> {
        let value = match self {
            Self::Items => Value::Array(Vec::new()),
            Self::Values => Value::Object(Map::new()),
            Self::Text => Value::String(String::new()),
            Self::Target => Value::String(String::new()),
        };
        Map::from_iter([(self.key().to_string(), value)])
    }
}

/// Apply operations with a proven end-to-end v0.13 planner and publication.
/// The `can` dictionary prints this as `implemented`, mirroring the honesty
/// rule of the Run dictionary: a name in the registry is not support.
pub(crate) const IMPLEMENTED_APPLY_OPERATIONS: &[&str] = &[
    "mxl.set",
    "event.implement",
    "form.add",
    "form.set",
    "form.remove",
    "form.create",
    "element.add",
    "element.remove",
    "formAttribute.add",
    "formCommand.add",
    "event.bind",
    "right.set",
    "role.create",
    "subsystem.create",
    "content.add",
    "content.remove",
    "childSubsystem.add",
    "childSubsystem.remove",
    "supportCapability.set",
    "supportRule.set",
    "field.add",
    "field.set",
    "field.remove",
    "fieldRole.set",
    "parameter.add",
    "parameter.set",
    "parameter.remove",
    "filter.add",
    "filter.clear",
    "selection.add",
    "selection.clear",
    "order.clear",
    "conditionalAppearance.clear",
    "query.set",
    "query.patch",
    "calculatedField.add",
    "total.add",
    "variant.add",
    "structure.set",
    "structure.patch",
    "object.create",
    "object.remove",
    "help.create",
    "props.set",
    "attribute.add",
    "attribute.set",
    "attribute.remove",
    "tabularSection.add",
    "tabularSection.set",
    "tabularSection.remove",
    "dimension.add",
    "dimension.set",
    "dimension.remove",
    "resource.add",
    "resource.set",
    "resource.remove",
    "enumValue.add",
    "enumValue.set",
    "enumValue.remove",
    "column.add",
    "column.set",
    "column.remove",
    "template.add",
    "template.set",
    "template.remove",
    "command.add",
    "command.set",
    "command.remove",
    "predefinedItem.add",
    "predefinedItem.set",
    "predefinedItem.remove",
    "relation.add",
    "relation.replace",
    "relation.remove",
    "code.insert",
    "code.replace",
    "valueType.add",
    "objectType.add",
    "property.add",
    "type.remove",
    "property.remove",
];

impl OperationDescriptor {
    pub(crate) const fn name(self) -> &'static str {
        self.name
    }

    pub(crate) const fn skeleton_key(self) -> &'static str {
        self.skeleton.key()
    }

    pub(crate) const fn family(self) -> OperationFamily {
        self.family
    }

    pub(crate) fn applies_to(self, kind: NodeKind) -> bool {
        self.applicability.matches(kind)
    }

    pub(crate) fn applies_to_operation_target(self, target: &QualifiedAddress) -> bool {
        let Some(terminal) = target.segments().last() else {
            return false;
        };
        if self.applies_to(terminal.kind()) {
            return true;
        }
        matches!(
            self.applicability,
            OperationApplicability::MetadataAttributes
        ) && terminal.kind() == NodeKind::Attribute
            && target
                .segments()
                .first()
                .is_some_and(|owner| self.applies_to(owner.kind()))
    }

    pub(crate) fn copyable_skeleton(self) -> OperationRef {
        OperationRef::new(self.name, self.skeleton.args())
    }

    fn copyable_skeleton_at(self, at: impl Into<String>) -> OperationRef {
        let mut args = self.skeleton.args();
        args.insert("at".to_string(), Value::String(at.into()));
        OperationRef::new(self.name, args)
    }
}

macro_rules! operation_descriptors {
    ($(($name:literal, $family:ident, $applicability:ident, $skeleton:ident)),+ $(,)?) => {
        const OPERATION_DESCRIPTORS: &[OperationDescriptor] = &[
            $(OperationDescriptor {
                name: $name,
                family: OperationFamily::$family,
                applicability: OperationApplicability::$applicability,
                skeleton: OperationSkeleton::$skeleton,
            }),+
        ];
    };
}

operation_descriptors!(
    ("object.create", Metadata, Metadata, Values),
    ("object.remove", Metadata, Metadata, Target),
    ("props.set", Properties, MetadataOrSubsystem, Values),
    ("relation.add", Metadata, Metadata, Values),
    ("relation.remove", Metadata, Metadata, Values),
    ("relation.replace", Metadata, Metadata, Values),
    ("help.create", Metadata, Metadata, Values),
    ("attribute.add", Metadata, MetadataAttributes, Items),
    ("attribute.set", Metadata, MetadataAttributes, Values),
    ("attribute.remove", Metadata, MetadataAttributes, Target),
    (
        "tabularSection.add",
        Metadata,
        MetadataTabularSections,
        Items
    ),
    (
        "tabularSection.set",
        Metadata,
        MetadataTabularSections,
        Values
    ),
    (
        "tabularSection.remove",
        Metadata,
        MetadataTabularSections,
        Values
    ),
    ("dimension.add", Metadata, MetadataDimensions, Items),
    ("dimension.set", Metadata, MetadataDimensions, Values),
    ("dimension.remove", Metadata, MetadataDimensions, Values),
    ("resource.add", Metadata, MetadataResources, Items),
    ("resource.set", Metadata, MetadataResources, Values),
    ("resource.remove", Metadata, MetadataResources, Values),
    ("enumValue.add", Metadata, MetadataEnumValues, Items),
    ("enumValue.set", Metadata, MetadataEnumValues, Values),
    ("enumValue.remove", Metadata, MetadataEnumValues, Values),
    ("column.add", Metadata, MetadataColumns, Items),
    ("column.set", Metadata, MetadataColumns, Values),
    ("column.remove", Metadata, MetadataColumns, Values),
    ("form.add", Form, MetadataForms, Items),
    ("form.set", Form, MetadataFormsOrForm, Values),
    ("form.remove", Form, MetadataFormsOrForm, Target),
    ("template.add", Metadata, MetadataTemplates, Items),
    ("template.set", Metadata, MetadataTemplates, Values),
    ("template.remove", Metadata, MetadataTemplates, Values),
    ("command.add", Metadata, MetadataCommands, Items),
    ("command.set", Metadata, MetadataCommands, Values),
    ("command.remove", Metadata, MetadataCommands, Values),
    (
        "predefinedItem.add",
        Metadata,
        MetadataPredefinedItems,
        Items
    ),
    (
        "predefinedItem.set",
        Metadata,
        MetadataPredefinedItems,
        Values
    ),
    (
        "predefinedItem.remove",
        Metadata,
        MetadataPredefinedItems,
        Values
    ),
    ("form.create", Form, Form, Values),
    ("element.add", Form, Form, Items),
    ("element.remove", Form, Form, Target),
    ("formAttribute.add", Form, Form, Items),
    ("formCommand.add", Form, Form, Items),
    ("event.bind", Form, Form, Values),
    ("role.create", Role, RoleOrRoot, Values),
    ("right.set", Role, Role, Values),
    ("dcs.set", Dcs, Dcs, Values),
    ("field.add", Dcs, Dcs, Items),
    ("field.set", Dcs, Dcs, Values),
    ("field.remove", Dcs, Dcs, Target),
    ("fieldRole.set", Dcs, Dcs, Values),
    ("total.add", Dcs, Dcs, Items),
    ("total.remove", Dcs, Dcs, Target),
    ("calculatedField.add", Dcs, Dcs, Items),
    ("calculatedField.remove", Dcs, Dcs, Target),
    ("parameter.add", Dcs, Dcs, Items),
    ("parameter.set", Dcs, Dcs, Values),
    ("parameter.rename", Dcs, Dcs, Values),
    ("parameter.reorder", Dcs, Dcs, Items),
    ("parameter.remove", Dcs, Dcs, Target),
    ("filter.add", Dcs, Dcs, Items),
    ("filter.set", Dcs, Dcs, Values),
    ("filter.remove", Dcs, Dcs, Target),
    ("filter.clear", Dcs, Dcs, Target),
    ("dataParameter.add", Dcs, Dcs, Items),
    ("dataParameter.set", Dcs, Dcs, Values),
    ("query.set", Dcs, Dcs, Values),
    ("query.patch", Dcs, Dcs, Values),
    ("selection.add", Dcs, Dcs, Items),
    ("selection.clear", Dcs, Dcs, Target),
    ("order.add", Dcs, Dcs, Items),
    ("order.clear", Dcs, Dcs, Target),
    ("conditionalAppearance.add", Dcs, Dcs, Items),
    ("conditionalAppearance.clear", Dcs, Dcs, Target),
    ("dataSetLink.add", Dcs, Dcs, Items),
    ("dataSet.add", Dcs, Dcs, Items),
    ("variant.add", Dcs, Dcs, Items),
    ("drilldown.add", Dcs, Dcs, Items),
    ("outputParameter.set", Dcs, Dcs, Values),
    ("structure.set", Dcs, Dcs, Values),
    ("structure.patch", Dcs, Dcs, Values),
    ("mxl.set", Mxl, Mxl, Values),
    ("valueType.add", Xdto, Xdto, Values),
    ("objectType.add", Xdto, Xdto, Values),
    ("property.add", Xdto, Xdto, Values),
    ("type.remove", Xdto, Xdto, Target),
    ("property.remove", Xdto, Xdto, Values),
    ("subsystem.create", Subsystem, Subsystem, Values),
    ("content.add", Subsystem, Subsystem, Items),
    ("content.remove", Subsystem, Subsystem, Target),
    ("childSubsystem.add", Subsystem, Subsystem, Items),
    ("childSubsystem.remove", Subsystem, Subsystem, Target),
    ("supportCapability.set", Support, Support, Values),
    ("supportRule.set", Support, Support, Values),
    ("code.insert", Code, Code, Text),
    ("code.replace", Code, Code, Text),
    ("event.implement", Event, Event, Target),
);

#[derive(Debug, Clone, Copy)]
pub(crate) struct OperationRegistry;

impl OperationRegistry {
    pub(crate) const fn closed() -> Self {
        Self
    }

    pub(crate) const fn descriptors(self) -> &'static [OperationDescriptor] {
        OPERATION_DESCRIPTORS
    }

    pub(crate) fn names(self) -> Vec<&'static str> {
        self.descriptors()
            .iter()
            .map(|descriptor| descriptor.name)
            .collect()
    }

    pub(crate) fn lookup(self, name: &str) -> Option<OperationDescriptor> {
        self.descriptors()
            .iter()
            .copied()
            .find(|descriptor| descriptor.name == name)
    }

    pub(crate) fn copyable_skeletons(self, kind: NodeKind) -> Vec<OperationRef> {
        self.descriptors()
            .iter()
            .copied()
            .filter(|descriptor| descriptor.applies_to(kind))
            .map(OperationDescriptor::copyable_skeleton)
            .collect()
    }

    pub(crate) fn event_implementation_skeleton(
        self,
        at: impl Into<String>,
        call_type: Option<&str>,
    ) -> OperationRef {
        let descriptor = self
            .lookup("event.implement")
            .expect("the closed operation registry owns event implementation");
        let mut args = descriptor.skeleton.args();
        args.insert("at".to_string(), Value::String(at.into()));
        if let Some(call_type) = call_type {
            args.insert("callType".to_string(), Value::String(call_type.to_string()));
        }
        OperationRef::new(descriptor.name, args)
    }
}

#[cfg(test)]
mod tests {
    use super::{ApplyRequest, OperationFamily, OperationRegistry};
    use crate::domain::address::NodeKind;
    use serde_json::{json, Value};

    fn parse(value: Value) -> Result<ApplyRequest, super::ApplyValidationError> {
        ApplyRequest::parse(
            value.as_object().expect("apply fixture must be an object"),
            &["main", "extension"],
        )
    }

    #[test]
    fn apply_request_retains_order_inherits_targets_and_keeps_items_as_data() {
        let request = parse(json!({
            "at": "main:Document.Order",
            "ops": [
                {"op": "props.set", "args": {"items": [{"at": "not-an-operation-target"}]}},
                {"op": "attribute.add", "args": {"at": "main:Document.Order.Attribute", "name": "Total"}},
                {"op": "code.replace", "args": {"at": "main:Document.Order.Module.Object.Body", "text": "Procedure X()\nEndProcedure"}}
            ],
            "ifRev": "unica-source-sha256-v1:7:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        }))
        .expect("valid ordered apply request");

        assert_eq!(request.at().to_string(), "main:Document.Order");
        assert_eq!(
            request
                .ops()
                .iter()
                .map(|operation| operation.name())
                .collect::<Vec<_>>(),
            ["props.set", "attribute.add", "code.replace"]
        );
        assert_eq!(request.ops()[0].at(), request.at());
        assert_eq!(
            request.ops()[1].at().to_string(),
            "main:Document.Order.Attribute"
        );
        assert_eq!(
            request.ops()[2].at().to_string(),
            "main:Document.Order.Module.Object.Body"
        );
        assert_eq!(
            request.ops()[0].args()["items"],
            json!([{"at": "not-an-operation-target"}]),
            "nested items[] are operation data, not sibling operations or targets"
        );
        assert!(!request.dry_run());
        assert_eq!(
            request.if_rev(),
            Some("unica-source-sha256-v1:7:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
    }

    #[test]
    fn apply_request_rejects_empty_ops_and_reports_the_exact_nested_target_location() {
        let empty = parse(json!({"at": "main:Document.Order", "ops": []})).unwrap_err();
        assert_eq!(empty.location(), "ops");
        assert_eq!(empty.code(), "bad_value");

        let cross_source = parse(json!({
            "at": "main:Document.Order",
            "ops": [
                {"op": "props.set"},
                {"op": "attribute.add", "args": {"at": "extension:Document.Order.Attribute"}}
            ]
        }))
        .unwrap_err();
        assert_eq!(cross_source.location(), "ops[1].args.at");

        let sibling = parse(json!({
            "at": "main:Document.Order",
            "ops": [
                {"op": "props.set"},
                {"op": "attribute.add", "args": {"at": "main:Catalog.Products.Attribute"}}
            ]
        }))
        .unwrap_err();
        assert_eq!(sibling.location(), "ops[1].args.at");
    }

    #[test]
    fn apply_request_preserves_explicit_dry_run_and_one_batch_revision() {
        let request = parse(json!({
            "at": "main:Document.Order",
            "ops": [{"op": "props.set", "args": {}}],
            "dryRun": true,
            "ifRev": "rev-42"
        }))
        .unwrap();
        assert!(request.dry_run());
        assert_eq!(request.if_rev(), Some("rev-42"));
    }

    #[test]
    fn operation_registry_is_exact_closed_unique_and_drives_skeletons_and_dispatch() {
        let expected = [
            "object.create",
            "object.remove",
            "props.set",
            "relation.add",
            "relation.remove",
            "relation.replace",
            "help.create",
            "attribute.add",
            "attribute.set",
            "attribute.remove",
            "tabularSection.add",
            "tabularSection.set",
            "tabularSection.remove",
            "dimension.add",
            "dimension.set",
            "dimension.remove",
            "resource.add",
            "resource.set",
            "resource.remove",
            "enumValue.add",
            "enumValue.set",
            "enumValue.remove",
            "column.add",
            "column.set",
            "column.remove",
            "form.add",
            "form.set",
            "form.remove",
            "template.add",
            "template.set",
            "template.remove",
            "command.add",
            "command.set",
            "command.remove",
            "predefinedItem.add",
            "predefinedItem.set",
            "predefinedItem.remove",
            "form.create",
            "element.add",
            "element.remove",
            "formAttribute.add",
            "formCommand.add",
            "event.bind",
            "role.create",
            "right.set",
            "dcs.set",
            "field.add",
            "field.set",
            "field.remove",
            "fieldRole.set",
            "total.add",
            "total.remove",
            "calculatedField.add",
            "calculatedField.remove",
            "parameter.add",
            "parameter.set",
            "parameter.rename",
            "parameter.reorder",
            "parameter.remove",
            "filter.add",
            "filter.set",
            "filter.remove",
            "filter.clear",
            "dataParameter.add",
            "dataParameter.set",
            "query.set",
            "query.patch",
            "selection.add",
            "selection.clear",
            "order.add",
            "order.clear",
            "conditionalAppearance.add",
            "conditionalAppearance.clear",
            "dataSetLink.add",
            "dataSet.add",
            "variant.add",
            "drilldown.add",
            "outputParameter.set",
            "structure.set",
            "structure.patch",
            "mxl.set",
            "valueType.add",
            "objectType.add",
            "property.add",
            "type.remove",
            "property.remove",
            "subsystem.create",
            "content.add",
            "content.remove",
            "childSubsystem.add",
            "childSubsystem.remove",
            "supportCapability.set",
            "supportRule.set",
            "code.insert",
            "code.replace",
            "event.implement",
        ];
        let registry = OperationRegistry::closed();
        assert_eq!(expected.len(), 96);
        assert_eq!(registry.names(), expected);
        let mut unique = registry.names().to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), expected.len());

        assert_eq!(
            registry.lookup("event.implement").unwrap().family(),
            OperationFamily::Event
        );
        assert!(registry.lookup("module.create").is_none());
        assert!(registry
            .lookup("props.set")
            .unwrap()
            .applies_to(NodeKind::Subsystem));
        assert!(registry
            .lookup("props.set")
            .unwrap()
            .applies_to(NodeKind::Document));

        let skeletons = registry.copyable_skeletons(NodeKind::Event);
        let event = skeletons
            .iter()
            .map(|item| serde_json::to_value(item).unwrap())
            .find(|value| value["op"] == "event.implement")
            .expect("Event.can must be projected from the dispatch registry");
        assert_eq!(event, json!({"op": "event.implement", "args": {"at": ""}}));
        for skeleton in registry
            .descriptors()
            .iter()
            .map(|descriptor| descriptor.copyable_skeleton())
        {
            let value = serde_json::to_value(skeleton).unwrap();
            assert!(
                value.to_string().len() <= 256,
                "skeleton must stay bounded: {value}"
            );
            let rendered = value.to_string();
            for forbidden in [
                "jsonPath",
                "definitionFile",
                "outputPath",
                "provider",
                "providerId",
            ] {
                assert!(
                    !rendered.contains(forbidden),
                    "physical/provider field leaked: {value}"
                );
            }
        }
    }

    #[test]
    fn metadata_collection_operations_follow_the_exact_typed_kind_matrix() {
        let registry = OperationRegistry::closed();
        let cases = [
            ("attribute.add", NodeKind::Catalog, true),
            ("attribute.add", NodeKind::InformationRegister, true),
            ("attribute.add", NodeKind::Enum, false),
            ("tabularSection.add", NodeKind::Document, true),
            ("tabularSection.add", NodeKind::InformationRegister, false),
            ("dimension.add", NodeKind::InformationRegister, true),
            ("dimension.add", NodeKind::Document, false),
            ("resource.remove", NodeKind::CalculationRegister, true),
            ("resource.remove", NodeKind::Catalog, false),
            ("enumValue.set", NodeKind::Enum, true),
            ("enumValue.set", NodeKind::Catalog, false),
            ("column.remove", NodeKind::DocumentJournal, true),
            ("column.remove", NodeKind::Document, false),
            ("predefinedItem.add", NodeKind::Catalog, true),
            ("predefinedItem.add", NodeKind::ChartOfAccounts, true),
            ("predefinedItem.add", NodeKind::Document, false),
            ("form.add", NodeKind::Constant, true),
            ("template.add", NodeKind::Constant, false),
            ("command.add", NodeKind::Constant, false),
            ("attribute.add", NodeKind::CommonModule, false),
        ];
        for (name, kind, expected) in cases {
            assert_eq!(
                registry.lookup(name).unwrap().applies_to(kind),
                expected,
                "{name} applicability for {kind:?}"
            );
        }
    }

    #[test]
    fn registry_skeletons_are_bounded_useful_and_copyable_operation_objects() {
        let registry = OperationRegistry::closed();
        let cases = [
            (
                "props.set",
                json!({"op": "props.set", "args": {"values": {}}}),
            ),
            (
                "attribute.add",
                json!({"op": "attribute.add", "args": {"items": []}}),
            ),
            (
                "code.replace",
                json!({"op": "code.replace", "args": {"text": ""}}),
            ),
            (
                "event.implement",
                json!({"op": "event.implement", "args": {"at": ""}}),
            ),
        ];
        for (name, expected) in cases {
            let descriptor = registry.lookup(name).unwrap();
            assert_eq!(
                serde_json::to_value(descriptor.copyable_skeleton()).unwrap(),
                expected
            );
            assert_eq!(descriptor.family(), registry.lookup(name).unwrap().family());
        }
        for descriptor in registry.descriptors() {
            let value = serde_json::to_value(descriptor.copyable_skeleton()).unwrap();
            assert!(value["args"]
                .as_object()
                .is_some_and(|args| !args.is_empty()));
            assert!(value.to_string().len() <= 256, "{value}");
        }
    }
}
