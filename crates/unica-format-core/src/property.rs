//! Neutral, registry-validated property envelopes for semantic values.

use std::{collections::BTreeMap, fmt};

use serde::{
    de::{Error as _, MapAccess, Visitor},
    Deserialize, Deserializer, Serialize,
};

use crate::{
    semantic_ids::SemanticPropertyId,
    source::{SourceAdapterError, SourceAdapterErrorKind},
    value::{PropertyType, PropertyValue},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PropertyValueState {
    Explicit,
    Defaulted,
    Inherited,
    Computed,
    Absent,
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PropertyProvenance {
    Declared,
    Default,
    Inherited,
    Derived,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PropertyCapability {
    ReadOnly,
    Authorable,
    Unavailable,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticPropertyDefinition {
    id: SemanticPropertyId,
    default_type: PropertyType,
    allowed_types: &'static [PropertyType],
}

impl SemanticPropertyDefinition {
    const fn fixed(id: SemanticPropertyId, value_type: PropertyType) -> Self {
        Self {
            id,
            default_type: value_type,
            allowed_types: match value_type {
                PropertyType::Boolean => &[PropertyType::Boolean],
                PropertyType::Integer => &[PropertyType::Integer],
                PropertyType::Decimal => &[PropertyType::Decimal],
                PropertyType::String => &[PropertyType::String],
                PropertyType::LocalizedString => &[PropertyType::LocalizedString],
                PropertyType::Uuid => &[PropertyType::Uuid],
                PropertyType::Enum => &[PropertyType::Enum],
                PropertyType::Date => &[PropertyType::Date],
                PropertyType::TypeSet => &[PropertyType::TypeSet],
                PropertyType::ObjectRef => &[PropertyType::ObjectRef],
                PropertyType::List => &[PropertyType::List],
                PropertyType::Structure => &[PropertyType::Structure],
                PropertyType::Null => &[PropertyType::Null],
                PropertyType::Unknown => &[PropertyType::Unknown],
            },
        }
    }

    const fn polymorphic(
        id: SemanticPropertyId,
        default_type: PropertyType,
        allowed_types: &'static [PropertyType],
    ) -> Self {
        Self {
            id,
            default_type,
            allowed_types,
        }
    }

    pub const fn id(self) -> SemanticPropertyId {
        self.id
    }

    pub const fn default_type(self) -> PropertyType {
        self.default_type
    }

    pub const fn allowed_types(self) -> &'static [PropertyType] {
        self.allowed_types
    }

    pub fn accepts(self, value_type: PropertyType) -> bool {
        self.allowed_types.contains(&value_type)
    }
}

macro_rules! fixed {
    ($id:ident, $value_type:ident) => {
        SemanticPropertyDefinition::fixed(SemanticPropertyId::$id, PropertyType::$value_type)
    };
}

const FILL_VALUE_TYPES: &[PropertyType] = &[
    PropertyType::Boolean,
    PropertyType::Integer,
    PropertyType::Decimal,
    PropertyType::String,
    PropertyType::Uuid,
    PropertyType::Enum,
    PropertyType::Date,
    PropertyType::ObjectRef,
    PropertyType::List,
    PropertyType::Structure,
    PropertyType::Null,
    PropertyType::Unknown,
];

/// One definition for every registered property ID. Order matches
/// `SemanticPropertyId::ALL` and is guarded by contract tests.
pub const SEMANTIC_PROPERTY_DEFINITIONS: &[SemanticPropertyDefinition] = &[
    fixed!(METADATA_KIND, String),
    fixed!(METADATA_NAME, String),
    fixed!(METADATA_UUID, Uuid),
    fixed!(METADATA_SYNONYM, LocalizedString),
    fixed!(METADATA_COMMENT, String),
    fixed!(METADATA_CODE, String),
    fixed!(METADATA_DESCRIPTION, String),
    fixed!(PRESENTATION_OBJECT, LocalizedString),
    fixed!(PRESENTATION_EXTENDED_OBJECT, LocalizedString),
    fixed!(PRESENTATION_LIST, LocalizedString),
    fixed!(PRESENTATION_EXTENDED_LIST, LocalizedString),
    fixed!(SUPPORT_STATE, String),
    fixed!(SUPPORT_AUTHORABILITY, String),
    fixed!(SUPPORT_EDIT_CAPABILITY, String),
    fixed!(DOCUMENT_NUMBER_TYPE, Enum),
    fixed!(DOCUMENT_NUMBER_LENGTH, Integer),
    fixed!(DOCUMENT_NUMBER_PERIODICITY, Enum),
    fixed!(DOCUMENT_NUMBER_AUTO, Boolean),
    fixed!(DOCUMENT_POSTING_MODE, Enum),
    fixed!(DOCUMENT_REAL_TIME_POSTING_MODE, Enum),
    fixed!(DOCUMENT_REGISTER_RECORDS_DELETION_MODE, Enum),
    fixed!(DOCUMENT_REGISTER_RECORDS_WRITING_ON_POST_MODE, Enum),
    fixed!(CATALOG_HIERARCHY_TYPE, Enum),
    fixed!(CATALOG_HIERARCHY_LEVEL_LIMIT, Integer),
    fixed!(CATALOG_CODE_LENGTH, Integer),
    fixed!(CATALOG_DESCRIPTION_LENGTH, Integer),
    fixed!(REGISTER_PERIODICITY, Enum),
    fixed!(REGISTER_WRITE_MODE, Enum),
    fixed!(REGISTER_TYPE, Enum),
    fixed!(CONSTANT_VALUE_TYPE, TypeSet),
    fixed!(REPORT_MAIN_DATA_COMPOSITION_SCHEMA, String),
    fixed!(DEFINED_TYPE, TypeSet),
    fixed!(MODULE_GLOBAL, Boolean),
    fixed!(MODULE_CLIENT_MANAGED_APPLICATION, Boolean),
    fixed!(MODULE_SERVER, Boolean),
    fixed!(MODULE_EXTERNAL_CONNECTION, Boolean),
    fixed!(MODULE_CLIENT_ORDINARY_APPLICATION, Boolean),
    fixed!(MODULE_SERVER_CALL, Boolean),
    fixed!(MODULE_PRIVILEGED, Boolean),
    fixed!(MODULE_RETURN_VALUES_REUSE, Enum),
    fixed!(JOB_METHOD, String),
    fixed!(JOB_USE, Boolean),
    fixed!(JOB_PREDEFINED, Boolean),
    fixed!(JOB_RESTART_COUNT, Integer),
    fixed!(JOB_RESTART_INTERVAL, Integer),
    fixed!(JOB_KEY, String),
    fixed!(SUBSCRIPTION_EVENT, String),
    fixed!(SUBSCRIPTION_HANDLER, String),
    fixed!(SUBSCRIPTION_SOURCE_TYPE, TypeSet),
    fixed!(HTTP_SERVICE_ROOT_URL, String),
    fixed!(HTTP_SERVICE_REUSE_SESSIONS, Enum),
    fixed!(HTTP_SERVICE_SESSION_MAX_AGE, Integer),
    fixed!(HTTP_SERVICE_URL_TEMPLATE, String),
    fixed!(HTTP_SERVICE_METHOD, String),
    fixed!(HTTP_SERVICE_HANDLER, String),
    fixed!(WEB_SERVICE_NAMESPACE, String),
    fixed!(WEB_SERVICE_XDTO_PACKAGES, List),
    fixed!(WEB_SERVICE_DESCRIPTOR_FILE_NAME, String),
    fixed!(WEB_SERVICE_REUSE_SESSIONS, Enum),
    fixed!(WEB_SERVICE_SESSION_MAX_AGE, Integer),
    fixed!(WEB_SERVICE_OPERATION_RETURN_TYPE, TypeSet),
    fixed!(WEB_SERVICE_OPERATION_NILLABLE, Boolean),
    fixed!(WEB_SERVICE_OPERATION_TRANSACTIONED, Boolean),
    fixed!(WEB_SERVICE_OPERATION_PROCEDURE_NAME, String),
    fixed!(WEB_SERVICE_PARAMETER_TYPE, TypeSet),
    fixed!(WEB_SERVICE_PARAMETER_NILLABLE, Boolean),
    fixed!(WEB_SERVICE_PARAMETER_DIRECTION, Enum),
    fixed!(FIELD_TYPE, TypeSet),
    fixed!(FIELD_REQUIRED, Boolean),
    fixed!(FIELD_FILL_CHECKING, Enum),
    fixed!(FIELD_INDEXING, Enum),
    fixed!(FIELD_MULTI_LINE, Boolean),
    fixed!(FIELD_USE, Enum),
    SemanticPropertyDefinition::polymorphic(
        SemanticPropertyId::FIELD_FILL_VALUE,
        PropertyType::Unknown,
        FILL_VALUE_TYPES,
    ),
    fixed!(FIELD_MASTER, Boolean),
    fixed!(FIELD_MAIN_FILTER, Boolean),
    fixed!(FIELD_DENY_INCOMPLETE_VALUES, Boolean),
    fixed!(FIELD_LENGTH, Integer),
    fixed!(FIELD_DIGITS, Integer),
    fixed!(FIELD_FRACTION_DIGITS, Integer),
    fixed!(TABULAR_SECTION_ORDER, Integer),
    fixed!(TABULAR_SECTION_LINE_NUMBER_LENGTH, Integer),
    fixed!(FORM_TYPE, String),
    fixed!(TEMPLATE_TYPE, String),
    fixed!(COMMAND_GROUP, String),
    fixed!(COMMAND_REPRESENTATION, String),
    fixed!(COMMAND_USE_STANDARD, Boolean),
    fixed!(HELP_INCLUDE_IN_CONTENTS, Boolean),
];

pub fn property_definition(id: SemanticPropertyId) -> &'static SemanticPropertyDefinition {
    SEMANTIC_PROPERTY_DEFINITIONS
        .iter()
        .find(|definition| definition.id == id)
        .expect("every closed semantic property ID has a definition")
}

/// A property can be created only through ID-aware validated constructors.
/// Its semantic fields cannot be changed afterward.
///
/// ```compile_fail
/// use unica_format_core::{
///     property::SemanticProperty,
///     semantic_ids::SemanticPropertyId,
///     value::PropertyValue,
/// };
/// let mut property = SemanticProperty::explicit(
///     SemanticPropertyId::METADATA_NAME,
///     PropertyValue::String("Items".to_string()),
/// ).unwrap();
/// property.value = Some(PropertyValue::Integer(1));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticProperty {
    #[serde(skip)]
    id: SemanticPropertyId,
    #[serde(rename = "type")]
    value_type: PropertyType,
    value_state: PropertyValueState,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<PropertyValue>,
    provenance: PropertyProvenance,
    capability: PropertyCapability,
}

impl SemanticProperty {
    pub fn explicit(
        id: SemanticPropertyId,
        value: PropertyValue,
    ) -> Result<Self, SourceAdapterError> {
        Self::with_value(
            id,
            PropertyValueState::Explicit,
            value,
            PropertyProvenance::Declared,
        )
    }

    pub fn defaulted(
        id: SemanticPropertyId,
        value: PropertyValue,
    ) -> Result<Self, SourceAdapterError> {
        Self::with_value(
            id,
            PropertyValueState::Defaulted,
            value,
            PropertyProvenance::Default,
        )
    }

    pub fn inherited(
        id: SemanticPropertyId,
        value: PropertyValue,
    ) -> Result<Self, SourceAdapterError> {
        Self::with_value(
            id,
            PropertyValueState::Inherited,
            value,
            PropertyProvenance::Inherited,
        )
    }

    pub fn computed(
        id: SemanticPropertyId,
        value: PropertyValue,
    ) -> Result<Self, SourceAdapterError> {
        Self::with_value(
            id,
            PropertyValueState::Computed,
            value,
            PropertyProvenance::Derived,
        )
    }

    pub fn absent(id: SemanticPropertyId) -> Self {
        let definition = property_definition(id);
        Self::from_parts(
            id,
            definition.default_type,
            PropertyValueState::Absent,
            None,
            PropertyProvenance::Declared,
            PropertyCapability::Unavailable,
        )
        .expect("core property definitions produce valid absent properties")
    }

    pub fn unresolved(id: SemanticPropertyId) -> Self {
        let definition = property_definition(id);
        Self::from_parts(
            id,
            definition.default_type,
            PropertyValueState::Unresolved,
            None,
            PropertyProvenance::Unknown,
            PropertyCapability::Unknown,
        )
        .expect("core property definitions produce valid unresolved properties")
    }

    pub fn with_capability(
        mut self,
        capability: PropertyCapability,
    ) -> Result<Self, SourceAdapterError> {
        self.capability = capability;
        self.validate()?;
        Ok(self)
    }

    pub const fn id(&self) -> SemanticPropertyId {
        self.id
    }

    pub const fn value_type(&self) -> PropertyType {
        self.value_type
    }

    pub const fn value_state(&self) -> PropertyValueState {
        self.value_state
    }

    pub const fn value(&self) -> Option<&PropertyValue> {
        self.value.as_ref()
    }

    pub const fn provenance(&self) -> PropertyProvenance {
        self.provenance
    }

    pub const fn capability(&self) -> PropertyCapability {
        self.capability
    }

    pub fn validate_for(&self, id: SemanticPropertyId) -> Result<(), SourceAdapterError> {
        if self.id != id {
            return Err(invalid_property(
                "semantic property key does not match its registered definition",
            ));
        }
        self.validate()
    }

    fn with_value(
        id: SemanticPropertyId,
        value_state: PropertyValueState,
        value: PropertyValue,
        provenance: PropertyProvenance,
    ) -> Result<Self, SourceAdapterError> {
        Self::from_parts(
            id,
            value.value_type(),
            value_state,
            Some(value),
            provenance,
            PropertyCapability::Unknown,
        )
    }

    fn from_parts(
        id: SemanticPropertyId,
        value_type: PropertyType,
        value_state: PropertyValueState,
        value: Option<PropertyValue>,
        provenance: PropertyProvenance,
        capability: PropertyCapability,
    ) -> Result<Self, SourceAdapterError> {
        let property = Self {
            id,
            value_type,
            value_state,
            value,
            provenance,
            capability,
        };
        property.validate()?;
        Ok(property)
    }

    fn validate(&self) -> Result<(), SourceAdapterError> {
        if !property_definition(self.id).accepts(self.value_type) {
            return Err(invalid_property(
                "semantic property type is not allowed by its registered definition",
            ));
        }
        match (&self.value, self.value_state) {
            (Some(value), PropertyValueState::Explicit)
            | (Some(value), PropertyValueState::Defaulted)
            | (Some(value), PropertyValueState::Inherited)
            | (Some(value), PropertyValueState::Computed)
                if value.value_type() == self.value_type => {}
            (None, PropertyValueState::Absent | PropertyValueState::Unresolved) => {}
            _ => {
                return Err(invalid_property(
                    "semantic property value is inconsistent with its type or state",
                ))
            }
        }
        if let Some(value) = &self.value {
            value
                .validate()
                .map_err(|error| invalid_property(error.message()))?;
        }
        let expected_provenance = match self.value_state {
            PropertyValueState::Explicit => PropertyProvenance::Declared,
            PropertyValueState::Defaulted => PropertyProvenance::Default,
            PropertyValueState::Inherited => PropertyProvenance::Inherited,
            PropertyValueState::Computed => PropertyProvenance::Derived,
            PropertyValueState::Absent => PropertyProvenance::Declared,
            PropertyValueState::Unresolved => PropertyProvenance::Unknown,
        };
        if self.provenance != expected_provenance {
            return Err(invalid_property(
                "semantic property provenance is inconsistent with its value state",
            ));
        }
        let capability_is_valid = match self.value_state {
            PropertyValueState::Absent => {
                matches!(self.capability, PropertyCapability::Unavailable)
            }
            PropertyValueState::Unresolved => matches!(
                self.capability,
                PropertyCapability::Unknown | PropertyCapability::Unavailable
            ),
            PropertyValueState::Explicit
            | PropertyValueState::Defaulted
            | PropertyValueState::Inherited
            | PropertyValueState::Computed => {
                !matches!(self.capability, PropertyCapability::Unavailable)
            }
        };
        if !capability_is_valid {
            return Err(invalid_property(
                "semantic property capability is inconsistent with its value state",
            ));
        }
        Ok(())
    }
}

fn invalid_property(message: &str) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::ProjectionAmbiguous, message)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SemanticPropertyWire {
    #[serde(rename = "type")]
    value_type: PropertyType,
    value_state: PropertyValueState,
    #[serde(default, deserialize_with = "deserialize_wire_value")]
    value: WireValue,
    provenance: PropertyProvenance,
    capability: PropertyCapability,
}

struct WireValue {
    present: bool,
    value: serde_json::Value,
}

impl Default for WireValue {
    fn default() -> Self {
        Self {
            present: false,
            value: serde_json::Value::Null,
        }
    }
}

fn deserialize_wire_value<'de, D>(deserializer: D) -> Result<WireValue, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(WireValue {
        present: true,
        value: serde_json::Value::deserialize(deserializer)?,
    })
}

impl SemanticPropertyWire {
    fn into_property(self, id: SemanticPropertyId) -> Result<SemanticProperty, SourceAdapterError> {
        let value = self
            .value
            .present
            .then(|| property_value_from_json(self.value_type, self.value.value))
            .transpose()?;
        SemanticProperty::from_parts(
            id,
            self.value_type,
            self.value_state,
            value,
            self.provenance,
            self.capability,
        )
    }
}

/// Deserialize a semantic property object whose JSON keys are the registered
/// property IDs. The key is injected into validation before an entry can exist.
pub fn deserialize_semantic_properties<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<SemanticPropertyId, SemanticProperty>, D::Error>
where
    D: Deserializer<'de>,
{
    struct PropertyMapVisitor;

    impl<'de> Visitor<'de> for PropertyMapVisitor {
        type Value = BTreeMap<SemanticPropertyId, SemanticProperty>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("an object keyed by registered semantic property IDs")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut properties = BTreeMap::new();
            while let Some(raw_id) = map.next_key::<String>()? {
                let id = SemanticPropertyId::parse(&raw_id)
                    .ok_or_else(|| A::Error::custom("unregistered semantic property ID"))?;
                let property = map
                    .next_value::<SemanticPropertyWire>()?
                    .into_property(id)
                    .map_err(A::Error::custom)?;
                if properties.insert(id, property).is_some() {
                    return Err(A::Error::custom("duplicate semantic property ID"));
                }
            }
            Ok(properties)
        }
    }

    deserializer.deserialize_map(PropertyMapVisitor)
}

fn property_value_from_json(
    value_type: PropertyType,
    value: serde_json::Value,
) -> Result<PropertyValue, SourceAdapterError> {
    let value = serde_json::from_value::<PropertyValue>(value)
        .map_err(|_| invalid_property("semantic property value is invalid"))?;
    if value.value_type() != value_type {
        return Err(invalid_property(
            "semantic property value tag is inconsistent with its registered type",
        ));
    }
    Ok(value)
}
