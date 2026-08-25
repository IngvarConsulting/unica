use crate::application::v13::view::ViewError;
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::support_state::{ConfigurationSupportData, ConfigurationSupportState};
use crate::infrastructure::native_operations::cf::parse_cf_info_xml;
use crate::infrastructure::platform::filesystem::RetainedDirectoryCapability;
use crate::infrastructure::source_revision::SourceRevisionService;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

const MAX_CONFIGURATION_BYTES: usize = 8 * 1024 * 1024;

/// One actor-issued read authority for one admitted source set. Hidden v0.13
/// reads are descriptor-relative to this retained directory and revisions come
/// from the paired actor-owned service.
pub(crate) struct ProviderReadAuthority {
    source_set: String,
    source_set_identity: String,
    root: Arc<RetainedDirectoryCapability>,
    revisions: Arc<SourceRevisionService>,
}

impl ProviderReadAuthority {
    pub(crate) fn new(
        source_set: impl Into<String>,
        source_set_identity: impl Into<String>,
        root: Arc<RetainedDirectoryCapability>,
        revisions: Arc<SourceRevisionService>,
    ) -> Self {
        Self {
            source_set: source_set.into(),
            source_set_identity: source_set_identity.into(),
            root,
            revisions,
        }
    }

    pub(crate) fn source_set(&self) -> &str {
        &self.source_set
    }

    pub(crate) fn source_set_identity(&self) -> &str {
        &self.source_set_identity
    }

    pub(crate) fn root_path(&self) -> &Path {
        self.root.path()
    }

    pub(crate) fn exact_revision(
        &self,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<String, ViewError> {
        self.root
            .validate_named_identity()
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?;
        self.revisions
            .snapshot(deadline, cancellation)
            .map(|revision| {
                format!(
                    "{}:{}:{}",
                    revision.algorithm, revision.generation, revision.digest
                )
            })
            .map_err(|error| ViewError::new("provider_unavailable", error))
    }

    pub(crate) fn read_relative(
        &self,
        relative: &Path,
        max_bytes: usize,
    ) -> Result<Vec<u8>, ViewError> {
        self.root
            .read_relative_regular_bounded(relative, max_bytes)
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))
    }

    pub(crate) fn read_optional_relative(
        &self,
        relative: &Path,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, ViewError> {
        match self.root.read_relative_regular_bounded(relative, max_bytes) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(ViewError::new("provider_unavailable", error.to_string())),
        }
    }

    pub(crate) fn configuration_payload(&self) -> Result<Value, ViewError> {
        let bytes = self.read_relative(Path::new("Configuration.xml"), MAX_CONFIGURATION_BYTES)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| {
            ViewError::new("provider_unavailable", "Configuration.xml is not UTF-8")
        })?;
        let parsed = parse_cf_info_xml(
            text,
            ConfigurationSupportData {
                state: ConfigurationSupportState::NotSupported,
                editing_enabled: None,
                objects: None,
            },
            None,
        )
        .map_err(|error| ViewError::new("provider_unavailable", error))?;
        let mut payload = serde_json::to_value(parsed.data)
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?
            .as_object()
            .cloned()
            .ok_or_else(|| {
                ViewError::new(
                    "provider_unavailable",
                    "configuration parser returned a non-object payload",
                )
            })?;
        payload.insert(
            "registeredObjects".to_string(),
            serde_json::to_value(parsed.registered_objects)
                .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?,
        );
        Ok(Value::Object(payload))
    }
}
