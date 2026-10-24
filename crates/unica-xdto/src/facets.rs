use serde::Serialize;
use serde_json::Value;
use std::cmp::Ordering;

pub const NAMES: [&str; 5] = [
    "maxLength",
    "totalDigits",
    "fractionDigits",
    "minInclusive",
    "maxInclusive",
];

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueFacets {
    #[serde(skip_serializing_if = "Option::is_none")]
    max_length: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_digits: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fraction_digits: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_inclusive: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_inclusive: Option<String>,
}

impl ValueFacets {
    pub fn from_json(value: Option<&Value>) -> Result<Self, &'static str> {
        let Some(value) = value else {
            return Ok(Self::default());
        };
        let object = value.as_object().ok_or("facets must be an object")?;
        if object.keys().any(|key| !NAMES.contains(&key.as_str())) {
            return Err("unknown XDTO value facet");
        }
        let integer = |key| -> Result<Option<u64>, &'static str> {
            object
                .get(key)
                .map(|v| nonnegative_integer(v).ok_or("facet must be a nonnegative integer"))
                .transpose()
        };
        let decimal = |key| -> Result<Option<String>, &'static str> {
            object
                .get(key)
                .map(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .ok_or("decimal bounds must be strings")
                })
                .transpose()
        };
        Self::from_parts(
            integer("maxLength")?,
            integer("totalDigits")?,
            integer("fractionDigits")?,
            decimal("minInclusive")?,
            decimal("maxInclusive")?,
        )
        .map_err(|(_, message)| message)
    }

    /// Facets from independently read values. A refusal names the facet it
    /// belongs to, so a caller can report it at that field.
    pub fn from_parts(
        max_length: Option<u64>,
        total_digits: Option<u64>,
        fraction_digits: Option<u64>,
        min_inclusive: Option<String>,
        max_inclusive: Option<String>,
    ) -> Result<Self, (&'static str, &'static str)> {
        for (key, bound) in [
            ("minInclusive", &min_inclusive),
            ("maxInclusive", &max_inclusive),
        ] {
            if bound
                .as_deref()
                .is_some_and(|text| Decimal::parse(text).is_none())
            {
                return Err((key, "bound must have XML decimal lexical form"));
            }
        }
        if total_digits == Some(0) {
            return Err(("totalDigits", "totalDigits must be positive"));
        }
        if let (Some(total), Some(fraction)) = (total_digits, fraction_digits) {
            if fraction > total {
                return Err(("fractionDigits", "fractionDigits exceeds totalDigits"));
            }
        }
        if let (Some(min), Some(max)) = (&min_inclusive, &max_inclusive) {
            let order = Decimal::parse(min)
                .expect("checked above")
                .cmp(&Decimal::parse(max).expect("checked above"));
            if order == Ordering::Greater {
                return Err(("minInclusive", "minInclusive exceeds maxInclusive"));
            }
        }
        Ok(Self {
            max_length,
            total_digits,
            fraction_digits,
            min_inclusive,
            max_inclusive,
        })
    }

    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    pub fn validate_base(&self, namespace: Option<&str>, local: &str) -> Result<(), &'static str> {
        if self.is_empty() {
            return Ok(());
        }
        if namespace != Some("http://www.w3.org/2001/XMLSchema") {
            return Err("facets require a supported XML Schema primitive base");
        }
        match local {
            "string"
                if self.total_digits.is_none()
                    && self.fraction_digits.is_none()
                    && self.min_inclusive.is_none()
                    && self.max_inclusive.is_none() =>
            {
                Ok(())
            }
            "decimal" if self.max_length.is_none() => Ok(()),
            _ => Err(
                "facets are incompatible with this base; supported profiles are string and decimal",
            ),
        }
    }

    pub fn validate_anonymous_base(
        &self,
        namespace: Option<&str>,
        local: &str,
    ) -> Result<(), &'static str> {
        if namespace != Some("http://www.w3.org/2001/XMLSchema")
            || !matches!(local, "string" | "decimal")
        {
            return Err("anonymous ValueType supports only XML Schema string or decimal bases");
        }
        self.validate_base(namespace, local)
    }

    pub fn attributes(&self) -> Vec<(&'static str, String)> {
        let mut result = Vec::new();
        for (key, value) in [
            ("maxLength", self.max_length),
            ("totalDigits", self.total_digits),
            ("fractionDigits", self.fraction_digits),
        ] {
            if let Some(value) = value {
                result.push((key, value.to_string()));
            }
        }
        for (key, value) in [
            ("minInclusive", &self.min_inclusive),
            ("maxInclusive", &self.max_inclusive),
        ] {
            if let Some(value) = value {
                result.push((key, value.clone()));
            }
        }
        result
    }

    pub fn equivalent(&self, other: &Self) -> bool {
        let same_decimal = |a: &Option<String>, b: &Option<String>| match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => Decimal::parse(a) == Decimal::parse(b),
            _ => false,
        };
        self.max_length == other.max_length
            && self.total_digits == other.total_digits
            && self.fraction_digits == other.fraction_digits
            && same_decimal(&self.min_inclusive, &other.min_inclusive)
            && same_decimal(&self.max_inclusive, &other.max_inclusive)
    }
}

pub fn nonnegative_integer(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value.as_f64().and_then(|number| {
            (number >= 0.0 && number < u64::MAX as f64 && number.fract() == 0.0)
                .then_some(number as u64)
        })
    })
}

// Compare arbitrary precision XML decimal values without floating point conversion.
#[derive(Debug, PartialEq, Eq)]
struct Decimal<'a> {
    negative: bool,
    integer: &'a str,
    fraction: &'a str,
}
impl<'a> Decimal<'a> {
    fn parse(text: &'a str) -> Option<Self> {
        let negative = text.starts_with('-');
        let unsigned = text.strip_prefix(['-', '+']).unwrap_or(text);
        let (integer, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
        if integer.is_empty() && fraction.is_empty() {
            return None;
        }
        if !integer
            .bytes()
            .chain(fraction.bytes())
            .all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let integer = integer.trim_start_matches('0');
        let fraction = fraction.trim_end_matches('0');
        Some(Self {
            negative: negative && !(integer.is_empty() && fraction.is_empty()),
            integer,
            fraction,
        })
    }
    fn cmp(&self, other: &Self) -> Ordering {
        match self.negative.cmp(&other.negative) {
            Ordering::Equal => {
                let magnitude = self
                    .integer
                    .len()
                    .cmp(&other.integer.len())
                    .then_with(|| self.integer.cmp(other.integer))
                    .then_with(|| self.fraction.cmp(other.fraction));
                if self.negative {
                    magnitude.reverse()
                } else {
                    magnitude
                }
            }
            sign => sign.reverse(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn decimal_bounds_remain_exact_beyond_float_precision() {
        assert!(ValueFacets::from_json(Some(
            &json!({"minInclusive":"9007199254740993", "maxInclusive":"9007199254740992"})
        ))
        .is_err());
        for (a, b) in [("-0.00", "0"), ("+001.00", "1"), (".10", "0.1")] {
            assert_eq!(Decimal::parse(a), Decimal::parse(b));
        }
        for invalid in ["", ".", "1e2", "NaN", " 1", "1 ", "1.2.3", "--1"] {
            assert!(Decimal::parse(invalid).is_none());
        }
        assert_eq!(
            Decimal::parse("-2")
                .unwrap()
                .cmp(&Decimal::parse("-1").unwrap()),
            Ordering::Less
        );
        assert_eq!(
            Decimal::parse("0.11")
                .unwrap()
                .cmp(&Decimal::parse("0.2").unwrap()),
            Ordering::Less
        );
    }
    #[test]
    fn independent_facets_and_invalid_combinations() {
        for value in [
            json!({}),
            json!({"maxLength":0}),
            json!({"totalDigits":1}),
            json!({"fractionDigits":2}),
            json!({"minInclusive":"0"}),
        ] {
            assert!(ValueFacets::from_json(Some(&value)).is_ok());
        }
        for value in [
            json!({"pattern":"x"}),
            json!({"maxLength":-1}),
            json!({"maxLength":1.5}),
            json!({"totalDigits":0}),
            json!({"totalDigits":2,"fractionDigits":3}),
            json!({"minInclusive":0}),
        ] {
            assert!(ValueFacets::from_json(Some(&value)).is_err());
        }
        let facet = ValueFacets::from_json(Some(&json!({"maxLength":10}))).unwrap();
        assert!(facet
            .validate_base(Some("http://www.w3.org/2001/XMLSchema"), "string")
            .is_ok());
        assert!(facet
            .validate_base(Some("http://www.w3.org/2001/XMLSchema"), "decimal")
            .is_err());
    }
}
