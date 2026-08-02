use super::{
    MetaDiagnostic, MetaDiagnosticCode, MetaPropertyChanges, MetadataReference, MetadataType,
};
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
        if before.is_some() == after.is_some() {
            return Err(invalid_operation(
                "position",
                "position requires exactly one of before or after",
            ));
        }
        if before.as_deref() == Some("") || after.as_deref() == Some("") {
            return Err(invalid_operation(
                "position",
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
struct MetaCollectionSpec {
    collection: MetaCollection,
    allows_type: bool,
    allows_required: bool,
    allows_fill_value: bool,
    allows_nested_attributes: bool,
    allows_position: bool,
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

fn collection_spec_for(collection: MetaCollection) -> &'static MetaCollectionSpec {
    COLLECTION_SPECS
        .iter()
        .find(|spec| spec.collection == collection)
        .expect("closed collection registry must be exhaustive")
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
        validate_name(&input.name, "elements.name")?;
        let spec = collection_spec_for(collection);
        validate_element_fields(
            spec,
            input.r#type.is_some(),
            input.required.is_some(),
            input.fill_value.is_some(),
            input.attributes.is_some(),
            input.position.is_some(),
        )?;
        let attributes = input
            .attributes
            .unwrap_or_default()
            .into_iter()
            .map(|nested| Self::convert(MetaCollection::Attributes, nested))
            .collect::<Result<Vec<_>, _>>()?;
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
        validate_name(&input.name, "elements.name")?;
        if let Some(new_name) = &input.new_name {
            validate_name(new_name, "elements.newName")?;
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
                "elements",
                "update must change at least one field",
            ));
        }
        let spec = collection_spec_for(collection);
        validate_element_fields(
            spec,
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
    has_type: bool,
    has_required: bool,
    has_fill_value: bool,
    has_nested_attributes: bool,
    has_position: bool,
) -> Result<(), MetaDiagnostic> {
    for (present, allowed, field) in [
        (has_type, spec.allows_type, "elements.type"),
        (has_required, spec.allows_required, "elements.required"),
        (has_fill_value, spec.allows_fill_value, "elements.fillValue"),
        (
            has_nested_attributes,
            spec.allows_nested_attributes,
            "elements.attributes",
        ),
        (has_position, spec.allows_position, "elements.position"),
    ] {
        if present && !allowed {
            return Err(invalid_operation(
                field,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RelationEditMode {
    Add,
    Remove,
    Replace,
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
        targets: Vec<MetadataReference>,
    },
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
            .map(|input| MetaElementDefinition::convert(collection, input))
            .collect::<Result<Vec<_>, _>>()?;
        reject_duplicate_names(elements.iter().map(|element| element.name.as_str()))?;
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
            .map(|input| MetaElementUpdate::convert(collection, input))
            .collect::<Result<Vec<_>, _>>()?;
        reject_duplicate_names(elements.iter().map(|element| element.name.as_str()))?;
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
        for name in &names {
            validate_name(name, "names")?;
        }
        reject_duplicate_names(names.iter().map(String::as_str))?;
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
                for element in elements {
                    if existing_names.contains(&element.name) {
                        return Err(MetaDiagnostic::error(
                            MetaDiagnosticCode::AlreadyExists,
                            format!("element `{}` already exists", element.name),
                        )
                        .with_field("elements.name"));
                    }
                }
            }
            Self::Update { elements, .. } => {
                for element in elements {
                    if !existing_names.contains(&element.name) {
                        return Err(missing_target(&element.name));
                    }
                    if let Some(new_name) = &element.new_name {
                        if new_name != &element.name && existing_names.contains(new_name) {
                            return Err(MetaDiagnostic::error(
                                MetaDiagnosticCode::AlreadyExists,
                                format!("element `{new_name}` already exists"),
                            )
                            .with_field("elements.newName"));
                        }
                    }
                }
            }
            Self::Remove { names, .. } => {
                for name in names {
                    if !existing_names.contains(name) {
                        return Err(missing_target(name));
                    }
                }
            }
            Self::SetProperties { .. } | Self::EditRelations { .. } => {}
        }
        Ok(())
    }
}

fn reject_duplicate_names<'a>(names: impl Iterator<Item = &'a str>) -> Result<(), MetaDiagnostic> {
    let mut seen = HashSet::new();
    for name in names {
        if !seen.insert(name) {
            return Err(invalid_operation(
                "elements.name",
                "element name is duplicated",
            ));
        }
    }
    Ok(())
}

fn missing_target(name: &str) -> MetaDiagnostic {
    MetaDiagnostic::error(
        MetaDiagnosticCode::TargetNotFound,
        format!("element `{name}` was not found"),
    )
    .with_field("elements.name")
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
}
