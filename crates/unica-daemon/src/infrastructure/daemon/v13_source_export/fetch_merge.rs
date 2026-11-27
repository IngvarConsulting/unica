//! Accept only a provider-attested working-configuration snapshot as the
//! runner's base. Source selection and writes belong to the caller.
use super::*;
use serde::Serialize;
use std::io;

const PREPARE: &str = "--prepare-merge-baseline";
const SNAPSHOT_ONLY: &str = "--snapshot-only";
const EPHEMERAL_PLATFORM: &str = "--snapshot-platform-from-sources";
pub(super) const SNAPSHOT_FLAGS: [&str; 2] = [SNAPSHOT_ONLY, EPHEMERAL_PLATFORM];
const CHECK: &str = "--check-merge-baseline";
const ACCEPT: &str = "--accept-merge-baseline";

#[derive(Clone, Copy)]
pub(super) struct Capabilities {
    pub(super) prepare_merge: bool,
}

impl Capabilities {
    pub(super) fn stage_flags(self) -> Vec<String> {
        let mut flags = SNAPSHOT_FLAGS.map(str::to_owned).to_vec();
        if self.prepare_merge {
            flags.push(PREPARE.to_string());
        }
        flags
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct RunnerBaseline {
    pub(super) id: String,
    pub(super) generation: String,
    pub(super) provider: String,
    pub(super) base_generation: Option<String>,
}

impl RunnerBaseline {
    pub(super) fn is_valid(&self) -> bool {
        uuid::Uuid::parse_str(&self.id).is_ok()
            && valid_token(&self.generation)
            && valid_token(&self.provider)
            && self.base_generation.as_deref().is_none_or(valid_token)
    }
}

fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'-' | b'_' | b'.'))
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(super) struct Binding {
    source_set: String,
    extension: Option<String>,
    target: String,
    config_sha256: String,
    local_config_sha256: Option<String>,
}

impl Binding {
    pub(super) fn new(
        prepared: &PreparedSourceExport,
        inputs: &StableInputs,
        source_set: &str,
        target: String,
    ) -> Self {
        Self {
            source_set: source_set.to_owned(),
            extension: prepared.arguments.extension.clone(),
            target,
            config_sha256: inputs.config_sha256.clone(),
            local_config_sha256: inputs.local_config_sha256.clone(),
        }
    }

    pub(super) fn base_key(&self) -> String {
        // One retained comparison tree per set. The complete binding is
        // checked before its contents can be used in a later comparison.
        let mut digest = Sha256::new();
        digest.update(self.source_set.as_bytes());
        digest.update([0]);
        if let Some(extension) = &self.extension {
            digest.update(extension.as_bytes());
        }
        format!("{:x}.base", digest.finalize())
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct MergeSnapshot {
    pub(super) stage_identity: [u8; 16],
    pub(super) fingerprint: String,
    pub(super) binding: Binding,
    pub(super) runner: RunnerBaseline,
}

pub(super) fn capabilities(
    root: &Path,
    operation: &str,
    runner: &dyn ProcessRunner,
    cancellation: &CancellationToken,
    tool: &BundledTool,
) -> Result<Capabilities, DomainResult> {
    use crate::infrastructure::daemon::runner::{origin_kind, OriginKind};
    use crate::infrastructure::daemon::v13_workspace_bootstrap::read_yaml_config;
    let unsupported = || {
        reject_at(
            operation,
            RefusalCode::UnsupportedOperation,
            "the bundled v8-runner does not confirm read-only snapshot support for this infobase",
        )
    };
    let cancelled = || {
        reject_at(
            operation,
            RefusalCode::Cancelled,
            "snapshot operation cancelled before provider launch",
        )
    };
    if cancellation.is_cancelled() {
        return Err(cancelled());
    }
    if origin_kind(root).ok() != Some(OriginKind::File) {
        return Err(unsupported());
    }
    let Ok(Some(config)) = read_yaml_config(root, CONFIG_NAME) else {
        return Err(unsupported());
    };
    if config["format"]
        .as_str()
        .is_some_and(|format| format != "DESIGNER")
    {
        return Err(unsupported());
    }
    let help = ProcessCommand {
        program: tool.program.clone(),
        args: vec![
            "--config".into(),
            root.join(CONFIG_NAME).display().to_string(),
            RUNNER_COMMAND.into(),
            "--help".into(),
        ],
        cwd: root.to_path_buf(),
        env: Vec::new(),
        env_remove: Vec::new(),
        capture_limits: Some((RUNNER_OUTPUT_LIMIT, RUNNER_OUTPUT_LIMIT)),
        timeout: Some(std::time::Duration::from_secs(5)),
        cancellation: cancellation.clone(),
    };
    let output = runner.run_staged(&help);
    if cancellation.is_cancelled() {
        return Err(cancelled());
    }
    let output = output.map_err(|_| unsupported())?;
    if output.cancelled {
        return Err(cancelled());
    }
    if !output.status_success
        || output.timed_out
        || output.stdout_truncated
        || output.stdout_had_invalid_utf8
    {
        return Err(unsupported());
    }
    let contains = |flag| {
        output
            .stdout
            .split_ascii_whitespace()
            .any(|word| word == flag)
    };
    if !SNAPSHOT_FLAGS.into_iter().all(contains) {
        return Err(unsupported());
    }
    Ok(Capabilities {
        prepare_merge: [PREPARE, CHECK, ACCEPT].into_iter().all(contains),
    })
}

/// Hash all regular files relative to the retained snapshot directory.
/// Entries that are neither real files nor real directories refuse adoption.
pub(super) fn fingerprint(
    directory: &StateDirectory,
    cancellation: &CancellationToken,
) -> io::Result<String> {
    fn collect(
        directory: &StateDirectory,
        prefix: &Path,
        cancellation: &CancellationToken,
        files: &mut Vec<(String, [u8; 32])>,
    ) -> io::Result<()> {
        let mut entries = DirectoryCursor::open(directory)?;
        while let Some(name) = entries.next_name()? {
            if cancellation.is_cancelled() {
                return Err(io::Error::other("snapshot verification was cancelled"));
            }
            if name.to_str().is_none() {
                return Err(io::Error::other("snapshot contains a non-Unicode name"));
            }
            let path = prefix.join(&name);
            match directory.retain_regular_child(&name) {
                Ok(file) => {
                    let mut hash = Sha256::new();
                    let mut stream = file.open_retained_handle()?;
                    io::copy(&mut stream, &mut hash)?;
                    file.validate_named_identity()?;
                    files.push((path_text(&path), hash.finalize().into()));
                }
                Err(_) => collect(
                    &directory.retain_directory_child(&name)?,
                    &path,
                    cancellation,
                    files,
                )?,
            }
        }
        directory.validate_named_identity()
    }

    let mut files = Vec::new();
    collect(directory, Path::new(""), cancellation, &mut files)?;
    files.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    let mut hash = Sha256::new();
    for (path, digest) in files {
        hash.update(path.as_bytes());
        hash.update([0]);
        hash.update(digest);
    }
    Ok(format!("sha256:{:x}", hash.finalize()))
}

pub(super) fn execute_with_tool(
    prepared: &PreparedSourceExport,
    id: &str,
    runner: &dyn ProcessRunner,
    cancellation: &CancellationToken,
    tool: &BundledTool,
) -> DomainResult {
    match execute_bound(prepared, id, runner, cancellation, tool) {
        Ok(result) | Err(result) => result,
    }
}

fn reject(code: RefusalCode, message: impl Into<String>) -> DomainResult {
    reject_at(FETCH_ACCEPT, code, message)
}

fn storage_error(error: io::Error) -> DomainResult {
    storage_refusal(FETCH_ACCEPT, error)
}

fn execute_bound(
    prepared: &PreparedSourceExport,
    id: &str,
    runner: &dyn ProcessRunner,
    cancellation: &CancellationToken,
    tool: &BundledTool,
) -> Result<DomainResult, DomainResult> {
    let root = normalize_path_identity(&prepared.context.workspace_root).map_err(|_| {
        reject(
            RefusalCode::InvalidState,
            "fetch.accept could not resolve the workspace root",
        )
    })?;
    let unavailable = || {
        reject(
            RefusalCode::InvalidState,
            "fetch.accept requires an owned snapshot with a provider-confirmed generation; fetch again with a compatible runner",
        )
    };
    let storage = open_storage(&root, false, FETCH_ACCEPT)?.ok_or_else(unavailable)?;
    let _lock = storage
        .lock(cancellation)
        .map_err(|error| lock_refusal(FETCH_ACCEPT, error))?;
    let owned = storage
        .owned_snapshot(id)
        .map_err(storage_error)?
        .ok_or_else(unavailable)?;
    let snapshot = storage
        .merge_snapshot(&owned)
        .map_err(storage_error)?
        .ok_or_else(unavailable)?;
    let inputs = capture_inputs(prepared).map_err(|mut result| {
        result.at = Some(FETCH_ACCEPT.to_string());
        result
    })?;
    if snapshot.binding.config_sha256 != inputs.config_sha256
        || snapshot.binding.local_config_sha256 != inputs.local_config_sha256
        || !inputs.declared.contains(&snapshot.binding.source_set)
    {
        return Err(reject(
            RefusalCode::ConcurrentChange,
            "the project recipe changed since the snapshot was fetched; fetch again before accepting changes",
        ));
    }
    let raw_fingerprint = fingerprint(owned.directory(), cancellation).map_err(|error| {
        if cancellation.is_cancelled() {
            reject(RefusalCode::Cancelled, "fetch.accept was cancelled")
        } else {
            storage_error(error)
        }
    })?;
    if raw_fingerprint != snapshot.fingerprint {
        return Err(reject(
            RefusalCode::ConcurrentChange,
            "the raw snapshot changed after fetch; fetch again before accepting changes",
        ));
    }
    let envelope = invoke(
        prepared.dry_run,
        &root,
        &snapshot,
        runner,
        cancellation,
        tool,
    )?;
    let data = &envelope["data"];
    let accepted = data["baseline"] == "accepted";
    if (!accepted && (!prepared.dry_run || data["baseline"] != "ready"))
        || data["source_set"] != snapshot.binding.source_set
        || data["generation"] != snapshot.runner.generation
        || data["provider"] != snapshot.runner.provider
    {
        return Err(reject(
            RefusalCode::InvalidResult,
            "v8-runner did not confirm the expected source set, provider, generation and baseline decision",
        ));
    }
    if !prepared.dry_run {
        // The runner commit precedes this bookkeeping. The runner recognizes
        // the same candidate on retry if our reply or this write was lost.
        storage
            .mark_merge_base(&owned, &snapshot)
            .map_err(storage_error)?;
    }
    let mut result = DomainResult::success(if prepared.dry_run {
        "fetch.accept verified that the snapshot still matches the infobase generation"
    } else {
        "fetch.accept set the runner's comparison base to the raw infobase snapshot"
    });
    result.data = Some(json!({
        "op": FETCH_ACCEPT,
        "dryRun": prepared.dry_run,
        "snapshotId": id,
        "sourceSet": snapshot.binding.source_set,
        "extension": snapshot.binding.extension,
        "generation": snapshot.runner.generation,
        "ready": true,
        "accepted": accepted,
    }));
    note_runner_warnings(&mut result, &envelope);
    Ok(result)
}

fn invoke(
    dry_run: bool,
    root: &Path,
    snapshot: &MergeSnapshot,
    runner: &dyn ProcessRunner,
    cancellation: &CancellationToken,
    tool: &BundledTool,
) -> Result<Value, DomainResult> {
    let mut args = vec![
        "--config".to_string(),
        root.join(CONFIG_NAME).display().to_string(),
        "--json-message".to_string(),
        RUNNER_COMMAND.to_string(),
        snapshot.binding.source_set.clone(),
        if dry_run { CHECK } else { ACCEPT }.to_string(),
        snapshot.runner.id.clone(),
    ];
    if let Some(extension) = &snapshot.binding.extension {
        args.extend(["--extension".to_string(), extension.clone()]);
    }
    args.push(EPHEMERAL_PLATFORM.to_string());
    let output = runner
        .run_staged(&ProcessCommand {
            program: tool.program.clone(),
            args,
            cwd: root.to_path_buf(),
            env: Vec::new(),
            env_remove: Vec::new(),
            capture_limits: Some((RUNNER_OUTPUT_LIMIT, RUNNER_OUTPUT_LIMIT)),
            timeout: None,
            cancellation: cancellation.clone(),
        })
        .map_err(|error| {
            runner_start_rejection(Some(FETCH_ACCEPT.to_string()), &error, &tool.program)
        })?;
    parse_runner_output(output).map_err(|(mut result, envelope)| {
        result.at = Some(FETCH_ACCEPT.to_string());
        if envelope["error"]["code"] == "non_fast_forward" {
            result = reject(
                RefusalCode::ConcurrentChange,
                "the infobase generation changed after fetch; fetch again before accepting changes",
            );
            result.data = Some(json!({"runnerCode": "non_fast_forward"}));
        }
        note_runner_warnings(&mut result, &envelope);
        result
    })
}

pub(super) fn comparison_base(
    storage: &SnapshotStorage,
    snapshot: &MergeSnapshot,
    root: &Path,
    cancellation: &CancellationToken,
) -> Option<String> {
    let generation = snapshot.runner.base_generation.as_ref()?;
    let base = storage.merge_base(&snapshot.binding).ok()??;
    let previous = storage.merge_snapshot(&base).ok()??;
    if previous.binding != snapshot.binding
        || previous.runner.generation != *generation
        || previous.runner.provider != snapshot.runner.provider
        || fingerprint(base.directory(), cancellation).ok()? != previous.fingerprint
    {
        return None;
    }
    Some(relative(base.directory().path(), root))
}
