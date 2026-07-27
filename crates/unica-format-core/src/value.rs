//! Typed semantic values independent of every source representation.

use std::collections::BTreeMap;

use serde::{Serialize, Serializer};
use uuid::Uuid;

use crate::{navigation::ObjectRef, semantic_ids::SemanticEnumValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PropertyType {
    Boolean,
    Integer,
    Decimal,
    String,
    LocalizedString,
    Uuid,
    Enum,
    Date,
    TypeSet,
    ObjectRef,
    List,
    Structure,
    Null,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeSetValue {
    pub variants: Vec<TypeVariant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PrimitiveTypeKind {
    Boolean,
    String,
    Number,
    Date,
}

impl PrimitiveTypeKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Boolean => "boolean",
            Self::String => "string",
            Self::Number => "number",
            Self::Date => "date",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StringLength {
    Fixed,
    Variable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NumberSign {
    Any,
    Nonnegative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DateFractions {
    Date,
    DateTime,
    Time,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StringQualifiers {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_length: Option<StringLength>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NumberQualifiers {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digits: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fraction_digits: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_sign: Option<NumberSign>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DateQualifiers {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_fractions: Option<DateFractions>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TypeQualifiers {
    String(StringQualifiers),
    Number(NumberQualifiers),
    Date(DateQualifiers),
}

/// A complete 1C type variant. Native qualified names are normalized before
/// they enter this contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TypeVariant {
    Primitive {
        kind: PrimitiveTypeKind,
        #[serde(skip_serializing_if = "Option::is_none")]
        qualifiers: Option<TypeQualifiers>,
    },
    Reference {
        target: String,
    },
    Enumeration {
        target: String,
    },
    DefinedType {
        target: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyValue {
    Boolean(bool),
    Integer(i64),
    Decimal(String),
    String(String),
    LocalizedString(BTreeMap<String, String>),
    Uuid(Uuid),
    EnumSymbol(SemanticEnumValue),
    Date(String),
    TypeSet(TypeSetValue),
    ObjectRef(ObjectRef),
    List(Vec<PropertyValue>),
    Structure(BTreeMap<String, PropertyValue>),
    Null,
    Unknown { summary: String },
}

impl Serialize for PropertyValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Boolean(value) => serializer.serialize_bool(*value),
            Self::Integer(value) => serializer.serialize_i64(*value),
            Self::Decimal(value) | Self::String(value) | Self::Date(value) => {
                serializer.serialize_str(value)
            }
            Self::LocalizedString(value) => value.serialize(serializer),
            Self::Uuid(value) => serializer.serialize_str(&value.to_string()),
            Self::EnumSymbol(value) => value.serialize(serializer),
            Self::TypeSet(value) => value.serialize(serializer),
            Self::ObjectRef(value) => value.serialize(serializer),
            Self::List(value) => value.serialize(serializer),
            Self::Structure(value) => value.serialize(serializer),
            Self::Null => serializer.serialize_none(),
            Self::Unknown { summary } => {
                let mut map = BTreeMap::new();
                map.insert("summary", summary);
                map.serialize(serializer)
            }
        }
    }
}
