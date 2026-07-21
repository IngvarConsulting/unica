use super::schema::{
    is_i_json_lf_text, is_i_json_single_line_text, is_normalized_utc_instant, string_schema,
    I_JSON_LF_TEXT_FORMAT, I_JSON_SINGLE_LINE_TEXT_FORMAT, NORMALIZED_UTC_INSTANT_FORMAT,
};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::de::Error as _;
use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

const MAX_I_JSON_INTEROPERABLE_INTEGER: u64 = (1 << 53) - 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScalarError {
    kind: &'static str,
    reason: &'static str,
}

impl fmt::Display for ScalarError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid {}: {}", self.kind, self.reason)
    }
}

impl std::error::Error for ScalarError {}

fn validate_text(
    value: &str,
    kind: &'static str,
    min_length: usize,
    max_length: usize,
    allow_line_feed: bool,
) -> Result<String, ScalarError> {
    let length = value.chars().count();
    if !(min_length..=max_length).contains(&length) {
        return Err(ScalarError {
            kind,
            reason: "has an invalid Unicode scalar length",
        });
    }
    let characters_are_valid = if allow_line_feed {
        is_i_json_lf_text(value)
    } else {
        is_i_json_single_line_text(value)
    };
    if !characters_are_valid {
        return Err(ScalarError {
            kind,
            reason: "contains a forbidden I-JSON or control Unicode scalar",
        });
    }
    Ok(value.to_owned())
}

macro_rules! bounded_text {
    ($name:ident, $kind:literal, $min:literal, $max:literal, $allow_line_feed:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
        #[serde(transparent)]
        pub(crate) struct $name(String);

        impl $name {
            pub(crate) fn parse(value: &str) -> Result<Self, ScalarError> {
                validate_text(value, $kind, $min, $max, $allow_line_feed).map(Self)
            }

            pub(crate) fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = ScalarError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::parse(&value).map_err(D::Error::custom)
            }
        }

        impl JsonSchema for $name {
            fn inline_schema() -> bool {
                true
            }

            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                let format = if $allow_line_feed {
                    I_JSON_LF_TEXT_FORMAT
                } else {
                    I_JSON_SINGLE_LINE_TEXT_FORMAT
                };
                string_schema($min, $max, None, Some(format))
            }
        }
    };
}

bounded_text!(Name, "name", 1, 256, false);
bounded_text!(Summary, "summary", 1, 2048, false);
bounded_text!(Narrative, "narrative", 1, 4096, true);
bounded_text!(TaskSummary, "task summary", 1, 4096, true);
bounded_text!(Reason, "reason", 1, 4096, true);
bounded_text!(Rationale, "rationale", 1, 4096, true);
bounded_text!(Comment, "comment", 1, 4096, true);
bounded_text!(DisplayPath, "display path", 1, 4096, false);
bounded_text!(PropertyPath, "property path", 1, 2048, false);
bounded_text!(Diagnostic, "redacted diagnostic", 0, 8192, true);
bounded_text!(OriginalProjectCwd, "original project cwd", 1, 4096, false);
bounded_text!(LocalProfileName, "local profile name", 1, 256, false);
bounded_text!(RepositoryVersion, "repository version", 1, 128, false);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(transparent)]
pub(crate) struct PositiveGeneration(u64);

impl PositiveGeneration {
    pub(crate) fn new(value: u64) -> Result<Self, ScalarError> {
        (1..=MAX_I_JSON_INTEROPERABLE_INTEGER)
            .contains(&value)
            .then_some(Self(value))
            .ok_or(ScalarError {
                kind: "positive generation",
                reason: "must be a positive I-JSON safe integer",
            })
    }

    pub(crate) const fn get(self) -> u64 {
        self.0
    }
}

impl<'de> serde::Deserialize<'de> for PositiveGeneration {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::new(u64::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

impl JsonSchema for PositiveGeneration {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "PositiveGeneration".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "integer",
            "minimum": 1,
            "maximum": MAX_I_JSON_INTEROPERABLE_INTEGER,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(transparent)]
pub(crate) struct BoundedVec<T, const MAX: usize>(Vec<T>);

impl<T, const MAX: usize> BoundedVec<T, MAX> {
    pub(crate) fn new(values: Vec<T>) -> Result<Self, ScalarError> {
        (values.len() <= MAX)
            .then_some(Self(values))
            .ok_or(ScalarError {
                kind: "bounded collection",
                reason: "exceeds its maximum item count",
            })
    }

    pub(crate) fn as_slice(&self) -> &[T] {
        &self.0
    }
}

impl<'de, T, const MAX: usize> serde::Deserialize<'de> for BoundedVec<T, MAX>
where
    T: serde::Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::new(Vec::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

impl<T, const MAX: usize> JsonSchema for BoundedVec<T, MAX>
where
    T: JsonSchema,
{
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        format!("BoundedVec{MAX}Of{}", T::schema_name()).into()
    }

    fn schema_id() -> Cow<'static, str> {
        format!("BoundedVec<{MAX},{}>", T::schema_id()).into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "maxItems": MAX,
            "items": generator.subschema_for::<T>(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(transparent)]
pub(crate) struct NormalizedUtcInstant(String);

impl NormalizedUtcInstant {
    pub(crate) fn parse(value: &str) -> Result<Self, ScalarError> {
        is_normalized_utc_instant(value)
            .then(|| Self(value.to_owned()))
            .ok_or(ScalarError {
                kind: "normalized UTC instant",
                reason: "must be a canonical, calendar-valid uppercase UTC RFC 3339 instant",
            })
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NormalizedUtcInstant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for NormalizedUtcInstant {
    type Err = ScalarError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl<'de> serde::Deserialize<'de> for NormalizedUtcInstant {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(D::Error::custom)
    }
}

impl JsonSchema for NormalizedUtcInstant {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "NormalizedUtcInstant".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        string_schema(
            20,
            30,
            Some(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.(?:\d{0,8}[1-9]))?Z$"),
            Some(NORMALIZED_UTC_INSTANT_FORMAT),
        )
    }
}
