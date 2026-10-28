//! Supplier extraction for checked support restoration. All mutable runner
//! state belongs to one temporary directory outside sources and workspace cache.
use std::path::{Component, Path, PathBuf};

use unica_address::source_target::MetadataAddress;
use unica_metadata_designer::support_restore::{
    read_restore_object_image, RestoreObjectImage, RestoreObjectRead, RestoreSupplierRead,
};
use unica_process::runner::{ProcessCommand, ProcessRunner, SystemProcessRunner};
use unica_source_contract::proposal::{
    ApplyPlanError, ApplyPlanErrorKind, ApplyStagingError, ApplyStagingErrorKind,
};
use unica_source_contract::read::{SourceDirectory, SourceEntry};
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::CancellationToken;
use unica_workspace::paths::normalize_path_identity;

use super::v13_infobase_exports::BundledRunner;
use super::v13_workspace_bootstrap::read_yaml_config;

pub(super) struct SupplierReader<'a> {
    runner: BundledRunner,
    context: &'a WorkspaceContext,
    source_root: &'a Path,
    cancellation: &'a CancellationToken,
    process: &'a dyn ProcessRunner,
}

impl<'a> SupplierReader<'a> {
    pub(super) fn new(
        runner: BundledRunner,
        context: &'a WorkspaceContext,
        source_root: &'a Path,
        cancellation: &'a CancellationToken,
    ) -> Self {
        Self {
            runner,
            context,
            source_root,
            cancellation,
            process: &SystemProcessRunner,
        }
    }

    fn convert(
        &self,
        payload: &[u8],
        target: &MetadataAddress,
        format: &str,
        root: &Path,
    ) -> Result<RestoreObjectImage, ApplyPlanError> {
        checkpoint(self.cancellation).map_err(stage_error)?;
        let project = root.join("project");
        std::fs::create_dir(&project).map_err(io_error)?;
        std::fs::create_dir(project.join("seed")).map_err(io_error)?;
        let config = project.join("v8project.yaml");
        std::fs::write(&config, "format: DESIGNER\nworkPath: work\nsource-set: [{name: main, type: CONFIGURATION, path: seed}]\n").map_err(io_error)?;
        // Only platform selection is inherited. In particular no infobase,
        // hooks, sources, workPath, output or provider selectors can escape
        // from the working project into the private conversion context.
        let mut platform = serde_yaml::Mapping::new();
        for name in ["v8project.yaml", "v8project.local.yaml"] {
            if let Some(config) =
                read_yaml_config(&self.context.workspace_root, name).map_err(|_| {
                    source_error("cannot read platform selection for supplier conversion")
                })?
            {
                if let Some(values) = config
                    .get("tools")
                    .and_then(|tools| tools.get("platform"))
                    .and_then(serde_yaml::Value::as_mapping)
                {
                    for (key, value) in values {
                        platform.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        if let Some(serde_yaml::Value::String(path)) =
            platform.get_mut(serde_yaml::Value::String("path".to_string()))
        {
            if Path::new(path).is_relative() {
                *path = self
                    .context
                    .workspace_root
                    .join(&*path)
                    .to_string_lossy()
                    .into_owned();
            }
        }
        if !platform.is_empty() {
            let tools = serde_yaml::Mapping::from_iter([(
                serde_yaml::Value::String("platform".into()),
                serde_yaml::Value::Mapping(platform),
            )]);
            let private = serde_yaml::Mapping::from_iter([(
                serde_yaml::Value::String("tools".into()),
                serde_yaml::Value::Mapping(tools),
            )]);
            let config = serde_yaml::to_string(&private)
                .map_err(|_| source_error("cannot represent private platform selection"))?;
            std::fs::write(project.join("v8project.local.yaml"), config).map_err(io_error)?;
        }
        let input = root.join("supplier.cf");
        let output = root.join("export");
        std::fs::write(&input, payload).map_err(io_error)?;
        checkpoint(self.cancellation).map_err(stage_error)?;
        let result = self.process.run_staged(&ProcessCommand {
            program: self.runner.tool.program.clone(),
            args: vec![
                "--config".into(),
                config.to_string_lossy().into_owned(),
                "--json-message".into(),
                "convert".into(),
                input.to_string_lossy().into_owned(),
                "--to".into(),
                "xml".into(),
                "--output".into(),
                output.to_string_lossy().into_owned(),
            ],
            cwd: project,
            env: Vec::new(),
            env_remove: Vec::new(),
            capture_limits: Some((
                super::v13_infobase_exports::RUNNER_OUTPUT_LIMIT,
                super::v13_infobase_exports::RUNNER_OUTPUT_LIMIT,
            )),
            timeout: None,
            cancellation: self.cancellation.clone(),
        });
        checkpoint(self.cancellation).map_err(stage_error)?;
        let result = result
            .map_err(|_| source_error("the supplier conversion process could not complete"))?;
        let envelope: serde_json::Value = serde_json::from_str(&result.stdout).map_err(|_| {
            source_error("supplier conversion did not return a complete runner result")
        })?;
        if !result.status_success
            || result.cancelled
            || result.timed_out
            || result.stdout_truncated
            || result.stdout_had_invalid_utf8
            || envelope.get("ok").and_then(serde_json::Value::as_bool) != Some(true)
            || envelope.get("command").and_then(serde_json::Value::as_str) != Some("convert")
            || envelope
                .pointer("/data/ok")
                .and_then(serde_json::Value::as_bool)
                != Some(true)
            || envelope
                .pointer("/data/provider/selected")
                .and_then(serde_json::Value::as_str)
                != Some("ibcmd")
            || envelope
                .pointer("/data/outputs")
                .and_then(serde_json::Value::as_array)
                .is_none_or(|outputs| {
                    outputs.len() != 1
                        || outputs[0]
                            .get("source_path")
                            .and_then(serde_json::Value::as_str)
                            .map(Path::new)
                            != Some(input.as_path())
                        || outputs[0]
                            .get("target_path")
                            .and_then(serde_json::Value::as_str)
                            .map(Path::new)
                            != Some(output.as_path())
                })
            || envelope
                .pointer("/data/direction")
                .and_then(serde_json::Value::as_str)
                != Some("PACKAGE_TO_DESIGNER")
            || envelope
                .pointer("/data/provider_dispatched")
                .and_then(serde_json::Value::as_bool)
                != Some(true)
        {
            // Do not forward opaque platform output or source text from an
            // untrusted CF as a recovery message.
            return Err(source_error("the platform could not extract the supplier configuration; support was not changed"));
        }
        let root = unica_workspace::read::open_source_directory(&output).map_err(io_error)?;
        let mut read = PrivateObjectRead {
            root,
            cancellation: self.cancellation,
        };
        read_restore_object_image(&mut read, target, format)
    }
}

impl RestoreSupplierRead for SupplierReader<'_> {
    fn object_image(
        &self,
        payload: &[u8],
        target: &MetadataAddress,
        format: &str,
    ) -> Result<RestoreObjectImage, ApplyPlanError> {
        checkpoint(self.cancellation).map_err(stage_error)?;
        // Check before creating even the first directory. A TMPDIR pointing
        // into the project must not turn preview into a source/cache write.
        let temporary_root =
            isolated_temporary_root(self.context, self.source_root, &std::env::temp_dir())?;
        let temporary = tempfile::Builder::new()
            .prefix("unica-supplier-")
            .tempdir_in(temporary_root)
            .map_err(io_error)?;
        let result = self.convert(payload, target, format, temporary.path());
        temporary
            .close()
            .map_err(|_| source_error("private supplier conversion cleanup did not complete"))?;
        result
    }
}

fn isolated_temporary_root(
    context: &WorkspaceContext,
    source_root: &Path,
    candidate: &Path,
) -> Result<PathBuf, ApplyPlanError> {
    let temporary_root = normalize_path_identity(candidate)
        .map_err(|_| source_error("private temporary directory is unavailable"))?;
    for protected in [&context.workspace_root, &context.cache_root, source_root] {
        let protected = normalize_path_identity(protected)
            .map_err(|_| source_error("cannot prove private conversion path isolation"))?;
        if temporary_root.starts_with(protected) {
            return Err(source_error("supplier conversion needs a temporary directory outside sources and workspace cache"));
        }
    }
    Ok(temporary_root)
}

fn source_error(message: &str) -> ApplyPlanError {
    ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message)
}
fn io_error(_: std::io::Error) -> ApplyPlanError {
    source_error("private supplier conversion files are unavailable")
}
fn stage_error(error: ApplyStagingError) -> ApplyPlanError {
    ApplyPlanError::staging(error, "at")
}
fn checkpoint(cancellation: &CancellationToken) -> Result<(), ApplyStagingError> {
    if cancellation.is_cancelled() {
        Err(ApplyStagingError::new(
            ApplyStagingErrorKind::Cancelled,
            "supplier comparison cancelled",
        ))
    } else {
        Ok(())
    }
}

struct PrivateObjectRead<'a> {
    root: SourceDirectory,
    cancellation: &'a CancellationToken,
}

impl PrivateObjectRead<'_> {
    fn read_failure(&self) -> ApplyStagingError {
        checkpoint(self.cancellation)
            .err()
            .unwrap_or_else(unsafe_entry)
    }
    fn directory(&self, path: &Path) -> Result<Option<SourceDirectory>, ApplyStagingError> {
        let mut directory = self.root.clone();
        for component in path.components() {
            checkpoint(self.cancellation)?;
            let Component::Normal(name) = component else {
                return Err(unsafe_entry());
            };
            match directory.retain_immediate_child_nofollow(name) {
                Ok(SourceEntry::Directory(child)) => directory = child,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                _ => return Err(unsafe_entry()),
            }
        }
        Ok(Some(directory))
    }

    fn files(
        &self,
        directory: &SourceDirectory,
        prefix: &Path,
        result: &mut Vec<PathBuf>,
    ) -> Result<(), ApplyStagingError> {
        checkpoint(self.cancellation)?;
        let names = directory
            .read_immediate_names_with_limit(None, &mut || {
                checkpoint(self.cancellation).map_err(std::io::Error::other)
            })
            .map_err(|_| self.read_failure())?;
        let compare = directory
            .child_name_comparator()
            .map_err(|_| unsafe_entry())?;
        for (index, name) in names.iter().enumerate() {
            for earlier in &names[..index] {
                if compare
                    .names_equivalent(earlier, name)
                    .map_err(|_| unsafe_entry())?
                {
                    return Err(unsafe_entry());
                }
            }
            checkpoint(self.cancellation)?;
            let path = prefix.join(name);
            match directory
                .retain_immediate_child_nofollow(name)
                .map_err(|_| unsafe_entry())?
            {
                SourceEntry::Directory(child) => self.files(&child, &path, result)?,
                SourceEntry::RegularFile(file)
                    if file.hard_link_count().map_err(|_| unsafe_entry())? == 1 =>
                {
                    result.push(path)
                }
                _ => return Err(unsafe_entry()),
            }
        }
        Ok(())
    }
}

impl RestoreObjectRead for PrivateObjectRead<'_> {
    fn read(&mut self, path: &Path) -> Result<Option<Vec<u8>>, ApplyStagingError> {
        checkpoint(self.cancellation)?;
        let parent_path = path.parent().ok_or_else(unsafe_entry)?;
        let Some(parent) = self.directory(parent_path)? else {
            return Ok(None);
        };
        let name = path.file_name().ok_or_else(unsafe_entry)?;
        match parent.retain_immediate_child_nofollow(name) {
            Ok(SourceEntry::RegularFile(file))
                if file.hard_link_count().map_err(|_| unsafe_entry())? == 1 =>
            {
                let mut bytes = Vec::new();
                parent
                    .visit_relative_regular_chunks(
                        Path::new(name),
                        || checkpoint(self.cancellation).map_err(std::io::Error::other),
                        |chunk: &[u8]| {
                            bytes.extend_from_slice(chunk);
                            Ok(())
                        },
                    )
                    .map_err(|_| self.read_failure())?;
                Ok(Some(bytes))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            _ => Err(unsafe_entry()),
        }
    }
    fn tree(&mut self, path: &Path) -> Result<Vec<PathBuf>, ApplyStagingError> {
        let mut result = Vec::new();
        if let Some(directory) = self.directory(path)? {
            self.files(&directory, path, &mut result)?;
        }
        result.sort();
        Ok(result)
    }
}

fn unsafe_entry() -> ApplyStagingError {
    ApplyStagingError::new(
        ApplyStagingErrorKind::ContainmentIdentity,
        "supplier output contains an unreadable, ambiguous, linked or nonregular entry",
    )
}

#[cfg(test)]
#[path = "v13_support_restore_tests.rs"]
mod tests;
