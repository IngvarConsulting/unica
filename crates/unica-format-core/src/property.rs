//! Neutral property envelopes for semantic values.

use serde::Serialize;

use crate::{
    source::{SourceAdapterError, SourceAdapterErrorKind},
    value::{PropertyType, PropertyValue},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PropertyValueState {
    Explicit,
    Defaulted,
    Inherited,
    Computed,
    Absent,
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PropertyProvenance {
    Declared,
    Default,
    Inherited,
    Derived,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PropertyCapability {
    ReadOnly,
    Authorable,
    Unavailable,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticProperty {
    #[serde(rename = "type")]
    pub value_type: PropertyType,
    pub value_state: PropertyValueState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<PropertyValue>,
    pub provenance: PropertyProvenance,
    pub capability: PropertyCapability,
}

impl SemanticProperty {
    pub fn explicit(
        value_type: PropertyType,
        value: PropertyValue,
        provenance: PropertyProvenance,
    ) -> Result<Self, SourceAdapterError> {
        Self::with_value(value_type, PropertyValueState::Explicit, value, provenance)
    }

    pub fn defaulted(
        value_type: PropertyType,
        value: PropertyValue,
    ) -> Result<Self, SourceAdapterError> {
        Self::with_value(
            value_type,
            PropertyValueState::Defaulted,
            value,
            PropertyProvenance::Default,
        )
    }

    pub fn inherited(
        value_type: PropertyType,
        value: PropertyValue,
    ) -> Result<Self, SourceAdapterError> {
        Self::with_value(
            value_type,
            PropertyValueState::Inherited,
            value,
            PropertyProvenance::Inherited,
        )
    }

    pub fn computed(
        value_type: PropertyType,
        value: PropertyValue,
    ) -> Result<Self, SourceAdapterError> {
        Self::with_value(
            value_type,
            PropertyValueState::Computed,
            value,
            PropertyProvenance::Derived,
        )
    }

    pub fn absent(value_type: PropertyType) -> Self {
        Self::without_value(
            value_type,
            PropertyValueState::Absent,
            PropertyProvenance::Declared,
        )
    }

    pub fn unresolved(value_type: PropertyType) -> Self {
        Self::without_value(
            value_type,
            PropertyValueState::Unresolved,
            PropertyProvenance::Unknown,
        )
    }

    pub fn with_capability(mut self, capability: PropertyCapability) -> Self {
        self.capability = capability;
        self
    }

    fn with_value(
        value_type: PropertyType,
        value_state: PropertyValueState,
        value: PropertyValue,
        provenance: PropertyProvenance,
    ) -> Result<Self, SourceAdapterError> {
        if !property_value_matches(value_type, &value) {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::ProjectionAmbiguous,
                "property type does not match its value",
            ));
        }
        Ok(Self {
            value_type,
            value_state,
            value: Some(value),
            provenance,
            capability: PropertyCapability::Unknown,
        })
    }

    fn without_value(
        value_type: PropertyType,
        value_state: PropertyValueState,
        provenance: PropertyProvenance,
    ) -> Self {
        Self {
            value_type,
            value_state,
            value: None,
            provenance,
            capability: PropertyCapability::Unknown,
        }
    }
}

fn property_value_matches(value_type: PropertyType, value: &PropertyValue) -> bool {
    matches!(
        (value_type, value),
        (PropertyType::Boolean, PropertyValue::Boolean(_))
            | (PropertyType::Integer, PropertyValue::Integer(_))
            | (PropertyType::Decimal, PropertyValue::Decimal(_))
            | (PropertyType::String, PropertyValue::String(_))
            | (
                PropertyType::LocalizedString,
                PropertyValue::LocalizedString(_)
            )
            | (PropertyType::Uuid, PropertyValue::Uuid(_))
            | (PropertyType::Enum, PropertyValue::EnumSymbol(_))
            | (PropertyType::Date, PropertyValue::Date(_))
            | (PropertyType::TypeSet, PropertyValue::TypeSet(_))
            | (PropertyType::ObjectRef, PropertyValue::ObjectRef(_))
            | (PropertyType::List, PropertyValue::List(_))
            | (PropertyType::Structure, PropertyValue::Structure(_))
            | (PropertyType::Null, PropertyValue::Null)
            | (PropertyType::Unknown, PropertyValue::Unknown { .. })
    )
}
