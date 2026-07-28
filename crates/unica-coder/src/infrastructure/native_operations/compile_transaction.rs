//! Host-side staging facade for BSL/application artifacts.
//!
//! It deliberately owns no locks, stages or rollback implementation. Commit
//! delegates to the adapter's format-neutral [`ArtifactWritePort`], so host
//! artifacts contend with Platform XML writers on the same lock registry.

use crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};
use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_format_core::{
    commands::{MutationMode, WriterLifecycle},
    ports::{ArtifactWriteIntent, ArtifactWritePort, ArtifactWriteRequest, OperationCancellation},
};

#[derive(Debug, Default)]
pub(crate) struct CompileTransaction {
    replacement: Option<PlannedReplacement>,
    exact_guards: BTreeMap<PathBuf, Vec<u8>>,
    absence_guards: BTreeSet<PathBuf>,
}

#[derive(Debug)]
struct PlannedReplacement {
    path: PathBuf,
    before: Vec<u8>,
    after: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommitReport {
    pub(crate) updated_paths: Vec<PathBuf>,
}

impl CompileTransaction {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn protects_path(&self, path: &Path) -> Result<bool, String> {
        Ok(self
            .replacement
            .as_ref()
            .is_some_and(|replacement| replacement.path == path))
    }

    pub(crate) fn replace_bytes(
        &mut self,
        path: impl Into<PathBuf>,
        expected_preimage: impl AsRef<[u8]>,
        replacement: impl Into<Vec<u8>>,
    ) -> Result<(), String> {
        if self.replacement.is_some() {
            return Err("artifact publication accepts exactly one replacement".to_string());
        }
        let path = path.into();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
            return Err(format!(
                "artifact publication target must be a regular non-link file: {}",
                path.display()
            ));
        }
        let before = fs::read(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        if before != expected_preimage.as_ref() {
            return Err(format!(
                "artifact publication target changed while planning: {}",
                path.display()
            ));
        }
        self.replacement = Some(PlannedReplacement {
            path,
            before,
            after: replacement.into(),
        });
        Ok(())
    }

    pub(crate) fn guard_or_verify_exact_preimage(
        &mut self,
        path: impl Into<PathBuf>,
        expected_preimage: impl AsRef<[u8]>,
    ) -> Result<(), String> {
        let path = path.into();
        let expected = expected_preimage.as_ref().to_vec();
        let actual = fs::read(&path)
            .map_err(|error| format!("failed to read guard {}: {error}", path.display()))?;
        if actual != expected {
            return Err(format!("guard changed while planning: {}", path.display()));
        }
        match self.exact_guards.get(&path) {
            Some(existing) if existing != &expected => {
                Err(format!("conflicting guard preimages: {}", path.display()))
            }
            _ => {
                self.exact_guards.insert(path, expected);
                Ok(())
            }
        }
    }

    pub(crate) fn guard_path_absent(&mut self, path: impl Into<PathBuf>) -> Result<(), String> {
        let path = path.into();
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == ErrorKind::NotFound => {
                self.absence_guards.insert(path);
                Ok(())
            }
            Ok(_) => Err(format!(
                "artifact transaction absence guard was violated while planning: {}",
                path.display()
            )),
            Err(error) => Err(format!("failed to inspect {}: {error}", path.display())),
        }
    }

    pub(crate) fn commit(self) -> Result<CommitReport, String> {
        self.commit_cancellable(&OperationCancellation::new())
    }

    pub(crate) fn commit_cancellable(
        self,
        cancellation: &OperationCancellation,
    ) -> Result<CommitReport, String> {
        let updated_paths = self
            .replacement
            .iter()
            .filter(|replacement| replacement.before != replacement.after)
            .map(|replacement| replacement.path.clone())
            .collect::<Vec<_>>();
        let replacement = self
            .replacement
            .map(|replacement| (replacement.path, replacement.before, replacement.after));
        let factory = PlatformXmlAdapterFactory::new();
        let session = factory
            .capture_artifact_write_session(
                replacement,
                self.exact_guards.into_iter().collect(),
                self.absence_guards.into_iter().collect(),
                Vec::new(),
            )
            .map_err(|error| error.message)?;
        let request = ArtifactWriteRequest::new(
            session,
            None,
            ArtifactWriteIntent::SemanticSourceTransaction,
            None,
            MutationMode::Apply,
            cancellation.clone(),
        );
        commit_through_port(
            factory.operational_registration().artifact_write(),
            &request,
        )?;
        Ok(CommitReport { updated_paths })
    }
}

fn commit_through_port(
    port: &dyn ArtifactWritePort,
    request: &ArtifactWriteRequest,
) -> Result<(), String> {
    let result = port.write(request).map_err(|error| error.message)?;
    match result.lifecycle() {
        WriterLifecycle::Applied => Ok(()),
        WriterLifecycle::Cancelled(_) => Err("artifact publication cancelled".to_string()),
        WriterLifecycle::Rejected(_) => Err("artifact publication failed".to_string()),
        WriterLifecycle::Previewed => {
            Err("artifact publication unexpectedly previewed".to_string())
        }
    }
}
