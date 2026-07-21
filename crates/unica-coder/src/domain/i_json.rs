use serde_json::{Map, Number, Value};

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
