//! One invocation argument object, retaining numeric source without changing
//! serde_json's global number model. The wire contains exactly one JSON object.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{value::RawValue, Map, Value};

#[derive(Clone)]
pub struct InvocationArguments {
    projection: Map<String, Value>,
    raw: Option<String>,
}

impl From<Map<String, Value>> for InvocationArguments {
    fn from(projection: Map<String, Value>) -> Self {
        Self {
            projection,
            raw: None,
        }
    }
}

impl InvocationArguments {
    pub fn projection(&self) -> &Map<String, Value> {
        &self.projection
    }
    pub fn raw(&self) -> Option<&str> {
        self.raw.as_deref()
    }
    pub fn with_raw(self, raw: Option<String>) -> Result<Self, String> {
        let Some(raw) = raw else { return Ok(self) };
        let projection: Map<String, Value> =
            serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        if projection != self.projection {
            return Err("raw invocation arguments do not match their JSON projection".into());
        }
        Ok(Self {
            projection,
            raw: Some(raw),
        })
    }
}

impl Serialize for InvocationArguments {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match &self.raw {
            Some(raw) => RawValue::from_string(raw.clone())
                .map_err(serde::ser::Error::custom)?
                .serialize(serializer),
            None => self.projection.serialize(serializer),
        }
    }
}
// Tagged serde envelopes buffer objects before invoking this deserializer.
// The authenticated protocol decoder restores the original field from its raw
// frame before deriving identity. Never infer original digits from this view.
impl<'de> Deserialize<'de> for InvocationArguments {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let projection = Map::<String, Value>::deserialize(deserializer)?;
        Ok(Self {
            projection,
            raw: None,
        })
    }
}
impl PartialEq for InvocationArguments {
    fn eq(&self, other: &Self) -> bool {
        // In-memory callers need not supply a raw witness. Compare the actual
        // JSON each instance would transmit, retaining exact numeric tokens.
        canonical_json(
            self.raw
                .as_deref()
                .unwrap_or(&serde_json::to_string(&self.projection).unwrap()),
        ) == canonical_json(
            other
                .raw
                .as_deref()
                .unwrap_or(&serde_json::to_string(&other.projection).unwrap()),
        )
    }
}

pub fn canonical_json(raw: &str) -> String {
    fn canonical(raw: &RawValue) -> String {
        let text = raw.get();
        match text.as_bytes().first() {
            Some(b'{') => {
                let entries: std::collections::BTreeMap<String, &RawValue> =
                    serde_json::from_str(text).expect("validated raw object");
                format!(
                    "{{{}}}",
                    entries
                        .iter()
                        .map(|(k, v)| format!(
                            "{}:{}",
                            serde_json::to_string(k).unwrap(),
                            canonical(v)
                        ))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }
            Some(b'[') => {
                let items: Vec<&RawValue> =
                    serde_json::from_str(text).expect("validated raw array");
                format!(
                    "[{}]",
                    items
                        .iter()
                        .map(|v| canonical(v))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }
            Some(b'"') => {
                serde_json::to_string(&serde_json::from_str::<String>(text).unwrap()).unwrap()
            }
            _ => text.to_owned(),
        }
    }
    canonical(serde_json::from_str::<&RawValue>(raw).expect("validated invocation arguments"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exact_arguments_serialize_once_without_changing_json_types() {
        let raw = r#"{"n":9223372036854775807.0,"object":{"$serde_json::private::Number":"1.0"},"text":"value"}"#;
        let args =
            InvocationArguments::from(serde_json::from_str::<Map<String, Value>>(raw).unwrap())
                .with_raw(Some(raw.to_owned()))
                .unwrap();
        let encoded = serde_json::to_string(&args).unwrap();
        assert_eq!(encoded, raw);
        let projection: Value = serde_json::from_str(&encoded).unwrap();
        assert!(projection["object"].is_object());
        assert!(
            InvocationArguments::from(json!({"n":1}).as_object().unwrap().clone())
                .with_raw(Some("{\"n\":2}".into()))
                .is_err()
        );

        let text = "\\\"".repeat(512 * 1024);
        let raw = serde_json::to_string(&json!({"text":text})).unwrap();
        let args =
            InvocationArguments::from(serde_json::from_str::<Map<String, Value>>(&raw).unwrap())
                .with_raw(Some(raw.clone()))
                .unwrap();
        assert_eq!(
            serde_json::to_vec(&args).unwrap(),
            raw.as_bytes(),
            "escaped payload must not be duplicated or escaped twice"
        );
    }

    #[test]
    fn raw_identity_ignores_object_order_but_retains_numeric_evidence() {
        assert_eq!(
            canonical_json(r#"{"b": [1,2], "a":"\u0061"}"#),
            canonical_json(r#"{"a":"a","b":[1,2]}"#)
        );
        assert_ne!(
            canonical_json(r#"{"n":9223372036854775807.0}"#),
            canonical_json(r#"{"n":9223372036854775808.0}"#)
        );
    }
}
