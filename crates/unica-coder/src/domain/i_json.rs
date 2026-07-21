use serde::de::{Error as _, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Number, Value};
use std::collections::HashSet;
use std::fmt;

const MAX_I_JSON_INTEROPERABLE_INTEGER: u64 = (1 << 53) - 1;

#[derive(Clone, Copy, Debug)]
pub(crate) struct NonInteroperableIJsonNumber;

#[derive(Clone, Copy, Debug)]
pub(crate) struct NonInteroperableIJsonString;

#[derive(Clone, Copy, Debug)]
pub(crate) enum IJsonValidationError {
    NonInteroperableNumber,
    NonInteroperableString,
}

/// Parses one complete JSON value while rejecting duplicate names and values that
/// cannot be represented interoperably by I-JSON consumers.
pub(crate) fn from_slice(input: &[u8]) -> Result<Value, serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_slice(input);
    let value = StrictValue::deserialize(&mut deserializer)?.0;
    deserializer.end()?;
    Ok(value)
}

struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictValueVisitor)
    }
}

struct StrictValueVisitor;

impl<'de> Visitor<'de> for StrictValueVisitor {
    type Value = StrictValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a strict JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(StrictValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.number(Number::from(value))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.number(Number::from(value))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        let number =
            Number::from_f64(value).ok_or_else(|| E::custom("non-interoperable JSON number"))?;
        self.number(number)
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        validate_i_json_string(value)
            .map_err(|_| E::custom("I-JSON string contains forbidden Unicode scalar"))?;
        Ok(StrictValue(Value::String(value.to_owned())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        validate_i_json_string(&value)
            .map_err(|_| E::custom("I-JSON string contains forbidden Unicode scalar"))?;
        Ok(StrictValue(Value::String(value)))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(StrictValue(Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(StrictValue(Value::Null))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<StrictValue>()? {
            values.push(value.0);
        }
        Ok(StrictValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut keys = HashSet::new();
        let mut values = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            validate_i_json_string(&key)
                .map_err(|_| A::Error::custom("I-JSON string contains forbidden Unicode scalar"))?;
            if !keys.insert(key.clone()) {
                return Err(A::Error::custom("duplicate JSON object member"));
            }
            let value = map.next_value::<StrictValue>()?;
            values.insert(key, value.0);
        }
        Ok(StrictValue(Value::Object(values)))
    }
}

impl StrictValueVisitor {
    fn number<E>(self, number: Number) -> Result<StrictValue, E>
    where
        E: serde::de::Error,
    {
        validate_i_json_number(&number).map_err(|_| E::custom("non-interoperable JSON number"))?;
        Ok(StrictValue(Value::Number(number)))
    }
}

pub(crate) fn validate_i_json_number(number: &Number) -> Result<(), NonInteroperableIJsonNumber> {
    let interoperable = number
        .as_i64()
        .map(|value| value.unsigned_abs() <= MAX_I_JSON_INTEROPERABLE_INTEGER)
        .or_else(|| {
            number
                .as_u64()
                .map(|value| value <= MAX_I_JSON_INTEROPERABLE_INTEGER)
        })
        .or_else(|| {
            number.as_f64().map(|value| {
                value.is_finite()
                    && (value.fract() != 0.0
                        || value.abs() <= MAX_I_JSON_INTEROPERABLE_INTEGER as f64)
            })
        })
        .unwrap_or(false);

    interoperable
        .then_some(())
        .ok_or(NonInteroperableIJsonNumber)
}

pub(crate) fn validate_i_json_string(value: &str) -> Result<(), NonInteroperableIJsonString> {
    value
        .chars()
        .all(|character| {
            let scalar = character as u32;
            !(0xfdd0..=0xfdef).contains(&scalar)
                && scalar & 0xffff != 0xfffe
                && scalar & 0xffff != 0xffff
        })
        .then_some(())
        .ok_or(NonInteroperableIJsonString)
}

pub(crate) fn validate_i_json_value(value: &Value) -> Result<(), IJsonValidationError> {
    match value {
        Value::Array(values) => {
            for value in values {
                validate_i_json_value(value)?;
            }
        }
        Value::Object(values) => validate_i_json_object(values)?,
        Value::Number(number) => validate_i_json_number(number)
            .map_err(|_| IJsonValidationError::NonInteroperableNumber)?,
        Value::String(value) => validate_i_json_string(value)
            .map_err(|_| IJsonValidationError::NonInteroperableString)?,
        Value::Null | Value::Bool(_) => {}
    }
    Ok(())
}

pub(crate) fn validate_i_json_object(
    values: &Map<String, Value>,
) -> Result<(), IJsonValidationError> {
    for (key, value) in values {
        validate_i_json_string(key).map_err(|_| IJsonValidationError::NonInteroperableString)?;
        validate_i_json_value(value)?;
    }
    Ok(())
}
