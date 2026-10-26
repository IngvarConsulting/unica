//! Caller-owned publication over the runner's versioned process protocol.
use super::*;
use serde::Deserialize;
use std::time::Duration;
use unica_process::runner::ProcessStreamFailure;
use unica_process::ProcessLineReply;
use unica_workspace::full_dump::{
    FullDumpCleanupReason, FullDumpFailure, FullDumpPhase, PublishedFullDump,
    RunnerProjectLockFailure,
};

const PROTOCOL_VERSION: u32 = 1;
const FINISH_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ExecutorKind {
    Local,
    Attached,
    Remote,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Executor {
    kind: ExecutorKind,
    executable: Option<PathBuf>,
    endpoint: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Publication {
    NotPublished,
    Published,
    Unknown,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Memory {
    Unchanged,
    Saved,
    NotSaved,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Snapshot {
    Prepared,
    Unavailable,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
enum Frame {
    Selected {
        version: u32,
        nonce: String,
        provider: Value,
        executor: Executor,
    },
    Ready {
        version: u32,
        nonce: String,
        command: String,
        requested_mode: String,
        mode: String,
        mode_reason: Option<String>,
        extension: Option<String>,
        provider_dispatched: bool,
        source_set: String,
        target_path: PathBuf,
        stage_path: PathBuf,
        snapshot: Snapshot,
        snapshot_warning: Option<String>,
    },
    Terminal {
        version: u32,
        nonce: String,
        publication: Publication,
        memory: Memory,
        result: Value,
    },
}

impl Frame {
    fn identity(&self) -> (u32, &str) {
        match self {
            Self::Selected { version, nonce, .. }
            | Self::Ready { version, nonce, .. }
            | Self::Terminal { version, nonce, .. } => (*version, nonce),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Phase {
    Selected,
    Ready,
    Terminal,
    Done,
}

pub(super) fn execute(
    prepared: &PreparedSourceExport,
    runner: &dyn ProcessRunner,
    cancellation: CancellationToken,
    tool: &BundledTool,
) -> DomainResult {
    if cancellation.is_cancelled() {
        return reject(RefusalCode::Cancelled, "pull cancelled before preparation");
    }
    let inputs = match capture_inputs(prepared) {
        Ok(value) => value,
        Err(result) => return result,
    };
    let root = match normalize_path_identity(&prepared.context.workspace_root) {
        Ok(value) => value,
        Err(_) => {
            return reject(
                RefusalCode::InvalidState,
                "pull could not resolve the workspace root",
            )
        }
    };
    let expected_target = if prepared.dry_run {
        None
    } else {
        let mut preview_request = prepared.clone();
        preview_request.dry_run = true;
        let preview = execute(&preview_request, runner, cancellation.clone(), tool);
        if !preview.ok {
            return preview;
        }
        match capture_inputs(prepared) {
            Ok(current) if current == inputs => {}
            _ => {
                return reject(
                    RefusalCode::ConcurrentChange,
                    "pull inputs changed during its internal preview",
                )
            }
        }
        let relative = preview
            .data
            .as_ref()
            .and_then(|data| data["plan"]["target"].as_str());
        match relative {
            Some(path) => Some(root.join(path)),
            None => return invalid("pull preview omitted its guarded target"),
        }
    };
    let store = match lock_project(&prepared.context.workspace_root, &cancellation) {
        Ok(value) => value,
        Err(RunnerProjectLockFailure::Cancelled) => {
            return reject(
                RefusalCode::Cancelled,
                "pull cancelled while waiting for private project ownership",
            )
        }
        Err(RunnerProjectLockFailure::InvalidState(_)) => {
            return reject(
                RefusalCode::InvalidState,
                "pull could not acquire private project ownership",
            )
        }
    };
    let mut guard = match unica_adapter_v8_runner::full_dump::prepare_publication(
        &prepared.context,
        FullDumpSelection {
            config_path: &prepared.context.workspace_root.join(CONFIG_NAME),
            source_set: prepared.arguments.source_set.as_deref(),
            extension: prepared.arguments.extension.as_deref(),
            expected_target: expected_target.as_deref(),
        },
        store,
        runner,
        &cancellation,
        normalize_full_export_config,
        project_full_export_config,
        crate::infrastructure::runtime_composition::full_dump_semantics(),
    ) {
        Ok(value) => value,
        Err(error) => return failure(&root, error),
    };
    let target = guard.target().to_path_buf();
    let stage = guard.staged_target().to_path_buf();
    let source_set = guard.source_set().to_string();
    let expected_extension = guard.selected_extension().map(str::to_owned);
    let mut args = vec![
        "--config".into(),
        guard.effective_config().display().to_string(),
        RUNNER_COMMAND.into(),
        source_set.clone(),
        "--force".into(),
        "--publication-stdio".into(),
        stage.display().to_string(),
    ];
    if let Some(extension) = &prepared.arguments.extension {
        args.extend(["--extension".into(), extension.clone()]);
    }
    if prepared.dry_run {
        args.push("--dry-run".into());
    }
    let command = ProcessCommand {
        program: tool.program.clone(),
        args,
        cwd: root.clone(),
        env: Vec::new(),
        env_remove: Vec::new(),
        capture_limits: Some((RUNNER_OUTPUT_LIMIT, RUNNER_OUTPUT_LIMIT)),
        timeout: None,
        cancellation: cancellation.clone(),
    };

    let mut phase = Phase::Selected;
    let mut nonce: Option<String> = None;
    let mut problem = None;
    let mut published: Option<PublishedFullDump> = None;
    let mut authorized = false;
    let mut terminal = None;
    let mut ready_mode_reason = None;
    guard.begin_execution();
    let output = runner.run_controlled_streaming_staged(&command, RUNNER_OUTPUT_LIMIT, &mut |_, bytes| {
        let frame: Frame = match serde_json::from_slice(bytes) {
            Ok(value) => value,
            Err(_) => { problem = Some(invalid("v8-runner returned an invalid publication frame")); return ProcessLineReply::Stop; }
        };
        let (version, frame_nonce) = frame.identity();
        if version != PROTOCOL_VERSION || uuid::Uuid::parse_str(frame_nonce).is_err()
            || nonce.as_deref().is_some_and(|expected| expected != frame_nonce) {
            problem = Some(invalid("v8-runner publication version or nonce does not match"));
            return ProcessLineReply::Stop;
        }
        nonce.get_or_insert_with(|| frame_nonce.to_owned());
        match frame {
            Frame::Selected { provider, executor, .. } if phase == Phase::Selected => {
                if executor.kind != ExecutorKind::Local || executor.endpoint.is_some()
                    || !matches!(provider["selected"].as_str(), Some("designer" | "ibcmd" | "agent")) {
                    problem = Some(DomainResult::canonical_rejection_detailed(
                        Some(OPERATION.to_owned()),
                        unica_refusal::RefusalDetail::ProviderAbsent,
                        "pull requires a verifiable local platform executor",
                    ));
                    phase = Phase::Terminal;
                    return reply(nonce.as_deref().unwrap(), "selected", "reject");
                }
                let Some(executable) = executor.executable else {
                    problem = Some(invalid("v8-runner omitted its selected platform executable"));
                    phase = Phase::Terminal;
                    return reply(nonce.as_deref().unwrap(), "selected", "reject");
                };
                let check = guard.recheck_before_dispatch()
                    .and_then(|()| unica_adapter_v8_runner::full_dump::attest_selected_local(&mut guard, &executable, runner, &cancellation))
                    .and_then(|()| if prepared.dry_run { Ok(()) } else { guard.prepare_dispatch_parent(&cancellation) });
                if let Err(error) = check {
                    problem = Some(failure(&root, error));
                    phase = Phase::Terminal;
                    return reply(nonce.as_deref().unwrap(), "selected", "reject");
                }
                if cancellation.is_cancelled() {
                    problem = Some(reject(RefusalCode::Cancelled, "pull cancelled before executor authorization"));
                    phase = Phase::Terminal;
                    return reply(nonce.as_deref().unwrap(), "selected", "reject");
                }
                phase = if prepared.dry_run { Phase::Terminal } else { Phase::Ready };
                authorized = true;
                reply(nonce.as_deref().unwrap(), "selected", "continue")
            }
            Frame::Ready { command, requested_mode, mode, mode_reason: reported_reason, extension,
                provider_dispatched, source_set: reported_set, target_path, stage_path, snapshot, snapshot_warning, .. }
                if phase == Phase::Ready => {
                if command != RUNNER_COMMAND || requested_mode != "FULL" || mode != "FULL"
                    || extension != expected_extension || !provider_dispatched
                    || reported_set != source_set || target_path != target || stage_path != stage {
                    problem = Some(invalid("v8-runner prepared a different operation, mode, extension, source set or target"));
                    phase = Phase::Terminal;
                    return reply(nonce.as_deref().unwrap(), "ready", "rejected");
                }
                ready_mode_reason = match mode_reason(&reported_reason.map(Value::String).unwrap_or(Value::Null)) {
                    Ok(reason) => reason,
                    Err(()) => {
                        problem = Some(invalid("v8-runner reported an unknown export mode reason"));
                        phase = Phase::Terminal;
                        return reply(nonce.as_deref().unwrap(), "ready", "rejected");
                    }
                };
                if snapshot != Snapshot::Prepared || snapshot_warning.is_some() {
                    problem = Some(reject(RefusalCode::ProviderFailed,
                        "v8-runner could not prepare synchronization memory; sources were not published"));
                    phase = Phase::Terminal;
                    return reply(nonce.as_deref().unwrap(), "ready", "rejected");
                }
                if let Err(error) = guard.capture_ready_staged_tree() {
                    problem = Some(failure(&root, error));
                    phase = Phase::Terminal;
                    return reply(nonce.as_deref().unwrap(), "ready", "rejected");
                }
                match capture_inputs(prepared) {
                    Ok(current) if current == inputs => {},
                    _ => {
                        problem = Some(reject(RefusalCode::ConcurrentChange, "pull inputs changed before publication"));
                        phase = Phase::Terminal;
                        return reply(nonce.as_deref().unwrap(), "ready", "rejected");
                    }
                }
                match guard.publish(&cancellation) {
                    Ok(proof) => {
                        published = Some(proof);
                        phase = Phase::Terminal;
                        ProcessLineReply::ReplyAndProtect { reply: reply_bytes(nonce.as_deref().unwrap(), "ready", "published"),
                            finish_timeout: FINISH_TIMEOUT }
                    }
                    Err(error) => {
                        let uncertain = error.publication_uncertain;
                        problem = Some(failure(&root, error));
                        phase = Phase::Terminal;
                        if uncertain { ProcessLineReply::Stop }
                        else { reply(nonce.as_deref().unwrap(), "ready", "rejected") }
                    }
                }
            }
            Frame::Terminal { publication, memory, result, .. } if phase != Phase::Done => {
                let expected = if published.is_some() { Publication::Published } else { Publication::NotPublished };
                if publication != expected || (published.is_none() && memory != Memory::Unchanged)
                    || result["command"] != RUNNER_COMMAND || (result["ok"] == true && !authorized) {
                    problem.get_or_insert_with(|| invalid("v8-runner returned an inconsistent publication outcome"));
                }
                if published.is_some() {
                    let checked = validate_runner_result(result.clone(), result["ok"] == true)
                        .and_then(|envelope| validate_envelope(prepared, &inputs, &root, &envelope, true));
                    match checked {
                        Ok(plan) if plan.source_set == source_set && plan.target_absolute == target
                            && plan.performed_mode == "full" && plan.mode_reason == ready_mode_reason => {},
                        Ok(_) => { problem.get_or_insert_with(|| invalid("v8-runner terminal result differs from its prepared publication")); },
                        Err(error) => { problem.get_or_insert(error); },
                    }
                }
                terminal = Some((memory, result));
                phase = Phase::Done;
                ProcessLineReply::Continue
            }
            _ => { problem.get_or_insert_with(|| invalid("v8-runner publication frame is out of order")); ProcessLineReply::Stop }
        }
    });

    let clean_exit = output.as_ref().is_ok_and(|out| {
        out.status_success
            && !out.timed_out
            && !out.cancelled
            && !out.stopped_by_consumer
            && out.line_error.is_none()
    });
    if matches!(&output, Err(ProcessStreamFailure::Spawn(_))) {
        guard.confirm_execution_finished();
    }

    if let Some(proof) = published {
        if clean_exit && phase == Phase::Done && terminal.is_some() && problem.is_none() {
            guard.confirm_execution_finished();
        }
        // The local publication is the receipt's authority even if ACK, terminal
        // output, cancellation or the runner's memory finish subsequently failed.
        let mut result = published_result(
            prepared,
            &root,
            &target,
            &source_set,
            proof.replaced,
            proof.files,
        );
        if let Some(data) = result.data.as_mut() {
            data["modeReason"] = json!(ready_mode_reason);
        }
        if let Some(mut error) = problem.take() {
            error.changed = result.changed;
            error.data = result.data;
            result = error;
        }
        if !proof.warnings.is_empty() {
            cleanup_warning(&mut result, FullDumpCleanupReason::PublicationCleanup);
        }
        let saved = terminal
            .as_ref()
            .is_some_and(|(memory, envelope)| *memory == Memory::Saved && envelope["ok"] == true)
            && result.ok
            && output.as_ref().is_ok_and(|out| {
                out.status_success
                    && !out.timed_out
                    && !out.cancelled
                    && !out.stopped_by_consumer
                    && out.line_error.is_none()
            });
        if !saved {
            warn(&mut result, "sync_memory_unconfirmed",
            "Sources were published; synchronization memory was not confirmed. Repeat a full pull to refresh it.");
        }
        if let Some((_, envelope)) = &terminal {
            note_another_copy(&mut result, envelope);
        }
        match guard.finish() {
            Ok(finish) => {
                report_recovery(
                    &root,
                    &mut result,
                    finish.retained,
                    finish.recovery_origin,
                    finish.cleanup_reasons,
                );
            }
            Err(error) => {
                report_recovery(
                    &root,
                    &mut result,
                    error.recovery,
                    error.report.recovery_origin,
                    error.report.cleanup_reasons,
                );
            }
        }
        return result;
    }

    let mut result = if let Some(problem) = problem {
        problem
    } else {
        match output {
            Err(error) => runner_start_rejection(Some(OPERATION.into()), &error, &tool.program),
            Ok(out) if out.cancelled => {
                reject(RefusalCode::Cancelled, "pull cancelled before publication")
            }
            Ok(out) if out.timed_out => reject(
                RefusalCode::DeadlineExceeded,
                "pull timed out before publication",
            ),
            Ok(out) if out.line_error.is_some() || out.stopped_by_consumer => {
                invalid("v8-runner returned an unreadable publication stream")
            }
            Ok(out) => match terminal {
                Some((_, envelope)) => match validate_runner_result(envelope, out.status_success) {
                    Err(result) => result,
                    Ok(envelope) if prepared.dry_run && phase == Phase::Done => {
                        match validate_envelope(prepared, &inputs, &root, &envelope, false) {
                            Ok(plan)
                                if plan.target_absolute == target
                                    && plan.source_set == source_set =>
                            {
                                preview_result(prepared, plan, &envelope)
                            }
                            Err(error) => error,
                            _ => {
                                invalid("v8-runner preview does not match the guarded export plan")
                            }
                        }
                    }
                    Ok(_) => {
                        invalid("v8-runner completed without publishing its prepared source tree")
                    }
                },
                None => invalid("v8-runner ended without a publication outcome"),
            },
        }
    };
    if result.ok && clean_exit && phase == Phase::Done {
        // A successful preview reaches this point only after its envelope and
        // exact source selection passed the checks above.
        guard.confirm_execution_finished();
    }
    if let Err(error) = guard.abort() {
        if error.publication_uncertain {
            let validation = result
                .data
                .as_ref()
                .and_then(|data| data.get("validation"))
                .cloned();
            result = failure(&root, error);
            if let Some(report) = validation {
                attach_validation(&mut result, report);
            }
        } else {
            report_recovery(
                &root,
                &mut result,
                error.recovery,
                error.report.recovery_origin,
                error.report.cleanup_reasons,
            );
        }
    }
    result
}

fn reply(nonce: &str, phase: &str, decision: &str) -> ProcessLineReply {
    ProcessLineReply::Reply(reply_bytes(nonce, phase, decision))
}
fn reply_bytes(nonce: &str, phase: &str, decision: &str) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(
        &json!({ "version":PROTOCOL_VERSION,"nonce":nonce,"phase":phase,"decision":decision }),
    )
    .expect("closed string publication reply is serializable");
    bytes.push(b'\n');
    bytes
}
fn invalid(message: &str) -> DomainResult {
    reject(RefusalCode::InvalidResult, message)
}
fn warn(result: &mut DomainResult, code: &str, message: impl AsRef<str>) {
    result
        .warnings
        .push(json!({"code":code,"message":redactor(message.as_ref())}));
}
fn failure(root: &Path, error: FullDumpFailure) -> DomainResult {
    let code = match error.phase {
        FullDumpPhase::Cancelled => RefusalCode::Cancelled,
        FullDumpPhase::Selection => RefusalCode::BadValue,
        FullDumpPhase::Integrity => RefusalCode::ConcurrentChange,
        FullDumpPhase::Validation => RefusalCode::InvalidResult,
        FullDumpPhase::Preparation | FullDumpPhase::Publication | FullDumpPhase::Cleanup => {
            RefusalCode::InvalidState
        }
    };
    let mut result = reject(code, error.to_string());
    if let Some(validation) = &error.report.validation {
        result.data = Some(json!({"op":OPERATION,"state":"notPublished"}));
        attach_validation(&mut result, validation_report(validation));
    }
    if error.publication_uncertain {
        let relative = match &error.recovery {
            Ok(Some(recovery)) => retained_path(root, recovery),
            Ok(None) | Err(_) => None,
        };
        let data = result.data.get_or_insert_with(|| json!({"op":OPERATION}));
        data["state"] = json!("unknown");
        data["recovery"] = json!(relative);
    }
    report_recovery(
        root,
        &mut result,
        error.recovery,
        error.report.recovery_origin,
        error.report.cleanup_reasons,
    );
    result
}

fn attach_validation(result: &mut DomainResult, report: Value) {
    let message = format!("{}; validation: {}", result.summary, report);
    result.summary = message.clone();
    if let Some(diagnostic) = result.diagnostics.first_mut() {
        diagnostic["message"] = json!(message);
    }
    let data = result.data.get_or_insert_with(|| json!({"op":OPERATION}));
    data["validation"] = report;
}

fn validation_report(
    error: &unica_source_contract::read::full_dump::DumpValidationFailure,
) -> Value {
    let mut report = json!({"reason":error.reason});
    if let Some(path) = error.path.as_deref() {
        report["path"] = json!(safe_relative_path(path));
    }
    if let Some((namespace, local_name)) = error.root.as_deref() {
        // The format owner owns the namespace vocabulary. Unknown namespace text
        // may contain connection credentials, a query or a private filesystem URI.
        let namespace = if unica_source_designer::roots::PLATFORM_XML_ROOTS
            .iter()
            .any(|root| root.namespace == namespace)
        {
            namespace.as_str()
        } else {
            "<unrecognized namespace>"
        };
        let local_name = if local_name.len() <= 128
            && local_name
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
        {
            local_name.as_str()
        } else {
            "<redacted>"
        };
        report["qname"] = json!(format!("{{{namespace}}}{local_name}"));
    }
    if let Some(requirement) = &error.version_requirement {
        report["versionRequirement"] = json!(requirement);
    }
    report
}

fn safe_relative_path(path: &Path) -> Option<String> {
    // Native separators are judged by `components()`; the public text always uses
    // `/`, so a separator or drive marker left inside one component is refused.
    let mut parts = Vec::new();
    for part in path.components() {
        let std::path::Component::Normal(part) = part else {
            return None;
        };
        let part = part.to_str()?;
        if part.contains(['\\', ':']) || part.chars().any(char::is_control) {
            return None;
        }
        parts.push(part);
    }
    let value = parts.join("/");
    if value.is_empty() || value.len() > 512 {
        return None;
    }
    (redactor(&value) == value).then_some(value)
}

fn cleanup_warning(result: &mut DomainResult, reason: FullDumpCleanupReason) {
    let message = match reason {
        FullDumpCleanupReason::VerifyPrivateStage => "private stage identity could not be verified; cleanup did not remove unverified data",
        FullDumpCleanupReason::CleanupPrivateStage => "private stage cleanup could not finish; retained data requires recovery",
        FullDumpCleanupReason::CleanupCreatedParents => "created target parent directories could not be cleaned",
        FullDumpCleanupReason::WriterCompletionUnconfirmed => "executor completion was not confirmed; private data was retained to protect pending writers",
        FullDumpCleanupReason::RecoveryRetained => "private execution data was retained for recovery",
        FullDumpCleanupReason::RecoveryLocationUnavailable => "a safe workspace-relative recovery location could not be verified",
        FullDumpCleanupReason::PublicationCleanup => "sources were published; publication cleanup requires recovery",
    };
    let warning = json!({"code":"export_cleanup","reason":reason,"message":message});
    if !result.warnings.contains(&warning) {
        result.warnings.push(warning);
    }
}

fn report_recovery(
    root: &Path,
    result: &mut DomainResult,
    recovery: Result<Option<PathBuf>, String>,
    origin: Option<PathBuf>,
    reasons: Vec<FullDumpCleanupReason>,
) {
    match recovery {
        Ok(Some(recovery)) => retain_artifact(root, result, &recovery),
        Ok(None) => {}
        Err(_) => {
            cleanup_warning(result, FullDumpCleanupReason::RecoveryLocationUnavailable);
            if let Some(name) = origin.as_deref().and_then(|path| retained_path(root, path)) {
                result.warnings.push(json!({"code":"export_cleanup","reason":"recoveryLocationUnconfirmed",
                    "originalName":name,"message":format!("retained private object's current location is unconfirmed; original name: {name}")}));
            }
        }
    }
    for reason in reasons {
        cleanup_warning(result, reason);
    }
}

fn retained_path(root: &Path, recovery: &Path) -> Option<String> {
    safe_relative_path(recovery.strip_prefix(root).ok()?)
}

fn retain_artifact(root: &Path, result: &mut DomainResult, recovery: &Path) {
    let Some(path) = retained_path(root, recovery) else {
        cleanup_warning(result, FullDumpCleanupReason::RecoveryLocationUnavailable);
        return;
    };
    let artifact = json!({"kind":"recovery","path":path});
    if !result.artifacts.contains(&artifact) {
        result.artifacts.push(artifact);
    }
}

fn published_result(
    prepared: &PreparedSourceExport,
    root: &Path,
    target: &Path,
    source_set: &str,
    existed: bool,
    files: u64,
) -> DomainResult {
    let target = path_text(
        target
            .strip_prefix(root)
            .expect("guarded target is inside the workspace"),
    );
    let state = if existed { "replaced" } else { "created" };
    let mut result = DomainResult::success(format!(
        "pull exported source set `{source_set}`; the target holds {files} files"
    ));
    result.data = Some(
        json!({"op":OPERATION,"dryRun":false,"sourceSet":source_set,"extension":prepared.arguments.extension,
        "mode":"full","performedMode":"full","modeReason":null,"target":target,"files":files,"state":state,"providerDispatched":true}),
    );
    result.changed.push(json!({"path":target,"kind":state}));
    result
}

fn preview_result(
    prepared: &PreparedSourceExport,
    plan: ExportPlan,
    envelope: &Value,
) -> DomainResult {
    let mut result = DomainResult::success(format!(
        "pull planned a full export of source set `{}`",
        plan.source_set
    ));
    result.data = Some(
        json!({"op":OPERATION,"dryRun":true,"plan":{"sourceSet":plan.source_set,
        "extension":prepared.arguments.extension,"mode":"full","plannedMode":plan.performed_mode,"modeReason":plan.mode_reason,
        "target":path_text(&plan.target),"targetExists":plan.existed,"force":true,"replacesLocalSources":true,"localWorkProtection":false},
        "providerDispatched":false,"requiresPlatform":true}),
    );
    result.next.push(json!({"tool":"unica.run","args":{"op":OPERATION,"args":public_arguments(prepared),"dryRun":false},"reason":"execute with the current arguments"}));
    note_another_copy(&mut result, envelope);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use unica_source_contract::read::full_dump::{DumpValidationFailure, DumpValidationReason};

    #[test]
    fn workspace_relative_paths_are_published_with_forward_slashes_on_every_os() {
        let native: PathBuf = ["main", ".unica-dump-guard-1", "recovery"].iter().collect();
        assert_eq!(
            safe_relative_path(&native).as_deref(),
            Some("main/.unica-dump-guard-1/recovery")
        );
        let root = Path::new("/workspace");
        let mut result = DomainResult::canonical_rejection(
            Some(OPERATION.to_string()),
            RefusalCode::InvalidResult,
            "rejected",
        );
        retain_artifact(root, &mut result, &root.join(&native));
        assert_eq!(
            result.artifacts,
            vec![json!({"kind":"recovery","path":"main/.unica-dump-guard-1/recovery"})]
        );
        assert!(result.warnings.is_empty());
        // Where `\` separates path components it is published as `/`; where it
        // is an ordinary character it stays inside one name and is refused.
        let backslash = safe_relative_path(Path::new("main/a\\b"));
        if std::path::MAIN_SEPARATOR == '\\' {
            assert_eq!(backslash.as_deref(), Some("main/a/b"));
        } else {
            assert_eq!(backslash, None);
        }
        for refused in ["main/c:d", "main/x\u{7}y", ""] {
            assert_eq!(safe_relative_path(Path::new(refused)), None, "{refused:?}");
        }
    }

    #[test]
    fn validation_projection_excludes_untrusted_text_and_keeps_uncertain_publication() {
        let root = Path::new("/private/workspace-marker");
        for path in [
            "/private/path-marker",
            "../outside",
            "C:\\private\\path-marker",
            "Catalogs/password=path-secret.xml",
        ] {
            let mut error = FullDumpFailure::new(
                FullDumpPhase::Validation,
                "password=detail-secret raw-provider-output".into(),
            );
            error.report.validation = Some(
                DumpValidationFailure::new(
                    DumpValidationReason::WrongVersion,
                    Some(Path::new(path)),
                    "parser-secret raw-provider-output".into(),
                )
                .with_root("http://v8.1c.ru/password=namespace-secret", "SafeRoot"),
            );
            error.publication_uncertain = true;
            error.recovery = Err("/private/recovery-marker password=recovery-secret".into());
            let result = failure(root, error);
            let data = result.data.as_ref().unwrap();
            assert_eq!(data["state"], "unknown");
            assert_eq!(data["validation"]["path"], Value::Null);
            assert_eq!(
                data["validation"]["qname"],
                "{<unrecognized namespace>}SafeRoot"
            );
            assert!(data["validation"].get("versionRequirement").is_none());
            assert!(result.artifacts.is_empty());
            let encoded = serde_json::to_string(&result).unwrap();
            for marker in [
                "path-marker",
                "path-secret",
                "detail-secret",
                "raw-provider-output",
                "parser-secret",
                "namespace-secret",
                "recovery-marker",
                "recovery-secret",
                "workspace-marker",
            ] {
                assert!(!encoded.contains(marker), "{path}: leaked {marker}");
            }
        }
    }
}
