//! The pinned v8-runner: its version, the lock that keeps two invocations for
//! one working copy from overlapping, and how the declared `origin` infobase is
//! read.
//!
//! The runner reads `v8project.yaml` and `v8project.local.yaml` of the working
//! copy in place, from the `--config` argument; Unica does not copy them,
//! except for a re-export, which runs on a config of its own
//! ([`write_reexport_config`]). The
//! only key Unica writes is the strict platform choice in the local file, once
//! per operation and before the operation reads its inputs
//! ([`prepare_working_copy`]). The runner records the directory of that config
//! as the working copy that holds a file infobase, so Unica and a person
//! running the runner by hand in the same working copy are one holder.
use super::v13_workspace_bootstrap::read_yaml_config;
use serde_yaml::{Mapping, Value};
use std::path::{Path, PathBuf};
use unica_execution_contract::invocation::DomainResult;
use unica_process::runner::{
    BreakawayProcessRunner, ProcessCommand, ProcessOutput, ProcessRunner, ProcessStreamFailure,
};
use unica_process::PendingProcessHandoff;
use unica_refusal::RefusalCode;
use unica_work_control::cancellation::{CancellationToken, CANCELLED_PREFIX};
use unica_workspace::paths::normalize_path_identity;
use unica_workspace::runner_lock::{RunnerLock, RunnerLockFailure};

pub(super) const VERSION: &str = "0.17.1";
pub(super) fn check_version(version: &str) -> Result<(), String> {
    if version == VERSION {
        Ok(())
    } else {
        Err(format!(
            "no verified Unica runner adapter for version {version}; expected {VERSION}"
        ))
    }
}

const BASE_NAME: &str = "v8project.yaml";
const LOCAL_NAME: &str = "v8project.local.yaml";
const ORIGIN: &str = "origin";

/// The warning of a command that the runner abandoned on a second interrupt
/// while the Designer agent of the standalone server may still hold the
/// infobase.
const ABANDONED_STANDALONE_CODE: &str = "agent_command_abandoned_standalone";
const ABANDONED_STANDALONE: &str = "the command was abandoned while the Designer agent of the standalone server may still be working on the infobase and holding it open; to release it, run `v8-runner standalone restart` in the working copy";

/// Runs the pinned runner on the working-copy config, one invocation per
/// working copy at a time. Its group lets the runner's live Designer agent
/// leave it, and a cancel interrupts it instead of ending it
/// ([`BreakawayProcessRunner`]). It keeps what the last runner command said
/// about that agent ([`agent_receipt`]) and whether the runner abandoned a
/// command of the agent.
#[derive(Default)]
pub(super) struct PinnedRunner {
    agent: std::sync::Mutex<Option<serde_json::Value>>,
    abandoned: std::sync::atomic::AtomicBool,
}

impl PinnedRunner {
    /// The runner that a cancel interrupted answered it itself: its answer is
    /// the outcome of the call, unless the answer is the cancel. A finished
    /// command or an abandoned one goes to the adapter as any answer of the
    /// runner, and the call publishes it instead of the cancel.
    fn take_answer_to_interrupt(
        &self,
        command: &ProcessCommand,
        answers_before: usize,
        output: &mut ProcessOutput,
    ) {
        if !output.cancelled || command.cancellation.interrupt_answers() == answers_before {
            return;
        }
        let Ok(envelope) = serde_json::from_str::<serde_json::Value>(output.stdout.trim()) else {
            return;
        };
        let code = envelope["error"]["code"].as_str();
        if code == Some("cancelled") || !envelope["ok"].is_boolean() {
            return;
        }
        output.cancelled = false;
        command.cancellation.record_answered_interrupt();
        if code == Some("agent_command_abandoned") {
            self.abandoned
                .store(true, std::sync::atomic::Ordering::Release);
        }
    }

    /// Whether the runner abandoned a command of its Designer agent.
    fn abandoned(&self) -> bool {
        self.abandoned.load(std::sync::atomic::Ordering::Acquire)
    }

    fn note(&self, output: &ProcessOutput) {
        if let Some(agent) = agent_receipt(&output.stdout) {
            *self.agent.lock().unwrap_or_else(|error| error.into_inner()) = Some(agent);
        }
    }

    /// `data.agent` of the last runner command that named its live agent.
    fn agent(&self) -> Option<serde_json::Value> {
        self.agent
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}

impl ProcessRunner for PinnedRunner {
    fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
        let _lock = lock_for(command)?;
        let answers = command.cancellation.interrupt_answers();
        let mut output = BreakawayProcessRunner.run(command)?;
        self.take_answer_to_interrupt(command, answers, &mut output);
        self.note(&output);
        Ok(output)
    }

    /// Waiting for the lock happens before any process exists, but it is not
    /// a failed launch of the runner: it is reported as a failure after start,
    /// never as an unstartable engine.
    fn run_staged(&self, command: &ProcessCommand) -> Result<ProcessOutput, ProcessStreamFailure> {
        let _lock = lock_for(command).map_err(ProcessStreamFailure::Run)?;
        let answers = command.cancellation.interrupt_answers();
        let mut output = BreakawayProcessRunner.run_staged(command)?;
        self.take_answer_to_interrupt(command, answers, &mut output);
        self.note(&output);
        Ok(output)
    }

    fn run_pending_handoff(
        &self,
        command: &ProcessCommand,
    ) -> Result<(ProcessOutput, PendingProcessHandoff), String> {
        let _lock = lock_for(command)?;
        let (output, handoff) = BreakawayProcessRunner.run_pending_handoff(command)?;
        self.note(&output);
        Ok((output, handoff))
    }

    /// As [`Self::run_staged`]: waiting for the lock is not a failed launch.
    fn run_pending_handoff_staged(
        &self,
        command: &ProcessCommand,
    ) -> Result<(ProcessOutput, PendingProcessHandoff), ProcessStreamFailure> {
        let _lock = lock_for(command).map_err(ProcessStreamFailure::Run)?;
        let (output, handoff) = BreakawayProcessRunner.run_pending_handoff_staged(command)?;
        self.note(&output);
        Ok((output, handoff))
    }
}

/// `data.agent` from the runner's answer: whether the command took a live
/// Designer agent (`reused`), whether the agent stays (`kept`) and why not
/// (`reason`, the runner's word), how many holders it has and whether this
/// Unica process is one of them. The runner names it in
/// `provider.endpoint.live` of a managed agent session; other holders' pids
/// stay private.
fn agent_receipt(stdout: &str) -> Option<serde_json::Value> {
    let envelope: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;
    let live = envelope["data"]["provider"]["endpoint"]["live"].as_object()?;
    let reused = live.get("reused")?.as_bool()?;
    let kept = live.get("kept")?.as_bool()?;
    let holders = live.get("holders")?.as_array()?;
    let own = u64::from(std::process::id());
    let mut agent = serde_json::json!({
        "reused": reused,
        "kept": kept,
        "holders": holders.len(),
        "heldByUnica": holders.iter().any(|holder| holder["pid"].as_u64() == Some(own)),
    });
    if let Some(reason) = live.get("reason").and_then(serde_json::Value::as_str) {
        agent["reason"] = serde_json::Value::from(reason);
    }
    Some(agent)
}

fn lock_for(command: &ProcessCommand) -> Result<RunnerLock, String> {
    // `init` takes no global `--config`: it names the project file it writes
    // beside with `--output`.
    let config = ["--config", "--output"]
        .iter()
        .find_map(|key| {
            command
                .args
                .iter()
                .position(|value| value == key)
                .and_then(|position| command.args.get(position + 1))
        })
        .ok_or("runner adapter requires an explicit project config")?;
    let root = Path::new(config)
        .parent()
        .ok_or("project config has no parent")?;
    lock_working_copy(root, &command.cancellation).map_err(|error| match error {
        RunnerLockFailure::Cancelled => {
            format!("{CANCELLED_PREFIX} while waiting for the working copy runner lock")
        }
        RunnerLockFailure::InvalidState(message) => message,
    })
}

/// Runs a runner operation of an adapter. With the bundled runner resolvable
/// from `cwd`, the working copy is prepared first
/// ([`with_prepared_working_copy`]). Without it `run` refuses with the absent
/// runner, and a refused operation leaves the working copy as it was: no lock
/// file, no written key.
///
/// The answer carries `data.agent` when a runner command of the operation
/// named its live Designer agent ([`agent_receipt`]).
pub(super) fn with_bundled_runner(
    root: &Path,
    cwd: &Path,
    at: &str,
    cancellation: CancellationToken,
    run: impl FnOnce(&PinnedRunner, CancellationToken) -> DomainResult,
) -> DomainResult {
    let runner = PinnedRunner::default();
    let Ok(resolved) = super::v13_infobase_exports::resolve_bundled_runner(cwd) else {
        return run(&runner, cancellation);
    };
    let agents = super::live_agent::live_agents();
    agents.touch(root, std::time::Instant::now());
    let mut result = with_prepared_working_copy(root, at, cancellation, |cancellation| {
        run(&runner, cancellation)
    });
    result.warnings.extend(abandoned_warning(root, &runner));
    let Some(mut agent) = runner.agent() else {
        return result;
    };
    let after = agents.after_command(
        root,
        &resolved.tool.program,
        &agent,
        std::time::Instant::now(),
    );
    // Holding and letting go take the runner lock of a working copy: they run
    // apart from the answer, which another command of that copy could hold up.
    if !after.marks.is_empty() {
        let marks = after.marks;
        let _ = std::thread::Builder::new()
            .name("unica-live-agent-mark".to_string())
            .spawn(move || {
                if agents.perform(&PinnedRunner::default(), &marks) {
                    agents.keep_sweeping();
                }
            });
    }
    match after.reply {
        Some(super::live_agent::AgentReply::Warning(warning)) => result.warnings.push(warning),
        Some(super::live_agent::AgentReply::Note(note)) => {
            agent["note"] = serde_json::Value::from(note);
        }
        None => {}
    }
    if let Some(serde_json::Value::Object(data)) = result.data.as_mut() {
        data.insert("agent".to_string(), agent);
    }
    result
}

/// After the runner abandoned a command of the Designer agent of a
/// standalone server, the agent may still work on the infobase and hold it
/// open: the answer says how to release it.
fn abandoned_warning(root: &Path, runner: &PinnedRunner) -> Option<serde_json::Value> {
    (runner.abandoned() && matches!(origin_kind(root), Ok(OriginKind::Standalone))).then(|| {
        serde_json::json!({
            "code": ABANDONED_STANDALONE_CODE,
            "message": ABANDONED_STANDALONE,
        })
    })
}

/// Runs a runner operation on a prepared working copy: [`prepare_working_copy`]
/// first, then `run`, whose answer also carries the warnings of the
/// preparation.
pub(super) fn with_prepared_working_copy(
    root: &Path,
    at: &str,
    cancellation: CancellationToken,
    run: impl FnOnce(CancellationToken) -> DomainResult,
) -> DomainResult {
    let warnings = match prepare_working_copy(root, at, &cancellation) {
        Ok(warnings) => warnings,
        Err(result) => return result,
    };
    let mut result = run(cancellation);
    for warning in warnings {
        if !result.warnings.contains(&warning) {
            result.warnings.push(warning);
        }
    }
    result
}

/// Prepares the working copy for a runner operation, before the operation
/// reads its inputs and under the working-copy lock: switches the strict
/// platform choice on ([`ensure_platform_line`]) and removes the private config
/// copy that Unica kept before runner 0.17 ([`remove_former_projection`]).
/// Returns the warnings of the preparation.
#[allow(clippy::result_large_err)]
fn prepare_working_copy(
    root: &Path,
    at: &str,
    cancellation: &CancellationToken,
) -> Result<Vec<serde_json::Value>, DomainResult> {
    let reject = |code, message: String| {
        DomainResult::canonical_rejection(Some(at.to_string()), code, message)
    };
    let _lock = lock_working_copy(root, cancellation).map_err(|error| match error {
        RunnerLockFailure::Cancelled => reject(
            RefusalCode::Cancelled,
            format!("{at} cancelled while waiting for the working copy runner lock"),
        ),
        RunnerLockFailure::InvalidState(message) => reject(RefusalCode::InvalidState, message),
    })?;
    let mut warnings: Vec<serde_json::Value> = remove_former_projection(root).into_iter().collect();
    // A file of the person changes: the answer says which and how. A file
    // Unica cannot write stays as it is and the operation goes on; the answer
    // says that the strict choice is off.
    match ensure_platform_line(root) {
        Ok(PlatformLine::Untouched) => {}
        Ok(written) => warnings.push(serde_json::json!({
            "code": PLATFORM_LINE_WRITTEN_CODE,
            "message": if written == PlatformLine::Rewritten {
                format!("{PLATFORM_LINE_WRITTEN}; the key could not be placed into the text of the file, so the file was written anew with the same keys and values, without its comments and layout")
            } else {
                PLATFORM_LINE_WRITTEN.to_string()
            },
        })),
        Err(error) => warnings.push(serde_json::json!({
            "code": PLATFORM_LINE_NOT_WRITTEN_CODE,
            "message": format!("Unica could not add tools.platform.version: sources to v8project.local.yaml ({error}); the runner chooses the platform without the line of the source format; add the key there, or set tools.platform.version to choose a platform yourself"),
        })),
    }
    Ok(warnings)
}

/// Code of the note that Unica wrote the strict platform choice.
const PLATFORM_LINE_WRITTEN_CODE: &str = "platform_line_written";

const PLATFORM_LINE_WRITTEN: &str = "Unica added tools.platform.version: sources to v8project.local.yaml: the runner now runs on the platform line that writes the format of the sources; set tools.platform.version there to choose a platform yourself";

/// Code of the warning that the strict platform choice could not be written.
const PLATFORM_LINE_NOT_WRITTEN_CODE: &str = "platform_line_not_written";

/// Code of the warning that the former private config copy stays.
const FORMER_PROJECTION_WARNING_CODE: &str = "former_runner_config_copy_kept";

/// Before runner 0.17 Unica gave the runner a private copy of both config
/// layers, secrets included, in `.build/unica/runner-project/` (keyed by the
/// working copy under `UNICA_CACHE_DIR`). The runner recorded that directory as
/// the holder of a file infobase; while it stays, the working copy itself is
/// another, live holder. Removing it makes that record a gone holder (its
/// directory is gone), which the next writing command of the working copy
/// takes over. Only that directory and what the former store kept beside it
/// are removed: its lock directory and, in the cache of the working copy, the
/// leftovers of an interrupted removal. A link or reparse point in their place
/// is removed itself, never followed. Removal is idempotent, and a failure
/// does not stop the operation: it becomes a warning of the answer.
fn remove_former_projection(root: &Path) -> Option<serde_json::Value> {
    let root = normalize_path_identity(root).ok()?;
    let cache = std::env::var_os("UNICA_CACHE_DIR").map(PathBuf::from);
    let shown = if cache.is_some() {
        "UNICA_CACHE_DIR/runner-project"
    } else {
        ".build/unica/runner-project"
    };
    let directory = former_projection_directory_in(&root, cache.clone());
    let mut leftovers = Vec::new();
    if let (Some(parent), Some(name)) = (directory.parent(), directory.file_name()) {
        use sha2::{Digest, Sha256};
        leftovers.push(parent.join(format!(
            ".runner-project-lock-{:x}",
            Sha256::digest(name.as_encoded_bytes())
        )));
        // A shared cache holds the leftovers of every working copy; only the
        // cache of this working copy is cleared of them.
        if cache.is_none() {
            leftovers.extend(
                std::fs::read_dir(parent)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with(".runner-project-cleanup-")
                    })
                    .map(|entry| entry.path()),
            );
        }
    }
    let failed = remove_without_following(&directory).err();
    for leftover in leftovers {
        let _ = remove_without_following(&leftover);
    }
    failed.map(|error| {
        serde_json::json!({
            "code": FORMER_PROJECTION_WARNING_CODE,
            "message": format!(
                "the private copy of v8project.yaml and v8project.local.yaml that earlier Unica versions kept for the runner in {shown} could not be removed ({}); it may hold infobase credentials, and while it stays the runner may treat it as another working copy holding a file infobase; remove the directory",
                error.kind()
            ),
        })
    })
}

/// Removes a path; a symbolic link or junction is removed itself, its target
/// stays. An absent path is already removed.
fn remove_without_following(path: &Path) -> std::io::Result<()> {
    let removed = match std::fs::symlink_metadata(path) {
        Err(error) => Err(error),
        Ok(metadata) if metadata.file_type().is_symlink() => {
            std::fs::remove_file(path).or_else(|_| std::fs::remove_dir(path))
        }
        // `remove_dir_all` removes links inside without following them.
        Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
    };
    match removed {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

fn former_projection_directory_in(root: &Path, cache_override: Option<PathBuf>) -> PathBuf {
    let lock = lock_directory_in(root, cache_override);
    match lock.file_name().filter(|name| *name != "runner-lock") {
        // `<cache>/runner-lock/<key>` -> `<cache>/runner-project/<key>`.
        Some(key) => lock
            .parent()
            .and_then(Path::parent)
            .expect("a keyed lock directory has a cache")
            .join("runner-project")
            .join(key),
        None => lock.with_file_name("runner-project"),
    }
}

pub(super) fn lock_working_copy(
    root: &Path,
    cancellation: &CancellationToken,
) -> Result<RunnerLock, RunnerLockFailure> {
    let root = normalize_path_identity(root).map_err(RunnerLockFailure::InvalidState)?;
    RunnerLock::acquire(&lock_directory(&root), cancellation)
}

/// The lock directory of one working copy, in the Unica cache:
/// `<workspace>/.build/unica/runner-lock` by default. A cache moved out by
/// `UNICA_CACHE_DIR` is shared by workspaces, so there the directory is keyed
/// by the workspace root.
pub(super) fn lock_directory(root: &Path) -> PathBuf {
    lock_directory_in(root, std::env::var_os("UNICA_CACHE_DIR").map(PathBuf::from))
}

fn lock_directory_in(root: &Path, cache_override: Option<PathBuf>) -> PathBuf {
    match cache_override {
        None => root.join(".build").join("unica").join("runner-lock"),
        Some(cache) => {
            use sha2::{Digest, Sha256};
            // A relative cache is read from the current directory, as the
            // Unica cache itself reads it.
            let cache = std::env::current_dir()
                .map(|current| current.join(&cache))
                .unwrap_or(cache);
            let digest = Sha256::digest(root.as_os_str().as_encoded_bytes());
            let key: String = digest[..8]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            cache.join("runner-lock").join(key)
        }
    }
}

/// The runner picks the platform by the line that writes the source format
/// (`tools.platform.version: sources`) and refuses when that line is not
/// installed. Unica switches the mode on for a DESIGNER project with a
/// CONFIGURATION source set; a version set in either layer is kept, and an
/// empty one is not set, as for the runner. The format and the source sets
/// live only in the project file, written as the runner reads them (upper
/// case): the runner refuses them in the local layer.
fn needs_platform_line(base: &Value, local: Option<&Value>) -> bool {
    let designer = base
        .get("format")
        .and_then(Value::as_str)
        .is_none_or(|format| format == "DESIGNER");
    let main_set = base
        .get("source-set")
        .and_then(Value::as_sequence)
        .is_some_and(|sets| {
            sets.iter().any(|set| {
                set["type"]
                    .as_str()
                    .is_some_and(|kind| kind == "CONFIGURATION")
            })
        });
    let pinned = std::iter::once(base).chain(local).any(|layer| {
        layer["tools"]["platform"]
            .get("version")
            .is_some_and(|version| !version.is_null())
    });
    designer && main_set && !pinned
}

/// What [`ensure_platform_line`] did with `v8project.local.yaml`.
#[derive(Debug, PartialEq, Eq)]
enum PlatformLine {
    Untouched,
    /// The key went into the text; the rest of it is as written.
    Inserted,
    /// The document was written anew with the same keys and values.
    Rewritten,
}

/// Writes `tools.platform.version: sources` into `v8project.local.yaml` when
/// [`needs_platform_line`] holds, creating the file when it is absent, and
/// says what it did. The rest of the file is kept as written; only when the
/// key cannot be placed into the text is the document written anew with the
/// same content. Once written, the key is set and the next call writes
/// nothing. A read-only file is not written. A local file that is a link is
/// not a file of the working copy: it is neither read nor written, and the
/// operation refuses it when it reads its inputs.
///
/// A config the runner cannot read is left as it is: the runner refuses it
/// with its own answer.
fn ensure_platform_line(root: &Path) -> Result<PlatformLine, String> {
    let Ok(Some(base)) = read_yaml_config(root, BASE_NAME) else {
        return Ok(PlatformLine::Untouched);
    };
    let Ok(local) = read_yaml_config(root, LOCAL_NAME) else {
        return Ok(PlatformLine::Untouched);
    };
    if !needs_platform_line(&base, local.as_ref()) {
        return Ok(PlatformLine::Untouched);
    }
    let path = root.join(LOCAL_NAME);
    let mut permissions = None;
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
        // A link is not a file of the working copy: Unica neither reads nor
        // writes through it, and the operation refuses it when it reads its
        // inputs.
        Ok(metadata) if !metadata.file_type().is_file() => return Ok(PlatformLine::Untouched),
        Ok(metadata) => {
            if metadata.permissions().readonly() {
                return Err("the file is read-only".to_string());
            }
            permissions = Some(metadata.permissions());
        }
    }
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.to_string()),
    };
    let Ok((updated, rewritten)) = with_platform_line(&text, local.as_ref()) else {
        return Ok(PlatformLine::Untouched);
    };
    let directory = path.parent().ok_or("the file has no directory")?;
    let staged = directory.join(format!(".{LOCAL_NAME}.{}.tmp", std::process::id()));
    write_staged(&staged, updated.as_bytes(), permissions)
        .and_then(|()| std::fs::rename(&staged, &path))
        .map_err(|error| {
            let _ = std::fs::remove_file(&staged);
            error.to_string()
        })?;
    Ok(if rewritten {
        PlatformLine::Rewritten
    } else {
        PlatformLine::Inserted
    })
}

/// Writes the staged copy of the local file. It may hold infobase
/// credentials: it is created with the permissions of the file it replaces,
/// never readable more widely, even for a moment.
fn write_staged(
    staged: &Path,
    bytes: &[u8],
    permissions: Option<std::fs::Permissions>,
) -> std::io::Result<()> {
    use std::io::Write;
    let _ = std::fs::remove_file(staged);
    let mut file = unica_workspace::host_platform::create_new_like(staged, permissions.as_ref())?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    match permissions {
        Some(permissions) => std::fs::set_permissions(staged, permissions),
        None => Ok(()),
    }
}

/// The local file with the key, and whether it had to be written anew.
fn with_platform_line(text: &str, local: Option<&Value>) -> Result<(String, bool), String> {
    let mut expected = match local {
        Some(Value::Mapping(map)) => map.clone(),
        None | Some(Value::Null) => Mapping::new(),
        Some(_) => return Err(format!("{LOCAL_NAME} is not a mapping")),
    };
    insert_platform_version(&mut expected)?;
    let expected = Value::Mapping(expected);
    if let Some(candidate) = insert_into_text(text) {
        if serde_yaml::from_str::<Value>(&candidate).ok().as_ref() == Some(&expected) {
            return Ok((candidate, false));
        }
    }
    serde_yaml::to_string(&expected)
        .map(|written| (written, true))
        .map_err(|error| error.to_string())
}

fn insert_platform_version(layer: &mut Mapping) -> Result<(), String> {
    let mut current = layer;
    for part in ["tools", "platform"] {
        let entry = current
            .entry(Value::from(part))
            .or_insert_with(|| Value::Mapping(Mapping::new()));
        if entry.is_null() {
            *entry = Value::Mapping(Mapping::new());
        }
        current = entry
            .as_mapping_mut()
            .ok_or_else(|| format!("{part} in {LOCAL_NAME} must be a mapping"))?;
    }
    current.insert(Value::from("version"), Value::from("sources"));
    Ok(())
}

/// Places the key into block-style YAML text without touching other lines.
/// The caller checks the result by parsing it.
fn insert_into_text(text: &str) -> Option<String> {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let (bom, text) = match text.strip_prefix('\u{feff}') {
        Some(rest) => ("\u{feff}", rest),
        None => ("", text),
    };
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    // `tools:` with an empty value is a section yet to be filled.
    for line in &mut lines {
        let value = line
            .strip_prefix("tools:")
            .map(|rest| rest.split(" #").next().unwrap_or_default().trim());
        if matches!(value, Some("null" | "~" | "Null" | "NULL")) {
            *line = "tools:".to_string();
        }
    }
    let indent = |line: &str| line.len() - line.trim_start().len();
    let is_key = |line: &str, key: &str| {
        line.trim_start()
            .strip_prefix(key)
            .and_then(|rest| rest.strip_prefix(':'))
            .is_some_and(|rest| rest.trim().is_empty() || rest.trim_start().starts_with('#'))
    };
    let content = |line: &str| !line.trim().is_empty() && !line.trim_start().starts_with('#');
    match lines
        .iter()
        .position(|line| indent(line) == 0 && is_key(line, "tools"))
    {
        None => {
            if lines
                .iter()
                .any(|line| indent(line) == 0 && line.starts_with("tools"))
            {
                return None;
            }
            lines.extend(["tools:", "  platform:", "    version: sources"].map(str::to_owned));
        }
        Some(tools) => {
            let block_end = lines[tools + 1..]
                .iter()
                .position(|line| content(line) && indent(line) == 0)
                .map_or(lines.len(), |offset| tools + 1 + offset);
            let children = &lines[tools + 1..block_end];
            let child = children
                .iter()
                .find(|line| content(line))
                .map_or(2, |line| indent(line));
            match children
                .iter()
                .position(|line| indent(line) == child && is_key(line, "platform"))
            {
                Some(offset) => {
                    let platform = tools + 1 + offset;
                    let grandchild = lines[platform + 1..block_end]
                        .iter()
                        .find(|line| content(line))
                        .map(|line| indent(line))
                        .filter(|width| *width > child)
                        .unwrap_or(child + 2);
                    lines.insert(
                        platform + 1,
                        format!("{}version: sources", " ".repeat(grandchild)),
                    );
                }
                None => {
                    lines.insert(tools + 1, format!("{}platform:", " ".repeat(child)));
                    lines.insert(
                        tools + 2,
                        format!("{}version: sources", " ".repeat(child + 2)),
                    );
                }
            }
        }
    }
    let mut out = format!("{bom}{}", lines.join(newline));
    out.push_str(newline);
    Some(out)
}

/// Removes the infobase description from a layer and returns its `origin`.
fn take_origin(layer: &mut Value, file: &str) -> Result<Option<Value>, String> {
    let map = layer
        .as_mapping_mut()
        .ok_or_else(|| format!("{file} must be a mapping"))?;
    let legacy = map.remove(Value::from("infobase"));
    let target = map
        .remove(Value::from("infobases"))
        .and_then(|bases| bases.as_mapping()?.get(Value::from(ORIGIN)).cloned());
    match legacy.or(target) {
        Some(section) if section.is_mapping() => Ok(Some(section)),
        Some(_) => Err(format!("the {ORIGIN} infobase in {file} must be a mapping")),
        None => Ok(None),
    }
}

/// The kind of the `origin` infobase, read the way the runner reads it: the
/// section of both layers merged (local wins), `standalone` declares a
/// standalone server, otherwise a `File=` connection a file infobase, otherwise
/// a cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OriginKind {
    Undeclared,
    File,
    Cluster,
    Standalone,
}

pub(super) fn origin_kind(root: &Path) -> Result<OriginKind, String> {
    let mut origin: Option<Value> = None;
    for file in [BASE_NAME, LOCAL_NAME] {
        let Some(mut layer) = read_yaml_config(root, file)? else {
            continue;
        };
        if let Some(section) = take_origin(&mut layer, file)? {
            match origin.as_mut() {
                Some(merged) => merge_yaml_values(merged, section),
                None => origin = Some(section),
            }
        }
    }
    let Some(origin) = origin else {
        return Ok(OriginKind::Undeclared);
    };
    if origin
        .get("standalone")
        .is_some_and(|value| !value.is_null())
    {
        return Ok(OriginKind::Standalone);
    }
    let file = origin
        .get("connection")
        .and_then(Value::as_str)
        .is_some_and(declares_a_file_infobase);
    Ok(if file {
        OriginKind::File
    } else {
        OriginKind::Cluster
    })
}

/// The runner's reading of a declared connection (`declared_parameters`):
/// non-empty `;` parts, each `key=value` with the key trimmed and compared
/// without case; a part without `=` makes the string undeclared, and a file
/// infobase has a non-empty `File` value. Raw argv names it with `/F`.
fn declares_a_file_infobase(connection: &str) -> bool {
    let text = connection.trim();
    if text.starts_with(['/', '-']) {
        return text
            .split_whitespace()
            .any(|token| token.eq_ignore_ascii_case("/F"));
    }
    let parts: Option<Vec<(String, &str)>> = text
        .split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.split_once('=')
                .map(|(key, value)| (key.trim().to_ascii_lowercase(), value.trim()))
        })
        .collect();
    parts.is_some_and(|parts| {
        parts
            .iter()
            .any(|(key, value)| key == "file" && !value.is_empty())
    })
}

/// The runner's own layer merge: mappings merge key by key, anything else is
/// replaced by the overlay.
fn merge_yaml_values(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Mapping(base), Value::Mapping(overlay)) => {
            for (key, overlay_value) in overlay {
                match base.get_mut(&key) {
                    Some(base_value) => merge_yaml_values(base_value, overlay_value),
                    None => {
                        base.insert(key, overlay_value);
                    }
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

/// The config of a re-export (INV.SOURCE.WRITABLE-PROFILE), written into
/// `directory`, the re-export's own: the project's source sets at their own
/// directories, a temporary file infobase `directory/ib`, the runner's working
/// files and personal state inside `directory`, and the platform line that
/// writes the target format. The project's own config, infobase, `.local` and
/// the runner's memory of them stay untouched. Only the shared settings of the
/// two layers are carried over: infobase sections with their credentials, the
/// platform location and the test and MCP settings are not. Returns the
/// private `v8project.yaml`; the runner reads its sibling local file.
pub(super) fn write_reexport_config(
    root: &Path,
    directory: &Path,
    platform_line: &str,
) -> Result<PathBuf, String> {
    let root = normalize_path_identity(root).map_err(|error| error.to_string())?;
    let mut base =
        read_yaml_config(&root, BASE_NAME)?.ok_or_else(|| format!("{BASE_NAME} is absent"))?;
    let mut local =
        read_yaml_config(&root, LOCAL_NAME)?.unwrap_or_else(|| Value::Mapping(Mapping::new()));
    for (layer, file) in [(&mut base, BASE_NAME), (&mut local, LOCAL_NAME)] {
        let map = layer
            .as_mapping_mut()
            .ok_or_else(|| format!("{file} must be a mapping"))?;
        for key in [
            "workPath",
            "localPath",
            "infobases",
            "infobase",
            "tests",
            "mcp",
        ] {
            map.remove(Value::from(key));
        }
        if let Some(platform) = map
            .get_mut(Value::from("tools"))
            .and_then(Value::as_mapping_mut)
            .and_then(|tools| tools.get_mut(Value::from("platform")))
            .and_then(Value::as_mapping_mut)
        {
            platform.remove(Value::from("path"));
            platform.remove(Value::from("version"));
        }
        absolutize_shared_paths(layer, &root, file)?;
    }
    let base_map = base.as_mapping_mut().expect("checked mapping");
    base_map.insert(
        Value::from("workPath"),
        Value::from(directory.join("work").display().to_string()),
    );
    base_map.insert(
        Value::from("localPath"),
        Value::from(directory.join("local").display().to_string()),
    );
    let local_map = local.as_mapping_mut().expect("checked mapping");
    let mut origin = Mapping::new();
    origin.insert(
        Value::from("connection"),
        Value::from(format!("File=\"{}\"", directory.join("ib").display())),
    );
    let mut infobases = Mapping::new();
    infobases.insert(Value::from(ORIGIN), Value::Mapping(origin));
    local_map.insert(Value::from("infobases"), Value::Mapping(infobases));
    let tools = local_map
        .entry(Value::from("tools"))
        .or_insert_with(|| Value::Mapping(Mapping::new()))
        .as_mapping_mut()
        .ok_or_else(|| format!("tools in {LOCAL_NAME} must be a mapping"))?;
    tools
        .entry(Value::from("platform"))
        .or_insert_with(|| Value::Mapping(Mapping::new()))
        .as_mapping_mut()
        .ok_or_else(|| format!("tools.platform in {LOCAL_NAME} must be a mapping"))?
        .insert(Value::from("version"), Value::from(platform_line));
    std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    for (name, layer) in [(BASE_NAME, &base), (LOCAL_NAME, &local)] {
        let text = serde_yaml::to_string(layer).map_err(|error| error.to_string())?;
        std::fs::write(directory.join(name), text).map_err(|error| error.to_string())?;
    }
    Ok(directory.join(BASE_NAME))
}

/// The runner resolves relative paths from the directory of its config. In a
/// config written elsewhere the shared paths of a layer become absolute from
/// the working-copy root: the source sets and the tools the runner may start.
/// A bare EDT name without a directory is a discovery hint, not a path.
fn absolutize_shared_paths(layer: &mut Value, root: &Path, file: &str) -> Result<(), String> {
    if let Some(sets) = layer.get_mut("source-set").and_then(Value::as_sequence_mut) {
        for set in sets {
            absolutize(set, &["path"], root, file)?;
        }
    }
    for parts in [
        &["tools", "va", "epf_path"][..],
        &["tools", "designer_agent", "host-key"],
        &["tools", "designer_agent", "base-dir"],
        &["tools", "client_mcp", "extension", "source", "path"],
        &["tools", "client_mcp", "extension", "artifact", "path"],
    ] {
        absolutize(layer, parts, root, file)?;
    }
    if let Some(edt) = layer
        .get_mut("tools")
        .and_then(|tools| tools.get_mut("edt_cli"))
        .and_then(|edt| edt.get_mut("path"))
    {
        if let Some(text) = edt
            .as_str()
            .filter(|text| Path::new(text).components().count() > 1)
        {
            *edt = Value::from(root.join(text).display().to_string());
        }
    }
    Ok(())
}

fn absolutize(value: &mut Value, parts: &[&str], root: &Path, file: &str) -> Result<(), String> {
    let mut current = value;
    for part in parts {
        match current.get_mut(*part) {
            Some(next) => current = next,
            None => return Ok(()),
        }
    }
    if current.is_null() {
        return Ok(());
    }
    let text = current
        .as_str()
        .ok_or_else(|| format!("{} in {file} must be a path", parts.join(".")))?;
    *current = Value::from(root.join(text).display().to_string());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(base: &str, local: Option<&str>) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(BASE_NAME), base).unwrap();
        if let Some(local) = local {
            std::fs::write(root.path().join(LOCAL_NAME), local).unwrap();
        }
        root
    }

    fn local_after(base: &str, local: Option<&str>) -> Option<String> {
        let root = workspace(base, local);
        ensure_platform_line(root.path()).unwrap();
        std::fs::read_to_string(root.path().join(LOCAL_NAME)).ok()
    }

    const DESIGNER_MAIN: &str = "format: DESIGNER
source-set:
  - {name: main, type: CONFIGURATION, path: src}
";

    /// Проект конфигуратора с основным набором запускается на линии формата:
    /// Unica дописывает `tools.platform.version: sources` в местный файл,
    /// создаёт его при отсутствии и оставляет остальное как написано.
    /// rule: INV.RUNTIME.PLATFORM-LINE
    #[test]
    fn a_designer_main_set_runs_on_the_platform_line_of_its_format() {
        assert_eq!(
            local_after(DESIGNER_MAIN, None).as_deref(),
            Some(
                "tools:
  platform:
    version: sources
"
            )
        );
        // Без формата действует формат конфигуратора.
        assert!(local_after(
            "source-set:
  - {name: main, type: CONFIGURATION, path: src}
",
            None
        )
        .is_some());

        let local = "# machine-local
infobases:
  origin:
    connection: 'File=ib' # base
tools:
    platform:
        path: platform
    edt_cli:
        path: 1cedtcli
";
        assert_eq!(
            local_after(DESIGNER_MAIN, Some(local)).unwrap(),
            "# machine-local
infobases:
  origin:
    connection: 'File=ib' # base
tools:
    platform:
        version: sources
        path: platform
    edt_cli:
        path: 1cedtcli
"
        );
        let local = "infobases: {origin: {connection: 'File=ib'}}
tools:
  edt_cli:
    path: x
";
        assert_eq!(
            local_after(DESIGNER_MAIN, Some(local)).unwrap(),
            "infobases: {origin: {connection: 'File=ib'}}
tools:
  platform:
    version: sources
  edt_cli:
    path: x
"
        );
        // Секцию в строчной записи текстом не дописать: файл пишется заново
        // с тем же содержимым.
        let written = local_after(
            DESIGNER_MAIN,
            Some(
                "tools: {platform: {path: platform}}
",
            ),
        )
        .unwrap();
        let written: Value = serde_yaml::from_str(&written).unwrap();
        assert_eq!(written["tools"]["platform"]["path"], "platform");
        assert_eq!(written["tools"]["platform"]["version"], "sources");

        // Метка порядка байтов и пустая секция `tools` текст не ломают.
        assert_eq!(
            local_after(
                DESIGNER_MAIN,
                Some("\u{feff}tools:\n  edt_cli:\n    path: x\n")
            )
            .unwrap(),
            "\u{feff}tools:\n  platform:\n    version: sources\n  edt_cli:\n    path: x\n"
        );
        assert_eq!(
            local_after(DESIGNER_MAIN, Some("# local\ntools: ~ # later\n")).unwrap(),
            "# local\ntools:\n  platform:\n    version: sources\n"
        );
        // Пустая версия не задана, как и для раннера.
        let written =
            local_after(DESIGNER_MAIN, Some("tools:\n  platform:\n    version: ~\n")).unwrap();
        let written: Value = serde_yaml::from_str(&written).unwrap();
        assert_eq!(written["tools"]["platform"]["version"], "sources");
    }

    /// Заданная человеком версия и проекты, где раннер строгий выбор не
    /// принимает, файл не меняют.
    /// rule: INV.RUNTIME.PLATFORM-LINE
    #[test]
    fn the_platform_line_stays_off_where_a_version_is_set_or_the_runner_refuses_it() {
        let pinned = "tools:
  platform:
    version: '8.3.24'
";
        assert_eq!(
            local_after(DESIGNER_MAIN, Some(pinned)).as_deref(),
            Some(pinned)
        );
        let pinned_base = format!(
            "{DESIGNER_MAIN}tools: {{platform: {{version: '8.3.27'}}}}
"
        );
        assert_eq!(local_after(&pinned_base, None), None);
        for base in [
            "format: EDT
source-set:
  - {name: main, type: CONFIGURATION, path: src}
",
            "format: DESIGNER
source-set:
  - {name: ext, type: EXTENSION, path: ext}
",
            "format: DESIGNER
",
            // Раннер читает формат и вид набора только прописными.
            "format: designer
source-set:
  - {name: main, type: CONFIGURATION, path: src}
",
            "format: DESIGNER
source-set:
  - {name: main, type: configuration, path: src}
",
        ] {
            assert_eq!(local_after(base, None), None, "{base}");
        }
        // Конфигурацию, которую раннер не прочтёт, Unica не трогает.
        let broken = "tools: [platform]
";
        assert_eq!(
            local_after(DESIGNER_MAIN, Some(broken)).as_deref(),
            Some(broken)
        );
    }

    /// Подготовка операции под замком: строгий выбор платформы записан до
    /// того, как операция снимет свои входы, а приватная копия конфигурации
    /// прежних версий Unica с секретами убрана.
    /// rule: INV.RUNTIME.PLATFORM-LINE
    #[test]
    fn an_operation_prepares_the_working_copy_before_reading_its_inputs() {
        let root = workspace(
            DESIGNER_MAIN,
            Some("infobases: {origin: {connection: 'File=ib', password: secret}}\n"),
        );
        let former = root.path().join(".build/unica/runner-project");
        std::fs::create_dir_all(&former).unwrap();
        std::fs::write(former.join(LOCAL_NAME), "password: secret\n").unwrap();
        // Ответ называет дописанный файл и ключ; повтор ничего не пишет и
        // ничего не сообщает.
        let warnings =
            prepare_working_copy(root.path(), "push", &CancellationToken::new()).unwrap();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0]["code"], "platform_line_written");
        let message = warnings[0]["message"].as_str().unwrap();
        assert!(
            message.contains("v8project.local.yaml")
                && message.contains("tools.platform.version: sources"),
            "{message}"
        );
        assert!(!former.exists());
        let local: Value =
            serde_yaml::from_str(&std::fs::read_to_string(root.path().join(LOCAL_NAME)).unwrap())
                .unwrap();
        assert_eq!(local["tools"]["platform"]["version"], "sources");
        assert_eq!(local["infobases"]["origin"]["password"], "secret");

        // Повтор ничего не меняет: входы следующей операции те же.
        let before = std::fs::read(root.path().join(LOCAL_NAME)).unwrap();
        assert!(
            prepare_working_copy(root.path(), "push", &CancellationToken::new())
                .unwrap()
                .is_empty()
        );
        assert_eq!(std::fs::read(root.path().join(LOCAL_NAME)).unwrap(), before);

        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let refused = prepare_working_copy(root.path(), "push", &cancelled).unwrap_err();
        assert_eq!(refused.diagnostics[0]["code"], "cancelled", "{refused:?}");
    }

    /// Файл только для чтения Unica не переписывает и операцию не держит:
    /// ответ говорит, что строгий выбор не включён, временного файла с
    /// секретами не остаётся.
    /// rule: INV.RUNTIME.PLATFORM-LINE
    #[test]
    fn a_read_only_local_file_stays_and_the_answer_says_so() {
        let local = "infobases: {origin: {connection: 'File=ib', password: secret}}\n";
        let root = workspace(DESIGNER_MAIN, Some(local));
        let path = root.path().join(LOCAL_NAME);
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions.clone()).unwrap();
        let warnings =
            prepare_working_copy(root.path(), "push", &CancellationToken::new()).unwrap();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0]["code"], "platform_line_not_written");
        assert!(warnings[0]["message"]
            .as_str()
            .unwrap()
            .contains("read-only"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), local);
        let names: Vec<_> = std::fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert!(
            !names
                .iter()
                .any(|name| name.to_string_lossy().ends_with(".tmp")),
            "{names:?}"
        );
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(&path, permissions).unwrap();
    }

    /// Файл, переписанный заново, ответ так и называет: комментариев в нём
    /// больше нет.
    #[test]
    fn a_rewritten_local_file_is_named_so_in_the_answer() {
        let root = workspace(DESIGNER_MAIN, Some("tools: {platform: {path: p}} # mine\n"));
        let warnings =
            prepare_working_copy(root.path(), "push", &CancellationToken::new()).unwrap();
        assert_eq!(warnings[0]["code"], "platform_line_written");
        assert!(warnings[0]["message"]
            .as_str()
            .unwrap()
            .contains("written anew"));
    }

    /// Местный файл-ссылкой Unica не читает и не пишет: ссылка остаётся
    /// ссылкой, а файл, на который она указывает, — прежним.
    #[test]
    fn a_linked_local_file_is_left_as_it_is() {
        use unica_workspace::test_support::{
            create_file_link_fixture_for_test, FileLinkFixtureOutcome,
        };
        let root = workspace(DESIGNER_MAIN, None);
        let shared = tempfile::tempdir().unwrap();
        let target = shared.path().join("shared.yaml");
        std::fs::write(&target, "tools:\n  edt_cli:\n    path: x\n").unwrap();
        if create_file_link_fixture_for_test(&target, root.path().join(LOCAL_NAME)).unwrap()
            != FileLinkFixtureOutcome::Created
        {
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::Symlink,
                "Создание ссылки требует полномочий ОС",
            );
            return;
        }
        assert_eq!(
            ensure_platform_line(root.path()).unwrap(),
            PlatformLine::Untouched
        );
        let link = std::fs::symlink_metadata(root.path().join(LOCAL_NAME)).unwrap();
        assert!(link.file_type().is_symlink());
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "tools:
  edt_cli:
    path: x
"
        );
    }

    /// Вместе с прежней копией уходит и то, что прежнее хранилище держало
    /// рядом: каталог его замка и остатки прерванного удаления.
    #[test]
    fn the_former_store_leaves_nothing_beside_the_copy() {
        use sha2::{Digest, Sha256};
        let root = workspace(DESIGNER_MAIN, Some("tools: {platform: {version: '8.3'}}\n"));
        let unica = root.path().join(".build/unica");
        std::fs::create_dir_all(unica.join("runner-project")).unwrap();
        let lock = unica.join(format!(
            ".runner-project-lock-{:x}",
            Sha256::digest(b"runner-project")
        ));
        std::fs::create_dir_all(&lock).unwrap();
        std::fs::write(lock.join(".runner-project.lock"), "").unwrap();
        let cleanup = unica.join(".runner-project-cleanup-1");
        std::fs::create_dir_all(&cleanup).unwrap();
        std::fs::write(cleanup.join(LOCAL_NAME), "password: secret\n").unwrap();
        assert!(
            prepare_working_copy(root.path(), "push", &CancellationToken::new())
                .unwrap()
                .is_empty()
        );
        assert!(!unica.join("runner-project").exists());
        assert!(!lock.exists());
        assert!(!cleanup.exists());
    }

    /// Ответ называет живого агента команды из `provider.endpoint.live`
    /// раннера: взят ли он, остаётся ли и почему нет, сколько у него
    /// держателей и держит ли его Unica. Номера чужих держателей наружу не идут.
    /// The runner that a cancel interrupted answered it itself. Its answer
    /// goes to the adapter and is the outcome of the call, unless it is the
    /// cancel; a run that was not interrupted, or killed, stays cancelled.
    /// rule: INV.APP.ADMISSION-FAILURE-ISOLATION
    #[test]
    fn the_answer_of_an_interrupted_runner_is_the_outcome_of_the_call() {
        use unica_work_control::cancellation::FactualOutcomeAuthority;
        let answered = |stdout: serde_json::Value, interrupted: bool| {
            let runner = PinnedRunner::default();
            let cancellation = CancellationToken::new();
            cancellation.cancel();
            let command = ProcessCommand {
                program: "v8-runner".into(),
                args: Vec::new(),
                cwd: std::env::temp_dir(),
                env: Vec::new(),
                env_remove: Vec::new(),
                capture_limits: None,
                timeout: None,
                cancellation: cancellation.clone(),
            };
            let before = cancellation.interrupt_answers();
            if interrupted {
                cancellation.note_interrupt_answered();
            }
            let mut output = ProcessOutput {
                status_success: false,
                status: "exit code: 4".to_string(),
                stdout: stdout.to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: true,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            };
            runner.take_answer_to_interrupt(&command, before, &mut output);
            (
                output.cancelled,
                cancellation.factual_outcome_authority(),
                runner.abandoned(),
            )
        };
        let abandoned = serde_json::json!({
            "ok": false, "error": {"code": "agent_command_abandoned", "kind": "platform"}
        });
        assert_eq!(
            answered(abandoned.clone(), true),
            (false, FactualOutcomeAuthority::AnsweredInterrupt, true)
        );
        assert_eq!(
            answered(serde_json::json!({"ok": true, "data": {}}), true),
            (false, FactualOutcomeAuthority::AnsweredInterrupt, false)
        );
        let cancelled = serde_json::json!({"ok": false, "error": {"code": "cancelled"}});
        assert_eq!(
            answered(cancelled, true),
            (true, FactualOutcomeAuthority::Cancellable, false)
        );
        assert_eq!(
            answered(abandoned, false),
            (true, FactualOutcomeAuthority::Cancellable, false)
        );
        assert_eq!(
            answered(serde_json::json!("not an envelope"), true),
            (true, FactualOutcomeAuthority::Cancellable, false)
        );
    }

    /// After an abandoned command of the agent of a standalone server the
    /// answer says how to release the infobase; elsewhere it says nothing.
    #[test]
    fn an_abandoned_command_on_a_standalone_server_names_the_restart() {
        let standalone = tempfile::tempdir().unwrap();
        std::fs::write(
            standalone.path().join("v8project.yaml"),
            "infobases: {origin: {connection: 'Srvr=srv:1541;Ref=demo', standalone: {}}}\n",
        )
        .unwrap();
        let file = tempfile::tempdir().unwrap();
        std::fs::write(
            file.path().join("v8project.yaml"),
            "infobases: {origin: {connection: 'File=ib'}}\n",
        )
        .unwrap();
        let runner = PinnedRunner::default();
        assert_eq!(abandoned_warning(standalone.path(), &runner), None);
        runner
            .abandoned
            .store(true, std::sync::atomic::Ordering::Release);
        assert_eq!(
            abandoned_warning(standalone.path(), &runner),
            Some(serde_json::json!({
                "code": "agent_command_abandoned_standalone",
                "message": ABANDONED_STANDALONE,
            }))
        );
        assert_eq!(abandoned_warning(file.path(), &runner), None);
    }

    #[test]
    fn the_answer_names_the_live_agent_of_the_command() {
        let envelope = |live: serde_json::Value| {
            serde_json::json!({"ok": true, "data": {"provider": {
                "selected": "designer",
                "endpoint": {"mode": "managed", "address": "127.0.0.1:1543", "live": live}
            }}})
            .to_string()
        };
        let own = std::process::id();
        let held = agent_receipt(&envelope(serde_json::json!({
            "reused": true, "kept": true,
            "holders": [{"pid": own, "started_at": "2026-11-13T08:00:00Z"}, {"pid": 4242}]
        })))
        .unwrap();
        assert_eq!(
            held,
            serde_json::json!({"reused": true, "kept": true, "holders": 2, "heldByUnica": true})
        );
        assert!(!held.to_string().contains("4242"));
        let one_off = agent_receipt(&envelope(serde_json::json!({
            "reused": false, "kept": false, "holders": [], "reason": "job"
        })))
        .unwrap();
        assert_eq!(
            one_off,
            serde_json::json!({
                "reused": false, "kept": false, "holders": 0, "heldByUnica": false, "reason": "job"
            })
        );
        // Без сессии управляемого агента ответ агента не называет.
        assert_eq!(
            agent_receipt(r#"{"ok": true, "data": {"provider": {"selected": "ibcmd"}}}"#),
            None
        );
        assert_eq!(agent_receipt("not json"), None);
    }

    #[test]
    fn the_former_projection_lies_beside_the_lock() {
        let root = Path::new("/work/a");
        assert_eq!(
            former_projection_directory_in(root, None),
            Path::new("/work/a/.build/unica/runner-project")
        );
        let cache = std::env::temp_dir().join("cache");
        let lock = lock_directory_in(root, Some(cache.clone()));
        let former = former_projection_directory_in(root, Some(cache.clone()));
        assert_eq!(former.parent().unwrap(), cache.join("runner-project"));
        assert_eq!(former.file_name(), lock.file_name());
    }

    /// Свой замок у каждой рабочей копии: по умолчанию в её кеше, а в общем
    /// кеше `UNICA_CACHE_DIR` — под ключом корня копии.
    #[test]
    fn each_working_copy_has_its_own_lock_directory() {
        assert_eq!(
            lock_directory_in(Path::new("/work/a"), None),
            Path::new("/work/a/.build/unica/runner-lock")
        );
        let cache = std::env::temp_dir().join("cache");
        let a = lock_directory_in(Path::new("/work/a"), Some(cache.clone()));
        let b = lock_directory_in(Path::new("/work/b"), Some(cache.clone()));
        assert!(a.starts_with(cache.join("runner-lock")), "{a:?}");
        assert_ne!(a, b);
        assert_eq!(a, lock_directory_in(Path::new("/work/a"), Some(cache)));
    }

    /// Вид базы `origin` читается так же, как его читает раннер: секции обоих
    /// слоёв сливаются (местный побеждает), `standalone` — автономный сервер,
    /// строка с `File=` — файловая база, иначе кластер.
    /// rule: INV.WIRE.INFOBASE-CREATE-RECREATES-BY-CHOICE
    #[test]
    fn the_origin_kind_follows_the_merged_infobase_section() {
        for (base, local, kind) in [
            ("workPath: build\n", None, OriginKind::Undeclared),
            (
                "infobase: {connection: 'File=ib'}\n",
                None,
                OriginKind::File,
            ),
            (
                "workPath: build\n",
                Some("infobases: {origin: {connection: 'File=\"/abs/ib\"'}}\n"),
                OriginKind::File,
            ),
            (
                "infobase: {connection: 'File=ib'}\n",
                Some("infobases: {origin: {connection: 'Srvr=srv;Ref=demo'}}\n"),
                OriginKind::Cluster,
            ),
            (
                "workPath: build\n",
                Some(
                    "infobases: {origin: {connection: 'Srvr=srv:1541;Ref=demo', standalone: {}}}\n",
                ),
                OriginKind::Standalone,
            ),
        ] {
            let root = workspace(base, local);
            assert_eq!(origin_kind(root.path()).unwrap(), kind, "{base} {local:?}");
        }
        // Разбор строки — как у раннера: пробелы вокруг ключа и регистр не
        // важны, часть без `=` делает строку не файловой.
        for (connection, file) in [
            ("File = build/ib", true),
            ("fIlE  =  'db'", true),
            ("File=ib;", true),
            ("File=ib;junk", false),
            ("File=", false),
            ("Srvr=srv;Ref=demo", false),
            ("/F /abs/ib", true),
        ] {
            assert_eq!(declares_a_file_infobase(connection), file, "{connection}");
        }
    }

    #[test]
    fn unknown_runner_versions_are_not_probed_by_executing_an_operation() {
        assert!(check_version(VERSION).is_ok());
        assert!(check_version("0.17.1").is_ok());
        for version in [
            "0.9.0",
            "0.11.3",
            "0.11.4",
            "0.12.0",
            "0.13.0",
            "0.14.0",
            "0.15.0",
            "0.15.1",
            "0.15.2",
            "0.15.3",
            "0.16.0",
            "0.17.0",
            "0.17.1-rc.1",
            "0.17.2",
            "1.0.0",
            "1.0.0-rc.1",
            "",
        ] {
            assert!(check_version(version).is_err());
        }
    }
}

#[cfg(test)]
mod reexport_config_tests {
    use super::*;

    /// The config of a re-export keeps the project's sets at their own
    /// directories and names a temporary file infobase, the runner's own
    /// working files and personal state and the platform line of the target
    /// format; credentials and the platform location are not carried over and
    /// the project files are not written.
    /// rule: INV.SOURCE.WRITABLE-PROFILE
    #[test]
    fn a_reexport_config_points_at_a_temporary_infobase_and_the_target_line() {
        let root = tempfile::tempdir().unwrap();
        let base = "format: DESIGNER\nlocalPath: personal\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n";
        let local = "infobases:\n  origin:\n    connection: 'File=base'\n    user: Admin\n    password: secret\ntools:\n  platform:\n    path: 'C:/Program Files/1cv8/8.3.24'\n    version: '8.3.24'\nproviders:\n  pull: designer\n";
        std::fs::write(root.path().join(BASE_NAME), base).unwrap();
        std::fs::write(root.path().join(LOCAL_NAME), local).unwrap();
        let work = root.path().join(".build/unica/reexport/one");
        let config = write_reexport_config(root.path(), &work, "8.5.4").unwrap();
        assert_eq!(config, work.join(BASE_NAME));
        let projected: Value =
            serde_yaml::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
        let private_text = std::fs::read_to_string(work.join(LOCAL_NAME)).unwrap();
        let private: Value = serde_yaml::from_str(&private_text).unwrap();
        let root_identity = normalize_path_identity(root.path()).unwrap();
        assert_eq!(
            projected["source-set"][0]["path"]
                .as_str()
                .map(PathBuf::from),
            Some(root_identity.join("src"))
        );
        assert_eq!(
            projected["workPath"].as_str().map(PathBuf::from),
            Some(work.join("work"))
        );
        assert_eq!(
            projected["localPath"].as_str().map(PathBuf::from),
            Some(work.join("local"))
        );
        assert_eq!(
            private["infobases"]["origin"]["connection"].as_str(),
            Some(format!("File=\"{}\"", work.join("ib").display()).as_str())
        );
        assert!(private["infobases"]["origin"].get("user").is_none());
        assert!(!private_text.contains("secret"));
        assert_eq!(
            private["tools"]["platform"]["version"],
            Value::from("8.5.4")
        );
        assert!(private["tools"]["platform"].get("path").is_none());
        assert_eq!(private["providers"]["pull"], Value::from("designer"));
        assert_eq!(
            std::fs::read_to_string(root.path().join(BASE_NAME)).unwrap(),
            base
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join(LOCAL_NAME)).unwrap(),
            local
        );
    }
}
