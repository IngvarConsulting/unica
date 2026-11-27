//! Writable picture value of a command group.
use super::*;
use crate::MetaPicture;
use serde_json::{json, Value};

pub(super) const SPEC: MetadataPropertySpec = property(
    "Picture",
    MetaPropertyKey::Picture,
    MetaPropertyValueKind::Picture,
    &[MetadataKind::CommandGroup],
);

pub fn picture_value_schema() -> Value {
    let identifier = crate::contracts::IDENTIFIER_PATTERN.trim_start_matches('^');
    let reference = format!(r"^(StdPicture|CommonPicture)\.{identifier}");
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "Ref": {"type": "string", "pattern": reference},
            "LoadTransparent": {"type": "boolean"}
        },
        "anyOf": [{"enum": [{}]}, {"required": ["Ref"]}],
        "description": "Set a StdPicture or registered CommonPicture reference. Omit LoadTransparent to use the platform default true. Pass {} to clear Picture."
    })
}

pub(super) fn valid_picture_reference(reference: &str) -> bool {
    reference.split_once('.').is_some_and(|(kind, name)| {
        matches!(kind, "StdPicture" | "CommonPicture") && crate::identifier::is_1c_identifier(name)
    })
}

pub(crate) fn parse_picture_value(
    value: &Value,
    field: &str,
) -> Result<Option<MetaPicture>, MetaDiagnostic> {
    let invalid = |part: &str, message: &str| {
        MetaDiagnostic::error(MetaDiagnosticCode::InvalidArguments, message)
            .with_field(&format!("{field}{part}"))
    };
    let object = value
        .as_object()
        .ok_or_else(|| invalid("", "Picture must be an object; use {} to clear it"))?;
    if object.is_empty() {
        return Ok(None);
    }
    for key in object.keys() {
        if !matches!(key.as_str(), "Ref" | "LoadTransparent") {
            return Err(invalid(&format!(".{key}"), "unknown Picture field"));
        }
    }
    let reference = object
        .get("Ref")
        .and_then(Value::as_str)
        .filter(|reference| valid_picture_reference(reference))
        .ok_or_else(|| {
            invalid(
                ".Ref",
                "Picture.Ref must be StdPicture.<Name> or CommonPicture.<Name>",
            )
        })?;
    let load_transparent = object
        .get("LoadTransparent")
        .map(|value| {
            value.as_bool().ok_or_else(|| {
                invalid(
                    ".LoadTransparent",
                    "Picture.LoadTransparent must be a boolean",
                )
            })
        })
        .transpose()?;
    Ok(Some(MetaPicture {
        reference: reference.to_string(),
        load_transparent,
    }))
}
