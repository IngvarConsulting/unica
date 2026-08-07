use super::{
    MetaDiagnostic, MetaDiagnosticCode, MetaEventSource, MetaPropertyChanges, MetadataKind,
    MetadataReference, MetadataType, MetadataTypeVariant, NumberSign, StringLengthMode,
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
    #[cfg(test)]
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

/// Metadata kinds that expose a platform `Ref` generated type in the active
/// format profile. Type parsing, public schemas and XML emission consume this
/// registry so they cannot drift into different reference-type domains.
const METADATA_REFERENCE_TYPE_KINDS: &[MetadataKind] = &[
    MetadataKind::Catalog,
    MetadataKind::Document,
    MetadataKind::Enum,
    MetadataKind::ChartOfAccounts,
    MetadataKind::ChartOfCharacteristicTypes,
    MetadataKind::ChartOfCalculationTypes,
    MetadataKind::BusinessProcess,
    MetadataKind::Task,
    MetadataKind::ExchangePlan,
];

pub(crate) fn metadata_reference_type_kinds() -> &'static [MetadataKind] {
    METADATA_REFERENCE_TYPE_KINDS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetaValueProfileContext {
    NewElement,
    Patch,
}

/// Validate the part of a typed element value profile that is knowable from a
/// request alone. A patch may supply only `type` or only `fillValue` because
/// the missing half comes from the existing post-image; when both halves are
/// present, they must already be compatible.
pub(crate) fn validate_metadata_element_value_profile(
    metadata_type: Option<&MetadataType>,
    fill_value: Option<&MetaFillValue>,
    context: MetaValueProfileContext,
) -> Result<(), MetaDiagnostic> {
    let Some(metadata_type) = metadata_type else {
        if fill_value.is_some() && context == MetaValueProfileContext::NewElement {
            return Err(invalid_operation(
                "fillValue",
                "fillValue requires an explicit metadata type",
            ));
        }
        return match fill_value {
            Some(MetaFillValue::Number(value)) if metadata_decimal_shape(value).is_none() => Err(
                invalid_fill_value("numeric fillValue is not lexically valid"),
            ),
            Some(MetaFillValue::DateTime(value)) if !metadata_xs_datetime_is_valid(value) => Err(
                invalid_fill_value("date-time fillValue is not lexically valid"),
            ),
            _ => Ok(()),
        };
    };

    for (index, variant) in metadata_type.variants.iter().enumerate() {
        match variant {
            MetadataTypeVariant::Reference { metadata_path } => {
                let kind = metadata_path
                    .segments()
                    .next()
                    .and_then(|value| MetadataKind::parse(value).ok());
                if !kind.is_some_and(|kind| metadata_reference_type_kinds().contains(&kind)) {
                    return Err(invalid_operation(
                        format!("type.variants[{index}].metadataPath"),
                        "metadata kind does not define a reference type",
                    ));
                }
            }
            MetadataTypeVariant::DefinedType { metadata_path }
                if metadata_path.segments().next() != Some(MetadataKind::DefinedType.as_str()) =>
            {
                return Err(invalid_operation(
                    format!("type.variants[{index}].metadataPath"),
                    "defined-type variant must target DefinedType",
                ));
            }
            _ => {}
        }
    }

    let Some(fill_value) = fill_value else {
        return Ok(());
    };
    let compatible = match fill_value {
        MetaFillValue::String(value) => metadata_type.variants.iter().any(|variant| {
            matches!(
                variant,
                MetadataTypeVariant::String { length, .. }
                    if *length == 0 || value.chars().count() <= *length as usize
            )
        }),
        MetaFillValue::Number(value) => metadata_type.variants.iter().any(|variant| {
            let MetadataTypeVariant::Number {
                digits,
                fraction,
                sign,
            } = variant
            else {
                return false;
            };
            metadata_decimal_shape(value).is_some_and(
                |(negative, integer_digits, fraction_digits)| {
                    (*sign == NumberSign::Any || !negative)
                        && integer_digits + fraction_digits <= *digits as usize
                        && fraction_digits <= *fraction as usize
                },
            )
        }),
        MetaFillValue::Boolean(_) => metadata_type
            .variants
            .iter()
            .any(|variant| matches!(variant, MetadataTypeVariant::Boolean)),
        MetaFillValue::DateTime(value) => {
            metadata_xs_datetime_is_valid(value)
                && metadata_type
                    .variants
                    .iter()
                    .any(|variant| matches!(variant, MetadataTypeVariant::Date { .. }))
        }
        MetaFillValue::Reference(reference) => metadata_type.variants.iter().any(|variant| {
            matches!(
                variant,
                MetadataTypeVariant::Reference { metadata_path }
                    if metadata_path == &reference.metadata_path
            )
        }),
    };
    if compatible {
        Ok(())
    } else {
        Err(invalid_fill_value(
            "fillValue is not lexically valid and compatible with the metadata type",
        ))
    }
}

fn invalid_fill_value(message: impl Into<String>) -> MetaDiagnostic {
    invalid_operation("fillValue", message)
}

pub(crate) fn metadata_decimal_shape(value: &str) -> Option<(bool, usize, usize)> {
    let (negative, unsigned) = match value.as_bytes().first() {
        Some(b'-') => (true, &value[1..]),
        Some(b'+') => (false, &value[1..]),
        _ => (false, value),
    };
    let (integer, fraction) = unsigned
        .split_once('.')
        .map_or((unsigned, ""), |parts| parts);
    if integer.is_empty()
        || !integer.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || (unsigned.contains('.') && fraction.is_empty())
    {
        return None;
    }
    Some((
        negative,
        integer.trim_start_matches('0').len().max(1),
        fraction.len(),
    ))
}

pub(crate) const METADATA_XS_DATETIME_PATTERN: &str = r"^[0-9]+-(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])T([01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9]([.][0-9]+)?(Z|[+-]((0[0-9]|1[0-3]):[0-5][0-9]|14:00))?$";

fn metadata_xs_timezone_offset_is_valid(timezone: &str) -> bool {
    if !timezone.is_ascii()
        || timezone.len() != 6
        || !matches!(timezone.as_bytes().first(), Some(b'+') | Some(b'-'))
        || timezone.as_bytes().get(3) != Some(&b':')
        || !timezone[1..3].bytes().all(|byte| byte.is_ascii_digit())
        || !timezone[4..].bytes().all(|byte| byte.is_ascii_digit())
    {
        return false;
    }
    let (Ok(hour), Ok(minute)) = (timezone[1..3].parse::<u8>(), timezone[4..].parse::<u8>()) else {
        return false;
    };
    (hour < 14 && minute <= 59) || (hour == 14 && minute == 0)
}

pub(crate) fn metadata_xs_datetime_is_valid(value: &str) -> bool {
    if !value.is_ascii() {
        return false;
    }
    let core = if let Some(core) = value.strip_suffix('Z') {
        core
    } else if value.len() >= 6 {
        let split = value.len() - 6;
        let timezone = &value[split..];
        if metadata_xs_timezone_offset_is_valid(timezone) {
            &value[..split]
        } else {
            value
        }
    } else {
        value
    };
    let Some((date, time)) = core.split_once('T') else {
        return false;
    };
    let mut date_parts = date.split('-');
    let (Some(year), Some(month), Some(day), None) = (
        date_parts.next(),
        date_parts.next(),
        date_parts.next(),
        date_parts.next(),
    ) else {
        return false;
    };
    let (Ok(year), Ok(month), Ok(day)) =
        (year.parse::<i32>(), month.parse::<u8>(), day.parse::<u8>())
    else {
        return false;
    };
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 400 == 0 || (year % 4 == 0 && year % 100 != 0) => 29,
        2 => 28,
        _ => return false,
    };
    if day == 0 || day > max_day {
        return false;
    }
    let mut time_parts = time.split(':');
    let (Some(hour), Some(minute), Some(second), None) = (
        time_parts.next(),
        time_parts.next(),
        time_parts.next(),
        time_parts.next(),
    ) else {
        return false;
    };
    let (second, fraction) = second
        .split_once('.')
        .map_or((second, None), |(second, fraction)| {
            (second, Some(fraction))
        });
    hour.parse::<u8>().is_ok_and(|hour| hour <= 23)
        && minute.parse::<u8>().is_ok_and(|minute| minute <= 59)
        && second.parse::<u8>().is_ok_and(|second| second <= 59)
        && fraction.is_none_or(|fraction| {
            !fraction.is_empty() && fraction.bytes().all(|byte| byte.is_ascii_digit())
        })
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
    #[cfg(test)]
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

/// Location of a metadata element relative to its owning metadata object.
///
/// The distinction is part of the public capability contract because the
/// Platform XML profiles for an owner attribute and a tabular-section
/// attribute are not interchangeable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MetaElementScope {
    TopLevel,
    TabularSection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MetaFillValueContextSpec {
    pub(crate) collection: MetaCollection,
    pub(crate) scope: MetaElementScope,
}

const TOP_LEVEL_ATTRIBUTE_FILL: MetaFillValueContextSpec = MetaFillValueContextSpec {
    collection: MetaCollection::Attributes,
    scope: MetaElementScope::TopLevel,
};
const TABULAR_ATTRIBUTE_FILL: MetaFillValueContextSpec = MetaFillValueContextSpec {
    collection: MetaCollection::Attributes,
    scope: MetaElementScope::TabularSection,
};
const TOP_LEVEL_DIMENSION_FILL: MetaFillValueContextSpec = MetaFillValueContextSpec {
    collection: MetaCollection::Dimensions,
    scope: MetaElementScope::TopLevel,
};
const TOP_LEVEL_RESOURCE_FILL: MetaFillValueContextSpec = MetaFillValueContextSpec {
    collection: MetaCollection::Resources,
    scope: MetaElementScope::TopLevel,
};

const ATTRIBUTE_FILL_CONTEXTS: &[MetaFillValueContextSpec] = &[TOP_LEVEL_ATTRIBUTE_FILL];
const TABULAR_FILL_CONTEXTS: &[MetaFillValueContextSpec] = &[TABULAR_ATTRIBUTE_FILL];
const INFORMATION_REGISTER_FILL_CONTEXTS: &[MetaFillValueContextSpec] = &[
    TOP_LEVEL_ATTRIBUTE_FILL,
    TOP_LEVEL_DIMENSION_FILL,
    TOP_LEVEL_RESOURCE_FILL,
];

/// Closed FillValue capability registry for every public metadata owner kind.
///
/// This mirrors the exact nodes emitted by the Platform XML writer, but lives
/// in the domain so request parsing, JSON Schema generation and the writer can
/// consume one decision source.
pub(crate) fn metadata_fill_value_contexts(
    kind: MetadataKind,
) -> &'static [MetaFillValueContextSpec] {
    match kind {
        MetadataKind::Catalog
        | MetadataKind::Document
        | MetadataKind::BusinessProcess
        | MetadataKind::Task
        | MetadataKind::ExchangePlan => ATTRIBUTE_FILL_CONTEXTS,
        MetadataKind::InformationRegister => INFORMATION_REGISTER_FILL_CONTEXTS,
        MetadataKind::Report | MetadataKind::DataProcessor => TABULAR_FILL_CONTEXTS,
        MetadataKind::Enum
        | MetadataKind::Constant
        | MetadataKind::AccumulationRegister
        | MetadataKind::AccountingRegister
        | MetadataKind::CalculationRegister
        | MetadataKind::ChartOfAccounts
        | MetadataKind::ChartOfCharacteristicTypes
        | MetadataKind::ChartOfCalculationTypes
        | MetadataKind::DocumentJournal
        | MetadataKind::CommonModule
        | MetadataKind::ScheduledJob
        | MetadataKind::EventSubscription
        | MetadataKind::HTTPService
        | MetadataKind::WebService
        | MetadataKind::DefinedType => &[],
    }
}

pub(crate) fn metadata_fill_value_is_allowed(
    kind: MetadataKind,
    collection: MetaCollection,
    scope: MetaElementScope,
) -> bool {
    metadata_fill_value_contexts(kind)
        .iter()
        .any(|candidate| candidate.collection == collection && candidate.scope == scope)
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
    #[cfg(test)]
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
    if !metadata_identifier_is_valid(name) {
        Err(invalid_operation(
            field,
            "name must be a valid 1C identifier",
        ))
    } else {
        Ok(())
    }
}

pub(crate) fn metadata_identifier_is_valid(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    metadata_identifier_start_is_valid(first)
        && chars.all(|ch| metadata_identifier_start_is_valid(ch) || ch.is_ascii_digit())
}

fn metadata_identifier_start_is_valid(ch: char) -> bool {
    ch == '_'
        || ch.is_ascii_alphabetic()
        || ('А'..='Я').contains(&ch)
        || ('а'..='я').contains(&ch)
        || ch == 'Ё'
        || ch == 'ё'
}

fn metadata_name_key(name: &str) -> String {
    name.to_lowercase()
}

pub(crate) fn metadata_name_eq(left: &str, right: &str) -> bool {
    metadata_name_key(left) == metadata_name_key(right)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetaRelation {
    Owners,
    RegisterRecords,
    BasedOn,
    InputByString,
    Source,
}

impl MetaRelation {
    pub(crate) const ALL: &'static [Self] = &[
        Self::Owners,
        Self::RegisterRecords,
        Self::BasedOn,
        Self::InputByString,
        Self::Source,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Owners => "owners",
            Self::RegisterRecords => "registerRecords",
            Self::BasedOn => "basedOn",
            Self::InputByString => "inputByString",
            Self::Source => "source",
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
pub(crate) enum MetaRelationTargetPolicy {
    /// The target must be a metadata object whose kind belongs to this closed
    /// list.
    MetadataKinds(&'static [MetadataKind]),
    /// The target must be a field path belonging to the exact owner object.
    /// Field existence remains a post-image validation because earlier
    /// operations in the same request may create or rename the field.
    SameOwnerField,
    /// The target is one member of the closed EventSubscription source type
    /// algebra. Source replacement is validated as one correlated set.
    EventSources,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MetaRelationSpec {
    pub(crate) relation: MetaRelation,
    pub(crate) target_policy: MetaRelationTargetPolicy,
}

const CATALOG_RELATION_TARGETS: &[MetadataKind] = &[MetadataKind::Catalog];
const REGISTER_RELATION_TARGETS: &[MetadataKind] = &[
    MetadataKind::InformationRegister,
    MetadataKind::AccumulationRegister,
    MetadataKind::AccountingRegister,
    MetadataKind::CalculationRegister,
];
const BASED_ON_RELATION_TARGETS: &[MetadataKind] = &[
    MetadataKind::Catalog,
    MetadataKind::Document,
    MetadataKind::ChartOfCharacteristicTypes,
    MetadataKind::ChartOfAccounts,
    MetadataKind::ChartOfCalculationTypes,
    MetadataKind::ExchangePlan,
    MetadataKind::BusinessProcess,
    MetadataKind::Task,
];
const CATALOG_RELATION_SPECS: &[MetaRelationSpec] = &[
    MetaRelationSpec {
        relation: MetaRelation::Owners,
        target_policy: MetaRelationTargetPolicy::MetadataKinds(CATALOG_RELATION_TARGETS),
    },
    MetaRelationSpec {
        relation: MetaRelation::BasedOn,
        target_policy: MetaRelationTargetPolicy::MetadataKinds(BASED_ON_RELATION_TARGETS),
    },
    MetaRelationSpec {
        relation: MetaRelation::InputByString,
        target_policy: MetaRelationTargetPolicy::SameOwnerField,
    },
];
const DOCUMENT_RELATION_SPECS: &[MetaRelationSpec] = &[
    MetaRelationSpec {
        relation: MetaRelation::RegisterRecords,
        target_policy: MetaRelationTargetPolicy::MetadataKinds(REGISTER_RELATION_TARGETS),
    },
    MetaRelationSpec {
        relation: MetaRelation::BasedOn,
        target_policy: MetaRelationTargetPolicy::MetadataKinds(BASED_ON_RELATION_TARGETS),
    },
    MetaRelationSpec {
        relation: MetaRelation::InputByString,
        target_policy: MetaRelationTargetPolicy::SameOwnerField,
    },
];
const BASED_ON_INPUT_RELATION_SPECS: &[MetaRelationSpec] = &[
    MetaRelationSpec {
        relation: MetaRelation::BasedOn,
        target_policy: MetaRelationTargetPolicy::MetadataKinds(BASED_ON_RELATION_TARGETS),
    },
    MetaRelationSpec {
        relation: MetaRelation::InputByString,
        target_policy: MetaRelationTargetPolicy::SameOwnerField,
    },
];
const EVENT_SUBSCRIPTION_RELATION_SPECS: &[MetaRelationSpec] = &[MetaRelationSpec {
    relation: MetaRelation::Source,
    target_policy: MetaRelationTargetPolicy::EventSources,
}];

/// Relations physically present in the minimal Platform XML template for an
/// owner kind, together with the target policy enforced by typed mutation.
pub(crate) fn metadata_relation_specs(kind: MetadataKind) -> &'static [MetaRelationSpec] {
    match kind {
        MetadataKind::Catalog => CATALOG_RELATION_SPECS,
        MetadataKind::Document => DOCUMENT_RELATION_SPECS,
        MetadataKind::ChartOfAccounts
        | MetadataKind::ChartOfCharacteristicTypes
        | MetadataKind::ChartOfCalculationTypes
        | MetadataKind::BusinessProcess
        | MetadataKind::Task
        | MetadataKind::ExchangePlan => BASED_ON_INPUT_RELATION_SPECS,
        MetadataKind::EventSubscription => EVENT_SUBSCRIPTION_RELATION_SPECS,
        MetadataKind::Enum
        | MetadataKind::Constant
        | MetadataKind::InformationRegister
        | MetadataKind::AccumulationRegister
        | MetadataKind::AccountingRegister
        | MetadataKind::CalculationRegister
        | MetadataKind::DocumentJournal
        | MetadataKind::Report
        | MetadataKind::DataProcessor
        | MetadataKind::CommonModule
        | MetadataKind::ScheduledJob
        | MetadataKind::HTTPService
        | MetadataKind::WebService
        | MetadataKind::DefinedType => &[],
    }
}

pub(crate) fn metadata_relation_spec(
    kind: MetadataKind,
    relation: MetaRelation,
) -> Option<&'static MetaRelationSpec> {
    metadata_relation_specs(kind)
        .iter()
        .find(|spec| spec.relation == relation)
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
    EventSource(MetaEventSource),
}

impl MetaRelationTarget {
    pub(crate) fn wire_value(&self) -> String {
        match self {
            Self::Object(reference) => reference.metadata_path.as_str().to_string(),
            Self::Field(path) => path.value.clone(),
            Self::EventSource(source) => source.identity_key(),
        }
    }

    pub(crate) fn dependency(&self) -> Option<&MetadataAddress> {
        match self {
            Self::Object(reference) => Some(&reference.metadata_path),
            Self::Field(path) => Some(&path.owner),
            Self::EventSource(source) => source.metadata_path(),
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

/// Validate the static relation profile. Dynamic field existence is
/// intentionally not checked here: earlier operations in one atomic request
/// may create or rename an inputByString field, so that check belongs to the
/// final private post-image.
pub(crate) fn validate_metadata_relation_target_profile(
    owner_kind: MetadataKind,
    owner: &MetadataAddress,
    relation: MetaRelation,
    target: &MetaRelationTarget,
) -> Result<(), MetaDiagnostic> {
    if owner.segments().next() != Some(owner_kind.as_str()) {
        return Err(invalid_operation(
            "metadataPath",
            "metadata owner kind does not match its logical address",
        ));
    }
    let spec = metadata_relation_spec(owner_kind, relation).ok_or_else(|| {
        MetaDiagnostic::error(
            MetaDiagnosticCode::UnsupportedKind,
            format!(
                "relation `{}` is not supported for {}",
                relation.as_str(),
                owner_kind.as_str()
            ),
        )
        .with_field("relation")
    })?;

    match (spec.target_policy, target) {
        (
            MetaRelationTargetPolicy::MetadataKinds(allowed),
            MetaRelationTarget::Object(reference),
        ) => {
            let target_kind = reference
                .metadata_path
                .segments()
                .next()
                .and_then(|name| MetadataKind::parse(name).ok());
            if target_kind.is_some_and(|kind| allowed.contains(&kind)) {
                Ok(())
            } else {
                Err(invalid_operation(
                    "targets",
                    format!(
                        "relation `{}` target kind is not allowed for {}",
                        relation.as_str(),
                        owner_kind.as_str()
                    ),
                ))
            }
        }
        (MetaRelationTargetPolicy::SameOwnerField, MetaRelationTarget::Field(field)) => {
            if &field.owner == owner {
                Ok(())
            } else {
                Err(invalid_operation(
                    "targets",
                    "inputByString field must belong to the edited object",
                ))
            }
        }
        (MetaRelationTargetPolicy::EventSources, MetaRelationTarget::EventSource(source)) => {
            validate_metadata_event_source(source)
        }
        (MetaRelationTargetPolicy::EventSources, _) => Err(invalid_operation(
            "targets",
            "source relation requires typed event source targets",
        )),
        (_, MetaRelationTarget::EventSource(_)) => Err(invalid_operation(
            "targets",
            "typed event source target is only legal for the source relation",
        )),
        (MetaRelationTargetPolicy::SameOwnerField, MetaRelationTarget::Object(_)) => Err(
            invalid_operation("targets", "inputByString requires typed field paths"),
        ),
        (MetaRelationTargetPolicy::MetadataKinds(_), MetaRelationTarget::Field(_)) => {
            Err(invalid_operation(
                "targets",
                "metadata object relation requires metadata object targets",
            ))
        }
    }
}

fn validate_metadata_event_source(source: &MetaEventSource) -> Result<(), MetaDiagnostic> {
    match source {
        MetaEventSource::String {
            length,
            allowed_length,
        } if *length != 0 || *allowed_length != StringLengthMode::Variable => {
            Err(invalid_operation(
                "targets",
                "event source string requires length 0 and allowedLength variable",
            ))
        }
        MetaEventSource::Number {
            digits, fraction, ..
        } if *digits > 38 || *fraction > *digits => Err(invalid_operation(
            "targets",
            "event source number digits must be 0..=38 and fraction must not exceed digits",
        )),
        source => {
            let Some(metadata_path) = source.metadata_path() else {
                return Ok(());
            };
            if metadata_path.segments().count() != 2 {
                return Err(invalid_operation(
                    "targets",
                    "event source metadataPath must identify a top-level metadata object",
                ));
            }
            let name = metadata_path.segments().nth(1).unwrap_or_default();
            if !metadata_identifier_is_valid(name) {
                return Err(invalid_operation(
                    "targets",
                    "event source metadataPath name must be a valid 1C identifier",
                ));
            }
            let target_kind = metadata_path
                .segments()
                .next()
                .and_then(|name| MetadataKind::parse(name).ok());
            if !target_kind.is_some_and(|kind| {
                source
                    .compatible_metadata_kinds()
                    .is_some_and(|allowed| allowed.contains(&kind))
            }) {
                return Err(invalid_operation(
                    "targets",
                    format!(
                        "{} event source is incompatible with metadataPath `{}`",
                        source.as_str(),
                        metadata_path.as_str()
                    ),
                ));
            }
            Ok(())
        }
    }
}

fn validate_relation_edit_shape(
    relation: MetaRelation,
    mode: RelationEditMode,
    targets: &[MetaRelationTarget],
) -> Result<(), MetaDiagnostic> {
    if relation != MetaRelation::Source {
        return if targets.is_empty() {
            Err(invalid_operation(
                "targets",
                "relation targets must not be empty",
            ))
        } else {
            Ok(())
        };
    }
    if mode != RelationEditMode::Replace {
        return Err(invalid_operation(
            "mode",
            "source relation supports replace mode only",
        ));
    }
    if targets.len() > 1
        && targets.iter().any(|target| {
            matches!(
                target,
                MetaRelationTarget::EventSource(MetaEventSource::ValueStorage)
            )
        })
    {
        return Err(invalid_operation(
            "targets",
            "ValueStorage must be the only event source target",
        ));
    }
    let mut identities = HashSet::new();
    for (index, target) in targets.iter().enumerate() {
        let MetaRelationTarget::EventSource(source) = target else {
            return Err(invalid_operation(
                format!("targets[{index}]"),
                "source relation requires typed event source targets",
            ));
        };
        validate_metadata_event_source(source).map_err(|mut diagnostic| {
            if diagnostic.field.as_deref() == Some("targets") {
                diagnostic.field = Some(format!("targets[{index}]"));
            }
            diagnostic
        })?;
        if !identities.insert(source.identity_key()) {
            return Err(invalid_operation(
                format!("targets[{index}]"),
                "duplicate event source identity",
            ));
        }
    }
    Ok(())
}

/// Validate every owner-dependent part of one already parsed typed operation.
/// This is the shared bridge for the public parser, schema and provider: it
/// rejects the complete operation before the provider mutates its private
/// working image.
pub(crate) fn validate_metadata_operation_capabilities(
    owner_kind: MetadataKind,
    owner: &MetadataAddress,
    operation: &MetaEditOperation,
) -> Result<(), MetaDiagnostic> {
    match operation {
        MetaEditOperation::SetProperties { .. } => Ok(()),
        MetaEditOperation::Add {
            collection,
            scope,
            elements,
        } => {
            validate_metadata_kind_collection(owner_kind, *collection)?;
            for (index, element) in elements.iter().enumerate() {
                validate_element_fill_value_capability(
                    owner_kind,
                    *collection,
                    if scope.is_some() {
                        MetaElementScope::TabularSection
                    } else {
                        MetaElementScope::TopLevel
                    },
                    element,
                    &format!("elements[{index}]"),
                )?;
            }
            Ok(())
        }
        MetaEditOperation::Update {
            collection,
            scope,
            elements,
        } => {
            validate_metadata_kind_collection(owner_kind, *collection)?;
            let context = if scope.is_some() {
                MetaElementScope::TabularSection
            } else {
                MetaElementScope::TopLevel
            };
            for (index, element) in elements.iter().enumerate() {
                if element.fill_value.is_some()
                    && !metadata_fill_value_is_allowed(owner_kind, *collection, context)
                {
                    return Err(invalid_operation(
                        format!("elements[{index}].fillValue"),
                        "fillValue is not available for this metadata field context",
                    ));
                }
                validate_metadata_element_value_profile(
                    element.r#type.as_ref(),
                    element.fill_value.as_ref(),
                    MetaValueProfileContext::Patch,
                )
                .map_err(|diagnostic| {
                    qualify_element_diagnostic(diagnostic, &format!("elements[{index}]"))
                })?;
            }
            Ok(())
        }
        MetaEditOperation::Remove { collection, .. } => {
            validate_metadata_kind_collection(owner_kind, *collection)
        }
        MetaEditOperation::EditRelations {
            relation,
            mode,
            targets,
        } => {
            validate_relation_edit_shape(*relation, *mode, targets)?;
            // Validate availability even before inspecting targets so an empty
            // or forged operation cannot turn an absent Platform XML node into
            // a provider concern.
            if metadata_relation_spec(owner_kind, *relation).is_none() {
                return Err(MetaDiagnostic::error(
                    MetaDiagnosticCode::UnsupportedKind,
                    format!(
                        "relation `{}` is not supported for {}",
                        relation.as_str(),
                        owner_kind.as_str()
                    ),
                )
                .with_field("relation"));
            }
            for (index, target) in targets.iter().enumerate() {
                validate_metadata_relation_target_profile(owner_kind, owner, *relation, target)
                    .map_err(|mut diagnostic| {
                        if diagnostic.field.as_deref() == Some("targets") {
                            diagnostic.field = Some(format!("targets[{index}]"));
                        }
                        diagnostic
                    })?;
            }
            Ok(())
        }
    }
}

fn validate_element_fill_value_capability(
    owner_kind: MetadataKind,
    collection: MetaCollection,
    scope: MetaElementScope,
    element: &MetaElementDefinition,
    field: &str,
) -> Result<(), MetaDiagnostic> {
    if element.fill_value.is_some()
        && !metadata_fill_value_is_allowed(owner_kind, collection, scope)
    {
        return Err(invalid_operation(
            format!("{field}.fillValue"),
            "fillValue is not available for this metadata field context",
        ));
    }
    validate_metadata_element_value_profile(
        element.r#type.as_ref(),
        element.fill_value.as_ref(),
        MetaValueProfileContext::NewElement,
    )
    .map_err(|diagnostic| qualify_element_diagnostic(diagnostic, field))?;
    if collection == MetaCollection::TabularSections {
        for (index, attribute) in element.attributes.iter().enumerate() {
            validate_element_fill_value_capability(
                owner_kind,
                MetaCollection::Attributes,
                MetaElementScope::TabularSection,
                attribute,
                &format!("{field}.attributes[{index}]"),
            )?;
        }
    }
    Ok(())
}

fn qualify_element_diagnostic(mut diagnostic: MetaDiagnostic, field: &str) -> MetaDiagnostic {
    diagnostic.field = Some(match diagnostic.field.take() {
        Some(suffix) => format!("{field}.{suffix}"),
        None => field.to_string(),
    });
    diagnostic
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

    #[cfg(test)]
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
        validate_relation_edit_shape(relation, mode, &targets)?;
        Ok(Self::EditRelations {
            relation,
            mode,
            targets,
        })
    }

    #[cfg(test)]
    pub(crate) fn validate_targets(
        &self,
        existing_names: &HashSet<String>,
    ) -> Result<(), MetaDiagnostic> {
        let existing_keys = existing_names
            .iter()
            .map(|name| metadata_name_key(name))
            .collect::<HashSet<_>>();
        match self {
            Self::Add { elements, .. } => {
                for (index, element) in elements.iter().enumerate() {
                    if existing_keys.contains(&metadata_name_key(&element.name)) {
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
                    if !existing_keys.contains(&metadata_name_key(&element.name)) {
                        return Err(missing_target(
                            &element.name,
                            format!("elements[{index}].name"),
                        ));
                    }
                }

                let mut final_names = existing_keys;
                for element in elements {
                    final_names.remove(&metadata_name_key(&element.name));
                }
                for (index, element) in elements.iter().enumerate() {
                    let final_name = element.new_name.as_ref().unwrap_or(&element.name);
                    if !final_names.insert(metadata_name_key(final_name)) {
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
                    if !existing_keys.contains(&metadata_name_key(name)) {
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
        if !seen.insert(metadata_name_key(name)) {
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
        if !seen.insert(metadata_name_key(value)) {
            return Err(invalid_operation(
                format!("{field}[{index}]"),
                "value is duplicated",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
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
            [
                "owners",
                "registerRecords",
                "basedOn",
                "inputByString",
                "source",
            ]
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
    fn relation_capability_registry_correlates_owner_and_target_profiles() {
        use MetadataKind::*;

        let relation_names = |kind| {
            metadata_relation_specs(kind)
                .iter()
                .map(|spec| spec.relation.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            relation_names(Catalog),
            ["owners", "basedOn", "inputByString"]
        );
        assert_eq!(
            relation_names(Document),
            ["registerRecords", "basedOn", "inputByString"]
        );
        for kind in [
            ChartOfAccounts,
            ChartOfCharacteristicTypes,
            ChartOfCalculationTypes,
            BusinessProcess,
            Task,
            ExchangePlan,
        ] {
            assert_eq!(relation_names(kind), ["basedOn", "inputByString"]);
        }
        for kind in [
            Enum,
            Constant,
            InformationRegister,
            AccumulationRegister,
            AccountingRegister,
            CalculationRegister,
            DocumentJournal,
            Report,
            DataProcessor,
            CommonModule,
            ScheduledJob,
            HTTPService,
            WebService,
            DefinedType,
        ] {
            assert!(
                metadata_relation_specs(kind).is_empty(),
                "{}",
                kind.as_str()
            );
        }

        assert_eq!(
            relation_names(EventSubscription),
            ["source"],
            "EventSubscription owns the typed source relation"
        );

        assert_eq!(
            metadata_relation_spec(Catalog, MetaRelation::Owners)
                .unwrap()
                .target_policy,
            MetaRelationTargetPolicy::MetadataKinds(&[Catalog])
        );
        assert_eq!(
            metadata_relation_spec(Document, MetaRelation::RegisterRecords)
                .unwrap()
                .target_policy,
            MetaRelationTargetPolicy::MetadataKinds(&[
                InformationRegister,
                AccumulationRegister,
                AccountingRegister,
                CalculationRegister,
            ])
        );
        assert_eq!(
            metadata_relation_spec(Task, MetaRelation::BasedOn)
                .unwrap()
                .target_policy,
            MetaRelationTargetPolicy::MetadataKinds(&[
                Catalog,
                Document,
                ChartOfCharacteristicTypes,
                ChartOfAccounts,
                ChartOfCalculationTypes,
                ExchangePlan,
                BusinessProcess,
                Task,
            ])
        );
        assert_eq!(
            metadata_relation_spec(Task, MetaRelation::InputByString)
                .unwrap()
                .target_policy,
            MetaRelationTargetPolicy::SameOwnerField
        );
        assert_eq!(
            metadata_relation_spec(EventSubscription, MetaRelation::Source)
                .unwrap()
                .target_policy,
            MetaRelationTargetPolicy::EventSources
        );
    }

    #[test]
    fn event_source_relation_is_replace_only_and_is_the_only_clearable_relation() {
        assert!(MetaEditOperation::edit_relation_targets(
            MetaRelation::Source,
            RelationEditMode::Replace,
            Vec::new(),
        )
        .is_ok());
        assert!(MetaEditOperation::edit_relation_targets(
            MetaRelation::Source,
            RelationEditMode::Add,
            vec![MetaRelationTarget::EventSource(MetaEventSource::Boolean)],
        )
        .is_err());
        assert!(MetaEditOperation::edit_relation_targets(
            MetaRelation::Owners,
            RelationEditMode::Replace,
            Vec::new(),
        )
        .is_err());
    }

    #[test]
    fn event_source_set_rejects_duplicate_identities_and_value_storage_composites() {
        let duplicate = MetaEditOperation::edit_relation_targets(
            MetaRelation::Source,
            RelationEditMode::Replace,
            vec![
                MetaRelationTarget::EventSource(MetaEventSource::String {
                    length: 0,
                    allowed_length: StringLengthMode::Variable,
                }),
                MetaRelationTarget::EventSource(MetaEventSource::String {
                    length: 0,
                    allowed_length: StringLengthMode::Variable,
                }),
            ],
        )
        .unwrap_err();
        assert_eq!(duplicate.field.as_deref(), Some("targets[1]"));

        let composite = MetaEditOperation::edit_relation_targets(
            MetaRelation::Source,
            RelationEditMode::Replace,
            vec![
                MetaRelationTarget::EventSource(MetaEventSource::ValueStorage),
                MetaRelationTarget::EventSource(MetaEventSource::Boolean),
            ],
        )
        .unwrap_err();
        assert_eq!(composite.field.as_deref(), Some("targets"));
    }

    #[test]
    fn event_source_profile_correlates_kind_and_metadata_path() {
        let owner = MetadataAddress::parse(
            crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            "EventSubscription.Notify",
        )
        .unwrap();
        let source = |source| MetaRelationTarget::EventSource(source);
        let address = |path: &str| -> MetadataAddress {
            MetadataAddress::parse(
                crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
                path,
            )
            .unwrap()
        };

        for event_source in [
            MetaEventSource::Object {
                metadata_path: address("Report.Sales"),
            },
            MetaEventSource::Reference {
                metadata_path: address("Enum.Status"),
            },
            MetaEventSource::RecordSet {
                metadata_path: address("InformationRegister.Facts"),
            },
            MetaEventSource::DefinedType {
                metadata_path: address("DefinedType.Identifier"),
            },
        ] {
            assert!(validate_metadata_relation_target_profile(
                MetadataKind::EventSubscription,
                &owner,
                MetaRelation::Source,
                &source(event_source),
            )
            .is_ok());
        }

        for event_source in [
            MetaEventSource::Object {
                metadata_path: address("Constant.Setting"),
            },
            MetaEventSource::Reference {
                metadata_path: address("InformationRegister.Facts"),
            },
            MetaEventSource::RecordSet {
                metadata_path: address("Catalog.Items"),
            },
            MetaEventSource::DefinedType {
                metadata_path: address("Catalog.Items"),
            },
        ] {
            assert!(validate_metadata_relation_target_profile(
                MetadataKind::EventSubscription,
                &owner,
                MetaRelation::Source,
                &source(event_source),
            )
            .is_err());
        }

        for invalid_name in ["Bad Name", "1Bad", "Bad:Name", "Bad-Name"] {
            let invalid_path = format!("Catalog.{invalid_name}");
            let event_source = MetaEventSource::Reference {
                metadata_path: address(&invalid_path),
            };
            let error = validate_metadata_relation_target_profile(
                MetadataKind::EventSubscription,
                &owner,
                MetaRelation::Source,
                &source(event_source),
            )
            .unwrap_err();
            assert!(error.message.contains("1C identifier"), "{error:?}");
        }
    }

    #[test]
    fn event_source_primitive_qualifiers_are_strict() {
        for source in [
            MetaEventSource::String {
                length: 1,
                allowed_length: StringLengthMode::Variable,
            },
            MetaEventSource::String {
                length: 0,
                allowed_length: StringLengthMode::Fixed,
            },
            MetaEventSource::Number {
                digits: 39,
                fraction: 0,
                sign: NumberSign::Any,
            },
            MetaEventSource::Number {
                digits: 5,
                fraction: 6,
                sign: NumberSign::Any,
            },
        ] {
            assert!(MetaEditOperation::edit_relation_targets(
                MetaRelation::Source,
                RelationEditMode::Replace,
                vec![MetaRelationTarget::EventSource(source)],
            )
            .is_err());
        }
    }

    #[test]
    fn relation_capability_validation_rejects_cross_kind_and_cross_owner_targets() {
        let catalog = MetadataAddress::parse(
            crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            "Catalog.Items",
        )
        .unwrap();
        let document = MetadataAddress::parse(
            crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            "Document.Order",
        )
        .unwrap();
        let object_target = |path: &str| {
            MetaRelationTarget::Object(MetadataReference {
                metadata_path: MetadataAddress::parse(
                    crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
                    path,
                )
                .unwrap(),
            })
        };

        assert!(validate_metadata_relation_target_profile(
            MetadataKind::Catalog,
            &catalog,
            MetaRelation::Owners,
            &object_target("Catalog.Parent"),
        )
        .is_ok());
        assert!(validate_metadata_relation_target_profile(
            MetadataKind::Catalog,
            &catalog,
            MetaRelation::Owners,
            &object_target("Document.Parent"),
        )
        .is_err());
        assert!(validate_metadata_relation_target_profile(
            MetadataKind::Document,
            &document,
            MetaRelation::RegisterRecords,
            &object_target("InformationRegister.Facts"),
        )
        .is_ok());
        assert!(validate_metadata_relation_target_profile(
            MetadataKind::Document,
            &document,
            MetaRelation::RegisterRecords,
            &object_target("Catalog.Items"),
        )
        .is_err());
        for based_on in [
            "Catalog.Items",
            "Document.Quote",
            "ChartOfCharacteristicTypes.Properties",
            "ChartOfAccounts.Main",
            "ChartOfCalculationTypes.Accruals",
            "ExchangePlan.Distributed",
            "BusinessProcess.Approval",
            "Task.Review",
        ] {
            assert!(
                validate_metadata_relation_target_profile(
                    MetadataKind::Document,
                    &document,
                    MetaRelation::BasedOn,
                    &object_target(based_on),
                )
                .is_ok(),
                "{based_on}"
            );
        }
        assert!(validate_metadata_relation_target_profile(
            MetadataKind::Document,
            &document,
            MetaRelation::BasedOn,
            &object_target("CommonModule.Utility"),
        )
        .is_err());

        let own_field = MetaRelationTarget::Field(
            MetadataFieldPath::parse("Document.Order.StandardAttribute.Number").unwrap(),
        );
        let foreign_field = MetaRelationTarget::Field(
            MetadataFieldPath::parse("Document.Quote.StandardAttribute.Number").unwrap(),
        );
        assert!(validate_metadata_relation_target_profile(
            MetadataKind::Document,
            &document,
            MetaRelation::InputByString,
            &own_field,
        )
        .is_ok());
        assert!(validate_metadata_relation_target_profile(
            MetadataKind::Document,
            &document,
            MetaRelation::InputByString,
            &foreign_field,
        )
        .is_err());
        assert!(validate_metadata_relation_target_profile(
            MetadataKind::Enum,
            &MetadataAddress::parse(
                crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
                "Enum.State",
            )
            .unwrap(),
            MetaRelation::BasedOn,
            &object_target("Enum.Other"),
        )
        .is_err());
    }

    #[test]
    fn fill_value_capability_registry_is_owner_collection_and_scope_specific() {
        use MetaCollection::*;
        use MetaElementScope::*;
        use MetadataKind::*;

        for kind in [
            Catalog,
            Document,
            InformationRegister,
            BusinessProcess,
            Task,
            ExchangePlan,
        ] {
            assert!(metadata_fill_value_is_allowed(kind, Attributes, TopLevel));
        }
        for kind in [
            ChartOfAccounts,
            ChartOfCharacteristicTypes,
            ChartOfCalculationTypes,
            AccumulationRegister,
            AccountingRegister,
            CalculationRegister,
            Report,
            DataProcessor,
        ] {
            assert!(!metadata_fill_value_is_allowed(kind, Attributes, TopLevel));
        }
        for kind in MetadataKind::ALL {
            assert_eq!(
                metadata_fill_value_is_allowed(*kind, Attributes, TabularSection),
                matches!(kind, Report | DataProcessor),
                "{}",
                kind.as_str()
            );
            assert_eq!(
                metadata_fill_value_is_allowed(*kind, Dimensions, TopLevel),
                *kind == InformationRegister,
                "{} dimensions",
                kind.as_str()
            );
            assert_eq!(
                metadata_fill_value_is_allowed(*kind, Resources, TopLevel),
                *kind == InformationRegister,
                "{} resources",
                kind.as_str()
            );
        }
    }

    #[test]
    fn typed_value_profile_registry_and_context_rules_are_closed() {
        use MetadataKind::*;

        assert_eq!(
            metadata_reference_type_kinds(),
            &[
                Catalog,
                Document,
                Enum,
                ChartOfAccounts,
                ChartOfCharacteristicTypes,
                ChartOfCalculationTypes,
                BusinessProcess,
                Task,
                ExchangePlan,
            ]
        );

        let fill = MetaFillValue::Boolean(true);
        let error = validate_metadata_element_value_profile(
            None,
            Some(&fill),
            MetaValueProfileContext::NewElement,
        )
        .unwrap_err();
        assert_eq!(error.field.as_deref(), Some("fillValue"));
        assert!(validate_metadata_element_value_profile(
            None,
            Some(&fill),
            MetaValueProfileContext::Patch,
        )
        .is_ok());
        let error = validate_metadata_element_value_profile(
            None,
            Some(&MetaFillValue::Number("1e3".into())),
            MetaValueProfileContext::Patch,
        )
        .unwrap_err();
        assert_eq!(error.field.as_deref(), Some("fillValue"));

        let string_type = MetadataType::new(vec![MetadataTypeVariant::String {
            length: 3,
            allowed_length: crate::domain::metadata::StringLengthMode::Variable,
        }])
        .unwrap();
        let error = validate_metadata_element_value_profile(
            Some(&string_type),
            Some(&MetaFillValue::String("LONG".into())),
            MetaValueProfileContext::NewElement,
        )
        .unwrap_err();
        assert_eq!(error.field.as_deref(), Some("fillValue"));

        let address = |value| {
            MetadataAddress::parse(
                crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
                value,
            )
            .unwrap()
        };
        for (variant, field) in [
            (
                MetadataTypeVariant::Reference {
                    metadata_path: address("Report.Sales"),
                },
                "type.variants[0].metadataPath",
            ),
            (
                MetadataTypeVariant::DefinedType {
                    metadata_path: address("Catalog.Items"),
                },
                "type.variants[0].metadataPath",
            ),
        ] {
            let metadata_type = MetadataType::new(vec![variant]).unwrap();
            let error = validate_metadata_element_value_profile(
                Some(&metadata_type),
                None,
                MetaValueProfileContext::Patch,
            )
            .unwrap_err();
            assert_eq!(error.field.as_deref(), Some(field));
        }
    }

    #[test]
    fn xs_datetime_timezone_stops_at_exactly_fourteen_hours() {
        for timezone in ["+13:59", "-13:59", "+14:00", "-14:00"] {
            assert!(
                metadata_xs_datetime_is_valid(&format!("2026-01-01T12:00:00{timezone}")),
                "valid XSD timezone was rejected: {timezone}"
            );
        }
        for timezone in ["+14:01", "-14:01", "+14:59", "-14:59"] {
            assert!(
                !metadata_xs_datetime_is_valid(&format!("2026-01-01T12:00:00{timezone}")),
                "timezone beyond the XSD boundary was accepted: {timezone}"
            );
        }
    }

    #[test]
    fn xs_datetime_rejects_non_ascii_without_panicking() {
        assert!(!metadata_xs_datetime_is_valid("2026-01-01T12:00:00Ω12345"));
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
    fn element_names_must_be_valid_1c_identifiers_at_every_depth() {
        let nested = MetaEditOperation::add(
            MetaCollection::TabularSections,
            None,
            vec![MetaElementInput {
                name: "Lines".into(),
                attributes: Some(vec![MetaElementInput::named("Товар Цена")]),
                ..MetaElementInput::default()
            }],
        )
        .unwrap_err();
        assert_eq!(nested.code, MetaDiagnosticCode::InvalidArguments);
        assert_eq!(
            nested.field.as_deref(),
            Some("elements[0].attributes[0].name")
        );

        let rename = MetaEditOperation::update(
            MetaCollection::Attributes,
            None,
            vec![MetaElementUpdateInput {
                name: "Товар".into(),
                new_name: Some("1Товар".into()),
                ..MetaElementUpdateInput::default()
            }],
        )
        .unwrap_err();
        assert_eq!(rename.code, MetaDiagnosticCode::InvalidArguments);
        assert_eq!(rename.field.as_deref(), Some("elements[0].newName"));
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
    fn target_validation_matches_metadata_names_case_insensitively() {
        let existing = HashSet::from(["Товар".to_string()]);
        let duplicate = MetaEditOperation::add(
            MetaCollection::Attributes,
            None,
            vec![MetaElementInput::named("товар")],
        )
        .unwrap()
        .validate_targets(&existing)
        .unwrap_err();
        assert_eq!(duplicate.code, MetaDiagnosticCode::AlreadyExists);

        let update = MetaEditOperation::update(
            MetaCollection::Attributes,
            None,
            vec![MetaElementUpdateInput {
                name: "тОвАр".into(),
                comment: Some("changed".into()),
                ..MetaElementUpdateInput::default()
            }],
        )
        .unwrap();
        assert!(update.validate_targets(&existing).is_ok());

        let remove =
            MetaEditOperation::remove(MetaCollection::Attributes, None, vec!["ТОВАР".into()])
                .unwrap();
        assert!(remove.validate_targets(&existing).is_ok());
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
