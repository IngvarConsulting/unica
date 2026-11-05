//! The existing document numerator's closed property write profile.
use serde_json::{json, Value};

pub const NUMBER_TYPES: &[&str] = &["String", "Number"];
pub const NUMBER_ALLOWED_LENGTHS: &[&str] = &["Variable", "Fixed"];
pub const NUMBER_PERIODICITIES: &[&str] = &["Nonperiodical", "Year", "Quarter", "Month", "Day"];

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DocumentNumeratorChanges {
    pub number_type: Option<String>,
    pub number_length: Option<u32>,
    pub number_allowed_length: Option<String>,
    pub number_periodicity: Option<String>,
    pub check_unique: Option<bool>,
}

impl DocumentNumeratorChanges {
    pub fn values(&self) -> impl Iterator<Item = (&'static str, String)> {
        [
            self.number_type.clone().map(|v| ("NumberType", v)),
            self.number_length.map(|v| ("NumberLength", v.to_string())),
            self.number_allowed_length
                .clone()
                .map(|v| ("NumberAllowedLength", v)),
            self.number_periodicity
                .clone()
                .map(|v| ("NumberPeriodicity", v)),
            self.check_unique.map(|v| ("CheckUnique", v.to_string())),
        ]
        .into_iter()
        .flatten()
    }
}

pub fn values_schema() -> Value {
    json!({"type":"object", "additionalProperties":false, "minProperties":1,
    "properties": {
        "NumberType":{"type":"string", "enum":NUMBER_TYPES},
        "NumberLength":{"type":"integer", "minimum":0, "maximum":u32::MAX},
        "NumberAllowedLength":{"type":"string", "enum":NUMBER_ALLOWED_LENGTHS},
        "NumberPeriodicity":{"type":"string", "enum":NUMBER_PERIODICITIES},
        "CheckUnique":{"type":"boolean"}
    }})
}
