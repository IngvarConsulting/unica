use crate::domain::source_target::ResolvedTarget;
use crate::domain::subsystem::SubsystemAddress;
use serde::Serialize;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfigurationSupportState {
    NotSupported,
    Extension,
    Removed,
    Supported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationSupportData {
    pub state: ConfigurationSupportState,
    pub editing_enabled: Option<bool>,
    pub objects: Option<SupportCounts>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportCounts {
    pub locked: u64,
    pub editable: u64,
    pub removed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ObjectSupportState {
    NotSupported,
    RemovedFromSupport,
    ConfigurationReadOnly,
    Locked,
    EditableWithSupport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectSupportData {
    pub state: ObjectSupportState,
    pub direct_edit_safe: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSubsystemTarget {
    pub source_set: String,
    pub address: SubsystemAddress,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportReadErrorCode {
    ProviderUnavailable,
    TargetUnsupported,
    EvidenceUnavailable,
    StateUnreadable,
    StateInvalid,
}

impl SupportReadErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProviderUnavailable => "provider_unavailable",
            Self::TargetUnsupported => "target_unsupported",
            Self::EvidenceUnavailable => "evidence_unavailable",
            Self::StateUnreadable => "state_unreadable",
            Self::StateInvalid => "state_invalid",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportReadError {
    pub code: SupportReadErrorCode,
    pub message: String,
}

impl SupportReadError {
    pub fn new(code: SupportReadErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for SupportReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for SupportReadError {}

pub trait SupportStateReader: Send + Sync {
    fn configuration_support(
        &self,
        target: &ResolvedTarget,
    ) -> Result<ConfigurationSupportData, SupportReadError>;

    fn object_support(
        &self,
        target: &ResolvedTarget,
    ) -> Result<ObjectSupportData, SupportReadError>;

    fn subsystem_support(
        &self,
        target: &ResolvedSubsystemTarget,
    ) -> Result<ObjectSupportData, SupportReadError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::source_target::{ResolvedTarget, TargetKind};
    use serde_json::json;

    struct UnavailableReader;

    impl SupportStateReader for UnavailableReader {
        fn configuration_support(
            &self,
            _target: &ResolvedTarget,
        ) -> Result<ConfigurationSupportData, SupportReadError> {
            Err(SupportReadError::new(
                SupportReadErrorCode::ProviderUnavailable,
                "support-state provider is unavailable",
            ))
        }

        fn object_support(
            &self,
            _target: &ResolvedTarget,
        ) -> Result<ObjectSupportData, SupportReadError> {
            Err(SupportReadError::new(
                SupportReadErrorCode::ProviderUnavailable,
                "support-state provider is unavailable",
            ))
        }

        fn subsystem_support(
            &self,
            _target: &ResolvedSubsystemTarget,
        ) -> Result<ObjectSupportData, SupportReadError> {
            Err(SupportReadError::new(
                SupportReadErrorCode::ProviderUnavailable,
                "support-state provider is unavailable",
            ))
        }
    }

    fn object_target() -> ResolvedTarget {
        ResolvedTarget {
            source_set: "main".to_string(),
            metadata_path: None,
            target_kind: TargetKind::MetadataObject,
        }
    }

    #[test]
    fn support_state_wire_values_preserve_the_reader_contract() {
        let configuration = ConfigurationSupportData {
            state: ConfigurationSupportState::Supported,
            editing_enabled: Some(true),
            objects: Some(SupportCounts {
                locked: 1,
                editable: 2,
                removed: 3,
            }),
        };
        assert_eq!(
            serde_json::to_value(configuration).unwrap(),
            json!({
                "state": "supported",
                "editingEnabled": true,
                "objects": {"locked": 1, "editable": 2, "removed": 3}
            })
        );

        let object_states = [
            (ObjectSupportState::NotSupported, "notSupported", None),
            (
                ObjectSupportState::RemovedFromSupport,
                "removedFromSupport",
                Some(true),
            ),
            (
                ObjectSupportState::ConfigurationReadOnly,
                "configurationReadOnly",
                Some(false),
            ),
            (ObjectSupportState::Locked, "locked", Some(false)),
            (
                ObjectSupportState::EditableWithSupport,
                "editableWithSupport",
                Some(true),
            ),
        ];
        for (state, expected_state, direct_edit_safe) in object_states {
            assert_eq!(
                serde_json::to_value(ObjectSupportData {
                    state,
                    direct_edit_safe,
                })
                .unwrap(),
                json!({
                    "state": expected_state,
                    "directEditSafe": direct_edit_safe,
                })
            );
        }
    }

    #[test]
    fn provider_unavailable_is_an_error_not_not_supported() {
        let error = UnavailableReader
            .object_support(&object_target())
            .expect_err("an unavailable provider must not fabricate support data");

        assert_eq!(error.code, SupportReadErrorCode::ProviderUnavailable);
        assert_eq!(
            error.to_string(),
            "provider_unavailable: support-state provider is unavailable"
        );
    }
}
