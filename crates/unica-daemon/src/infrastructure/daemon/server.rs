use unica_daemon_protocol::identity::CoreIdentity;
use unica_execution::receipt_runtime::ports::V5CanonicalPrepareError;
use unica_refusal::{RefusalCode, RefusalDetail};
#[path = "invocation_service.rs"]
mod invocation_service;
#[cfg(test)]
use self::invocation_service::bind_workspace_invocation;
#[cfg(test)]
pub(crate) use self::invocation_service::test_control as admission_test_control;
#[cfg(test)]
use self::invocation_service::{
    actor_read_source_capability_for_test, actor_read_source_metadata_for_test,
    bind_workspace_invocation_with_source_override_for_test, ActorInvocationResourcesForTest,
};
use self::invocation_service::{
    bind_workspace_invocation_cancellable, ActorInvocationResources, UnadmittedCause,
    WorkspaceAdmissionError,
};
pub(in crate::infrastructure::daemon) use self::invocation_service::{
    text_search, ActorReadSourceCapability,
};
pub(crate) use self::invocation_service::{
    ActorBoundExecution, ActorBoundInvocation, CanonicalInvocationService,
};
use super::protocol::InvocationRequest;
use crate::application::tool_contracts::SurfaceRelease;
pub(super) use invocation_service::workspace_admission::admit_editor_folder;
use unica_analysis_services::provider_host::ProviderHostOwner;
use unica_execution::clock::TokioClock;
use unica_execution::invocation::InvocationResponseDeadline;
use unica_execution::receipt_runtime::ports::{ExecutionClass, KnownLongReason};
use unica_work_control::clock::Clock;
#[cfg(test)]
type RuntimeJobService = unica_execution::runtime_jobs::RuntimeJobService<
    unica_adapter_v8_runner::request::RuntimeJobRequest,
>;
use crate::infrastructure::source_workspace::WorkspaceActorRegistry;
use std::sync::Arc;
use std::time::Duration;
use unica_execution::runtime_jobs::RuntimeResourceOwner;
use unica_execution_contract::invocation::{DomainResult, InvocationFailure};
use unica_work_control::cancellation::CancellationToken;

/// Безымянный неуспех — только для стенда.
///
/// На проводе такого ответа нет: у отказа канонической поверхности есть код из
/// закрытого словаря, исход и продолжение, см. [`reject_workspace_admission`].
#[cfg(test)]
fn failed_domain_result(summary: &str) -> DomainResult {
    let mut result = DomainResult::success(summary);
    result.ok = false;
    result
}

/// Отказ допуска словами: код из закрытого словаря, названная причина и
/// маршрут дальше.
///
/// Из того же каталога `unica.view {}` отвечает фактами и рецептом
/// `v8project.yaml`, `unica.check {}` — вердиктом, `unica.run {}` — словарём
/// инициализации. Всем трём набор исходников не нужен, поэтому отказ ведёт
/// туда, а не в тупик.
fn reject_workspace_admission(
    request: &InvocationRequest,
    error: WorkspaceAdmissionError,
) -> V5CanonicalPrepareError {
    // Отравленный реестр — состояние демона, а не рабочего пространства:
    // у него свой маршрут, и объяснений про наборы он не ждёт.
    let daemon_state = matches!(error, WorkspaceAdmissionError::RegistryFailed);
    if !daemon_state {
        // Недоступная операция `run` объясняется своим словарём: спрашивали не
        // о наборах, и рассказ о них увёл бы в сторону.
        if let Some(result) =
            super::v13_run_dictionary::reject_unavailable_run_before_admission(request)
        {
            return V5CanonicalPrepareError::Direct(Box::new(result));
        }
    }
    let (workspace_root, cause) = match error {
        WorkspaceAdmissionError::RegistryFailed => {
            return V5CanonicalPrepareError::WorkspaceRegistryFailed
        }
        // Корня нет — фактов о нём тоже нет, и звать за ними некуда.
        WorkspaceAdmissionError::WorkspaceUndiscoverable { reason } => {
            return V5CanonicalPrepareError::Rejected(Box::new(DomainResult::canonical_rejection(
                None,
                RefusalCode::ProviderUnavailable,
                format!("workspace discovery failed: {reason}"),
            )))
        }
        WorkspaceAdmissionError::Unadmitted {
            workspace_root,
            cause,
        } => (workspace_root, cause),
    };

    let facts = || {
        super::v13_workspace_bootstrap::next_action(
            "unica.view",
            serde_json::Value::Object(serde_json::Map::new()),
            "source sets, infobase target, and the recommended v8project.yaml content",
        )
    };
    let verdict = || {
        super::v13_workspace_bootstrap::next_action(
            "unica.check",
            serde_json::Value::Object(serde_json::Map::new()),
            "the workspace readiness verdict with its diagnostics",
        )
    };
    let initialization = || {
        super::v13_workspace_bootstrap::next_action(
            "unica.run",
            serde_json::Value::Object(serde_json::Map::new()),
            "the initialization dictionary: source, CF/DT, and existing-infobase routes",
        )
    };
    let retry = || {
        super::v13_workspace_bootstrap::next_action(
            &format!("unica.{}", request.tool().catalog_name()),
            serde_json::Value::Object(request.arguments().clone()),
            "retry the same call after the source root becomes available",
        )
    };

    let mut payload = serde_json::Map::new();
    payload.insert(
        "workspaceRoot".to_string(),
        serde_json::Value::String(workspace_root.to_string_lossy().into_owned()),
    );
    let (code, summary, next) = match cause {
        UnadmittedCause::Uninitialized => (
            RefusalCode::InvalidState,
            "no PlatformXml source set is admitted: workspace is uninitialized; no v8project.yaml or 1C source roots were found"
                .to_string(),
            vec![facts(), verdict(), initialization()],
        ),
        UnadmittedCause::NoPlatformXmlSourceSet(declared) => {
            let declared = serde_json::to_value(declared).expect("project source sets serialize");
            let formats = declared
                .as_array()
                .map(|sets| {
                    let mut formats = sets
                        .iter()
                        .filter_map(|source| source["sourceFormat"].as_str())
                        .collect::<Vec<_>>();
                    formats.sort_unstable();
                    formats.dedup();
                    formats.join(", ")
                })
                .unwrap_or_default();
            let count = declared.as_array().map_or(0, Vec::len);
            payload.insert("sourceSets".to_string(), declared);
            (
                RefusalCode::InvalidSource,
                format!(
                    "no PlatformXml source set is admitted: all {count} declared source sets are in a format this surface does not read ({formats})"
                ),
                vec![facts(), verdict(), initialization()],
            )
        }
        UnadmittedCause::SourceRootUnreadable {
            source_set,
            path,
            reason,
        } => {
            payload.insert(
                "sourceSet".to_string(),
                serde_json::Value::String(source_set.clone()),
            );
            payload.insert("path".to_string(), serde_json::Value::String(path.clone()));
            (
                RefusalCode::InvalidSource,
                format!(
                    "no PlatformXml source set is admitted: source set `{source_set}` declares `{path}`, and {reason}"
                ),
                vec![facts(), verdict()],
            )
        }
        // Те же слова, что у корня: причина одна, и разойтись им нельзя.
        UnadmittedCause::ProjectConfigInvalid(reason) => {
            payload.insert(
                "config".to_string(),
                serde_json::json!({"state": "invalid", "path": "v8project.yaml"}),
            );
            (
                RefusalCode::InvalidState,
                format!("v8project.yaml is present but invalid: {reason}"),
                vec![facts(), verdict()],
            )
        }
        UnadmittedCause::SourceDiscoveryFailed(reason) => (
            RefusalCode::ProviderUnavailable,
            format!("workspace source discovery failed: {reason}"),
            vec![verdict()],
        ),
        // Допуск остановила явная отмена задачи: продолжения нет, конечный
        // исход задачи уже опубликован отменой.
        UnadmittedCause::AdmissionCancelled => (
            RefusalCode::Cancelled,
            "workspace actor admission was cancelled".to_string(),
            Vec::new(),
        ),
        UnadmittedCause::ActorBindingFailed { stage } => (
            RefusalCode::InvalidState,
            format!("workspace actor admission failed: {stage}"),
            vec![facts(), verdict()],
        ),
        UnadmittedCause::ActorRootUnavailable => {
            let mut result = DomainResult::canonical_rejection_detailed(
                None,
                RefusalDetail::SourceTemporarilyUnavailable,
                "workspace actor source root is temporarily unavailable; the retained actor and its saved plans were preserved",
            );
            result.data = Some(serde_json::Value::Object(payload));
            result.next = vec![retry()];
            super::v13_workspace_bootstrap::annotate_workspace_origin(
                &mut result,
                request.workspace_hint(),
                request.workspace_origin(),
            );
            return V5CanonicalPrepareError::Rejected(Box::new(result));
        }
    };

    let mut result = DomainResult::canonical_rejection(None, code, summary);
    result.data = Some(serde_json::Value::Object(payload));
    result.next = next;
    super::v13_workspace_bootstrap::annotate_workspace_origin(
        &mut result,
        request.workspace_hint(),
        request.workspace_origin(),
    );
    V5CanonicalPrepareError::Rejected(Box::new(result))
}

/// Адрес `<узел>.Can.<op>` называет операцию словаря узла, а не узел
/// исходников: его читает только `unica.view`. Остальные инструменты
/// отвечают на него одним отказом с переходом к этому чтению, а не тем,
/// что маршрутизация скажет о несуществующем узле. `diff` отказывает сам:
/// у него свой код (`target_kind_unsupported`).
fn operation_address_outside_view(request: &InvocationRequest) -> Option<DomainResult> {
    use unica_execution::invocation_store::ToolIdentity;
    let arguments = request.arguments();
    let text = |value: Option<&serde_json::Value>| {
        value
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    };
    let mut candidates: Vec<(String, Option<String>)> = Vec::new();
    match request.tool() {
        ToolIdentity::Check | ToolIdentity::Resolve => {
            candidates.push(("at".to_string(), text(arguments.get("at"))));
        }
        ToolIdentity::Search => {
            candidates.push(("scope".to_string(), text(arguments.get("scope"))));
        }
        ToolIdentity::Apply => {
            candidates.push(("at".to_string(), text(arguments.get("at"))));
            for (index, operation) in arguments
                .get("ops")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                candidates.push((format!("ops[{index}].at"), text(operation.get("at"))));
                candidates.push((
                    format!("ops[{index}].args.at"),
                    text(operation.get("args").and_then(|args| args.get("at"))),
                ));
            }
        }
        _ => return None,
    }
    let address = candidates.into_iter().find_map(|(location, raw)| {
        let raw = raw?;
        let parsed = unica_address::address::QualifiedAddress::parse(&raw).ok()?;
        let what = if parsed.operation_query().is_some() {
            "an operation-dictionary"
        } else if parsed.property_query().is_some() {
            // Свойство узла (#2088) — не узел исходников: проверять, менять
            // и искать в нём нечего, его читает view.
            "a node-property"
        } else {
            return None;
        };
        Some((location, raw, what))
    })?;
    let (location, address, what) = (address.0, address.1, address.2);
    let mut result = DomainResult::canonical_rejection(
        Some(location),
        RefusalCode::BadValue,
        format!("`{address}` is {what} address; only unica.view reads it"),
    );
    result.next.push(serde_json::json!({
        "tool": "unica.view",
        "args": {"at": address},
        "reason": if what == "a node-property" {
            "свойство узла читает view"
        } else {
            "контракт операции читает view"
        },
    }));
    Some(result)
}

fn validate_hidden_v13_request(request: &InvocationRequest) -> Result<(), String> {
    if request.tool() == unica_execution::invocation_store::ToolIdentity::Apply {
        let arguments = request.arguments();
        if arguments.contains_key("dryRun") || arguments.contains_key("ifRev") {
            return Err("unica.apply no longer accepts dryRun or ifRev. Call with at and ops to get a saved plan and data.executionToken, then call with executionToken only to execute that plan".to_string());
        }
        if arguments.contains_key("executionToken") {
            if arguments.len() != 1 {
                return Err("executionToken must be the only apply argument; at and ops belong to the planning call".to_string());
            }
            if arguments["executionToken"]
                .as_str()
                .is_none_or(|token| token.is_empty())
            {
                return Err(
                    "executionToken must be non-empty text returned by a successful apply plan"
                        .to_string(),
                );
            }
        } else if !arguments.contains_key("at") || !arguments.contains_key("ops") {
            return Err("call unica.apply with at and ops to prepare a plan, or with executionToken only to execute a saved plan".to_string());
        }
    }
    let catalog = crate::application::v13::tool_catalog::catalog_for(SurfaceRelease::V13)
        .ok_or_else(|| "canonical v0.13 catalog is unavailable".to_string())?;
    let contract = catalog
        .tools
        .iter()
        .find(|contract| contract.name == request.tool().catalog_name())
        .ok_or_else(|| "canonical tool identity is not in the v0.13 catalog".to_string())?;
    let schema = contract
        .input_schema
        .as_object()
        .ok_or_else(|| "canonical tool schema is not an object".to_string())?;
    let properties = schema
        .get("properties")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| "canonical tool schema has no properties".to_string())?;
    if let Some(unknown) = request
        .arguments()
        .keys()
        .find(|name| !properties.contains_key(*name))
    {
        return Err(format!(
            "canonical invocation has unknown argument `{unknown}`"
        ));
    }
    for required in schema
        .get("required")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
    {
        if !request.arguments().contains_key(required) {
            return Err(format!(
                "canonical invocation requires argument `{required}`"
            ));
        }
    }
    for (name, value) in request.arguments() {
        validate_canonical_value(name, value, &properties[name])?;
    }
    Ok(())
}

fn validate_canonical_value(
    name: &str,
    value: &serde_json::Value,
    schema: &serde_json::Value,
) -> Result<(), String> {
    let matches_type = |expected: Option<&str>| match expected {
        Some("string") => value.is_string(),
        Some("boolean") => value.is_boolean(),
        Some("integer") => value.as_i64().is_some() || value.as_u64().is_some(),
        Some("object") => value.is_object(),
        Some("array") => value.is_array(),
        Some("null") => value.is_null(),
        Some(_) | None => true,
    };
    let type_matches = match schema.get("type") {
        Some(serde_json::Value::Array(types)) => {
            types.iter().any(|expected| matches_type(expected.as_str()))
        }
        expected => matches_type(expected.and_then(serde_json::Value::as_str)),
    };
    if !type_matches {
        return Err(format!(
            "canonical invocation argument `{name}` has the wrong type"
        ));
    }
    if matches!(name, "at" | "scope" | "left" | "right")
        && value
            .as_str()
            .is_some_and(|text| text.trim().is_empty() || text.chars().any(char::is_control))
    {
        return Err(format!(
            "canonical invocation address argument `{name}` is invalid"
        ));
    }
    if let (Some(minimum), Some(number)) = (
        schema.get("minimum").and_then(serde_json::Value::as_u64),
        value.as_u64(),
    ) {
        if number < minimum {
            return Err(format!(
                "canonical invocation argument `{name}` is below its minimum"
            ));
        }
    }
    if let (Some(min_items), Some(items)) = (
        schema.get("minItems").and_then(serde_json::Value::as_u64),
        value.as_array(),
    ) {
        if items.len() < min_items as usize {
            return Err(format!(
                "canonical invocation argument `{name}` has too few items"
            ));
        }
    }
    if let (Some(items), Some(item_schema)) = (value.as_array(), schema.get("items")) {
        for item in items {
            validate_canonical_value(name, item, item_schema)?;
        }
    }
    if let (Some(object), Some(properties)) = (
        value.as_object(),
        schema
            .get("properties")
            .and_then(serde_json::Value::as_object),
    ) {
        if schema
            .get("additionalProperties")
            .is_some_and(|value| value == false)
        {
            if let Some(unknown) = object.keys().find(|field| !properties.contains_key(*field)) {
                return Err(format!(
                    "canonical invocation argument `{name}` has unknown field `{unknown}`"
                ));
            }
        }
        for required in schema
            .get("required")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
        {
            if !object.contains_key(required) {
                return Err(format!(
                    "canonical invocation argument `{name}` requires field `{required}`"
                ));
            }
        }
        for (field, nested) in object {
            if let Some(field_schema) = properties.get(field) {
                validate_canonical_value(field, nested, field_schema)?;
            }
        }
    }
    Ok(())
}

pub(super) struct V5CanonicalInvocationRuntime {
    service: Arc<dyn CanonicalInvocationService>,
    clock: Arc<dyn Clock>,
    documentation_cursors: Arc<unica_navigation::result_store::SearchCursorStore>,
    root_check_continuations:
        Arc<unica_project_health::inspection::resources::RootCheckContinuationStore>,
    /// Saved check results, shared with the read service: one retention
    /// budget covers node checks and the workspace verdict.
    check_snapshots: Arc<unica_navigation::result_store::ViewCursorStore>,
    /// Shared with the daemon's LSP service: an editor's folder and an MCP
    /// call on the same workspace reach one actor.
    workspace_actors: Arc<WorkspaceActorRegistry>,
    deliveries: Arc<unica_artifact_delivery::desk::DeliveryDesk>,
    provider_hosts: Arc<ProviderHostOwner>,
    runtime_resources: Arc<RuntimeResourceOwner>,
    #[cfg(test)]
    runtime_service: Option<Arc<RuntimeJobService>>,
}

/// The single product binding for the durable receipt owner's typed phases.
pub(crate) struct CanonicalInvocationHost {
    invocation_runtime: Arc<V5CanonicalInvocationRuntime>,
}

impl CanonicalInvocationHost {
    fn over(invocation_runtime: Arc<V5CanonicalInvocationRuntime>) -> Self {
        Self { invocation_runtime }
    }
}

impl unica_execution::receipt_runtime::ports::InvocationHost for CanonicalInvocationHost {
    // resource-limit: daemon.body-response-bytes
    // class: result
    // unit: bytes
    // scope: request
    // value: 73728
    // derived: true
    // resource: Наибольший размер тела прямого ответа канонического вызова, который поддерживает хост демона (наибольший бюджет ответа известных хостов).
    // status: unproven
    // origin: неизвестно
    // problem: неизвестно
    // on-reach: Отказ result_too_large: терминал вызова публикуется как неуспешный с причиной ResultTooLarge.
    // issue: #1119
    // review: #1119 PR9
    const MAX_SUPPORTED_BODY_RESPONSE_BYTES: usize = unica_bootstrap::MAX_RESPONSE_BUDGET;

    type Request = unica_daemon_protocol::protocol_v5::V5InvocationRequest;
    type Bound = V5ActorBoundCanonicalInvocation;
    type Prepared = V5PreparedCanonicalInvocation;

    fn has_retained_workspace_work(&self) -> bool {
        self.invocation_runtime.has_retained_workspace_work()
    }

    fn capture_response_deadline(&self, response_budget_ms: u64) -> InvocationResponseDeadline {
        self.invocation_runtime
            .capture_response_deadline(response_budget_ms)
    }

    fn bind(
        &self,
        invocation: Self::Request,
        response_deadline: InvocationResponseDeadline,
        admission_cancellation: &CancellationToken,
    ) -> Result<Self::Bound, V5CanonicalPrepareError> {
        let tool = match invocation.tool() {
            unica_execution::receipt_ledger::V5ToolIdentity::View => {
                unica_execution::invocation_store::ToolIdentity::View
            }
            unica_execution::receipt_ledger::V5ToolIdentity::Apply => {
                unica_execution::invocation_store::ToolIdentity::Apply
            }
            unica_execution::receipt_ledger::V5ToolIdentity::Resolve => {
                unica_execution::invocation_store::ToolIdentity::Resolve
            }
            unica_execution::receipt_ledger::V5ToolIdentity::Search => {
                unica_execution::invocation_store::ToolIdentity::Search
            }
            unica_execution::receipt_ledger::V5ToolIdentity::Check => {
                unica_execution::invocation_store::ToolIdentity::Check
            }
            unica_execution::receipt_ledger::V5ToolIdentity::Diff => {
                unica_execution::invocation_store::ToolIdentity::Diff
            }
            unica_execution::receipt_ledger::V5ToolIdentity::Run => {
                unica_execution::invocation_store::ToolIdentity::Run
            }
            unica_execution::receipt_ledger::V5ToolIdentity::Docs => {
                unica_execution::invocation_store::ToolIdentity::Docs
            }
        };
        let request = super::protocol::InvocationRequest::new(
            tool,
            serde_json::Value::Object(invocation.arguments().clone()),
            invocation.workspace_hint().to_owned(),
            invocation.response_budget_ms(),
        )
        .map(|request| request.with_workspace_origin(invocation.workspace_origin().clone()))
        .map_err(|error| {
            V5CanonicalPrepareError::Rejected(Box::new(
                unica_execution_contract::invocation::DomainResult::canonical_rejection(
                    None,
                    RefusalCode::BadValue,
                    error,
                ),
            ))
        })?;
        self.invocation_runtime.bind_with_deadline(
            request,
            response_deadline,
            admission_cancellation,
        )
    }

    fn discover_join_workspace_root(
        &self,
        workspace_hint: &str,
    ) -> Result<std::path::PathBuf, String> {
        discover_join_workspace_root(workspace_hint)
    }

    fn discover_join_source_set_paths(
        &self,
        root: &std::path::Path,
    ) -> Result<Vec<String>, String> {
        discover_join_source_set_paths(root)
    }
}

pub(crate) fn discover_join_workspace_root(
    workspace_hint: &str,
) -> Result<std::path::PathBuf, String> {
    crate::infrastructure::workspace::discover_workspace(Some(std::path::PathBuf::from(
        workspace_hint,
    )))
    .map(|context| context.workspace_root)
}

pub(crate) fn discover_join_source_set_paths(
    root: &std::path::Path,
) -> Result<Vec<String>, String> {
    crate::infrastructure::project_sources::discover_project_source_map(root).map(|map| {
        map.source_sets
            .into_iter()
            .map(|source| source.path)
            .collect()
    })
}

impl unica_execution::receipt_runtime::ports::BoundInvocation for V5ActorBoundCanonicalInvocation {
    type Prepared = V5PreparedCanonicalInvocation;
    fn response_deadline(&self) -> Option<InvocationResponseDeadline> {
        V5ActorBoundCanonicalInvocation::response_deadline(self)
    }
    fn workspace_identity_hash(&self) -> &unica_execution_contract::invocation::SafeIdentityHash {
        V5ActorBoundCanonicalInvocation::workspace_identity_hash(self)
    }
    fn prepare(self) -> Result<Self::Prepared, Box<DomainResult>> {
        V5ActorBoundCanonicalInvocation::prepare(self)
    }
}

impl unica_execution::receipt_runtime::ports::PreparedInvocation for V5PreparedCanonicalInvocation {
    fn execution_class(&self) -> &ExecutionClass {
        V5PreparedCanonicalInvocation::execution_class(self)
    }
    fn execute(self, cancellation: CancellationToken) -> Result<DomainResult, InvocationFailure> {
        V5PreparedCanonicalInvocation::execute(self, cancellation)
    }
}

#[derive(Clone)]
pub(super) struct RunEngineDelivery {
    workspace_hint: std::path::PathBuf,
    desk: Arc<unica_artifact_delivery::desk::DeliveryDesk>,
}

impl RunEngineDelivery {
    fn execute(
        &self,
        cancellation: CancellationToken,
        run: impl FnOnce(CancellationToken) -> DomainResult,
    ) -> DomainResult {
        use unica_artifact_delivery::shared::EngineDeliveryState;

        if cancellation.is_cancelled() {
            return DomainResult::canonical_rejection(
                None,
                RefusalCode::Cancelled,
                "run cancelled before engine delivery",
            );
        }
        let progress = unica_artifact_delivery::desk::CanonicalDeliveryProgress::default();
        let plugin_root =
            crate::infrastructure::plugin_runtime::find_plugin_root(&self.workspace_hint);
        let state = plugin_root
            .as_deref()
            .map(|plugin_root| {
                crate::infrastructure::engine_delivery::deliver_if_missing(
                    &self.desk,
                    plugin_root,
                    "v8-runner",
                    &cancellation,
                    &progress,
                )
            })
            .unwrap_or(EngineDeliveryState::NotRequired);
        if cancellation.is_cancelled() {
            return DomainResult::canonical_rejection(
                None,
                RefusalCode::Cancelled,
                "run cancelled while waiting for engine delivery",
            );
        }
        let delivery = match plugin_root.as_deref() {
            Some(plugin_root) => {
                crate::infrastructure::engine_delivery::canonical_engine_delivery_result(
                    state,
                    &progress,
                    plugin_root,
                    "v8-runner",
                )
            }
            None => {
                crate::infrastructure::engine_delivery::canonical_delivery_result(state, &progress)
            }
        };
        match delivery {
            Some(result) => result,
            None => {
                // #2124: the result never carries a secret of the project's
                // configuration, however the runner repeated it.
                let secrets = super::runner_secrets::project_secrets(&self.workspace_hint);
                super::runner_secrets::redact_result(run(cancellation), &secrets)
            }
        }
    }
}

pub(crate) struct V5ActorBoundCanonicalInvocation(ActorBoundCanonicalInvocationState);

enum ActorBoundCanonicalInvocationState {
    WorkspaceInspection {
        inspection: Arc<super::v13_workspace_bootstrap::PreparedWorkspaceInspection>,
    },
    Workspace {
        invocation: Box<ActorBoundInvocation>,
        service: Arc<dyn CanonicalInvocationService>,
    },
    Extensions {
        extensions: Arc<super::v13_extensions::PreparedExtensions>,
        workspace_identity_hash: unica_execution_contract::invocation::SafeIdentityHash,
        delivery: RunEngineDelivery,
    },
    InfobaseExport {
        export: Arc<super::v13_infobase_exports::PreparedInfobaseExport>,
        workspace_identity_hash: unica_execution_contract::invocation::SafeIdentityHash,
        delivery: RunEngineDelivery,
    },
    /// Терминальный запуск клиента: как выгрузка, готовится до admission
    /// PlatformXml и известен длинным по внешнему процессу.
    ClientRun {
        launch: Arc<super::v13_client_run::PreparedClientRun>,
        workspace_identity_hash: unica_execution_contract::invocation::SafeIdentityHash,
        delivery: RunEngineDelivery,
    },
    /// Создание базы: previewApply с забором, без аргументов, известно
    /// длинным по внешнему процессу.
    InfobaseCreate {
        create: Arc<super::v13_infobase_create::PreparedInfobaseCreate>,
        workspace_identity_hash: unica_execution_contract::invocation::SafeIdentityHash,
        delivery: RunEngineDelivery,
    },
    /// Импорт исходников в базу: previewApply с забором, известен длинным по
    /// внешнему процессу.
    SourceImport {
        import: Arc<super::v13_source_import::PreparedSourceImport>,
        workspace_identity_hash: unica_execution_contract::invocation::SafeIdentityHash,
        delivery: RunEngineDelivery,
    },
    /// Выгрузка базы в исходники: previewApply с забором, известна длинной
    /// по внешнему процессу.
    SourceExport {
        export: Arc<super::v13_source_export::PreparedSourceExport>,
        workspace_identity_hash: unica_execution_contract::invocation::SafeIdentityHash,
        delivery: RunEngineDelivery,
    },
    /// Сборка CF/CFE из исходников: previewApply с забором, известна длинной
    /// по внешнему процессу.
    ArtifactBuild {
        build: Arc<super::v13_artifact_build::PreparedArtifactBuild>,
        workspace_identity_hash: unica_execution_contract::invocation::SafeIdentityHash,
        delivery: RunEngineDelivery,
    },
    Documentation {
        search: Arc<super::v13_documentation::PreparedDocumentationSearch>,
        workspace_identity_hash: unica_execution_contract::invocation::SafeIdentityHash,
        response_deadline: InvocationResponseDeadline,
    },
}

pub(crate) struct V5PreparedCanonicalInvocation(PreparedCanonicalInvocationState);

enum PreparedCanonicalInvocationState {
    WorkspaceInspection {
        inspection: Arc<super::v13_workspace_bootstrap::PreparedWorkspaceInspection>,
    },
    Workspace {
        invocation: Box<ActorBoundInvocation>,
        class: ExecutionClass,
        service: Arc<dyn CanonicalInvocationService>,
    },
    Extensions {
        extensions: Arc<super::v13_extensions::PreparedExtensions>,
        delivery: RunEngineDelivery,
    },
    InfobaseExport {
        export: Arc<super::v13_infobase_exports::PreparedInfobaseExport>,
        delivery: RunEngineDelivery,
    },
    ClientRun {
        launch: Arc<super::v13_client_run::PreparedClientRun>,
        delivery: RunEngineDelivery,
    },
    InfobaseCreate {
        create: Arc<super::v13_infobase_create::PreparedInfobaseCreate>,
        delivery: RunEngineDelivery,
    },
    SourceImport {
        import: Arc<super::v13_source_import::PreparedSourceImport>,
        delivery: RunEngineDelivery,
    },
    SourceExport {
        export: Arc<super::v13_source_export::PreparedSourceExport>,
        delivery: RunEngineDelivery,
    },
    ArtifactBuild {
        build: Arc<super::v13_artifact_build::PreparedArtifactBuild>,
        delivery: RunEngineDelivery,
    },
    Documentation {
        search: Arc<super::v13_documentation::PreparedDocumentationSearch>,
    },
}

/// Only a test stand-in service keeps no check snapshots; the workspace
/// verdict then saves its pages in a store of its own. The product read
/// service always shares its store, which
/// `the_root_check_shares_the_read_service_check_snapshots` holds.
fn detached_check_snapshots() -> Arc<unica_navigation::result_store::ViewCursorStore> {
    Arc::new(super::v13_service::product_view_cursors())
}

impl V5CanonicalInvocationRuntime {
    #[cfg(test)]
    pub(super) fn new(service: Arc<dyn CanonicalInvocationService>, clock: Arc<dyn Clock>) -> Self {
        Self::with_workspace_actors(
            service,
            clock,
            crate::infrastructure::source_workspace::actor_registry(),
        )
    }

    /// Root checks continue from files in `store` instead of starting over.
    fn with_root_check_continuations(
        mut self,
        store: Arc<unica_project_health::inspection::resources::RootCheckContinuationStore>,
    ) -> Self {
        self.root_check_continuations = store;
        self
    }

    /// The workspace verdict saves its pages in `store` instead of the
    /// service's check snapshots: a test that bounds the store detaches the
    /// root check from the read service.
    #[cfg(test)]
    pub(crate) fn with_detached_check_snapshots(
        mut self,
        store: unica_navigation::result_store::ViewCursorStore,
    ) -> Self {
        self.check_snapshots = Arc::new(store);
        self
    }

    pub(super) fn has_retained_workspace_work(&self) -> bool {
        self.workspace_actors.has_retained_work()
    }

    #[cfg(test)]
    fn with_workspace_actors(
        service: Arc<dyn CanonicalInvocationService>,
        clock: Arc<dyn Clock>,
        workspace_actors: WorkspaceActorRegistry,
    ) -> Self {
        Self::with_shared_workspace_actors(service, clock, Arc::new(workspace_actors))
    }

    fn with_shared_workspace_actors(
        service: Arc<dyn CanonicalInvocationService>,
        clock: Arc<dyn Clock>,
        workspace_actors: Arc<WorkspaceActorRegistry>,
    ) -> Self {
        let check_snapshots = service
            .check_snapshots()
            .unwrap_or_else(detached_check_snapshots);
        Self {
            service,
            clock,
            documentation_cursors: Arc::new(
                unica_navigation::result_store::SearchCursorStore::default(),
            ),
            root_check_continuations: Arc::new(
                unica_project_health::inspection::resources::RootCheckContinuationStore::default(),
            ),
            check_snapshots,
            workspace_actors,
            deliveries: Arc::new(unica_artifact_delivery::desk::DeliveryDesk::default()),
            provider_hosts: Arc::new(ProviderHostOwner::default()),
            runtime_resources: Arc::new(RuntimeResourceOwner::default()),
            #[cfg(test)]
            runtime_service: None,
        }
    }

    /// The same runtime over a registry with a test policy: capacity or
    /// warm-set limits the production registry would never expose.
    #[cfg(test)]
    pub(super) fn with_workspace_actors_for_test(
        service: Arc<dyn CanonicalInvocationService>,
        clock: Arc<dyn Clock>,
        workspace_actors: WorkspaceActorRegistry,
    ) -> Self {
        Self::with_workspace_actors(service, clock, workspace_actors)
    }

    #[cfg(test)]
    pub(super) fn with_runtime_service_for_test(mut self, service: Arc<RuntimeJobService>) -> Self {
        self.runtime_service = Some(service);
        self
    }

    #[cfg(test)]
    pub(super) fn with_delivery_downloader_for_test(
        mut self,
        downloader: Arc<dyn unica_artifact_delivery::Downloader>,
    ) -> Self {
        self.deliveries = Arc::new(
            unica_artifact_delivery::desk::DeliveryDesk::with_downloader_for_test(downloader),
        );
        self
    }

    /// One fresh daemon-side deadline of this runtime's clock: what `bind`
    /// captures for every request, exposed to tests that bind by hand.
    #[cfg(test)]
    pub(super) fn capture_response_deadline_for_test(&self) -> InvocationResponseDeadline {
        InvocationResponseDeadline::capture(Arc::clone(&self.clock))
    }

    /// The one daemon-side deadline of a request: captured on this runtime's
    /// clock before strict validation and narrowed to the frontend budget.
    pub(super) fn capture_response_deadline(
        &self,
        response_budget_ms: u64,
    ) -> InvocationResponseDeadline {
        InvocationResponseDeadline::capture(Arc::clone(&self.clock))
            .restrict_to_frontend_budget(Duration::from_millis(response_budget_ms))
    }

    /// Bind under a deadline captured here and now: the direct path tests
    /// take, production captures at the reservation and binds with it.
    #[cfg(test)]
    pub(super) fn bind(
        &self,
        request: InvocationRequest,
    ) -> Result<V5ActorBoundCanonicalInvocation, V5CanonicalPrepareError> {
        let response_deadline = self.capture_response_deadline(request.response_budget_ms());
        self.bind_with_deadline(request, response_deadline, &CancellationToken::new())
    }

    /// Binds under a deadline the caller captured earlier: the same clock and
    /// the same handoff moment every later stage measures against.
    pub(super) fn bind_with_deadline(
        &self,
        mut request: InvocationRequest,
        response_deadline: InvocationResponseDeadline,
        admission_cancellation: &CancellationToken,
    ) -> Result<V5ActorBoundCanonicalInvocation, V5CanonicalPrepareError> {
        request.normalize_omitted_options();
        if let Err(summary) = validate_hidden_v13_request(&request) {
            return Err(V5CanonicalPrepareError::Rejected(Box::new(
                DomainResult::canonical_rejection(None, RefusalCode::BadValue, summary),
            )));
        }
        if let Some(refusal) = operation_address_outside_view(&request) {
            return Err(V5CanonicalPrepareError::Rejected(Box::new(refusal)));
        }
        match super::v13_workspace_bootstrap::prepare(
            &request,
            response_deadline.clone(),
            &self.root_check_continuations,
            &self.check_snapshots,
        ) {
            super::v13_workspace_bootstrap::Preparation::NotApplicable => {}
            super::v13_workspace_bootstrap::Preparation::Rejected(result) => {
                return Err(V5CanonicalPrepareError::Rejected(result))
            }
            super::v13_workspace_bootstrap::Preparation::Ready(inspection) => {
                return Ok(V5ActorBoundCanonicalInvocation(
                    ActorBoundCanonicalInvocationState::WorkspaceInspection { inspection },
                ));
            }
        }
        if let Some(result) = super::v13_run_dictionary::execute_run_dictionary(&request) {
            return Err(V5CanonicalPrepareError::Direct(Box::new(result)));
        }
        let run_delivery = RunEngineDelivery {
            workspace_hint: std::path::PathBuf::from(request.workspace_hint()),
            desk: Arc::clone(&self.deliveries),
        };
        match super::v13_extensions::prepare(&request) {
            super::v13_extensions::Preparation::NotApplicable => {}
            super::v13_extensions::Preparation::Rejected(result) => {
                return Err(V5CanonicalPrepareError::Rejected(result))
            }
            super::v13_extensions::Preparation::Ready(extensions) => {
                let workspace_identity_hash = extensions.workspace_identity_hash();
                return Ok(V5ActorBoundCanonicalInvocation(
                    ActorBoundCanonicalInvocationState::Extensions {
                        extensions,
                        workspace_identity_hash,
                        delivery: run_delivery.clone(),
                    },
                ));
            }
        }
        match super::v13_infobase_exports::prepare(&request) {
            super::v13_infobase_exports::Preparation::NotApplicable => {}
            super::v13_infobase_exports::Preparation::Rejected(result) => {
                return Err(V5CanonicalPrepareError::Rejected(result))
            }
            super::v13_infobase_exports::Preparation::Ready(export) => {
                let workspace_identity_hash = export.workspace_identity_hash();
                return Ok(V5ActorBoundCanonicalInvocation(
                    ActorBoundCanonicalInvocationState::InfobaseExport {
                        export,
                        workspace_identity_hash,
                        delivery: run_delivery.clone(),
                    },
                ));
            }
        }
        match super::v13_client_run::prepare(&request) {
            super::v13_client_run::Preparation::NotApplicable => {}
            super::v13_client_run::Preparation::Rejected(result) => {
                return Err(V5CanonicalPrepareError::Rejected(result))
            }
            super::v13_client_run::Preparation::Ready(launch) => {
                let workspace_identity_hash = launch.workspace_identity_hash();
                return Ok(V5ActorBoundCanonicalInvocation(
                    ActorBoundCanonicalInvocationState::ClientRun {
                        launch,
                        workspace_identity_hash,
                        delivery: run_delivery.clone(),
                    },
                ));
            }
        }
        match super::v13_infobase_create::prepare(&request) {
            super::v13_infobase_create::Preparation::NotApplicable => {}
            super::v13_infobase_create::Preparation::Rejected(result) => {
                return Err(V5CanonicalPrepareError::Rejected(result))
            }
            super::v13_infobase_create::Preparation::Ready(create) => {
                let workspace_identity_hash = create.workspace_identity_hash();
                return Ok(V5ActorBoundCanonicalInvocation(
                    ActorBoundCanonicalInvocationState::InfobaseCreate {
                        create,
                        workspace_identity_hash,
                        delivery: run_delivery.clone(),
                    },
                ));
            }
        }
        match super::v13_source_import::prepare(&request) {
            super::v13_source_import::Preparation::NotApplicable => {}
            super::v13_source_import::Preparation::Rejected(result) => {
                return Err(V5CanonicalPrepareError::Rejected(result))
            }
            super::v13_source_import::Preparation::Ready(import) => {
                let workspace_identity_hash = import.workspace_identity_hash();
                return Ok(V5ActorBoundCanonicalInvocation(
                    ActorBoundCanonicalInvocationState::SourceImport {
                        import,
                        workspace_identity_hash,
                        delivery: run_delivery.clone(),
                    },
                ));
            }
        }
        match super::v13_source_export::prepare(&request) {
            super::v13_source_export::Preparation::NotApplicable => {}
            super::v13_source_export::Preparation::Rejected(result) => {
                return Err(V5CanonicalPrepareError::Rejected(result))
            }
            super::v13_source_export::Preparation::Ready(export) => {
                let workspace_identity_hash = export.workspace_identity_hash();
                return Ok(V5ActorBoundCanonicalInvocation(
                    ActorBoundCanonicalInvocationState::SourceExport {
                        export,
                        workspace_identity_hash,
                        delivery: run_delivery.clone(),
                    },
                ));
            }
        }
        match super::v13_artifact_build::prepare(&request) {
            super::v13_artifact_build::Preparation::NotApplicable => {}
            super::v13_artifact_build::Preparation::Rejected(result) => {
                return Err(V5CanonicalPrepareError::Rejected(result))
            }
            super::v13_artifact_build::Preparation::Ready(build) => {
                let workspace_identity_hash = build.workspace_identity_hash();
                return Ok(V5ActorBoundCanonicalInvocation(
                    ActorBoundCanonicalInvocationState::ArtifactBuild {
                        build,
                        workspace_identity_hash,
                        delivery: run_delivery.clone(),
                    },
                ));
            }
        }
        match super::v13_documentation::prepare(&request, Arc::clone(&self.documentation_cursors)) {
            super::v13_documentation::Preparation::NotApplicable => {}
            super::v13_documentation::Preparation::Rejected(result) => {
                return Err(V5CanonicalPrepareError::Rejected(result))
            }
            super::v13_documentation::Preparation::Ready(search) => {
                let workspace_identity_hash = search.workspace_identity_hash();
                return Ok(V5ActorBoundCanonicalInvocation(
                    ActorBoundCanonicalInvocationState::Documentation {
                        search,
                        workspace_identity_hash,
                        response_deadline,
                    },
                ));
            }
        }
        #[cfg(test)]
        let runtime_service = self.runtime_service.clone();
        #[cfg(not(test))]
        let runtime_service = None;
        let invocation = bind_workspace_invocation_cancellable(
            &request,
            &self.workspace_actors,
            ActorInvocationResources::new(
                Arc::clone(&self.deliveries),
                Arc::clone(&self.provider_hosts),
                Arc::clone(&self.runtime_resources),
                runtime_service,
            ),
            response_deadline,
            admission_cancellation,
        )
        .map_err(|error| reject_workspace_admission(&request, error))?;
        Ok(V5ActorBoundCanonicalInvocation(
            ActorBoundCanonicalInvocationState::Workspace {
                invocation: Box::new(invocation),
                service: Arc::clone(&self.service),
            },
        ))
    }
}

impl V5ActorBoundCanonicalInvocation {
    /// The monotonic cutoff capability captured at bind: the daemon-side
    /// handoff moment of a workspace invocation. Infobase exports carry none;
    /// they are known-long by construction.
    pub(super) fn response_deadline(&self) -> Option<InvocationResponseDeadline> {
        match &self.0 {
            ActorBoundCanonicalInvocationState::WorkspaceInspection { inspection, .. } => {
                Some(inspection.response_deadline().clone())
            }
            ActorBoundCanonicalInvocationState::Workspace { invocation, .. } => {
                Some(invocation.response_deadline().clone())
            }
            ActorBoundCanonicalInvocationState::Extensions { .. }
            | ActorBoundCanonicalInvocationState::InfobaseExport { .. }
            | ActorBoundCanonicalInvocationState::ClientRun { .. }
            | ActorBoundCanonicalInvocationState::InfobaseCreate { .. }
            | ActorBoundCanonicalInvocationState::SourceImport { .. }
            | ActorBoundCanonicalInvocationState::SourceExport { .. }
            | ActorBoundCanonicalInvocationState::ArtifactBuild { .. } => None,
            // Справка не известна длинной: локальное попадание отвечает
            // сразу, сетевое доходит до седьмой секунды. Значит у неё тот же
            // cutoff, что у чтений, — захваченный при bind.
            ActorBoundCanonicalInvocationState::Documentation {
                response_deadline, ..
            } => Some(response_deadline.clone()),
        }
    }

    pub(super) fn workspace_identity_hash(
        &self,
    ) -> &unica_execution_contract::invocation::SafeIdentityHash {
        match &self.0 {
            ActorBoundCanonicalInvocationState::Workspace { invocation, .. } => {
                invocation.workspace_identity_hash()
            }
            ActorBoundCanonicalInvocationState::WorkspaceInspection { inspection } => {
                inspection.workspace_identity_hash()
            }
            ActorBoundCanonicalInvocationState::Extensions {
                workspace_identity_hash,
                ..
            }
            | ActorBoundCanonicalInvocationState::InfobaseExport {
                workspace_identity_hash,
                ..
            }
            | ActorBoundCanonicalInvocationState::ClientRun {
                workspace_identity_hash,
                ..
            }
            | ActorBoundCanonicalInvocationState::InfobaseCreate {
                workspace_identity_hash,
                ..
            }
            | ActorBoundCanonicalInvocationState::SourceImport {
                workspace_identity_hash,
                ..
            }
            | ActorBoundCanonicalInvocationState::SourceExport {
                workspace_identity_hash,
                ..
            }
            | ActorBoundCanonicalInvocationState::ArtifactBuild {
                workspace_identity_hash,
                ..
            }
            | ActorBoundCanonicalInvocationState::Documentation {
                workspace_identity_hash,
                ..
            } => workspace_identity_hash,
        }
    }

    pub(super) fn prepare(self) -> Result<V5PreparedCanonicalInvocation, Box<DomainResult>> {
        match self.0 {
            ActorBoundCanonicalInvocationState::WorkspaceInspection { inspection, .. } => {
                Ok(V5PreparedCanonicalInvocation(
                    PreparedCanonicalInvocationState::WorkspaceInspection { inspection },
                ))
            }
            ActorBoundCanonicalInvocationState::Workspace {
                invocation,
                service,
            } => {
                let class = service.prepare(&invocation)?;
                Ok(V5PreparedCanonicalInvocation(
                    PreparedCanonicalInvocationState::Workspace {
                        invocation,
                        class,
                        service,
                    },
                ))
            }
            ActorBoundCanonicalInvocationState::Extensions {
                extensions,
                delivery,
                ..
            } => Ok(V5PreparedCanonicalInvocation(
                PreparedCanonicalInvocationState::Extensions {
                    extensions,
                    delivery,
                },
            )),
            ActorBoundCanonicalInvocationState::InfobaseExport {
                export, delivery, ..
            } => Ok(V5PreparedCanonicalInvocation(
                PreparedCanonicalInvocationState::InfobaseExport { export, delivery },
            )),
            ActorBoundCanonicalInvocationState::ClientRun {
                launch, delivery, ..
            } => Ok(V5PreparedCanonicalInvocation(
                PreparedCanonicalInvocationState::ClientRun { launch, delivery },
            )),
            ActorBoundCanonicalInvocationState::InfobaseCreate {
                create, delivery, ..
            } => Ok(V5PreparedCanonicalInvocation(
                PreparedCanonicalInvocationState::InfobaseCreate { create, delivery },
            )),
            ActorBoundCanonicalInvocationState::SourceImport {
                import, delivery, ..
            } => Ok(V5PreparedCanonicalInvocation(
                PreparedCanonicalInvocationState::SourceImport { import, delivery },
            )),
            ActorBoundCanonicalInvocationState::SourceExport {
                export, delivery, ..
            } => Ok(V5PreparedCanonicalInvocation(
                PreparedCanonicalInvocationState::SourceExport { export, delivery },
            )),
            ActorBoundCanonicalInvocationState::ArtifactBuild {
                build, delivery, ..
            } => Ok(V5PreparedCanonicalInvocation(
                PreparedCanonicalInvocationState::ArtifactBuild { build, delivery },
            )),
            ActorBoundCanonicalInvocationState::Documentation { search, .. } => {
                Ok(V5PreparedCanonicalInvocation(
                    PreparedCanonicalInvocationState::Documentation { search },
                ))
            }
        }
    }
}

/// Only a binding that left this workspace actor is a changed workspace; an
/// invariant failure at the execution boundary stays internal (#1338). The
/// boundary's text never leaves the daemon.
fn boundary_failure(error: invocation_service::ExecutionBoundaryError) -> InvocationFailure {
    match error {
        invocation_service::ExecutionBoundaryError::BindingChanged(_) => InvocationFailure::new(
            unica_execution_contract::invocation::WORKSPACE_CHANGED_FAILURE,
            "workspace actor binding changed during execution",
        ),
        invocation_service::ExecutionBoundaryError::Internal(_) => InvocationFailure::new(
            "execution_boundary_failed",
            "actor-bound execution boundary failed",
        ),
    }
}

impl V5PreparedCanonicalInvocation {
    pub(super) const fn execution_class(&self) -> &ExecutionClass {
        const INFOBASE_EXPORT_CLASS: ExecutionClass =
            ExecutionClass::KnownLong(KnownLongReason::ExternalProcess);
        match &self.0 {
            PreparedCanonicalInvocationState::Workspace { class, .. } => class,
            PreparedCanonicalInvocationState::Extensions { .. }
            | PreparedCanonicalInvocationState::InfobaseExport { .. }
            | PreparedCanonicalInvocationState::ClientRun { .. }
            | PreparedCanonicalInvocationState::InfobaseCreate { .. }
            | PreparedCanonicalInvocationState::SourceImport { .. }
            | PreparedCanonicalInvocationState::SourceExport { .. }
            | PreparedCanonicalInvocationState::ArtifactBuild { .. } => &INFOBASE_EXPORT_CLASS,
            PreparedCanonicalInvocationState::WorkspaceInspection { .. }
            | PreparedCanonicalInvocationState::Documentation { .. } => {
                &ExecutionClass::InlineCandidate
            }
        }
    }

    pub(super) fn execute(
        self,
        cancellation: CancellationToken,
    ) -> Result<DomainResult, InvocationFailure> {
        match self.0 {
            PreparedCanonicalInvocationState::WorkspaceInspection { inspection } => {
                inspection.execute(cancellation)
            }
            PreparedCanonicalInvocationState::Workspace {
                invocation,
                service,
                ..
            } => {
                let execution = invocation
                    .begin_execution(&cancellation)
                    .map_err(boundary_failure)?;
                let outcome = service.execute(&execution, cancellation.clone());
                execution
                    .publish(outcome, &cancellation)
                    .map_err(boundary_failure)?
            }
            PreparedCanonicalInvocationState::Extensions {
                extensions,
                delivery,
            } => Ok(delivery.execute(cancellation, |cancellation| {
                extensions.execute(cancellation)
            })),
            PreparedCanonicalInvocationState::InfobaseExport { export, delivery } => {
                Ok(delivery.execute(cancellation, |cancellation| export.execute(cancellation)))
            }
            PreparedCanonicalInvocationState::ClientRun { launch, delivery } => {
                Ok(delivery.execute(cancellation, |cancellation| launch.execute(cancellation)))
            }
            PreparedCanonicalInvocationState::InfobaseCreate { create, delivery } => {
                Ok(delivery.execute(cancellation, |cancellation| create.execute(cancellation)))
            }
            PreparedCanonicalInvocationState::SourceImport { import, delivery } => {
                Ok(delivery.execute(cancellation, |cancellation| import.execute(cancellation)))
            }
            PreparedCanonicalInvocationState::SourceExport { export, delivery } => {
                Ok(delivery.execute(cancellation, |cancellation| export.execute(cancellation)))
            }
            PreparedCanonicalInvocationState::ArtifactBuild { build, delivery } => {
                Ok(delivery.execute(cancellation, |cancellation| build.execute(cancellation)))
            }
            PreparedCanonicalInvocationState::Documentation { search } => {
                Ok(search.execute(cancellation))
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct DaemonServerConfig {
    pub(crate) state_root: std::path::PathBuf,
    pub(crate) core_identity: CoreIdentity,
    pub(crate) idle_grace: Duration,
    pub(crate) capacity_observer:
        Arc<crate::infrastructure::capacity_observation::CapacityObserver>,
    invocation_service: Arc<dyn CanonicalInvocationService>,
    /// The workspace actors of the daemon, shared by MCP and LSP.
    workspace_actors: Arc<crate::infrastructure::source_workspace::WorkspaceActorRegistry>,
    /// The LSP service behind the daemon's LSP lane.
    lsp_host: Arc<dyn unica_lsp_contract::host::LspHost>,
    /// Overrides installed by tests and the contract harness. Production
    /// leaves every one of them empty; the fields exist unconditionally so
    /// the runtime reads one configuration whatever the build.
    invocation_clock: Option<Arc<dyn Clock>>,
    v5_epoch_clock: Option<Arc<dyn unica_execution::invocation_store::EpochMillisClock>>,
    skip_v5_startup_reconciliation: bool,
    runtime_hooks: Option<
        Arc<dyn unica_execution::receipt_runtime::hooks::V5RuntimeHooks<CanonicalInvocationHost>>,
    >,
    #[cfg(test)]
    canonical_runtime: Option<Arc<V5CanonicalInvocationRuntime>>,
}

impl DaemonServerConfig {
    pub(crate) fn new(
        state_root: std::path::PathBuf,
        core_identity: CoreIdentity,
        idle_grace: Duration,
    ) -> Self {
        let capacity_observer =
            Arc::new(crate::infrastructure::capacity_observation::CapacityObserver::default());
        let read_service = Arc::new(
            crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::with_capacity_observer(
                Arc::clone(&capacity_observer),
            ),
        );
        let invocation_service: Arc<dyn CanonicalInvocationService> = read_service.clone();
        let workspace_actors = Arc::new(crate::infrastructure::source_workspace::actor_registry());
        // The editor's door reaches the same actors and the same services as
        // the agent's calls.
        let lsp_host = Arc::new(unica_lsp::UnicaLspHost::new(Arc::new(
            super::lsp_workspaces::ProductLspWorkspaces::new(
                Arc::clone(&workspace_actors),
                read_service,
            ),
        )));
        Self {
            state_root,
            core_identity,
            idle_grace,
            capacity_observer,
            invocation_service,
            workspace_actors,
            lsp_host,
            invocation_clock: None,
            v5_epoch_clock: None,
            skip_v5_startup_reconciliation: false,
            runtime_hooks: None,
            #[cfg(test)]
            canonical_runtime: None,
        }
    }

    pub(super) fn invocation_service_for_v5(&self) -> Arc<dyn CanonicalInvocationService> {
        Arc::clone(&self.invocation_service)
    }

    /// A canonical runtime the test already holds: the live daemon binds
    /// through it, and the test observes its actor and capability registries.
    #[cfg(test)]
    pub(super) fn with_canonical_runtime_for_test(
        mut self,
        runtime: Arc<V5CanonicalInvocationRuntime>,
    ) -> Self {
        self.canonical_runtime = Some(runtime);
        self
    }

    #[cfg(test)]
    pub(super) fn with_lsp_host_for_test(
        mut self,
        host: Arc<dyn unica_lsp_contract::host::LspHost>,
    ) -> Self {
        self.lsp_host = host;
        self
    }

    #[cfg(test)]
    pub(super) fn canonical_runtime_for_v5(&self) -> Option<Arc<V5CanonicalInvocationRuntime>> {
        self.canonical_runtime.clone()
    }

    pub(super) fn invocation_clock_for_v5(&self) -> Arc<dyn Clock> {
        match &self.invocation_clock {
            Some(clock) => Arc::clone(clock),
            None => Arc::new(TokioClock),
        }
    }

    pub(super) fn epoch_clock_for_v5(
        &self,
    ) -> Option<Arc<dyn unica_execution::invocation_store::EpochMillisClock>> {
        self.v5_epoch_clock.clone()
    }

    pub(super) const fn skips_v5_startup_reconciliation(&self) -> bool {
        self.skip_v5_startup_reconciliation
    }

    pub(super) fn runtime_hooks_for_v5(
        &self,
    ) -> Option<
        Arc<dyn unica_execution::receipt_runtime::hooks::V5RuntimeHooks<CanonicalInvocationHost>>,
    > {
        self.runtime_hooks.clone()
    }

    #[cfg(any(test, feature = "receipt-ledger-test-support"))]
    pub(crate) fn with_runtime_hooks_for_test(
        mut self,
        hooks: Arc<
            dyn unica_execution::receipt_runtime::hooks::V5RuntimeHooks<CanonicalInvocationHost>,
        >,
    ) -> Self {
        self.runtime_hooks = Some(hooks);
        self
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    pub(crate) fn without_v5_startup_reconciliation_for_test(mut self) -> Self {
        self.skip_v5_startup_reconciliation = true;
        self
    }

    #[cfg(any(test, feature = "receipt-ledger-test-support"))]
    pub(crate) fn with_invocation_service(
        mut self,
        service: Arc<dyn CanonicalInvocationService>,
    ) -> Self {
        self.invocation_service = service;
        self
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    pub(crate) fn with_invocation_clock_for_test(mut self, clock: Arc<dyn Clock>) -> Self {
        self.invocation_clock = Some(clock);
        self
    }

    #[cfg(any(test, feature = "receipt-ledger-test-support"))]
    pub(crate) fn with_v5_epoch_clock_for_test(
        mut self,
        clock: Arc<dyn unica_execution::invocation_store::EpochMillisClock>,
    ) -> Self {
        self.v5_epoch_clock = Some(clock);
        self
    }
}

/// The private child of the daemon state that holds root check continuations.
const ROOT_CHECK_CONTINUATIONS: &str = "project-health";

impl DaemonServerConfig {
    pub(crate) fn receipt_runtime_config(
        &self,
    ) -> unica_execution::receipt_runtime::ports::ReceiptRuntimeConfig<CanonicalInvocationHost>
    {
        let epoch_clock = self
            .epoch_clock_for_v5()
            .unwrap_or_else(|| Arc::new(unica_execution::invocation_store::SystemEpochMillisClock));
        let hooks = self
            .runtime_hooks_for_v5()
            .unwrap_or_else(|| Arc::new(unica_execution::receipt_runtime::hooks::NoHooks));
        let source = self.clone();
        // Continuations live in a private directory of the daemon's own
        // state, never in a workspace, and age out with the same idle period
        // as stale daemon state. Without that directory a root check starts
        // from the beginning each time.
        let root_check_continuations = Arc::new(
            match unica_daemon_protocol::identity::DaemonStateDirectory::open(
                &self.state_root,
                &self.core_identity,
            )
            .and_then(|state| {
                state.create_private_subdirectory(ROOT_CHECK_CONTINUATIONS)?;
                Ok(state.path().join(ROOT_CHECK_CONTINUATIONS))
            }) {
                Ok(directory) => {
                    unica_project_health::inspection::resources::RootCheckContinuationStore::in_directory(
                        directory,
                        unica_daemon_protocol::state_collection::STALE_DAEMON_STATE_IDLE,
                    )
                }
                Err(_) => {
                    eprintln!("root check continuation disabled: daemon state unavailable");
                    Default::default()
                }
            },
        );
        let factory: Arc<dyn Fn() -> Arc<CanonicalInvocationHost> + Send + Sync> =
            Arc::new(move || {
                #[cfg(test)]
                let preset = source.canonical_runtime_for_v5();
                #[cfg(not(test))]
                let preset: Option<Arc<V5CanonicalInvocationRuntime>> = None;
                let runtime = match preset {
                    Some(runtime) => runtime,
                    None => Arc::new(
                        V5CanonicalInvocationRuntime::with_shared_workspace_actors(
                            source.invocation_service_for_v5(),
                            source.invocation_clock_for_v5(),
                            Arc::clone(&source.workspace_actors),
                        )
                        .with_root_check_continuations(Arc::clone(&root_check_continuations)),
                    ),
                };
                Arc::new(CanonicalInvocationHost::over(runtime))
            });
        unica_execution::receipt_runtime::ports::ReceiptRuntimeConfig::new(
            self.core_identity.digest().clone(),
            factory,
            epoch_clock,
            hooks,
            !self.skips_v5_startup_reconciliation(),
        )
    }

    pub(crate) fn into_transport_config(
        self,
    ) -> unica_daemon_protocol::transport::DaemonServerConfig<
        CanonicalInvocationHost,
        CapacityObservation,
    > {
        let runtime = self.receipt_runtime_config();
        unica_daemon_protocol::transport::DaemonServerConfig {
            state_root: self.state_root,
            core_identity: self.core_identity,
            idle_grace: self.idle_grace,
            runtime,
            observation: CapacityObservation(self.capacity_observer),
            lsp: self.lsp_host,
        }
    }
}

pub(crate) struct CapacityObservation(
    Arc<crate::infrastructure::capacity_observation::CapacityObserver>,
);

impl unica_daemon_protocol::transport::ObservationLifecycle for CapacityObservation {
    type Guard = crate::infrastructure::capacity_observation::CapacityWriterGuard;
    fn start(
        &self,
        state: &unica_daemon_protocol::identity::DaemonStateDirectory,
        authority_valid: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> std::io::Result<Self::Guard> {
        crate::infrastructure::capacity_observation::start_background_writer(
            state,
            Arc::clone(&self.0),
            authority_valid,
        )
    }
}

#[cfg(test)]
#[path = "code_target_tests.rs"]
mod code_target_tests;
#[cfg(test)]
#[path = "form_lifecycle_tests.rs"]
mod form_lifecycle_tests;
#[cfg(test)]
#[path = "form_move_tests.rs"]
mod form_move_tests;
#[cfg(test)]
#[path = "form_native_profile_tests.rs"]
mod form_native_profile_tests;
#[cfg(test)]
#[path = "form_properties_tests.rs"]
mod form_properties_tests;
#[cfg(test)]
#[path = "server/external_cache_apply_tests.rs"]
mod medium_external_cache_apply_tests;
#[cfg(test)]
#[path = "metadata_can_tests.rs"]
mod metadata_can_tests;
#[cfg(test)]
mod retained_directory_removal_tests;
#[cfg(test)]
#[path = "xdto_can_tests.rs"]
mod xdto_can_tests;

#[cfg(test)]
mod client_interface_tests;
#[cfg(test)]
mod html_template_tests;
#[cfg(test)]
mod new_xml_tests;

#[cfg(test)]
mod template_new_xml_tests;

#[cfg(test)]
mod form_add_owner_bytes_tests;

#[cfg(test)]
mod legacy_inline_check_tests;

#[cfg(test)]
mod apply_answer_size_tests;

#[cfg(test)]
#[path = "server/stored_secret_tests.rs"]
mod medium_stored_secret_tests;

// Shared helpers keep their former name for the modules that import them.
#[cfg(test)]
pub(crate) use medium_actor_capacity_tests as actor_capacity_tests;
#[cfg(test)]
pub(crate) mod medium_actor_capacity_tests;
