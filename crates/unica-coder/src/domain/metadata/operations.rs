use super::{
    MetaDiagnostic, MetaDiagnosticCode, MetaPropertyChanges, MetadataReference, MetadataType,
};
use crate::domain::source_target::MetadataAddress;
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MetaCollection {
    Attributes,
    TabularSections,
    Dimensions,
    Resources,
    EnumValues,
    Columns,
    Forms,
    Templates,
    Commands,
}

impl MetaCollection {
    pub(crate) const ALL: &'static [Self] = &[
        Self::Attributes,
        Self::TabularSections,
        Self::Dimensions,
        Self::Resources,
        Self::EnumValues,
        Self::Columns,
        Self::Forms,
        Self::Templates,
        Self::Commands,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Attributes => "attributes",
            Self::TabularSections => "tabularSections",
            Self::Dimensions => "dimensions",
            Self::Resources => "resources",
            Self::EnumValues => "enumValues",
            Self::Columns => "columns",
            Self::Forms => "forms",
            Self::Templates => "templates",
            Self::Commands => "commands",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, MetaDiagnostic> {
        Self::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.as_str() == value)
            .ok_or_else(|| {
                invalid_operation(
                    "collection",
                    format!("unsupported metadata collection `{value}`"),
                )
            })
    }
}

/// Closed collection capability matrix for the 23 metadata kinds exposed by
/// the typed metadata surface. Platform XML validation and mutation both use
/// this registry so a child cannot be accepted by one boundary and rejected by
/// the other.
pub(crate) fn metadata_kind_collections(kind: super::MetadataKind) -> &'static [MetaCollection] {
    use super::MetadataKind;
    use MetaCollection::*;

    match kind {
        MetadataKind::Catalog
        | MetadataKind::Document
        | MetadataKind::ChartOfAccounts
        | MetadataKind::ChartOfCharacteristicTypes
        | MetadataKind::ChartOfCalculationTypes
        | MetadataKind::BusinessProcess
        | MetadataKind::Task
        | MetadataKind::ExchangePlan
        | MetadataKind::Report
        | MetadataKind::DataProcessor => &[Attributes, TabularSections, Forms, Templates, Commands],
        MetadataKind::Enum => &[EnumValues, Forms, Templates, Commands],
        MetadataKind::Constant => &[Forms],
        MetadataKind::InformationRegister
        | MetadataKind::AccumulationRegister
        | MetadataKind::AccountingRegister
        | MetadataKind::CalculationRegister => &[
            Attributes, Dimensions, Resources, Forms, Templates, Commands,
        ],
        MetadataKind::DocumentJournal => &[Columns, Forms, Templates, Commands],
        MetadataKind::CommonModule
        | MetadataKind::ScheduledJob
        | MetadataKind::EventSubscription
        | MetadataKind::HTTPService
        | MetadataKind::WebService
        | MetadataKind::DefinedType => &[],
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetaScope {
    pub(crate) tabular_section: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetaPosition {
    pub(crate) before: Option<String>,
    pub(crate) after: Option<String>,
}

impl MetaPosition {
    pub(crate) fn new(
        before: Option<String>,
        after: Option<String>,
    ) -> Result<Self, MetaDiagnostic> {
        Self::new_at(before, after, "position")
    }

    pub(crate) fn new_at(
        before: Option<String>,
        after: Option<String>,
        field: &str,
    ) -> Result<Self, MetaDiagnostic> {
        if before.is_some() == after.is_some() {
            return Err(invalid_operation(
                field,
                "position requires exactly one of before or after",
            ));
        }
        if before.as_deref() == Some("") || after.as_deref() == Some("") {
            return Err(invalid_operation(
                field,
                "position anchor must not be empty",
            ));
        }
        Ok(Self { before, after })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MetaFillValue {
    String(String),
    Number(String),
    Boolean(bool),
    DateTime(String),
    Reference(MetadataReference),
}

impl Serialize for MetaFillValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::String(value) => {
                let mut state = serializer.serialize_struct("MetaFillValue", 2)?;
                state.serialize_field("kind", "string")?;
                state.serialize_field("value", value)?;
                state.end()
            }
            Self::Number(value) => {
                let mut state = serializer.serialize_struct("MetaFillValue", 2)?;
                state.serialize_field("kind", "number")?;
                state.serialize_field("value", value)?;
                state.end()
            }
            Self::Boolean(value) => {
                let mut state = serializer.serialize_struct("MetaFillValue", 2)?;
                state.serialize_field("kind", "boolean")?;
                state.serialize_field("value", value)?;
                state.end()
            }
            Self::DateTime(value) => {
                let mut state = serializer.serialize_struct("MetaFillValue", 2)?;
                state.serialize_field("kind", "dateTime")?;
                state.serialize_field("value", value)?;
                state.end()
            }
            Self::Reference(reference) => {
                let mut state = serializer.serialize_struct("MetaFillValue", 2)?;
                state.serialize_field("kind", "reference")?;
                state.serialize_field("metadataPath", &reference.metadata_path)?;
                state.end()
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct MetaElementInput {
    pub(crate) name: String,
    pub(crate) synonym: Option<String>,
    pub(crate) comment: Option<String>,
    pub(crate) r#type: Option<MetadataType>,
    pub(crate) required: Option<bool>,
    pub(crate) fill_value: Option<MetaFillValue>,
    pub(crate) attributes: Option<Vec<MetaElementInput>>,
    pub(crate) position: Option<MetaPosition>,
}

impl MetaElementInput {
    pub(crate) fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetaElementDefinition {
    pub(crate) name: String,
    pub(crate) synonym: Option<String>,
    pub(crate) comment: Option<String>,
    pub(crate) r#type: Option<MetadataType>,
    pub(crate) required: Option<bool>,
    pub(crate) fill_value: Option<MetaFillValue>,
    pub(crate) attributes: Vec<MetaElementDefinition>,
    pub(crate) position: Option<MetaPosition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct MetaElementUpdateInput {
    pub(crate) name: String,
    pub(crate) new_name: Option<String>,
    pub(crate) synonym: Option<String>,
    pub(crate) comment: Option<String>,
    pub(crate) r#type: Option<MetadataType>,
    pub(crate) required: Option<bool>,
    pub(crate) fill_value: Option<MetaFillValue>,
    pub(crate) position: Option<MetaPosition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetaElementUpdate {
    pub(crate) name: String,
    pub(crate) new_name: Option<String>,
    pub(crate) synonym: Option<String>,
    pub(crate) comment: Option<String>,
    pub(crate) r#type: Option<MetadataType>,
    pub(crate) required: Option<bool>,
    pub(crate) fill_value: Option<MetaFillValue>,
    pub(crate) position: Option<MetaPosition>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct MetaCollectionSpec {
    pub(crate) collection: MetaCollection,
    pub(crate) allows_type: bool,
    pub(crate) allows_required: bool,
    pub(crate) allows_fill_value: bool,
    pub(crate) allows_nested_attributes: bool,
    pub(crate) allows_position: bool,
}

const COLLECTION_SPECS: &[MetaCollectionSpec] = &[
    collection_spec(MetaCollection::Attributes, true, true, true, false, true),
    collection_spec(
        MetaCollection::TabularSections,
        false,
        false,
        false,
        true,
        true,
    ),
    collection_spec(MetaCollection::Dimensions, true, true, true, false, true),
    collection_spec(MetaCollection::Resources, true, true, true, false, true),
    collection_spec(MetaCollection::EnumValues, false, false, false, false, true),
    collection_spec(MetaCollection::Columns, true, false, false, false, true),
    collection_spec(MetaCollection::Forms, false, false, false, false, true),
    collection_spec(MetaCollection::Templates, false, false, false, false, true),
    collection_spec(MetaCollection::Commands, false, false, false, false, true),
];

const fn collection_spec(
    collection: MetaCollection,
    allows_type: bool,
    allows_required: bool,
    allows_fill_value: bool,
    allows_nested_attributes: bool,
    allows_position: bool,
) -> MetaCollectionSpec {
    MetaCollectionSpec {
        collection,
        allows_type,
        allows_required,
        allows_fill_value,
        allows_nested_attributes,
        allows_position,
    }
}

pub(crate) fn metadata_collection_spec(collection: MetaCollection) -> &'static MetaCollectionSpec {
    COLLECTION_SPECS
        .iter()
        .find(|spec| spec.collection == collection)
        .expect("closed collection registry must be exhaustive")
}

pub(crate) fn validate_metadata_kind_collection(
    kind: super::MetadataKind,
    collection: MetaCollection,
) -> Result<(), MetaDiagnostic> {
    if metadata_kind_collections(kind).contains(&collection) {
        Ok(())
    } else {
        Err(MetaDiagnostic::error(
            MetaDiagnosticCode::UnsupportedKind,
            format!(
                "collection `{}` is not supported for {}",
                collection.as_str(),
                kind.as_str()
            ),
        )
        .with_field("collection"))
    }
}

pub(crate) fn validate_collection_scope(
    collection: MetaCollection,
    scope: &Option<MetaScope>,
) -> Result<(), MetaDiagnostic> {
    if let Some(scope) = scope {
        if collection != MetaCollection::Attributes {
            return Err(invalid_operation(
                "scope",
                "scope.tabularSection is allowed only for attributes",
            ));
        }
        if scope.tabular_section.is_empty() {
            return Err(invalid_operation(
                "scope.tabularSection",
                "tabular section name must not be empty",
            ));
        }
    }
    Ok(())
}

impl MetaElementDefinition {
    pub(crate) fn convert(
        collection: MetaCollection,
        input: MetaElementInput,
    ) -> Result<Self, MetaDiagnostic> {
        Self::convert_at(collection, input, "elements")
    }

    fn convert_at(
        collection: MetaCollection,
        input: MetaElementInput,
        field: &str,
    ) -> Result<Self, MetaDiagnostic> {
        validate_name(&input.name, &format!("{field}.name"))?;
        let spec = metadata_collection_spec(collection);
        validate_element_fields(
            spec,
            field,
            input.r#type.is_some(),
            input.required.is_some(),
            input.fill_value.is_some(),
            input.attributes.is_some(),
            input.position.is_some(),
        )?;
        let attributes_field = format!("{field}.attributes");
        let attributes = input
            .attributes
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(index, nested)| {
                Self::convert_at(
                    MetaCollection::Attributes,
                    nested,
                    &format!("{attributes_field}[{index}]"),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        reject_duplicate_names(
            attributes.iter().map(|attribute| attribute.name.as_str()),
            &attributes_field,
        )?;
        Ok(Self {
            name: input.name,
            synonym: input.synonym,
            comment: input.comment,
            r#type: input.r#type,
            required: input.required,
            fill_value: input.fill_value,
            attributes,
            position: input.position,
        })
    }
}

impl MetaElementUpdate {
    fn convert(
        collection: MetaCollection,
        input: MetaElementUpdateInput,
    ) -> Result<Self, MetaDiagnostic> {
        Self::convert_at(collection, input, "elements")
    }

    fn convert_at(
        collection: MetaCollection,
        input: MetaElementUpdateInput,
        field: &str,
    ) -> Result<Self, MetaDiagnostic> {
        validate_name(&input.name, &format!("{field}.name"))?;
        if let Some(new_name) = &input.new_name {
            validate_name(new_name, &format!("{field}.newName"))?;
        }
        let renames = input
            .new_name
            .as_ref()
            .is_some_and(|new_name| new_name != &input.name);
        if !renames
            && input.synonym.is_none()
            && input.comment.is_none()
            && input.r#type.is_none()
            && input.required.is_none()
            && input.fill_value.is_none()
            && input.position.is_none()
        {
            return Err(invalid_operation(
                field,
                "update must change at least one field",
            ));
        }
        let spec = metadata_collection_spec(collection);
        validate_element_fields(
            spec,
            field,
            input.r#type.is_some(),
            input.required.is_some(),
            input.fill_value.is_some(),
            false,
            input.position.is_some(),
        )?;
        Ok(Self {
            name: input.name,
            new_name: input.new_name,
            synonym: input.synonym,
            comment: input.comment,
            r#type: input.r#type,
            required: input.required,
            fill_value: input.fill_value,
            position: input.position,
        })
    }
}

fn validate_element_fields(
    spec: &MetaCollectionSpec,
    field: &str,
    has_type: bool,
    has_required: bool,
    has_fill_value: bool,
    has_nested_attributes: bool,
    has_position: bool,
) -> Result<(), MetaDiagnostic> {
    for (present, allowed, suffix) in [
        (has_type, spec.allows_type, "type"),
        (has_required, spec.allows_required, "required"),
        (has_fill_value, spec.allows_fill_value, "fillValue"),
        (
            has_nested_attributes,
            spec.allows_nested_attributes,
            "attributes",
        ),
        (has_position, spec.allows_position, "position"),
    ] {
        if present && !allowed {
            return Err(invalid_operation(
                format!("{field}.{suffix}"),
                "field is not legal for collection",
            ));
        }
    }
    Ok(())
}

fn validate_name(name: &str, field: &str) -> Result<(), MetaDiagnostic> {
    if name.is_empty() {
        Err(invalid_operation(field, "name must not be empty"))
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetaRelation {
    Owners,
    RegisterRecords,
    BasedOn,
    InputByString,
}

impl MetaRelation {
    pub(crate) const ALL: &'static [Self] = &[
        Self::Owners,
        Self::RegisterRecords,
        Self::BasedOn,
        Self::InputByString,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Owners => "owners",
            Self::RegisterRecords => "registerRecords",
            Self::BasedOn => "basedOn",
            Self::InputByString => "inputByString",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, MetaDiagnostic> {
        Self::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.as_str() == value)
            .ok_or_else(|| {
                invalid_operation(
                    "relation",
                    format!("unsupported metadata relation `{value}`"),
                )
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RelationEditMode {
    Add,
    Remove,
    Replace,
}

impl RelationEditMode {
    pub(crate) const ALL: &'static [Self] = &[Self::Add, Self::Remove, Self::Replace];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Remove => "remove",
            Self::Replace => "replace",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, MetaDiagnostic> {
        Self::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.as_str() == value)
            .ok_or_else(|| {
                invalid_operation("mode", format!("unsupported relation mode `{value}`"))
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetaEditOperationTag {
    SetProperties,
    Add,
    Update,
    Remove,
    EditRelations,
}

impl MetaEditOperationTag {
    pub(crate) const ALL: &'static [Self] = &[
        Self::SetProperties,
        Self::Add,
        Self::Update,
        Self::Remove,
        Self::EditRelations,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::SetProperties => "setProperties",
            Self::Add => "add",
            Self::Update => "update",
            Self::Remove => "remove",
            Self::EditRelations => "editRelations",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, MetaDiagnostic> {
        Self::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.as_str() == value)
            .ok_or_else(|| {
                invalid_operation(
                    "op",
                    format!("unsupported metadata edit operation `{value}`"),
                )
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MetaEditOperation {
    SetProperties {
        values: MetaPropertyChanges,
    },
    Add {
        collection: MetaCollection,
        scope: Option<MetaScope>,
        elements: Vec<MetaElementDefinition>,
    },
    Update {
        collection: MetaCollection,
        scope: Option<MetaScope>,
        elements: Vec<MetaElementUpdate>,
    },
    Remove {
        collection: MetaCollection,
        scope: Option<MetaScope>,
        names: Vec<String>,
    },
    EditRelations {
        relation: MetaRelation,
        mode: RelationEditMode,
        targets: Vec<MetaRelationTarget>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MetaRelationTarget {
    Object(MetadataReference),
    Field(MetadataFieldPath),
}

impl MetaRelationTarget {
    pub(crate) fn wire_value(&self) -> &str {
        match self {
            Self::Object(reference) => reference.metadata_path.as_str(),
            Self::Field(path) => &path.value,
        }
    }

    pub(crate) fn dependency(&self) -> &MetadataAddress {
        match self {
            Self::Object(reference) => &reference.metadata_path,
            Self::Field(path) => &path.owner,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetadataFieldKind {
    Attribute,
    StandardAttribute,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetadataFieldPath {
    pub(crate) owner: MetadataAddress,
    pub(crate) kind: MetadataFieldKind,
    pub(crate) name: String,
    value: String,
}

impl MetadataFieldPath {
    pub(crate) fn parse(value: &str) -> Result<Self, MetaDiagnostic> {
        let parts = value.split('.').collect::<Vec<_>>();
        if parts.len() != 4 {
            return Err(invalid_operation(
                "fieldPath",
                "field path must be Kind.Name.Attribute.Name or Kind.Name.StandardAttribute.Name",
            ));
        }
        validate_name(parts[3], "fieldPath")?;
        let owner = MetadataAddress::parse(
            crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            &format!("{}.{}", parts[0], parts[1]),
        )
        .map_err(|_| invalid_operation("fieldPath", "field path owner is invalid"))?;
        let kind = match parts[2] {
            "Attribute" => MetadataFieldKind::Attribute,
            "StandardAttribute" => MetadataFieldKind::StandardAttribute,
            _ => {
                return Err(invalid_operation(
                    "fieldPath",
                    "field path kind must be Attribute or StandardAttribute",
                ))
            }
        };
        Ok(Self {
            owner,
            kind,
            name: parts[3].to_string(),
            value: value.to_string(),
        })
    }
}

impl MetaEditOperation {
    pub(crate) fn add(
        collection: MetaCollection,
        scope: Option<MetaScope>,
        inputs: Vec<MetaElementInput>,
    ) -> Result<Self, MetaDiagnostic> {
        validate_collection_scope(collection, &scope)?;
        if inputs.is_empty() {
            return Err(invalid_operation(
                "elements",
                "add elements must not be empty",
            ));
        }
        let elements = inputs
            .into_iter()
            .enumerate()
            .map(|(index, input)| {
                MetaElementDefinition::convert_at(collection, input, &format!("elements[{index}]"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        reject_duplicate_names(
            elements.iter().map(|element| element.name.as_str()),
            "elements",
        )?;
        Ok(Self::Add {
            collection,
            scope,
            elements,
        })
    }

    pub(crate) fn update(
        collection: MetaCollection,
        scope: Option<MetaScope>,
        inputs: Vec<MetaElementUpdateInput>,
    ) -> Result<Self, MetaDiagnostic> {
        validate_collection_scope(collection, &scope)?;
        if inputs.is_empty() {
            return Err(invalid_operation(
                "elements",
                "update elements must not be empty",
            ));
        }
        let elements = inputs
            .into_iter()
            .enumerate()
            .map(|(index, input)| {
                MetaElementUpdate::convert_at(collection, input, &format!("elements[{index}]"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        reject_duplicate_names(
            elements.iter().map(|element| element.name.as_str()),
            "elements",
        )?;
        Ok(Self::Update {
            collection,
            scope,
            elements,
        })
    }

    pub(crate) fn remove(
        collection: MetaCollection,
        scope: Option<MetaScope>,
        names: Vec<String>,
    ) -> Result<Self, MetaDiagnostic> {
        validate_collection_scope(collection, &scope)?;
        if names.is_empty() {
            return Err(invalid_operation("names", "remove names must not be empty"));
        }
        for (index, name) in names.iter().enumerate() {
            validate_name(name, &format!("names[{index}]"))?;
        }
        reject_duplicate_values(names.iter().map(String::as_str), "names")?;
        Ok(Self::Remove {
            collection,
            scope,
            names,
        })
    }

    pub(crate) fn edit_relations(
        relation: MetaRelation,
        mode: RelationEditMode,
        targets: Vec<MetadataReference>,
    ) -> Result<Self, MetaDiagnostic> {
        Self::edit_relation_targets(
            relation,
            mode,
            targets
                .into_iter()
                .map(MetaRelationTarget::Object)
                .collect(),
        )
    }

    pub(crate) fn edit_relation_targets(
        relation: MetaRelation,
        mode: RelationEditMode,
        targets: Vec<MetaRelationTarget>,
    ) -> Result<Self, MetaDiagnostic> {
        if targets.is_empty() {
            return Err(invalid_operation(
                "targets",
                "relation targets must not be empty",
            ));
        }
        Ok(Self::EditRelations {
            relation,
            mode,
            targets,
        })
    }

    pub(crate) fn validate_targets(
        &self,
        existing_names: &HashSet<String>,
    ) -> Result<(), MetaDiagnostic> {
        match self {
            Self::Add { elements, .. } => {
                for (index, element) in elements.iter().enumerate() {
                    if existing_names.contains(&element.name) {
                        return Err(MetaDiagnostic::error(
                            MetaDiagnosticCode::AlreadyExists,
                            format!("element `{}` already exists", element.name),
                        )
                        .with_field(format!("elements[{index}].name")));
                    }
                }
            }
            Self::Update { elements, .. } => {
                for (index, element) in elements.iter().enumerate() {
                    if !existing_names.contains(&element.name) {
                        return Err(missing_target(
                            &element.name,
                            format!("elements[{index}].name"),
                        ));
                    }
                }

                let mut final_names = existing_names.clone();
                for element in elements {
                    final_names.remove(&element.name);
                }
                for (index, element) in elements.iter().enumerate() {
                    let final_name = element.new_name.as_ref().unwrap_or(&element.name);
                    if !final_names.insert(final_name.clone()) {
                        return Err(MetaDiagnostic::error(
                            MetaDiagnosticCode::AlreadyExists,
                            format!("element `{final_name}` already exists after update"),
                        )
                        .with_field(format!("elements[{index}].newName")));
                    }
                }
            }
            Self::Remove { names, .. } => {
                for (index, name) in names.iter().enumerate() {
                    if !existing_names.contains(name) {
                        return Err(missing_target(name, format!("names[{index}]")));
                    }
                }
            }
            Self::SetProperties { .. } | Self::EditRelations { .. } => {}
        }
        Ok(())
    }
}

fn reject_duplicate_names<'a>(
    names: impl Iterator<Item = &'a str>,
    field: &str,
) -> Result<(), MetaDiagnostic> {
    let mut seen = HashSet::new();
    for (index, name) in names.enumerate() {
        if !seen.insert(name) {
            return Err(invalid_operation(
                format!("{field}[{index}].name"),
                "element name is duplicated",
            ));
        }
    }
    Ok(())
}

fn reject_duplicate_values<'a>(
    values: impl Iterator<Item = &'a str>,
    field: &str,
) -> Result<(), MetaDiagnostic> {
    let mut seen = HashSet::new();
    for (index, value) in values.enumerate() {
        if !seen.insert(value) {
            return Err(invalid_operation(
                format!("{field}[{index}]"),
                "value is duplicated",
            ));
        }
    }
    Ok(())
}

fn missing_target(name: &str, field: impl Into<String>) -> MetaDiagnostic {
    MetaDiagnostic::error(
        MetaDiagnosticCode::TargetNotFound,
        format!("element `{name}` was not found"),
    )
    .with_field(field)
}

fn invalid_operation(field: impl Into<String>, message: impl Into<String>) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::InvalidArguments, message).with_field(field)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metadata::MetaDiagnosticCode;
    use std::collections::HashSet;

    #[test]
    fn public_operation_vocabulary_is_owned_by_closed_domain_registries() {
        assert_eq!(
            MetaCollection::ALL
                .iter()
                .copied()
                .map(MetaCollection::as_str)
                .collect::<Vec<_>>(),
            [
                "attributes",
                "tabularSections",
                "dimensions",
                "resources",
                "enumValues",
                "columns",
                "forms",
                "templates",
                "commands",
            ]
        );
        assert_eq!(
            MetaRelation::ALL
                .iter()
                .copied()
                .map(MetaRelation::as_str)
                .collect::<Vec<_>>(),
            ["owners", "registerRecords", "basedOn", "inputByString"]
        );
        assert_eq!(
            RelationEditMode::ALL
                .iter()
                .copied()
                .map(RelationEditMode::as_str)
                .collect::<Vec<_>>(),
            ["add", "remove", "replace"]
        );
        assert_eq!(
            MetaEditOperationTag::ALL
                .iter()
                .copied()
                .map(MetaEditOperationTag::as_str)
                .collect::<Vec<_>>(),
            ["setProperties", "add", "update", "remove", "editRelations"]
        );

        assert_eq!(
            MetaCollection::parse("attributes"),
            Ok(MetaCollection::Attributes)
        );
        assert_eq!(MetaRelation::parse("owners"), Ok(MetaRelation::Owners));
        assert_eq!(
            RelationEditMode::parse("replace"),
            Ok(RelationEditMode::Replace)
        );
        assert_eq!(
            MetaEditOperationTag::parse("setProperties"),
            Ok(MetaEditOperationTag::SetProperties)
        );
        for diagnostic in [
            MetaCollection::parse("attribute").unwrap_err(),
            MetaRelation::parse("owner").unwrap_err(),
            RelationEditMode::parse("set").unwrap_err(),
            MetaEditOperationTag::parse("patch").unwrap_err(),
        ] {
            assert_eq!(diagnostic.code, MetaDiagnosticCode::InvalidArguments);
        }
    }

    #[test]
    fn position_requires_exactly_one_anchor() {
        assert!(MetaPosition::new(None, None).is_err());
        assert!(MetaPosition::new(Some("A".into()), Some("B".into())).is_err());
        assert!(MetaPosition::new(Some("A".into()), None).is_ok());
        assert!(MetaPosition::new(None, Some("B".into())).is_ok());
    }

    #[test]
    fn only_attributes_allow_a_tabular_section_scope() {
        let scope = Some(MetaScope {
            tabular_section: "Lines".into(),
        });
        assert!(validate_collection_scope(MetaCollection::Attributes, &scope).is_ok());
        assert!(validate_collection_scope(MetaCollection::Attributes, &None).is_ok());
        for collection in [
            MetaCollection::TabularSections,
            MetaCollection::Dimensions,
            MetaCollection::Resources,
            MetaCollection::EnumValues,
            MetaCollection::Columns,
            MetaCollection::Forms,
            MetaCollection::Templates,
            MetaCollection::Commands,
        ] {
            assert!(validate_collection_scope(collection, &scope).is_err());
        }
    }

    #[test]
    fn nested_attributes_are_legal_only_on_new_tabular_sections() {
        let nested = vec![MetaElementInput::named("Quantity")];
        assert!(MetaElementDefinition::convert(
            MetaCollection::TabularSections,
            MetaElementInput {
                name: "Lines".into(),
                attributes: Some(nested.clone()),
                ..MetaElementInput::default()
            },
        )
        .is_ok());
        assert!(MetaElementDefinition::convert(
            MetaCollection::Attributes,
            MetaElementInput {
                name: "Item".into(),
                attributes: Some(nested),
                ..MetaElementInput::default()
            },
        )
        .is_err());
    }

    #[test]
    fn new_tabular_section_rejects_duplicate_nested_attribute_names() {
        let result = MetaEditOperation::add(
            MetaCollection::TabularSections,
            None,
            vec![MetaElementInput {
                name: "Lines".into(),
                attributes: Some(vec![
                    MetaElementInput::named("Quantity"),
                    MetaElementInput::named("Quantity"),
                ]),
                ..MetaElementInput::default()
            }],
        );

        assert_eq!(
            result.unwrap_err().code,
            MetaDiagnosticCode::InvalidArguments
        );
    }

    #[test]
    fn add_rejects_duplicates_while_update_and_remove_reject_missing_targets() {
        let existing = HashSet::from(["Existing".to_string()]);
        let add = MetaEditOperation::add(
            MetaCollection::Attributes,
            None,
            vec![MetaElementInput::named("Existing")],
        )
        .unwrap();
        assert_eq!(
            add.validate_targets(&existing).unwrap_err().code,
            MetaDiagnosticCode::AlreadyExists
        );

        let update = MetaEditOperation::update(
            MetaCollection::Attributes,
            None,
            vec![MetaElementUpdateInput {
                name: "Missing".into(),
                new_name: Some("Renamed".into()),
                ..MetaElementUpdateInput::default()
            }],
        )
        .unwrap();
        assert_eq!(
            update.validate_targets(&existing).unwrap_err().code,
            MetaDiagnosticCode::TargetNotFound
        );

        let remove =
            MetaEditOperation::remove(MetaCollection::Attributes, None, vec!["Missing".into()])
                .unwrap();
        assert_eq!(
            remove.validate_targets(&existing).unwrap_err().code,
            MetaDiagnosticCode::TargetNotFound
        );
    }

    #[test]
    fn target_validation_reports_exact_array_item_paths() {
        let existing = HashSet::from(["A".to_string(), "B".to_string(), "Occupied".to_string()]);

        let add = MetaEditOperation::add(
            MetaCollection::Attributes,
            None,
            vec![
                MetaElementInput::named("Fresh"),
                MetaElementInput::named("A"),
            ],
        )
        .unwrap();
        let error = add.validate_targets(&existing).unwrap_err();
        assert_eq!(error.code, MetaDiagnosticCode::AlreadyExists);
        assert_eq!(error.field.as_deref(), Some("elements[1].name"));

        let update = MetaEditOperation::update(
            MetaCollection::Attributes,
            None,
            vec![
                MetaElementUpdateInput {
                    name: "A".into(),
                    comment: Some("changed".into()),
                    ..MetaElementUpdateInput::default()
                },
                MetaElementUpdateInput {
                    name: "Missing".into(),
                    comment: Some("changed".into()),
                    ..MetaElementUpdateInput::default()
                },
            ],
        )
        .unwrap();
        let error = update.validate_targets(&existing).unwrap_err();
        assert_eq!(error.code, MetaDiagnosticCode::TargetNotFound);
        assert_eq!(error.field.as_deref(), Some("elements[1].name"));

        let rename = MetaEditOperation::update(
            MetaCollection::Attributes,
            None,
            vec![
                MetaElementUpdateInput {
                    name: "A".into(),
                    new_name: Some("Fresh".into()),
                    ..MetaElementUpdateInput::default()
                },
                MetaElementUpdateInput {
                    name: "B".into(),
                    new_name: Some("Occupied".into()),
                    ..MetaElementUpdateInput::default()
                },
            ],
        )
        .unwrap();
        let error = rename.validate_targets(&existing).unwrap_err();
        assert_eq!(error.code, MetaDiagnosticCode::AlreadyExists);
        assert_eq!(error.field.as_deref(), Some("elements[1].newName"));

        let remove = MetaEditOperation::remove(
            MetaCollection::Attributes,
            None,
            vec!["A".into(), "Missing".into()],
        )
        .unwrap();
        let error = remove.validate_targets(&existing).unwrap_err();
        assert_eq!(error.code, MetaDiagnosticCode::TargetNotFound);
        assert_eq!(error.field.as_deref(), Some("names[1]"));
    }

    #[test]
    fn update_and_remove_reject_empty_changes() {
        assert!(MetaEditOperation::update(
            MetaCollection::Attributes,
            None,
            vec![MetaElementUpdateInput {
                name: "Existing".into(),
                ..MetaElementUpdateInput::default()
            }],
        )
        .is_err());
        assert!(MetaEditOperation::update(
            MetaCollection::Attributes,
            None,
            vec![MetaElementUpdateInput {
                name: "Existing".into(),
                new_name: Some("Existing".into()),
                ..MetaElementUpdateInput::default()
            }],
        )
        .is_err());
        assert!(MetaEditOperation::remove(MetaCollection::Attributes, None, vec![]).is_err());
    }

    #[test]
    fn update_rejects_duplicate_final_names_after_all_renames() {
        let existing = HashSet::from(["A".to_string(), "B".to_string()]);
        let update = MetaEditOperation::update(
            MetaCollection::Attributes,
            None,
            vec![
                MetaElementUpdateInput {
                    name: "A".into(),
                    new_name: Some("X".into()),
                    ..MetaElementUpdateInput::default()
                },
                MetaElementUpdateInput {
                    name: "B".into(),
                    new_name: Some("X".into()),
                    ..MetaElementUpdateInput::default()
                },
            ],
        )
        .unwrap();

        assert_eq!(
            update.validate_targets(&existing).unwrap_err().code,
            MetaDiagnosticCode::AlreadyExists
        );
    }

    #[test]
    fn update_allows_a_destination_vacated_by_another_rename() {
        let existing = HashSet::from(["A".to_string(), "B".to_string()]);
        let update = MetaEditOperation::update(
            MetaCollection::Attributes,
            None,
            vec![
                MetaElementUpdateInput {
                    name: "A".into(),
                    new_name: Some("B".into()),
                    ..MetaElementUpdateInput::default()
                },
                MetaElementUpdateInput {
                    name: "B".into(),
                    new_name: Some("C".into()),
                    ..MetaElementUpdateInput::default()
                },
            ],
        )
        .unwrap();

        assert!(update.validate_targets(&existing).is_ok());
    }
}
