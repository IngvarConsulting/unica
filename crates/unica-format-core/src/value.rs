//! Typed semantic values independent of every source representation.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::{Display, Formatter},
};

use serde::{
    de::{DeserializeOwned, Error as _, MapAccess, SeqAccess, Visitor},
    ser::Error as _,
    Deserialize, Deserializer, Serialize, Serializer,
};
use uuid::Uuid;

use crate::{
    navigation::{IdentityStrength, ObjectKey, ObjectRef},
    semantic_ids::{SemanticEnumValue, SemanticObjectKind},
    source::SourceId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticValueError {
    message: &'static str,
}

impl SemanticValueError {
    const fn new(message: &'static str) -> Self {
        Self { message }
    }

    pub const fn message(self) -> &'static str {
        self.message
    }
}

impl Display for SemanticValueError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for SemanticValueError {}

/// A JSON value decoded through `MapAccess` before semantic dispatch. Unlike
/// `serde_json::Value`, it rejects repeated object keys instead of retaining
/// the last value.
enum StrictJsonValue {
    Null,
    Boolean(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

impl StrictJsonValue {
    fn into_json(self) -> serde_json::Value {
        match self {
            Self::Null => serde_json::Value::Null,
            Self::Boolean(value) => serde_json::Value::Bool(value),
            Self::Number(value) => serde_json::Value::Number(value),
            Self::String(value) => serde_json::Value::String(value),
            Self::Array(values) => {
                serde_json::Value::Array(values.into_iter().map(Self::into_json).collect())
            }
            Self::Object(values) => serde_json::Value::Object(
                values
                    .into_iter()
                    .map(|(key, value)| (key, value.into_json()))
                    .collect(),
            ),
        }
    }
}

impl<'de> Deserialize<'de> for StrictJsonValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct StrictJsonVisitor;

        impl<'de> Visitor<'de> for StrictJsonVisitor {
            type Value = StrictJsonValue;

            fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a JSON value without duplicate object keys")
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(StrictJsonValue::Null)
            }

            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(StrictJsonValue::Null)
            }

            fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: Deserializer<'de>,
            {
                StrictJsonValue::deserialize(deserializer)
            }

            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
                Ok(StrictJsonValue::Boolean(value))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
                Ok(StrictJsonValue::Number(value.into()))
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
                Ok(StrictJsonValue::Number(value.into()))
            }

            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                serde_json::Number::from_f64(value)
                    .map(StrictJsonValue::Number)
                    .ok_or_else(|| E::custom("non-finite JSON number is invalid"))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
                Ok(StrictJsonValue::String(value.to_string()))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
                Ok(StrictJsonValue::String(value))
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0));
                while let Some(value) = sequence.next_element::<StrictJsonValue>()? {
                    values.push(value);
                }
                Ok(StrictJsonValue::Array(values))
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut values = BTreeMap::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(A::Error::custom(format!(
                            "duplicate JSON object key `{key}`"
                        )));
                    }
                    values.insert(key, map.next_value::<StrictJsonValue>()?);
                }
                Ok(StrictJsonValue::Object(values))
            }
        }

        deserializer.deserialize_any(StrictJsonVisitor)
    }
}

fn deserialize_strict<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let value = StrictJsonValue::deserialize(deserializer)?;
    serde_json::from_value(value.into_json()).map_err(D::Error::custom)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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
    variants: Vec<TypeVariant>,
}

impl TypeSetValue {
    pub fn new(variants: Vec<TypeVariant>) -> Result<Self, SemanticValueError> {
        if variants.is_empty() {
            return Err(SemanticValueError::new(
                "semantic type set must contain at least one variant",
            ));
        }
        for variant in &variants {
            variant.validate()?;
        }
        if variants.iter().collect::<BTreeSet<_>>().len() != variants.len() {
            return Err(SemanticValueError::new(
                "semantic type set contains duplicate variants",
            ));
        }
        Ok(Self { variants })
    }

    pub fn variants(&self) -> &[TypeVariant] {
        &self.variants
    }

    pub fn validate(&self) -> Result<(), SemanticValueError> {
        Self::new(self.variants.clone()).map(|_| ())
    }
}

impl<'de> Deserialize<'de> for TypeSetValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            variants: Vec<TypeVariant>,
        }

        let wire: Wire = deserialize_strict(deserializer)?;
        Self::new(wire.variants).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StringLength {
    Fixed,
    Variable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NumberSign {
    Any,
    Nonnegative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DateFractions {
    Date,
    DateTime,
    Time,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StringQualifiers {
    #[serde(skip_serializing_if = "Option::is_none")]
    length: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    allowed_length: Option<StringLength>,
}

impl StringQualifiers {
    pub fn new(
        length: Option<u32>,
        allowed_length: Option<StringLength>,
    ) -> Result<Self, SemanticValueError> {
        if length == Some(0) && allowed_length != Some(StringLength::Variable) {
            return Err(SemanticValueError::new(
                "zero semantic string length must be variable",
            ));
        }
        if allowed_length.is_some() && length.is_none() {
            return Err(SemanticValueError::new(
                "semantic allowed string length requires a length",
            ));
        }
        if length.is_none() && allowed_length.is_none() {
            return Err(SemanticValueError::new(
                "semantic string qualifiers cannot be empty",
            ));
        }
        Ok(Self {
            length,
            allowed_length,
        })
    }

    pub const fn length(&self) -> Option<u32> {
        self.length
    }

    pub const fn allowed_length(&self) -> Option<StringLength> {
        self.allowed_length
    }
}

impl<'de> Deserialize<'de> for StringQualifiers {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            length: Option<u32>,
            allowed_length: Option<StringLength>,
        }

        let wire: Wire = deserialize_strict(deserializer)?;
        Self::new(wire.length, wire.allowed_length).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NumberQualifiers {
    #[serde(skip_serializing_if = "Option::is_none")]
    digits: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fraction_digits: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    allowed_sign: Option<NumberSign>,
}

impl NumberQualifiers {
    pub fn new(
        digits: Option<u32>,
        fraction_digits: Option<u32>,
        allowed_sign: Option<NumberSign>,
    ) -> Result<Self, SemanticValueError> {
        if digits == Some(0) {
            return Err(SemanticValueError::new(
                "semantic number qualifier digits must be positive",
            ));
        }
        if fraction_digits.is_some() && digits.is_none() {
            return Err(SemanticValueError::new(
                "semantic fraction digits require total digits",
            ));
        }
        if matches!((digits, fraction_digits), (Some(digits), Some(fraction)) if fraction > digits)
        {
            return Err(SemanticValueError::new(
                "semantic fraction digits cannot exceed total digits",
            ));
        }
        if digits.is_none() && fraction_digits.is_none() && allowed_sign.is_none() {
            return Err(SemanticValueError::new(
                "semantic number qualifiers cannot be empty",
            ));
        }
        Ok(Self {
            digits,
            fraction_digits,
            allowed_sign,
        })
    }

    pub const fn digits(&self) -> Option<u32> {
        self.digits
    }

    pub const fn fraction_digits(&self) -> Option<u32> {
        self.fraction_digits
    }

    pub const fn allowed_sign(&self) -> Option<NumberSign> {
        self.allowed_sign
    }
}

impl<'de> Deserialize<'de> for NumberQualifiers {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            digits: Option<u32>,
            fraction_digits: Option<u32>,
            allowed_sign: Option<NumberSign>,
        }

        let wire: Wire = deserialize_strict(deserializer)?;
        Self::new(wire.digits, wire.fraction_digits, wire.allowed_sign).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DateQualifiers {
    #[serde(skip_serializing_if = "Option::is_none")]
    date_fractions: Option<DateFractions>,
}

impl DateQualifiers {
    pub fn new(date_fractions: Option<DateFractions>) -> Result<Self, SemanticValueError> {
        if date_fractions.is_none() {
            return Err(SemanticValueError::new(
                "semantic date qualifiers cannot be empty",
            ));
        }
        Ok(Self { date_fractions })
    }

    pub const fn date_fractions(&self) -> Option<DateFractions> {
        self.date_fractions
    }
}

impl<'de> Deserialize<'de> for DateQualifiers {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            date_fractions: Option<DateFractions>,
        }

        let wire: Wire = deserialize_strict(deserializer)?;
        Self::new(wire.date_fractions).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TypeQualifiers {
    String(StringQualifiers),
    Number(NumberQualifiers),
    Date(DateQualifiers),
}

impl<'de> Deserialize<'de> for TypeQualifiers {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        enum Wire {
            String(StringQualifiers),
            Number(NumberQualifiers),
            Date(DateQualifiers),
        }

        let wire: Wire = deserialize_strict(deserializer)?;
        Ok(match wire {
            Wire::String(value) => Self::String(value),
            Wire::Number(value) => Self::Number(value),
            Wire::Date(value) => Self::Date(value),
        })
    }
}

/// A validated semantic metadata target. The kind is compiler-owned and the
/// name is an identifier, never a native qualified name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticTypeTarget {
    kind: SemanticObjectKind,
    name: String,
}

impl SemanticTypeTarget {
    fn new(kind: SemanticObjectKind, name: impl Into<String>) -> Result<Self, SemanticValueError> {
        let name = name.into();
        if !is_semantic_target_name(&name) {
            return Err(SemanticValueError::new(
                "semantic type target name is invalid",
            ));
        }
        Ok(Self { kind, name })
    }

    pub const fn kind(&self) -> SemanticObjectKind {
        self.kind
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

fn is_semantic_target_name(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|first| first == '_' || first.is_alphabetic())
        && chars.all(|character| character == '_' || character.is_alphanumeric())
}

fn is_reference_target_kind(kind: SemanticObjectKind) -> bool {
    matches!(
        kind,
        SemanticObjectKind::Catalog
            | SemanticObjectKind::Document
            | SemanticObjectKind::ExchangePlan
            | SemanticObjectKind::ChartOfCharacteristicTypes
            | SemanticObjectKind::ChartOfAccounts
            | SemanticObjectKind::ChartOfCalculationTypes
            | SemanticObjectKind::BusinessProcess
            | SemanticObjectKind::Task
    )
}

/// A complete semantic type variant. Its representation is private so callers
/// cannot pair a primitive with incompatible qualifiers or invent a target.
///
/// ```compile_fail
/// use unica_format_core::value::TypeVariant;
///
/// let _ = TypeVariant::Reference {
///     target: "cfg:CatalogRef.Products".to_string(),
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TypeVariant {
    value: TypeVariantValue,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum TypeVariantValue {
    Primitive {
        kind: PrimitiveTypeKind,
        qualifiers: Option<TypeQualifiers>,
    },
    Reference(SemanticTypeTarget),
    Enumeration(SemanticTypeTarget),
    DefinedType(SemanticTypeTarget),
}

impl TypeVariant {
    pub fn primitive(
        kind: PrimitiveTypeKind,
        qualifiers: Option<TypeQualifiers>,
    ) -> Result<Self, SemanticValueError> {
        let compatible = matches!(
            (kind, qualifiers.as_ref()),
            (PrimitiveTypeKind::Boolean, None)
                | (
                    PrimitiveTypeKind::String,
                    None | Some(TypeQualifiers::String(_))
                )
                | (
                    PrimitiveTypeKind::Number,
                    None | Some(TypeQualifiers::Number(_))
                )
                | (
                    PrimitiveTypeKind::Date,
                    None | Some(TypeQualifiers::Date(_))
                )
        );
        if !compatible {
            return Err(SemanticValueError::new(
                "semantic type qualifiers are incompatible with the primitive",
            ));
        }
        Ok(Self {
            value: TypeVariantValue::Primitive { kind, qualifiers },
        })
    }

    pub fn reference(
        kind: SemanticObjectKind,
        name: impl Into<String>,
    ) -> Result<Self, SemanticValueError> {
        if !is_reference_target_kind(kind) {
            return Err(SemanticValueError::new(
                "semantic reference target kind is not referenceable",
            ));
        }
        Ok(Self {
            value: TypeVariantValue::Reference(SemanticTypeTarget::new(kind, name)?),
        })
    }

    pub fn enumeration(name: impl Into<String>) -> Result<Self, SemanticValueError> {
        Ok(Self {
            value: TypeVariantValue::Enumeration(SemanticTypeTarget::new(
                SemanticObjectKind::Enumeration,
                name,
            )?),
        })
    }

    pub fn defined_type(name: impl Into<String>) -> Result<Self, SemanticValueError> {
        Ok(Self {
            value: TypeVariantValue::DefinedType(SemanticTypeTarget::new(
                SemanticObjectKind::DefinedType,
                name,
            )?),
        })
    }

    pub const fn primitive_kind(&self) -> Option<PrimitiveTypeKind> {
        match &self.value {
            TypeVariantValue::Primitive { kind, .. } => Some(*kind),
            _ => None,
        }
    }

    pub fn qualifiers(&self) -> Option<&TypeQualifiers> {
        match &self.value {
            TypeVariantValue::Primitive { qualifiers, .. } => qualifiers.as_ref(),
            _ => None,
        }
    }

    pub fn target(&self) -> Option<&SemanticTypeTarget> {
        match &self.value {
            TypeVariantValue::Reference(target)
            | TypeVariantValue::Enumeration(target)
            | TypeVariantValue::DefinedType(target) => Some(target),
            TypeVariantValue::Primitive { .. } => None,
        }
    }

    pub fn validate(&self) -> Result<(), SemanticValueError> {
        match &self.value {
            TypeVariantValue::Primitive { kind, qualifiers } => {
                Self::primitive(*kind, qualifiers.clone()).map(|_| ())
            }
            TypeVariantValue::Reference(target) => {
                Self::reference(target.kind, target.name.clone()).map(|_| ())
            }
            TypeVariantValue::Enumeration(target)
                if target.kind == SemanticObjectKind::Enumeration =>
            {
                Self::enumeration(target.name.clone()).map(|_| ())
            }
            TypeVariantValue::DefinedType(target)
                if target.kind == SemanticObjectKind::DefinedType =>
            {
                Self::defined_type(target.name.clone()).map(|_| ())
            }
            TypeVariantValue::Enumeration(_) | TypeVariantValue::DefinedType(_) => Err(
                SemanticValueError::new("semantic type target kind is inconsistent"),
            ),
        }
    }
}

impl Serialize for TypeVariant {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        #[serde(tag = "kind", rename_all = "camelCase")]
        enum Wire<'a> {
            Primitive {
                primitive: PrimitiveTypeKind,
                #[serde(skip_serializing_if = "Option::is_none")]
                qualifiers: Option<&'a TypeQualifiers>,
            },
            Reference {
                target: &'a SemanticTypeTarget,
            },
            Enumeration {
                target: &'a SemanticTypeTarget,
            },
            DefinedType {
                target: &'a SemanticTypeTarget,
            },
        }

        self.validate().map_err(S::Error::custom)?;
        match &self.value {
            TypeVariantValue::Primitive { kind, qualifiers } => Wire::Primitive {
                primitive: *kind,
                qualifiers: qualifiers.as_ref(),
            },
            TypeVariantValue::Reference(target) => Wire::Reference { target },
            TypeVariantValue::Enumeration(target) => Wire::Enumeration { target },
            TypeVariantValue::DefinedType(target) => Wire::DefinedType { target },
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TypeVariant {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct TargetWire {
            kind: String,
            name: String,
        }

        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
        enum Wire {
            Primitive {
                primitive: PrimitiveTypeKind,
                qualifiers: Option<TypeQualifiers>,
            },
            Reference {
                target: TargetWire,
            },
            Enumeration {
                target: TargetWire,
            },
            DefinedType {
                target: TargetWire,
            },
        }

        fn target_kind<E: serde::de::Error>(target: &TargetWire) -> Result<SemanticObjectKind, E> {
            SemanticObjectKind::parse(&target.kind)
                .ok_or_else(|| E::custom("semantic type target kind is unregistered"))
        }

        let wire: Wire = deserialize_strict(deserializer)?;
        match wire {
            Wire::Primitive {
                primitive,
                qualifiers,
            } => Self::primitive(primitive, qualifiers),
            Wire::Reference { target } => {
                Self::reference(target_kind::<D::Error>(&target)?, target.name)
            }
            Wire::Enumeration { target } => {
                if target_kind::<D::Error>(&target)? != SemanticObjectKind::Enumeration {
                    return Err(D::Error::custom(
                        "semantic enumeration target has an impossible kind",
                    ));
                }
                Self::enumeration(target.name)
            }
            Wire::DefinedType { target } => {
                if target_kind::<D::Error>(&target)? != SemanticObjectKind::DefinedType {
                    return Err(D::Error::custom(
                        "semantic defined-type target has an impossible kind",
                    ));
                }
                Self::defined_type(target.name)
            }
        }
        .map_err(D::Error::custom)
    }
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

impl PropertyValue {
    pub const fn value_type(&self) -> PropertyType {
        match self {
            Self::Boolean(_) => PropertyType::Boolean,
            Self::Integer(_) => PropertyType::Integer,
            Self::Decimal(_) => PropertyType::Decimal,
            Self::String(_) => PropertyType::String,
            Self::LocalizedString(_) => PropertyType::LocalizedString,
            Self::Uuid(_) => PropertyType::Uuid,
            Self::EnumSymbol(_) => PropertyType::Enum,
            Self::Date(_) => PropertyType::Date,
            Self::TypeSet(_) => PropertyType::TypeSet,
            Self::ObjectRef(_) => PropertyType::ObjectRef,
            Self::List(_) => PropertyType::List,
            Self::Structure(_) => PropertyType::Structure,
            Self::Null => PropertyType::Null,
            Self::Unknown { .. } => PropertyType::Unknown,
        }
    }

    pub fn validate(&self) -> Result<(), SemanticValueError> {
        self.validate_at_depth(0)
    }

    fn validate_at_depth(&self, depth: usize) -> Result<(), SemanticValueError> {
        match self {
            Self::Decimal(value) if !is_canonical_decimal(value) => {
                Err(SemanticValueError::new("semantic decimal value is invalid"))
            }
            Self::LocalizedString(values)
                if values.is_empty() || values.keys().any(|locale| !is_semantic_locale(locale)) =>
            {
                Err(SemanticValueError::new(
                    "semantic localized string locale is invalid",
                ))
            }
            Self::Date(value) if !is_semantic_date(value) => {
                Err(SemanticValueError::new("semantic date value is invalid"))
            }
            Self::TypeSet(value) => value.validate(),
            Self::ObjectRef(value)
                if value.display_name.is_empty()
                    || value.display_name.chars().any(char::is_control) =>
            {
                Err(SemanticValueError::new(
                    "semantic object reference display name is invalid",
                ))
            }
            Self::List(values) => {
                for value in values {
                    value.validate_at_depth(depth + 1)?;
                }
                Ok(())
            }
            Self::Structure(values) => {
                for (name, value) in values {
                    if name.is_empty() || name.chars().any(char::is_control) {
                        return Err(SemanticValueError::new("semantic structure key is invalid"));
                    }
                    value.validate_at_depth(depth + 1)?;
                }
                Ok(())
            }
            Self::Unknown { summary }
                if summary.is_empty() || summary.chars().any(char::is_control) =>
            {
                Err(SemanticValueError::new(
                    "semantic unknown value summary is invalid",
                ))
            }
            Self::Boolean(_)
            | Self::Integer(_)
            | Self::Decimal(_)
            | Self::String(_)
            | Self::LocalizedString(_)
            | Self::Uuid(_)
            | Self::EnumSymbol(_)
            | Self::Date(_)
            | Self::ObjectRef(_)
            | Self::Null
            | Self::Unknown { .. } => Ok(()),
        }
    }
}

impl Serialize for PropertyValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        struct UnknownRef<'a> {
            summary: &'a str,
        }

        #[derive(Serialize)]
        #[serde(tag = "type", content = "value", rename_all = "camelCase")]
        enum Wire<'a> {
            Boolean(bool),
            Integer(i64),
            Decimal(&'a str),
            String(&'a str),
            LocalizedString(&'a BTreeMap<String, String>),
            Uuid(String),
            #[serde(rename = "enum")]
            EnumSymbol(&'a SemanticEnumValue),
            Date(&'a str),
            TypeSet(&'a TypeSetValue),
            ObjectRef(&'a ObjectRef),
            List(&'a [PropertyValue]),
            Structure(&'a BTreeMap<String, PropertyValue>),
            Null,
            Unknown(UnknownRef<'a>),
        }

        self.validate().map_err(S::Error::custom)?;
        match self {
            Self::Boolean(value) => Wire::Boolean(*value),
            Self::Integer(value) => Wire::Integer(*value),
            Self::Decimal(value) => Wire::Decimal(value),
            Self::String(value) => Wire::String(value),
            Self::LocalizedString(value) => Wire::LocalizedString(value),
            Self::Uuid(value) => Wire::Uuid(value.to_string()),
            Self::EnumSymbol(value) => Wire::EnumSymbol(value),
            Self::Date(value) => Wire::Date(value),
            Self::TypeSet(value) => Wire::TypeSet(value),
            Self::ObjectRef(value) => Wire::ObjectRef(value),
            Self::List(value) => Wire::List(value),
            Self::Structure(value) => Wire::Structure(value),
            Self::Null => Wire::Null,
            Self::Unknown { summary } => Wire::Unknown(UnknownRef { summary }),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PropertyValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct ObjectRefWire {
            source_id: String,
            object_key: String,
            identity_strength: String,
            kind: String,
            display_name: String,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct UnknownWire {
            summary: String,
        }

        #[derive(Deserialize)]
        #[serde(
            tag = "type",
            content = "value",
            rename_all = "camelCase",
            deny_unknown_fields
        )]
        enum Wire {
            Boolean(bool),
            Integer(i64),
            Decimal(String),
            String(String),
            LocalizedString(BTreeMap<String, String>),
            Uuid(String),
            #[serde(rename = "enum")]
            EnumSymbol(String),
            Date(String),
            TypeSet(TypeSetValue),
            ObjectRef(ObjectRefWire),
            List(Vec<PropertyValue>),
            Structure(BTreeMap<String, PropertyValue>),
            Null,
            Unknown(UnknownWire),
        }

        let raw = StrictJsonValue::deserialize(deserializer)?.into_json();
        if raw.get("type").and_then(serde_json::Value::as_str) == Some("null")
            && raw
                .as_object()
                .is_some_and(|value| value.contains_key("value"))
        {
            return Err(D::Error::custom(
                "semantic null value cannot carry a payload",
            ));
        }
        let wire = serde_json::from_value::<Wire>(raw).map_err(D::Error::custom)?;
        let value = match wire {
            Wire::Boolean(value) => Self::Boolean(value),
            Wire::Integer(value) => Self::Integer(value),
            Wire::Decimal(value) => Self::Decimal(value),
            Wire::String(value) => Self::String(value),
            Wire::LocalizedString(value) => Self::LocalizedString(value),
            Wire::Uuid(value) => Self::Uuid(
                Uuid::parse_str(&value)
                    .map_err(|_| D::Error::custom("semantic UUID value is invalid"))?,
            ),
            Wire::EnumSymbol(value) => Self::EnumSymbol(
                SemanticEnumValue::parse(&value)
                    .ok_or_else(|| D::Error::custom("semantic enum value is unregistered"))?,
            ),
            Wire::Date(value) => Self::Date(value),
            Wire::TypeSet(value) => Self::TypeSet(value),
            Wire::ObjectRef(value) => {
                let identity_strength = match value.identity_strength.as_str() {
                    "persistent" => IdentityStrength::Persistent,
                    "derived" => IdentityStrength::Derived,
                    "snapshotOnly" => IdentityStrength::SnapshotOnly,
                    _ => {
                        return Err(D::Error::custom(
                            "semantic object reference identity strength is invalid",
                        ))
                    }
                };
                let kind = SemanticObjectKind::parse(&value.kind).ok_or_else(|| {
                    D::Error::custom("semantic object reference kind is unregistered")
                })?;
                Self::ObjectRef(ObjectRef::new(
                    SourceId::new(value.source_id).map_err(D::Error::custom)?,
                    ObjectKey::new(value.object_key).map_err(D::Error::custom)?,
                    identity_strength,
                    kind,
                    value.display_name,
                ))
            }
            Wire::List(value) => Self::List(value),
            Wire::Structure(value) => Self::Structure(value),
            Wire::Null => Self::Null,
            Wire::Unknown(value) => Self::Unknown {
                summary: value.summary,
            },
        };
        value.validate().map_err(D::Error::custom)?;
        Ok(value)
    }
}

fn is_semantic_locale(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn is_canonical_decimal(value: &str) -> bool {
    let value = value.strip_prefix('-').unwrap_or(value);
    if value.is_empty() {
        return false;
    }
    let mut parts = value.split('.');
    let integer = parts.next().unwrap_or_default();
    let fraction = parts.next();
    if parts.next().is_some()
        || integer.is_empty()
        || !integer.bytes().all(|byte| byte.is_ascii_digit())
        || (integer.len() > 1 && integer.starts_with('0'))
    {
        return false;
    }
    fraction.is_none_or(|fraction| {
        !fraction.is_empty() && fraction.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn is_semantic_date(value: &str) -> bool {
    if !value.is_ascii() || value.len() < 10 || !valid_date_part(&value.as_bytes()[..10]) {
        return false;
    }
    if value.len() == 10 {
        return true;
    }
    if value.as_bytes().get(10) != Some(&b'T') {
        return false;
    }
    valid_time_part(&value[11..])
}

fn valid_date_part(value: &[u8]) -> bool {
    if value.len() != 10 || value[4] != b'-' || value[7] != b'-' {
        return false;
    }
    let Some(year) = decimal_component(&value[0..4]) else {
        return false;
    };
    let Some(month) = decimal_component(&value[5..7]) else {
        return false;
    };
    let Some(day) = decimal_component(&value[8..10]) else {
        return false;
    };
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 400 == 0 || (year % 4 == 0 && year % 100 != 0) => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day)
}

fn valid_time_part(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 8 || bytes[2] != b':' || bytes[5] != b':' {
        return false;
    }
    let Some(hour) = decimal_component(&bytes[0..2]) else {
        return false;
    };
    let Some(minute) = decimal_component(&bytes[3..5]) else {
        return false;
    };
    let Some(second) = decimal_component(&bytes[6..8]) else {
        return false;
    };
    if hour > 23 || minute > 59 || second > 59 {
        return false;
    }
    let mut remainder = &value[8..];
    if let Some(fraction) = remainder.strip_prefix('.') {
        let digits = fraction
            .bytes()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if digits == 0 {
            return false;
        }
        remainder = &fraction[digits..];
    }
    if remainder.is_empty() || remainder == "Z" {
        return true;
    }
    let Some(offset) = remainder
        .strip_prefix('+')
        .or_else(|| remainder.strip_prefix('-'))
    else {
        return false;
    };
    let bytes = offset.as_bytes();
    bytes.len() == 5
        && bytes[2] == b':'
        && decimal_component(&bytes[0..2]).is_some_and(|hour| hour <= 23)
        && decimal_component(&bytes[3..5]).is_some_and(|minute| minute <= 59)
}

fn decimal_component(value: &[u8]) -> Option<u32> {
    value.iter().all(|byte| byte.is_ascii_digit()).then(|| {
        value
            .iter()
            .fold(0, |result, byte| result * 10 + u32::from(*byte - b'0'))
    })
}
