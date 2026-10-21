use std::path::PathBuf;
use std::sync::Arc;

/// Stable byte form used in an exact delivery identity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DeliveryFormIdentity {
    Archive,
    ZipArchive,
    File,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DeliveryWorkKey {
    artifact: String,
    version: String,
    target: String,
    sha256: String,
    form: DeliveryFormIdentity,
}

impl DeliveryWorkKey {
    pub fn new(
        artifact: impl Into<String>,
        version: impl Into<String>,
        target: impl Into<String>,
        sha256: impl Into<String>,
        form: DeliveryFormIdentity,
    ) -> Result<Self, DeliveryFailure> {
        let key = Self {
            artifact: artifact.into(),
            version: version.into(),
            target: target.into(),
            sha256: sha256.into(),
            form,
        };
        if key.artifact.is_empty()
            || key.version.is_empty()
            || key.target.is_empty()
            || key.sha256.len() != 64
            || !key
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(DeliveryFailure::new(
                DeliveryFailureClass::Configuration,
                "invalid exact delivery identity",
            ));
        }
        Ok(key)
    }

    pub fn artifact(&self) -> &str {
        &self.artifact
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactReady {
    identity: DeliveryWorkKey,
    install_root: PathBuf,
}

impl ArtifactReady {
    pub fn new(identity: DeliveryWorkKey, install_root: PathBuf) -> Result<Self, DeliveryFailure> {
        if !install_root.is_absolute() {
            return Err(DeliveryFailure::new(
                DeliveryFailureClass::Internal,
                "artifact-ready installation root is not absolute",
            ));
        }
        Ok(Self {
            identity,
            install_root,
        })
    }

    pub fn identity(&self) -> &DeliveryWorkKey {
        &self.identity
    }

    pub fn install_root(&self) -> &std::path::Path {
        &self.install_root
    }
}

#[allow(dead_code)] // Closed classes reserved for real Task22 producer adapters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryFailureClass {
    Network,
    /// ОС не доверяет цепочке сертификатов узла выпуска: чаще всего это
    /// перехват TLS средством защиты или шлюзом. Повтор не поможет.
    UntrustedCertificate,
    Timeout,
    Disk,
    Checksum,
    Configuration,
    Internal,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryFailure {
    class: DeliveryFailureClass,
    /// Retained only for the byte-compatible V12 projection. Canonical V13
    /// callers project the closed class and never runtime prose.
    legacy_diagnostic: String,
}

#[derive(Clone, Debug, Default)]
pub enum EngineDeliveryState {
    #[default]
    NotRequired,
    Ready(Arc<ArtifactReady>),
    Working {
        artifact: String,
        received: u64,
        total: Option<u64>,
        poll_interval_ms: Option<u64>,
    },
    Failed {
        artifact: String,
        failure: Arc<DeliveryFailure>,
    },
}

impl DeliveryFailure {
    pub fn new(class: DeliveryFailureClass, legacy_diagnostic: impl Into<String>) -> Self {
        Self {
            class,
            legacy_diagnostic: legacy_diagnostic.into(),
        }
    }

    #[allow(dead_code)] // Canonical V13 delivery projection consumes the closed class at cutover.
    pub fn class(&self) -> DeliveryFailureClass {
        self.class
    }

    pub fn legacy_diagnostic(&self) -> &str {
        &self.legacy_diagnostic
    }
}
