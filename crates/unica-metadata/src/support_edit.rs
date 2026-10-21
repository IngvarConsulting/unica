//! Existing typed support facts.
use serde_json::{Map, Value};
/// Typed answer of `unica.support.edit` (ADR-0023). The prose said what changed
/// and what to do next; the data says only what changed, and `applied: false`
/// with a `reason` is how "nothing to switch" is now reported.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportEditData {
    /// `capability` or `objectRule`.
    pub action: String,
    pub applied: bool,
    /// Why nothing was applied; `null` when the edit went through.
    pub reason: Option<String>,
    /// For `capability`: whether configuration editing is now enabled.
    pub editing_enabled: Option<bool>,
    /// For `objectRule`: the object and the rule it now carries.
    pub object_uuid: Option<String>,
    pub rule: Option<String>,
    /// Support records rewritten in `ParentConfigurations.bin`.
    pub records_changed: usize,
    /// Per-object rules reset to locked by a capability switch.
    pub object_rules_reset: usize,
    pub mutation: unica_execution_contract::mutation::MutationData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportCapability {
    On,
    Off,
}

impl SupportCapability {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "on" => Some(Self::On),
            "off" => Some(Self::Off),
            _ => None,
        }
    }

    pub fn target_flag(self) -> u8 {
        match self {
            Self::On => 0,
            Self::Off => 1,
        }
    }

    pub fn enabled(self) -> bool {
        matches!(self, Self::On)
    }

    pub fn argument_value(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Off => "off",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportObjectRule {
    Locked,
    Editable,
    OffSupport,
}

impl SupportObjectRule {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "locked" => Some(Self::Locked),
            "editable" => Some(Self::Editable),
            "off-support" => Some(Self::OffSupport),
            _ => None,
        }
    }

    pub fn flag(self) -> u8 {
        match self {
            Self::Locked => 0,
            Self::Editable => 1,
            Self::OffSupport => 2,
        }
    }

    pub fn state_text(self) -> &'static str {
        match self {
            Self::Locked => "на замке (правка запрещена)",
            Self::Editable => {
                "редактируется с сохранением поддержки (объект продолжит получать обновления вендора — возможны конфликты при обновлении)"
            }
            Self::OffSupport => "снят с поддержки (обновления вендора по этому объекту прекращаются)",
        }
    }
}

pub enum SupportEditAction {
    Capability(SupportCapability),
    Set(SupportObjectRule),
}

pub fn support_edit_action(args: &Map<String, Value>) -> Result<SupportEditAction, String> {
    let capability = string_arg(args, &["Capability", "capability"]);
    let set = string_arg(args, &["Set", "set"]);
    match (capability, set) {
        (Some(_), Some(_)) | (None, None) => Err(
            "Укажите ровно одно: Capability=on|off ЛИБО Set=editable|off-support|locked"
                .to_string(),
        ),
        (Some(value), None) => SupportCapability::parse(&value)
            .map(SupportEditAction::Capability)
            .ok_or_else(|| "Capability must be one of: on, off".to_string()),
        (None, Some(value)) => SupportObjectRule::parse(&value)
            .map(SupportEditAction::Set)
            .ok_or_else(|| "Set must be one of: editable, off-support, locked".to_string()),
    }
}

pub fn support_edit_reads_uuid_dependency(args: &Map<String, Value>) -> bool {
    matches!(support_edit_action(args), Ok(SupportEditAction::Set(_)))
}

fn string_arg(args: &Map<String, Value>, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))
        .map(ToOwned::to_owned)
}
