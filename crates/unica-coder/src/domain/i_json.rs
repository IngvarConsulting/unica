use serde_json::Number;

const MAX_I_JSON_INTEROPERABLE_INTEGER: u64 = (1 << 53) - 1;

#[derive(Clone, Copy, Debug)]
pub(crate) struct NonInteroperableIJsonNumber;

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
