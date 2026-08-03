use crate::application::AdapterOutcome;
use crate::domain::cancellation::{CancellationToken, CANCELLED_PREFIX};
use crate::domain::code_intelligence::{
    CodeDefinition, CodeDefinitionResult, CodeIntelligenceContext, CodeIntelligenceReadData,
    CodeIntelligenceReadRequest, ProviderDeadline,
};
use crate::domain::metadata::{
    MetaCompleteness, MetaDiagnostic, MetaDiagnosticCode, MetaFreshness, MetaRelatedSection,
    MetaRelatedSections, MetaRelatedStatus,
};
use crate::domain::metadata::{MetaProfileResult, MetaProfileSection};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::platform::secure_read::{
    capture_root_relative_regular_files, SecureTreeCaptureLimits,
};
use crate::infrastructure::workspace_index::IndexReadiness;
use crate::infrastructure::workspace_services::{
    WorkspaceRlmOperation, WorkspaceServiceManager, WorkspaceServiceRlmOutput,
};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

const RLM_NAVIGATION_TIMEOUT: Duration = Duration::from_secs(45);
const RELATED_READINESS_SETTLE_LIMIT: Duration = Duration::from_millis(500);
const RELATED_READINESS_POLL_INTERVAL: Duration = Duration::from_millis(10);
const RELATED_SOURCE_MAX_ENTRIES: usize = 20_000;
const RELATED_SOURCE_MAX_FILES: usize = 20_000;
const RELATED_SOURCE_MAX_BYTES: usize = 64 * 1024 * 1024;
const RELATED_SOURCE_MAX_DEPTH: usize = 256;

/// Opaque identity of the exact relevant source paths and bytes observed by a
/// typed related-info read. It is deliberately not serializable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WorkspaceSourceSnapshot([u8; 32]);

#[derive(Debug, Clone, PartialEq, Eq)]
enum RelatedSnapshotHashPhase {
    BeforeEntry {
        logical_path: String,
        ordinal: usize,
        total: usize,
    },
    AfterEntry {
        logical_path: String,
        ordinal: usize,
        total: usize,
    },
    AfterFinalize,
}

#[cfg(test)]
type RelatedSnapshotHashHook = Box<dyn FnMut(&RelatedSnapshotHashPhase)>;

#[cfg(test)]
thread_local! {
    static RELATED_SNAPSHOT_HASH_HOOK:
        std::cell::RefCell<Option<RelatedSnapshotHashHook>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn with_related_snapshot_hash_hook<T>(
    hook: impl FnMut(&RelatedSnapshotHashPhase) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<RelatedSnapshotHashHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            RELATED_SNAPSHOT_HASH_HOOK.with(|slot| slot.replace(self.0.take()));
        }
    }

    let previous = RELATED_SNAPSHOT_HASH_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

fn emit_related_snapshot_hash_phase(phase: RelatedSnapshotHashPhase) {
    #[cfg(test)]
    RELATED_SNAPSHOT_HASH_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().as_mut() {
            hook(&phase);
        }
    });
    #[cfg(not(test))]
    let _ = phase;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelatedReadinessState {
    Ready,
    Stale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RelatedReadinessProof {
    state: RelatedReadinessState,
    source: Option<WorkspaceSourceSnapshot>,
}

trait RlmNavigationClient: Send + Sync {
    fn readiness(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<IndexReadiness, String>;

    fn call(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        operation: WorkspaceRlmOperation,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceServiceRlmOutput, String>;
}

struct WorkspaceRlmNavigationClient;

impl RlmNavigationClient for WorkspaceRlmNavigationClient {
    fn readiness(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<IndexReadiness, String> {
        WorkspaceServiceManager::new().rlm_readiness_cancellable_with_timeout(
            context,
            source_root,
            &Map::new(),
            timeout,
            cancellation,
        )
    }

    fn call(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        operation: WorkspaceRlmOperation,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceServiceRlmOutput, String> {
        WorkspaceServiceManager::new().call_rlm_cancellable(
            context,
            source_root,
            operation,
            timeout,
            cancellation,
        )
    }
}

static WORKSPACE_RLM_NAVIGATION_CLIENT: WorkspaceRlmNavigationClient = WorkspaceRlmNavigationClient;

pub(crate) struct RlmNavigationAdapter<'a> {
    client: &'a (dyn RlmNavigationClient + Send + Sync),
}

impl RlmNavigationAdapter<'static> {
    pub(crate) fn new() -> Self {
        Self {
            client: &WORKSPACE_RLM_NAVIGATION_CLIENT,
        }
    }
}

impl<'a> RlmNavigationAdapter<'a> {
    #[cfg(test)]
    fn with_client(client: &'a (dyn RlmNavigationClient + Send + Sync)) -> Self {
        Self { client }
    }

    pub(crate) fn metadata_related(
        &self,
        metadata_path: &str,
        sections: &[String],
        limit: usize,
        context: &CodeIntelligenceContext,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> MetaRelatedSections {
        let unavailable = |freshness| {
            unavailable_related_sections(
                sections,
                freshness,
                "related metadata index is unavailable",
            )
        };
        if cancellation.is_cancelled() || deadline.remaining().is_zero() {
            return unavailable(MetaFreshness::Unknown);
        }
        let before = match related_readiness_proof(self.client, context, deadline, cancellation) {
            Some(proof) => proof,
            None => return unavailable(MetaFreshness::Unknown),
        };
        let timeout = deadline.remaining().min(RLM_NAVIGATION_TIMEOUT);
        if timeout.is_zero() {
            return unavailable(MetaFreshness::Unknown);
        }
        let output = match self.client.call(
            &context.workspace,
            &context.source_root.path,
            WorkspaceRlmOperation::ObjectProfile {
                name: metadata_path.to_string(),
                sections: Some(sections.to_vec()),
                limit,
            },
            timeout,
            cancellation,
        ) {
            Ok(output) => output,
            Err(_) => return unavailable(MetaFreshness::Unknown),
        };
        let Ok(value) = serde_json::from_str::<Value>(output.result_text.trim()) else {
            return unavailable(MetaFreshness::Unknown);
        };
        if value.get("error").and_then(Value::as_str).is_some() {
            return unavailable(MetaFreshness::Unknown);
        }
        let Ok(profile) = profile_result(&value) else {
            return unavailable(MetaFreshness::Unknown);
        };
        let freshness = match before.state {
            RelatedReadinessState::Stale => MetaFreshness::Stale,
            RelatedReadinessState::Ready => {
                match related_readiness_proof(self.client, context, deadline, cancellation) {
                    Some(after)
                        if after.state == RelatedReadinessState::Ready
                            && before.source == after.source =>
                    {
                        MetaFreshness::Current
                    }
                    Some(_) => MetaFreshness::Stale,
                    None => MetaFreshness::Unknown,
                }
            }
        };
        related_sections_from_profile(profile, sections, limit, freshness)
    }

    pub(crate) fn invoke_resolved_cancellable(
        &self,
        request: &CodeIntelligenceReadRequest,
        context: &CodeIntelligenceContext,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<RlmNavigationOutcome, String> {
        let operation_name = request.operation_name();
        let operation = operation_for_request(request)?;
        if cancellation.is_cancelled() {
            return Ok(RlmNavigationOutcome::plain(AdapterOutcome::cancelled(
                format!("{operation_name} cancelled before provider work"),
            )));
        }
        let readiness_timeout = deadline.remaining().min(RLM_NAVIGATION_TIMEOUT);
        if readiness_timeout.is_zero() {
            return Err(format!(
                "{operation_name} provider deadline exceeded before readiness check"
            ));
        }
        let readiness = match self.client.readiness(
            &context.workspace,
            &context.source_root.path,
            readiness_timeout,
            cancellation,
        ) {
            Ok(readiness) => readiness,
            Err(error) if error.starts_with(CANCELLED_PREFIX) => {
                return Ok(RlmNavigationOutcome::plain(cancelled_client_outcome(
                    operation_name,
                    &error,
                )));
            }
            Err(error) => return Err(error),
        };
        let db_path = match readiness {
            IndexReadiness::Ready { db_path } => db_path,
            other => {
                return Ok(RlmNavigationOutcome::plain(index_unavailable_outcome(
                    request, other,
                )))
            }
        };
        let timeout = deadline.remaining().min(RLM_NAVIGATION_TIMEOUT);
        if timeout.is_zero() {
            return Err(format!("{operation_name} provider deadline exceeded"));
        }
        let output = match self.client.call(
            &context.workspace,
            &context.source_root.path,
            operation,
            timeout,
            cancellation,
        ) {
            Ok(output) => output,
            Err(error) if error.starts_with(CANCELLED_PREFIX) => {
                return Ok(RlmNavigationOutcome::plain(cancelled_client_outcome(
                    operation_name,
                    &error,
                )));
            }
            Err(error) => return Err(error),
        };
        let value: Value = serde_json::from_str(output.result_text.trim()).map_err(|error| {
            format!("{operation_name} received invalid index helper JSON: {error}")
        })?;
        if let Some(error) = value
            .get("error")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            return Err(format!("{operation_name} index helper failed: {error}"));
        }
        let mut outcome = AdapterOutcome::ok(format!(
            "{operation_name} completed through the persistent RLM MCP API"
        ));
        let data;
        match request {
            // ADR-0023: the index already answers with structure, so the tool
            // publishes it instead of rendering it into a line grammar.
            CodeIntelligenceReadRequest::Definition { .. } => {
                let (result, warnings) = definition_result(&value)?;
                // The transport phrase stays: the issue-89 service test proves
                // reuse of the persistent RLM process through this summary.
                outcome.summary = format!(
                    "{operation_name} found {} definition(s) for {} through the persistent RLM MCP API",
                    result.definitions.len(),
                    result.name
                );
                outcome.warnings.extend(warnings);
                data = Some(CodeIntelligenceReadData::Definition(result));
            }
            CodeIntelligenceReadRequest::ObjectProfile { .. } => {
                let result = profile_result(&value)?;
                outcome.summary = format!(
                    "{operation_name} described {} across {} section(s) through the persistent RLM MCP API",
                    result.object_name,
                    result.sections.len()
                );
                data = Some(CodeIntelligenceReadData::ObjectProfile(result));
            }
            CodeIntelligenceReadRequest::Outline { .. } => {
                return Err("code outline is not an index navigation capability".to_string())
            }
        }
        outcome.artifacts = vec![
            context.source_root.path.display().to_string(),
            db_path.display().to_string(),
        ];
        if !output.stderr.trim().is_empty() {
            outcome
                .warnings
                .push(format!("RLM stderr: {}", output.stderr.trim()));
            outcome.stderr = Some(output.stderr);
        }
        Ok(RlmNavigationOutcome { outcome, data })
    }
}

fn related_readiness_proof(
    client: &dyn RlmNavigationClient,
    context: &CodeIntelligenceContext,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Option<RelatedReadinessProof> {
    let settle_until = Instant::now() + deadline.remaining().min(RELATED_READINESS_SETTLE_LIMIT);
    let mut ready_snapshot = None;
    loop {
        if cancellation.is_cancelled() || deadline.remaining().is_zero() {
            return None;
        }
        let settle_remaining = settle_until.saturating_duration_since(Instant::now());
        if settle_remaining.is_zero() {
            return Some(RelatedReadinessProof {
                state: RelatedReadinessState::Stale,
                source: None,
            });
        }
        let readiness = client
            .readiness(
                &context.workspace,
                &context.source_root.path,
                deadline
                    .remaining()
                    .min(RLM_NAVIGATION_TIMEOUT)
                    .min(settle_remaining),
                cancellation,
            )
            .ok()?;
        match readiness {
            IndexReadiness::Ready { .. } => {
                let source = related_source_content_snapshot(
                    &context.source_root.path,
                    deadline,
                    settle_until,
                    cancellation,
                )
                .ok()?;
                if ready_snapshot == Some(source) {
                    return Some(RelatedReadinessProof {
                        state: RelatedReadinessState::Ready,
                        source: Some(source),
                    });
                }
                ready_snapshot = Some(source);
            }
            IndexReadiness::Stale { .. } => {
                return Some(RelatedReadinessProof {
                    state: RelatedReadinessState::Stale,
                    source: None,
                });
            }
            IndexReadiness::Building => {
                ready_snapshot = None;
                if Instant::now() >= settle_until {
                    return Some(RelatedReadinessProof {
                        state: RelatedReadinessState::Stale,
                        source: None,
                    });
                }
            }
            IndexReadiness::Missing
            | IndexReadiness::Failed(_)
            | IndexReadiness::Unavailable(_) => return None,
        }
        let remaining = deadline.remaining();
        if remaining.is_zero() {
            return None;
        }
        let settle_remaining = settle_until.saturating_duration_since(Instant::now());
        if settle_remaining.is_zero() {
            continue;
        }
        std::thread::sleep(
            remaining
                .min(RELATED_READINESS_POLL_INTERVAL)
                .min(settle_remaining),
        );
    }
}

fn related_source_content_snapshot(
    source_root: &Path,
    deadline: ProviderDeadline,
    settle_until: Instant,
    cancellation: &CancellationToken,
) -> Result<WorkspaceSourceSnapshot, ()> {
    let checkpoint = || {
        ensure_snapshot_budget(deadline, settle_until, cancellation)
            .map_err(|()| io::Error::new(io::ErrorKind::Interrupted, "snapshot interrupted"))
    };
    let files = capture_root_relative_regular_files(
        source_root,
        Path::new(""),
        SecureTreeCaptureLimits {
            maximum_depth: RELATED_SOURCE_MAX_DEPTH,
            maximum_entries: RELATED_SOURCE_MAX_ENTRIES,
            maximum_files: RELATED_SOURCE_MAX_FILES,
            maximum_bytes: RELATED_SOURCE_MAX_BYTES,
        },
        |path| path.file_name().and_then(|name| name.to_str()) != Some(".build"),
        is_related_source_file,
        checkpoint,
    )
    .map_err(|_| ())?;
    if files.start_missing {
        return Err(());
    }

    let mut hasher = Sha256::new();
    update_snapshot_hash(
        &mut hasher,
        b"unica-related-source-content-v1",
        deadline,
        settle_until,
        cancellation,
    )?;
    let total = files.files.len();
    for (ordinal, entry) in files.files.into_iter().enumerate() {
        let logical_path = entry.logical_path;
        let bytes = entry.bytes;
        emit_related_snapshot_hash_phase(RelatedSnapshotHashPhase::BeforeEntry {
            logical_path: logical_path.clone(),
            ordinal,
            total,
        });
        update_snapshot_hash(
            &mut hasher,
            (logical_path.len() as u64).to_le_bytes(),
            deadline,
            settle_until,
            cancellation,
        )?;
        update_snapshot_hash(
            &mut hasher,
            logical_path.as_bytes(),
            deadline,
            settle_until,
            cancellation,
        )?;
        update_snapshot_hash(
            &mut hasher,
            (bytes.len() as u64).to_le_bytes(),
            deadline,
            settle_until,
            cancellation,
        )?;
        update_snapshot_hash(&mut hasher, &bytes, deadline, settle_until, cancellation)?;
        emit_related_snapshot_hash_phase(RelatedSnapshotHashPhase::AfterEntry {
            logical_path,
            ordinal,
            total,
        });
        ensure_snapshot_budget(deadline, settle_until, cancellation)?;
    }
    ensure_snapshot_budget(deadline, settle_until, cancellation)?;
    let digest = hasher.finalize().into();
    emit_related_snapshot_hash_phase(RelatedSnapshotHashPhase::AfterFinalize);
    ensure_snapshot_budget(deadline, settle_until, cancellation)?;
    Ok(WorkspaceSourceSnapshot(digest))
}

fn update_snapshot_hash(
    hasher: &mut Sha256,
    bytes: impl AsRef<[u8]>,
    deadline: ProviderDeadline,
    settle_until: Instant,
    cancellation: &CancellationToken,
) -> Result<(), ()> {
    ensure_snapshot_budget(deadline, settle_until, cancellation)?;
    hasher.update(bytes.as_ref());
    ensure_snapshot_budget(deadline, settle_until, cancellation)
}

fn ensure_snapshot_budget(
    deadline: ProviderDeadline,
    settle_until: Instant,
    cancellation: &CancellationToken,
) -> Result<(), ()> {
    if cancellation.is_cancelled()
        || deadline.remaining().is_zero()
        || settle_until
            .saturating_duration_since(Instant::now())
            .is_zero()
    {
        Err(())
    } else {
        Ok(())
    }
}

fn is_related_source_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("bsl" | "xml" | "yaml" | "yml")
    )
}

fn unavailable_related_sections(
    sections: &[String],
    freshness: MetaFreshness,
    message: &str,
) -> MetaRelatedSections {
    let section = || MetaRelatedSection {
        status: MetaRelatedStatus::Unavailable,
        freshness,
        completeness: MetaCompleteness::Unknown,
        total: 0,
        returned: 0,
        truncated: false,
        items: Vec::new(),
        diagnostics: vec![MetaDiagnostic::error(
            MetaDiagnosticCode::ProviderUnavailable,
            message,
        )],
    };
    MetaRelatedSections {
        modules: requested(sections, "modules").then(section),
        roles: requested(sections, "roles").then(section),
        subscriptions: requested(sections, "subscriptions").then(section),
        functional_options: requested(sections, "functionalOptions").then(section),
        predefined_items: requested(sections, "predefinedItems").then(section),
    }
}

fn related_sections_from_profile(
    profile: MetaProfileResult,
    sections: &[String],
    limit: usize,
    freshness: MetaFreshness,
) -> MetaRelatedSections {
    let find = |name: &str| {
        profile
            .sections
            .iter()
            .find(|section| section.name == name)
            .map(|section| typed_related_section(section, limit, freshness))
            .unwrap_or_else(|| MetaRelatedSection {
                status: MetaRelatedStatus::Unavailable,
                freshness,
                completeness: MetaCompleteness::Unknown,
                total: 0,
                returned: 0,
                truncated: false,
                items: Vec::new(),
                diagnostics: vec![MetaDiagnostic::error(
                    MetaDiagnosticCode::ProviderUnavailable,
                    format!("related metadata section `{name}` is unavailable"),
                )],
            })
    };
    MetaRelatedSections {
        modules: requested(sections, "modules").then(|| find("modules")),
        roles: requested(sections, "roles").then(|| find("roles")),
        subscriptions: requested(sections, "subscriptions").then(|| find("subscriptions")),
        functional_options: requested(sections, "functionalOptions")
            .then(|| find("functionalOptions")),
        predefined_items: requested(sections, "predefinedItems").then(|| find("predefinedItems")),
    }
}

fn requested(sections: &[String], name: &str) -> bool {
    sections.iter().any(|section| section == name)
}

fn typed_related_section(
    section: &MetaProfileSection,
    limit: usize,
    freshness: MetaFreshness,
) -> MetaRelatedSection<Value> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();
    for item in &section.items {
        let sanitized = sanitize_related_item(item.clone());
        let key = serde_json::to_string(&sanitized).unwrap_or_default();
        if seen.insert(key) {
            unique.push(sanitized);
        }
    }
    let source_truncated =
        section.total_is_lower_bound || section.returned < section.total || unique.len() > limit;
    let total = if section.total_is_lower_bound {
        usize::try_from(section.total).unwrap_or(usize::MAX)
    } else if section.returned == section.total {
        unique.len()
    } else {
        usize::try_from(section.total).unwrap_or(usize::MAX)
    };
    unique.truncate(limit);
    let status = match section.status.as_str() {
        "ok" | "empty" => MetaRelatedStatus::Ready,
        "partial" => MetaRelatedStatus::Partial,
        _ if !unique.is_empty() => MetaRelatedStatus::Partial,
        _ => MetaRelatedStatus::Unavailable,
    };
    let diagnostics = if section.error.is_some() || status != MetaRelatedStatus::Ready {
        vec![MetaDiagnostic::error(
            MetaDiagnosticCode::ProviderUnavailable,
            format!(
                "related metadata section `{}` is not complete",
                section.name
            ),
        )]
    } else {
        Vec::new()
    };
    MetaRelatedSection {
        status,
        freshness,
        completeness: if status == MetaRelatedStatus::Unavailable {
            MetaCompleteness::Unknown
        } else if status == MetaRelatedStatus::Partial || source_truncated {
            MetaCompleteness::Partial
        } else {
            MetaCompleteness::Complete
        },
        total,
        returned: unique.len(),
        truncated: source_truncated || total > unique.len(),
        items: unique,
        diagnostics,
    }
}

fn sanitize_related_item(value: Value) -> Value {
    match value {
        Value::Object(mut object) => {
            for key in [
                "path",
                "file",
                "dbPath",
                "db_path",
                "sourceDir",
                "source_dir",
            ] {
                object.remove(key);
            }
            for value in object.values_mut() {
                *value = sanitize_related_item(std::mem::take(value));
            }
            Value::Object(object)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sanitize_related_item).collect()),
        other => other,
    }
}

/// A navigation answer plus the typed payload the tool publishes, when its
/// contract has one.
#[derive(Debug)]
pub(crate) struct RlmNavigationOutcome {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<CodeIntelligenceReadData>,
}

impl RlmNavigationOutcome {
    fn plain(outcome: AdapterOutcome) -> Self {
        Self {
            outcome,
            data: None,
        }
    }
}

/// ADR-0020: the index serves definition and object profile. The outline is
/// built from the current BSL file, so it has no RLM operation at all and asking
/// for one is a routing defect rather than a runtime condition.
fn operation_for_request(
    request: &CodeIntelligenceReadRequest,
) -> Result<WorkspaceRlmOperation, String> {
    Ok(match request {
        CodeIntelligenceReadRequest::Definition {
            name,
            module_hint,
            limit,
        } => WorkspaceRlmOperation::Definition {
            name: name.clone(),
            module_hint: module_hint.clone(),
            limit: *limit,
        },
        CodeIntelligenceReadRequest::ObjectProfile {
            name,
            sections,
            limit,
        } => WorkspaceRlmOperation::ObjectProfile {
            name: name.clone(),
            sections: sections.clone(),
            limit: *limit,
        },
        CodeIntelligenceReadRequest::Outline { .. } => {
            return Err(format!(
                "{} is built from the current BSL source and has no RLM operation",
                request.operation_name()
            ))
        }
    })
}

/// Reads the index answer as data. A malformed entry becomes a warning instead
/// of a `diagnostic:` line mixed into the report, so the caller can tell a
/// dropped definition from one that was never there.
fn definition_result(value: &Value) -> Result<(CodeDefinitionResult, Vec<String>), String> {
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let definitions = value
        .get("definitions")
        .and_then(Value::as_array)
        .ok_or_else(|| "RLM definition response is missing definitions".to_string())?;
    let mut warnings = Vec::new();
    let mut typed = Vec::new();
    for (index, definition) in definitions.iter().enumerate() {
        let Some(file) = definition
            .get("file")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        else {
            warnings.push(format!(
                "ignored malformed RLM definition #{}: missing file",
                index + 1
            ));
            continue;
        };
        let optional = |key: &str| {
            definition
                .get(key)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        typed.push(CodeDefinition {
            file: file.to_string(),
            line: definition
                .get("line")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            kind: definition
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("method")
                .to_string(),
            params: definition
                .get("params")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
            export: definition
                .get("is_export")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            category: optional("category"),
            object_name: optional("object_name"),
            module_type: optional("module_type"),
        });
    }
    Ok((
        CodeDefinitionResult {
            name,
            definitions: typed,
        },
        warnings,
    ))
}

/// Reads the object profile as data. Section items keep the shape the index
/// gave them instead of being flattened to one line each.
fn profile_result(value: &Value) -> Result<MetaProfileResult, String> {
    let object_name = required_value_string(value, "object_name", "RLM object profile")?;
    let category = value
        .get("category")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let sections = value
        .get("sections")
        .and_then(Value::as_object)
        .ok_or_else(|| "RLM object profile response is missing sections".to_string())?;
    Ok(MetaProfileResult {
        object_name: object_name.to_string(),
        category,
        sections: sections
            .iter()
            .map(|(name, section)| MetaProfileSection {
                name: public_profile_section_name(name).to_string(),
                status: section
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                total: section
                    .get("total")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
                // Upstream counts `total` before applying its item limit, so
                // only a section that cannot count marks the value as a floor.
                total_is_lower_bound: section
                    .pointer("/_meta/total_is_lower_bound")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                returned: section
                    .get("returned")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
                items: section
                    .get("items")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
                error: section
                    .pointer("/_meta/error")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
            })
            .collect(),
    })
}

fn public_profile_section_name(name: &str) -> &str {
    match name {
        "functional_options" => "functionalOptions",
        "predefined_items" => "predefinedItems",
        other => other,
    }
}

fn required_value_string<'a>(
    value: &'a Value,
    key: &str,
    description: &str,
) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{description} response is missing {key}"))
}

fn index_unavailable_outcome(
    request: &CodeIntelligenceReadRequest,
    readiness: IndexReadiness,
) -> AdapterOutcome {
    let tool_name = request.operation_name();
    let warning = readiness_warning(readiness);
    if warning.starts_with(CANCELLED_PREFIX) {
        return AdapterOutcome::cancelled(
            warning
                .strip_prefix(CANCELLED_PREFIX)
                .unwrap_or(&warning)
                .trim(),
        );
    }
    // Definition and object profile still answer something useful without the
    // index, so an unready index is a warning rather than a typed failure.
    let mut outcome = AdapterOutcome::ok(format!(
        "{tool_name} could not use the persistent RLM MCP API"
    ));
    outcome.warnings.push(warning);
    outcome
}

fn cancelled_client_outcome(tool_name: &str, error: &str) -> AdapterOutcome {
    AdapterOutcome::cancelled(format!(
        "{tool_name} {}",
        error.strip_prefix(CANCELLED_PREFIX).unwrap_or(error).trim()
    ))
}

fn readiness_warning(readiness: IndexReadiness) -> String {
    match readiness {
        IndexReadiness::Ready { .. } => {
            unreachable!("ready indexes are handled before readiness warnings")
        }
        IndexReadiness::Missing => "rlm index unavailable: index is missing".to_string(),
        IndexReadiness::Stale { status } => format!("rlm index stale: {status}"),
        IndexReadiness::Building => "rlm index building".to_string(),
        IndexReadiness::Failed(error) | IndexReadiness::Unavailable(error)
            if error.starts_with(CANCELLED_PREFIX) =>
        {
            error
        }
        IndexReadiness::Failed(error) | IndexReadiness::Unavailable(error) => {
            format!("rlm index unavailable: {error}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{operation_for_request, profile_result, RlmNavigationAdapter, RlmNavigationClient};
    use crate::application::AdapterOutcome;
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::CodeIntelligenceReadData;
    use crate::domain::code_intelligence::{
        CodeIntelligenceContext, CodeIntelligenceReadRequest, ProviderDeadline,
    };
    use crate::domain::source_roots::ResolvedSourceRoot;
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::workspace_index::IndexReadiness;
    use crate::infrastructure::workspace_services::{
        WorkspaceRlmOperation, WorkspaceServiceRlmOutput,
    };
    use serde_json::json;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    fn secure_temp_root(label: &str) -> PathBuf {
        std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "{label}-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ))
    }

    #[test]
    fn a_definition_keeps_every_field_the_helper_reported() {
        let (result, warnings) = super::definition_result(&json!({
            "name": "Найти",
            "definitions": [{
                "file": "CommonModules/X/Module.bsl",
                "line": 7,
                "type": "function",
                "is_export": true,
                "params": ["Значение"],
                "category": "CommonModule",
                "object_name": "X",
                "module_type": "Module"
            }]
        }))
        .unwrap();

        assert!(warnings.is_empty());
        let definition = &result.definitions[0];
        assert_eq!(definition.file, "CommonModules/X/Module.bsl");
        assert_eq!(definition.line, 7);
        assert_eq!(definition.kind, "function");
        assert!(definition.export);
        assert_eq!(definition.params, vec!["Значение".to_string()]);
        assert_eq!(definition.category.as_deref(), Some("CommonModule"));
        assert_eq!(definition.module_type.as_deref(), Some("Module"));
    }

    #[test]
    fn a_malformed_definition_is_dropped_with_a_warning_not_listed_as_one() {
        let (result, warnings) = super::definition_result(&json!({
            "name": "Найти",
            "definitions": [
                {"file": "CommonModules/X/Module.bsl", "line": 7, "type": "function"},
                {"line": 11, "type": "procedure"}
            ]
        }))
        .unwrap();

        assert_eq!(result.definitions.len(), 1);
        assert_eq!(result.definitions[0].line, 7);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("missing file"), "{warnings:?}");
    }

    #[test]
    fn a_profile_maps_upstream_section_names_to_public_names() {
        let result = profile_result(&json!({
            "object_name": "Заказ",
            "category": "Document",
            "sections": {
                "functional_options": {
                    "status": "ok",
                    "items": [{"name": "ИспользоватьЗаказы"}],
                    "total": 1,
                    "returned": 1
                },
                "predefined_items": {
                    "status": "empty",
                    "items": [],
                    "total": 0,
                    "returned": 0
                }
            }
        }))
        .unwrap();

        assert_eq!(result.object_name, "Заказ");
        assert_eq!(result.category.as_deref(), Some("Document"));
        let names = result
            .sections
            .iter()
            .map(|section| section.name.as_str())
            .collect::<Vec<_>>();
        assert!(names.contains(&"functionalOptions"), "{names:?}");
        assert!(names.contains(&"predefinedItems"), "{names:?}");
        // Items keep their own shape rather than one line of rendered JSON.
        let options = result
            .sections
            .iter()
            .find(|section| section.name == "functionalOptions")
            .unwrap();
        assert_eq!(options.items[0]["name"], "ИспользоватьЗаказы");
    }

    #[test]
    fn a_section_that_cannot_count_marks_its_total_as_a_lower_bound() {
        let result = profile_result(&json!({
            "object_name": "Заказ",
            "category": "Document",
            "sections": {
                "predefined_items": {
                    "status": "ok",
                    "items": [{"name": "Основной"}],
                    "total": 1,
                    "returned": 1,
                    "has_more": true,
                    "_meta": {
                        "source": "index",
                        "truncated": true,
                        "total_is_lower_bound": true
                    }
                }
            }
        }))
        .unwrap();

        assert!(result.sections[0].total_is_lower_bound);
        assert_eq!(result.sections[0].total, 1);
    }

    /// Upstream counts before applying its item limit, so a limited section
    /// still reports an exact total.
    #[test]
    fn a_limited_section_keeps_its_exact_total() {
        let result = profile_result(&json!({
            "object_name": "Заказ",
            "category": "Document",
            "sections": {
                "structure": {
                    "status": "ok",
                    "items": [{"name": "Реквизит1"}],
                    "total": 100,
                    "returned": 20,
                    "has_more": true
                }
            }
        }))
        .unwrap();

        assert_eq!(result.sections[0].total, 100);
        assert_eq!(result.sections[0].returned, 20);
        assert!(!result.sections[0].total_is_lower_bound);
    }

    #[test]
    fn an_untruncated_section_keeps_its_exact_total() {
        let result = profile_result(&json!({
            "object_name": "Заказ",
            "category": "Document",
            "sections": {
                "predefined_items": {
                    "status": "ok",
                    "items": [{"name": "Основной"}],
                    "total": 1,
                    "returned": 1,
                    "has_more": false,
                    "_meta": {"source": "index", "truncated": false}
                }
            }
        }))
        .unwrap();

        assert_eq!(result.sections[0].total, 1);
        assert!(!result.sections[0].total_is_lower_bound);
    }

    #[test]
    fn object_profile_operation_keeps_predefined_items_request() {
        let request = CodeIntelligenceReadRequest::ObjectProfile {
            name: "Document.Заказ".to_string(),
            sections: Some(vec![
                "structure".to_string(),
                "functionalOptions".to_string(),
                "predefinedItems".to_string(),
            ]),
            limit: 11,
        };
        let operation = operation_for_request(&request).unwrap();

        assert_eq!(
            operation,
            WorkspaceRlmOperation::ObjectProfile {
                name: "Document.Заказ".to_string(),
                sections: Some(vec![
                    "structure".to_string(),
                    "functionalOptions".to_string(),
                    "predefinedItems".to_string()
                ]),
                limit: 11
            }
        );
    }

    struct RecordingClient {
        operations: Mutex<Vec<WorkspaceRlmOperation>>,
    }

    struct CancelledClient {
        cancel_during_call: bool,
    }

    impl RlmNavigationClient for CancelledClient {
        fn readiness(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<IndexReadiness, String> {
            if self.cancel_during_call {
                Ok(IndexReadiness::Ready {
                    db_path: PathBuf::from("/tmp/index.db"),
                })
            } else {
                Err("cancelled: readiness stopped".to_string())
            }
        }

        fn call(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
            _operation: WorkspaceRlmOperation,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<WorkspaceServiceRlmOutput, String> {
            Err("cancelled: provider call stopped".to_string())
        }
    }

    /// The index answers with structure; the tool must publish it rather than
    /// render a line grammar the caller parses back.
    #[test]
    fn a_definition_answer_carries_every_field_the_index_reported() {
        let value = json!({
            "name": "ОбщегоНазначенияКлиентСервер",
            "definitions": [
                {
                    "file": "src/CommonModules/Общий/Ext/Module.bsl",
                    "line": 42,
                    "type": "function",
                    "params": ["Параметр1", "Параметр2 = Неопределено"],
                    "is_export": true,
                    "category": "CommonModule",
                    "object_name": "Общий",
                    "module_type": "Module"
                },
                {"line": 7}
            ]
        });

        let (result, warnings) = super::definition_result(&value).unwrap();

        assert_eq!(result.definitions.len(), 1);
        let definition = &result.definitions[0];
        assert_eq!(definition.line, 42);
        assert_eq!(definition.kind, "function");
        assert_eq!(definition.params.len(), 2);
        assert!(definition.export);
        assert_eq!(definition.object_name.as_deref(), Some("Общий"));
        // A dropped entry is a warning, not a `diagnostic:` line mixed into the
        // report where it reads like a definition.
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("missing file"), "{:?}", warnings);
    }

    #[test]
    fn adapter_normalizes_client_cancellation_from_readiness_and_call() {
        let request = CodeIntelligenceReadRequest::Definition {
            name: "Найти".to_string(),
            module_hint: String::new(),
            limit: 50,
        };
        let context = CodeIntelligenceContext::new(
            WorkspaceContext {
                cwd: PathBuf::from("/workspace"),
                workspace_root: PathBuf::from("/workspace"),
                cache_root: PathBuf::from("/cache"),
                workspace_epoch: 1,
            },
            ResolvedSourceRoot {
                source_set: Some("main".to_string()),
                path: PathBuf::from("/workspace/src"),
            },
        );

        for cancel_during_call in [false, true] {
            let outcome =
                RlmNavigationAdapter::with_client(&CancelledClient { cancel_during_call })
                    .invoke_resolved_cancellable(
                        &request,
                        &context,
                        ProviderDeadline::new(Instant::now() + Duration::from_secs(1)),
                        &CancellationToken::new(),
                    )
                    .expect("client cancellation must be normalized into an outcome")
                    .outcome;

            assert!(!outcome.ok);
            assert!(outcome.summary.contains("cancelled"));
        }
    }

    struct UnreadyClient {
        readiness: IndexReadiness,
    }

    impl RlmNavigationClient for UnreadyClient {
        fn readiness(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<IndexReadiness, String> {
            Ok(self.readiness.clone())
        }

        fn call(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
            _operation: WorkspaceRlmOperation,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<WorkspaceServiceRlmOutput, String> {
            panic!("a not-ready index must never reach the RLM call")
        }
    }

    fn unready_index_outcome(
        request: &CodeIntelligenceReadRequest,
        readiness: IndexReadiness,
    ) -> AdapterOutcome {
        RlmNavigationAdapter::with_client(&UnreadyClient { readiness })
            .invoke_resolved_cancellable(
                request,
                &unready_index_context(),
                ProviderDeadline::new(Instant::now() + Duration::from_secs(1)),
                &CancellationToken::new(),
            )
            .expect("a not-ready index must be reported as an outcome")
            .outcome
    }

    fn unready_index_context() -> CodeIntelligenceContext {
        let source_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/unica_mcp_script_parity/meta-validate-language-aware");
        CodeIntelligenceContext::new(
            WorkspaceContext {
                cwd: PathBuf::from("/workspace"),
                workspace_root: PathBuf::from("/workspace"),
                cache_root: PathBuf::from("/cache"),
                workspace_epoch: 1,
            },
            ResolvedSourceRoot {
                source_set: Some("main".to_string()),
                path: source_root,
            },
        )
    }

    fn outline_request() -> CodeIntelligenceReadRequest {
        CodeIntelligenceReadRequest::Outline {
            path: "CommonModules/X/Ext/Module.bsl".to_string(),
            include_methods: true,
        }
    }

    fn definition_request() -> CodeIntelligenceReadRequest {
        CodeIntelligenceReadRequest::Definition {
            name: "ОбщегоНазначения".to_string(),
            module_hint: String::new(),
            limit: 50,
        }
    }

    #[test]
    fn definition_and_profile_keep_the_warning_only_contract_for_an_unready_index() {
        for request in [
            CodeIntelligenceReadRequest::Definition {
                name: "Найти".to_string(),
                module_hint: String::new(),
                limit: 50,
            },
            CodeIntelligenceReadRequest::ObjectProfile {
                name: "Справочники.Номенклатура".to_string(),
                sections: None,
                limit: 20,
            },
        ] {
            let outcome = unready_index_outcome(&request, IndexReadiness::Missing);

            assert!(outcome.ok, "{}", request.operation_name());
            assert!(outcome.errors.is_empty(), "{}", request.operation_name());
            assert_eq!(
                outcome.warnings,
                vec!["rlm index unavailable: index is missing".to_string()]
            );
        }
    }

    #[test]
    fn a_cancelled_readiness_is_reported_as_cancellation_not_as_an_index_failure() {
        let outcome = unready_index_outcome(
            &definition_request(),
            IndexReadiness::Unavailable("cancelled: readiness stopped".to_string()),
        );

        assert!(!outcome.ok);
        assert!(outcome.summary.contains("cancelled"), "{}", outcome.summary);
        assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    }

    #[test]
    fn the_index_adapter_refuses_to_serve_the_outline() {
        // ADR-0020: the outline is owned by the current-source provider. Reaching
        // this adapter with it is a routing defect, so it fails before any RLM
        // work rather than answering from the index.
        let client = RecordingClient {
            operations: Mutex::new(Vec::new()),
        };
        let error = RlmNavigationAdapter::with_client(&client)
            .invoke_resolved_cancellable(
                &outline_request(),
                &unready_index_context(),
                ProviderDeadline::new(Instant::now() + Duration::from_secs(1)),
                &CancellationToken::new(),
            )
            .unwrap_err();

        assert!(
            error.contains("built from the current BSL source"),
            "{error}"
        );
        assert!(operation_for_request(&outline_request()).is_err());
        assert!(
            client.operations.lock().unwrap().is_empty(),
            "a misrouted outline must not reach the RLM client"
        );
    }

    impl RlmNavigationClient for RecordingClient {
        fn readiness(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<IndexReadiness, String> {
            Ok(IndexReadiness::Ready {
                db_path: PathBuf::from("/tmp/index.db"),
            })
        }

        fn call(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
            operation: WorkspaceRlmOperation,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<WorkspaceServiceRlmOutput, String> {
            self.operations.lock().unwrap().push(operation);
            Ok(WorkspaceServiceRlmOutput {
                result_text: json!({
                    "name": "Найти",
                    "definitions": [],
                    "total": 0,
                    "truncated": false
                })
                .to_string(),
                stderr: String::new(),
            })
        }
    }

    #[test]
    fn adapter_routes_definition_through_rlm_client() {
        let root = secure_temp_root("unica-rlm-navigation");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("v8project.yaml"),
            "source-set:\n  - name: main\n    path: src\n    type: CONFIGURATION\n",
        )
        .unwrap();
        let context = CodeIntelligenceContext::new(
            WorkspaceContext {
                cwd: root.clone(),
                workspace_root: root.clone(),
                cache_root: root.join(".build/unica"),
                workspace_epoch: 1,
            },
            ResolvedSourceRoot {
                source_set: Some("main".to_string()),
                path: root.join("src"),
            },
        );
        let client = RecordingClient {
            operations: Mutex::new(Vec::new()),
        };
        let request = CodeIntelligenceReadRequest::Definition {
            name: "Найти".to_string(),
            module_hint: String::new(),
            limit: 50,
        };

        let outcome = RlmNavigationAdapter::with_client(&client)
            .invoke_resolved_cancellable(
                &request,
                &context,
                ProviderDeadline::new(Instant::now() + Duration::from_secs(1)),
                &CancellationToken::new(),
            )
            .unwrap();

        assert!(outcome.outcome.ok);
        // ADR-0023: an empty answer is an empty list, not a sentence.
        assert!(outcome.outcome.stdout.is_none());
        let Some(CodeIntelligenceReadData::Definition(result)) = outcome.data else {
            panic!("code.definition must answer with typed data");
        };
        assert_eq!(result.name, "Найти");
        assert!(result.definitions.is_empty());
        assert!(outcome.outcome.summary.contains("0 definition(s)"));
        assert_eq!(
            client.operations.lock().unwrap().as_slice(),
            &[WorkspaceRlmOperation::Definition {
                name: "Найти".to_string(),
                module_hint: String::new(),
                limit: 50
            }]
        );
        let _ = std::fs::remove_dir_all(root);
    }

    struct RelatedClient {
        readiness: Result<IndexReadiness, String>,
        result: Result<WorkspaceServiceRlmOutput, String>,
    }

    impl RlmNavigationClient for RelatedClient {
        fn readiness(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<IndexReadiness, String> {
            self.readiness.clone()
        }

        fn call(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
            _operation: WorkspaceRlmOperation,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<WorkspaceServiceRlmOutput, String> {
            self.result.clone()
        }
    }

    fn related_client(readiness: IndexReadiness, value: serde_json::Value) -> RelatedClient {
        RelatedClient {
            readiness: Ok(readiness),
            result: Ok(WorkspaceServiceRlmOutput {
                result_text: value.to_string(),
                stderr: String::new(),
            }),
        }
    }

    #[test]
    fn metadata_related_maps_current_partial_limits_and_opt_in_sections() {
        let client = related_client(
            IndexReadiness::Ready {
                db_path: PathBuf::from("/private/index.db"),
            },
            json!({
                "object_name": "Catalog.Items",
                "sections": {
                    "modules": {
                        "status": "ok",
                        "total": 4,
                        "returned": 4,
                        "items": [
                            {"name": "ObjectModule", "path": "/must/not/leak"},
                            {"name": "ObjectModule"},
                            {"name": "ManagerModule"},
                            {"name": "Second"}
                        ]
                    },
                    "roles": {
                        "status": "partial",
                        "total": 2,
                        "returned": 1,
                        "items": [{"name": "Reader"}],
                        "_meta": {"error": "one role could not be classified"}
                    },
                    "predefined_items": {
                        "status": "ok",
                        "total": 1,
                        "returned": 1,
                        "items": [{"name": "Main"}]
                    }
                }
            }),
        );
        let sections = vec![
            "modules".to_string(),
            "roles".to_string(),
            "predefinedItems".to_string(),
        ];

        let related = RlmNavigationAdapter::with_client(&client).metadata_related(
            "Catalog.Items",
            &sections,
            2,
            &unready_index_context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(1)),
            &CancellationToken::new(),
        );

        let modules = related.modules.unwrap();
        assert_eq!(
            modules.status,
            crate::domain::metadata::MetaRelatedStatus::Ready
        );
        assert_eq!(
            modules.freshness,
            crate::domain::metadata::MetaFreshness::Current
        );
        assert_eq!(
            (modules.total, modules.returned, modules.truncated),
            (3, 2, true)
        );
        assert_eq!(modules.items[0], json!({"name": "ObjectModule"}));
        assert_eq!(modules.items[1], json!({"name": "ManagerModule"}));
        let roles = related.roles.unwrap();
        assert_eq!(
            roles.status,
            crate::domain::metadata::MetaRelatedStatus::Partial
        );
        assert_eq!(roles.diagnostics.len(), 1);
        assert!(related.subscriptions.is_none());
        assert!(related.functional_options.is_none());
        assert!(related.predefined_items.is_some());
    }

    #[test]
    fn metadata_related_never_labels_stale_or_failed_readiness_current() {
        let stale = related_client(
            IndexReadiness::Stale {
                status: "stale (content)".to_string(),
            },
            json!({
                "object_name": "Catalog.Items",
                "sections": {"modules": {"status": "ok", "total": 1, "returned": 1, "items": [{"name": "ObjectModule"}]}}
            }),
        );
        let requested = vec!["modules".to_string()];
        let stale_result = RlmNavigationAdapter::with_client(&stale).metadata_related(
            "Catalog.Items",
            &requested,
            20,
            &unready_index_context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(1)),
            &CancellationToken::new(),
        );
        assert_eq!(
            stale_result.modules.unwrap().freshness,
            crate::domain::metadata::MetaFreshness::Stale
        );

        for client in [
            RelatedClient {
                readiness: Err("service start failed at /private/root".to_string()),
                result: Err("must not call".to_string()),
            },
            RelatedClient {
                readiness: Ok(IndexReadiness::Ready {
                    db_path: PathBuf::from("/private/index.db"),
                }),
                result: Err("provider timed out at /private/root".to_string()),
            },
        ] {
            let result = RlmNavigationAdapter::with_client(&client).metadata_related(
                "Catalog.Items",
                &requested,
                20,
                &unready_index_context(),
                ProviderDeadline::new(Instant::now() + Duration::from_secs(1)),
                &CancellationToken::new(),
            );
            let section = result.modules.unwrap();
            assert_eq!(
                section.status,
                crate::domain::metadata::MetaRelatedStatus::Unavailable
            );
            assert_eq!(
                section.freshness,
                crate::domain::metadata::MetaFreshness::Unknown
            );
            let serialized = serde_json::to_string(&section.diagnostics).unwrap();
            assert!(!serialized.contains("/private"), "{serialized}");
            assert!(!serialized.contains("RLM"), "{serialized}");
        }

        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let result = RlmNavigationAdapter::with_client(&stale).metadata_related(
            "Catalog.Items",
            &requested,
            20,
            &unready_index_context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(1)),
            &cancelled,
        );
        assert_eq!(
            result.modules.unwrap().status,
            crate::domain::metadata::MetaRelatedStatus::Unavailable
        );

        let result = RlmNavigationAdapter::with_client(&stale).metadata_related(
            "Catalog.Items",
            &requested,
            20,
            &unready_index_context(),
            ProviderDeadline::new(Instant::now()),
            &CancellationToken::new(),
        );
        assert_eq!(
            result.modules.unwrap().freshness,
            crate::domain::metadata::MetaFreshness::Unknown
        );
    }

    struct SourceChangingRelatedClient {
        source_file: PathBuf,
        replacement: Vec<u8>,
        original_modified: std::time::SystemTime,
    }

    impl RlmNavigationClient for SourceChangingRelatedClient {
        fn readiness(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<IndexReadiness, String> {
            Ok(IndexReadiness::Ready {
                db_path: PathBuf::from("/private/index.db"),
            })
        }

        fn call(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
            _operation: WorkspaceRlmOperation,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<WorkspaceServiceRlmOutput, String> {
            std::fs::write(&self.source_file, &self.replacement).unwrap();
            std::fs::File::options()
                .write(true)
                .open(&self.source_file)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(self.original_modified))
                .unwrap();
            Ok(WorkspaceServiceRlmOutput {
                result_text: json!({
                    "object_name": "Catalog.Items",
                    "sections": {"modules": {"status": "ok", "total": 1, "returned": 1, "items": [{"name": "ObjectModule"}]}}
                })
                .to_string(),
                stderr: String::new(),
            })
        }
    }

    #[test]
    fn metadata_related_never_marks_a_changed_real_source_snapshot_current() {
        let root = secure_temp_root("unica-related-generation");
        let source_root = root.join("src");
        std::fs::create_dir_all(source_root.join("CommonModules/Test/Ext")).unwrap();
        let source_file = source_root.join("CommonModules/Test/Ext/Module.bsl");
        let initial = b"initial snapshot";
        let replacement = b"changed snapshot";
        assert_eq!(initial.len(), replacement.len());
        std::fs::write(&source_file, initial).unwrap();
        let original_modified = std::fs::metadata(&source_file).unwrap().modified().unwrap();
        let context = CodeIntelligenceContext::new(
            WorkspaceContext {
                cwd: root.clone(),
                workspace_root: root.clone(),
                cache_root: root.join(".build/unica"),
                workspace_epoch: 1,
            },
            ResolvedSourceRoot {
                source_set: Some("main".to_string()),
                path: source_root,
            },
        );

        let result = RlmNavigationAdapter::with_client(&SourceChangingRelatedClient {
            source_file,
            replacement: replacement.to_vec(),
            original_modified,
        })
        .metadata_related(
            "Catalog.Items",
            &["modules".to_string()],
            20,
            &context,
            ProviderDeadline::new(Instant::now() + Duration::from_secs(1)),
            &CancellationToken::new(),
        );

        assert_eq!(
            result.modules.unwrap().freshness,
            crate::domain::metadata::MetaFreshness::Stale
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn source_snapshot_rejects_cancellation_after_the_last_large_hash_update() {
        let root = secure_temp_root("unica-related-post-hash-cancel");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("Large.bsl"), vec![b'x'; 4 * 1024 * 1024]).unwrap();
        let cancellation = CancellationToken::new();
        let cancellation_for_hook = cancellation.clone();

        let result = super::with_related_snapshot_hash_hook(
            move |phase| {
                if matches!(
                    phase,
                    super::RelatedSnapshotHashPhase::AfterEntry {
                        ordinal: 0,
                        total: 1,
                        ..
                    }
                ) {
                    cancellation_for_hook.cancel();
                }
            },
            || {
                super::related_source_content_snapshot(
                    &root,
                    ProviderDeadline::new(Instant::now() + Duration::from_secs(1)),
                    Instant::now() + Duration::from_secs(1),
                    &cancellation,
                )
            },
        );

        assert!(
            result.is_err(),
            "the last hashed bytes need a post-update checkpoint"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn source_snapshot_rejects_cancellation_after_hash_finalize() {
        let root = secure_temp_root("unica-related-post-finalize-cancel");
        std::fs::create_dir_all(&root).unwrap();
        let cancellation = CancellationToken::new();
        let cancellation_for_hook = cancellation.clone();

        let result = super::with_related_snapshot_hash_hook(
            move |phase| {
                if phase == &super::RelatedSnapshotHashPhase::AfterFinalize {
                    cancellation_for_hook.cancel();
                }
            },
            || {
                super::related_source_content_snapshot(
                    &root,
                    ProviderDeadline::new(Instant::now() + Duration::from_secs(1)),
                    Instant::now() + Duration::from_secs(1),
                    &cancellation,
                )
            },
        );

        assert!(
            result.is_err(),
            "a finalized digest needs one last budget checkpoint"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn metadata_related_maps_post_capture_hash_cancellation_to_unavailable_unknown() {
        let root = secure_temp_root("unica-related-post-hash-soft-failure");
        let source_root = root.join("src");
        std::fs::create_dir_all(&source_root).unwrap();
        std::fs::write(source_root.join("Large.bsl"), vec![b'x'; 4 * 1024 * 1024]).unwrap();
        let context = CodeIntelligenceContext::new(
            WorkspaceContext {
                cwd: root.clone(),
                workspace_root: root.clone(),
                cache_root: root.join(".build/unica"),
                workspace_epoch: 1,
            },
            ResolvedSourceRoot {
                source_set: Some("main".to_string()),
                path: source_root,
            },
        );
        let cancellation = CancellationToken::new();
        let cancellation_for_hook = cancellation.clone();
        let client = related_client(
            IndexReadiness::Ready {
                db_path: PathBuf::from("/private/index.db"),
            },
            json!({
                "object_name": "Catalog.Items",
                "sections": {"modules": {"status": "ok", "total": 1, "returned": 1, "items": [{"name": "ObjectModule"}]}}
            }),
        );

        let related = super::with_related_snapshot_hash_hook(
            move |phase| {
                if matches!(
                    phase,
                    super::RelatedSnapshotHashPhase::AfterEntry {
                        ordinal: 0,
                        total: 1,
                        ..
                    }
                ) {
                    cancellation_for_hook.cancel();
                }
            },
            || {
                RlmNavigationAdapter::with_client(&client).metadata_related(
                    "Catalog.Items",
                    &["modules".to_string()],
                    20,
                    &context,
                    ProviderDeadline::new(Instant::now() + Duration::from_secs(1)),
                    &cancellation,
                )
            },
        );

        let modules = related.modules.unwrap();
        assert_eq!(
            modules.status,
            crate::domain::metadata::MetaRelatedStatus::Unavailable
        );
        assert_eq!(
            modules.freshness,
            crate::domain::metadata::MetaFreshness::Unknown
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn metadata_related_does_not_claim_current_for_an_oversized_source_corpus() {
        let root = secure_temp_root("unica-related-oversized");
        let source_root = root.join("src");
        std::fs::create_dir_all(&source_root).unwrap();
        std::fs::File::create(source_root.join("Oversized.bsl"))
            .unwrap()
            .set_len(64 * 1024 * 1024 + 1)
            .unwrap();
        let context = CodeIntelligenceContext::new(
            WorkspaceContext {
                cwd: root.clone(),
                workspace_root: root.clone(),
                cache_root: root.join(".build/unica"),
                workspace_epoch: 1,
            },
            ResolvedSourceRoot {
                source_set: Some("main".to_string()),
                path: source_root,
            },
        );
        let client = related_client(
            IndexReadiness::Ready {
                db_path: PathBuf::from("/private/index.db"),
            },
            json!({
                "object_name": "Catalog.Items",
                "sections": {"modules": {"status": "ok", "total": 1, "returned": 1, "items": [{"name": "ObjectModule"}]}}
            }),
        );

        let result = RlmNavigationAdapter::with_client(&client).metadata_related(
            "Catalog.Items",
            &["modules".to_string()],
            20,
            &context,
            ProviderDeadline::new(Instant::now() + Duration::from_secs(1)),
            &CancellationToken::new(),
        );

        let modules = result.modules.unwrap();
        assert_eq!(
            modules.status,
            crate::domain::metadata::MetaRelatedStatus::Unavailable
        );
        assert_eq!(
            modules.freshness,
            crate::domain::metadata::MetaFreshness::Unknown
        );
        let _ = std::fs::remove_dir_all(root);
    }

    struct BuildingThenReadyClient {
        readiness_calls: Mutex<usize>,
    }

    impl RlmNavigationClient for BuildingThenReadyClient {
        fn readiness(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<IndexReadiness, String> {
            let mut calls = self.readiness_calls.lock().unwrap();
            *calls += 1;
            Ok(if *calls == 1 {
                IndexReadiness::Building
            } else {
                IndexReadiness::Ready {
                    db_path: PathBuf::from("/private/index.db"),
                }
            })
        }

        fn call(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
            _operation: WorkspaceRlmOperation,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<WorkspaceServiceRlmOutput, String> {
            Ok(WorkspaceServiceRlmOutput {
                result_text: json!({
                    "object_name": "Catalog.Items",
                    "sections": {"modules": {"status": "ok", "total": 0, "returned": 0, "items": []}}
                })
                .to_string(),
                stderr: String::new(),
            })
        }
    }

    #[test]
    fn metadata_related_boundedly_waits_for_a_production_building_state() {
        let client = BuildingThenReadyClient {
            readiness_calls: Mutex::new(0),
        };

        let result = RlmNavigationAdapter::with_client(&client).metadata_related(
            "Catalog.Items",
            &["modules".to_string()],
            20,
            &unready_index_context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(1)),
            &CancellationToken::new(),
        );

        let modules = result.modules.unwrap();
        assert_eq!(
            modules.status,
            crate::domain::metadata::MetaRelatedStatus::Ready
        );
        assert_eq!(
            modules.freshness,
            crate::domain::metadata::MetaFreshness::Current
        );
        assert!(*client.readiness_calls.lock().unwrap() >= 3);
    }

    struct AlwaysBuildingClient;

    impl RlmNavigationClient for AlwaysBuildingClient {
        fn readiness(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<IndexReadiness, String> {
            Ok(IndexReadiness::Building)
        }

        fn call(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
            _operation: WorkspaceRlmOperation,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<WorkspaceServiceRlmOutput, String> {
            Ok(WorkspaceServiceRlmOutput {
                result_text: json!({
                    "object_name": "Catalog.Items",
                    "sections": {"modules": {"status": "ok", "total": 1, "returned": 1, "items": [{"name": "ObjectModule"}]}}
                })
                .to_string(),
                stderr: String::new(),
            })
        }
    }

    #[test]
    fn metadata_related_bounds_a_build_that_does_not_settle_and_keeps_data_stale() {
        let started = Instant::now();

        let result = RlmNavigationAdapter::with_client(&AlwaysBuildingClient).metadata_related(
            "Catalog.Items",
            &["modules".to_string()],
            20,
            &unready_index_context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(2)),
            &CancellationToken::new(),
        );

        assert!(
            started.elapsed() < Duration::from_millis(900),
            "a non-settling build must consume at most one bounded readiness window"
        );
        let modules = result.modules.unwrap();
        assert_eq!(
            modules.freshness,
            crate::domain::metadata::MetaFreshness::Stale
        );
        assert_eq!(modules.items, vec![json!({"name": "ObjectModule"})]);
    }

    fn selected_related_section<'a>(
        related: &'a crate::domain::metadata::MetaRelatedSections,
        name: &str,
    ) -> &'a crate::domain::metadata::MetaRelatedSection<serde_json::Value> {
        match name {
            "modules" => related.modules.as_ref(),
            "roles" => related.roles.as_ref(),
            "subscriptions" => related.subscriptions.as_ref(),
            "functionalOptions" => related.functional_options.as_ref(),
            "predefinedItems" => related.predefined_items.as_ref(),
            _ => None,
        }
        .unwrap_or_else(|| panic!("missing selected section {name}"))
    }

    #[test]
    fn metadata_related_maps_every_section_through_the_complete_status_matrix() {
        use crate::domain::metadata::{MetaFreshness, MetaRelatedStatus};

        let section_names = [
            ("modules", "modules"),
            ("roles", "roles"),
            ("subscriptions", "subscriptions"),
            ("functionalOptions", "functional_options"),
            ("predefinedItems", "predefined_items"),
        ];
        for (public_name, upstream_name) in section_names {
            for scenario in ["ready", "partial", "stale", "unavailable"] {
                let (readiness, section, want_status, want_freshness, want_counts) = match scenario
                {
                    "ready" => (
                        IndexReadiness::Ready {
                            db_path: PathBuf::from("/private/index.db"),
                        },
                        Some(json!({
                            "status": "ok",
                            "total": 4,
                            "returned": 4,
                            "items": [
                                {"name": "First", "path": "/private/root", "nested": {"file": "/private/file"}},
                                {"name": "First", "nested": {}},
                                {"name": "Second"},
                                {"name": "Third"}
                            ]
                        })),
                        MetaRelatedStatus::Ready,
                        MetaFreshness::Current,
                        (3, 2, true),
                    ),
                    "partial" => (
                        IndexReadiness::Ready {
                            db_path: PathBuf::from("/private/index.db"),
                        },
                        Some(json!({
                            "status": "partial",
                            "total": 4,
                            "returned": 2,
                            "items": [{"name": "First"}, {"name": "Second"}],
                            "_meta": {"error": "incomplete provider answer"}
                        })),
                        MetaRelatedStatus::Partial,
                        MetaFreshness::Current,
                        (4, 2, true),
                    ),
                    "stale" => (
                        IndexReadiness::Stale {
                            status: "stale (content)".to_string(),
                        },
                        Some(json!({
                            "status": "ok",
                            "total": 1,
                            "returned": 1,
                            "items": [{"name": "First"}]
                        })),
                        MetaRelatedStatus::Ready,
                        MetaFreshness::Stale,
                        (1, 1, false),
                    ),
                    "unavailable" => (
                        IndexReadiness::Ready {
                            db_path: PathBuf::from("/private/index.db"),
                        },
                        None,
                        MetaRelatedStatus::Unavailable,
                        MetaFreshness::Current,
                        (0, 0, false),
                    ),
                    _ => unreachable!(),
                };
                let mut sections = serde_json::Map::new();
                if let Some(section) = section {
                    sections.insert(upstream_name.to_string(), section);
                }
                let client = related_client(
                    readiness,
                    json!({"object_name": "Catalog.Items", "sections": sections}),
                );

                let related = RlmNavigationAdapter::with_client(&client).metadata_related(
                    "Catalog.Items",
                    &[public_name.to_string()],
                    2,
                    &unready_index_context(),
                    ProviderDeadline::new(Instant::now() + Duration::from_secs(1)),
                    &CancellationToken::new(),
                );
                let section = selected_related_section(&related, public_name);

                assert_eq!(section.status, want_status, "{public_name}/{scenario}");
                assert_eq!(
                    section.freshness, want_freshness,
                    "{public_name}/{scenario}"
                );
                assert_eq!(
                    (section.total, section.returned, section.truncated),
                    want_counts,
                    "{public_name}/{scenario}"
                );
                if scenario == "ready" {
                    assert_eq!(section.items[0], json!({"name": "First", "nested": {}}));
                    assert_eq!(section.items[1], json!({"name": "Second"}));
                    assert!(!serde_json::to_string(section).unwrap().contains("/private"));
                }
                assert_eq!(related.modules.is_some(), public_name == "modules");
                assert_eq!(related.roles.is_some(), public_name == "roles");
                assert_eq!(
                    related.subscriptions.is_some(),
                    public_name == "subscriptions"
                );
                assert_eq!(
                    related.functional_options.is_some(),
                    public_name == "functionalOptions"
                );
                assert_eq!(
                    related.predefined_items.is_some(),
                    public_name == "predefinedItems"
                );
            }
        }
    }
}
