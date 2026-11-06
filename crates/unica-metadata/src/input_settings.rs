//! Closed, type-dependent input settings for stored metadata values.
use crate::{MetadataType, MetadataTypeVariant};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataInputSetting {
    PasswordMode,
    MultiLine,
    ExtendedEdit,
    Mask,
    MarkNegatives,
    Format,
    EditFormat,
}
impl MetadataInputSetting {
    pub const ALL: &[Self] = &[
        Self::PasswordMode,
        Self::MultiLine,
        Self::ExtendedEdit,
        Self::Mask,
        Self::MarkNegatives,
        Self::Format,
        Self::EditFormat,
    ];
    pub fn names(self) -> (&'static str, &'static str) {
        match self {
            Self::PasswordMode => ("PasswordMode", "passwordMode"),
            Self::MultiLine => ("MultiLine", "multiLine"),
            Self::ExtendedEdit => ("ExtendedEdit", "extendedEdit"),
            Self::Mask => ("Mask", "mask"),
            Self::MarkNegatives => ("MarkNegatives", "markNegatives"),
            Self::Format => ("Format", "format"),
            Self::EditFormat => ("EditFormat", "editFormat"),
        }
    }
    pub fn from_xml_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|setting| setting.names().0 == name)
    }
    pub fn boolean(self) -> bool {
        matches!(
            self,
            Self::PasswordMode | Self::MultiLine | Self::ExtendedEdit | Self::MarkNegatives
        )
    }
    pub fn localized(self) -> bool {
        matches!(self, Self::Format | Self::EditFormat)
    }
    pub fn allows(self, ty: &MetadataType) -> bool {
        match ty.variants.as_slice() {
            [MetadataTypeVariant::String { .. }] => matches!(
                self,
                Self::PasswordMode | Self::MultiLine | Self::ExtendedEdit | Self::Mask
            ),
            [MetadataTypeVariant::Number { .. }] => {
                matches!(self, Self::MarkNegatives | Self::Format | Self::EditFormat)
            }
            _ => false,
        }
    }
    pub fn schema(self) -> Value {
        if self.boolean() {
            json!({"type":"boolean"})
        } else {
            json!({"type":"string"})
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataInputValue {
    Boolean(bool),
    Text(String),
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MetadataInputSettings(pub Vec<(MetadataInputSetting, MetadataInputValue)>);
impl MetadataInputSettings {
    pub fn writes_localized_text(&self) -> bool {
        self.0.iter().any(|(setting, _)| setting.localized())
    }
}

/// The provider has already observed the Type. Only an editable, fully
/// recognized type can publish these new fields; no raw type-name heuristics.
pub fn constrain_input_settings_contract(
    contract: &mut unica_operation_contract::OperationContract,
    observed: Option<&Value>,
) {
    let ty = observed
        .filter(|value| value.get("mutationCapability").and_then(Value::as_str) == Some("editable"))
        .and_then(|value| {
            crate::request::parse_metadata_type(&json!({"variants":value.get("variants")?}), "type")
                .ok()
        });
    let base = contract.schema["properties"]["values"].clone();
    let Some(fields) = base["properties"].as_object() else {
        return;
    };
    let constant = fields.contains_key("PasswordMode");
    if !constant && !fields.contains_key("passwordMode") {
        return;
    }
    let type_key = if constant { "Type" } else { "type" };
    let name = |setting: MetadataInputSetting| {
        if constant {
            setting.names().0
        } else {
            setting.names().1
        }
    };
    let mut current = base.clone();
    let fields = current["properties"]
        .as_object_mut()
        .expect("property object");
    fields.remove(type_key);
    for setting in MetadataInputSetting::ALL {
        if !ty.as_ref().is_some_and(|ty| setting.allows(ty)) {
            fields.remove(name(*setting));
        }
    }
    let mut branches = vec![current];
    for kind in ["string", "number", "other"] {
        let mut branch = base.clone();
        branch["required"] = json!([type_key]);
        let fields = branch["properties"]
            .as_object_mut()
            .expect("property object");
        for setting in MetadataInputSetting::ALL {
            let allowed = match kind {
                "string" => matches!(
                    setting,
                    MetadataInputSetting::PasswordMode
                        | MetadataInputSetting::MultiLine
                        | MetadataInputSetting::ExtendedEdit
                        | MetadataInputSetting::Mask
                ),
                "number" => matches!(
                    setting,
                    MetadataInputSetting::MarkNegatives
                        | MetadataInputSetting::Format
                        | MetadataInputSetting::EditFormat
                ),
                _ => false,
            };
            if !allowed {
                fields.remove(name(*setting));
            }
        }
        if kind != "other" {
            let original = fields[type_key].clone();
            fields.insert(type_key.into(),json!({"allOf":[original,{"properties":{"variants":{"minItems":1,"maxItems":1,"items":{"properties":{"kind":{"const":kind}}}}}}]}));
        }
        branches.push(branch);
    }
    contract.schema["properties"]["values"] = json!({"anyOf":branches});
}
