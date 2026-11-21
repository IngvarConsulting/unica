//! `fetch` — снимок рабочей конфигурации базы в отдельный каталог рабочего
//! пространства (INV.RUNTIME.FETCH-LEAVES-THE-WORKING-COPY).
//!
//! Раннер выгружает набор целиком внешней полной выгрузкой
//! (`pull <набор> --force --publication-stdio <стадия>`, CTR.WIRE.FULL-PULL-
//! PUBLICATION раннера): Unica согласует исполнителя, на готовое дерево
//! отвечает отказом от публикации, и стадия становится снимком. Каталог
//! набора и память раннера при отказе от публикации не меняются; Unica
//! сверяет это по итоговому кадру. Каталог снимка, не ставший готовым,
//! удаляется.
use super::*;
use serde::Deserialize;
use std::time::SystemTime;
use unica_process::ProcessLineReply;
use unica_process::STREAM_LINE_TOO_LONG_ERROR;

/// Версия кадров внешней полной выгрузки раннера.
const PROTOCOL_VERSION: u32 = 1;

// resource-limit: run.fetch-snapshots-kept
// class: retention
// unit: count
// scope: workspace
// value: 4
// resource: Снимки `unica.run fetch` в `.build/unica/fetch` одной рабочей копии.
// status: unproven
// origin: #3195
// problem: неизвестно
// on-reach: Новый fetch до выгрузки удаляет самые старые снимки так, чтобы вместе с ним их осталось не больше четырёх; снимок, который не удалось удалить, называет предупреждение.
const SNAPSHOTS_KEPT: usize = 4;

/// Каталог снимков относительно корня рабочего пространства.
const SNAPSHOT_ROOT: [&str; 3] = [".build", "unica", "fetch"];

/// Предупреждение: прежний снимок не удалось удалить.
const SNAPSHOT_KEPT_WARNING: &str = "fetch_snapshot_kept";

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

/// Кадр внешней полной выгрузки. Поля, которые снимку не нужны, разбирает
/// serde и не читает Unica: форма кадра закрыта схемой раннера.
#[derive(Debug, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
enum Frame {
    Selected {
        version: u32,
        nonce: String,
    },
    Ready {
        version: u32,
        nonce: String,
        command: String,
        requested_mode: String,
        mode: String,
        extension: Option<String>,
        source_set: String,
        target_path: PathBuf,
        stage_path: PathBuf,
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
            Self::Selected { version, nonce }
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

impl PreparedSourceExport {
    /// `fetch {sourceSet?, extension?}`: набор выгружается всегда целиком, в
    /// новый каталог снимка; согласия человека не нужно — ничего не заменяется.
    pub(super) fn parse_fetch(request: &InvocationRequest) -> Result<Self, DomainResult> {
        let arguments = request.arguments();
        let args = arguments
            .get("args")
            .and_then(Value::as_object)
            .ok_or_else(|| reject_fetch(RefusalCode::BadValue, "run args must be an object"))?;
        let dry_run = arguments
            .get("dryRun")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                reject_fetch(
                    RefusalCode::BadValue,
                    "fetch requires dryRun: true to preview or dryRun: false to execute",
                )
            })?;
        if let Some(unknown) = args
            .keys()
            .find(|key| !["sourceSet", "extension"].contains(&key.as_str()))
        {
            return Err(reject_fetch(
                RefusalCode::BadValue,
                format!(
                    "fetch does not accept `{unknown}`; the closed args are sourceSet and extension"
                ),
            ));
        }
        let source_set = match args.get("sourceSet") {
            None => None,
            Some(Value::String(value)) if valid_source_set_name(value) => Some(value.clone()),
            Some(_) => {
                return Err(reject_fetch(
                    RefusalCode::BadValue,
                    source_set_name_guidance(FETCH),
                ))
            }
        };
        let extension = match args.get("extension") {
            None => None,
            Some(Value::String(value)) if valid_1c_identifier(value) => Some(value.clone()),
            Some(_) => {
                return Err(reject_fetch(
                    RefusalCode::BadValue,
                    "fetch extension must be a non-empty 1C identifier",
                ))
            }
        };
        let context = discover_context(request).map_err(retag)?;
        Ok(Self {
            arguments: ExportArguments {
                mode: ExportMode::Full,
                source_set,
                extension,
            },
            dry_run,
            context,
            reexport: None,
            fetch: true,
            drop: None,
            call: request.arguments().clone(),
        })
    }
}

impl PreparedSourceExport {
    /// `fetch.drop {snapshotId}`: один снимок по идентификатору из ответа
    /// `fetch`. Раннер не нужен.
    pub(super) fn parse_fetch_drop(request: &InvocationRequest) -> Result<Self, DomainResult> {
        let reject = |message: &str| reject_at(FETCH_DROP, RefusalCode::BadValue, message);
        let arguments = request.arguments();
        let args = arguments
            .get("args")
            .and_then(Value::as_object)
            .ok_or_else(|| reject("run args must be an object"))?;
        let dry_run = arguments
            .get("dryRun")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                reject("fetch.drop requires dryRun: true to preview or dryRun: false to execute")
            })?;
        if args.keys().any(|key| key != "snapshotId") {
            return Err(reject(
                "fetch.drop accepts only snapshotId, the identifier from the answer of fetch",
            ));
        }
        let id = args
            .get("snapshotId")
            .and_then(Value::as_str)
            .filter(|id| valid_snapshot_id(id))
            .ok_or_else(|| {
                reject("fetch.drop snapshotId must be the 32 lowercase hexadecimal digits that fetch returned")
            })?
            .to_string();
        let context = discover_context(request).map_err(|mut refusal| {
            refusal.at = Some(FETCH_DROP.to_string());
            refusal
        })?;
        Ok(Self {
            arguments: ExportArguments {
                mode: ExportMode::Full,
                source_set: None,
                extension: None,
            },
            dry_run,
            context,
            reexport: None,
            fetch: true,
            drop: Some(id),
            call: request.arguments().clone(),
        })
    }
}

/// Идентификатор снимка: 32 строчные шестнадцатеричные цифры. Другое имя не
/// может указать за пределы каталога снимков.
fn valid_snapshot_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// Удаляет каталог одного снимка. Отсутствующий снимок уже удалён: ответ
/// называет это `removed: false`.
pub(super) fn drop_snapshot(prepared: &PreparedSourceExport, id: &str) -> DomainResult {
    let Ok(root) = normalize_path_identity(&prepared.context.workspace_root) else {
        return reject_at(
            FETCH_DROP,
            RefusalCode::InvalidState,
            "fetch.drop could not resolve the workspace root",
        );
    };
    let path = root
        .join(SNAPSHOT_ROOT.iter().collect::<PathBuf>())
        .join(id);
    let snapshot = relative(&path, &root);
    let exists = path.is_dir();
    if prepared.dry_run {
        let mut result = DomainResult::success(if exists {
            format!("fetch.drop plans to delete the snapshot {snapshot}")
        } else {
            format!("fetch.drop finds no snapshot {snapshot}; there is nothing to delete")
        });
        result.data = Some(json!({
            "op": FETCH_DROP,
            "dryRun": true,
            "plan": {"snapshotId": id, "snapshot": snapshot, "exists": exists},
        }));
        result.next.push(json!({
            "tool": "unica.run",
            "args": {"op": FETCH_DROP, "args": {"snapshotId": id}, "dryRun": false},
            "reason": "execute with the current arguments",
        }));
        return result;
    }
    if !exists {
        let mut result = DomainResult::success(format!(
            "fetch.drop found no snapshot {snapshot}; it is already deleted"
        ));
        result.data = Some(json!({
            "op": FETCH_DROP, "dryRun": false, "snapshotId": id, "snapshot": snapshot, "removed": false,
        }));
        return result;
    }
    match remove_snapshot(&path) {
        Ok(()) => {
            let mut result =
                DomainResult::success(format!("fetch.drop deleted the snapshot {snapshot}"));
            result.data = Some(json!({
                "op": FETCH_DROP, "dryRun": false, "snapshotId": id, "snapshot": snapshot, "removed": true,
            }));
            result
                .changed
                .push(json!({"path": snapshot, "kind": "deleted"}));
            result
        }
        Err(error) => reject_at(
            FETCH_DROP,
            RefusalCode::InvalidState,
            format!(
                "fetch.drop could not delete the snapshot {snapshot} ({error}); repeat when no program holds its files"
            ),
        ),
    }
}

pub(super) fn execute(
    prepared: &PreparedSourceExport,
    runner: &dyn ProcessRunner,
    cancellation: CancellationToken,
) -> DomainResult {
    let tool = match resolve_bundled_runner(&prepared.context.cwd) {
        Ok(resolved) => resolved.tool,
        Err(absence) => return missing_runner_rejection(Some(FETCH.to_string()), absence),
    };
    execute_with_tool(prepared, runner, cancellation, &tool)
}

pub(super) fn execute_with_tool(
    prepared: &PreparedSourceExport,
    runner: &dyn ProcessRunner,
    cancellation: CancellationToken,
    tool: &BundledTool,
) -> DomainResult {
    if cancellation.is_cancelled() {
        return reject_fetch(RefusalCode::Cancelled, "fetch cancelled before preparation");
    }
    let inputs = match capture_inputs(prepared) {
        Ok(value) => value,
        Err(result) => return retag(result),
    };
    let Ok(root) = normalize_path_identity(&prepared.context.workspace_root) else {
        return reject_fetch(
            RefusalCode::InvalidState,
            "fetch could not resolve the workspace root",
        );
    };
    let Some((source_set, directory)) = selected_source_set(&root, &prepared.arguments) else {
        return reject_fetch(
            RefusalCode::InvalidState,
            format!("{CONFIG_NAME} declares no directory of the selected source set"),
        );
    };
    if prepared.dry_run {
        return preview(prepared, &inputs, &root, runner, &cancellation, tool);
    }
    let snapshots = root.join(SNAPSHOT_ROOT.iter().collect::<PathBuf>());
    let mut warnings = prune_snapshots(&root, &snapshots, SNAPSHOTS_KEPT - 1);
    let id = uuid::Uuid::new_v4().simple().to_string();
    let stage = snapshots.join(&id);
    if let Err(error) = std::fs::create_dir_all(&stage) {
        return reject_fetch(
            RefusalCode::InvalidState,
            format!("fetch could not create its snapshot directory: {error}"),
        );
    }
    // Отпечаток снимается до выгрузки: правка набора во время неё — уже
    // расхождение с тем, что видел снимок.
    let outcome = source_fingerprint(&root, &directory, &cancellation).and_then(|fingerprint| {
        exchange(
            prepared,
            &root,
            &source_set,
            &directory,
            &stage,
            runner,
            &cancellation,
            tool,
        )
        .map(|envelope| (fingerprint, envelope))
    });
    let (fingerprint, envelope) = match outcome {
        Ok(snapshot) => snapshot,
        Err(mut result) => {
            // Отказ, отмена или неподтверждённый итог: снимка нет.
            if let Err(error) = remove_snapshot(&stage) {
                result.warnings.push(json!({
                    "code": SNAPSHOT_KEPT_WARNING,
                    "message": format!(
                        "fetch could not remove its unfinished snapshot {} ({error}); delete the directory when no 1C process holds it",
                        relative(&stage, &root)
                    ),
                    "path": relative(&stage, &root),
                }));
            }
            return result;
        }
    };
    let snapshot = relative(&stage, &root);
    let mut result = DomainResult::success(format!(
        "fetch exported source set `{source_set}` of the infobase into {snapshot}; the sources, the runner's memory and the infobase are unchanged"
    ));
    result.data = Some(json!({
        "op": FETCH,
        "dryRun": false,
        "snapshotId": id,
        "snapshot": snapshot,
        "sourceSet": source_set,
        "extension": prepared.arguments.extension,
        "target": relative(&directory, &root),
        "performedMode": ExportMode::Full.public_name(),
        "sourceFingerprint": fingerprint,
        "generation": Value::Null,
        "generationReason": "the pinned v8-runner does not report the infobase generation of a snapshot",
    }));
    result
        .changed
        .push(json!({"path": snapshot, "kind": "created"}));
    note_runner_warnings(&mut result, &envelope);
    result.warnings.append(&mut warnings);
    result
}

/// Превью: план полной выгрузки у раннера (`pull --force --dry-run`), без
/// каталога снимка и без записи.
fn preview(
    prepared: &PreparedSourceExport,
    inputs: &StableInputs,
    root: &Path,
    runner: &dyn ProcessRunner,
    cancellation: &CancellationToken,
    tool: &BundledTool,
) -> DomainResult {
    let envelope = match invoke_runner(prepared, tool, runner, cancellation, true) {
        Ok(envelope) => envelope,
        Err(result) => return retag(result),
    };
    let plan = match validate_envelope(prepared, inputs, root, &envelope, false) {
        Ok(plan) => plan,
        Err(result) => return retag(result),
    };
    let snapshots = path_text(&SNAPSHOT_ROOT.iter().collect::<PathBuf>());
    let mut result = DomainResult::success(format!(
        "fetch plans a full export of source set `{}` into a new snapshot under {snapshots}",
        plan.source_set
    ));
    result.data = Some(json!({
        "op": FETCH,
        "dryRun": true,
        "plan": {
            "sourceSet": plan.source_set,
            "extension": prepared.arguments.extension,
            "target": path_text(&plan.target),
            "snapshots": snapshots,
            "replacesLocalSources": false,
        },
        "providerDispatched": false,
        "requiresPlatform": true,
    }));
    result.next.push(json!({
        "tool": "unica.run",
        "args": {"op": FETCH, "args": fetch_arguments(prepared), "dryRun": false},
        "reason": "execute with the current arguments",
    }));
    note_runner_warnings(&mut result, &envelope);
    result
}

/// Обмен кадрами с раннером: исполнитель согласуется, готовое дерево
/// отклоняется, итог должен подтвердить, что ничего не опубликовано и память
/// не тронута. Успех — конверт итогового кадра.
#[allow(clippy::too_many_arguments)]
fn exchange(
    prepared: &PreparedSourceExport,
    root: &Path,
    source_set: &str,
    directory: &Path,
    stage: &Path,
    runner: &dyn ProcessRunner,
    cancellation: &CancellationToken,
    tool: &BundledTool,
) -> Result<Value, DomainResult> {
    let mut args = vec![
        "--config".to_string(),
        root.join(CONFIG_NAME).display().to_string(),
        RUNNER_COMMAND.to_string(),
        source_set.to_string(),
        "--force".to_string(),
        "--publication-stdio".to_string(),
        stage.display().to_string(),
    ];
    if let Some(extension) = &prepared.arguments.extension {
        args.extend(["--extension".to_string(), extension.clone()]);
    }
    let command = ProcessCommand {
        program: tool.program.clone(),
        args,
        cwd: root.to_path_buf(),
        env: Vec::new(),
        env_remove: Vec::new(),
        capture_limits: Some((RUNNER_OUTPUT_LIMIT, RUNNER_OUTPUT_LIMIT)),
        timeout: None,
        cancellation: cancellation.clone(),
    };
    let mut phase = Phase::Selected;
    let mut nonce: Option<String> = None;
    let mut problem: Option<DomainResult> = None;
    let mut terminal: Option<(Publication, Memory, Value)> = None;
    let mut ready = false;
    let output = runner.run_controlled_streaming_staged(
        &command,
        RUNNER_OUTPUT_LIMIT,
        &mut |_, bytes| {
            let Ok(frame) = serde_json::from_slice::<Frame>(bytes) else {
                problem = Some(invalid("v8-runner returned an invalid publication frame"));
                return ProcessLineReply::Stop;
            };
            let (version, frame_nonce) = frame.identity();
            if version != PROTOCOL_VERSION
                || uuid::Uuid::parse_str(frame_nonce).is_err()
                || nonce.as_deref().is_some_and(|expected| expected != frame_nonce)
            {
                problem = Some(invalid(
                    "v8-runner publication version or nonce does not match",
                ));
                return ProcessLineReply::Stop;
            }
            let nonce = nonce.get_or_insert_with(|| frame_nonce.to_owned()).clone();
            match frame {
                Frame::Selected { .. } if phase == Phase::Selected => {
                    if cancellation.is_cancelled() {
                        problem = Some(reject_fetch(
                            RefusalCode::Cancelled,
                            "fetch cancelled before the export started",
                        ));
                        phase = Phase::Terminal;
                        return reply(&nonce, "selected", "reject");
                    }
                    phase = Phase::Ready;
                    reply(&nonce, "selected", "continue")
                }
                Frame::Ready {
                    command,
                    requested_mode,
                    mode,
                    extension,
                    source_set: reported_set,
                    target_path,
                    stage_path,
                    ..
                } if phase == Phase::Ready => {
                    phase = Phase::Terminal;
                    ready = true;
                    if command != RUNNER_COMMAND
                        || requested_mode != "FULL"
                        || mode != "FULL"
                        || extension != prepared.arguments.extension
                        || reported_set != source_set
                        || !same_path(&target_path, directory)
                        || !same_path(&stage_path, stage)
                    {
                        problem = Some(invalid(
                            "v8-runner prepared a different operation, mode, extension, source set, target or stage",
                        ));
                    }
                    // Публикации нет ни в каком случае: снимок — сама стадия.
                    reply(&nonce, "ready", "rejected")
                }
                Frame::Terminal {
                    publication,
                    memory,
                    result,
                    ..
                } if phase != Phase::Done => {
                    terminal = Some((publication, memory, result));
                    phase = Phase::Done;
                    ProcessLineReply::Continue
                }
                _ => {
                    problem.get_or_insert_with(|| {
                        invalid("v8-runner publication frame is out of order")
                    });
                    ProcessLineReply::Stop
                }
            }
        },
    );
    if let Some(problem) = problem {
        return Err(problem);
    }
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            return Err(retag(runner_start_rejection(
                Some(FETCH.into()),
                &error,
                &tool.program,
            )))
        }
    };
    if output.cancelled {
        return Err(reject_fetch(RefusalCode::Cancelled, "fetch was cancelled"));
    }
    if output.timed_out {
        return Err(reject_fetch(
            RefusalCode::DeadlineExceeded,
            "fetch timed out",
        ));
    }
    if output
        .line_error
        .as_ref()
        .is_some_and(|(_, reason)| reason == STREAM_LINE_TOO_LONG_ERROR)
    {
        return Err(invalid(&format!(
            "v8-runner sent a publication frame longer than the {RUNNER_OUTPUT_LIMIT}-byte line limit"
        )));
    }
    if output.line_error.is_some() || output.stopped_by_consumer {
        return Err(invalid(
            "v8-runner returned an unreadable publication stream",
        ));
    }
    let Some((publication, memory, envelope)) = terminal else {
        return Err(invalid("v8-runner ended without a publication outcome"));
    };
    if publication != Publication::NotPublished || memory != Memory::Unchanged {
        return Err(invalid(
            "v8-runner did not confirm that the sources and its memory stayed unchanged",
        ));
    }
    if envelope["command"] != RUNNER_COMMAND {
        return Err(invalid(
            "v8-runner returned a result for a different operation",
        ));
    }
    if !ready {
        // До `ready` раннер отказал сам: его отказ — ответ.
        let code = envelope["error"]["code"]
            .as_str()
            .unwrap_or("provider_failed");
        let message = envelope["error"]["message"]
            .as_str()
            .map(redactor)
            .unwrap_or_else(|| "v8-runner failed without a typed message".to_string());
        return Err(runner_envelope_refusal(
            runner_rejection(
                Some(FETCH.to_string()),
                code,
                envelope["error"]["layer"].as_str(),
                message,
            ),
            &envelope,
        ));
    }
    remove_legacy_dump_info(stage)?;
    if !stage_holds_an_export(stage) {
        return Err(invalid("v8-runner prepared an empty fetch snapshot"));
    }
    Ok(envelope)
}

/// Раннер может перенести старую опись из исходников в стадию.
fn remove_legacy_dump_info(stage: &Path) -> Result<(), DomainResult> {
    match std::fs::remove_file(stage.join("ConfigDumpInfo.xml")) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(invalid(&format!(
            "fetch could not remove ConfigDumpInfo.xml from its snapshot: {error}"
        ))),
    }
}

/// Пустая стадия готовым снимком не становится.
fn stage_holds_an_export(stage: &Path) -> bool {
    std::fs::read_dir(stage)
        .is_ok_and(|mut entries| entries.next().is_some_and(|entry| entry.is_ok()))
}

/// Отпечаток набора: sha256 строк «путь от каталога набора через `/`, NUL,
/// sha256 содержимого в шестнадцатеричной записи, перевод строки», по
/// возрастанию байтов пути. Учитываются обычные файлы; символические ссылки,
/// каталоги `.git` и каталог `.build` рабочего пространства — нет.
/// Отсутствующий каталог набора — пустой набор.
pub(super) fn source_fingerprint(
    root: &Path,
    directory: &Path,
    cancellation: &CancellationToken,
) -> Result<String, DomainResult> {
    let cache = root.join(".build");
    let mut files = Vec::new();
    let mut pending = vec![directory.to_path_buf()];
    while let Some(current) = pending.pop() {
        if cancellation.is_cancelled() {
            return Err(reject_fetch(
                RefusalCode::Cancelled,
                "fetch cancelled while reading the source set",
            ));
        }
        let entries = match std::fs::read_dir(&current) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && current == directory => {
                break
            }
            Err(error) => return Err(unreadable(&current, root, &error)),
        };
        for entry in entries {
            let entry = entry.map_err(|error| unreadable(&current, root, &error))?;
            let path = entry.path();
            let kind = entry
                .file_type()
                .map_err(|error| unreadable(&path, root, &error))?;
            if kind.is_dir() {
                if entry.file_name() != ".git" && path != cache {
                    pending.push(path);
                }
            } else if kind.is_file() {
                files.push(path);
            }
        }
    }
    let mut lines = files
        .into_iter()
        .map(|path| {
            let relative = path
                .strip_prefix(directory)
                .map(path_text)
                .unwrap_or_else(|_| path_text(&path));
            (relative, path)
        })
        .collect::<Vec<_>>();
    lines.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    let mut digest = Sha256::new();
    for (relative, path) in lines {
        let hash = file_hash(&path).map_err(|error| unreadable(&path, root, &error))?;
        digest.update(relative.as_bytes());
        digest.update([0]);
        digest.update(hash.as_bytes());
        digest.update(b"\n");
    }
    Ok(format!("sha256:{:x}", digest.finalize()))
}

/// Хеш одного файла в отпечатке набора: шестнадцатеричный sha256 его байтов.
fn file_hash(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut content = Sha256::new();
    std::io::copy(&mut file, &mut content)?;
    Ok(format!("{:x}", content.finalize()))
}

/// Удаляет самые старые снимки, оставляя `keep` самых новых.
fn prune_snapshots(root: &Path, snapshots: &Path, keep: usize) -> Vec<Value> {
    let Ok(entries) = std::fs::read_dir(snapshots) else {
        return Vec::new();
    };
    let mut existing = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| {
            let modified = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (modified, entry.path())
        })
        .collect::<Vec<_>>();
    existing.sort_by(|left, right| right.0.cmp(&left.0));
    existing
        .into_iter()
        .skip(keep)
        .filter_map(|(_, path)| {
            remove_snapshot(&path).err().map(|error| {
                json!({
                    "code": SNAPSHOT_KEPT_WARNING,
                    "message": format!(
                        "fetch could not remove the older snapshot {} ({error}); delete the directory when no program holds it",
                        relative(&path, root)
                    ),
                    "path": relative(&path, root),
                })
            })
        })
        .collect()
}

/// Платформа может ещё держать файлы стадии после ответа раннера: удаление
/// повторяется несколько секунд, отсутствующий каталог уже удалён.
fn remove_snapshot(path: &Path) -> std::io::Result<()> {
    remove_reexport_directory(path)
}

fn fetch_arguments(prepared: &PreparedSourceExport) -> Value {
    let mut args = Map::new();
    if let Some(source_set) = &prepared.arguments.source_set {
        args.insert("sourceSet".to_string(), Value::String(source_set.clone()));
    }
    if let Some(extension) = &prepared.arguments.extension {
        args.insert("extension".to_string(), Value::String(extension.clone()));
    }
    Value::Object(args)
}

fn reply(nonce: &str, phase: &str, decision: &str) -> ProcessLineReply {
    let mut bytes = serde_json::to_vec(
        &json!({"version": PROTOCOL_VERSION, "nonce": nonce, "phase": phase, "decision": decision}),
    )
    .expect("a closed publication reply is serializable");
    bytes.push(b'\n');
    ProcessLineReply::Reply(bytes)
}

/// Раннер называет каталог своим написанием: на Windows это расширенная форма
/// `\\?\E:\…`. Сравнивается то, что путь значит, а не его запись.
fn same_path(reported: &Path, expected: &Path) -> bool {
    matches!(
        (normalize_path_identity(reported), normalize_path_identity(expected)),
        (Ok(reported), Ok(expected)) if reported == expected
    )
}

fn relative(path: &Path, root: &Path) -> String {
    path_text(
        unica_workspace::paths::host_path_relative_to(path, root)
            .as_deref()
            .unwrap_or(path),
    )
}

fn unreadable(path: &Path, root: &Path, error: &std::io::Error) -> DomainResult {
    reject_fetch(
        RefusalCode::InvalidState,
        format!(
            "fetch could not read {} of the source set: {error}",
            relative(path, root)
        ),
    )
}

fn invalid(message: &str) -> DomainResult {
    reject_fetch(RefusalCode::InvalidResult, message)
}

fn reject_fetch(code: RefusalCode, message: impl Into<String>) -> DomainResult {
    reject_at(FETCH, code, message)
}

/// Отказ общего помощника `pull` относится к `fetch`.
fn retag(mut result: DomainResult) -> DomainResult {
    if result.at.as_deref() == Some(OPERATION) {
        result.at = Some(FETCH.to_string());
    }
    result
}

#[cfg(test)]
#[path = "fetch_tests.rs"]
mod tests;
