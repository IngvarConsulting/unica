use unica_daemon::identity::CoreIdentity;
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

/// `filter` снят с `view` и `diff` (#1374): предмет вопроса задаёт адрес.
/// Это всё ещё отказ неизвестного аргумента, но он называет, куда ушло то,
/// что фильтр делал: словарь — в каждом ответе узла, контракт операции —
/// по адресу `<узел>.Can.<op>`, сравнение части — по адресу этой части.
fn retired_read_filter(request: &InvocationRequest) -> Option<DomainResult> {
    use unica_execution::invocation_store::ToolIdentity;
    let arguments = request.arguments();
    let filter = arguments.get("filter")?;
    let text = |name: &str| {
        arguments
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    };
    let mut next = Vec::new();
    let summary = match request.tool() {
        ToolIdentity::View => {
            if let Some(at) = text("at") {
                if let Some(op) = filter
                    .get("can")
                    .and_then(|can| can.get("op"))
                    .and_then(serde_json::Value::as_str)
                    .filter(|op| !op.trim().is_empty())
                {
                    next.push(serde_json::json!({
                        "tool": "unica.view",
                        "args": {"at": format!("{at}.Can.{op}")},
                        "reason": "контракт аргументов одной операции",
                    }));
                }
                next.push(serde_json::json!({
                    "tool": "unica.view",
                    "args": {"at": at},
                    "reason": "ответ узла несёт его словарь операций can",
                }));
            } else {
                next.push(serde_json::json!({
                    "tool": "unica.view",
                    "args": {},
                    "reason": "рабочее пространство и его наборы исходников",
                }));
            }
            "canonical invocation has unknown argument `filter`: unica.view answers every node with its operation dictionary can; read one operation's argument contract at `<node>.Can.<op>`, and page collections with limit"
        }
        ToolIdentity::Diff => {
            if let (Some(left), Some(right)) = (text("left"), text("right")) {
                next.push(serde_json::json!({
                    "tool": "unica.diff",
                    "args": {"left": left, "right": right},
                    "reason": "сравнение сужается адресом сравниваемых узлов",
                }));
            }
            "canonical invocation has unknown argument `filter`: unica.diff narrows a comparison by address; compare the member nodes themselves"
        }
        _ => return None,
    };
    let mut result = DomainResult::canonical_rejection(None, RefusalCode::BadValue, summary);
    result.next = next;
    Some(result)
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
    workspace_actors: WorkspaceActorRegistry,
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
    const MAX_SUPPORTED_BODY_RESPONSE_BYTES: usize = unica_bootstrap::MAX_RESPONSE_BUDGET;

    type Request = unica_daemon::protocol_v5::V5InvocationRequest;
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

    fn with_workspace_actors(
        service: Arc<dyn CanonicalInvocationService>,
        clock: Arc<dyn Clock>,
        workspace_actors: WorkspaceActorRegistry,
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
        if let Some(refusal) = retired_read_filter(&request) {
            return Err(V5CanonicalPrepareError::Rejected(Box::new(refusal)));
        }
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
    /// The LSP service behind the daemon's LSP lane. It declares no
    /// capability until a product service is composed here (#2171).
    lsp_host: Arc<dyn unica_daemon::lsp_lane::LspHost>,
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
        let invocation_service: Arc<dyn CanonicalInvocationService> = Arc::new(
            crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::with_capacity_observer(
                Arc::clone(&capacity_observer),
            ),
        );
        Self {
            state_root,
            core_identity,
            idle_grace,
            capacity_observer,
            invocation_service,
            lsp_host: Arc::new(unica_daemon::lsp_lane::NoLspHost),
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
        host: Arc<dyn unica_daemon::lsp_lane::LspHost>,
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
            match unica_daemon::identity::DaemonStateDirectory::open(
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
                        unica_daemon::state_collection::STALE_DAEMON_STATE_IDLE,
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
                        V5CanonicalInvocationRuntime::new(
                            source.invocation_service_for_v5(),
                            source.invocation_clock_for_v5(),
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
    ) -> unica_daemon::transport::DaemonServerConfig<CanonicalInvocationHost, CapacityObservation>
    {
        let runtime = self.receipt_runtime_config();
        unica_daemon::transport::DaemonServerConfig {
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

impl unica_daemon::transport::ObservationLifecycle for CapacityObservation {
    type Guard = crate::infrastructure::capacity_observation::CapacityWriterGuard;
    fn start(
        &self,
        state: &unica_daemon::identity::DaemonStateDirectory,
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
#[path = "apply_large_input_tests.rs"]
mod apply_large_input_tests;
#[cfg(test)]
#[path = "code_target_tests.rs"]
mod code_target_tests;
#[cfg(test)]
mod external_cache_apply_tests;
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
mod stored_secret_tests;

#[cfg(test)]
pub(crate) mod actor_capacity_tests {
    use super::super::runner_015::VERSION as RUNNER_VERSION;
    use super::*;
    use unica_analysis_services::provider_host::ProviderHostKey;
    use unica_artifact_delivery::shared::{ArtifactReady, DeliveryFormIdentity, DeliveryWorkKey};
    use unica_daemon::client_v5::V5DaemonProcessOwner;
    use unica_daemon::identity::DaemonStateDirectory;
    use unica_daemon::protocol_v5::{V5InvocationRequest, V5ServerResponse};
    use unica_execution::invocation::INVOCATION_HANDOFF_WINDOW;
    use unica_execution::invocation_store::ToolIdentity;
    use unica_execution::receipt_ledger::{ReceiptTerminalOutcome, V5ToolIdentity};
    use unica_execution::receipt_runtime::ports::KnownLongReason;
    use unica_execution::receipt_runtime::response::{V5DaemonTaskSnapshot, V5InvocationResponse};
    use unica_source_contract::{SourceFormat, SourceProfile, SourceSetKind};

    use crate::infrastructure::source_workspace::WorkspaceActor;
    use crate::infrastructure::workspace::discover_workspace;
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{mpsc, Barrier, Condvar, Mutex};
    use std::thread;
    use std::time::Instant;
    use unica_adapter_v8_runner::request::RuntimeJobRequest;
    use unica_execution_contract::invocation::InvocationStatus;
    use unica_runner::operation::RuntimeJobOperation;
    use unica_work_control::provider_deadline::ProviderDeadline;
    use unica_workspace::actor::{IndexWorkIdentity, WorkspaceSourceSetInput};
    use unica_workspace::test_support::retained_read_probe::{RetainedRead, RetainedReadProbe};

    thread_local! {
        static LOGICAL_READ_NOW: RefCell<Option<Instant>> = const { RefCell::new(None) };
    }

    fn logical_read_now() -> Instant {
        LOGICAL_READ_NOW.with(|now| now.borrow().expect("logical read clock is initialized"))
    }

    fn set_logical_read_now(now: Instant) {
        LOGICAL_READ_NOW.with(|current| *current.borrow_mut() = Some(now));
    }

    pub(crate) fn canonical_v13_service() -> Arc<dyn CanonicalInvocationService> {
        Arc::new(crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default())
    }

    pub(crate) fn subsystem_picture_workspace(picture: &str, has_ext: bool) -> tempfile::TempDir {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        std::fs::create_dir_all(source.join("Subsystems")).unwrap();
        if has_ext {
            std::fs::create_dir_all(source.join("Subsystems/Sales/Ext")).unwrap();
        }
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects><Subsystem>Sales</Subsystem></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Subsystems/Sales.xml"),
            format!(
                "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                <MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"2.20\">\n\
                <Subsystem uuid=\"55555555-5555-4555-8555-555555555555\"><Properties>\n\
                <Name>Sales</Name>\n\
                <Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Продажи</v8:content></v8:item></Synonym>\n\
                <Comment/>\n\
                <IncludeHelpInContents>true</IncludeHelpInContents>\n\
                <IncludeInCommandInterface>true</IncludeInCommandInterface>\n\
                <UseOneCommand>false</UseOneCommand>\n\
                <Explanation><v8:item><v8:lang>ru</v8:lang><v8:content>Описание</v8:content></v8:item></Explanation>\n\
                <Picture>{picture}</Picture>\n\
                <Content><xr:Item xsi:type=\"xr:MDObjectRef\">Catalog.Old</xr:Item></Content>\n\
                </Properties><ChildObjects/></Subsystem></MetaDataObject>\n"
            ),
        )
        .unwrap();
        workspace
    }

    #[test]
    fn canonical_subsystem_content_preserves_picture_and_preview_plan_without_entity_churn() {
        for (reference, transparent) in [
            (true, Some("true")),
            (true, Some("false")),
            (true, None),
            (false, Some("true")),
            (false, Some("false")),
            (false, None),
        ] {
            let mut picture = String::new();
            if reference {
                picture.push_str("<xr:Ref>CommonPicture.Sales</xr:Ref>");
            }
            if let Some(value) = transparent {
                picture.push_str(&format!("<xr:LoadTransparent>{value}</xr:LoadTransparent>"));
            }
            let workspace = subsystem_picture_workspace(&picture, true);
            let descriptor = workspace.path().join("src/Subsystems/Sales.xml");
            let runtime = V5CanonicalInvocationRuntime::new(
                Arc::new(
                    crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(),
                ),
                Arc::new(TokioClock),
            );
            let workspace_hint = std::fs::canonicalize(workspace.path()).unwrap();
            let call = |arguments| {
                let request = InvocationRequest::new(
                    ToolIdentity::Apply,
                    arguments,
                    workspace_hint.to_string_lossy(),
                    7_000,
                )
                .unwrap();
                direct_v5(&runtime, request).unwrap()
            };
            let arguments = serde_json::json!({
                "at": "main:Subsystem.Sales",
                "ops": [
                    {"op": "content.add", "args": {"items": [{"object": "Catalog.Items"}]}},
                    {"op": "content.remove", "args": {"items": [{"object": "Catalog.Old"}]}}
                ]
            });
            let before = unica_workspace::test_support::tree_snapshot(workspace.path());
            let preview = call(arguments.clone());
            assert!(preview.ok, "{preview:?}");
            assert_eq!(
                before,
                unica_workspace::test_support::tree_snapshot(workspace.path())
            );
            let applied = call(
                serde_json::json!({"executionToken": preview.data.as_ref().unwrap()["executionToken"]}),
            );
            assert!(applied.ok, "{applied:?}");
            let preview_data = preview.data.as_ref().unwrap();
            let applied_data = applied.data.as_ref().unwrap();
            assert!(preview_data["planHash"].is_string(), "{preview_data}");
            assert_eq!(preview_data["planHash"], applied_data["planHash"]);
            assert_eq!(preview_data["effects"], applied_data["effects"]);
            let edited = std::fs::read_to_string(&descriptor).unwrap();
            assert!(!edited.contains("&#13;"), "{edited}");
            assert!(!edited.contains("&#xD;"), "{edited}");
            assert!(!edited.contains("\r"), "{edited}");
            assert!(edited.contains("Catalog.Items"), "{edited}");
            assert!(!edited.contains("Catalog.Old"), "{edited}");
            assert!(edited.contains("Продажи"), "{edited}");
            assert!(edited.contains("Описание"), "{edited}");
            let doc = roxmltree::Document::parse(edited.trim_start_matches('\u{feff}')).unwrap();
            let value = |name| {
                doc.descendants()
                    .find(|node| node.has_tag_name(("http://v8.1c.ru/8.3/xcf/readable", name)))
                    .and_then(|node| node.text())
            };
            assert_eq!(value("LoadTransparent"), transparent);
            assert_eq!(value("Ref"), reference.then_some("CommonPicture.Sales"));

            let child = serde_json::json!({
                "at": "main:Subsystem.Sales",
                "ops": [{"op": "childSubsystem.add", "args": {"items": [{"name": "Orders"}]}}]
            });
            let before = unica_workspace::test_support::tree_snapshot(workspace.path());
            let preview = call(child.clone());
            assert!(preview.ok, "{preview:?}");
            assert_eq!(
                before,
                unica_workspace::test_support::tree_snapshot(workspace.path())
            );
            assert!(!workspace
                .path()
                .join("src/Subsystems/Sales/Subsystems")
                .exists());
            let applied = call(
                serde_json::json!({"executionToken": preview.data.as_ref().unwrap()["executionToken"]}),
            );
            assert!(applied.ok, "{applied:?}");
            assert_eq!(
                preview.data.unwrap()["planHash"],
                applied.data.unwrap()["planHash"]
            );
            let stub = std::fs::read_to_string(
                workspace
                    .path()
                    .join("src/Subsystems/Sales/Subsystems/Orders.xml"),
            )
            .unwrap();
            assert!(stub.contains("<Picture/>"), "{stub}");
            assert!(!stub.contains("LoadTransparent"), "{stub}");
            let parent = std::fs::read_to_string(&descriptor).unwrap();
            assert!(parent.contains("<Subsystem>Orders</Subsystem>"), "{parent}");
            assert!(!parent.contains("&#13;"), "{parent}");
            let doc = roxmltree::Document::parse(parent.trim_start_matches('\u{feff}')).unwrap();
            let transparency = doc
                .descendants()
                .find(|node| {
                    node.has_tag_name(("http://v8.1c.ru/8.3/xcf/readable", "LoadTransparent"))
                })
                .and_then(|node| node.text());
            assert_eq!(transparency, transparent);

            let removal = serde_json::json!({
                "at": "main:Subsystem.Sales.Subsystem.Orders",
                "ops": [{"op": "childSubsystem.remove", "args": {}}]
            });
            let before = unica_workspace::test_support::tree_snapshot(workspace.path());
            let preview = call(removal.clone());
            assert!(preview.ok, "{preview:?}");
            assert_eq!(
                before,
                unica_workspace::test_support::tree_snapshot(workspace.path())
            );
            let applied = call(
                serde_json::json!({"executionToken": preview.data.as_ref().unwrap()["executionToken"]}),
            );
            assert!(applied.ok, "{applied:?}");
            assert_eq!(
                preview.data.unwrap()["planHash"],
                applied.data.unwrap()["planHash"]
            );
            assert!(!workspace
                .path()
                .join("src/Subsystems/Sales/Subsystems/Orders.xml")
                .exists());
            let parent = std::fs::read_to_string(&descriptor).unwrap();
            assert!(
                !parent.contains("<Subsystem>Orders</Subsystem>"),
                "{parent}"
            );
            assert!(!parent.contains("&#13;"), "{parent}");
            let doc = roxmltree::Document::parse(parent.trim_start_matches('\u{feff}')).unwrap();
            let value = |name| {
                doc.descendants()
                    .find(|node| node.has_tag_name(("http://v8.1c.ru/8.3/xcf/readable", name)))
                    .and_then(|node| node.text())
            };
            assert_eq!(value("LoadTransparent"), transparent);
            assert_eq!(value("Ref"), reference.then_some("CommonPicture.Sales"));
        }
    }

    const ACCEPTANCE_SUBSYSTEMS: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/acceptance/workspace/src/Subsystems"
    );
    const ADMINISTRATION_INTERFACE: &str = "Subsystems/Администрирование/Ext/CommandInterface.xml";
    const NESTED_CONTROL_INTERFACE: &str =
        "Subsystems/Администрирование/Subsystems/КонтрольРаботыПользователей/Ext/CommandInterface.xml";
    const ROOT_CONTROL_INTERFACE: &str =
        "Subsystems/КонтрольРаботыПользователей/Ext/CommandInterface.xml";
    const ROOT_INTERFACE: &str = "Ext/CommandInterface.xml";
    const NESTED_CONTROL_AT: &str =
        "main:Subsystem.Администрирование.Subsystem.КонтрольРаботыПользователей.Interface";

    /// Командный интерфейс в форме выгрузки 8.3.27: три команды одной группы
    /// в `CommandsOrder`, ролевое значение видимости и три группы.
    fn control_interface_document() -> String {
        let command = |name: &str, group: &str| {
            format!("\t\t<Command name=\"{name}\">\r\n\t\t\t<CommandGroup>{group}</CommandGroup>\r\n\t\t</Command>\r\n")
        };
        let mut text = String::from(
            "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<CommandInterface xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"2.20\">\r\n\t<CommandsVisibility>\r\n\t\t<Command name=\"Catalog.Пользователи.StandardCommand.OpenList\">\r\n\t\t\t<Visibility>\r\n\t\t\t\t<xr:Common>true</xr:Common>\r\n\t\t\t\t<xr:Value name=\"Role.ПолныеПрава\">false</xr:Value>\r\n\t\t\t</Visibility>\r\n\t\t</Command>\r\n\t</CommandsVisibility>\r\n\t<CommandsOrder>\r\n",
        );
        for name in [
            "Catalog.Пользователи.StandardCommand.OpenList",
            "DataProcessor.ЖурналРегистрации.Command.ЖурналРегистрации",
            "Catalog.ГруппыДоступа.StandardCommand.OpenList",
        ] {
            text.push_str(&command(name, "NavigationPanelOrdinary"));
        }
        text.push_str("\t</CommandsOrder>\r\n\t<GroupsOrder>\r\n\t\t<Group>NavigationPanelImportant</Group>\r\n\t\t<Group>NavigationPanelOrdinary</Group>\r\n\t\t<Group>NavigationPanelSeeAlso</Group>\r\n\t</GroupsOrder>\r\n</CommandInterface>");
        text
    }

    /// Рабочее пространство на настоящих файлах выгрузки приёмочного корпуса:
    /// подсистема `Администрирование` с её командным интерфейсом (порядок
    /// нескольких групп, видимость, размещение) и вложенной
    /// `КонтрольРаботыПользователей`. Рядом лежит одноимённая корневая
    /// подсистема с тем же интерфейсом, а корень несёт порядок трёх подсистем.
    fn interface_order_workspace() -> tempfile::TempDir {
        let fixture = std::path::Path::new(ACCEPTANCE_SUBSYSTEMS);
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        let write = |relative: &str, bytes: &[u8]| {
            let path = source.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        };
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        write(
            "Configuration.xml",
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects><Subsystem>Администрирование</Subsystem><Subsystem>КонтрольРаботыПользователей</Subsystem><Subsystem>Продажи</Subsystem></ChildObjects></Configuration></MetaDataObject>"#.as_bytes(),
        );
        write(
            "Subsystems/Администрирование.xml",
            &std::fs::read(fixture.join("Администрирование.xml")).unwrap(),
        );
        write(
            ADMINISTRATION_INTERFACE,
            &std::fs::read(fixture.join("Администрирование/Ext/CommandInterface.xml")).unwrap(),
        );
        let nested_descriptor = std::fs::read_to_string(
            fixture.join("Администрирование/Subsystems/КонтрольРаботыПользователей.xml"),
        )
        .unwrap();
        write(
            "Subsystems/Администрирование/Subsystems/КонтрольРаботыПользователей.xml",
            nested_descriptor.as_bytes(),
        );
        write(
            NESTED_CONTROL_INTERFACE,
            control_interface_document().as_bytes(),
        );
        // Одноимённая корневая подсистема: другой uuid, тот же интерфейс.
        write(
            "Subsystems/КонтрольРаботыПользователей.xml",
            nested_descriptor
                .replace(
                    "054627a7-71c2-5d9e-bb19-5a71f42ee959",
                    "154627a7-71c2-5d9e-bb19-5a71f42ee959",
                )
                .as_bytes(),
        );
        write(
            ROOT_CONTROL_INTERFACE,
            control_interface_document().as_bytes(),
        );
        write(
            "Subsystems/Продажи.xml",
            nested_descriptor
                .replace(
                    "054627a7-71c2-5d9e-bb19-5a71f42ee959",
                    "254627a7-71c2-5d9e-bb19-5a71f42ee959",
                )
                .replace("КонтрольРаботыПользователей", "Продажи")
                .as_bytes(),
        );
        // У корневой `Продажи` есть свой интерфейс: маршрут по последнему
        // имени нашёл бы его вместо отказа.
        write(
            "Subsystems/Продажи/Ext/CommandInterface.xml",
            control_interface_document().as_bytes(),
        );
        write(
            ROOT_INTERFACE,
            "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<CommandInterface xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"2.20\">\r\n\t<SubsystemsOrder>\r\n\t\t<Subsystem>Subsystem.Администрирование</Subsystem>\r\n\t\t<Subsystem>Subsystem.КонтрольРаботыПользователей</Subsystem>\r\n\t\t<Subsystem>Subsystem.Продажи</Subsystem>\r\n\t</SubsystemsOrder>\r\n</CommandInterface>".as_bytes(),
        );
        workspace
    }

    /// Порядок одной секции интерфейса так, как его видит разбор XML, а не
    /// писатель: `(команда, группа)` для `CommandsOrder`, значение элемента для
    /// `GroupsOrder` и `SubsystemsOrder`.
    fn interface_section(text: &str, section: &str) -> Vec<(String, Option<String>)> {
        let document = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).unwrap();
        let Some(section) = document
            .root_element()
            .children()
            .find(|node| node.has_tag_name(section))
        else {
            return Vec::new();
        };
        section
            .children()
            .filter(roxmltree::Node::is_element)
            .map(|item| match item.attribute("name") {
                Some(name) => (
                    name.to_string(),
                    item.children()
                        .find(|child| child.has_tag_name("CommandGroup"))
                        .and_then(|child| child.text())
                        .map(str::to_string),
                ),
                None => (item.text().unwrap_or_default().trim().to_string(), None),
            })
            .collect()
    }

    fn group_commands(text: &str, group: &str) -> Vec<String> {
        interface_section(text, "CommandsOrder")
            .into_iter()
            .filter(|(_, owner)| owner.as_deref() == Some(group))
            .map(|(name, _)| name)
            .collect()
    }

    fn section_values(text: &str, section: &str) -> Vec<String> {
        interface_section(text, section)
            .into_iter()
            .map(|(name, _)| name)
            .collect()
    }

    /// Всё, кроме названной секции: соседние секции файла не должна задеть
    /// правка порядка.
    fn without_section(text: &str, section: &str) -> String {
        let open = text.find(&format!("<{section}>")).unwrap();
        let close = text.find(&format!("</{section}>")).unwrap() + section.len() + 3;
        format!("{}{}", &text[..open], &text[close..])
    }

    fn interface_apply_call(
        runtime: &V5CanonicalInvocationRuntime,
        workspace: &std::path::Path,
        arguments: serde_json::Value,
    ) -> DomainResult {
        submit_canonical(runtime, workspace, ToolIdentity::Apply, arguments)
    }

    fn interface_op(op: &str, at: &str, values: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "at": at,
            "ops": [{"op": op, "args": {"at": at, "values": values}}]
        })
    }

    fn assert_refused_without_trace(result: &DomainResult, code: &str, needle: &str) {
        assert!(!result.ok, "{result:?}");
        assert_eq!(result.diagnostics[0]["code"], code, "{result:?}");
        let message = result.diagnostics[0]["message"]
            .as_str()
            .unwrap_or_default();
        assert!(message.contains(needle), "`{needle}` not in {message:?}");
        assert!(
            result
                .data
                .as_ref()
                .and_then(|data| data.get("executionToken"))
                .is_none(),
            "a refusal never mints an executable plan: {result:?}"
        );
    }

    /// Перестановка — это прежний состав в новом порядке. Для каждой из трёх
    /// операций порядка пропуск, повтор и чужой элемент отклоняются до записи:
    /// дерево исходников не меняется, а план, построенный до отказов,
    /// по-прежнему исполняется — значит, ревизия, которой он связан, та же.
    /// Для команд состав сравнивается в пределах одной группы: команда
    /// соседней группы того же файла — чужая.
    #[test]
    fn canonical_interface_order_permutes_stored_composition_and_refuses_any_other_without_writing()
    {
        let workspace = interface_order_workspace();
        let source = workspace.path().join("src");
        let runtime =
            V5CanonicalInvocationRuntime::new(canonical_v13_service(), Arc::new(TokioClock));
        let read = |relative: &str| std::fs::read_to_string(source.join(relative)).unwrap();
        let administration = read(ADMINISTRATION_INTERFACE);
        let important = group_commands(&administration, "NavigationPanelImportant");
        let ordinary = group_commands(&administration, "NavigationPanelOrdinary");
        assert!(
            important.len() >= 3 && !ordinary.is_empty(),
            "{important:?}"
        );
        let groups = section_values(&administration, "GroupsOrder");
        let subsystems = section_values(&read(ROOT_INTERFACE), "SubsystemsOrder");
        assert_eq!(subsystems.len(), 3);

        let swap = |names: &[String]| {
            let mut swapped = names.to_vec();
            swapped.swap(0, 1);
            swapped
        };
        let administration_at = "main:Subsystem.Администрирование.Interface";
        struct Case {
            op: &'static str,
            at: &'static str,
            key: &'static str,
            group: Option<&'static str>,
            relative: &'static str,
            section: &'static str,
            stored: Vec<String>,
            foreign: String,
        }
        let cases = [
            Case {
                op: "commandOrder.set",
                at: "main:Subsystem.Администрирование.Interface.Group.NavigationPanelImportant",
                key: "commands",
                group: Some("NavigationPanelImportant"),
                relative: ADMINISTRATION_INTERFACE,
                section: "CommandsOrder",
                stored: important.clone(),
                // Команда того же файла, но другой группы: состав сравнивается
                // в области операции.
                foreign: ordinary[0].clone(),
            },
            Case {
                op: "groupOrder.set",
                at: administration_at,
                key: "groups",
                group: None,
                relative: ADMINISTRATION_INTERFACE,
                section: "GroupsOrder",
                stored: groups.clone(),
                foreign: "CommandGroup.НеСохранённая".to_string(),
            },
            Case {
                op: "subsystemOrder.set",
                at: "main:Configuration",
                key: "subsystems",
                group: None,
                relative: ROOT_INTERFACE,
                section: "SubsystemsOrder",
                stored: subsystems.clone(),
                foreign: "Subsystem.Склад".to_string(),
            },
        ];
        for case in cases {
            let values = |names: Vec<String>| {
                // Группа порядка команд — в адресе, а не в аргументах.
                let mut values = serde_json::Map::new();
                values.insert(case.key.to_string(), serde_json::json!(names));
                serde_json::Value::Object(values)
            };
            let reordered = swap(&case.stored);
            let before = unica_workspace::test_support::tree_snapshot(workspace.path());
            let preview = interface_apply_call(
                &runtime,
                workspace.path(),
                interface_op(case.op, case.at, values(reordered.clone())),
            );
            assert!(preview.ok, "{}: {preview:?}", case.op);
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(workspace.path()),
                before,
                "{}: preview writes nothing",
                case.op
            );
            let token = preview.data.as_ref().unwrap()["executionToken"].clone();

            let last = case.stored.len() - 1;
            let mut omitted = reordered.clone();
            omitted.pop();
            let mut repeated = reordered.clone();
            repeated.push(reordered[0].clone());
            let mut doubled = reordered.clone();
            doubled[last] = reordered[0].clone();
            let mut substituted = reordered.clone();
            substituted[last] = case.foreign.clone();
            let mut added = reordered.clone();
            added.push(case.foreign.clone());
            for (label, names, needle) in [
                ("omission", omitted, case.stored[last].clone()),
                ("repeat", repeated, reordered[0].clone()),
                ("repeat in place", doubled, reordered[0].clone()),
                ("substitution", substituted, case.foreign.clone()),
                ("addition", added, case.foreign.clone()),
            ] {
                let refused = interface_apply_call(
                    &runtime,
                    workspace.path(),
                    interface_op(case.op, case.at, values(names)),
                );
                assert_refused_without_trace(&refused, "bad_value", &needle);
                assert_eq!(
                    refused.at.as_deref(),
                    Some(format!("ops[0].args.values.{}", case.key).as_str()),
                    "{} {label}: {refused:?}",
                    case.op
                );
                assert_eq!(
                    unica_workspace::test_support::tree_snapshot(workspace.path()),
                    before,
                    "{} {label}: a refusal writes nothing",
                    case.op
                );
            }

            let applied = interface_apply_call(
                &runtime,
                workspace.path(),
                serde_json::json!({"executionToken": token}),
            );
            assert!(
                applied.ok,
                "{}: refusals left the revision of the earlier plan intact: {applied:?}",
                case.op
            );
            let edited = read(case.relative);
            let after = match case.group {
                Some(group) => group_commands(&edited, group),
                None => section_values(&edited, case.section),
            };
            assert_eq!(after, reordered, "{}", case.op);
        }
    }

    /// Правка порядка одной группы трогает только её команды: порядок второй
    /// группы, остальные секции, видимость и размещение остаются байт в байт.
    #[test]
    fn canonical_command_order_of_one_group_keeps_other_groups_and_sections() {
        let workspace = interface_order_workspace();
        let runtime =
            V5CanonicalInvocationRuntime::new(canonical_v13_service(), Arc::new(TokioClock));
        let path = workspace.path().join("src").join(ADMINISTRATION_INTERFACE);
        let original = std::fs::read_to_string(&path).unwrap();
        let entries = interface_section(&original, "CommandsOrder");
        let important = group_commands(&original, "NavigationPanelImportant");
        let mut reordered = important.clone();
        reordered.reverse();

        let preview = interface_apply_call(
            &runtime,
            workspace.path(),
            interface_op(
                "commandOrder.set",
                "main:Subsystem.Администрирование.Interface.Group.NavigationPanelImportant",
                serde_json::json!({"commands": reordered}),
            ),
        );
        assert!(preview.ok, "{preview:?}");
        let applied = interface_apply_call(
            &runtime,
            workspace.path(),
            serde_json::json!({"executionToken": preview.data.as_ref().unwrap()["executionToken"]}),
        );
        assert!(applied.ok, "{applied:?}");

        let edited = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            without_section(&edited, "CommandsOrder"),
            without_section(&original, "CommandsOrder"),
            "видимость, размещение и порядок групп не меняются"
        );
        let mut targets = reordered.iter();
        let expected = entries
            .iter()
            .map(|(name, group)| {
                if group.as_deref() == Some("NavigationPanelImportant") {
                    (targets.next().unwrap().clone(), group.clone())
                } else {
                    (name.clone(), group.clone())
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(
            interface_section(&edited, "CommandsOrder"),
            expected,
            "команды других групп остаются на своих местах"
        );
    }

    /// Пустой интерфейс платформа не выгружает: у новой подсистемы нет
    /// `CommandInterface.xml`, и его создаёт первая правка видимости или
    /// размещения — тем же пакетом, что `subsystem.create`, или следующим
    /// вызовом. Предпросмотр ничего не пишет, а файл совпадает с выгрузкой
    /// 8.3.27 байт в байт.
    #[test]
    fn canonical_first_command_edit_creates_interface_of_new_subsystem() {
        let runtime =
            V5CanonicalInvocationRuntime::new(canonical_v13_service(), Arc::new(TokioClock));
        let command = "Catalog.Пользователи.StandardCommand.OpenList";
        let document = |section: &str, value: &str| {
            format!(
                "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<CommandInterface xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"2.20\">\r\n\t<{section}>\r\n\t\t<Command name=\"{command}\">\r\n{value}\t\t</Command>\r\n\t</{section}>\r\n</CommandInterface>"
            )
        };
        let hidden = document(
            "CommandsVisibility",
            "\t\t\t<Visibility>\r\n\t\t\t\t<xr:Common>false</xr:Common>\r\n\t\t\t</Visibility>\r\n",
        );
        let placed = document(
            "CommandsPlacement",
            "\t\t\t<CommandGroup>NavigationPanelImportant</CommandGroup>\r\n\t\t\t<Placement>Auto</Placement>\r\n",
        );
        let create = serde_json::json!({
            "op": "subsystem.create",
            "args": {"at": "main:Configuration", "values": {"name": "Новая"}}
        });
        let at = "main:Subsystem.Новая.Interface";
        let visibility = serde_json::json!({
            "op": "commandVisibility.set",
            "args": {"at": at, "items": [{"command": command, "visible": false}]}
        });
        let placement = serde_json::json!({
            "op": "commandPlacement.set",
            "args": {"at": at, "items": [{"command": command, "group": "NavigationPanelImportant"}]}
        });
        let interface = "Subsystems/Новая/Ext/CommandInterface.xml";
        let publish = |workspace: &std::path::Path, at: &str, ops: serde_json::Value| {
            let preview = interface_apply_call(
                &runtime,
                workspace,
                serde_json::json!({"at": at, "ops": ops}),
            );
            assert!(preview.ok, "{preview:?}");
            assert!(
                !workspace.join("src").join(interface).exists(),
                "предпросмотр не пишет файл"
            );
            let applied = interface_apply_call(
                &runtime,
                workspace,
                serde_json::json!({"executionToken": preview.data.as_ref().unwrap()["executionToken"]}),
            );
            assert!(applied.ok, "{applied:?}");
        };
        let read = |workspace: &std::path::Path| {
            std::fs::read_to_string(workspace.join("src").join(interface)).unwrap()
        };

        let workspace = interface_order_workspace();
        publish(
            workspace.path(),
            "main:Configuration",
            serde_json::json!([create, visibility]),
        );
        assert_eq!(read(workspace.path()), hidden, "один пакет");

        for (edit, expected) in [(&visibility, &hidden), (&placement, &placed)] {
            let workspace = interface_order_workspace();
            publish(
                workspace.path(),
                "main:Configuration",
                serde_json::json!([create]),
            );
            assert!(!workspace.path().join("src").join(interface).exists());
            publish(workspace.path(), at, serde_json::json!([edit]));
            assert_eq!(&read(workspace.path()), expected, "два вызова: {edit}");
        }

        // Вложенная подсистема: владелец — её дескриптор внутри родителя,
        // одноимённая корневая подсистема не появляется.
        let workspace = interface_order_workspace();
        let nested_at = "main:Subsystem.Администрирование.Subsystem.Новая.Interface";
        publish(
            workspace.path(),
            "main:Subsystem.Администрирование",
            serde_json::json!([
                {"op": "childSubsystem.add", "args": {"items": [{"name": "Новая"}]}},
                {"op": "commandVisibility.set", "args": {"at": nested_at, "items": [{"command": command, "visible": false}]}}
            ]),
        );
        let source = workspace.path().join("src");
        assert_eq!(
            std::fs::read_to_string(
                source
                    .join("Subsystems/Администрирование/Subsystems/Новая/Ext/CommandInterface.xml")
            )
            .unwrap(),
            hidden
        );
        assert!(!source.join("Subsystems/Новая").exists());

        // Порядок переставляет хранимый состав: без файла он отказывает
        // и ничего не создаёт (#2010).
        let workspace = interface_order_workspace();
        publish(
            workspace.path(),
            "main:Configuration",
            serde_json::json!([create]),
        );
        let refused = interface_apply_call(
            &runtime,
            workspace.path(),
            interface_op(
                "groupOrder.set",
                at,
                serde_json::json!({"groups": ["NavigationPanelImportant"]}),
            ),
        );
        assert!(!refused.ok, "{refused:?}");
        assert!(!workspace.path().join("src").join(interface).exists());
    }

    /// Пока порядок разделов не задан, корневого `CommandInterface.xml` нет.
    /// Первый порядок, как у платформы, может быть частичным: он называет
    /// разделы интерфейса — подсистемы верхнего уровня, включённые в него, —
    /// и создаёт файл в форме выгрузки 8.3.27. Чужое имя, раздел вне
    /// интерфейса и повтор отклоняются без записи.
    #[test]
    fn canonical_first_subsystem_order_creates_root_interface_from_sections() {
        let runtime =
            V5CanonicalInvocationRuntime::new(canonical_v13_service(), Arc::new(TokioClock));
        let root_order = |subsystems: serde_json::Value| {
            serde_json::json!({
                "op": "subsystemOrder.set",
                "args": {"at": "main:Configuration", "values": {"subsystems": subsystems}}
            })
        };
        let document = |subsystems: &[&str]| {
            let items = subsystems
                .iter()
                .map(|name| format!("\t\t<Subsystem>{name}</Subsystem>\r\n"))
                .collect::<String>();
            format!(
                "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<CommandInterface xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"2.20\">\r\n\t<SubsystemsOrder>\r\n{items}\t</SubsystemsOrder>\r\n</CommandInterface>"
            )
        };
        let fresh = || {
            let workspace = interface_order_workspace();
            let source = workspace.path().join("src");
            std::fs::remove_file(source.join(ROOT_INTERFACE)).unwrap();
            // `Продажи` не показывается в интерфейсе: разделом она не станет.
            let sales = source.join("Subsystems/Продажи.xml");
            let text = std::fs::read_to_string(&sales).unwrap().replace(
                "<IncludeInCommandInterface>true</IncludeInCommandInterface>",
                "<IncludeInCommandInterface>false</IncludeInCommandInterface>",
            );
            std::fs::write(&sales, text).unwrap();
            workspace
        };
        let preview = |workspace: &std::path::Path, ops: serde_json::Value| {
            interface_apply_call(
                &runtime,
                workspace,
                serde_json::json!({"at": "main:Configuration", "ops": ops}),
            )
        };

        let workspace = fresh();
        let root = workspace.path().join("src").join(ROOT_INTERFACE);
        let planned = preview(
            workspace.path(),
            serde_json::json!([root_order(serde_json::json!([
                "Subsystem.КонтрольРаботыПользователей"
            ]))]),
        );
        assert!(planned.ok, "{planned:?}");
        assert!(!root.exists(), "предпросмотр не пишет файл");
        let applied = interface_apply_call(
            &runtime,
            workspace.path(),
            serde_json::json!({"executionToken": planned.data.as_ref().unwrap()["executionToken"]}),
        );
        assert!(applied.ok, "{applied:?}");
        assert_eq!(
            std::fs::read_to_string(&root).unwrap(),
            document(&["Subsystem.КонтрольРаботыПользователей"])
        );

        // Новая подсистема того же пакета уже раздел интерфейса.
        let workspace = fresh();
        let root = workspace.path().join("src").join(ROOT_INTERFACE);
        let planned = preview(
            workspace.path(),
            serde_json::json!([
                {"op": "subsystem.create", "args": {"at": "main:Configuration", "values": {"name": "Новая"}}},
                root_order(serde_json::json!(["Subsystem.Новая", "Subsystem.Администрирование"]))
            ]),
        );
        assert!(planned.ok, "{planned:?}");
        let applied = interface_apply_call(
            &runtime,
            workspace.path(),
            serde_json::json!({"executionToken": planned.data.as_ref().unwrap()["executionToken"]}),
        );
        assert!(applied.ok, "{applied:?}");
        assert_eq!(
            std::fs::read_to_string(&root).unwrap(),
            document(&["Subsystem.Новая", "Subsystem.Администрирование"])
        );

        // Файл есть, но порядка разделов в нём нет: тот же первый порядок,
        // соседняя секция остаётся на месте.
        let workspace = fresh();
        let root = workspace.path().join("src").join(ROOT_INTERFACE);
        let visibility = "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<CommandInterface xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"2.20\">\r\n\t<CommandsVisibility>\r\n\t\t<Command name=\"Catalog.Пользователи.StandardCommand.OpenList\">\r\n\t\t\t<Visibility>\r\n\t\t\t\t<xr:Common>false</xr:Common>\r\n\t\t\t</Visibility>\r\n\t\t</Command>\r\n\t</CommandsVisibility>\r\n</CommandInterface>";
        std::fs::write(&root, visibility).unwrap();
        let planned = preview(
            workspace.path(),
            serde_json::json!([root_order(serde_json::json!([
                "Subsystem.Администрирование"
            ]))]),
        );
        assert!(planned.ok, "{planned:?}");
        let applied = interface_apply_call(
            &runtime,
            workspace.path(),
            serde_json::json!({"executionToken": planned.data.as_ref().unwrap()["executionToken"]}),
        );
        assert!(applied.ok, "{applied:?}");
        assert_eq!(
            std::fs::read_to_string(&root).unwrap(),
            visibility.replace(
                "\t</CommandsVisibility>\r\n",
                "\t</CommandsVisibility>\r\n\t<SubsystemsOrder>\r\n\t\t<Subsystem>Subsystem.Администрирование</Subsystem>\r\n\t</SubsystemsOrder>\r\n",
            )
        );

        for (subsystems, needle) in [
            (
                serde_json::json!(["Subsystem.Продажи"]),
                "not a top-level subsystem shown in the command interface: `Subsystem.Продажи`",
            ),
            (
                serde_json::json!(["Subsystem.Нет", "Subsystem.Нет"]),
                "not a top-level subsystem shown in the command interface: `Subsystem.Нет`; the sections are",
            ),
            // Вложенная подсистема разделом не бывает, даже одноимённая
            // корневой.
            (
                serde_json::json!(["Subsystem.Администрирование.Subsystem.КонтрольРаботыПользователей"]),
                "not a top-level subsystem shown in the command interface",
            ),
            (
                serde_json::json!(["Subsystem.Администрирование", "Subsystem.Администрирование"]),
                "listed more than once",
            ),
        ] {
            let workspace = fresh();
            let refused = preview(
                workspace.path(),
                serde_json::json!([root_order(subsystems)]),
            );
            assert!(!refused.ok, "{refused:?}");
            assert!(
                refused.diagnostics[0]["message"]
                    .as_str()
                    .is_some_and(|message| message.contains(needle)),
                "{refused:?}"
            );
            assert!(!workspace.path().join("src").join(ROOT_INTERFACE).exists());
        }
    }

    /// Каждая из пяти операций меняет только свою секцию и названные в запросе
    /// элементы: для видимости и размещения файл после правки совпадает
    /// с исходным байт в байт, кроме значения одной команды, для порядков —
    /// всё вне своей секции. Ролевое значение видимости переживает правку
    /// общего.
    #[test]
    fn canonical_each_interface_operation_changes_only_its_own_section_and_items() {
        let workspace = interface_order_workspace();
        let source = workspace.path().join("src");
        let runtime =
            V5CanonicalInvocationRuntime::new(canonical_v13_service(), Arc::new(TokioClock));
        let publish = |arguments: serde_json::Value| {
            let preview = interface_apply_call(&runtime, workspace.path(), arguments);
            assert!(preview.ok, "{preview:?}");
            let applied = interface_apply_call(
                &runtime,
                workspace.path(),
                serde_json::json!({"executionToken": preview.data.as_ref().unwrap()["executionToken"]}),
            );
            assert!(applied.ok, "{applied:?}");
        };
        let read = |relative: &str| std::fs::read_to_string(source.join(relative)).unwrap();
        // Значение одного дочернего элемента одной команды в одной секции.
        let with_child = |text: &str, section: &str, command: &str, from: &str, to: &str| {
            let open = text.find(&format!("<{section}>")).unwrap();
            let start = open + text[open..].find(&format!("name=\"{command}\"")).unwrap();
            let end = start + text[start..].find("</Command>").unwrap();
            let block = text[start..end].replacen(from, to, 1);
            assert_ne!(block, text[start..end], "{command}: {from} is in the block");
            format!("{}{block}{}", &text[..start], &text[end..])
        };
        let administration_at = "main:Subsystem.Администрирование.Interface";
        let command = "Catalog.Пользователи.StandardCommand.OpenList";

        let original = read(ADMINISTRATION_INTERFACE);
        publish(serde_json::json!({
            "at": administration_at,
            "ops": [{"op": "commandVisibility.set", "args": {"at": administration_at, "items": [{"command": command, "visible": true}]}}]
        }));
        assert_eq!(
            read(ADMINISTRATION_INTERFACE),
            with_child(
                &original,
                "CommandsVisibility",
                command,
                "<xr:Common>false</xr:Common>",
                "<xr:Common>true</xr:Common>"
            )
        );

        let original = read(ADMINISTRATION_INTERFACE);
        publish(serde_json::json!({
            "at": administration_at,
            "ops": [{"op": "commandPlacement.set", "args": {"at": administration_at, "items": [{"command": command, "group": "NavigationPanelOrdinary"}]}}]
        }));
        assert_eq!(
            read(ADMINISTRATION_INTERFACE),
            with_child(
                &original,
                "CommandsPlacement",
                command,
                "<CommandGroup>NavigationPanelImportant</CommandGroup>",
                "<CommandGroup>NavigationPanelOrdinary</CommandGroup>"
            )
        );

        let original = read(ADMINISTRATION_INTERFACE);
        let mut groups = section_values(&original, "GroupsOrder");
        groups.reverse();
        publish(interface_op(
            "groupOrder.set",
            administration_at,
            serde_json::json!({"groups": groups}),
        ));
        let edited = read(ADMINISTRATION_INTERFACE);
        assert_eq!(section_values(&edited, "GroupsOrder"), groups);
        assert_eq!(
            without_section(&edited, "GroupsOrder"),
            without_section(&original, "GroupsOrder")
        );

        // Корень: рядом с порядком подсистем лежит видимость с ролевым значением.
        let root = control_interface_document().replace(
            "\t</CommandsOrder>\r\n",
            "\t</CommandsOrder>\r\n\t<SubsystemsOrder>\r\n\t\t<Subsystem>Subsystem.Администрирование</Subsystem>\r\n\t\t<Subsystem>Subsystem.Продажи</Subsystem>\r\n\t</SubsystemsOrder>\r\n",
        );
        std::fs::write(source.join(ROOT_INTERFACE), &root).unwrap();
        publish(interface_op(
            "subsystemOrder.set",
            "main:Configuration",
            serde_json::json!({"subsystems": ["Subsystem.Продажи", "Subsystem.Администрирование"]}),
        ));
        let edited = read(ROOT_INTERFACE);
        assert_eq!(
            section_values(&edited, "SubsystemsOrder"),
            ["Subsystem.Продажи", "Subsystem.Администрирование"]
        );
        assert_eq!(
            without_section(&edited, "SubsystemsOrder"),
            without_section(&root, "SubsystemsOrder")
        );
        assert!(edited.contains("<xr:Value name=\"Role.ПолныеПрава\">false</xr:Value>"));
    }

    /// Адрес интерфейса несёт всю цепочку владельцев: правка вложенной
    /// подсистемы не уходит в одноимённую корневую. Неподдержанная цель и
    /// чужой маршрут отказывают, а не выбирают другой интерфейс.
    #[test]
    fn canonical_interface_edits_reach_exactly_the_nested_owner_and_refuse_foreign_targets() {
        let workspace = interface_order_workspace();
        let source = workspace.path().join("src");
        let runtime =
            V5CanonicalInvocationRuntime::new(canonical_v13_service(), Arc::new(TokioClock));
        let commands = group_commands(&control_interface_document(), "NavigationPanelOrdinary");
        let root_before = std::fs::read(source.join(ROOT_CONTROL_INTERFACE)).unwrap();
        let reversed = |names: &[String]| names.iter().rev().cloned().collect::<Vec<_>>();
        let ops = [
            serde_json::json!({"op": "commandVisibility.set", "args": {"at": NESTED_CONTROL_AT, "items": [{"command": commands[0], "visible": false}]}}),
            serde_json::json!({"op": "commandPlacement.set", "args": {"at": NESTED_CONTROL_AT, "items": [{"command": commands[1], "group": "NavigationPanelImportant"}]}}),
            serde_json::json!({"op": "commandOrder.set", "args": {"at": format!("{NESTED_CONTROL_AT}.Group.NavigationPanelOrdinary"), "values": {"commands": reversed(&commands)}}}),
            serde_json::json!({"op": "groupOrder.set", "args": {"at": NESTED_CONTROL_AT, "values": {"groups": ["NavigationPanelSeeAlso", "NavigationPanelOrdinary", "NavigationPanelImportant"]}}}),
        ];
        let preview = interface_apply_call(
            &runtime,
            workspace.path(),
            serde_json::json!({"at": NESTED_CONTROL_AT, "ops": ops}),
        );
        assert!(preview.ok, "{preview:?}");
        let applied = interface_apply_call(
            &runtime,
            workspace.path(),
            serde_json::json!({"executionToken": preview.data.as_ref().unwrap()["executionToken"]}),
        );
        assert!(applied.ok, "{applied:?}");
        assert_eq!(
            std::fs::read(source.join(ROOT_CONTROL_INTERFACE)).unwrap(),
            root_before,
            "одноимённая корневая подсистема не тронута"
        );
        let nested = std::fs::read_to_string(source.join(NESTED_CONTROL_INTERFACE)).unwrap();
        assert!(nested.contains("<xr:Common>false</xr:Common>"), "{nested}");
        assert!(
            nested.contains("<xr:Value name=\"Role.ПолныеПрава\">false</xr:Value>"),
            "ролевое значение видимости сохранено: {nested}"
        );
        assert!(
            nested.contains("<CommandsPlacement>") && nested.contains(&commands[1]),
            "{nested}"
        );
        assert_eq!(
            group_commands(&nested, "NavigationPanelOrdinary"),
            reversed(&commands)
        );
        assert_eq!(
            section_values(&nested, "GroupsOrder"),
            [
                "NavigationPanelSeeAlso",
                "NavigationPanelOrdinary",
                "NavigationPanelImportant"
            ]
        );

        let before = unica_workspace::test_support::tree_snapshot(workspace.path());
        // Вложенной подсистемы с таким именем нет — одноимённая корневая
        // `Продажи` с годным для этой перестановки интерфейсом целью не
        // становится.
        let missing = interface_apply_call(
            &runtime,
            workspace.path(),
            interface_op(
                "groupOrder.set",
                "main:Subsystem.Администрирование.Subsystem.Продажи.Interface",
                serde_json::json!({"groups": ["NavigationPanelSeeAlso", "NavigationPanelOrdinary", "NavigationPanelImportant"]}),
            ),
        );
        assert_refused_without_trace(&missing, "not_found", "command interface");
        // Порядок подсистем живёт только в корне: на интерфейсе подсистемы
        // операция отклоняется реестром, до планирования.
        for at in [
            "main:Subsystem.Администрирование.Interface",
            NESTED_CONTROL_AT,
            "main:Interface",
        ] {
            let refused = interface_apply_call(
                &runtime,
                workspace.path(),
                interface_op(
                    "subsystemOrder.set",
                    at,
                    serde_json::json!({"subsystems": ["Subsystem.КонтрольРаботыПользователей"]}),
                ),
            );
            assert_refused_without_trace(
                &refused,
                "bad_value",
                "apply operation `subsystemOrder.set` does not apply to Interface",
            );
        }
        // Остальные четыре операции не метят в корень и в `main:Interface`.
        for (op, key, value) in [
            (
                "commandVisibility.set",
                "items",
                serde_json::json!([{"command": commands[0], "visible": true}]),
            ),
            (
                "commandPlacement.set",
                "items",
                serde_json::json!([{"command": commands[0], "group": "NavigationPanelImportant"}]),
            ),
            (
                "commandOrder.set",
                "values",
                serde_json::json!({"commands": commands}),
            ),
            (
                "groupOrder.set",
                "values",
                serde_json::json!({"groups": ["NavigationPanelOrdinary"]}),
            ),
        ] {
            for (at, needle) in [
                (
                    "main:Configuration",
                    format!("apply operation `{op}` does not apply to Configuration"),
                ),
                (
                    "main:Interface",
                    // Порядок команд адресует группу этого интерфейса.
                    if op == "commandOrder.set" {
                        "it addresses `Subsystem.Name[.Subsystem.Child].Interface.Group.G`"
                    } else {
                        "it addresses `Subsystem.Name[.Subsystem.Child].Interface`"
                    }
                    .to_string(),
                ),
            ] {
                let refused = interface_apply_call(
                    &runtime,
                    workspace.path(),
                    serde_json::json!({"at": at, "ops": [{"op": op, "args": {"at": at, key: value}}]}),
                );
                assert_refused_without_trace(&refused, "bad_value", &needle);
            }
        }
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(workspace.path()),
            before,
            "отказы ничего не пишут"
        );
    }

    /// Предпросмотр ничего не пишет, а план связан с ревизией исходников:
    /// если после него состав группы изменился, исполнение отказывает
    /// и оставляет файл таким, каким его сделала сторонняя правка.
    #[test]
    fn canonical_interface_order_plan_is_refused_after_the_composition_changes() {
        let workspace = interface_order_workspace();
        let runtime =
            V5CanonicalInvocationRuntime::new(canonical_v13_service(), Arc::new(TokioClock));
        let path = workspace.path().join("src").join(NESTED_CONTROL_INTERFACE);
        let original = std::fs::read_to_string(&path).unwrap();
        let mut commands = group_commands(&original, "NavigationPanelOrdinary");
        commands.swap(0, 1);
        let before = unica_workspace::test_support::tree_snapshot(workspace.path());
        let preview = interface_apply_call(
            &runtime,
            workspace.path(),
            interface_op(
                "commandOrder.set",
                &format!("{NESTED_CONTROL_AT}.Group.NavigationPanelOrdinary"),
                serde_json::json!({"commands": commands}),
            ),
        );
        assert!(preview.ok, "{preview:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(workspace.path()),
            before
        );

        let extended = original.replace(
            "\t</CommandsOrder>",
            "\t\t<Command name=\"CommonCommand.НоваяКоманда\">\r\n\t\t\t<CommandGroup>NavigationPanelOrdinary</CommandGroup>\r\n\t\t</Command>\r\n\t</CommandsOrder>",
        );
        std::fs::write(&path, &extended).unwrap();
        let stale = interface_apply_call(
            &runtime,
            workspace.path(),
            serde_json::json!({"executionToken": preview.data.as_ref().unwrap()["executionToken"]}),
        );
        assert!(!stale.ok, "{stale:?}");
        assert_eq!(stale.diagnostics[0]["code"], "stale_revision", "{stale:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), extended);

        // Новый план видит новый состав, и прежний список уже не перестановка.
        let replanned = interface_apply_call(
            &runtime,
            workspace.path(),
            interface_op(
                "commandOrder.set",
                &format!("{NESTED_CONTROL_AT}.Group.NavigationPanelOrdinary"),
                serde_json::json!({"commands": commands}),
            ),
        );
        assert_refused_without_trace(&replanned, "bad_value", "CommonCommand.НоваяКоманда");
    }

    /// Группа — узел адреса: порядок команд правит только группу, которую
    /// интерфейс объявляет, как её видит чтение. Неизвестная группа отвечает
    /// `not_found` без записи, а не заводится порядком.
    #[test]
    fn canonical_command_order_refuses_an_undeclared_group_without_writing() {
        let workspace = interface_order_workspace();
        let runtime =
            V5CanonicalInvocationRuntime::new(canonical_v13_service(), Arc::new(TokioClock));
        let before = unica_workspace::test_support::tree_snapshot(workspace.path());
        let refused = interface_apply_call(
            &runtime,
            workspace.path(),
            interface_op(
                "commandOrder.set",
                &format!("{NESTED_CONTROL_AT}.Group.CommandGroup\\.НетТакой"),
                serde_json::json!({"commands": ["CommonCommand.Любая"]}),
            ),
        );
        assert_refused_without_trace(
            &refused,
            "not_found",
            "declares no group `CommandGroup.НетТакой`",
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(workspace.path()),
            before
        );
    }

    #[test]
    fn canonical_subsystem_picture_property_refusal_never_mints_a_token_and_execution_cannot_override_ops(
    ) {
        let workspace =
            subsystem_picture_workspace("<xr:LoadTransparent>true</xr:LoadTransparent>", false);
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(
                crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(),
            ),
            Arc::new(TokioClock),
        );
        let workspace_hint = std::fs::canonicalize(workspace.path()).unwrap();
        let call = |arguments| {
            let request = InvocationRequest::new(
                ToolIdentity::Apply,
                arguments,
                workspace_hint.to_string_lossy(),
                7_000,
            )
            .unwrap();
            direct_v5(&runtime, request).unwrap()
        };
        let mut arguments = serde_json::json!({
            "at": "main:Subsystem.Sales",
            "ops": [{"op": "childSubsystem.add", "args": {"items": [{"name": "Orders"}]}}]
        });
        let before = unica_workspace::test_support::tree_snapshot(workspace.path());
        let preview = call(arguments.clone());
        assert!(preview.ok, "{preview:?}");
        arguments["ops"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::Value::Null);
        for property in ["LoadTransparent", "Picture.LoadTransparent"] {
            arguments["ops"][1] = serde_json::json!({
                "op": "props.set", "args": {"values": {property: true}}
            });
            let refused_preview = call(arguments.clone());
            let refused_apply = call(serde_json::json!({
                "executionToken": preview.data.as_ref().unwrap()["executionToken"],
                "ops": arguments["ops"]
            }));
            assert!(!refused_preview.ok, "{refused_preview:?}");
            assert!(!refused_apply.ok, "{refused_apply:?}");
            assert_eq!(refused_preview.diagnostics[0]["code"], "bad_value");
            assert!(refused_preview
                .data
                .as_ref()
                .and_then(|data| data.get("executionToken"))
                .is_none());
            assert_eq!(refused_apply.diagnostics[0]["code"], "bad_value");
            assert!(format!("{refused_preview:?}").contains(property));
            assert_eq!(
                before,
                unica_workspace::test_support::tree_snapshot(workspace.path())
            );
            assert!(!workspace
                .path()
                .join("src/Subsystems/Sales/Subsystems")
                .exists());
        }
    }

    fn dcs_check_workspace(kind: &str) -> (tempfile::TempDir, String, std::path::PathBuf) {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        let external = kind != "Report";
        let source_type = match kind {
            "ExternalReport" => "EXTERNAL_REPORTS",
            "ExternalDataProcessor" => "EXTERNAL_DATA_PROCESSORS",
            "Report" => "CONFIGURATION",
            _ => unreachable!(),
        };
        let owner = if external {
            source.join("Sales")
        } else {
            source.join("Reports/Sales")
        };
        std::fs::create_dir_all(owner.join("Templates/Dcs/Ext")).unwrap();
        std::fs::write(workspace.path().join("v8project.yaml"), format!(
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: {source_type}\n    path: src\n"
        )).unwrap();
        let descriptor = |body: String| {
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">{body}</MetaDataObject>"#
            )
        };
        if !external {
            std::fs::write(source.join("Configuration.xml"), descriptor(
                "<Configuration uuid=\"10000000-0000-4000-8000-000000000001\"><Properties><Name>Control</Name><DefaultLanguage>Language.Русский</DefaultLanguage></Properties><ChildObjects><Language>Русский</Language><Report>Sales</Report></ChildObjects></Configuration>".into()
            )).unwrap();
            std::fs::create_dir_all(source.join("Languages")).unwrap();
            std::fs::write(source.join("Languages/Русский.xml"), descriptor(
                "<Language uuid=\"10000000-0000-4000-8000-000000000002\"><Properties><Name>Русский</Name><Comment/><LanguageCode>ru</LanguageCode></Properties></Language>".into()
            )).unwrap();
        }
        std::fs::write(owner.with_extension("xml"), descriptor(format!(
            "<{kind} uuid=\"10000000-0000-4000-8000-000000000010\"><Properties><Name>Sales</Name></Properties><ChildObjects><Template>Dcs</Template></ChildObjects></{kind}>"
        ))).unwrap();
        std::fs::write(owner.join("Templates/Dcs.xml"), descriptor(
            "<Template uuid=\"10000000-0000-4000-8000-000000000013\"><Properties><Name>Dcs</Name><TemplateType>DataCompositionSchema</TemplateType></Properties></Template>".into()
        )).unwrap();
        let body = owner.join("Templates/Dcs/Ext/Template.xml");
        std::fs::write(
            &body,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema"
		xmlns:dcscom="http://v8.1c.ru/8.1/data-composition-system/common"
		xmlns:dcscor="http://v8.1c.ru/8.1/data-composition-system/core"
		xmlns:dcsset="http://v8.1c.ru/8.1/data-composition-system/settings"
		xmlns:v8="http://v8.1c.ru/8.1/data/core"
		xmlns:v8ui="http://v8.1c.ru/8.1/data/ui"
		xmlns:xs="http://www.w3.org/2001/XMLSchema"
		xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
	<dataSource>
		<name>ИсточникДанных1</name>
		<dataSourceType>Local</dataSourceType>
	</dataSource>
	<dataSet xsi:type="DataSetQuery">
		<name>НаборДанных1</name>
		<field xsi:type="DataSetFieldField">
			<dataPath>Amount</dataPath>
			<field>Amount</field>
		</field>
		<dataSource>ИсточникДанных1</dataSource>
		<query>ВЫБРАТЬ Amount КАК Amount</query>
	</dataSet>
	<settingsVariant>
		<dcsset:name>Основной</dcsset:name>
		<dcsset:settings>
			<dcsset:selection>
			</dcsset:selection>
			<dcsset:filter>
			</dcsset:filter>
			<dcsset:order>
			</dcsset:order>
			<dcsset:item xsi:type="dcsset:StructureItemGroup">
				<dcsset:selection>
					<dcsset:item xsi:type="dcsset:SelectedItemAuto"/>
				</dcsset:selection>
			</dcsset:item>
		</dcsset:settings>
	</settingsVariant>
</DataCompositionSchema>
"#,
        )
        .unwrap();
        (workspace, format!("main:{kind}.Sales.Template.Dcs"), body)
    }

    fn call_dcs_check(workspace: &std::path::Path, tool: ToolIdentity, at: &str) -> DomainResult {
        direct_v5(
            &bootstrap_runtime(),
            InvocationRequest::new(
                tool,
                serde_json::json!({"at": at}),
                std::fs::canonicalize(workspace).unwrap().to_string_lossy(),
                7_000,
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn form_check_workspace(kind: &str) -> (tempfile::TempDir, String, std::path::PathBuf) {
        let extension = kind == "Extension";
        let kind = if extension { "Report" } else { kind };
        let (workspace, _, template) = dcs_check_workspace(kind);
        if extension {
            let config = workspace.path().join("v8project.yaml");
            let text = std::fs::read_to_string(&config).unwrap();
            std::fs::write(config, text.replace("CONFIGURATION", "EXTENSION")).unwrap();
            let descriptor = workspace.path().join("src/Configuration.xml");
            let text = std::fs::read_to_string(&descriptor).unwrap();
            std::fs::write(descriptor, text.replace("<Name>Control</Name>", "<Name>Control</Name><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose>")).unwrap();
        }
        let owner = template
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let descriptor = owner.with_extension("xml");
        let text = std::fs::read_to_string(&descriptor).unwrap();
        std::fs::write(
            &descriptor,
            text.replace("</ChildObjects>", "<Form>Main</Form></ChildObjects>"),
        )
        .unwrap();
        let body = owner.join("Forms/Main/Ext/Form.xml");
        std::fs::create_dir_all(body.parent().unwrap()).unwrap();
        std::fs::write(owner.join("Forms/Main.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form uuid="10000000-0000-4000-8000-000000000020"><Properties><Name>Main</Name><FormType>Managed</FormType></Properties></Form></MetaDataObject>"#).unwrap();
        std::fs::write(&body, r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><AutoCommandBar name="FormCommandBar" id="-1"/><ChildItems/><Attributes/><Commands/></Form>"#).unwrap();
        (workspace, format!("main:{kind}.Sales.Form.Main"), body)
    }

    #[test]
    fn canonical_form_check_accepts_external_report_processor_and_configuration() {
        for kind in [
            "ExternalDataProcessor",
            "ExternalReport",
            "Report",
            "Extension",
        ] {
            let (workspace, at, _) = form_check_workspace(kind);
            let before =
                unica_workspace::test_support::tree_snapshot(&workspace.path().join("src"));
            let viewed = call_dcs_check(workspace.path(), ToolIdentity::View, &at);
            assert!(viewed.ok, "{kind}: {viewed:?}");
            let checked = call_dcs_check(workspace.path(), ToolIdentity::Check, &at);
            assert!(checked.ok, "{kind}: {checked:?}");
            let data = checked.data.as_ref().unwrap();
            assert_eq!(data["status"], "passed", "{kind}: {checked:?}");
            assert_eq!(data["validators"], serde_json::json!(["form"]));
            assert_eq!(data["diagnosticCount"], 0, "{checked:?}");
            assert_eq!(
                before,
                unica_workspace::test_support::tree_snapshot(&workspace.path().join("src"))
            );
        }
    }

    #[test]
    fn canonical_form_check_reports_semantics_and_handler_signatures() {
        for kind in [
            "ExternalDataProcessor",
            "ExternalReport",
            "Report",
            "Extension",
        ] {
            let (workspace, at, body) = form_check_workspace(kind);
            let original = std::fs::read_to_string(&body).unwrap();
            std::fs::write(&body, original.replace("id=\"-1\"", "id=\"7\"")).unwrap();
            let invalid = call_dcs_check(workspace.path(), ToolIdentity::Check, &at);
            assert_eq!(
                invalid.data.as_ref().unwrap()["status"],
                "failed",
                "{invalid:?}"
            );
            assert!(
                invalid.data.as_ref().unwrap()["diagnostics"]
                    .to_string()
                    .contains("AutoCommandBar"),
                "{invalid:?}"
            );
            std::fs::write(
                &body,
                original.replace(
                    "<ChildItems/>",
                    "<Events><Event name=\"OnOpen\">ПриОткрытии</Event></Events><ChildItems/>",
                ),
            )
            .unwrap();
            let module = body.parent().unwrap().join("Form/Module.bsl");
            std::fs::create_dir_all(module.parent().unwrap()).unwrap();
            std::fs::write(
                module,
                "&НаКлиенте\nПроцедура ПриОткрытии(Отказ, Лишний, ЕщёОдин)\nКонецПроцедуры\n",
            )
            .unwrap();
            let checked = call_dcs_check(workspace.path(), ToolIdentity::Check, &at);
            assert!(checked.ok, "{checked:?}");
            let diagnostics = &checked.data.as_ref().unwrap()["diagnostics"];
            assert!(
                diagnostics
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["severity"] == "warning"
                        && d["message"].as_str().unwrap().contains("ПриОткрытии")),
                "{checked:?}"
            );
        }
    }

    #[test]
    fn canonical_form_check_refuses_missing_unregistered_and_mismatched_sources() {
        for case in ["missing", "unregistered", "mismatched"] {
            let (workspace, at, body) = form_check_workspace("ExternalDataProcessor");
            match case {
                "missing" => std::fs::remove_file(&body).unwrap(),
                "unregistered" => {
                    let owner = workspace.path().join("src/Sales.xml");
                    let text = std::fs::read_to_string(&owner).unwrap();
                    std::fs::write(owner, text.replace("<Form>Main</Form>", "")).unwrap();
                }
                "mismatched" => {
                    let descriptor = workspace.path().join("src/Sales/Forms/Main.xml");
                    let text = std::fs::read_to_string(&descriptor).unwrap();
                    std::fs::write(
                        descriptor,
                        text.replace("<Name>Main</Name>", "<Name>Other</Name>"),
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
            let checked = call_dcs_check(workspace.path(), ToolIdentity::Check, &at);
            assert!(!checked.ok, "{case}: {checked:?}");
            assert!(!checked.diagnostics.is_empty(), "{case}: {checked:?}");
        }
    }

    #[test]
    fn canonical_form_check_keeps_owner_format_warnings() {
        for kind in ["ExternalDataProcessor", "Report", "Extension"] {
            let (workspace, at, body) = form_check_workspace(kind);
            let wrapper = body
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .with_extension("xml");
            let owner = if kind == "ExternalDataProcessor" {
                workspace.path().join("src/Sales.xml")
            } else {
                workspace.path().join("src/Configuration.xml")
            };
            for path in [wrapper, owner] {
                let original = std::fs::read_to_string(&path).unwrap();
                for (version, code) in [
                    ("2.19", "formatMigrationAvailable"),
                    ("2.21", "platformVersionUnsupported"),
                ] {
                    std::fs::write(&path, original.replace("2.20", version)).unwrap();
                    let checked = call_dcs_check(workspace.path(), ToolIdentity::Check, &at);
                    assert_eq!(
                        checked.data.as_ref().unwrap()["status"],
                        "passed",
                        "{checked:?}"
                    );
                    assert!(
                        checked.data.as_ref().unwrap()["diagnostics"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|d| d["code"] == code),
                        "{checked:?}"
                    );
                }
                std::fs::write(path, original).unwrap();
            }
        }
    }

    #[test]
    fn canonical_form_check_supports_common_forms() {
        for kind in ["Report", "Extension"] {
            let (workspace, _, body) = form_check_workspace(kind);
            let source = workspace.path().join("src");
            let config = source.join("Configuration.xml");
            let text = std::fs::read_to_string(&config).unwrap();
            std::fs::write(
                config,
                text.replace(
                    "</ChildObjects>",
                    "<CommonForm>Shared</CommonForm></ChildObjects>",
                ),
            )
            .unwrap();
            let common = source.join("CommonForms/Shared/Ext/Form.xml");
            std::fs::create_dir_all(common.parent().unwrap()).unwrap();
            std::fs::copy(body, common).unwrap();
            std::fs::write(source.join("CommonForms/Shared.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonForm uuid="10000000-0000-4000-8000-000000000030"><Properties><Name>Shared</Name><FormType>Managed</FormType></Properties></CommonForm></MetaDataObject>"#).unwrap();
            let at = "main:CommonForm.Shared";
            assert!(call_dcs_check(workspace.path(), ToolIdentity::View, at).ok);
            let checked = call_dcs_check(workspace.path(), ToolIdentity::Check, at);
            assert_eq!(
                checked.data.as_ref().unwrap()["status"],
                "passed",
                "{checked:?}"
            );
            assert_eq!(
                checked.data.as_ref().unwrap()["diagnosticCount"],
                0,
                "{checked:?}"
            );
        }
    }

    #[test]
    fn canonical_form_check_refuses_linked_payload_and_module() {
        use unica_workspace::test_support::{
            create_file_link_fixture_for_test, FileLinkFixtureOutcome,
        };
        for module in [false, true] {
            let (workspace, at, body) = form_check_workspace("ExternalDataProcessor");
            let path = if module {
                let text = std::fs::read_to_string(&body).unwrap();
                std::fs::write(
                    &body,
                    text.replace(
                        "</Form>",
                        "<Events><Event name=\"OnOpen\">ПриОткрытии</Event></Events></Form>",
                    ),
                )
                .unwrap();
                let path = body.parent().unwrap().join("Form/Module.bsl");
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, "Процедура Тест()\nКонецПроцедуры").unwrap();
                path
            } else {
                body
            };
            let outside = tempfile::tempdir().unwrap();
            let external = outside.path().join("source");
            std::fs::rename(&path, &external).unwrap();
            if create_file_link_fixture_for_test(&external, &path).unwrap()
                != FileLinkFixtureOutcome::Created
            {
                unica_process::test_support::skip_without_host_capability(
                    unica_process::test_support::HostCapability::Symlink,
                    "Создание ссылки требует полномочий ОС",
                );
                return;
            }
            let before = std::fs::read(&external).unwrap();
            let checked = call_dcs_check(workspace.path(), ToolIdentity::Check, &at);
            assert!(!checked.ok, "{checked:?}");
            assert_eq!(before, std::fs::read(external).unwrap());
        }
    }

    #[test]
    fn canonical_dcs_check_accepts_external_report_processor_and_configuration() {
        for kind in ["ExternalReport", "ExternalDataProcessor", "Report"] {
            let (workspace, at, _) = dcs_check_workspace(kind);
            let before =
                unica_workspace::test_support::tree_snapshot(&workspace.path().join("src"));
            let viewed = call_dcs_check(workspace.path(), ToolIdentity::View, &at);
            assert!(viewed.ok, "{kind}: {viewed:?}");
            let checked = call_dcs_check(workspace.path(), ToolIdentity::Check, &at);
            assert!(checked.ok, "{kind}: {checked:?}");
            let data = checked.data.as_ref().unwrap();
            assert_eq!(data["status"], "passed", "{kind}: {checked:?}");
            assert_eq!(data["validators"], serde_json::json!(["dcs"]));
            assert_eq!(data["diagnosticCount"], 0);
            assert_eq!(
                before,
                unica_workspace::test_support::tree_snapshot(&workspace.path().join("src"))
            );
        }
    }

    #[test]
    fn canonical_template_read_warns_while_writes_keep_the_exact_owner_profile() {
        for kind in ["Report", "ExternalReport", "ExternalDataProcessor"] {
            for version in [Some("2.21"), Some("2.19"), None] {
                let (workspace, at, body) = dcs_check_workspace(kind);
                let wrapper = body
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .with_extension("xml");
                let original = std::fs::read_to_string(&wrapper).unwrap();
                let runtime = bootstrap_runtime();
                let call = |tool, args| {
                    direct_v5(
                        &runtime,
                        InvocationRequest::new(
                            tool,
                            args,
                            std::fs::canonicalize(workspace.path())
                                .unwrap()
                                .to_string_lossy(),
                            7_000,
                        )
                        .unwrap(),
                    )
                    .unwrap()
                };
                let arguments = serde_json::json!({
                    "at": format!("{at}.DataSet.НаборДанных1"),
                    "ops": [{"op": "field.add", "args": {"items": [{"dataPath": "Other", "title": "Other"}]}}]
                });
                let preview = call(ToolIdentity::Apply, arguments.clone());
                // External source sets have read/check support; the existing
                // DCS write surface is restricted to configuration/extension.
                let token = if kind == "Report" {
                    assert!(preview.ok, "supported owner: {preview:?}");
                    Some(preview.data.as_ref().unwrap()["executionToken"].clone())
                } else {
                    assert!(!preview.ok, "{preview:?}");
                    assert_eq!(preview.diagnostics[0]["code"], "provider_unavailable");
                    None
                };
                std::fs::write(
                    &wrapper,
                    original.replace(
                        "version=\"2.20\"",
                        &version
                            .map(|v| format!("version=\"{v}\""))
                            .unwrap_or_default(),
                    ),
                )
                .unwrap();
                let before =
                    unica_workspace::test_support::tree_snapshot(&workspace.path().join("src"));
                let viewed = call(ToolIdentity::View, serde_json::json!({"at": at}));
                assert!(viewed.ok, "{kind} {version:?}: {viewed:?}");
                let checked = call(ToolIdentity::Check, serde_json::json!({"at": at}));
                assert!(checked.ok, "{kind} {version:?}: {checked:?}");
                let expected = if version == Some("2.21") {
                    "platformVersionUnsupported"
                } else {
                    "formatMigrationAvailable"
                };
                assert!(
                    checked.data.as_ref().unwrap()["diagnostics"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|d| d["code"] == expected),
                    "{checked:?}"
                );
                let refused = call(ToolIdentity::Apply, arguments);
                assert!(!refused.ok, "{refused:?}");
                assert_eq!(
                    refused.diagnostics[0]["code"],
                    if kind == "Report" {
                        "invalid_source"
                    } else {
                        "provider_unavailable"
                    },
                    "{refused:?}"
                );
                assert!(refused
                    .data
                    .as_ref()
                    .and_then(|d| d.get("executionToken"))
                    .is_none());
                if let Some(token) = token {
                    let stale = call(
                        ToolIdentity::Apply,
                        serde_json::json!({"executionToken": token}),
                    );
                    assert!(!stale.ok, "{stale:?}");
                }
                assert_eq!(
                    before,
                    unica_workspace::test_support::tree_snapshot(&workspace.path().join("src"))
                );
            }
        }
    }

    #[test]
    fn canonical_dcs_check_external_semantics_and_owner_isolation() {
        for kind in ["ExternalReport", "ExternalDataProcessor"] {
            let (workspace, at, body) = dcs_check_workspace(kind);
            let valid = std::fs::read_to_string(&body).unwrap();
            let invalid = valid.replace("<dcsset:order>",
                "<dcsset:dataParameters><dcscor:item xsi:type=\"dcsset:SettingsParameterValue\"><dcscor:parameter/></dcscor:item></dcsset:dataParameters><dcsset:order>");
            assert_ne!(valid, invalid);
            std::fs::write(&body, invalid).unwrap();
            // A second owner has the same template name and valid content.
            let source = workspace.path().join("src");
            std::fs::create_dir_all(source.join("Other/Templates/Dcs/Ext")).unwrap();
            std::fs::write(
                source.join("Other.xml"),
                std::fs::read_to_string(source.join("Sales.xml"))
                    .unwrap()
                    .replace("Sales", "Other"),
            )
            .unwrap();
            std::fs::copy(
                source.join("Sales/Templates/Dcs.xml"),
                source.join("Other/Templates/Dcs.xml"),
            )
            .unwrap();
            std::fs::write(source.join("Other/Templates/Dcs/Ext/Template.xml"), valid).unwrap();
            let before = unica_workspace::test_support::tree_snapshot(&source);
            assert!(call_dcs_check(workspace.path(), ToolIdentity::View, &at).ok);
            let checked = call_dcs_check(workspace.path(), ToolIdentity::Check, &at);
            assert!(checked.ok, "{checked:?}");
            let data = checked.data.as_ref().unwrap();
            assert_eq!(data["status"], "failed", "{checked:?}");
            assert!(
                data["diagnostics"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|item| item["validator"] == "dcs"
                        && item["message"]
                            .as_str()
                            .is_some_and(|text| text
                                .contains("SettingsParameterValue has empty parameter name"))),
                "{checked:?}"
            );
            let sibling = call_dcs_check(
                workspace.path(),
                ToolIdentity::Check,
                &at.replace(".Sales.", ".Other."),
            );
            assert_eq!(
                sibling.data.as_ref().unwrap()["status"],
                "passed",
                "{sibling:?}"
            );
            assert_eq!(
                before,
                unica_workspace::test_support::tree_snapshot(&source)
            );
        }
    }

    #[test]
    fn canonical_dcs_check_refuses_unregistered_or_mismatched_external_template() {
        for case in ["unregistered", "wrong-name", "wrong-kind", "missing"] {
            let (workspace, at, body) = dcs_check_workspace("ExternalReport");
            let source = workspace.path().join("src");
            let descriptor = source.join("Sales/Templates/Dcs.xml");
            match case {
                "unregistered" => {
                    let owner = source.join("Sales.xml");
                    std::fs::write(
                        &owner,
                        std::fs::read_to_string(&owner)
                            .unwrap()
                            .replace("<Template>Dcs</Template>", ""),
                    )
                    .unwrap();
                }
                "wrong-name" => std::fs::write(
                    &descriptor,
                    std::fs::read_to_string(&descriptor)
                        .unwrap()
                        .replace("<Name>Dcs</Name>", "<Name>Other</Name>"),
                )
                .unwrap(),
                "wrong-kind" => std::fs::write(
                    &descriptor,
                    std::fs::read_to_string(&descriptor)
                        .unwrap()
                        .replace("<Template ", "<Form ")
                        .replace("</Template>", "</Form>"),
                )
                .unwrap(),
                "missing" => std::fs::remove_file(body).unwrap(),
                _ => unreachable!(),
            }
            let before = unica_workspace::test_support::tree_snapshot(&source);
            let checked = call_dcs_check(workspace.path(), ToolIdentity::Check, &at);
            assert!(!checked.ok, "{case}: {checked:?}");
            assert_eq!(
                checked.diagnostics[0]["code"],
                if case == "unregistered" {
                    "not_found"
                } else {
                    "provider_unavailable"
                },
                "{case}: {checked:?}"
            );
            assert_eq!(
                before,
                unica_workspace::test_support::tree_snapshot(&source)
            );
        }
    }

    #[test]
    fn canonical_dcs_check_refuses_linked_external_payload() {
        use unica_workspace::test_support::{
            create_file_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let (workspace, at, body) = dcs_check_workspace("ExternalReport");
        let outside = tempfile::tempdir().unwrap();
        let external_body = outside.path().join("Template.xml");
        std::fs::rename(&body, &external_body).unwrap();
        let outcome = create_file_link_fixture_for_test(&external_body, &body)
            .expect("unexpected file-link creation error must fail the fixture test");
        if outcome != FileLinkFixtureOutcome::Created {
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::Symlink,
                "creating a link fixture needs the symlink privilege",
            );
            return;
        }
        let before = std::fs::read(&external_body).unwrap();
        let checked = call_dcs_check(workspace.path(), ToolIdentity::Check, &at);
        assert!(!checked.ok, "{checked:?}");
        assert_eq!(
            checked.diagnostics[0]["code"], "provider_unavailable",
            "{checked:?}"
        );
        assert_eq!(before, std::fs::read(&external_body).unwrap());
        assert!(std::fs::symlink_metadata(body)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn canonical_external_named_owner_accepts_large_inventory_and_descriptor() {
        for kind in ["ExternalDataProcessor", "ExternalReport"] {
            let (workspace, _, _) = dcs_check_workspace(kind);
            let descriptor = workspace.path().join("src/Sales.xml");
            let original = std::fs::read_to_string(&descriptor).unwrap();
            let large = original.replace(
                "</MetaDataObject>",
                &format!("<!--{}--></MetaDataObject>", "x".repeat(9 * 1024 * 1024)),
            );
            std::fs::write(&descriptor, &large).unwrap();
            let at = format!("main:{kind}.Sales");
            let result = call_dcs_check(workspace.path(), ToolIdentity::View, &at);
            assert!(
                result.ok,
                "actual canonical actor named owner refused beyond former8MiB: {result:?}"
            );
            let node = result.data.unwrap();
            assert_eq!(node["at"], at);
            assert_eq!(node["kind"], kind);
            assert_eq!(node["title"], "Sales");
            assert_eq!(std::fs::read_to_string(descriptor).unwrap(), large);
        }
    }

    #[test]
    fn canonical_external_inventory_reports_all_257_owners_and_reaches_the_last() {
        let (workspace, _, _) = dcs_check_workspace("ExternalDataProcessor");
        // Existing fixture Sales.xml + Sales directory =2 immediate members.
        // Add256 actual independent owners; useful count257 exceeds former256.
        for index in 0..256 {
            let name = format!("Owner{index:03}");
            let descriptor = format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><ExternalDataProcessor uuid="10000000-0000-4000-8000-{index:012}"><Properties><Name>{name}</Name></Properties><ChildObjects/></ExternalDataProcessor></MetaDataObject>"#
            );
            std::fs::write(workspace.path().join(format!("src/{name}.xml")), descriptor).unwrap();
        }
        let root = call_dcs_check(workspace.path(), ToolIdentity::View, "main:Configuration");
        assert!(
            root.ok,
            "actual canonical actor inventory refused former count limit: {root:?}"
        );
        let root = root.data.unwrap();
        let branch = root["branches"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["at"] == "main:ExternalDataProcessor")
            .unwrap();
        assert_eq!(branch["count"], 257);
        let named = call_dcs_check(
            workspace.path(),
            ToolIdentity::View,
            "main:ExternalDataProcessor.Owner255",
        );
        assert!(named.ok, "late actual canonical owner missing: {named:?}");
        assert_eq!(
            named.data.as_ref().unwrap()["at"],
            "main:ExternalDataProcessor.Owner255"
        );
        assert_eq!(named.data.as_ref().unwrap()["title"], "Owner255");
    }
    fn bootstrap_runtime() -> V5CanonicalInvocationRuntime {
        V5CanonicalInvocationRuntime::new(canonical_v13_service(), Arc::new(TokioClock))
    }

    /// One canonical Direct call on the shared v5 runtime: bind, prepare and
    /// execute in the calling thread, exactly the path the daemon takes before
    /// its cutoff. A known-long class is a Task, never a Direct outcome.
    pub(super) fn direct_v5(
        runtime: &V5CanonicalInvocationRuntime,
        request: InvocationRequest,
    ) -> Result<DomainResult, V5CanonicalPrepareError> {
        let bound = match runtime.bind(request) {
            Ok(bound) => bound,
            Err(V5CanonicalPrepareError::Direct(result))
            | Err(V5CanonicalPrepareError::Rejected(result)) => return Ok(*result),
            Err(error) => return Err(error),
        };
        let prepared = match bound.prepare() {
            Ok(prepared) => prepared,
            Err(result) => return Ok(*result),
        };
        assert!(
            !matches!(prepared.execution_class(), ExecutionClass::KnownLong(_)),
            "a known-long class is handed off to a Task, not answered directly"
        );
        Ok(prepared
            .execute(CancellationToken::new())
            .unwrap_or_else(|_| failed_domain_result("canonical execution failed")))
    }

    fn submit_bootstrap(
        runtime: &V5CanonicalInvocationRuntime,
        workspace: &std::path::Path,
        arguments: serde_json::Value,
    ) -> DomainResult {
        submit_canonical(runtime, workspace, ToolIdentity::View, arguments)
    }

    /// Вердикт по корню: `check {}` отвечает до допуска наборов.
    fn submit_root_verdict(
        runtime: &V5CanonicalInvocationRuntime,
        workspace: &std::path::Path,
    ) -> DomainResult {
        submit_canonical(
            runtime,
            workspace,
            ToolIdentity::Check,
            serde_json::json!({}),
        )
    }

    fn submit_canonical(
        runtime: &V5CanonicalInvocationRuntime,
        workspace: &std::path::Path,
        tool: ToolIdentity,
        arguments: serde_json::Value,
    ) -> DomainResult {
        let request = InvocationRequest::new(
            tool,
            arguments,
            std::fs::canonicalize(workspace).unwrap().to_string_lossy(),
            7_000,
        )
        .unwrap();
        direct_v5(runtime, request).expect("a pre-admission answer must finish directly")
    }

    fn retained_payload_workspace(member_count: usize) -> (tempfile::TempDir, Vec<String>) {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        std::fs::create_dir_all(source.join("CommonModules")).unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let names = (0..member_count)
            .map(|index| format!("Module{index:03}{}", "X".repeat(56)))
            .collect::<Vec<_>>();
        let registered = names
            .iter()
            .map(|name| format!("<CommonModule>{name}</CommonModule>"))
            .collect::<String>();
        std::fs::write(
            source.join("Configuration.xml"),
            format!(r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>RetainedPayload</Name></Properties><ChildObjects>{registered}</ChildObjects></Configuration></MetaDataObject>"#),
        )
        .unwrap();
        for (index, name) in names.iter().enumerate() {
            std::fs::write(
                source.join(format!("CommonModules/{name}.xml")),
                format!(r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule uuid="10000000-0000-4000-8000-{index:012}"><Properties><Name>{name}</Name><Synonym/><Comment/><Global>false</Global><ClientManagedApplication>false</ClientManagedApplication><Server>true</Server><ExternalConnection>false</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>false</ServerCall><Privileged>false</Privileged><ReturnValuesReuse>DontUse</ReturnValuesReuse></Properties></CommonModule></MetaDataObject>"#),
            )
            .unwrap();
        }
        (workspace, names)
    }

    fn retained_payload_page(
        runtime: &V5CanonicalInvocationRuntime,
        workspace: &std::path::Path,
        at: &str,
        cursor: Option<&str>,
    ) -> DomainResult {
        let mut arguments = serde_json::json!({"at":at,"limit":1});
        if let Some(cursor) = cursor {
            arguments["cursor"] = serde_json::json!(cursor);
        }
        let page = submit_canonical(runtime, workspace, ToolIdentity::View, arguments);
        assert!(page.ok, "canonical retained page failed: {page:?}");
        assert!(
            page.rev.is_none(),
            "canonical view must keep its revision private"
        );
        page
    }

    /// Save the actual input cursor with the complete typed response and successor.
    fn retain_payload_page_chain(
        runtime: &V5CanonicalInvocationRuntime,
        workspace: &std::path::Path,
        at: &str,
        first: &DomainResult,
    ) -> Vec<(String, DomainResult)> {
        let mut cursor = first.cursor.clone();
        let mut saved = Vec::new();
        while let Some(input) = cursor {
            let page = retained_payload_page(runtime, workspace, at, Some(&input));
            cursor = page.cursor.clone();
            saved.push((input, page));
        }
        saved
    }

    fn replay_retained_payload_pages(
        runtime: &V5CanonicalInvocationRuntime,
        workspace: &std::path::Path,
        at: &str,
        saved: &[(String, DomainResult)],
    ) {
        for (cursor, expected) in saved {
            assert_eq!(
                retained_payload_page(runtime, workspace, at, Some(cursor)),
                *expected,
                "saved typed page and its exact successor must survive later admissions"
            );
        }
    }

    #[test]
    fn canonical_130_fresh_leases_share_one_payload_without_skipping_full_reads() {
        use crate::infrastructure::daemon::v13_service::{
            CanonicalV13ReadService, ViewCapabilityProbeForTest,
        };
        use crate::infrastructure::v13_read::CompletedViewProjectionProbeForTest;
        use std::collections::BTreeSet;
        use unica_navigation::result_store::ViewCollectionSnapshot;

        const ADMISSIONS: usize = 130;
        const AT: &str = "main:CommonModule";
        let (workspace, _) = retained_payload_workspace(192);

        // This separate calibration read measures the actual source projection.
        // The pressure loop below keeps one new service and runtime for all calls.
        let calibration_service = Arc::new(CanonicalV13ReadService::default());
        let calibration_runtime =
            V5CanonicalInvocationRuntime::new(calibration_service.clone(), Arc::new(TokioClock));
        let calibration_page =
            retained_payload_page(&calibration_runtime, workspace.path(), AT, None);
        assert!(calibration_page.cursor.is_some());
        let one = calibration_service.retained_view_usage_for_test();
        assert_eq!(one.records, 1);
        assert_eq!(one.unique_payloads, 1);
        assert!(
            one.payload_bytes > 16 * one.record_bytes,
            "the payload must dominate admission metadata: {one:?}"
        );
        let index_slot = std::mem::size_of::<Arc<ViewCollectionSnapshot>>();
        let fixed_bytes = one.retained_bytes
            - one.record_bytes
            - one.payload_bytes
            - one.index_capacity * index_slot;
        let mut reserved_index: Vec<Arc<ViewCollectionSnapshot>> = Vec::new();
        reserved_index.try_reserve_exact(ADMISSIONS).unwrap();
        let index_capacity = reserved_index.capacity();
        let budget = fixed_bytes
            + one.payload_bytes
            + ADMISSIONS * one.record_bytes
            + index_capacity * index_slot;
        assert!(budget < ADMISSIONS * one.payload_bytes);
        drop(reserved_index);
        drop(calibration_runtime);
        drop(calibration_service);

        let service = Arc::new(CanonicalV13ReadService::with_view_cursor_budget_for_test(
            budget,
        ));
        let runtime = V5CanonicalInvocationRuntime::new(service.clone(), Arc::new(TokioClock));
        let capabilities = ViewCapabilityProbeForTest::start(AT);
        let projections = CompletedViewProjectionProbeForTest::start(AT);
        let mut tokens = BTreeSet::new();
        let mut saved = Vec::new();
        for admission in 0..ADMISSIONS {
            let first = retained_payload_page(&runtime, workspace.path(), AT, None);
            assert_eq!(
                first.data, calibration_page.data,
                "fresh admission {admission}"
            );
            let input = first
                .cursor
                .as_ref()
                .expect("the collection has multiple pages");
            assert!(
                tokens.insert(input.clone()),
                "each real lease keeps a distinct record/token"
            );
            let second = retained_payload_page(&runtime, workspace.path(), AT, Some(input));
            let successor = second
                .cursor
                .as_ref()
                .expect("the second page has a successor");
            let third = retained_payload_page(&runtime, workspace.path(), AT, Some(successor));
            saved.push((input.clone(), second.clone()));
            saved.push((successor.clone(), third));
            assert_eq!(projections.revisions().len(), admission + 1);
        }
        let issued = capabilities.revisions();
        assert_eq!(issued.len(), ADMISSIONS);
        assert_eq!(issued.iter().collect::<BTreeSet<_>>().len(), ADMISSIONS);
        for revision in &issued {
            assert_eq!(
                uuid::Uuid::parse_str(revision).unwrap().get_version_num(),
                4
            );
        }
        assert_eq!(
            projections.revisions(),
            issued,
            "every admission finished its own full projection"
        );
        assert_eq!(service.retained_view_binding_ids_for_test(), issued);
        let usage = service.retained_view_usage_for_test();
        assert_eq!(usage.records, ADMISSIONS);
        assert_eq!(
            usage.unique_payloads, 1,
            "all distinct records own the same full payload"
        );
        let payloads = service.retained_view_payload_identities_for_test();
        assert_eq!(payloads.len(), ADMISSIONS);
        assert_eq!(
            payloads.iter().collect::<BTreeSet<_>>().len(),
            1,
            "all real canonical records must point at the same payload allocation"
        );
        assert_eq!(usage.payload_bytes, one.payload_bytes);
        assert_eq!(usage.record_bytes, ADMISSIONS * one.record_bytes);
        assert!(usage.index_capacity <= index_capacity);
        assert_eq!(
            usage.retained_bytes,
            fixed_bytes
                + usage.payload_bytes
                + usage.record_bytes
                + usage.index_capacity * index_slot
        );
        assert!(usage.retained_bytes <= budget);

        replay_retained_payload_pages(&runtime, workspace.path(), AT, &saved);
        assert_eq!(
            projections.revisions(),
            issued,
            "cursor replay cannot read the source again"
        );
        assert_eq!(
            service.retained_view_usage_for_test(),
            usage,
            "replay retains no new state"
        );
        std::fs::remove_file(workspace.path().join("src/Configuration.xml")).unwrap();
        replay_retained_payload_pages(&runtime, workspace.path(), AT, &saved);
        assert_eq!(
            projections.revisions(),
            issued,
            "saved pages survive deletion without source reads"
        );
        assert_eq!(service.retained_view_usage_for_test(), usage);
    }

    #[test]
    fn canonical_change_beyond_first_page_keeps_old_payload_and_allocates_a_new_full_payload() {
        use crate::infrastructure::daemon::v13_service::CanonicalV13ReadService;
        use crate::infrastructure::v13_read::CompletedViewProjectionProbeForTest;

        const AT: &str = "main:CommonModule";
        let (workspace, names) = retained_payload_workspace(4);
        let service = Arc::new(CanonicalV13ReadService::default());
        let runtime = V5CanonicalInvocationRuntime::new(service.clone(), Arc::new(TokioClock));
        let projections = CompletedViewProjectionProbeForTest::start(AT);
        let first = retained_payload_page(&runtime, workspace.path(), AT, None);
        let saved = retain_payload_page_chain(&runtime, workspace.path(), AT, &first);
        assert_eq!(saved.len(), 3, "save the complete original chain");
        let last_name = names.last().unwrap();
        let replacement = format!("{last_name}Changed");
        let descriptor = workspace
            .path()
            .join(format!("src/CommonModules/{last_name}.xml"));
        let changed_descriptor = workspace
            .path()
            .join(format!("src/CommonModules/{replacement}.xml"));
        let source = std::fs::read_to_string(&descriptor)
            .unwrap()
            .replace(last_name, &replacement);
        std::fs::write(&changed_descriptor, source).unwrap();
        std::fs::remove_file(&descriptor).unwrap();
        let configuration = workspace.path().join("src/Configuration.xml");
        let source = std::fs::read_to_string(&configuration)
            .unwrap()
            .replace(last_name, &replacement);
        std::fs::write(&configuration, source).unwrap();

        let changed = retained_payload_page(&runtime, workspace.path(), AT, None);
        assert_eq!(
            changed.data, first.data,
            "the real change is outside the first page"
        );
        assert_ne!(changed.cursor, first.cursor);
        let changed_pages = retain_payload_page_chain(&runtime, workspace.path(), AT, &changed);
        assert_eq!(changed_pages.len(), saved.len());
        assert_ne!(
            changed_pages.last().unwrap().1.data,
            saved.last().unwrap().1.data
        );
        assert_eq!(
            changed_pages.last().unwrap().1.data.as_ref().unwrap()["items"][0]["title"],
            replacement
        );
        let usage = service.retained_view_usage_for_test();
        assert_eq!(usage.records, 2);
        assert_eq!(
            usage.unique_payloads, 2,
            "equality must cover all items beyond the first page"
        );
        let payloads = service.retained_view_payload_identities_for_test();
        assert_eq!(payloads.len(), 2);
        assert_ne!(payloads[0], payloads[1]);
        let reads = projections.revisions();
        assert_eq!(reads.len(), 2);
        replay_retained_payload_pages(&runtime, workspace.path(), AT, &saved);
        std::fs::remove_file(changed_descriptor).unwrap();
        std::fs::remove_file(configuration).unwrap();
        replay_retained_payload_pages(&runtime, workspace.path(), AT, &saved);
        replay_retained_payload_pages(&runtime, workspace.path(), AT, &changed_pages);
        assert_eq!(projections.revisions(), reads);
        assert_eq!(service.retained_view_usage_for_test(), usage);
    }

    #[test]
    fn canonical_changed_collection_node_with_equal_items_keeps_a_separate_full_payload() {
        use crate::infrastructure::daemon::v13_service::CanonicalV13ReadService;
        use crate::infrastructure::v13_read::CompletedViewProjectionProbeForTest;

        const AT: &str = "main:Subsystem.Sales.Relation";
        let (workspace, names) = retained_payload_workspace(3);
        let configuration = workspace.path().join("src/Configuration.xml");
        let source = std::fs::read_to_string(&configuration).unwrap().replace(
            "</ChildObjects>",
            "<Subsystem>Sales</Subsystem></ChildObjects>",
        );
        std::fs::write(&configuration, source).unwrap();
        let content = names
            .iter()
            .map(|name| {
                format!(r#"<xr:Item xsi:type="xr:MDObjectRef">CommonModule.{name}</xr:Item>"#)
            })
            .collect::<String>();
        std::fs::create_dir_all(workspace.path().join("src/Subsystems")).unwrap();
        let descriptor = workspace.path().join("src/Subsystems/Sales.xml");
        let original = format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20"><Subsystem uuid="55555555-5555-4555-8555-555555555555"><Properties><Name>Sales</Name><Content>{content}<xr:Item xsi:type="xr:MDObjectRef">11111111-1111-4111-8111-111111111111</xr:Item></Content></Properties><ChildObjects/></Subsystem></MetaDataObject>"#
        );
        std::fs::write(&descriptor, &original).unwrap();
        let service = Arc::new(CanonicalV13ReadService::default());
        let runtime = V5CanonicalInvocationRuntime::new(service.clone(), Arc::new(TokioClock));
        let projections = CompletedViewProjectionProbeForTest::start(AT);
        let first = retained_payload_page(&runtime, workspace.path(), AT, None);
        let saved = retain_payload_page_chain(&runtime, workspace.path(), AT, &first);
        assert_eq!(saved.len(), 2);
        std::fs::write(
            &descriptor,
            original.replace(
                "11111111-1111-4111-8111-111111111111",
                "22222222-2222-4222-8222-222222222222",
            ),
        )
        .unwrap();
        let changed = retained_payload_page(&runtime, workspace.path(), AT, None);
        let changed_pages = retain_payload_page_chain(&runtime, workspace.path(), AT, &changed);
        assert_eq!(changed_pages.len(), saved.len());
        assert_eq!(
            first.data.as_ref().unwrap()["items"],
            changed.data.as_ref().unwrap()["items"]
        );
        assert_ne!(
            first.data.as_ref().unwrap()["limits"],
            changed.data.as_ref().unwrap()["limits"]
        );
        for ((_, old), (_, new)) in saved.iter().zip(&changed_pages) {
            assert_eq!(
                old.data.as_ref().unwrap()["items"],
                new.data.as_ref().unwrap()["items"],
                "all ordered items are equal; the real source change affects the collection node"
            );
        }
        let usage = service.retained_view_usage_for_test();
        assert_eq!(usage.records, 2);
        assert_eq!(
            usage.unique_payloads, 2,
            "a changed node must not share an equal-items payload"
        );
        let payloads = service.retained_view_payload_identities_for_test();
        assert_eq!(payloads.len(), 2);
        assert_ne!(payloads[0], payloads[1]);
        let reads = projections.revisions();
        assert_eq!(reads.len(), 2);
        replay_retained_payload_pages(&runtime, workspace.path(), AT, &saved);
        std::fs::remove_file(descriptor).unwrap();
        replay_retained_payload_pages(&runtime, workspace.path(), AT, &saved);
        replay_retained_payload_pages(&runtime, workspace.path(), AT, &changed_pages);
        assert_eq!(projections.revisions(), reads);
        assert_eq!(service.retained_view_usage_for_test(), usage);
    }

    const LIVE_V5_IDLE_GRACE: Duration = Duration::from_millis(400);
    const LIVE_V5_OPERATION_TIMEOUT: Duration = Duration::from_secs(20);
    const LIVE_V5_WAIT_MS: u64 = 7_000;

    fn v5_tool(tool: ToolIdentity) -> V5ToolIdentity {
        match tool {
            ToolIdentity::View => V5ToolIdentity::View,
            ToolIdentity::Apply => V5ToolIdentity::Apply,
            ToolIdentity::Resolve => V5ToolIdentity::Resolve,
            ToolIdentity::Search => V5ToolIdentity::Search,
            ToolIdentity::Check => V5ToolIdentity::Check,
            ToolIdentity::Diff => V5ToolIdentity::Diff,
            ToolIdentity::Run => V5ToolIdentity::Run,
            ToolIdentity::Docs => V5ToolIdentity::Docs,
        }
    }

    /// What one submission over the wire came back as.
    #[derive(Debug)]
    pub(crate) enum V5Submission {
        Direct(Box<DomainResult>),
        Task(unica_execution_contract::invocation::TaskId),
    }

    /// A live v5 daemon in a thread over a canonical runtime the test holds:
    /// Task work goes through the wire exactly as production submits it, and
    /// the test still observes the actor and capability registries.
    pub(crate) struct LiveV5Daemon {
        _state: tempfile::TempDir,
        state_root: std::path::PathBuf,
        canonical: Arc<V5CanonicalInvocationRuntime>,
        thread: Option<std::thread::JoinHandle<Result<(), String>>>,
        owned_stop: Option<Arc<AtomicBool>>,
    }

    impl LiveV5Daemon {
        pub(crate) fn start(service: Arc<dyn CanonicalInvocationService>) -> Self {
            let canonical = Arc::new(V5CanonicalInvocationRuntime::new(
                Arc::clone(&service),
                Arc::new(TokioClock),
            ));
            Self::over(canonical, service)
        }

        pub(crate) fn start_with_owned_stop(service: Arc<dyn CanonicalInvocationService>) -> Self {
            let canonical = Arc::new(V5CanonicalInvocationRuntime::new(
                Arc::clone(&service),
                Arc::new(TokioClock),
            ));
            Self::over_with_owned_stop(canonical, service, Some(Arc::new(AtomicBool::new(false))))
        }

        pub(crate) fn start_with_delivery_downloader(
            service: Arc<dyn CanonicalInvocationService>,
            downloader: Arc<dyn unica_artifact_delivery::Downloader>,
        ) -> Self {
            let canonical = Arc::new(
                V5CanonicalInvocationRuntime::new(Arc::clone(&service), Arc::new(TokioClock))
                    .with_delivery_downloader_for_test(downloader),
            );
            Self::over(canonical, service)
        }

        /// The same live daemon with an LSP service the test controls.
        pub(crate) fn start_with_lsp_host(
            service: Arc<dyn CanonicalInvocationService>,
            lsp_host: Arc<dyn unica_daemon::lsp_lane::LspHost>,
        ) -> Self {
            let canonical = Arc::new(V5CanonicalInvocationRuntime::new(
                Arc::clone(&service),
                Arc::new(TokioClock),
            ));
            Self::over_configured(
                canonical,
                service,
                Some(Arc::new(AtomicBool::new(false))),
                Some(lsp_host),
            )
        }

        fn over(
            canonical: Arc<V5CanonicalInvocationRuntime>,
            service: Arc<dyn CanonicalInvocationService>,
        ) -> Self {
            Self::over_with_owned_stop(canonical, service, None)
        }

        fn over_with_owned_stop(
            canonical: Arc<V5CanonicalInvocationRuntime>,
            service: Arc<dyn CanonicalInvocationService>,
            owned_stop: Option<Arc<AtomicBool>>,
        ) -> Self {
            Self::over_configured(canonical, service, owned_stop, None)
        }

        fn over_configured(
            canonical: Arc<V5CanonicalInvocationRuntime>,
            service: Arc<dyn CanonicalInvocationService>,
            owned_stop: Option<Arc<AtomicBool>>,
            lsp_host: Option<Arc<dyn unica_daemon::lsp_lane::LspHost>>,
        ) -> Self {
            let state = tempfile::tempdir().unwrap();
            let state_root = std::fs::canonicalize(state.path()).unwrap();
            let mut config = DaemonServerConfig::new(
                state_root.clone(),
                CoreIdentity::production_v5(),
                LIVE_V5_IDLE_GRACE,
            )
            .with_invocation_service(service)
            .with_canonical_runtime_for_test(Arc::clone(&canonical));
            if let Some(lsp_host) = lsp_host {
                config = config.with_lsp_host_for_test(lsp_host);
            }
            let thread = Self::spawn(config, owned_stop.clone());
            Self {
                _state: state,
                state_root,
                canonical,
                thread: Some(thread),
                owned_stop,
            }
        }

        fn spawn(
            config: DaemonServerConfig,
            owned_stop: Option<Arc<AtomicBool>>,
        ) -> std::thread::JoinHandle<Result<(), String>> {
            std::thread::spawn(move || match owned_stop {
                Some(stop) => unica_daemon::transport::run_daemon_until_for_test(
                    config.into_transport_config(),
                    stop,
                ),
                None => unica_daemon::transport::run_daemon(config.into_transport_config()),
            })
        }

        fn runtime(&self) -> &V5CanonicalInvocationRuntime {
            &self.canonical
        }

        /// The daemon's private state directory: receipts, tasks and links.
        pub(crate) fn state_directory(&self) -> std::path::PathBuf {
            DaemonStateDirectory::path_for(&self.state_root, &CoreIdentity::production_v5())
        }

        /// The owner lease that keeps the daemon alive for the test.
        pub(crate) fn owner(&self) -> V5DaemonProcessOwner {
            let identity = CoreIdentity::production_v5();
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let state = DaemonStateDirectory::open(&self.state_root, &identity).unwrap();
                if let Some(record) = state.read_v5_endpoint_record().unwrap() {
                    return V5DaemonProcessOwner::connect_before(record, deadline)
                        .expect("connect the live v5 daemon");
                }
                assert!(Instant::now() < deadline, "v5 endpoint was not published");
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        pub(crate) fn submit(
            &self,
            owner: &V5DaemonProcessOwner,
            request: &InvocationRequest,
        ) -> V5Submission {
            let invocation = V5InvocationRequest::new(
                unica_execution_contract::invocation::InvocationId::new(),
                unica_execution_contract::invocation::TaskId::new(),
                v5_tool(request.tool()),
                request.arguments().clone(),
                unica_bootstrap::ResolvedWorkspace {
                    directory: request.workspace_hint().to_owned(),
                    origin: request
                        .workspace_origin()
                        .cloned()
                        .unwrap_or(unica_bootstrap::WorkspaceOrigin::LaunchCwd { roots: None }),
                },
                request.response_budget_ms(),
            )
            .unwrap();
            let deadline = Instant::now() + LIVE_V5_OPERATION_TIMEOUT;
            let mut peer = owner.connect_peer_before(deadline).expect("peer session");
            match peer
                .submit_invocation_before(invocation, deadline)
                .expect("submit over the wire")
            {
                V5ServerResponse::Invocation {
                    outcome: V5InvocationResponse::Direct { receipt },
                } => match receipt.terminal() {
                    ReceiptTerminalOutcome::Completed { result } => {
                        V5Submission::Direct(result.clone())
                    }
                    other => panic!("direct receipt without a completed result: {other:?}"),
                },
                V5ServerResponse::Invocation {
                    outcome: V5InvocationResponse::Task { snapshot },
                } => V5Submission::Task(snapshot.task_id()),
                V5ServerResponse::Error { code } => panic!("submission refused: {code:?}"),
                other => panic!("unexpected submit response {other:?}"),
            }
        }

        pub(crate) fn task_id(
            &self,
            owner: &V5DaemonProcessOwner,
            request: &InvocationRequest,
        ) -> unica_execution_contract::invocation::TaskId {
            match self.submit(owner, request) {
                V5Submission::Task(task_id) => task_id,
                other => panic!("expected durable task: {other:?}"),
            }
        }

        pub(crate) fn get(
            &self,
            owner: &V5DaemonProcessOwner,
            task_id: unica_execution_contract::invocation::TaskId,
        ) -> V5DaemonTaskSnapshot {
            let deadline = Instant::now() + LIVE_V5_OPERATION_TIMEOUT;
            let mut peer = owner.connect_peer_before(deadline).expect("peer session");
            peer.get_task_before(task_id, deadline).expect("get task")
        }

        pub(crate) fn cancel(
            &self,
            owner: &V5DaemonProcessOwner,
            task_id: unica_execution_contract::invocation::TaskId,
        ) -> V5DaemonTaskSnapshot {
            let deadline = Instant::now() + LIVE_V5_OPERATION_TIMEOUT;
            let mut peer = owner.connect_peer_before(deadline).expect("peer session");
            peer.cancel_task_before(task_id, deadline)
                .expect("cancel task")
        }

        /// Waits for a terminal snapshot within `total`, one bounded wire wait
        /// at a time.
        pub(crate) fn wait_terminal(
            &self,
            owner: &V5DaemonProcessOwner,
            task_id: unica_execution_contract::invocation::TaskId,
            total: Duration,
        ) -> V5DaemonTaskSnapshot {
            let settle_by = Instant::now() + total;
            loop {
                let deadline = Instant::now() + LIVE_V5_OPERATION_TIMEOUT;
                let mut peer = owner.connect_peer_before(deadline).expect("peer session");
                let snapshot = peer
                    .wait_task_before(task_id, LIVE_V5_WAIT_MS, deadline)
                    .expect("wait task");
                if matches!(
                    snapshot.status(),
                    InvocationStatus::Completed
                        | InvocationStatus::Failed
                        | InvocationStatus::Cancelled
                ) {
                    return snapshot;
                }
                assert!(
                    Instant::now() < settle_by,
                    "the Task did not settle in {total:?}: {snapshot:?}"
                );
            }
        }

        /// Ends the daemon: the owner lease goes first, then the idle grace.
        pub(crate) fn finish(mut self, owner: V5DaemonProcessOwner) {
            self.stop(owner);
        }

        /// Ends one life of the daemon; the state root stays for the next.
        pub(crate) fn stop(&mut self, owner: V5DaemonProcessOwner) {
            if let Some(stop) = &self.owned_stop {
                stop.store(true, Ordering::Release);
            }
            drop(owner);
            self.thread
                .take()
                .expect("a running daemon")
                .join()
                .expect("join the v5 daemon thread")
                .expect("the v5 daemon exits on its idle grace");
        }

        /// Starts the daemon again over the same state root, as a new process
        /// would: a fresh canonical runtime reads what the first life left durable.
        pub(crate) fn restart(&mut self, service: Arc<dyn CanonicalInvocationService>) {
            assert!(
                self.thread.is_none(),
                "the previous daemon life must be finished first"
            );
            let canonical = Arc::new(V5CanonicalInvocationRuntime::new(
                Arc::clone(&service),
                Arc::new(TokioClock),
            ));
            let config = DaemonServerConfig::new(
                self.state_root.clone(),
                CoreIdentity::production_v5(),
                LIVE_V5_IDLE_GRACE,
            )
            .with_invocation_service(service)
            .with_canonical_runtime_for_test(Arc::clone(&canonical));
            self.canonical = canonical;
            if let Some(stop) = &self.owned_stop {
                stop.store(false, Ordering::Release);
            }
            self.thread = Some(Self::spawn(config, self.owned_stop.clone()));
        }
    }

    #[test]
    fn infobase_export_is_task_admitted_without_platform_xml_source_sets() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\ninfobase:\n  connection: 'File=base'\n",
        )
        .unwrap();
        let runtime = bootstrap_runtime();
        let request = InvocationRequest::new(
            ToolIdentity::Run,
            serde_json::json!({
                "op": "infobase.dump",
                "args": {"output": "dist/base.dt"},
                "dryRun": true
            }),
            workspace.path().display().to_string(),
            7_000,
        )
        .unwrap();

        let bound = runtime.bind(request).expect("export admission");
        assert!(matches!(
            bound.0,
            ActorBoundCanonicalInvocationState::InfobaseExport { .. }
        ));
        let prepared = bound.prepare().expect("export preparation");
        assert!(matches!(
            prepared.execution_class(),
            ExecutionClass::KnownLong(KnownLongReason::ExternalProcess)
        ));
    }

    #[test]
    fn canonical_selected_engine_routes_request_delivery_before_provider_start() {
        const CHILD: &str = "UNICA_RUN_DELIVERY_TEST_CHILD";
        const ROOT: &str = "UNICA_RUN_DELIVERY_TEST_ROOT";
        if std::env::var_os(CHILD).is_some() {
            let root = std::path::PathBuf::from(std::env::var_os(ROOT).unwrap());
            let workspace = root.join("workspace");
            let runtime = bootstrap_runtime();
            let request = InvocationRequest::new(
                ToolIdentity::Run,
                serde_json::json!({"op":"extensions.list","args":{},"dryRun":true}),
                workspace.display().to_string(),
                7_000,
            )
            .unwrap();
            let result = runtime
                .bind(request)
                .unwrap()
                .prepare()
                .unwrap()
                .execute(CancellationToken::new())
                .unwrap();
            assert!(!result.ok, "{result:?}");
            assert_eq!(
                result.data.as_ref().unwrap()["delivery"]["artifact"],
                "v8-runner"
            );
            assert_eq!(
                result.data.as_ref().unwrap()["delivery"]["status"],
                "failed"
            );
            assert!(!serde_json::to_string(&result)
                .unwrap()
                .contains("bundled_tool_missing"));

            for (role, artifact) in [("symbol", "bsl-analyzer"), ("semantic", "rlm-tools-bsl")] {
                let result = submit_canonical(
                    &runtime,
                    &workspace,
                    ToolIdentity::Search,
                    serde_json::json!({"query":"Needle","role":role}),
                );
                assert!(!result.ok, "{role}: {result:?}");
                assert_eq!(
                    result.data.as_ref().unwrap()["delivery"]["artifact"],
                    artifact,
                    "{role}: {result:?}"
                );
            }
            // Ветвь графа — единственный вход в граф вызовов: недостающий
            // анализатор она заказывает этим же вызовом, и провал поставки
            // приходит её типизированным отказом. Обычное чтение метода
            // поставку не заказывает.
            let method = "main:CommonModule.Probe.Method.Ping";
            let plain = submit_canonical(
                &runtime,
                &workspace,
                ToolIdentity::View,
                serde_json::json!({"at": method}),
            );
            assert!(plain.ok, "{plain:?}");
            assert!(plain.data.as_ref().unwrap().get("delivery").is_none());
            for kind in ["Caller", "Callee"] {
                let branch = format!("{method}.{kind}");
                let result = submit_canonical(
                    &runtime,
                    &workspace,
                    ToolIdentity::View,
                    serde_json::json!({"at": branch}),
                );
                assert!(!result.ok, "{kind}: {result:?}");
                assert_eq!(result.at.as_deref(), Some(branch.as_str()));
                assert_eq!(
                    result.data.as_ref().unwrap()["delivery"],
                    serde_json::json!({
                        "artifact": "bsl-analyzer",
                        "status": "failed",
                        "failureClass": "configuration",
                    }),
                    "{kind}: {result:?}"
                );
                assert_eq!(
                    serde_json::to_value(&result).unwrap()["diagnostics"][0]["detailCode"],
                    "provider_absent",
                    "{kind}: {result:?}"
                );
            }
            // Ветвь метода без графа отказывает по адресу и поставку не
            // заказывает, хотя здесь заказ был бы возможен.
            for branch in [
                "main:HTTPService.Связь.URLTemplate.Шаблон.Method.POST.Caller",
                "main:CommonForm.Форма.Module.Form.Method.ПриОткрытии.Callee",
                "main:InformationRegister.Курсы.Module.RecordSet.Method.ПередЗаписью.Caller",
            ] {
                let result = submit_canonical(
                    &runtime,
                    &workspace,
                    ToolIdentity::View,
                    serde_json::json!({"at": branch}),
                );
                assert!(!result.ok, "a branch without a call graph is refused");
                assert_eq!(
                    serde_json::to_value(&result).unwrap()["diagnostics"][0]["code"],
                    "bad_value",
                    "a branch without a call graph is refused by its address"
                );
                assert!(
                    result
                        .data
                        .as_ref()
                        .and_then(|data| data.get("delivery"))
                        .is_none(),
                    "a branch without a call graph never orders the analyzer"
                );
            }
            for arguments in [
                serde_json::json!({"query":"Needle","ignoreCase":true}),
                serde_json::json!({"query":"Needle"}),
            ] {
                let result =
                    submit_canonical(&runtime, &workspace, ToolIdentity::Search, arguments);
                assert!(
                    result
                        .data
                        .as_ref()
                        .and_then(|data| data.get("delivery"))
                        .is_none(),
                    "literal search must not order an external engine: {result:?}"
                );
            }
            return;
        }

        // A one-test child owns its environment. Other Rust tests never see
        // the temporary package or its intentionally absent release manifest.
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let plugin = root.path().join("plugin");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(plugin.join("third-party")).unwrap();
        std::fs::create_dir_all(workspace.join("src")).unwrap();
        std::fs::write(workspace.join("v8project.yaml"), "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\ninfobase:\n  connection: 'File=base'\n").unwrap();
        std::fs::write(workspace.join("src/Configuration.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Test</Name></Properties><ChildObjects><CommonModule>Probe</CommonModule></ChildObjects></Configuration></MetaDataObject>"#).unwrap();
        std::fs::create_dir_all(workspace.join("src/CommonModules/Probe/Ext")).unwrap();
        std::fs::write(workspace.join("src/CommonModules/Probe.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>Probe</Name><Synonym/><Comment/><Global>false</Global><ClientManagedApplication>false</ClientManagedApplication><Server>true</Server><ExternalConnection>false</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>false</ServerCall><Privileged>false</Privileged><ReturnValuesReuse>DontUse</ReturnValuesReuse></Properties></CommonModule></MetaDataObject>"#).unwrap();
        std::fs::write(
            workspace.join("src/CommonModules/Probe/Ext/Module.bsl"),
            "Процедура Ping() Экспорт\nКонецПроцедуры\n",
        )
        .unwrap();
        let target = crate::infrastructure::platform::current_target_id().unwrap();
        let exe = std::env::consts::EXE_SUFFIX;
        std::fs::write(
            plugin.join("third-party/manifest.json"),
            serde_json::json!({
                "schemaVersion": 2,
                "tools": [
                    {
                        "name": "v8-runner",
                        "version": RUNNER_VERSION,
                        "binaryPath": format!("bin/{target}/v8-runner{exe}"),
                        "sha256": "0".repeat(64),
                    },
                    {
                        "name": "bsl-analyzer",
                        "version": "0.2.67",
                        "binaryPath": format!("bin/{target}/bsl-analyzer{exe}"),
                        "sha256": "0".repeat(64),
                    },
                    {
                        "name": "rlm-bsl-mcp",
                        "artifact": "rlm-tools-bsl",
                        "version": "1.33.0",
                        "binaryPath": format!("bin/{target}/rlm-bsl-mcp{exe}"),
                        "sha256": "0".repeat(64),
                    }
                ],
            })
            .to_string(),
        )
        .unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "infrastructure::daemon::server::actor_capacity_tests::canonical_selected_engine_routes_request_delivery_before_provider_start",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env(ROOT, root.path())
            .env("UNICA_PLUGIN_ROOT", &plugin)
            .env("UNICA_ARTIFACT_CACHE", root.path().join("cache"))
            .env("UNICA_RUNTIME_MANIFEST", root.path().join("missing-release.json"))
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stdout.contains("running 1 test"), "{stdout}\n{stderr}");
        assert!(output.status.success(), "{stdout}\n{stderr}");
    }

    /// #549: «нет поставляемого движка» — один отказ на всех четырёх
    /// маршрутах. `run` (раннер), `search` с ролью (поставщик поиска), `check`
    /// модуля (анализ BSL) и `view` со сводкой графа (анализатор) отвечают
    /// `provider_unavailable` с уточнением `provider_absent` и одинаковыми
    /// полями движка в `data` — и когда файла нет, и когда его контрольная
    /// сумма не совпала с манифестом.
    #[test]
    fn every_route_refuses_a_missing_engine_with_one_typed_refusal() {
        const CHILD: &str = "UNICA_MISSING_ENGINE_ROUTES_CHILD";
        const ROOT: &str = "UNICA_MISSING_ENGINE_ROUTES_ROOT";
        let target = crate::infrastructure::platform::current_target_id().unwrap();
        let exe = std::env::consts::EXE_SUFFIX;
        if std::env::var_os(CHILD).is_some() {
            let root = std::path::PathBuf::from(std::env::var_os(ROOT).unwrap());
            let workspace = root.join("workspace");
            let plugin = root.join("plugin");
            let runtime = bootstrap_runtime();
            let call = |tool: ToolIdentity, arguments: serde_json::Value| -> DomainResult {
                let request = InvocationRequest::new(
                    tool,
                    arguments,
                    std::fs::canonicalize(&workspace).unwrap().to_string_lossy(),
                    7_000,
                )
                .unwrap();
                match runtime.bind(request) {
                    Ok(bound) => match bound.prepare() {
                        Ok(prepared) => prepared.execute(CancellationToken::new()).unwrap(),
                        Err(result) => *result,
                    },
                    Err(V5CanonicalPrepareError::Direct(result))
                    | Err(V5CanonicalPrepareError::Rejected(result)) => *result,
                    Err(error) => panic!("{error:?}"),
                }
            };
            // `view` спрашивает граф адресом ветви метода: это единственный
            // вход в граф вызовов.
            let view_arguments = serde_json::json!({
                "at": "main:CommonModule.Пример.Method.ОбработатьПример.Caller",
            });
            let routes = [
                (
                    "run",
                    "v8-runner",
                    ToolIdentity::Run,
                    serde_json::json!({"op":"extensions.list","args":{},"dryRun":true}),
                ),
                (
                    "search",
                    "bsl-analyzer",
                    ToolIdentity::Search,
                    serde_json::json!({"query":"ОбработатьПример","role":"symbol"}),
                ),
                (
                    "check",
                    "bsl-analyzer",
                    ToolIdentity::Check,
                    serde_json::json!({"at":"main:CommonModule.Пример"}),
                ),
                ("view", "bsl-analyzer", ToolIdentity::View, view_arguments),
            ];
            let binary = |tool: &str| plugin.join(format!("bin/{target}/{tool}{exe}"));
            let flat = |sha: &dyn Fn(&str) -> String| {
                serde_json::json!({
                    "schemaVersion": 2,
                    "tools": (["v8-runner", "bsl-analyzer"].map(|tool| serde_json::json!({
                        "name": tool,
                        "version": if tool == "v8-runner" { RUNNER_VERSION } else { "0.2.86" },
                        "binaryPath": format!("bin/{target}/{tool}{exe}"),
                        "sha256": sha(tool),
                    }))),
                })
            };
            let write_manifest = |manifest: serde_json::Value| {
                std::fs::write(
                    plugin.join("third-party/manifest.json"),
                    manifest.to_string(),
                )
                .unwrap();
            };
            // Файл, который ОС не может запустить: без права исполнения на
            // Unix (иначе ОС отдала бы его оболочке как сценарий) и не образ
            // программы на Windows.
            let unstartable = b"not the published engine";
            let unstartable_sha = {
                use sha2::Digest;
                format!("{:x}", sha2::Sha256::digest(unstartable))
            };
            for stage in ["manifest", "target", "delivery", "checksum", "start"] {
                match stage {
                    "manifest" => write_manifest(serde_json::json!({
                        "schemaVersion": 2,
                        "tools": [],
                    })),
                    "target" => write_manifest(serde_json::json!({
                        "schemaVersion": 2,
                        "tools": (["v8-runner", "bsl-analyzer"].map(|tool| serde_json::json!({
                            "name": tool,
                            "version": "0.0.1",
                            "binaries": {"plan9-mips": {
                                "binaryPath": format!("bin/plan9-mips/{tool}"),
                                "sha256": "0".repeat(64),
                            }},
                        }))),
                    })),
                    "delivery" => write_manifest(flat(&|_| "0".repeat(64))),
                    "checksum" => {
                        std::fs::create_dir_all(plugin.join(format!("bin/{target}"))).unwrap();
                        for tool in ["v8-runner", "bsl-analyzer"] {
                            std::fs::write(binary(tool), unstartable).unwrap();
                            unica_workspace::test_support::clear_execute_permission_for_test(
                                &binary(tool),
                            )
                            .unwrap();
                        }
                    }
                    _ => write_manifest(flat(&|_| unstartable_sha.clone())),
                }
                for (route, tool, identity, arguments) in &routes {
                    let result = call(*identity, arguments.clone());
                    let case = format!("{route} at {stage}");
                    assert!(!result.ok, "{case}");
                    assert_eq!(result.diagnostics.len(), 1, "{case}");
                    let diagnostic = &result.diagnostics[0];
                    assert_eq!(diagnostic["code"], "provider_unavailable", "{case}");
                    assert_eq!(diagnostic["detailCode"], "provider_absent", "{case}");
                    assert_eq!(diagnostic["outcome"], "needsHuman", "{case}");
                    let message = diagnostic["message"].as_str().unwrap();
                    assert!(
                        message.starts_with(&format!(
                            "bundled_tool_missing: {tool} is not available for "
                        )),
                        "одна формулировка на всех маршрутах: {case}"
                    );
                    let data = result.data.as_ref().expect("поля движка в data");
                    assert_eq!(data["tool"], *tool, "{case}");
                    assert_eq!(data["target"], target, "{case}");
                    assert_eq!(data["stage"], stage, "{case}");
                    assert_eq!(data["retryable"], false, "{case}");
                    if matches!(stage, "delivery" | "checksum" | "start") {
                        assert_eq!(
                            *data,
                            serde_json::json!({
                                "tool": tool,
                                "path": binary(tool).display().to_string(),
                                "target": target,
                                "pinnedVersion": if *tool == "v8-runner" { RUNNER_VERSION } else { "0.2.86" },
                                "stage": stage,
                                "retryable": false,
                            }),
                            "{case}"
                        );
                    }
                }
            }
            return;
        }

        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let plugin = root.path().join("plugin");
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-diagnostics");
        for relative in [
            "v8project.yaml",
            "src/Configuration.xml",
            "src/Languages/Русский.xml",
            "src/CommonModules/Пример.xml",
            "src/CommonModules/Пример/Ext/Module.bsl",
        ] {
            let destination = workspace.join(relative);
            std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
            std::fs::copy(fixture.join(relative), destination).unwrap();
        }
        std::fs::OpenOptions::new()
            .append(true)
            .open(workspace.join("v8project.yaml"))
            .and_then(|mut file| {
                std::io::Write::write_all(&mut file, b"infobase:\n  connection: 'File=base'\n")
            })
            .unwrap();
        std::fs::create_dir_all(plugin.join("third-party")).unwrap();
        std::fs::write(
            plugin.join("third-party/manifest.json"),
            serde_json::json!({
                "schemaVersion": 2,
                "tools": [
                    {
                        "name": "v8-runner",
                        "version": RUNNER_VERSION,
                        "binaryPath": format!("bin/{target}/v8-runner{exe}"),
                        "sha256": "0".repeat(64),
                    },
                    {
                        "name": "bsl-analyzer",
                        "version": "0.2.86",
                        "binaryPath": format!("bin/{target}/bsl-analyzer{exe}"),
                        "sha256": "0".repeat(64),
                    },
                ],
            })
            .to_string(),
        )
        .unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "infrastructure::daemon::server::actor_capacity_tests::every_route_refuses_a_missing_engine_with_one_typed_refusal",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env(ROOT, root.path())
            .env("UNICA_PLUGIN_ROOT", &plugin)
            .env_remove("UNICA_ARTIFACT_CACHE")
            .env_remove("UNICA_RUNTIME_MANIFEST")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stdout.contains("running 1 test"), "{stdout}\n{stderr}");
        assert!(output.status.success(), "{stdout}\n{stderr}");
    }

    /// #406: канонический `run` создания базы в рабочей области без
    /// поставляемого раннера отвечает одним типизированным отказом и не
    /// раскрывает ни путь рабочей области, ни строку подключения из
    /// `v8project.yaml`, ни значения, переданные в аргументах.
    #[test]
    fn canonical_infobase_create_refusal_discloses_no_private_input() {
        const CHILD: &str = "UNICA_ISSUE_406_RUN_CHILD";
        const ROOT: &str = "UNICA_ISSUE_406_RUN_ROOT";
        const WORKSPACE_SENTINEL: &str = "unica-issue-406-private-workspace";
        const CWD_SENTINEL: &str = "/missing/unica-issue-406-private-cwd";
        const CONNECTION_SENTINEL: &str = "File=/private/issue-406-sensitive.ib";
        if std::env::var_os(CHILD).is_some() {
            let root = std::path::PathBuf::from(std::env::var_os(ROOT).unwrap());
            let workspace = std::fs::canonicalize(root.join(WORKSPACE_SENTINEL)).unwrap();
            let runtime = bootstrap_runtime();
            let call = |arguments: serde_json::Value| -> DomainResult {
                let request = InvocationRequest::new(
                    ToolIdentity::Run,
                    arguments,
                    workspace.to_string_lossy(),
                    7_000,
                )
                .unwrap();
                match runtime.bind(request) {
                    Ok(bound) => match bound.prepare() {
                        Ok(prepared) => prepared.execute(CancellationToken::new()).unwrap(),
                        Err(result) => *result,
                    },
                    Err(V5CanonicalPrepareError::Direct(result))
                    | Err(V5CanonicalPrepareError::Rejected(result)) => *result,
                    Err(error) => panic!("{error:?}"),
                }
            };
            // Файлы и каталоги: пустой каталог, созданный отказом, тоже изменение.
            let image = |root: &std::path::Path| {
                fn directories(
                    root: &std::path::Path,
                    current: &std::path::Path,
                    found: &mut std::collections::BTreeSet<std::path::PathBuf>,
                ) {
                    for entry in std::fs::read_dir(current).unwrap() {
                        let entry = entry.unwrap();
                        if entry.file_type().unwrap().is_dir() {
                            found.insert(entry.path().strip_prefix(root).unwrap().to_path_buf());
                            directories(root, &entry.path(), found);
                        }
                    }
                }
                let mut found = std::collections::BTreeSet::new();
                directories(root, root, &mut found);
                (unica_workspace::test_support::tree_snapshot(root), found)
            };
            let before = image(&workspace);
            for (case, arguments, code) in [
                (
                    "configured connection",
                    serde_json::json!({"op":"infobase.create","args":{},"dryRun":false}),
                    "provider_unavailable",
                ),
                (
                    "private arguments",
                    serde_json::json!({"op":"infobase.create","args":{
                        "cwd": CWD_SENTINEL,
                        "connection": CONNECTION_SENTINEL
                    },"dryRun":false}),
                    "bad_value",
                ),
            ] {
                let refused = call(arguments);
                assert!(!refused.ok, "{case}: {refused:?}");
                assert_eq!(refused.diagnostics.len(), 1, "{case}: {refused:?}");
                assert_eq!(refused.diagnostics[0]["code"], code, "{case}: {refused:?}");
                if code == "provider_unavailable" {
                    assert_eq!(
                        refused.diagnostics[0]["detailCode"], "provider_absent",
                        "{case}: {refused:?}"
                    );
                }
                let serialized = serde_json::to_string(&refused).unwrap();
                for private in [
                    WORKSPACE_SENTINEL,
                    CWD_SENTINEL,
                    CONNECTION_SENTINEL,
                    "issue-406-sensitive",
                ] {
                    assert!(
                        !serialized.contains(private),
                        "{case} discloses {private}: {serialized}"
                    );
                }
            }
            assert_eq!(image(&workspace), before);
            return;
        }

        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join(WORKSPACE_SENTINEL);
        std::fs::create_dir_all(workspace.join("src")).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            format!(
                "format: DESIGNER\ninfobase:\n  connection: '{CONNECTION_SENTINEL}'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n"
            ),
        )
        .unwrap();
        // Поставка без раннера: манифест есть, инструментов в нём нет.
        let plugin = root.path().join("plugin");
        std::fs::create_dir_all(plugin.join("third-party")).unwrap();
        std::fs::write(
            plugin.join("third-party/manifest.json"),
            serde_json::json!({"schemaVersion": 2, "tools": []}).to_string(),
        )
        .unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "infrastructure::daemon::server::actor_capacity_tests::canonical_infobase_create_refusal_discloses_no_private_input",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env(ROOT, root.path())
            .env("UNICA_PLUGIN_ROOT", &plugin)
            .env_remove("UNICA_ARTIFACT_CACHE")
            .env_remove("UNICA_RUNTIME_MANIFEST")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stdout.contains("running 1 test"), "{stdout}\n{stderr}");
        assert!(output.status.success(), "{stdout}\n{stderr}");
    }

    const ARCHIVE_FIXTURE_BINARY_SKIP: &str =
        "the ordinary test archive binds its Unica fixture binary; run through run-tests.py --rust-lane ordinary (phase run or medium over the build archive)";

    /// The ordinary archive's prepared `unica`, verified against its producer's
    /// SHA256. `None` only when no lane runner bound it; a bound but damaged
    /// fixture fails instead of skipping.
    fn canonical_delivery_fixture_binary() -> Option<(std::path::PathBuf, String)> {
        use sha2::{Digest, Sha256};

        let binary =
            std::path::PathBuf::from(std::env::var_os("UNICA_CANONICAL_DELIVERY_SUCCESS_BINARY")?);
        let expected = std::env::var("UNICA_CANONICAL_DELIVERY_SUCCESS_SHA256")
            .expect("the archive fixture binary requires its producer's SHA256");
        assert!(binary.is_absolute(), "fixture binary path must be absolute");
        assert!(
            expected.len() == 64
                && expected
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "fixture binary SHA256 must be canonical lowercase hex"
        );
        let bytes = std::fs::read(&binary).expect("read the prepared archive fixture binary");
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes)),
            expected,
            "archive fixture binary does not match its producer's SHA256"
        );
        Some((binary, expected))
    }

    #[test]
    fn canonical_run_and_role_search_deliver_engines_and_run_supported_providers() {
        const CHILD: &str = "UNICA_CANONICAL_DELIVERY_SUCCESS_CHILD";
        const ROOT: &str = "UNICA_CANONICAL_DELIVERY_SUCCESS_ROOT";
        if std::env::var_os(CHILD).is_some() {
            use sha2::{Digest, Sha256};
            use unica_artifact_delivery::{
                BootstrapError, DownloadObserver, Downloader, HostTarget,
            };

            struct PublishedFixture(
                std::collections::BTreeMap<String, Vec<u8>>,
                std::sync::atomic::AtomicUsize,
            );
            impl Downloader for PublishedFixture {
                fn download(
                    &self,
                    _url: &str,
                    destination: &std::path::Path,
                    observer: &dyn DownloadObserver,
                ) -> Result<(), BootstrapError> {
                    let name = _url.rsplit('/').next().unwrap();
                    let bytes = &self.0[name];
                    std::fs::write(destination, bytes)?;
                    observer.transferred(bytes.len() as u64, Some(bytes.len() as u64));
                    self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok(())
                }
            }

            let root = std::path::PathBuf::from(std::env::var_os(ROOT).unwrap());
            let workspace = root.join("workspace");
            let plugin = root.join("plugin");
            let cache = root.join("cache");
            std::fs::create_dir_all(workspace.join("src")).unwrap();
            std::fs::create_dir_all(plugin.join("third-party")).unwrap();
            std::fs::write(workspace.join("v8project.yaml"), "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\ninfobase:\n  connection: 'File=base'\n").unwrap();
            std::fs::write(workspace.join("src/Configuration.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Test</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#).unwrap();
            let fixture_source = root.join("fake-engine.rs");
            std::fs::write(&fixture_source, r###"
use std::io::{self, BufRead, Write};
fn main() {
    let marker = std::path::PathBuf::from(std::env::var("UNICA_TEST_PROVIDER_MARKER_DIR").unwrap());
    let args = std::env::args().collect::<Vec<_>>();
    if args.iter().any(|argument| argument == "push") {
        assert!(args.iter().any(|argument| argument == "--dry-run"));
        assert!(args.iter().any(|argument| argument == "main"));
        std::fs::write(marker.join("push"), "v8-runner push preview invoked").unwrap();
        println!("{}", r#"{"ok":true,"command":"push","data":{"ok":true,"provider_dispatched":false,"steps":[{"source_set":"main","mode":"full","ok":true,"applied":false}]}}"#);
        return;
    }
    if args.iter().any(|argument| argument == "extensions") {
        std::fs::write(marker.join("run"), "v8-runner invoked").unwrap();
        println!("{}", r#"{"ok":true,"command":"extensions","data":{"ok":true,"provider_dispatched":false,"provider":{"selected":"ibcmd","origin":{"kind":"default"}},"requested":{"kind":"all"},"extensions":[],"plan":"fixture"}}"#);
        return;
    }
    if args.get(1).is_some_and(|argument| argument == "index") {
        let db = std::path::PathBuf::from(std::env::var("RLM_INDEX_DIR").unwrap()).join("bsl_index.db");
        if args.get(2).is_some_and(|argument| argument == "info") {
            if db.is_file() {
                println!("Index: {}\nStatus: fresh", db.display());
            } else {
                println!("Index not found");
            }
        } else {
            std::fs::create_dir_all(db.parent().unwrap()).unwrap();
            std::fs::write(db, "fixture-index").unwrap();
            std::fs::write(marker.join("index"), "rlm-bsl-index invoked").unwrap();
        }
        return;
    }
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        if line.contains("\"method\":\"initialize\"") {
            println!("{}", r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-03-26","capabilities":{},"serverInfo":{"name":"fixture","version":"1"}}}"#);
            io::stdout().flush().unwrap();
        } else if line.contains("\"name\":\"search\"") {
            assert!(line.contains("search_code"), "wrong analyzer action: {line}");
            std::fs::write(marker.join("search"), "bsl-analyzer invoked").unwrap();
            println!("{}", r#"{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"No results found."}]}}"#);
            io::stdout().flush().unwrap();
        } else if line.contains("\"name\":\"rlm_start\"") {
            println!("{}", r#"{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"{\"session_id\":\"fixture\",\"index\":{\"index_status\":\"ready\"}}"}]}}"#);
            io::stdout().flush().unwrap();
        } else if line.contains("\"name\":\"rlm_execute\"") {
            std::fs::write(marker.join("semantic"), "rlm-bsl-mcp invoked").unwrap();
            println!("{}", r#"{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"{\"stdout\":\"[]\"}"}]}}"#);
            io::stdout().flush().unwrap();
        } else if line.contains("\"name\":\"rlm_end\"") {
            println!("{}", r#"{"jsonrpc":"2.0","id":4,"result":{"content":[{"type":"text","text":"{}"}]}}"#);
            io::stdout().flush().unwrap();
        }
    }
}
"###).unwrap();
            let fixture_binary = root.join(format!("fake-engine{}", std::env::consts::EXE_SUFFIX));
            let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
            let compilation = std::process::Command::new(rustc)
                .args(["--edition=2021", "-o"])
                .arg(&fixture_binary)
                .arg(&fixture_source)
                .output()
                .unwrap();
            assert!(
                compilation.status.success(),
                "{}",
                String::from_utf8_lossy(&compilation.stderr)
            );
            let bytes = std::fs::read(&fixture_binary).unwrap();
            let digest = format!("{:x}", Sha256::digest(&bytes));
            let mut assets = std::collections::BTreeMap::new();
            let mut core_targets = serde_json::Map::new();
            let mut runner_targets = serde_json::Map::new();
            let mut analyzer_targets = serde_json::Map::new();
            let mut rlm_targets = serde_json::Map::new();
            for host in HostTarget::ALL {
                let target = host.as_str();
                let suffix = if target == "win-x64" { ".exe" } else { "" };
                let archive_name = format!("rlm-tools-bsl-{target}.tar.gz");
                let archive = {
                    let encoder =
                        flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                    let mut builder = tar::Builder::new(encoder);
                    for name in [
                        format!("rlm-bsl-mcp{suffix}"),
                        format!("rlm-bsl-index{suffix}"),
                    ] {
                        let mut header = tar::Header::new_gnu();
                        header.set_size(bytes.len() as u64);
                        header.set_mode(0o755);
                        header.set_cksum();
                        builder
                            .append_data(&mut header, name, bytes.as_slice())
                            .unwrap();
                    }
                    builder.into_inner().unwrap().finish().unwrap()
                };
                let archive_digest = format!("{:x}", Sha256::digest(&archive));
                assets.insert(archive_name.clone(), archive);
                rlm_targets.insert(target.to_owned(), serde_json::json!({
                    "asset": {"name": archive_name, "url": format!("https://github.com/IngvarConsulting/unica-toolchain/releases/download/rlm-tools-bsl-v1.33.0-build.3/{archive_name}"), "mediaType": "application/gzip", "sha256": archive_digest},
                    "files": [
                        {"path": format!("rlm-bsl-mcp{suffix}"), "sha256": digest, "executable": true},
                        {"path": format!("rlm-bsl-index{suffix}"), "sha256": digest, "executable": true}
                    ],
                }));
                let core_asset = format!("unica-runtime-{target}.tar.gz");
                let core_path = format!("bin/{target}/unica{suffix}");
                core_targets.insert(target.to_owned(), serde_json::json!({
                    "asset": {"name": core_asset, "url": format!("https://github.com/IngvarConsulting/unica/releases/download/v{}/{core_asset}", env!("CARGO_PKG_VERSION")), "mediaType": "application/gzip", "sha256": "0".repeat(64)},
                    "files": [{"path": core_path, "sha256": "0".repeat(64), "executable": true}],
                    "entrypoint": core_path,
                }));
                for (name, targets, origin, version) in [
                    (
                        "v8-runner",
                        &mut runner_targets,
                        "https://github.com/IngvarConsulting/v8-runner-rust/releases/download",
                        RUNNER_VERSION,
                    ),
                    (
                        "bsl-analyzer",
                        &mut analyzer_targets,
                        "https://github.com/IngvarConsulting/unica-toolchain/releases/download",
                        "0.2.67",
                    ),
                ] {
                    let asset = format!("{name}-{target}{suffix}");
                    assets.insert(asset.clone(), bytes.clone());
                    targets.insert(target.to_owned(), serde_json::json!({
                        "asset": {"name": asset, "url": format!("{origin}/{name}-v{version}-build.1/{asset}"), "mediaType": "application/octet-stream", "sha256": digest},
                        "files": [{"path": format!("bin/{target}/{name}{suffix}"), "sha256": digest, "executable": true}],
                    }));
                }
            }
            let release = serde_json::json!({
                "schemaVersion": 2,
                "pluginVersion": env!("CARGO_PKG_VERSION"),
                "source": {"repository": "https://github.com/IngvarConsulting/unica", "commit": "0".repeat(40)},
                "release": {"repository": "https://github.com/IngvarConsulting/unica", "tag": format!("v{}", env!("CARGO_PKG_VERSION"))},
                "artifacts": {
                    "unica": {"version": env!("CARGO_PKG_VERSION"), "role": "core", "targets": core_targets},
                    "v8-runner": {"version": RUNNER_VERSION, "role": "engine", "targets": runner_targets},
                    "bsl-analyzer": {"version": "0.2.67", "role": "engine", "targets": analyzer_targets},
                    "rlm-tools-bsl": {"version": "1.33.0", "role": "engine", "targets": rlm_targets},
                }
            });
            let release_path = root.join("runtime-manifest.json");
            std::fs::write(&release_path, release.to_string()).unwrap();
            let target = crate::infrastructure::platform::current_target_id().unwrap();
            let suffix = std::env::consts::EXE_SUFFIX;
            let rlm_archive = format!("rlm-tools-bsl-{target}.tar.gz");
            let rlm_digest = format!("{:x}", Sha256::digest(&assets[&rlm_archive]));
            std::fs::write(plugin.join("third-party/manifest.json"), serde_json::json!({
                "schemaVersion": 2,
                "artifactAssets": {
                    "v8-runner": {"sha256": digest},
                    "bsl-analyzer": {"sha256": digest},
                    "rlm-tools-bsl": {"sha256": rlm_digest},
                },
                "tools": [
                    {"name": "v8-runner", "version": RUNNER_VERSION, "binaryPath": format!("bin/{target}/v8-runner{suffix}"), "deliveredPath": format!("bin/{target}/v8-runner{suffix}"), "sha256": digest},
                    {"name": "bsl-analyzer", "version": "0.2.67", "binaryPath": format!("bin/{target}/bsl-analyzer{suffix}"), "deliveredPath": format!("bin/{target}/bsl-analyzer{suffix}"), "sha256": digest},
                    {"name": "rlm-bsl-mcp", "artifact": "rlm-tools-bsl", "version": "1.33.0", "binaryPath": format!("bin/{target}/rlm-bsl-mcp{suffix}"), "deliveredPath": format!("rlm-bsl-mcp{suffix}"), "sha256": digest},
                    {"name": "rlm-bsl-index", "artifact": "rlm-tools-bsl", "version": "1.33.0", "binaryPath": format!("bin/{target}/rlm-bsl-index{suffix}"), "deliveredPath": format!("rlm-bsl-index{suffix}"), "sha256": digest},
                ],
            }).to_string()).unwrap();
            std::env::set_var("UNICA_PLUGIN_ROOT", &plugin);
            std::env::set_var("UNICA_ARTIFACT_CACHE", &cache);
            std::env::set_var("UNICA_RUNTIME_MANIFEST", &release_path);
            std::env::set_var("UNICA_TEST_PROVIDER_MARKER_DIR", &root);
            std::env::set_var("UNICA_WORKSPACE_SERVICE_IDLE_SECS", "1");
            let (server, _) = canonical_delivery_fixture_binary()
                .expect("the parent binds the archive fixture binary for its child");
            std::env::set_var("UNICA_TEST_WORKSPACE_SERVICE_EXE", server);
            let downloader = Arc::new(PublishedFixture(
                assets,
                std::sync::atomic::AtomicUsize::new(0),
            ));
            let runtime = V5CanonicalInvocationRuntime::new(
                Arc::new(super::super::v13_service::CanonicalV13ReadService::default()),
                Arc::new(TokioClock),
            )
            .with_delivery_downloader_for_test(downloader.clone());
            assert!(!cache.join("v8-runner").exists());
            assert!(!cache.join("bsl-analyzer").exists());

            let request = InvocationRequest::new(
                ToolIdentity::Run,
                serde_json::json!({"op":"push","args":{"sourceSet":"main","force":true},"dryRun":true}),
                workspace.display().to_string(),
                7_000,
            )
            .unwrap();
            let push = runtime
                .bind(request)
                .unwrap()
                .prepare()
                .unwrap()
                .execute(CancellationToken::new())
                .unwrap();
            assert!(push.ok, "{push:?}");
            assert_eq!(push.data.as_ref().unwrap()["op"], "push");
            assert_eq!(push.data.as_ref().unwrap()["dryRun"], true);
            assert_eq!(
                std::fs::read_to_string(root.join("push")).unwrap(),
                "v8-runner push preview invoked"
            );

            let request = InvocationRequest::new(
                ToolIdentity::Run,
                serde_json::json!({"op":"extensions.list","args":{},"dryRun":true}),
                workspace.display().to_string(),
                7_000,
            )
            .unwrap();
            let run = runtime
                .bind(request)
                .unwrap()
                .prepare()
                .unwrap()
                .execute(CancellationToken::new())
                .unwrap();
            assert!(run.ok, "{run:?}");
            assert_eq!(
                std::fs::read_to_string(root.join("run")).unwrap(),
                "v8-runner invoked"
            );
            let search = submit_canonical(
                &runtime,
                &workspace,
                ToolIdentity::Search,
                serde_json::json!({"query":"Needle","role":"symbol"}),
            );
            assert!(
                search.ok,
                "{search:?}; marker={:?}; analyzer_cache_sha_verified={}",
                std::fs::read_to_string(root.join("search")).ok(),
                crate::infrastructure::bundled_tools::resolve_bundled_tool(
                    &plugin,
                    "bsl-analyzer",
                    true
                )
                .ok()
                .and_then(|resolved| std::fs::read(resolved.program).ok())
                .is_some_and(|bytes| format!("{:x}", Sha256::digest(bytes)) == digest)
            );
            assert_eq!(
                std::fs::read_to_string(root.join("search")).unwrap(),
                "bsl-analyzer invoked"
            );
            assert_eq!(
                search.data.as_ref().unwrap()["matches"][0]["provider"],
                "bsl-analyzer"
            );
            assert_eq!(
                search.data.as_ref().unwrap()["matches"][0]["status"],
                "empty"
            );
            assert_eq!(
                search.data.as_ref().unwrap()["matches"][0]["hits"],
                serde_json::json!([])
            );
            let mut semantic = submit_canonical(
                &runtime,
                &workspace,
                ToolIdentity::Search,
                serde_json::json!({"query":"Needle","role":"semantic"}),
            );
            for _ in 0..5 {
                if semantic.ok {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
                semantic = submit_canonical(
                    &runtime,
                    &workspace,
                    ToolIdentity::Search,
                    serde_json::json!({"query":"Needle","role":"semantic"}),
                );
            }
            assert!(
                semantic.ok,
                "{semantic:?}; index={:?}; rlm={:?}",
                std::fs::read_to_string(root.join("index")),
                std::fs::read_to_string(root.join("semantic"))
            );
            assert_eq!(
                std::fs::read_to_string(root.join("index")).unwrap(),
                "rlm-bsl-index invoked"
            );
            assert_eq!(
                std::fs::read_to_string(root.join("semantic")).unwrap(),
                "rlm-bsl-mcp invoked"
            );
            assert_eq!(
                semantic.data.as_ref().unwrap()["matches"][0]["provider"],
                "rlm"
            );
            for (name, version) in [("v8-runner", RUNNER_VERSION), ("bsl-analyzer", "0.2.67")] {
                assert!(cache
                    .join(name)
                    .join(format!("{version}--{digest}"))
                    .join(target)
                    .is_dir());
            }
            assert!(cache
                .join("rlm-tools-bsl")
                .join(format!("1.33.0--{rlm_digest}"))
                .join(target)
                .is_dir());
            for tool in ["rlm-bsl-index", "rlm-bsl-mcp"] {
                let resolved =
                    crate::infrastructure::bundled_tools::resolve_bundled_tool(&plugin, tool, true)
                        .unwrap();
                assert_eq!(
                    format!(
                        "{:x}",
                        Sha256::digest(std::fs::read(resolved.program).unwrap())
                    ),
                    digest,
                    "{tool} must resolve to the checked delivery"
                );
            }
            assert_eq!(downloader.1.load(std::sync::atomic::Ordering::SeqCst), 3);
            // The real service normally lives for hours. Wait for its short
            // idle shutdown before the parent removes this isolated tempdir.
            let services = workspace.join(".build/unica/services");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                let live_record = std::fs::read_dir(&services)
                    .unwrap()
                    .flatten()
                    .any(|entry| entry.path().join("service.json").is_file());
                if !live_record {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "workspace service did not stop after idle timeout"
                );
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            return;
        }

        let Some((server, digest)) = canonical_delivery_fixture_binary() else {
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::ArchiveFixtureBinary,
                ARCHIVE_FIXTURE_BINARY_SKIP,
            );
            return;
        };
        let root = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "infrastructure::daemon::server::actor_capacity_tests::canonical_run_and_role_search_deliver_engines_and_run_supported_providers",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env(ROOT, root.path())
            .env("UNICA_CANONICAL_DELIVERY_SUCCESS_BINARY", server)
            .env("UNICA_CANONICAL_DELIVERY_SUCCESS_SHA256", digest)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stdout.contains("running 1 test"), "{stdout}\n{stderr}");
        assert!(output.status.success(), "{stdout}\n{stderr}");
    }

    #[test]
    fn extension_operations_bind_before_source_admission_as_known_long_tasks() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\ninfobase:\n  connection: 'File=base'\n",
        )
        .unwrap();
        for (op, args) in [
            ("extensions.list", serde_json::json!({})),
            ("push", serde_json::json!({"delete":"Test"})),
            (
                "extensions.set",
                serde_json::json!({"name":"Test","active":true}),
            ),
        ] {
            let runtime = bootstrap_runtime();
            let request = InvocationRequest::new(
                ToolIdentity::Run,
                serde_json::json!({"op":op,"args":args,"dryRun":true}),
                workspace.path().display().to_string(),
                7_000,
            )
            .unwrap();
            let bound = runtime
                .bind(request)
                .expect("extension admission without a source set");
            assert!(matches!(
                bound.0,
                ActorBoundCanonicalInvocationState::Extensions { .. }
            ));
            assert_eq!(
                bound.prepare().unwrap().execution_class(),
                &ExecutionClass::KnownLong(KnownLongReason::ExternalProcess)
            );
        }
    }

    #[test]
    fn canonical_view_bootstrap_does_not_claim_source_sets_in_an_empty_configuration() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\n",
        )
        .unwrap();
        let runtime = bootstrap_runtime();

        let result = submit_bootstrap(&runtime, workspace.path(), serde_json::json!({}));

        assert!(result.ok, "{result:?}");
        assert_eq!(
            result.summary,
            "workspace configuration discovered; no source sets are attached and no infobase target is configured"
        );
        let data = result.data.as_ref().expect("bootstrap data");
        assert_eq!(data["config"]["state"], "configured");
        assert_eq!(data["sourceSets"], serde_json::json!([]));
        assert_eq!(data["infobase"]["configured"], false);
    }

    #[test]
    fn canonical_view_bootstrap_recognizes_an_infobase_only_workspace() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\ninfobase:\n  connection: 'File=base'\n",
        )
        .unwrap();
        let runtime = bootstrap_runtime();

        let result = submit_bootstrap(&runtime, workspace.path(), serde_json::json!({}));

        assert!(result.ok, "{result:?}");
        assert_eq!(
            result.summary,
            "workspace configuration and infobase target discovered; no source sets are attached"
        );
        let data = result.data.as_ref().expect("bootstrap data");
        assert_eq!(data["config"]["state"], "configured");
        assert_eq!(data["sourceSets"], serde_json::json!([]));
        assert_eq!(data["infobase"]["configured"], true);
        assert_eq!(data["sourceSelectionError"], serde_json::Value::Null);
        assert_eq!(data["setup"], serde_json::Value::Null);
        // Факты не несут вердикта: ни готовности, ни проверок, ни диагностик.
        for verdict in [
            "ready",
            "discoveredReady",
            "repositoryReady",
            "readinessState",
            "checks",
            "diagnostics",
        ] {
            assert_eq!(data.get(verdict), None, "{verdict} принадлежит check {{}}");
        }
        assert_eq!(result.next.len(), 3, "{result:?}");
        assert_eq!(
            result.next[0]["args"],
            serde_json::json!({
                "op": "download",
                "args": {"state": "working", "output": "dist/main.cf"},
                "dryRun": true
            })
        );
        assert_eq!(
            result.next[1]["args"],
            serde_json::json!({
                "op": "infobase.dump",
                "args": {"output": "dist/base.dt"},
                "dryRun": true
            })
        );
        assert_eq!(result.next[2]["tool"], "unica.check", "{result:?}");

        let verdict = submit_root_verdict(&runtime, workspace.path());

        assert!(verdict.ok, "{verdict:?}");
        let verdict_data = verdict.data.as_ref().expect("verdict data");
        // The unpinned platform is a warning: the workspace stays ready.
        assert_eq!(verdict_data["status"], "passed");
        assert_eq!(verdict_data["ready"], true);
        let diagnostics = verdict_data["diagnostics"].as_array().unwrap();
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0]["code"], "config.platform_unpinned");
        assert_eq!(diagnostics[0]["severity"], "warning");
    }

    #[test]
    fn canonical_root_verdict_is_silent_about_a_pinned_platform() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\n",
        )
        .unwrap();
        std::fs::write(
            workspace.path().join("v8project.local.yaml"),
            "infobases:\n  origin:\n    connection: 'File=base'\ntools:\n  platform:\n    version: '8.3.27'\n",
        )
        .unwrap();
        let runtime = bootstrap_runtime();

        let verdict = submit_root_verdict(&runtime, workspace.path());

        let data = verdict.data.as_ref().expect("verdict data");
        assert_eq!(data["ready"], true, "{data}");
        assert_eq!(data["diagnostics"], serde_json::json!([]), "{data}");
    }

    #[test]
    fn canonical_view_setup_recommends_the_local_layer_without_a_password() {
        let workspace = tempfile::tempdir().unwrap();
        let runtime = bootstrap_runtime();

        let result = submit_bootstrap(&runtime, workspace.path(), serde_json::json!({}));

        let data = result.data.as_ref().expect("bootstrap data");
        let local = &data["setup"]["local"];
        assert_eq!(local["path"], "v8project.local.yaml", "{data}");
        let content = local["content"].as_str().unwrap();
        let layer: serde_yaml::Value = serde_yaml::from_str(content).unwrap();
        assert_eq!(layer["tools"]["platform"]["version"], "8.3.27");
        assert_eq!(layer["infobases"]["origin"]["connection"], "File=build/ib");
        assert!(layer["infobases"]["origin"]["password"].is_null());
        assert!(!content.to_lowercase().contains("password"), "{content}");
        assert!(!workspace.path().join("v8project.local.yaml").exists());

        std::fs::write(workspace.path().join("v8project.local.yaml"), "{}\n").unwrap();
        let result = submit_bootstrap(&runtime, workspace.path(), serde_json::json!({}));
        let data = result.data.as_ref().expect("bootstrap data");
        assert!(
            data["setup"]["local"].is_null(),
            "an existing local layer is kept: {data}"
        );
    }

    #[test]
    pub(crate) fn canonical_view_without_at_bootstraps_an_empty_workspace() {
        let workspace = tempfile::tempdir().unwrap();
        let runtime = bootstrap_runtime();
        let before = unica_workspace::test_support::tree_snapshot(workspace.path());

        let result = submit_bootstrap(&runtime, workspace.path(), serde_json::json!({}));

        assert!(result.ok, "{result:?}");
        let data = result.data.as_ref().expect("bootstrap data");
        assert_eq!(data["config"]["state"], "missing");
        assert_eq!(data["config"]["path"], "v8project.yaml");
        assert_eq!(data["sourceSets"], serde_json::json!([]));
        assert_eq!(data["setup"]["path"], "v8project.yaml");
        assert_eq!(data["setup"]["content"], serde_json::Value::Null);
        for verdict in [
            "ready",
            "discoveredReady",
            "repositoryReady",
            "readinessState",
            "checks",
            "diagnostics",
        ] {
            assert_eq!(data.get(verdict), None, "{verdict} belongs to check {{}}");
        }
        let wire = serde_json::to_string(data).unwrap();
        assert!(
            !wire.contains("unica.project."),
            "bootstrap must not recommend retired project tools: {wire}"
        );

        // Ненастроенное пространство — как раз тот случай, где вопрос «что не
        // так» главный, поэтому маршрут к вердикту здесь обязателен.
        assert!(
            result
                .next
                .iter()
                .any(|action| action["tool"] == "unica.check"),
            "{result:?}"
        );
        let verdict = submit_root_verdict(&runtime, workspace.path());

        assert!(verdict.ok, "{verdict:?}");
        let verdict_data = verdict.data.as_ref().expect("verdict data");
        assert_eq!(verdict_data["status"], "failed");
        assert_eq!(verdict_data["ready"], false);
        assert!(
            verdict.rev.is_none(),
            "root check has no source revision lease"
        );
        for sources in ["sources", "sourceSets"] {
            assert_eq!(
                verdict_data.get(sources),
                None,
                "{sources} belongs to view {{}}"
            );
        }
        assert_eq!(verdict_data["checks"], serde_json::json!([]));
        assert_eq!(verdict_data["diagnostics"].as_array().unwrap().len(), 1);
        assert_eq!(
            verdict_data["diagnostics"][0]["code"],
            "source_roots_missing"
        );
        assert_eq!(
            before,
            unica_workspace::test_support::tree_snapshot(workspace.path())
        );
    }

    #[test]
    pub(crate) fn canonical_view_bootstrap_separates_source_and_repository_readiness() {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let runtime = bootstrap_runtime();

        let result = submit_bootstrap(&runtime, workspace.path(), serde_json::json!({}));

        assert!(result.ok, "{result:?}");
        let data = result.data.as_ref().expect("bootstrap data");
        assert_eq!(data["config"]["state"], "configured");
        assert_eq!(data["effectiveSourceSet"], "main");
        assert_eq!(result.next[0]["tool"], "unica.view");
        assert_eq!(result.next[0]["args"]["at"], "main:Configuration");

        let verdict = submit_root_verdict(&runtime, workspace.path());

        assert!(verdict.ok, "{verdict:?}");
        let verdict_data = verdict.data.as_ref().expect("verdict data");
        assert_eq!(verdict_data["ready"], true);
        assert_eq!(verdict_data["repositoryReady"], false);
    }

    #[test]
    fn canonical_view_bootstrap_reports_autodetected_sources_and_config_recipe() {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let runtime = bootstrap_runtime();

        let result = submit_bootstrap(&runtime, workspace.path(), serde_json::json!({}));

        assert!(result.ok, "{result:?}");
        let data = result.data.as_ref().expect("bootstrap data");
        assert_eq!(data["config"]["state"], "autodetected");
        assert_eq!(data["sourceSets"][0]["name"], "main");
        assert_eq!(data["sourceSets"][0]["path"], "src");
        assert!(data["setup"]["content"]
            .as_str()
            .unwrap()
            .contains("path: src"));
        assert!(!workspace.path().join("v8project.yaml").exists());
    }

    #[test]
    fn canonical_view_bootstrap_does_not_offer_actor_calls_for_edt_sources() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join("src/Configuration")).unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: EDT\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            workspace.path().join("src/.project"),
            "<projectDescription/>",
        )
        .unwrap();
        std::fs::write(
            workspace.path().join("src/Configuration/Configuration.mdo"),
            "<mdclass:Configuration/>",
        )
        .unwrap();
        let runtime = bootstrap_runtime();

        let result = submit_bootstrap(&runtime, workspace.path(), serde_json::json!({}));

        assert!(result.ok, "{result:?}");
        assert_eq!(
            result.data.as_ref().unwrap()["sourceSets"][0]["sourceFormat"],
            "edt"
        );
        // Единственный оставшийся маршрут — вопрос о вердикте: адрес узла
        // предлагать нечего, EDT в допуск не входит.
        assert_eq!(result.next.len(), 1, "{result:?}");
        assert_eq!(result.next[0]["tool"], "unica.check", "{result:?}");
    }

    /// Отказ допуска называет причину и следующий шаг.
    ///
    /// Из каталога без набора исходников `unica.view {}` отвечает фактами и
    /// маршрутом, а актор-связанные инструменты отвечали безымянным
    /// `workspace actor admission failed`: ни кода из закрытого словаря, ни
    /// причины, ни продолжения. Причина известна ровно в точке отказа — вторым
    /// обходом снаружи её пришлось бы восстанавливать по уже другому дереву.
    #[test]
    pub(crate) fn canonical_admission_names_why_no_source_set_was_admitted() {
        fn routes(result: &DomainResult) -> Vec<&str> {
            result
                .next
                .iter()
                .map(|action| action["tool"].as_str().expect("next names a tool"))
                .collect()
        }

        let runtime = bootstrap_runtime();

        // Рабочая область не заведена: ни `v8project.yaml`, ни автоопределяемых
        // корней 1С. Среду правит человек, поэтому исход — `needsHuman`.
        let bare = tempfile::tempdir().unwrap();
        let uninitialized = submit_canonical(
            &runtime,
            bare.path(),
            ToolIdentity::Search,
            serde_json::json!({"query": "Справочник"}),
        );

        assert!(!uninitialized.ok, "{uninitialized:?}");
        assert_eq!(uninitialized.diagnostics.len(), 1, "{uninitialized:?}");
        assert_eq!(
            uninitialized.diagnostics[0]["code"], "invalid_state",
            "{uninitialized:?}"
        );
        assert_eq!(
            uninitialized.diagnostics[0]["outcome"], "needsHuman",
            "{uninitialized:?}"
        );
        assert!(
            uninitialized.summary.contains("workspace is uninitialized"),
            "{uninitialized:?}"
        );
        assert_eq!(
            routes(&uninitialized),
            vec!["unica.view", "unica.check", "unica.run"],
            "{uninitialized:?}"
        );
        assert_eq!(
            uninitialized.data.as_ref().expect("refusal data")["workspaceRoot"],
            serde_json::Value::String(
                std::fs::canonicalize(bare.path())
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            ),
            "{uninitialized:?}"
        );

        // Наборы объявлены, но ни один не в формате выгрузки конфигуратора:
        // менять надо исходники, а не вызов.
        let edt = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(edt.path().join("src/Configuration")).unwrap();
        std::fs::write(
            edt.path().join("v8project.yaml"),
            "format: EDT\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(edt.path().join("src/.project"), "<projectDescription/>").unwrap();
        std::fs::write(
            edt.path().join("src/Configuration/Configuration.mdo"),
            "<mdclass:Configuration/>",
        )
        .unwrap();
        let unreadable_format = submit_canonical(
            &runtime,
            edt.path(),
            ToolIdentity::Resolve,
            serde_json::json!({"at": "main:Configuration"}),
        );

        assert!(!unreadable_format.ok, "{unreadable_format:?}");
        assert_eq!(
            unreadable_format.diagnostics[0]["code"], "invalid_source",
            "{unreadable_format:?}"
        );
        assert_eq!(
            unreadable_format.diagnostics[0]["outcome"], "fixSource",
            "{unreadable_format:?}"
        );
        let declared = unreadable_format.data.as_ref().expect("refusal data");
        assert_eq!(declared["sourceSets"][0]["name"], "main", "{declared}");
        assert_eq!(
            declared["sourceSets"][0]["sourceFormat"], "edt",
            "{declared}"
        );
        assert_eq!(
            routes(&unreadable_format),
            vec!["unica.view", "unica.check", "unica.run"],
            "{unreadable_format:?}"
        );

        // Настройка на месте, но её нельзя прочитать. Допуск отвечает теми же
        // словами, что `unica.view {}`: причина одна, и разойтись им нельзя.
        let broken = tempfile::tempdir().unwrap();
        std::fs::write(broken.path().join("v8project.yaml"), "format: [DESIGNER\n").unwrap();
        let invalid_config = submit_canonical(
            &runtime,
            broken.path(),
            ToolIdentity::Diff,
            serde_json::json!({"left": "main:Configuration", "right": "main:Configuration"}),
        );

        assert!(!invalid_config.ok, "{invalid_config:?}");
        assert_eq!(
            invalid_config.diagnostics[0]["code"], "invalid_state",
            "{invalid_config:?}"
        );
        assert!(
            invalid_config
                .summary
                .starts_with("v8project.yaml is present but invalid: "),
            "{invalid_config:?}"
        );
        assert_eq!(
            submit_bootstrap(&runtime, broken.path(), serde_json::json!({})).summary,
            invalid_config.summary,
            "the root and the admission name one cause with one sentence"
        );
        assert_eq!(
            routes(&invalid_config),
            vec!["unica.view", "unica.check"],
            "{invalid_config:?}"
        );
    }

    #[test]
    fn canonical_view_bootstrap_does_not_offer_an_unparseable_logical_address() {
        for (source_type, root_kind, root_file) in [
            ("CONFIGURATION", "Configuration", "Configuration.xml"),
            (
                "EXTERNAL_DATA_PROCESSORS",
                "ExternalDataProcessor",
                "Store.xml",
            ),
            ("EXTERNAL_REPORTS", "ExternalReport", "Store.xml"),
        ] {
            let workspace = tempfile::tempdir().unwrap();
            let source = workspace.path().join("src");
            std::fs::create_dir_all(&source).unwrap();
            std::fs::write(
            workspace.path().join("v8project.yaml"),
            format!("format: DESIGNER\nsource-set:\n  - name: bad name\n    type: {source_type}\n    path: src\n"),
        )
        .unwrap();
            std::fs::write(
            source.join(root_file),
            format!(r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{root_kind}><Properties><Name>Store</Name></Properties><ChildObjects/></{root_kind}></MetaDataObject>"#),
        )
        .unwrap();
            let runtime = bootstrap_runtime();

            let result = submit_bootstrap(&runtime, workspace.path(), serde_json::json!({}));

            assert!(result.ok, "{result:?}");
            assert_eq!(result.next.len(), 1, "{result:?}");
            assert_eq!(result.next[0]["tool"], "unica.check", "{result:?}");

            let verdict = submit_root_verdict(&runtime, workspace.path());

            assert!(verdict.ok, "{verdict:?}");
            let data = verdict.data.as_ref().unwrap();
            assert_eq!(data["ready"], false, "{data}");
            assert!(
                data["diagnostics"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|item| { item["code"] == "source_set.logical_name_invalid" }),
                "{data}"
            );
        }
    }

    #[test]
    fn canonical_view_bootstrap_does_not_fabricate_one_format_for_mixed_autodiscovery() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join("src/Configuration")).unwrap();
        std::fs::write(
            workspace.path().join("src/.project"),
            "<projectDescription/>",
        )
        .unwrap();
        std::fs::write(
            workspace.path().join("src/Configuration/Configuration.mdo"),
            "<mdclass:Configuration/>",
        )
        .unwrap();
        std::fs::create_dir_all(workspace.path().join("src/cfe/addon")).unwrap();
        std::fs::write(
            workspace.path().join("src/cfe/addon/Configuration.xml"),
            "<MetaDataObject/>",
        )
        .unwrap();
        let runtime = bootstrap_runtime();

        let result = submit_bootstrap(&runtime, workspace.path(), serde_json::json!({}));

        assert!(result.ok, "{result:?}");
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["config"]["state"], "autodetected");
        assert_eq!(data["setup"]["content"], serde_json::Value::Null, "{data}");
        assert!(result.next.iter().all(|next| {
            next["tool"] != "unica.run" || next["args"]["op"] != "workspace.initialize"
        }));
    }

    #[test]
    fn canonical_view_bootstrap_does_not_hide_an_invalid_project_config() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("v8project.yaml"), "source-set: [").unwrap();
        let runtime = bootstrap_runtime();

        let result = submit_bootstrap(&runtime, workspace.path(), serde_json::json!({}));

        assert!(!result.ok, "{result:?}");
        assert_eq!(result.data.as_ref().unwrap()["config"]["state"], "invalid");
        assert_eq!(result.diagnostics[0]["code"], "invalid_state");
        assert!(result.next.is_empty(), "{result:?}");
    }

    #[test]
    fn canonical_view_bootstrap_classifies_wrong_kind_project_config_as_invalid() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::create_dir(workspace.path().join("v8project.yaml")).unwrap();
        let runtime = bootstrap_runtime();

        let result = submit_bootstrap(&runtime, workspace.path(), serde_json::json!({}));

        assert!(!result.ok, "{result:?}");
        assert_eq!(result.data.as_ref().unwrap()["config"]["state"], "invalid");
        assert_eq!(result.diagnostics[0]["code"], "invalid_state");
    }

    #[test]
    fn canonical_view_bootstrap_classifies_broken_project_config_link_as_invalid() {
        use unica_workspace::test_support::{
            create_file_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let workspace = tempfile::tempdir().unwrap();
        let outcome = create_file_link_fixture_for_test(
            "missing-v8project.yaml",
            workspace.path().join("v8project.yaml"),
        )
        .unwrap();
        if outcome != FileLinkFixtureOutcome::Created {
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::Symlink,
                "creating a link fixture needs the symlink privilege",
            );
            return;
        }
        let runtime = bootstrap_runtime();

        let result = submit_bootstrap(&runtime, workspace.path(), serde_json::json!({}));

        assert!(!result.ok, "{result:?}");
        assert_eq!(result.data.as_ref().unwrap()["config"]["state"], "invalid");
        assert_eq!(result.diagnostics[0]["code"], "invalid_state");
    }

    #[test]
    fn canonical_view_bootstrap_repairs_a_valid_config_without_source_sets() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "workPath: .work\nformat: DESIGNER\n",
        )
        .unwrap();
        let runtime = bootstrap_runtime();

        let result = submit_bootstrap(&runtime, workspace.path(), serde_json::json!({}));

        assert!(result.ok, "{result:?}");
        let data = result.data.as_ref().expect("bootstrap data");
        assert_eq!(data["config"]["state"], "configured");
        assert_eq!(data["setup"]["path"], "v8project.yaml");
        assert_eq!(data["setup"]["content"], serde_json::Value::Null, "{data}");
        assert_eq!(data["setup"]["sourceSetExample"]["name"], "main");
        assert_eq!(data["setup"]["sourceSetExample"]["type"], "CONFIGURATION");
        assert_eq!(data["setup"]["sourceSetExample"]["path"], "src");
        assert!(workspace.path().join("v8project.yaml").is_file());
        assert_eq!(
            std::fs::read(workspace.path().join("v8project.yaml")).unwrap(),
            b"workPath: .work\nformat: DESIGNER\n"
        );
    }

    #[test]
    fn canonical_view_bootstrap_does_not_equate_git_presence_with_repository_readiness() {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let git = std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(workspace.path())
            .status()
            .unwrap();
        assert!(git.success());
        let runtime = bootstrap_runtime();

        let result = submit_root_verdict(&runtime, workspace.path());

        assert!(result.ok, "{result:?}");
        let data = result.data.as_ref().expect("verdict data");
        assert_eq!(data["repositoryReady"], false, "{data}");
        assert!(data["checks"].is_array(), "{data}");
        assert!(data["diagnostics"].is_array(), "{data}");
    }

    /// `view {}` reports facts: it starts no repository process, runs no
    /// project-health inspection and opens no object descriptor, while
    /// `check {}` on the same workspace still computes the verdict (#1457).
    /// A Git workspace whose root check reports three diagnostic groups per
    /// extension set and three more: generated content and an unclassified
    /// dump inside each long-named set root.
    pub(crate) fn root_check_groups_fixture(extensions: usize) -> tempfile::TempDir {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path();
        let mut project = String::from(
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        );
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(root.join("src/Configuration.xml"), "<MetaDataObject/>").unwrap();
        for n in 0..extensions {
            // Long set roots make each group heavy enough to stop a page by
            // bytes; deeper than 150 characters Git for Windows refuses the
            // paths without `core.longpaths` once the temporary root is added.
            let path = format!("ext{n:02}{}", "x".repeat(150));
            project.push_str(&format!(
                "  - name: ext{n:02}\n    type: EXTENSION\n    path: {path}\n"
            ));
            std::fs::create_dir_all(root.join(&path).join(".build")).unwrap();
            std::fs::write(root.join(&path).join("ConfigDumpInfo.xml"), "x").unwrap();
        }
        std::fs::write(root.join("v8project.yaml"), project).unwrap();
        for args in [&["init", "--quiet"][..], &["add", "."][..]] {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(root)
                .status()
                .unwrap()
                .success());
        }
        workspace
    }

    /// Walks a root check from its first page by cursor and returns the pages.
    fn walk_root_check(
        runtime: &V5CanonicalInvocationRuntime,
        workspace: &std::path::Path,
        limit: usize,
    ) -> Vec<DomainResult> {
        let first = serde_json::json!({"limit": limit});
        let mut pages = vec![submit_canonical(
            runtime,
            workspace,
            ToolIdentity::Check,
            first,
        )];
        while let Some(cursor) = pages.last().unwrap().cursor.clone() {
            let next = &pages.last().unwrap().next[0];
            assert_eq!(next["tool"], "unica.check", "{next}");
            assert_eq!(
                next["args"],
                serde_json::json!({"limit": limit, "cursor": cursor}),
                "the continuation repeats the question"
            );
            let page = submit_canonical(
                runtime,
                workspace,
                ToolIdentity::Check,
                next["args"].clone(),
            );
            assert!(page.ok, "{page:?}");
            pages.push(page);
            assert!(pages.len() < 100, "a walk must end");
        }
        pages
    }

    fn verdict_without_groups(page: &DomainResult) -> serde_json::Value {
        let mut data = page.data.clone().unwrap();
        data.as_object_mut().unwrap().remove("diagnostics");
        data
    }

    fn groups(page: &DomainResult) -> Vec<serde_json::Value> {
        page.data.as_ref().unwrap()["diagnostics"]
            .as_array()
            .unwrap()
            .clone()
    }

    /// #1119 (9c): a root check pages its diagnostic groups by count and by
    /// encoded size. Every page carries the whole verdict and the number of
    /// groups; the walk returns every group once, in the first answer's order,
    /// and a continuation reads the saved verdict without inspecting again.
    #[test]
    fn root_check_pages_its_groups_by_count_and_bytes_with_the_whole_verdict() {
        use unica_project_health::inspection::health_inspection_probe::inspections_for_test;

        let workspace = root_check_groups_fixture(20);
        let runtime = bootstrap_runtime();
        let whole = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Check,
            serde_json::json!({"limit": 50}),
        );
        assert!(whole.ok, "{whole:?}");
        let count = whole.data.as_ref().unwrap()["diagnosticCount"]
            .as_u64()
            .unwrap() as usize;
        assert!(count > 50, "the fixture needs more groups than a page");

        for (limit, stop) in [(7, "limit"), (50, "bytes")] {
            let inspected = inspections_for_test(workspace.path());
            let pages = walk_root_check(&runtime, workspace.path(), limit);
            assert_eq!(
                inspections_for_test(workspace.path()),
                inspected + 1,
                "continuations read the saved verdict"
            );
            assert!(pages.len() > 1, "limit {limit}: one page holds everything");
            let verdict = verdict_without_groups(&pages[0]);
            assert_eq!(verdict["diagnosticCount"], count);
            assert_eq!(verdict["status"], "failed");
            let mut walked = Vec::new();
            for (index, page) in pages.iter().enumerate() {
                assert_eq!(verdict_without_groups(page), verdict, "page {index}");
                let last = index + 1 == pages.len();
                let stopped = &page.page.as_ref().unwrap()["stoppedBy"];
                assert_eq!(
                    stopped,
                    if last { "complete" } else { stop },
                    "page {index}"
                );
                assert_eq!(page.cursor.is_none(), last, "page {index}");
                let size = serde_json::to_vec(page).unwrap().len();
                if !last && stop == "bytes" {
                    assert!(size <= 64 * 1024, "page {index}: {size} bytes");
                    assert!(groups(page).len() > 1, "small groups share a page");
                }
                if stop == "limit" && !last {
                    assert_eq!(groups(page).len(), limit, "page {index}");
                }
                walked.extend(groups(page));
            }
            let mut first_answer = groups(&whole);
            first_answer.extend(
                walk_root_check(&runtime, workspace.path(), 50)
                    .iter()
                    .skip(1)
                    .flat_map(groups),
            );
            assert_eq!(
                walked.len(),
                count,
                "limit {limit}: groups lost or repeated"
            );
            assert_eq!(walked, first_answer, "limit {limit}: the order holds");
        }
    }

    /// #1119 (9c): a root check cursor answers only its own question. A forged
    /// cursor, another workspace or another limit refuse it; an empty cursor
    /// opens page one, and a limit over 50 is refused.
    #[test]
    fn root_check_cursor_answers_only_the_question_that_issued_it() {
        let workspace = root_check_groups_fixture(3);
        let other = root_check_groups_fixture(3);
        let runtime = bootstrap_runtime();
        let call = |workspace: &std::path::Path, arguments| {
            submit_canonical(&runtime, workspace, ToolIdentity::Check, arguments)
        };
        let first = call(workspace.path(), serde_json::json!({"limit": 2}));
        let cursor = first.cursor.clone().expect("a cut page issues a cursor");
        assert_eq!(
            call(
                workspace.path(),
                serde_json::json!({"limit": 2, "cursor": ""})
            ),
            first,
            "an empty cursor opens page one"
        );
        assert_eq!(
            call(
                workspace.path(),
                serde_json::json!({"limit": 2, "cursor": null})
            ),
            first,
            "a null cursor opens page one"
        );
        // One flipped character of the authenticated token, and the token
        // with a character more.
        let flipped = if cursor.ends_with('0') { '1' } else { '0' };
        let forged = format!("{}{flipped}", &cursor[..cursor.len() - 1]);
        for (target, arguments) in [
            (
                workspace.path(),
                serde_json::json!({"limit": 2, "cursor": "opaque"}),
            ),
            (
                workspace.path(),
                serde_json::json!({"limit": 2, "cursor": format!("{cursor}0")}),
            ),
            (
                workspace.path(),
                serde_json::json!({"limit": 2, "cursor": forged}),
            ),
            (
                workspace.path(),
                serde_json::json!({"limit": 3, "cursor": cursor}),
            ),
            (
                other.path(),
                serde_json::json!({"limit": 2, "cursor": cursor}),
            ),
        ] {
            let refused = call(target, arguments.clone());
            assert!(!refused.ok, "{arguments}: {refused:?}");
            assert_eq!(
                refused.diagnostics[0]["code"], "invalid_cursor",
                "{arguments}: {refused:?}"
            );
        }
        for limit in [0, 51] {
            let refused = call(workspace.path(), serde_json::json!({"limit": limit}));
            assert_eq!(refused.diagnostics[0]["code"], "bad_value", "{refused:?}");
        }
        let refused = call(workspace.path(), serde_json::json!({"cursor": 7}));
        assert_eq!(refused.diagnostics[0]["code"], "bad_value", "{refused:?}");
    }

    /// #1119 (9c): a workspace verdict is saved in the check snapshot store.
    /// A cursor whose verdict that store does not hold — as after a daemon
    /// restart — is refused, never answered as a complete page; a verdict the
    /// store cannot hold is refused before any cursor is promised.
    #[test]
    fn a_root_check_cursor_without_its_saved_verdict_is_refused() {
        let workspace = root_check_groups_fixture(3);
        let limited = serde_json::json!({"limit": 2});
        let page = submit_canonical(
            &bootstrap_runtime(),
            workspace.path(),
            ToolIdentity::Check,
            limited.clone(),
        );
        let cursor = page.cursor.clone().unwrap();
        let restarted = submit_canonical(
            &bootstrap_runtime(),
            workspace.path(),
            ToolIdentity::Check,
            serde_json::json!({"limit": 2, "cursor": cursor}),
        );
        assert!(!restarted.ok, "{restarted:?}");
        assert_eq!(restarted.diagnostics[0]["code"], "invalid_cursor");
        assert!(
            restarted.page.is_none() && restarted.data.is_none(),
            "{restarted:?}"
        );

        let full = bootstrap_runtime().with_detached_check_snapshots(
            unica_navigation::result_store::ViewCursorStore::new(400),
        );
        let refused = submit_canonical(&full, workspace.path(), ToolIdentity::Check, limited);
        assert!(!refused.ok, "{refused:?}");
        assert_eq!(refused.diagnostics[0]["code"], "result_too_large");
        assert!(refused.cursor.is_none(), "{refused:?}");
    }

    /// #1119 (9c): the product runtime saves the workspace verdict in the read
    /// service's check snapshots, so one retention budget covers both.
    #[test]
    fn the_root_check_shares_the_read_service_check_snapshots() {
        let runtime = bootstrap_runtime();
        let service = runtime
            .service
            .check_snapshots()
            .expect("the read service keeps check snapshots");
        assert!(
            Arc::ptr_eq(&runtime.check_snapshots, &service),
            "the root check must save its pages in the read service store"
        );
    }

    /// #1119 (9c): a cursor answers only the check that issued it. A
    /// workspace verdict cursor does not continue a node check, and a node
    /// check cursor does not continue the workspace verdict, although both
    /// live in one store.
    #[test]
    fn root_and_node_check_cursors_do_not_continue_each_other() {
        let workspace = tempfile::tempdir().unwrap();
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-xdto-check");
        copy_tree_for_test(&fixture, workspace.path());
        for args in [&["init", "--quiet"][..], &["add", "."][..]] {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(workspace.path())
                .status()
                .unwrap()
                .success());
        }
        let runtime = bootstrap_runtime();
        let call = |arguments| {
            submit_canonical(&runtime, workspace.path(), ToolIdentity::Check, arguments)
        };
        let view = |at: &str| {
            submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::View,
                serde_json::json!({"at": at}),
            )
        };
        // Ветвь и пакет выбираются по виду и имени: корень перечисляет и
        // пустые виды (#2153), а коллекция идёт в порядке `ChildObjects`.
        let packages = "main:XDTOPackage".to_owned();
        assert!(view("main:Configuration").data.unwrap()["branches"]
            .as_array()
            .unwrap()
            .iter()
            .any(|branch| branch["at"] == packages.as_str()));
        let node = view(&packages).data.unwrap()["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["title"] == "Unsupported")
            .and_then(|item| item["at"].as_str())
            .unwrap()
            .to_owned();
        let node_page = call(serde_json::json!({"at": node, "limit": 1}));
        let node_cursor = node_page.cursor.clone().expect("the node check is cut");
        let root_page = call(serde_json::json!({"limit": 1}));
        let root_cursor = root_page
            .cursor
            .clone()
            .expect("the workspace verdict is cut");
        for arguments in [
            serde_json::json!({"at": node, "limit": 1, "cursor": root_cursor}),
            serde_json::json!({"limit": 1, "cursor": node_cursor}),
        ] {
            let refused = call(arguments.clone());
            assert!(!refused.ok, "{arguments}: {refused:?}");
            assert_eq!(
                refused.diagnostics[0]["code"], "invalid_cursor",
                "{arguments}: {refused:?}"
            );
        }
    }

    fn copy_tree_for_test(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree_for_test(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    #[test]
    fn canonical_view_bootstrap_leaves_project_health_to_the_root_check() {
        use unica_process::runner::process_launch_probe::launches_under_for_test;
        use unica_project_health::inspection::health_inspection_probe::inspections_for_test;

        let (workspace, source) = source_selection_read_fixture();
        add_unrelated_catalogs(&source, 3);
        for args in [&["init", "--quiet"][..], &["add", "."][..]] {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(workspace.path())
                .status()
                .unwrap()
                .success());
        }
        let runtime = bootstrap_runtime();
        let reads = RetainedReadProbe::watch(workspace.path());

        let view = submit_bootstrap(&runtime, workspace.path(), serde_json::json!({}));

        assert!(view.ok, "{view:?}");
        let data = view.data.as_ref().unwrap();
        for field in [
            "ready",
            "discoveredReady",
            "repositoryReady",
            "readinessState",
            "checks",
            "diagnostics",
        ] {
            assert!(data.get(field).is_none(), "view leaked {field}: {data}");
        }
        assert!(view
            .next
            .iter()
            .any(|next| next["tool"] == "unica.check" && next["args"] == serde_json::json!({})));
        assert!(
            view.next
                .iter()
                .any(|next| next["tool"] == "unica.view"
                    && next["args"]["at"] == "main:Configuration"),
            "a selected Platform XML source keeps its root link: {view:?}"
        );
        assert_eq!(inspections_for_test(workspace.path()), 0);
        assert_eq!(
            launches_under_for_test(workspace.path()),
            Vec::<String>::new()
        );
        let view_reads = reads.take();
        assert!(
            view_reads
                .iter()
                .all(|read| !format!("{read:?}").contains("Catalogs")),
            "view {{}} read object descriptors: {view_reads:?}"
        );

        let verdict = submit_root_verdict(&runtime, workspace.path());

        assert!(verdict.ok, "{verdict:?}");
        assert!(verdict.data.as_ref().unwrap()["checks"].is_array());
        assert_eq!(inspections_for_test(workspace.path()), 1);
        assert!(
            launches_under_for_test(workspace.path())
                .iter()
                .any(|program| program.ends_with("git") || program.ends_with("git.exe")),
            "the root check must still inspect the repository"
        );
    }

    #[test]
    fn canonical_invalid_view_address_points_back_to_bootstrap() {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let runtime = bootstrap_runtime();

        let result = submit_bootstrap(
            &runtime,
            workspace.path(),
            serde_json::json!({"at": "Catalog.Items"}),
        );

        assert!(!result.ok, "{result:?}");
        assert!(
            result.diagnostics[0]["message"]
                .as_str()
                .unwrap()
                .contains("<sourceSet>:<Kind>"),
            "{result:?}"
        );
        assert_eq!(result.next[0]["tool"], "unica.view");
        assert_eq!(result.next[0]["args"], serde_json::json!({}));
    }

    /// Audit the text-search walk: it reaches sources only through the
    /// retained root of the actor capability it receives, checks that
    /// capability's closed deadline and keeps its bounds.
    fn audit_text_search_walk(source: &str) -> Result<(), String> {
        use quote::ToTokens;
        use syn::visit::Visit;
        fn tokens(node: &impl ToTokens) -> String {
            node.to_token_stream().to_string()
        }
        fn expression_is(expression: &syn::Expr, expected: &str) -> bool {
            let expected = syn::parse_str::<syn::Expr>(expected)
                .expect("the closed authority expression fixture parses");
            tokens(expression) == tokens(&expected)
        }
        let file = syn::parse_file(source).map_err(|error| error.to_string())?;
        // The walk and the subtree descent it starts from.
        let walk = ["walk_source", "retained_subtree"].map(|name| {
            file.items.iter().find_map(|item| match item {
                syn::Item::Fn(function) if function.sig.ident == name => Some(function),
                _ => None,
            })
        });
        let [Some(walk_source), Some(retained_subtree)] = walk else {
            return Err("text search has no `walk_source` or `retained_subtree`".to_string());
        };
        #[derive(Default)]
        struct SearchCalls {
            names: std::collections::BTreeSet<String>,
            closed_deadline_checked: bool,
            foreign_deadline_checked: bool,
        }

        impl<'ast> Visit<'ast> for SearchCalls {
            fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
                self.names.insert(call.method.to_string());
                if call.method == "is_elapsed" {
                    if expression_is(&call.receiver, "source.source.deadline")
                        && call.args.is_empty()
                    {
                        self.closed_deadline_checked = true;
                    } else {
                        self.foreign_deadline_checked = true;
                    }
                }
                syn::visit::visit_expr_method_call(self, call);
            }
        }

        let mut calls = SearchCalls::default();
        calls.visit_block(&walk_source.block);
        calls.visit_block(&retained_subtree.block);
        for required in [
            "retained_root",
            "validate_named_identity",
            "retain_immediate_child_nofollow",
            "open_named_identity_for_read",
            "is_elapsed",
            "is_cancelled",
        ] {
            if !calls.names.contains(required) {
                return Err(format!(
                    "literal search must retain closed `{required}` authority; found {:?}",
                    calls.names
                ));
            }
        }
        if !calls.closed_deadline_checked || calls.foreign_deadline_checked {
            return Err(
                "literal search must check only its exact closed self.deadline authority"
                    .to_string(),
            );
        }
        for ambient in [
            "canonicalize",
            "metadata",
            "read",
            "read_dir",
            "read_to_string",
            "symlink_metadata",
        ] {
            if calls.names.contains(ambient) {
                return Err(format!(
                    "literal search must not use ambient filesystem method `{ambient}`"
                ));
            }
        }
        let body = tokens(&walk_source.block) + &tokens(&retained_subtree.block);
        for bound in [
            "CANONICAL_SEARCH_MAX_DEPTH",
            "CANONICAL_SEARCH_MAX_LINE_BYTES",
            "read_search_line_bounded",
        ] {
            if !body.contains(bound) {
                return Err(format!("literal search must preserve the `{bound}` bound"));
            }
        }
        if !body.contains(&tokens(
            &syn::parse_str::<syn::Expr>("source.source.retained_root()")
                .expect("retained search root expression parses"),
        )) {
            return Err(
                "literal search root must come from the private retained binding".to_string(),
            );
        }
        // Directory width is unbounded: names come only from the retained
        // directory through the ordered spool audited separately.
        if !body.contains(&tokens(
            &syn::parse_str::<syn::Expr>("OrderedNames::collect")
                .expect("ordered names entry point parses"),
        )) {
            return Err(
                "literal search must enumerate retained directories through OrderedNames"
                    .to_string(),
            );
        }
        Ok(())
    }

    fn audit_actor_read_source_capability_api(source: &str) -> Result<(), String> {
        use quote::ToTokens;
        use syn::visit::Visit;

        const CAPABILITY: &str = "ActorReadSourceCapability";

        fn semantic_name(identifier: &syn::Ident) -> String {
            use syn::ext::IdentExt;
            identifier.unraw().to_string()
        }

        fn identifier_is(identifier: &syn::Ident, expected: &str) -> bool {
            semantic_name(identifier) == expected
        }

        fn tokens(node: &impl ToTokens) -> String {
            node.to_token_stream().to_string()
        }

        fn is_daemon_visible(visibility: &syn::Visibility) -> bool {
            let syn::Visibility::Restricted(restricted) = visibility else {
                return false;
            };
            restricted.in_token.is_some()
                && restricted.path.leading_colon.is_none()
                && restricted.path.segments.len() == 3
                && restricted
                    .path
                    .segments
                    .iter()
                    .map(|segment| segment.ident.to_string())
                    .eq(["crate", "infrastructure", "daemon"]
                        .into_iter()
                        .map(str::to_string))
        }

        fn meta_list_is_exact_ident(list: &syn::MetaList, expected: &str) -> bool {
            syn::parse2::<syn::Path>(list.tokens.clone()).is_ok_and(|path| {
                path.leading_colon.is_none() && path.segments.len() == 1 && path.is_ident(expected)
            })
        }

        fn is_exact_dead_code_allow(attributes: &[syn::Attribute]) -> bool {
            matches!(
                attributes,
                [attribute]
                    if attribute.path().is_ident("allow")
                        && matches!(
                            &attribute.meta,
                            syn::Meta::List(list) if meta_list_is_exact_ident(list, "dead_code")
                        )
            )
        }

        fn expression_is(expression: &syn::Expr, expected: &str) -> bool {
            let expected = syn::parse_str::<syn::Expr>(expected)
                .expect("the closed authority expression fixture parses");
            tokens(expression) == tokens(&expected)
        }

        fn signature_is(signature: &syn::Signature, expected: &str) -> bool {
            let expected = syn::parse_str::<syn::Signature>(expected)
                .expect("the closed capability signature fixture parses");
            tokens(signature) == tokens(&expected)
        }

        fn local_initializer<'a>(
            statement: &'a syn::Stmt,
            expected_name: &str,
        ) -> Result<&'a syn::Expr, String> {
            let syn::Stmt::Local(local) = statement else {
                return Err(format!(
                    "`{expected_name}` must be a local authority binding"
                ));
            };
            let syn::Pat::Ident(pattern) = &local.pat else {
                return Err(format!(
                    "`{expected_name}` must use an exact identifier pattern"
                ));
            };
            if pattern.ident != expected_name
                || pattern.by_ref.is_some()
                || pattern.mutability.is_some()
            {
                return Err(format!("unexpected authority local `{}`", pattern.ident));
            }
            let Some(initializer) = &local.init else {
                return Err(format!("`{expected_name}` has no initializer"));
            };
            if initializer.diverge.is_some() {
                return Err(format!(
                    "`{expected_name}` must not carry a let-else fallback"
                ));
            }
            Ok(initializer.expr.as_ref())
        }

        fn call_arguments<'a>(
            expression: &'a syn::Expr,
            expected_function: &str,
        ) -> Result<&'a syn::punctuated::Punctuated<syn::Expr, syn::Token![,]>, String> {
            let syn::Expr::Call(call) = expression else {
                return Err(format!(
                    "authority dataflow must call `{expected_function}`"
                ));
            };
            if !expression_is(call.func.as_ref(), expected_function) {
                return Err(format!(
                    "authority dataflow called `{}` instead of `{expected_function}`",
                    tokens(call.func.as_ref())
                ));
            }
            Ok(&call.args)
        }

        fn audit_builder_body(method: &syn::ImplItemFn) -> Result<(), String> {
            if method.block.stmts.len() != 4 {
                return Err(format!(
                    "logical authority builder must have four closed dataflow statements, found {}",
                    method.block.stmts.len()
                ));
            }
            let source_profile = local_initializer(&method.block.stmts[0], "source_profile")?;
            if !expression_is(source_profile, "self.binding.source_profile()") {
                return Err(
                    "source profile must come directly from the private binding".to_string()
                );
            }

            let platform_profile = local_initializer(&method.block.stmts[1], "platform_profile")?;
            let syn::Expr::Try(platform_profile) = platform_profile else {
                return Err("platform profile must fail closed with `?`".to_string());
            };
            let syn::Expr::MethodCall(to_result) = platform_profile.expr.as_ref() else {
                return Err("platform profile must fail closed through `ok_or_else`".to_string());
            };
            if to_result.method != "ok_or_else" || to_result.args.len() != 1 {
                return Err(
                    "platform profile must fail closed through exact `ok_or_else`".to_string(),
                );
            }
            let from_profile = call_arguments(
                to_result.receiver.as_ref(),
                "crate::domain::source_capabilities::bsl_profile_for_source",
            )?;
            let Some(error_closure) = to_result.args.first() else {
                return Err("platform profile requires one exact error closure".to_string());
            };
            if from_profile.len() != 1
                || !expression_is(&from_profile[0], "source_profile")
                || !expression_is(
                    error_closure,
                    r#"|| "actor-bound logical source has no supported platform profile".to_string()"#,
                )
            {
                return Err(
                    "platform profile must use the bound profile and exact side-effect-free error"
                        .to_string(),
                );
            }

            let read = local_initializer(&method.block.stmts[2], "read")?;
            let read_args = call_arguments(
                read,
                "crate::infrastructure::v13_read_port::ProviderReadAuthority::new_local_snapshot",
            )?;
            let expected_read_args = [
                "self.binding.source_set_name()",
                "self.identity.clone()",
                "self.binding.source_kind()",
                "self.binding.retained_root()",
                "self.read_identity.clone()",
            ];
            if read_args.len() != expected_read_args.len()
                || read_args
                    .iter()
                    .zip(expected_read_args)
                    .any(|(actual, expected)| !expression_is(actual, expected))
            {
                return Err(format!(
                    "provider read authority must use exact binding/identity/fence dataflow; found `{}`",
                    tokens(read)
                ));
            }

            let syn::Stmt::Expr(result, None) = &method.block.stmts[3] else {
                return Err("logical authority builder must return one exact authority".to_string());
            };
            let ok_args = call_arguments(result, "Ok")?;
            let Some(authority) = ok_args.first().filter(|_| ok_args.len() == 1) else {
                return Err(
                    "logical authority builder must return exactly one `Ok` value".to_string(),
                );
            };
            let authority_args = call_arguments(
                authority,
                "crate::infrastructure::v13_read::LogicalViewReadAuthority::with_read_authority_and_registration_cache",
            )?;
            let expected_authority_args = [
                "cancellation",
                "read",
                "platform_profile",
                "self.deadline",
                "Arc::clone(&self.registration_cache)",
            ];
            if authority_args.len() != expected_authority_args.len()
                || authority_args
                    .iter()
                    .zip(expected_authority_args)
                    .any(|(actual, expected)| !expression_is(actual, expected))
            {
                return Err(format!(
                    "logical authority must preserve cancellation/read/profile/deadline/actor-cache exactly; found `{}`",
                    tokens(authority)
                ));
            }
            Ok(())
        }

        fn is_exact_cfg_test(attributes: &[syn::Attribute]) -> bool {
            attributes.iter().any(|attribute| {
                attribute.path().leading_colon.is_none()
                    && attribute.path().segments.len() == 1
                    && attribute.path().is_ident("cfg")
                    && matches!(&attribute.meta, syn::Meta::List(list) if cfg_predicate_is(list, "test"))
            })
        }

        fn cfg_predicate_is(list: &syn::MetaList, expected: &str) -> bool {
            let Ok(predicate) = syn::parse2::<syn::Meta>(list.tokens.clone()) else {
                return false;
            };
            match (expected, predicate) {
                ("test", syn::Meta::Path(path)) => {
                    path.leading_colon.is_none()
                        && path.segments.len() == 1
                        && path.is_ident("test")
                }
                ("not(test)", syn::Meta::List(not)) => {
                    not.path.leading_colon.is_none()
                        && not.path.segments.len() == 1
                        && not.path.is_ident("not")
                        && syn::parse2::<syn::Path>(not.tokens).is_ok_and(|path| {
                            path.leading_colon.is_none()
                                && path.segments.len() == 1
                                && path.is_ident("test")
                        })
                }
                _ => false,
            }
        }

        fn derive_payload_is_proven_builtin(list: &syn::MetaList) -> bool {
            use syn::parse::Parser;

            let parser = syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated;
            let Ok(paths) = parser.parse2(list.tokens.clone()) else {
                return false;
            };
            if paths.is_empty() || paths.trailing_punct() {
                return false;
            }
            let names = paths
                .iter()
                .map(|path| {
                    (path.leading_colon.is_none() && path.segments.len() == 1)
                        .then(|| path.segments.first())
                        .flatten()
                        .filter(|segment| matches!(segment.arguments, syn::PathArguments::None))
                        .map(|segment| segment.ident.to_string())
                })
                .collect::<Option<Vec<_>>>();
            let Some(names) = names else {
                return false;
            };
            const PROVEN_DERIVE_PAYLOADS: [&[&str]; 4] = [
                &["Clone"],
                &["Debug"],
                &["Default"],
                &["Debug", "Clone", "Copy", "PartialEq", "Eq"],
            ];
            PROVEN_DERIVE_PAYLOADS.iter().any(|expected| {
                names
                    .iter()
                    .map(String::as_str)
                    .eq(expected.iter().copied())
            })
        }

        fn attribute_is_proven_builtin(attribute: &syn::Attribute) -> bool {
            let path = attribute.path();
            if path.leading_colon.is_some() || path.segments.len() != 1 {
                return false;
            }
            match (
                &attribute.meta,
                path.segments.first().map(|segment| &segment.ident),
            ) {
                (syn::Meta::List(list), Some(name)) if name == "allow" => {
                    meta_list_is_exact_ident(list, "dead_code")
                }
                (syn::Meta::List(list), Some(name)) if name == "derive" => {
                    derive_payload_is_proven_builtin(list)
                }
                (syn::Meta::List(list), Some(name)) if name == "cfg" => {
                    cfg_predicate_is(list, "test") || cfg_predicate_is(list, "not(test)")
                }
                (syn::Meta::NameValue(value), Some(name)) if name == "doc" => {
                    matches!(&value.value, syn::Expr::Lit(literal) if matches!(literal.lit, syn::Lit::Str(_)))
                }
                _ => false,
            }
        }

        fn use_tree_can_shadow_proven_attribute(tree: &syn::UseTree) -> bool {
            const PROVEN_ATTRIBUTE_NAMES: [&str; 10] = [
                "allow",
                "cfg",
                "derive",
                "doc",
                "Clone",
                "Copy",
                "Debug",
                "Default",
                "Eq",
                "PartialEq",
            ];
            let is_proven_name =
                |name: &syn::Ident| PROVEN_ATTRIBUTE_NAMES.contains(&semantic_name(name).as_str());
            match tree {
                syn::UseTree::Name(name) => is_proven_name(&name.ident),
                syn::UseTree::Rename(rename) => {
                    is_proven_name(&rename.ident) || is_proven_name(&rename.rename)
                }
                syn::UseTree::Path(path) => {
                    is_proven_name(&path.ident)
                        || use_tree_can_shadow_proven_attribute(path.tree.as_ref())
                }
                syn::UseTree::Group(group) => {
                    group.items.iter().any(use_tree_can_shadow_proven_attribute)
                }
                syn::UseTree::Glob(_) => true,
            }
        }

        #[derive(Default)]
        struct CapabilityPathFinder {
            found: bool,
        }

        impl<'ast> Visit<'ast> for CapabilityPathFinder {
            fn visit_path_segment(&mut self, segment: &'ast syn::PathSegment) {
                if identifier_is(&segment.ident, CAPABILITY) {
                    self.found = true;
                }
                syn::visit::visit_path_segment(self, segment);
            }
        }

        fn type_mentions_capability(node: &syn::Type) -> bool {
            let mut finder = CapabilityPathFinder::default();
            finder.visit_type(node);
            finder.found
        }

        fn signature_mentions_capability(node: &syn::Signature) -> bool {
            let mut finder = CapabilityPathFinder::default();
            finder.visit_signature(node);
            finder.found
        }

        fn item_type_mentions_capability(node: &syn::ItemType) -> bool {
            let mut finder = CapabilityPathFinder::default();
            finder.visit_item_type(node);
            finder.found
        }

        fn use_tree_mentions_capability(tree: &syn::UseTree) -> bool {
            match tree {
                syn::UseTree::Name(name) => identifier_is(&name.ident, CAPABILITY),
                syn::UseTree::Rename(rename) => {
                    identifier_is(&rename.ident, CAPABILITY)
                        || identifier_is(&rename.rename, CAPABILITY)
                }
                syn::UseTree::Path(path) => {
                    identifier_is(&path.ident, CAPABILITY)
                        || use_tree_mentions_capability(path.tree.as_ref())
                }
                syn::UseTree::Group(group) => group.items.iter().any(use_tree_mentions_capability),
                syn::UseTree::Glob(_) => false,
            }
        }

        fn use_tree_can_shadow_inert_macro(tree: &syn::UseTree) -> bool {
            const INERT_MACROS: [&str; 3] = ["format", "matches", "vec"];
            match tree {
                syn::UseTree::Name(name) => {
                    INERT_MACROS.contains(&semantic_name(&name.ident).as_str())
                }
                syn::UseTree::Rename(rename) => {
                    INERT_MACROS.contains(&semantic_name(&rename.ident).as_str())
                        || INERT_MACROS.contains(&semantic_name(&rename.rename).as_str())
                }
                syn::UseTree::Path(path) => {
                    INERT_MACROS.contains(&semantic_name(&path.ident).as_str())
                        || use_tree_can_shadow_inert_macro(path.tree.as_ref())
                }
                syn::UseTree::Group(group) => {
                    group.items.iter().any(use_tree_can_shadow_inert_macro)
                }
                syn::UseTree::Glob(_) => true,
            }
        }

        fn inert_macro_payload_is_closed(mac: &syn::Macro) -> bool {
            const FORBIDDEN_IDENTIFIERS: [&str; 15] = [
                CAPABILITY,
                "issue_read_source",
                "const",
                "enum",
                "extern",
                "fn",
                "impl",
                "macro",
                "macro_rules",
                "pub",
                "static",
                "struct",
                "trait",
                "type",
                "use",
            ];
            fn tokens_are_closed(stream: proc_macro2::TokenStream) -> bool {
                stream.into_iter().all(|token| match token {
                    proc_macro2::TokenTree::Ident(identifier) => {
                        !FORBIDDEN_IDENTIFIERS.contains(&semantic_name(&identifier).as_str())
                    }
                    proc_macro2::TokenTree::Punct(punctuation) => punctuation.as_char() != '!',
                    proc_macro2::TokenTree::Group(group) => tokens_are_closed(group.stream()),
                    proc_macro2::TokenTree::Literal(_) => true,
                })
            }
            tokens_are_closed(mac.tokens.clone())
        }

        fn macro_is_proven_inert(mac: &syn::Macro) -> bool {
            mac.path.leading_colon.is_none()
                && mac.path.segments.len() == 1
                && matches!(
                    mac.path.segments.first(),
                    Some(segment)
                        if matches!(segment.ident.to_string().as_str(), "format" | "matches" | "vec")
                            && matches!(segment.arguments, syn::PathArguments::None)
                )
                && inert_macro_payload_is_closed(mac)
        }

        struct CapabilityConstruction<'ast> {
            expression: &'ast syn::ExprStruct,
            owner_impl: Option<String>,
            function: Option<String>,
        }

        struct CapabilityFunctionReference<'ast> {
            owner_impl: Option<String>,
            signature: String,
            method: Option<&'ast syn::ImplItemFn>,
        }

        #[derive(Default)]
        struct CapabilityItems<'ast> {
            declarations: Vec<&'ast syn::ItemStruct>,
            implementations: Vec<&'ast syn::ItemImpl>,
            aliases: Vec<String>,
            macro_references: Vec<String>,
            attribute_references: Vec<String>,
            item_references: Vec<String>,
            function_references: Vec<CapabilityFunctionReference<'ast>>,
            constructions: Vec<CapabilityConstruction<'ast>>,
            capability_path_references: usize,
            issuer_path_references: usize,
            issuance_calls: Vec<(Option<String>, Option<String>, &'ast syn::ExprMethodCall)>,
            current_impl: Option<String>,
            current_function: Option<String>,
            item_macro_depth: usize,
        }

        impl<'ast> Visit<'ast> for CapabilityItems<'ast> {
            fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
                if !is_exact_cfg_test(&item.attrs) {
                    syn::visit::visit_item_mod(self, item);
                }
            }

            fn visit_item_struct(&mut self, item: &'ast syn::ItemStruct) {
                if is_exact_cfg_test(&item.attrs) {
                    return;
                }
                if identifier_is(&item.ident, CAPABILITY) {
                    self.declarations.push(item);
                }
                syn::visit::visit_item_struct(self, item);
            }

            fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
                if is_exact_cfg_test(&item.attrs) {
                    return;
                }
                let owner = tokens(item.self_ty.as_ref());
                if type_mentions_capability(item.self_ty.as_ref()) {
                    self.implementations.push(item);
                }
                let previous = self.current_impl.replace(owner);
                syn::visit::visit_item_impl(self, item);
                self.current_impl = previous;
            }

            fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
                if is_exact_cfg_test(&item.attrs) {
                    return;
                }
                if signature_mentions_capability(&item.sig) {
                    self.function_references.push(CapabilityFunctionReference {
                        owner_impl: None,
                        signature: tokens(&item.sig),
                        method: None,
                    });
                }
                let previous = self.current_function.replace(item.sig.ident.to_string());
                syn::visit::visit_item_fn(self, item);
                self.current_function = previous;
            }

            fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
                if is_exact_cfg_test(&item.attrs) {
                    return;
                }
                if signature_mentions_capability(&item.sig) {
                    self.function_references.push(CapabilityFunctionReference {
                        owner_impl: self.current_impl.clone(),
                        signature: tokens(&item.sig),
                        method: Some(item),
                    });
                }
                let previous = self.current_function.replace(item.sig.ident.to_string());
                syn::visit::visit_impl_item_fn(self, item);
                self.current_function = previous;
            }

            fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
                if is_exact_cfg_test(&item.attrs) {
                    return;
                }
                if signature_mentions_capability(&item.sig) {
                    self.function_references.push(CapabilityFunctionReference {
                        owner_impl: None,
                        signature: tokens(&item.sig),
                        method: None,
                    });
                }
                syn::visit::visit_trait_item_fn(self, item);
            }

            fn visit_foreign_item_fn(&mut self, item: &'ast syn::ForeignItemFn) {
                if is_exact_cfg_test(&item.attrs) {
                    return;
                }
                if signature_mentions_capability(&item.sig) {
                    self.function_references.push(CapabilityFunctionReference {
                        owner_impl: None,
                        signature: tokens(&item.sig),
                        method: None,
                    });
                }
                syn::visit::visit_foreign_item_fn(self, item);
            }

            fn visit_field(&mut self, field: &'ast syn::Field) {
                if is_exact_cfg_test(&field.attrs) {
                    return;
                }
                if type_mentions_capability(&field.ty) {
                    self.item_references.push(tokens(field));
                }
                syn::visit::visit_field(self, field);
            }

            fn visit_item_const(&mut self, item: &'ast syn::ItemConst) {
                if is_exact_cfg_test(&item.attrs) {
                    return;
                }
                if type_mentions_capability(&item.ty) {
                    self.item_references.push(tokens(item));
                }
                syn::visit::visit_item_const(self, item);
            }

            fn visit_item_static(&mut self, item: &'ast syn::ItemStatic) {
                if is_exact_cfg_test(&item.attrs) {
                    return;
                }
                if type_mentions_capability(&item.ty) {
                    self.item_references.push(tokens(item));
                }
                syn::visit::visit_item_static(self, item);
            }

            fn visit_item_type(&mut self, item: &'ast syn::ItemType) {
                if is_exact_cfg_test(&item.attrs) {
                    return;
                }
                if item_type_mentions_capability(item) {
                    self.aliases.push(tokens(item));
                }
                syn::visit::visit_item_type(self, item);
            }

            fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
                if is_exact_cfg_test(&item.attrs) {
                    return;
                }
                if use_tree_mentions_capability(&item.tree) {
                    self.aliases.push(tokens(item));
                }
                if use_tree_can_shadow_inert_macro(&item.tree) {
                    self.macro_references.push(tokens(item));
                }
                if use_tree_can_shadow_proven_attribute(&item.tree) {
                    self.attribute_references.push(tokens(item));
                }
                syn::visit::visit_item_use(self, item);
            }

            fn visit_item_extern_crate(&mut self, item: &'ast syn::ItemExternCrate) {
                if is_exact_cfg_test(&item.attrs) {
                    return;
                }
                const PROVEN_ATTRIBUTE_NAMES: [&str; 10] = [
                    "allow",
                    "cfg",
                    "derive",
                    "doc",
                    "Clone",
                    "Copy",
                    "Debug",
                    "Default",
                    "Eq",
                    "PartialEq",
                ];
                if PROVEN_ATTRIBUTE_NAMES.contains(&semantic_name(&item.ident).as_str())
                    || item.rename.as_ref().is_some_and(|(_, rename)| {
                        PROVEN_ATTRIBUTE_NAMES.contains(&semantic_name(rename).as_str())
                    })
                {
                    self.attribute_references.push(tokens(item));
                }
                syn::visit::visit_item_extern_crate(self, item);
            }

            fn visit_attribute(&mut self, attribute: &'ast syn::Attribute) {
                if !attribute_is_proven_builtin(attribute) {
                    self.attribute_references.push(tokens(attribute));
                }
                syn::visit::visit_attribute(self, attribute);
            }

            fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
                if is_exact_cfg_test(&item.attrs) {
                    return;
                }
                self.item_macro_depth += 1;
                self.visit_macro(&item.mac);
                self.item_macro_depth -= 1;
            }

            fn visit_stmt_macro(&mut self, statement: &'ast syn::StmtMacro) {
                if !is_exact_cfg_test(&statement.attrs) {
                    self.visit_macro(&statement.mac);
                }
            }

            fn visit_expr_macro(&mut self, expression: &'ast syn::ExprMacro) {
                if !is_exact_cfg_test(&expression.attrs) {
                    self.visit_macro(&expression.mac);
                }
            }

            fn visit_macro(&mut self, mac: &'ast syn::Macro) {
                if self.item_macro_depth != 0 || !macro_is_proven_inert(mac) {
                    self.macro_references.push(tokens(mac));
                }
            }

            fn visit_expr_struct(&mut self, expression: &'ast syn::ExprStruct) {
                let path_mentions = expression
                    .path
                    .segments
                    .iter()
                    .any(|segment| identifier_is(&segment.ident, CAPABILITY));
                let qualified_mentions = expression
                    .qself
                    .as_ref()
                    .is_some_and(|qualified| type_mentions_capability(qualified.ty.as_ref()));
                if path_mentions || qualified_mentions {
                    self.constructions.push(CapabilityConstruction {
                        expression,
                        owner_impl: self.current_impl.clone(),
                        function: self.current_function.clone(),
                    });
                }
                syn::visit::visit_expr_struct(self, expression);
            }

            fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
                if identifier_is(&call.method, "issue_read_source") {
                    self.issuance_calls.push((
                        self.current_impl.clone(),
                        self.current_function.clone(),
                        call,
                    ));
                }
                syn::visit::visit_expr_method_call(self, call);
            }

            fn visit_path_segment(&mut self, segment: &'ast syn::PathSegment) {
                if identifier_is(&segment.ident, CAPABILITY) {
                    self.capability_path_references += 1;
                }
                if identifier_is(&segment.ident, "issue_read_source") {
                    self.issuer_path_references += 1;
                }
                syn::visit::visit_path_segment(self, segment);
            }
        }

        let file = syn::parse_file(source)
            .map_err(|error| format!("actor capability source must parse as Rust AST: {error}"))?;
        let mut found = CapabilityItems::default();
        found.visit_file(&file);
        if !found.attribute_references.is_empty() {
            return Err(format!(
                "opaque, path-qualified or shadowed production attributes could generate actor read capability: {:?}",
                found.attribute_references
            ));
        }
        if !found.macro_references.is_empty() {
            return Err(format!(
                "opaque production macros or shadowing imports could generate actor read capability: {:?}",
                found.macro_references
            ));
        }
        if !found.aliases.is_empty() {
            return Err(format!(
                "actor read capability must not be reopened through a type/use alias: {:?}",
                found.aliases
            ));
        }
        if found.capability_path_references != 6 {
            return Err(format!(
                "actor read capability must have exactly six production path references (impl, four closed issuance signatures, singleton construction), found {}",
                found.capability_path_references
            ));
        }
        if !found.item_references.is_empty() {
            return Err(format!(
                "actor read capability must not escape through another production item: {:?}",
                found.item_references
            ));
        }
        let expected_issuance_methods = [
            r#"
            #[allow(dead_code)]
            pub(in crate::infrastructure::daemon) fn read_sources(
                &self,
            ) -> Result<Vec<ActorReadSourceCapability>, String> {
                let ActorExecutionRevision::LogicalRead(lease) = &self.revision else {
                    return Err("logical read sources are unavailable to a legacy invocation".to_string());
                };
                let parents = lease
                    .parent_sources
                    .lock()
                    .map_err(|_| "parent read leases are poisoned")?;
                lease.sources.iter().chain(parents.iter())
                    .map(|source| self.issue_read_source(source, lease.deadline))
                    .collect()
            }
            "#,
            r#"
            pub(in crate::infrastructure::daemon) fn layout_read_source(
                &self,
                name: &str,
            ) -> Result<ActorReadSourceCapability, String> {
                let ActorExecutionRevision::LayoutRead(lease) = &self.revision else {
                    return Err("typed layout reads require a layout invocation".into());
                };
                let binding = lease.bindings.iter()
                    .find(|binding| binding.source_set_name() == name)
                    .ok_or_else(|| "the selected layout source was not admitted".to_string())?;
                let source = ActorLogicalReadSourceLease {
                    binding: binding.clone(),
                    read_identity: uuid::Uuid::new_v4().to_string(),
                    registration_cache: self.invocation.actor.source_artifacts(binding)?,
                };
                self.issue_read_source(&source, lease.deadline)
            }
            "#,
            r#"
    pub(in crate::infrastructure::daemon) fn apply_range_refusal_source(
        &self,
        at: &QualifiedAddress,
    ) -> Result<ActorReadSourceCapability, String> {
        let ActorExecutionRevision::UnpublishedApply(confirmed) = &self.revision else {
            return Err("range-refusal reads require an unpublished apply invocation".into());
        };
        if confirmed.load(std::sync::atomic::Ordering::Acquire)
            || at.line_range().is_none()
            || !self.original_body_line_targets().contains(at)
        {
            return Err(
                "range-refusal read must name an original read-only Body range before mutation"
                    .into(),
            );
        }
        let binding = &self
            .invocation
            .read_sources
            .iter()
            .find(|source| source.binding.source_set_name() == at.source_set())
            .ok_or_else(|| "range-refusal source was not admitted".to_string())?
            .binding;
        let source = ActorLogicalReadSourceLease {
            binding: binding.clone(),
            read_identity: uuid::Uuid::new_v4().to_string(),
            registration_cache: self.invocation.actor.source_artifacts(binding)?,
        };
        self.issue_read_source(&source, ProviderDeadline::no_deadline())
    }

            "#,
            r#"
            fn issue_read_source(
                &self,
                source: &ActorLogicalReadSourceLease,
                read_deadline: ProviderDeadline,
            ) -> Result<ActorReadSourceCapability, String> {
                self.invocation.actor.validate_binding(&source.binding)?;
                Ok(ActorReadSourceCapability {
                    binding: source.binding.clone(),
                    identity: format!(
                        "{}:{}",
                        self.invocation.workspace_identity_hash.as_str(),
                        source.binding.source_set_name()
                    ),
                    read_identity: source.read_identity.clone(),
                    deadline: read_deadline,
                    registration_cache: Arc::clone(&source.registration_cache),
                })
            }
            "#,
        ]
        .map(|expected| {
            syn::parse_str::<syn::ImplItemFn>(expected)
                .expect("the closed issuance method fixture parses")
        });
        if found.function_references.len() != expected_issuance_methods.len() {
            return Err(format!(
                "actor read capability may appear only in the four closed issuance signatures, found {}",
                found.function_references.len()
            ));
        }
        for expected in &expected_issuance_methods {
            let references = found
                .function_references
                .iter()
                .filter(|reference| {
                    reference.owner_impl.as_deref() == Some("ActorBoundExecution")
                        && reference.signature == tokens(&expected.sig)
                })
                .collect::<Vec<_>>();
            let [reference] = references.as_slice() else {
                return Err(format!(
                    "missing or duplicate closed issuance method `{}`",
                    expected.sig.ident
                ));
            };
            let Some(actual) = reference.method else {
                return Err("capability issuance must be an inherent method".to_string());
            };
            // Visibility belongs to ImplItemFn, not syn::Signature. Comparing the
            // signature alone would let a sibling call the private issuer.
            if tokens(&actual.vis) != tokens(&expected.vis)
                || actual
                    .attrs
                    .iter()
                    .filter(|attribute| !attribute.path().is_ident("doc"))
                    .map(tokens)
                    .collect::<Vec<_>>()
                    != expected.attrs.iter().map(tokens).collect::<Vec<_>>()
                || tokens(&actual.block) != tokens(&expected.block)
            {
                return Err(format!(
                    "issuance method `{}` changed its closed visibility, attributes or authority dataflow",
                    expected.sig.ident
                ));
            }
        }

        if found.issuer_path_references != 0 || found.issuance_calls.len() != 3 {
            return Err(
                "the private issuer may only be called directly by the three closed read entries"
                    .to_string(),
            );
        }
        for (owner, function, call) in &found.issuance_calls {
            let expected = match function.as_deref() {
                Some("read_sources") => "self.issue_read_source(source, lease.deadline)",
                Some("layout_read_source") => "self.issue_read_source(&source, lease.deadline)",
                Some("apply_range_refusal_source") => {
                    "self.issue_read_source(&source, ProviderDeadline::no_deadline())"
                }
                _ => {
                    return Err("private issuance escaped the three closed read entries".to_string())
                }
            };
            if owner.as_deref() != Some("ActorBoundExecution")
                || !expression_is(&syn::Expr::MethodCall((*call).clone()), expected)
            {
                return Err(
                    "private issuance must preserve the exact selected lease and deadline"
                        .to_string(),
                );
            }
        }

        let [construction] = found.constructions.as_slice() else {
            return Err(format!(
                "actor read capability must have exactly one production construction, found {}",
                found.constructions.len()
            ));
        };
        let expression = construction.expression;
        if construction.owner_impl.as_deref() != Some("ActorBoundExecution")
            || construction.function.as_deref() != Some("issue_read_source")
            || expression.qself.is_some()
            || !expression.path.is_ident(CAPABILITY)
            || !expression.attrs.is_empty()
            || expression.rest.is_some()
        {
            return Err(format!(
                "only private ActorBoundExecution::issue_read_source may construct the capability; found owner={:?}, function={:?}, expression=`{}`",
                construction.owner_impl,
                construction.function,
                tokens(expression)
            ));
        }
        let mut construction_fields = std::collections::BTreeMap::new();
        for field in &expression.fields {
            let syn::Member::Named(name) = &field.member else {
                return Err("actor capability construction permits named fields only".to_string());
            };
            if field.attrs.is_empty()
                && field.colon_token.is_some()
                && construction_fields
                    .insert(name.to_string(), &field.expr)
                    .is_none()
            {
                continue;
            }
            return Err(
                "actor capability construction fields must be unique, explicit and attribute-free"
                    .to_string(),
            );
        }
        let expected_construction_fields = std::collections::BTreeMap::from([
            ("binding", "source.binding.clone()"),
            ("deadline", "read_deadline"),
            ("read_identity", "source.read_identity.clone()"),
            (
                "registration_cache",
                "Arc::clone(&source.registration_cache)",
            ),
            (
                "identity",
                "format!(\"{}:{}\", self.invocation.workspace_identity_hash.as_str(), source.binding.source_set_name())",
            ),
        ]);
        if construction_fields.len() != expected_construction_fields.len()
            || expected_construction_fields.iter().any(|(name, expected)| {
                construction_fields
                    .get(*name)
                    .is_none_or(|actual| !expression_is(actual, expected))
            })
        {
            return Err(format!(
                "actor capability construction must use exact binding/identity/fence/deadline/actor-cache dataflow; found `{}`",
                tokens(expression)
            ));
        }
        let [declaration] = found.declarations.as_slice() else {
            return Err(format!(
                "actor read capability must have exactly one named declaration, found {}",
                found.declarations.len()
            ));
        };
        if !is_daemon_visible(&declaration.vis)
            || !declaration.generics.params.is_empty()
            || declaration.generics.where_clause.is_some()
            || !is_exact_dead_code_allow(&declaration.attrs)
        {
            return Err("actor read capability declaration must be exact daemon-visible with no derives or generics".to_string());
        }
        let syn::Fields::Named(named) = &declaration.fields else {
            return Err("actor read capability must have five named private fields".to_string());
        };
        let mut actual_fields = std::collections::BTreeMap::new();
        for field in &named.named {
            if !matches!(field.vis, syn::Visibility::Inherited) || !field.attrs.is_empty() {
                return Err(
                    "actor read capability fields must be private and attribute-free".to_string(),
                );
            }
            let Some(name) = &field.ident else {
                return Err("actor read capability field has no identifier".to_string());
            };
            actual_fields.insert(name.to_string(), tokens(&field.ty));
        }
        let expected_fields = std::collections::BTreeMap::from([
            ("binding".to_string(), "ProviderRootBinding".to_string()),
            ("deadline".to_string(), "ProviderDeadline".to_string()),
            ("read_identity".to_string(), "String".to_string()),
            ("identity".to_string(), "String".to_string()),
            (
                "registration_cache".to_string(),
                "Arc < RegistrationCache >".to_string(),
            ),
        ]);
        if actual_fields != expected_fields {
            return Err(format!(
                "actor read capability fields must remain the exact private typed shape; found {actual_fields:?}"
            ));
        }

        let [implementation] = found.implementations.as_slice() else {
            return Err(format!(
                "actor read capability must have exactly one inherent impl and no trait/extra impls, found {}",
                found.implementations.len()
            ));
        };
        if implementation.trait_.is_some()
            || implementation.defaultness.is_some()
            || implementation.unsafety.is_some()
            || !implementation.generics.params.is_empty()
            || implementation.generics.where_clause.is_some()
            || !is_exact_dead_code_allow(&implementation.attrs)
        {
            return Err("actor read capability permits one exact inherent impl and no trait/default/unsafe/generic impl".to_string());
        }

        let expected_methods = std::collections::BTreeMap::from([
            (
                "source_set_name",
                (
                    true,
                    "fn source_set_name(&self) -> &str",
                    "self.binding.source_set_name()",
                ),
            ),
            (
                "source_kind",
                (
                    true,
                    "const fn source_kind(&self) -> SourceSetKind",
                    "self.binding.source_kind()",
                ),
            ),
            // Аварийный мост строит справочник раскладки на том же корне, что
            // читает `view`, поэтому корень стал видим соседям вместе с видом
            // набора. Ни то, ни другое не даёт обойти учёт: это те же
            // доказанные допуском значения.
            (
                "retained_root",
                (
                    true,
                    "fn retained_root(&self) -> Arc<SourceDirectory>",
                    "self.binding.retained_root()",
                ),
            ),
            (
                "source_format",
                (
                    false,
                    "const fn source_format(&self) -> SourceFormat",
                    "self.binding.source_format()",
                ),
            ),
            (
                "source_profile",
                (
                    false,
                    "const fn source_profile(&self) -> SourceProfile",
                    "self.binding.source_profile()",
                ),
            ),
            (
                "deadline",
                (
                    true,
                    "const fn deadline(&self) -> ProviderDeadline",
                    "self.deadline",
                ),
            ),
            (
                "logical_view_read_authority",
                (
                    true,
                    "fn logical_view_read_authority<'a>(&self, cancellation: &'a CancellationToken,) -> Result<crate::infrastructure::v13_read::LogicalViewReadAuthority<'a>, String>",
                    "",
                ),
            ),
            (
                "revision_identity",
                (
                    true,
                    "fn revision_identity(&self) -> String",
                    "self.read_identity.clone()",
                ),
            ),
            (
                "search_hit_owners",
                (
                    true,
                    "fn search_hit_owners(&self) -> SearchHitOwners<'_>",
                    "SearchHitOwners::new(self.binding.retained_root(), self.binding.source_kind(), &self.registration_cache,)",
                ),
            ),
        ]);
        let mut seen_methods = std::collections::BTreeSet::new();
        for item in &implementation.items {
            let syn::ImplItem::Fn(method) = item else {
                return Err("actor read capability inherent impl permits methods only".to_string());
            };
            let name = method.sig.ident.to_string();
            let Some((sibling_visible, expected_signature, expected_body)) =
                expected_methods.get(name.as_str())
            else {
                return Err(format!(
                    "actor read capability has unapproved method `{name}`"
                ));
            };
            if !seen_methods.insert(name.clone())
                || !method.attrs.is_empty()
                || (*sibling_visible && !is_daemon_visible(&method.vis))
                || (!*sibling_visible && !matches!(method.vis, syn::Visibility::Inherited))
                || !signature_is(&method.sig, expected_signature)
            {
                return Err(format!(
                    "actor read capability method `{name}` has unapproved visibility/signature/attributes; found `{}`, expected `{expected_signature}`",
                    tokens(&method.sig)
                ));
            }
            if name == "logical_view_read_authority" {
                audit_builder_body(method)?;
            } else {
                let [syn::Stmt::Expr(expression, None)] = method.block.stmts.as_slice() else {
                    return Err(format!(
                        "actor capability accessor `{name}` must be one expression"
                    ));
                };
                if !expression_is(expression, expected_body) {
                    return Err(format!(
                        "actor capability accessor `{name}` detached from its private authority field"
                    ));
                }
            }
        }
        if seen_methods.len() != expected_methods.len() {
            return Err(format!(
                "actor read capability method set is incomplete: found {seen_methods:?}"
            ));
        }
        Ok(())
    }

    /// Audit the ordered spool: it may list names only through the retained
    /// directory it receives and never opens or resolves source paths itself.
    fn audit_ordered_search_names(source: &str) -> Result<(), String> {
        #[derive(Default)]
        struct SpoolCalls {
            retained_visits: usize,
            names: std::collections::BTreeSet<String>,
        }

        impl<'ast> syn::visit::Visit<'ast> for SpoolCalls {
            fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
                if call.method == "visit_immediate_names"
                    && matches!(&*call.receiver, syn::Expr::Path(receiver)
                        if receiver.qself.is_none() && receiver.path.is_ident("directory"))
                {
                    self.retained_visits += 1;
                }
                self.names.insert(call.method.to_string());
                syn::visit::visit_expr_method_call(self, call);
            }

            fn visit_path_segment(&mut self, segment: &'ast syn::PathSegment) {
                self.names.insert(segment.ident.to_string());
                syn::visit::visit_path_segment(self, segment);
            }
        }

        let file = syn::parse_file(source)
            .map_err(|error| format!("ordered search names must parse as Rust AST: {error}"))?;
        let cfg_test = |attributes: &[syn::Attribute]| {
            attributes.iter().any(|attribute| {
                attribute.path().is_ident("cfg")
                    && matches!(&attribute.meta, syn::Meta::List(list)
                        if list.tokens.to_string() == "test")
            })
        };
        let mut calls = SpoolCalls::default();
        for item in &file.items {
            let test_only = match item {
                syn::Item::Mod(module) => cfg_test(&module.attrs),
                syn::Item::Fn(function) => cfg_test(&function.attrs),
                _ => false,
            };
            if !test_only {
                syn::visit::Visit::visit_item(&mut calls, item);
            }
        }
        if calls.retained_visits == 0 {
            return Err(
                "ordered search names must list only the retained `directory` it receives"
                    .to_string(),
            );
        }
        for ambient in [
            "canonicalize",
            "read_dir",
            "read_immediate_names_bounded",
            "read_to_string",
            "retain_immediate_child_nofollow",
            "retain_regular_child",
            "symlink_metadata",
        ] {
            if calls.names.contains(ambient) {
                return Err(format!(
                    "ordered search names must not reach sources through `{ambient}`"
                ));
            }
        }
        Ok(())
    }

    fn audited_actor_read_source() -> &'static str {
        let source = include_str!("invocation_service.rs");
        audit_actor_read_source_capability_api(source).unwrap_or_else(|error| {
            panic!("the unmodified actor capability source must pass its audit: {error}")
        });
        source
    }

    #[test]
    pub(crate) fn actor_read_source_capability_is_sealed_after_binding() {
        let source = include_str!("invocation_service.rs");
        audit_actor_read_source_capability_api(source).unwrap_or_else(|error| panic!("{error}"));
        audit_ordered_search_names(include_str!("ordered_search_names.rs"))
            .unwrap_or_else(|error| panic!("{error}"));
        audit_text_search_walk(include_str!("text_search.rs"))
            .unwrap_or_else(|error| panic!("{error}"));
        // Проверки независимы и в сумме идут секунды: каждая разбирает
        // исходник заново, часть зовёт rustc. Параллельно, с той же паникой (#1920).
        let checks: [fn(); 34] = [
            actor_read_source_capability_ast_audit_rejects_unordered_literal_enumeration,
            ordered_search_names_audit_rejects_ambient_or_foreign_listing,
            actor_read_source_capability_sibling_field_privacy_is_enforced_by_rustc,
            actor_read_source_capability_audit_rejects_sibling_visible_forge_and_mutator,
            actor_read_source_capability_ast_audit_rejects_split_sibling_visibility,
            actor_read_source_capability_ast_audit_rejects_multiline_derive_clone,
            actor_read_source_capability_ast_audit_rejects_multiline_manual_clone,
            actor_read_source_capability_ast_audit_rejects_multiline_extra_inherent_impl,
            actor_read_source_capability_ast_audit_rejects_macro_generated_api,
            actor_read_source_capability_ast_audit_rejects_opaque_item_macro_invocation,
            actor_read_source_capability_ast_audit_rejects_opaque_const_expression_macro,
            actor_read_source_capability_ast_audit_rejects_no_arg_opaque_const_expression_macro,
            actor_read_source_capability_ast_audit_rejects_opaque_static_and_statement_macros,
            actor_read_source_capability_ast_audit_rejects_opaque_macro_nested_in_inert_allowlist,
            actor_read_source_capability_ast_audit_rejects_inert_macro_import_rename_or_glob_shadow,
            review_audit_rejects_opaque_attribute_macro_invocation,
            actor_read_source_capability_ast_audit_rejects_custom_derive_macro,
            actor_read_source_capability_ast_audit_rejects_attribute_macro_evasion_shapes,
            actor_read_source_capability_procedural_macros_can_reopen_sibling_api,
            actor_read_source_capability_ast_audit_rejects_type_alias_impl,
            actor_read_source_capability_ast_audit_rejects_defaulted_generic_alias_impl,
            actor_read_source_capability_ast_audit_rejects_sibling_visible_free_factory,
            actor_read_source_capability_ast_audit_rejects_nested_production_factory,
            actor_read_source_capability_ast_audit_rejects_foreign_impl_signature_escape,
            actor_read_source_capability_ast_audit_rejects_associated_type_escape,
            actor_read_source_capability_ast_audit_rejects_item_field_escape,
            actor_read_source_capability_ast_audit_rejects_hardcoded_platform_profile,
            actor_read_source_capability_ast_audit_rejects_hardcoded_source_kind,
            actor_read_source_capability_ast_audit_rejects_replenished_deadline,
            actor_read_source_capability_ast_audit_rejects_side_effecting_profile_error_closure,
            actor_read_source_capability_ast_audit_rejects_substituted_construction_fields,
            actor_read_source_capability_ast_audit_rejects_issuance_visibility_and_lazy_substitution,
            actor_read_source_capability_issuer_privacy_is_enforced_by_rustc,
            actor_read_source_capability_ast_audit_skips_only_exact_cfg_test_scope,
        ];
        std::thread::scope(|scope| {
            let running: Vec<_> = checks.iter().map(|check| scope.spawn(check)).collect();
            for check in running {
                if let Err(panic) = check.join() {
                    std::panic::resume_unwind(panic);
                }
            }
        });
    }

    #[test]
    fn actor_read_source_capability_sibling_field_privacy_is_enforced_by_rustc() {
        use quote::ToTokens;

        let source = include_str!("invocation_service.rs");
        audit_actor_read_source_capability_api(source).unwrap_or_else(|error| panic!("{error}"));
        let file = syn::parse_file(source).expect("the audited production source parses");
        let declaration = file
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Struct(item) if item.ident == "ActorReadSourceCapability" => Some(item),
                _ => None,
            })
            .expect("the audited production declaration is present")
            .clone();
        let mut declaration = declaration;
        let fixture = |declaration: &syn::ItemStruct| {
            let declaration = declaration.to_token_stream();
            format!(
                r#"
mod infrastructure {{
    pub mod daemon {{
        mod owner {{
            use std::sync::Arc;
            pub(super) struct ProviderRootBinding;
            struct RegistrationCache;
            struct ProviderDeadline;
            {declaration}
        }}
        mod sibling {{
            fn inspect(capability: &super::owner::ActorReadSourceCapability) {{
                let _ = &capability.binding;
            }}
        }}
    }}
}}
fn main() {{}}
"#
            )
        };
        let directory = tempfile::tempdir().unwrap();
        let fixture_path = directory.path().join("actor-capability-sibling-privacy.rs");
        let compile = |source: String| {
            std::fs::write(&fixture_path, source).unwrap();
            std::process::Command::new("rustc")
                .arg("--edition=2021")
                .arg("--emit=metadata")
                .arg("--out-dir")
                .arg(directory.path())
                .arg(&fixture_path)
                .output()
                .expect("rustc is available to the Rust test suite")
        };
        let output = compile(fixture(&declaration));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success() && stderr.contains("E0616") && stderr.contains("binding"),
            "a sibling module accessed the audited private binding; status={:?}, stderr={stderr}",
            output.status.code()
        );
        let binding = declaration
            .fields
            .iter_mut()
            .find(|field| field.ident.as_ref().is_some_and(|name| name == "binding"))
            .expect("the audited binding field exists");
        binding.vis = syn::parse_quote!(pub(in crate::infrastructure::daemon));
        let exposed = compile(fixture(&declaration));
        assert!(
            exposed.status.success(),
            "the field-visibility counterexample must compile: {}",
            String::from_utf8_lossy(&exposed.stderr)
        );
    }

    fn assert_rustc_fixture_compiles_and_runs(name: &str, fixture: &str) {
        let directory = tempfile::tempdir().unwrap();
        let fixture_path = directory.path().join(format!("{name}.rs"));
        let binary_path = directory
            .path()
            .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&fixture_path, fixture).unwrap();
        let compile = std::process::Command::new("rustc")
            .arg("--edition=2021")
            .arg("-o")
            .arg(&binary_path)
            .arg(&fixture_path)
            .output()
            .expect("rustc is available to the Rust test suite");
        assert!(
            compile.status.success(),
            "hostile Rust fixture `{name}` did not compile: {}",
            String::from_utf8_lossy(&compile.stderr)
        );
        let run = std::process::Command::new(&binary_path)
            .output()
            .expect("the hostile Rust fixture binary starts");
        assert!(
            run.status.success(),
            "hostile Rust fixture `{name}` was not sibling-callable: {}",
            String::from_utf8_lossy(&run.stderr)
        );
    }

    #[test]
    fn actor_read_source_capability_audit_rejects_sibling_visible_forge_and_mutator() {
        let source = audited_actor_read_source();
        let inject_method = |hostile_method: &str| {
            source.replacen(
                "impl ActorReadSourceCapability {",
                &format!("impl ActorReadSourceCapability {{{hostile_method}"),
                1,
            )
        };
        let hostile_sources = [
            inject_method(
            r#"
    pub(super) fn forge(
        binding: ProviderRootBinding,
        identity: String,
        fence: WorkspaceLogicalReadFence,
        deadline: ProviderDeadline,
    ) -> Self {
        Self { binding, identity, fence, deadline }
    }
"#,
            ),
            inject_method(
            r#"
    pub(super) fn replace_fence(&mut self, fence: WorkspaceLogicalReadFence) {
        self.fence = fence;
    }
"#,
            ),
            inject_method(
                r#"
    pub(super) fn into_parts(
        self,
    ) -> (ProviderRootBinding, String, WorkspaceLogicalReadFence, ProviderDeadline) {
        (self.binding, self.identity, self.fence, self.deadline)
    }
"#,
            ),
            source.replacen(
                "#[allow(dead_code)]\npub(in crate::infrastructure::daemon) struct ActorReadSourceCapability {",
                "#[allow(dead_code)]\n#[derive(Clone)]\npub(in crate::infrastructure::daemon) struct ActorReadSourceCapability {",
                1,
            ),
            source.replacen(
                "pub(in crate::infrastructure::daemon) const fn deadline(&self) -> ProviderDeadline {",
                "pub(in crate::infrastructure::daemon) const fn deadline(&mut self) -> ProviderDeadline {",
                1,
            ),
        ];
        for hostile in hostile_sources {
            assert_ne!(hostile, source, "hostile mutation must alter its source");
            syn::parse_file(&hostile)
                .expect("hostile capability source must remain valid Rust syntax");
            assert!(
                audit_actor_read_source_capability_api(&hostile).is_err(),
                "hostile sibling-visible capability shape escaped the closed API audit"
            );
        }
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_split_sibling_visibility() {
        let source = audited_actor_read_source();
        let hostile = source.replacen(
            "impl ActorReadSourceCapability {",
            r#"impl ActorReadSourceCapability {
    pub
    (super) fn forge(
        binding: ProviderRootBinding,
        identity: String,
        fence: WorkspaceLogicalReadFence,
        deadline: ProviderDeadline,
    ) -> Self {
        Self { binding, identity, fence, deadline }
    }"#,
            1,
        );
        assert_ne!(hostile, source, "hostile mutation must alter its source");
        syn::parse_file(&hostile).expect("hostile capability source must remain valid Rust syntax");
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "split `pub (super)` visibility escaped the capability API audit"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_multiline_derive_clone() {
        let source = audited_actor_read_source();
        let multiline_derive = source.replacen(
            "#[allow(dead_code)]\npub(in crate::infrastructure::daemon) struct ActorReadSourceCapability {",
            "#[allow(dead_code)]\n#[derive(\n    Clone\n)]\npub(in crate::infrastructure::daemon) struct ActorReadSourceCapability {",
            1,
        );
        assert_ne!(
            multiline_derive, source,
            "hostile mutation must alter its source"
        );
        syn::parse_file(&multiline_derive)
            .expect("hostile capability source must remain valid Rust syntax");
        assert!(
            audit_actor_read_source_capability_api(&multiline_derive).is_err(),
            "multiline derive Clone escaped the capability API audit"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_multiline_manual_clone() {
        let source = audited_actor_read_source();
        let manual_clone = source.replacen(
            "struct ActorLogicalReadLease {",
            r#"impl
    Clone
    for ActorReadSourceCapability {
        fn clone(&self) -> Self {
            panic!("hostile clone")
        }
    }

struct ActorLogicalReadLease {"#,
            1,
        );
        assert_ne!(
            manual_clone, source,
            "hostile mutation must alter its source"
        );
        syn::parse_file(&manual_clone)
            .expect("hostile capability source must remain valid Rust syntax");
        assert!(
            audit_actor_read_source_capability_api(&manual_clone).is_err(),
            "multiline manual Clone escaped the capability API audit"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_multiline_extra_inherent_impl() {
        let source = audited_actor_read_source();
        let hostile = source.replacen(
            "struct ActorLogicalReadLease {",
            r#"impl
    ActorReadSourceCapability {
        pub(super) fn forge_from_parts(
            binding: ProviderRootBinding,
            identity: String,
            fence: WorkspaceLogicalReadFence,
            deadline: ProviderDeadline,
        ) -> Self {
            Self { binding, identity, fence, deadline }
        }
    }

struct ActorLogicalReadLease {"#,
            1,
        );
        assert_ne!(hostile, source, "hostile mutation must alter its source");
        syn::parse_file(&hostile).expect("hostile capability source must remain valid Rust syntax");
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "multiline extra inherent impl escaped the capability API audit"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_macro_generated_api() {
        let source = audited_actor_read_source();
        let hostile = source.replacen(
            "struct ActorLogicalReadLease {",
            r#"macro_rules! add_actor_capability_api {
        ($capability:ty) => {
            impl $capability {
                pub(super) fn forge_from_macro(
                    binding: ProviderRootBinding,
                    identity: String,
                    fence: WorkspaceLogicalReadFence,
                    deadline: ProviderDeadline,
                ) -> Self {
                    Self { binding, identity, fence, deadline }
                }
            }
        };
    }
    add_actor_capability_api!(ActorReadSourceCapability);

struct ActorLogicalReadLease {"#,
            1,
        );
        assert_ne!(hostile, source, "hostile mutation must alter its source");
        syn::parse_file(&hostile).expect("hostile capability source must remain valid Rust syntax");
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "macro-generated sibling API escaped the capability API audit"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_opaque_item_macro_invocation() {
        let source = audited_actor_read_source();
        let hostile = source.replacen(
            "struct ActorLogicalReadLease {",
            "reopen_sealed_capability!();\n\nstruct ActorLogicalReadLease {",
            1,
        );
        assert_ne!(hostile, source, "hostile mutation must alter its source");
        syn::parse_file(&hostile).expect("hostile capability source must remain valid Rust syntax");
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "an opaque item-position macro could generate a sibling capability API"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_opaque_const_expression_macro() {
        assert_rustc_fixture_compiles_and_runs(
            "actor-capability-opaque-const-macro",
            r#"
macro_rules! external_reopen_capability {
    ($capability:ty) => {{
        impl $capability {
            pub(super) fn forge() -> Self {
                Self {
                    binding: ProviderRootBinding,
                    identity: String::new(),
                    fence: WorkspaceLogicalReadFence,
                    deadline: ProviderDeadline,
                }
            }
        }
        ()
    }};
}
mod daemon {
    mod owner {
        pub(super) struct ProviderRootBinding;
        pub(super) struct WorkspaceLogicalReadFence;
        pub(super) struct ProviderDeadline;
        pub(super) struct ActorReadSourceCapability {
            binding: ProviderRootBinding,
            identity: String,
            fence: WorkspaceLogicalReadFence,
            deadline: ProviderDeadline,
        }
        const _: () = external_reopen_capability!(ActorReadSourceCapability);
    }
    mod sibling {
        pub(super) fn call_forge() {
            let _ = super::owner::ActorReadSourceCapability::forge();
        }
    }
    pub(super) fn run() {
        sibling::call_forge();
    }
}
fn main() {
    daemon::run();
}
"#,
        );

        let source = audited_actor_read_source();
        let hostile = source.replacen(
            "struct ActorLogicalReadLease {",
            "const _: () = external_reopen_capability!(ActorReadSourceCapability);\n\nstruct ActorLogicalReadLease {",
            1,
        );
        assert_ne!(hostile, source, "hostile mutation must alter its source");
        syn::parse_file(&hostile).expect("hostile capability source must remain valid Rust syntax");
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "an opaque const-expression macro generated a sibling capability forge"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_no_arg_opaque_const_expression_macro() {
        assert_rustc_fixture_compiles_and_runs(
            "actor-capability-no-arg-opaque-const-macro",
            r#"
macro_rules! external_reopen_capability {
    () => {{
        impl ActorReadSourceCapability {
            pub(super) fn forge() -> Self {
                Self {
                    binding: ProviderRootBinding,
                    identity: String::new(),
                    fence: WorkspaceLogicalReadFence,
                    deadline: ProviderDeadline,
                }
            }
        }
        ()
    }};
}
mod daemon {
    mod owner {
        pub(super) struct ProviderRootBinding;
        pub(super) struct WorkspaceLogicalReadFence;
        pub(super) struct ProviderDeadline;
        pub(super) struct ActorReadSourceCapability {
            binding: ProviderRootBinding,
            identity: String,
            fence: WorkspaceLogicalReadFence,
            deadline: ProviderDeadline,
        }
        const _: () = external_reopen_capability!();
    }
    mod sibling {
        pub(super) fn call_forge() {
            let _ = super::owner::ActorReadSourceCapability::forge();
        }
    }
    pub(super) fn run() {
        sibling::call_forge();
    }
}
fn main() {
    daemon::run();
}
"#,
        );

        let source = audited_actor_read_source();
        let hostile = source.replacen(
            "struct ActorLogicalReadLease {",
            "const _: () = external_reopen_capability!();\n\nstruct ActorLogicalReadLease {",
            1,
        );
        assert_ne!(hostile, source, "hostile mutation must alter its source");
        syn::parse_file(&hostile).expect("hostile capability source must remain valid Rust syntax");
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "a no-argument opaque const-expression macro generated a sibling capability forge"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_opaque_static_and_statement_macros() {
        let source = audited_actor_read_source();
        let hostile_shapes = [
            "static OPAQUE: () = external_reopen_capability!();\n\n",
            "fn opaque_statement() { external_reopen_capability!(); }\n\n",
        ];
        let mut accepted = Vec::new();
        for shape in hostile_shapes {
            let hostile = source.replacen(
                "struct ActorLogicalReadLease {",
                &format!("{shape}struct ActorLogicalReadLease {{"),
                1,
            );
            assert_ne!(hostile, source, "hostile mutation must alter its source");
            syn::parse_file(&hostile)
                .expect("hostile capability source must remain valid Rust syntax");
            if audit_actor_read_source_capability_api(&hostile).is_ok() {
                accepted.push(shape);
            }
        }
        assert!(
            accepted.is_empty(),
            "opaque static/statement macros escaped the capability audit: {accepted:?}"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_opaque_macro_nested_in_inert_allowlist() {
        let source = audited_actor_read_source();
        let hostile = source.replacen(
            "struct ActorLogicalReadLease {",
            r#"fn nested_opaque_macro() {
    let _ = format!("{}", external_reopen_capability!());
}

struct ActorLogicalReadLease {"#,
            1,
        );
        assert_ne!(hostile, source, "hostile mutation must alter its source");
        syn::parse_file(&hostile).expect("hostile capability source must remain valid Rust syntax");
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "an opaque macro hid inside the inert standard-macro allowlist"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_inert_macro_import_rename_or_glob_shadow() {
        let source = audited_actor_read_source();
        let hostile_imports = [
            "use hostile_macros::reopen as format;\n",
            "use hostile_macros::matches;\n",
            "use hostile_macros::*;\n",
        ];
        let mut accepted = Vec::new();
        for import in hostile_imports {
            let hostile = source.replacen(
                "use super::super::protocol::InvocationRequest;",
                &format!("{import}use super::super::protocol::InvocationRequest;"),
                1,
            );
            assert_ne!(hostile, source, "hostile mutation must alter its source");
            syn::parse_file(&hostile)
                .expect("hostile capability source must remain valid Rust syntax");
            if audit_actor_read_source_capability_api(&hostile).is_ok() {
                accepted.push(import);
            }
        }
        assert!(
            accepted.is_empty(),
            "imports could impersonate allowlisted standard macros: {accepted:?}"
        );
    }

    #[test]
    fn review_audit_rejects_opaque_attribute_macro_invocation() {
        let source = audited_actor_read_source();
        let hostile = source.replacen(
            "struct ActorLogicalReadLease {",
            "#[external_reopen_capability]\nconst ATTRIBUTE_MACRO_TRIGGER: () = ();\n\nstruct ActorLogicalReadLease {",
            1,
        );
        assert_ne!(
            hostile, source,
            "hostile attribute fixture was not inserted"
        );
        syn::parse_file(&hostile).expect("hostile capability source must remain valid Rust syntax");
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "an opaque procedural attribute could generate a sibling capability API"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_custom_derive_macro() {
        let source = audited_actor_read_source();
        let hostile = source.replacen(
            "struct ActorLogicalReadLease {",
            "#[derive(ExternalReopenCapability)]\nstruct DERIVE_MACRO_TRIGGER;\n\nstruct ActorLogicalReadLease {",
            1,
        );
        assert_ne!(hostile, source, "hostile derive fixture was not inserted");
        syn::parse_file(&hostile).expect("hostile capability source must remain valid Rust syntax");
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "a custom derive could generate a sibling capability API"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_attribute_macro_evasion_shapes() {
        let source = audited_actor_read_source();
        let hostile_items = [
            "#[hostile_attr::external_reopen_capability]\nconst ATTRIBUTE_MACRO_TRIGGER: () = ();\n\n",
            "#[cfg_attr(not(test), external_reopen_capability)]\nconst ATTRIBUTE_MACRO_TRIGGER: () = ();\n\n",
            "#[derive(Clone, ExternalReopenCapability)]\nstruct DERIVE_MACRO_TRIGGER;\n\n",
        ];
        let hostile_imports = [
            "use hostile_attr::ExternalReopenCapability as Clone;\n",
            "use hostile_attr::*;\n",
        ];
        let mut accepted = Vec::new();
        for item in hostile_items {
            let hostile = source.replacen(
                "struct ActorLogicalReadLease {",
                &format!("{item}struct ActorLogicalReadLease {{"),
                1,
            );
            assert_ne!(hostile, source, "hostile mutation must alter its source");
            syn::parse_file(&hostile)
                .expect("hostile capability source must remain valid Rust syntax");
            if audit_actor_read_source_capability_api(&hostile).is_ok() {
                accepted.push(item);
            }
        }
        for import in hostile_imports {
            let hostile = source.replacen(
                "use super::super::protocol::InvocationRequest;",
                &format!("{import}use super::super::protocol::InvocationRequest;"),
                1,
            );
            assert_ne!(hostile, source, "hostile mutation must alter its source");
            syn::parse_file(&hostile)
                .expect("hostile capability source must remain valid Rust syntax");
            if audit_actor_read_source_capability_api(&hostile).is_ok() {
                accepted.push(import);
            }
        }
        assert!(
            accepted.is_empty(),
            "attribute macro evasion shapes escaped the capability audit: {accepted:?}"
        );
    }

    #[test]
    fn actor_read_source_capability_procedural_macros_can_reopen_sibling_api() {
        let directory = tempfile::tempdir().unwrap();
        let proc_macro_source = directory.path().join("hostile_attr.rs");
        let proc_macro_library = directory.path().join(format!(
            "{}hostile_attr{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_SUFFIX
        ));
        std::fs::write(
            &proc_macro_source,
            r#"
extern crate proc_macro;
use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn external_reopen_capability(_attribute: TokenStream, item: TokenStream) -> TokenStream {
    let mut output = item.to_string();
    output.push_str(
        "impl ActorReadSourceCapability { pub(super) fn forge_from_attribute() -> Self { Self } }",
    );
    output.parse().unwrap()
}

#[proc_macro_derive(ExternalReopenCapability)]
pub fn external_reopen_capability_derive(_item: TokenStream) -> TokenStream {
    "impl ActorReadSourceCapability { pub(super) fn forge_from_derive() -> Self { Self } }"
        .parse()
        .unwrap()
}
"#,
        )
        .unwrap();
        let proc_macro_compile = std::process::Command::new("rustc")
            .arg("--edition=2021")
            .arg("--crate-type=proc-macro")
            .arg("--crate-name")
            .arg("hostile_attr")
            .arg(&proc_macro_source)
            .arg("-o")
            .arg(&proc_macro_library)
            .output()
            .expect("rustc is available to compile the hostile procedural-macro crate");
        assert!(
            proc_macro_compile.status.success(),
            "hostile procedural-macro crate did not compile: {}",
            String::from_utf8_lossy(&proc_macro_compile.stderr)
        );

        let main_source = directory.path().join("main.rs");
        let binary = directory
            .path()
            .join(format!("proc-macro-probe{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(
            &main_source,
            r#"
use hostile_attr::ExternalReopenCapability as Clone;

mod daemon {
    mod owner {
        use super::super::Clone;

        pub(super) struct ActorReadSourceCapability;

        #[hostile_attr::external_reopen_capability]
        const ATTRIBUTE_MACRO_TRIGGER: () = ();

        #[derive(Clone)]
        struct DeriveMacroTrigger;
    }

    mod sibling {
        pub(super) fn call_generated_apis() {
            let _ = super::owner::ActorReadSourceCapability::forge_from_attribute();
            let _ = super::owner::ActorReadSourceCapability::forge_from_derive();
        }
    }

    pub(super) fn run() {
        sibling::call_generated_apis();
    }
}

fn main() {
    daemon::run();
}
"#,
        )
        .unwrap();
        let main_compile = std::process::Command::new("rustc")
            .arg("--edition=2021")
            .arg(&main_source)
            .arg("--extern")
            .arg(format!("hostile_attr={}", proc_macro_library.display()))
            .arg("-o")
            .arg(&binary)
            .output()
            .expect("rustc is available to compile the sibling-call probe crate");
        assert!(
            main_compile.status.success(),
            "procedural-macro sibling-call probe did not compile: {}",
            String::from_utf8_lossy(&main_compile.stderr)
        );
        let run = std::process::Command::new(&binary)
            .output()
            .expect("procedural-macro sibling-call probe starts");
        assert!(
            run.status.success(),
            "procedural-macro sibling-call probe failed: {}",
            String::from_utf8_lossy(&run.stderr)
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_type_alias_impl() {
        let source = audited_actor_read_source();
        let hostile = source.replacen(
            "struct ActorLogicalReadLease {",
            r#"type CapabilityAlias = ActorReadSourceCapability;
    impl CapabilityAlias {
        pub(super) fn forge_alias(
            binding: ProviderRootBinding,
            identity: String,
            fence: WorkspaceLogicalReadFence,
            deadline: ProviderDeadline,
        ) -> Self {
            Self { binding, identity, fence, deadline }
        }
    }

struct ActorLogicalReadLease {"#,
            1,
        );
        assert_ne!(hostile, source, "hostile mutation must alter its source");
        syn::parse_file(&hostile).expect("hostile capability source must remain valid Rust syntax");
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "a type alias reopened the sealed capability outside the enumerated impl"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_defaulted_generic_alias_impl() {
        assert_rustc_fixture_compiles_and_runs(
            "actor-capability-defaulted-generic-alias",
            r#"
mod daemon {
    mod owner {
        pub(super) struct ProviderRootBinding;
        pub(super) struct WorkspaceLogicalReadFence;
        pub(super) struct ProviderDeadline;
        pub(super) struct ActorReadSourceCapability {
            binding: ProviderRootBinding,
            identity: String,
            fence: WorkspaceLogicalReadFence,
            deadline: ProviderDeadline,
        }
        type CapabilityAlias<T = ActorReadSourceCapability> = T;
        impl CapabilityAlias {
            pub(super) fn forge() -> Self {
                Self {
                    binding: ProviderRootBinding,
                    identity: String::new(),
                    fence: WorkspaceLogicalReadFence,
                    deadline: ProviderDeadline,
                }
            }
        }
    }
    mod sibling {
        pub(super) fn call_forge() {
            let _ = super::owner::ActorReadSourceCapability::forge();
        }
    }
    pub(super) fn run() {
        sibling::call_forge();
    }
}
fn main() {
    daemon::run();
}
"#,
        );

        let source = audited_actor_read_source();
        let hostile = source.replacen(
            "struct ActorLogicalReadLease {",
            r#"type CapabilityAlias<T = ActorReadSourceCapability> = T;
    impl CapabilityAlias {
        pub(super) fn forge_alias(
            binding: ProviderRootBinding,
            identity: String,
            fence: WorkspaceLogicalReadFence,
            deadline: ProviderDeadline,
        ) -> Self {
            Self { binding, identity, fence, deadline }
        }
    }

struct ActorLogicalReadLease {"#,
            1,
        );
        assert_ne!(hostile, source, "hostile mutation must alter its source");
        syn::parse_file(&hostile).expect("hostile capability source must remain valid Rust syntax");
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "a defaulted generic alias reopened the sealed capability"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_sibling_visible_free_factory() {
        assert_rustc_fixture_compiles_and_runs(
            "actor-capability-sibling-free-factory",
            r#"
mod daemon {
    mod owner {
        pub(super) struct ProviderRootBinding;
        pub(super) struct WorkspaceLogicalReadFence;
        pub(super) struct ProviderDeadline;
        pub(super) struct ActorReadSourceCapability {
            binding: ProviderRootBinding,
            identity: String,
            fence: WorkspaceLogicalReadFence,
            deadline: ProviderDeadline,
        }
        pub(super) fn forge(
            binding: ProviderRootBinding,
            identity: String,
            fence: WorkspaceLogicalReadFence,
            deadline: ProviderDeadline,
        ) -> ActorReadSourceCapability {
            ActorReadSourceCapability { binding, identity, fence, deadline }
        }
    }
    mod sibling {
        pub(super) fn call_forge() {
            let _ = super::owner::forge(
                super::owner::ProviderRootBinding,
                String::new(),
                super::owner::WorkspaceLogicalReadFence,
                super::owner::ProviderDeadline,
            );
        }
    }
    pub(super) fn run() {
        sibling::call_forge();
    }
}
fn main() {
    daemon::run();
}
"#,
        );

        let source = audited_actor_read_source();
        let hostile = source.replacen(
            "struct ActorLogicalReadLease {",
            r#"pub(super) fn forge_actor_read_source_capability(
    binding: ProviderRootBinding,
    identity: String,
    fence: WorkspaceLogicalReadFence,
    deadline: ProviderDeadline,
) -> ActorReadSourceCapability {
    ActorReadSourceCapability { binding, identity, fence, deadline }
}

struct ActorLogicalReadLease {"#,
            1,
        );
        assert_ne!(hostile, source, "hostile mutation must alter its source");
        syn::parse_file(&hostile).expect("hostile capability source must remain valid Rust syntax");
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "a sibling-visible owner-module free factory escaped the capability audit"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_nested_production_factory() {
        let source = audited_actor_read_source();
        let hostile = source.replacen(
            "struct ActorLogicalReadLease {",
            r#"mod nested_factory {
    pub(super) fn forge(
        binding: super::ProviderRootBinding,
        identity: String,
        fence: super::WorkspaceLogicalReadFence,
        deadline: ProviderDeadline,
    ) -> super::ActorReadSourceCapability {
        super::ActorReadSourceCapability { binding, identity, fence, deadline }
    }
}

struct ActorLogicalReadLease {"#,
            1,
        );
        assert_ne!(hostile, source, "hostile mutation must alter its source");
        syn::parse_file(&hostile).expect("hostile capability source must remain valid Rust syntax");
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "a nested production module exported a capability factory"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_foreign_impl_signature_escape() {
        let source = audited_actor_read_source();
        let hostile = source.replacen(
            "struct ActorLogicalReadLease {",
            r#"struct CapabilityExporter;
impl CapabilityExporter {
    pub(super) fn export(
        &self,
        capability: ActorReadSourceCapability,
    ) -> ActorReadSourceCapability {
        capability
    }
}

struct ActorLogicalReadLease {"#,
            1,
        );
        assert_ne!(hostile, source, "hostile mutation must alter its source");
        syn::parse_file(&hostile).expect("hostile capability source must remain valid Rust syntax");
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "a foreign impl accepted and returned the capability"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_associated_type_escape() {
        let source = audited_actor_read_source();
        let hostile = source.replacen(
            "struct ActorLogicalReadLease {",
            r#"trait CapabilityCarrier {
    type Capability;
}
struct CapabilityCarrierImpl;
impl CapabilityCarrier for CapabilityCarrierImpl {
    type Capability = ActorReadSourceCapability;
}

struct ActorLogicalReadLease {"#,
            1,
        );
        assert_ne!(hostile, source, "hostile mutation must alter its source");
        syn::parse_file(&hostile).expect("hostile capability source must remain valid Rust syntax");
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "an associated type exported the capability outside the closed surface"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_item_field_escape() {
        let source = audited_actor_read_source();
        let hostile = source.replacen(
            "struct ActorLogicalReadLease {",
            r#"pub(super) struct CapabilityEscape {
    pub(super) capability: ActorReadSourceCapability,
}

struct ActorLogicalReadLease {"#,
            1,
        );
        assert_ne!(hostile, source, "hostile mutation must alter its source");
        syn::parse_file(&hostile).expect("hostile capability source must remain valid Rust syntax");
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "a sibling-visible item field exported the capability"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_side_effecting_profile_error_closure() {
        let source = audited_actor_read_source();
        let hostile = source.replacen(
            r#"|| "actor-bound logical source has no supported platform profile".to_string()"#,
            r#"|| {
            PROFILE_ERROR_SIDE_EFFECT.store(true, Ordering::SeqCst);
            "actor-bound logical source has no supported platform profile".to_string()
        }"#,
            1,
        );
        assert_ne!(
            hostile, source,
            "the unsupported-profile closure mutation must alter the source"
        );
        syn::parse_file(&hostile).expect("hostile capability source must remain valid Rust syntax");
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "a side-effecting unsupported-profile closure escaped the exact builder audit"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_substituted_construction_fields() {
        let source = audited_actor_read_source();
        let hostile_fields = [
            (
                "Ok(ActorReadSourceCapability {\n            binding: source.binding.clone(),",
                "Ok(ActorReadSourceCapability {\n            binding: self.invocation.provider_root.clone(),",
            ),
            (
                r#"identity: format!(
                "{}:{}",
                self.invocation.workspace_identity_hash.as_str(),
                source.binding.source_set_name()
            ),"#,
                "identity: String::from(\"caller-supplied\"),",
            ),
            (
                "read_identity: source.read_identity.clone(),",
                "read_identity: lease.sources[0].read_identity.clone(),",
            ),
            (
                "deadline: read_deadline,",
                "deadline: ProviderDeadline::from_budget(Duration::from_secs(120)),",
            ),
            (
                "registration_cache: Arc::clone(&source.registration_cache),",
                "registration_cache: Arc::new(RegistrationCache::default()),",
            ),
        ];
        let mut accepted = Vec::new();
        for (original, substituted) in hostile_fields {
            let hostile = source.replacen(original, substituted, 1);
            assert_ne!(
                hostile, source,
                "hostile construction fixture did not replace `{original}`"
            );
            syn::parse_file(&hostile)
                .expect("hostile capability source must remain valid Rust syntax");
            if audit_actor_read_source_capability_api(&hostile).is_ok() {
                accepted.push(substituted);
            }
        }
        assert!(
            accepted.is_empty(),
            "substituted actor capability construction fields escaped: {accepted:?}"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_issuance_visibility_and_lazy_substitution() {
        let source = audited_actor_read_source();
        // First prove the valid source passes: a permanently red audit cannot
        // demonstrate that any of these individual boundary mutations is caught.
        audit_actor_read_source_capability_api(source).unwrap();
        let substitutions = [
            ("|| !self.original_body_line_targets().contains(at)", "|| false"),
            ("if confirmed.load(std::sync::atomic::Ordering::Acquire)", "if false"),
            (".find(|source| source.binding.source_set_name() == at.source_set())", ".find(|source| source.binding.source_set_name() == \"foreign\")"),
            ("self.issue_read_source(&source, ProviderDeadline::no_deadline())", "self.issue_read_source(&source, ProviderDeadline::from_budget(Duration::from_secs(120)))"),

            ("    fn issue_read_source(", "    pub(super) fn issue_read_source("),
            ("    fn issue_read_source(", "    pub fn issue_read_source("),
            (
                "self.invocation.actor.validate_binding(&source.binding)?;\n        Ok(ActorReadSourceCapability",
                "Ok(ActorReadSourceCapability",
            ),
            (
                ".map(|source| self.issue_read_source(source, lease.deadline))",
                ".map(|source| self.issue_read_source(source, ProviderDeadline::no_deadline()))",
            ),
            (
                ".find(|binding| binding.source_set_name() == name)",
                ".find(|binding| binding.source_set_name() == \"other\")",
            ),
            (
                "let binding = lease\n            .bindings",
                "let binding = self.invocation\n            .read_sources",
            ),
            (
                "let source = ActorLogicalReadSourceLease {\n            binding: binding.clone(),",
                "let source = ActorLogicalReadSourceLease {\n            binding: self.invocation.provider_root.clone(),",
            ),
            (
                "registration_cache: self.invocation.actor.source_artifacts(binding)?,",
                "registration_cache: self.invocation.actor.source_artifacts(&self.invocation.provider_root)?,",
            ),
            (
                "registration_cache: self.invocation.actor.source_artifacts(binding)?,",
                "registration_cache: Arc::new(RegistrationCache::default()),",
            ),
            (
                "self.issue_read_source(&source, lease.deadline)",
                "self.issue_read_source(&source, ProviderDeadline::no_deadline())",
            ),
            (
                "self.issue_read_source(&source, lease.deadline)",
                "self.issue_read_source(&lease.sources[0], lease.deadline)",
            ),
            (
                "read_identity: uuid::Uuid::new_v4().to_string(),\n            registration_cache: self.invocation.actor.source_artifacts(binding)?,",
                "read_identity: String::from(\"foreign\"),\n            registration_cache: self.invocation.actor.source_artifacts(binding)?,",
            ),
        ];
        let escapes = [
            r#"impl ActorBoundExecution {
                fn foreign_issuance(&self, source: &ActorLogicalReadSourceLease) {
                    let _ = self.issue_read_source(source, ProviderDeadline::no_deadline());
                }
            }"#,
            r#"impl ActorBoundExecution {
                fn foreign_issuance(&self, source: &ActorLogicalReadSourceLease) {
                    let issue = ActorBoundExecution::issue_read_source;
                    let _ = issue(self, source, ProviderDeadline::no_deadline());
                }
            }"#,
            r#"impl ActorBoundExecution {
                fn foreign_issuance(&self, source: &ActorLogicalReadSourceLease) {
                    let _ = vec![self.issue_read_source(source, ProviderDeadline::no_deadline())];
                }
            }"#,
            r#"impl ActorBoundExecution {
                fn foreign_issuance(&self, source: &ActorLogicalReadSourceLease) {
                    let _ = vec![self.r#issue_read_source(source, ProviderDeadline::no_deadline())];
                }
            }"#,
            r#"impl ActorBoundExecution {
                fn foreign_issuance(&self, source: &ActorLogicalReadSourceLease) {
                    let _ = vec![r#ActorReadSourceCapability {
                        binding: source.binding.clone(),
                        identity: String::from("foreign"),
                        read_identity: source.read_identity.clone(),
                        deadline: ProviderDeadline::no_deadline(),
                        registration_cache: Arc::clone(&source.registration_cache),
                    }];
                }
            }"#,
            r#"impl ActorBoundExecution {
                fn foreign_issuance(&self, source: &ActorLogicalReadSourceLease) {
                    let _ = self.r#issue_read_source(source, ProviderDeadline::no_deadline());
                }
            }"#,
            r#"impl ActorBoundExecution {
                fn foreign_issuance(&self, source: &ActorLogicalReadSourceLease) {
                    let issue = ActorBoundExecution::r#issue_read_source;
                    let _ = issue(self, source, ProviderDeadline::no_deadline());
                }
            }"#,
            r#"impl ActorBoundExecution {
                fn foreign_issuance(&self, source: &ActorLogicalReadSourceLease) {
                    let _ = r#ActorReadSourceCapability {
                        binding: source.binding.clone(),
                        identity: String::from("foreign"),
                        read_identity: source.read_identity.clone(),
                        deadline: ProviderDeadline::no_deadline(),
                        registration_cache: Arc::clone(&source.registration_cache),
                    };
                }
            }"#,
            r#"use external::x as r#vec;"#,
            r#"use external::x as r#Clone;"#,
        ];
        for escape in escapes {
            let hostile = format!("{source}\n{escape}");
            assert_ne!(hostile, source, "an issuance escape must alter the source");
            syn::parse_file(&hostile).expect("an issuance escape is valid Rust syntax");
            assert!(
                audit_actor_read_source_capability_api(&hostile).is_err(),
                "an inferred capability escaped the closed issuer calls: {escape}"
            );
        }
        for (original, replacement) in substitutions {
            let hostile = source.replacen(original, replacement, 1);
            assert_ne!(hostile, source, "mutation must alter `{original}`");
            syn::parse_file(&hostile).expect("each hostile dataflow must remain valid Rust syntax");
            assert!(
                audit_actor_read_source_capability_api(&hostile).is_err(),
                "issuance mutation escaped the AST boundary: `{replacement}`"
            );
        }
    }

    #[test]
    fn actor_read_source_capability_issuer_privacy_is_enforced_by_rustc() {
        use quote::ToTokens;

        let source = include_str!("invocation_service.rs");
        audit_actor_read_source_capability_api(source).unwrap();
        let file = syn::parse_file(source).unwrap();
        let mut issuer = file
            .items
            .iter()
            .filter_map(|item| match item {
                syn::Item::Impl(item) => Some(item),
                _ => None,
            })
            .flat_map(|item| item.items.iter())
            .find_map(|item| match item {
                syn::ImplItem::Fn(method) if method.sig.ident == "issue_read_source" => {
                    Some(method.clone())
                }
                _ => None,
            })
            .expect("the audited private issuer exists");
        // Keep the production signature and real visibility; unrelated actor
        // dependencies in the body are not needed to ask rustc about privacy.
        issuer.block = syn::parse_quote!({ unimplemented!() });
        let fixture = |issuer: &syn::ImplItemFn| {
            format!(
                r#"
mod owner {{
    pub(super) struct ActorBoundExecution;
    pub(super) struct ActorLogicalReadSourceLease;
    pub(super) struct ProviderDeadline;
    pub(super) struct ActorReadSourceCapability;
    impl ActorBoundExecution {{ {} }}
}}
mod sibling {{
    fn forge(execution: &super::owner::ActorBoundExecution) {{
        let _ = execution.issue_read_source(
            &super::owner::ActorLogicalReadSourceLease,
            super::owner::ProviderDeadline,
        );
    }}
}}
fn main() {{}}
"#,
                issuer.to_token_stream()
            )
        };
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("actor_issuer_privacy.rs");
        let compile = |source: String| {
            std::fs::write(&path, source).unwrap();
            std::process::Command::new("rustc")
                .arg("--edition=2021")
                .arg("--emit=metadata")
                .arg("--out-dir")
                .arg(directory.path())
                .arg(&path)
                .output()
                .expect("rustc is available to the Rust test suite")
        };
        let private = compile(fixture(&issuer));
        let stderr = String::from_utf8_lossy(&private.stderr);
        assert!(
            !private.status.success()
                && stderr.contains("E0624")
                && stderr.contains("issue_read_source"),
            "a sibling could call the actual private issuer: {stderr}"
        );
        issuer.vis = syn::parse_quote!(pub(super));
        let exposed = compile(fixture(&issuer));
        assert!(
            exposed.status.success(),
            "the visibility counterexample must compile: {}",
            String::from_utf8_lossy(&exposed.stderr)
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_skips_only_exact_cfg_test_scope() {
        let source = audited_actor_read_source();
        let exact_test_only = source.replacen(
            "struct ActorLogicalReadLease {",
            r#"#[cfg(test)]
mod test_only_escape {
    pub(super) fn forge(
        binding: super::ProviderRootBinding,
        identity: String,
        fence: super::WorkspaceLogicalReadFence,
        deadline: ProviderDeadline,
    ) -> super::ActorReadSourceCapability {
        super::ActorReadSourceCapability { binding, identity, fence, deadline }
    }
}

struct ActorLogicalReadLease {"#,
            1,
        );
        assert_ne!(
            exact_test_only, source,
            "the exact cfg(test) insertion must alter the source"
        );
        audit_actor_read_source_capability_api(&exact_test_only)
            .expect("the production audit deliberately excludes exact cfg(test) modules");

        let partly_production = exact_test_only.replacen(
            "#[cfg(test)]\nmod test_only_escape",
            "#[cfg(any(test, feature = \"hostile-production\"))]\nmod test_only_escape",
            1,
        );
        assert_ne!(
            partly_production, exact_test_only,
            "hostile mutation must alter its source"
        );
        syn::parse_file(&partly_production)
            .expect("hostile capability source must remain valid Rust syntax");
        assert!(
            audit_actor_read_source_capability_api(&partly_production).is_err(),
            "a module that can compile outside cfg(test) escaped the production audit"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_hardcoded_platform_profile() {
        let source = audited_actor_read_source();
        let hardcoded_profile = source.replacen(
            "crate::domain::source_capabilities::bsl_profile_for_source(source_profile)",
            "crate::domain::source_capabilities::bsl_profile_for_source(SourceProfile::platform_xml_8_3_27_format_2_20())",
            1,
        );
        assert_ne!(
            hardcoded_profile, source,
            "hostile mutation must alter its source"
        );
        syn::parse_file(&hardcoded_profile)
            .expect("hostile capability source must remain valid Rust syntax");
        assert!(
            audit_actor_read_source_capability_api(&hardcoded_profile).is_err(),
            "hardcoded platform profile escaped the authority-construction dataflow audit"
        );
    }

    fn actor_read_source_capability_ast_audit_rejects_unordered_literal_enumeration() {
        let source = include_str!("text_search.rs");
        let unordered = source.replacen(
            "OrderedNames::collect(&frame.directory, checkpoint)",
            "OrderedNames::from_listing(&frame.directory, checkpoint)",
            1,
        );
        assert_ne!(unordered, source, "hostile mutation must alter its source");
        syn::parse_file(&unordered)
            .expect("hostile capability source must remain valid Rust syntax");
        assert!(
            audit_text_search_walk(&unordered).is_err(),
            "literal search escaped the retained ordered enumeration audit"
        );
    }

    fn ordered_search_names_audit_rejects_ambient_or_foreign_listing() {
        let source = include_str!("ordered_search_names.rs");
        let foreign = source.replacen(
            "directory.visit_immediate_names(",
            "foreign.visit_immediate_names(",
            1,
        );
        let ambient = source.replacen(
            "directory.visit_immediate_names(",
            "std::fs::read_dir(\".\").map(drop)?;\n        directory.visit_immediate_names(",
            1,
        );
        for hostile in [foreign, ambient] {
            assert_ne!(hostile, source, "hostile mutation must alter its source");
            syn::parse_file(&hostile).expect("hostile spool source must remain valid Rust syntax");
            assert!(
                audit_ordered_search_names(&hostile).is_err(),
                "ordered search names escaped the retained listing audit"
            );
        }
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_hardcoded_source_kind() {
        let source = audited_actor_read_source();
        let hardcoded_kind = source.replacen(
            "self.binding.source_kind(),\n            self.binding.retained_root(),",
            "SourceSetKind::Configuration,\n            self.binding.retained_root(),",
            1,
        );
        assert_ne!(
            hardcoded_kind, source,
            "the hostile fixture must alter the local snapshot constructor"
        );
        syn::parse_file(&hardcoded_kind)
            .expect("hostile capability source must remain valid Rust syntax");
        assert!(
            audit_actor_read_source_capability_api(&hardcoded_kind).is_err(),
            "hardcoded source kind escaped the authority-construction dataflow audit"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_replenished_deadline() {
        let source = audited_actor_read_source();
        let replenished_deadline = source.replacen(
            "platform_profile,\n                self.deadline,",
            "platform_profile,\n                ProviderDeadline::from_budget(Duration::from_secs(120)),",
            1,
        );
        assert_ne!(
            replenished_deadline, source,
            "hostile mutation must alter its source"
        );
        syn::parse_file(&replenished_deadline)
            .expect("hostile capability source must remain valid Rust syntax");
        assert!(
            audit_actor_read_source_capability_api(&replenished_deadline).is_err(),
            "replenished deadline escaped the authority-construction dataflow audit"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_foreign_search_deadline() {
        let source = include_str!("text_search.rs");
        audit_text_search_walk(source).unwrap();
        let replenished_search = source.replacen(
            "source.source.deadline.is_elapsed()",
            "ProviderDeadline::from_budget(Duration::from_secs(120)).is_elapsed()",
            1,
        );
        assert_ne!(replenished_search, source, "the hostile search must change");
        syn::parse_file(&replenished_search)
            .expect("hostile capability source must remain valid Rust syntax");
        assert!(
            audit_text_search_walk(&replenished_search).is_err(),
            "a fresh search deadline escaped the closed receiver audit while other checks remained"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_missing_search_deadline() {
        let source = include_str!("text_search.rs");
        let unchecked_search = source.replace("source.source.deadline.is_elapsed()", "false");
        assert_ne!(unchecked_search, source, "the unchecked search must change");
        syn::parse_file(&unchecked_search)
            .expect("hostile capability source must remain valid Rust syntax");
        assert!(
            audit_text_search_walk(&unchecked_search).is_err(),
            "literal search lost all checks of its closed deadline authority"
        );
    }

    fn actor_issued_read_capability(
        kind: SourceSetKind,
        profile: SourceProfile,
        deadline: ProviderDeadline,
    ) -> (
        tempfile::TempDir,
        Arc<WorkspaceActor>,
        ActorReadSourceCapability,
    ) {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let context = discover_workspace(Some(workspace.path().to_path_buf())).unwrap();
        let actor = crate::infrastructure::source_workspace::actor_registry()
            .get_or_create(
                &context,
                [WorkspaceSourceSetInput::new(
                    "main",
                    &source,
                    kind,
                    SourceFormat::PlatformXml,
                    profile,
                )],
                "canonical-v0.13",
            )
            .unwrap();
        let binding = actor.bind_provider_root("main", &source).unwrap();
        let identity = format!(
            "{}:{}",
            actor.safe_identity_hash().unwrap().as_str(),
            binding.source_set_name()
        );
        let capability = actor_read_source_capability_for_test(binding, identity, deadline);
        (workspace, actor, capability)
    }

    #[test]
    fn actor_read_authority_builder_rejects_actor_bound_unsupported_profile() {
        let (_workspace, _actor, capability) = actor_issued_read_capability(
            SourceSetKind::Configuration,
            SourceProfile::TestPlatform8_3_28Format2_20,
            ProviderDeadline::from_budget(Duration::from_secs(30)),
        );
        let cancellation = CancellationToken::new();
        let error = capability
            .logical_view_read_authority(&cancellation)
            .err()
            .expect("an unsupported actor-bound profile must fail closed");
        assert_eq!(
            error,
            "actor-bound logical source has no supported platform profile"
        );
    }

    #[test]
    fn actor_read_authority_builder_preserves_actor_bound_source_kind() {
        let (_workspace, _actor, capability) = actor_issued_read_capability(
            SourceSetKind::Extension,
            SourceProfile::platform_xml_8_3_27_format_2_20(),
            ProviderDeadline::from_budget(Duration::from_secs(30)),
        );
        let cancellation = CancellationToken::new();
        let authority = capability
            .logical_view_read_authority(&cancellation)
            .expect("the supported actor-bound profile builds a reader");
        assert_eq!(
            authority.source_set_kind_for_test(),
            SourceSetKind::Extension,
            "the built reader detached source kind from its actor-issued binding"
        );
    }

    #[test]
    fn actor_read_authority_builder_preserves_non_replenishing_deadline() {
        let started = Instant::now();
        set_logical_read_now(started);
        let deadline =
            ProviderDeadline::with_clock(started + Duration::from_secs(30), logical_read_now);
        let (_workspace, _actor, capability) = actor_issued_read_capability(
            SourceSetKind::Configuration,
            SourceProfile::platform_xml_8_3_27_format_2_20(),
            deadline,
        );
        set_logical_read_now(started + Duration::from_secs(11));
        let cancellation = CancellationToken::new();
        let authority = capability
            .logical_view_read_authority(&cancellation)
            .expect("the supported actor-bound profile builds a reader");
        assert_eq!(authority.deadline_for_test(), deadline);
        assert_eq!(
            authority.deadline_for_test().remaining(),
            Some(Duration::from_secs(19)),
            "the authority builder replenished the captured operation deadline"
        );
    }

    #[test]
    pub(crate) fn provider_binding_and_actor_bound_invocation_cannot_substitute_kind_or_profile() {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(
                crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(),
            ),
            Arc::new(TokioClock),
        );
        let request = InvocationRequest::new(
            ToolIdentity::View,
            serde_json::json!({"at": "main:Catalog.Items"}),
            std::fs::canonicalize(workspace.path())
                .unwrap()
                .to_string_lossy(),
            7_000,
        )
        .unwrap();
        let invocation = bind_workspace_invocation_with_source_override_for_test(
            &request,
            &runtime.workspace_actors,
            ActorInvocationResourcesForTest::new(
                Arc::clone(&runtime.deliveries),
                Arc::clone(&runtime.provider_hosts),
                Arc::clone(&runtime.runtime_resources),
                None,
            ),
            runtime.capture_response_deadline_for_test(),
            SourceSetKind::Extension,
            SourceFormat::Edt,
            SourceProfile::TestPlatform8_3_28Format2_20,
        )
        .unwrap();
        let binding = invocation
            .read_source_binding_for_test("main")
            .unwrap()
            .clone();
        let execution = invocation
            .begin_execution(&CancellationToken::new())
            .unwrap();
        let capability = execution.read_sources().unwrap().remove(0);
        let (source_kind, source_format, source_profile) =
            actor_read_source_metadata_for_test(&capability);

        assert_eq!(capability.source_set_name(), binding.source_set_name());
        assert_eq!(source_kind, binding.source_kind());
        assert_eq!(source_format, binding.source_format());
        assert_eq!(source_profile, binding.source_profile());
        assert_eq!(
            crate::domain::source_capabilities::bsl_profile_for_source(source_profile),
            crate::domain::source_capabilities::bsl_profile_for_source(binding.source_profile()),
            "reader profile must be derived from the actor-issued source profile"
        );
    }

    #[test]
    fn production_daemon_configuration_executes_useful_modes_for_all_eight_v13_tools() {
        struct PlatformDocsStandIn;

        impl unica_documentation::model::DocumentationProvider for PlatformDocsStandIn {
            fn id(&self) -> unica_documentation::model::DocumentationProviderId {
                unica_documentation::model::DocumentationProviderId::new("platform-test")
            }

            fn corpora(&self) -> Vec<unica_documentation::model::DocumentationCorpus> {
                vec![unica_documentation::model::DocumentationCorpus {
                    id: "platform-test".to_string(),
                    source_kind: unica_documentation::model::SourceKind::PlatformHelp,
                    authority: unica_documentation::model::Authority::Vendor,
                }]
            }

            fn needs_network(&self) -> bool {
                false
            }

            fn search(
                &self,
                request: &unica_documentation::model::DocumentationSearchRequest,
                _: &unica_documentation::model::DocumentationContext,
            ) -> Vec<unica_documentation::model::DocumentationSection> {
                vec![unica_documentation::model::DocumentationSection::empty(
                    self.id(),
                    "platform-test",
                    unica_documentation::model::SourceKind::PlatformHelp,
                    unica_documentation::model::Authority::Vendor,
                    &request.language,
                )]
            }
        }

        let _docs =
            crate::infrastructure::application_ports::install_documentation_registry_stand_in(
                Arc::new(PlatformDocsStandIn),
            );
        let state_root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        std::fs::create_dir_all(source.join("Catalogs/Items/Ext")).unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects><Catalog>Items</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>Items</Name><Synonym/><Comment/></Properties><ChildObjects/></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Catalogs/Items/Ext/ObjectModule.bsl"),
            "Процедура Проверка()\n    UniqueSearchNeedle = Истина;\nКонецПроцедуры\n",
        )
        .unwrap();
        let config = DaemonServerConfig::new(
            std::fs::canonicalize(state_root.path()).unwrap(),
            CoreIdentity::production(),
            Duration::from_millis(50),
        );
        let runtime =
            V5CanonicalInvocationRuntime::new(config.invocation_service, Arc::new(TokioClock));
        let workspace_hint = std::fs::canonicalize(workspace.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let call = |tool, arguments| {
            let request =
                InvocationRequest::new(tool, arguments, workspace_hint.as_str(), 7_000).unwrap();
            let result = direct_v5(&runtime, request).unwrap_or_else(|error| {
                panic!(
                    "production daemon must accept {} invocation: {error:?}",
                    tool.catalog_name()
                )
            });
            result
        };
        let cases = [
            (
                ToolIdentity::View,
                serde_json::json!({"at": "main:Catalog.Items"}),
                "kind",
            ),
            (
                ToolIdentity::View,
                serde_json::json!({"at": "main:Catalog.Items"}),
                "can",
            ),
            (
                ToolIdentity::View,
                serde_json::json!({"at": "main:Catalog.Items.Can.attribute.add"}),
                "props",
            ),
            (
                ToolIdentity::Resolve,
                serde_json::json!({"at": "main:Catalog.Items"}),
                "path",
            ),
            (
                ToolIdentity::Search,
                serde_json::json!({
                    "query": "UniqueSearchNeedle",
                    "scope": "main:Configuration"
                }),
                "matches",
            ),
            (
                ToolIdentity::Search,
                serde_json::json!({
                    "query": "UniqueSearchNeedle",
                    "scope": "main:Catalog.Items"
                }),
                "matches",
            ),
            (ToolIdentity::Check, serde_json::json!({}), "readinessState"),
            (
                ToolIdentity::Check,
                serde_json::json!({"at": "main:Catalog.Items"}),
                "validators",
            ),
            (
                ToolIdentity::Check,
                serde_json::json!({"at": "main:Catalog.Items", "limit": 1}),
                "diagnosticCount",
            ),
            (
                ToolIdentity::Diff,
                serde_json::json!({
                    "left": "main:Catalog.Items",
                    "right": "main:Catalog.Items"
                }),
                "equal",
            ),
            (ToolIdentity::Run, serde_json::json!({}), "operations"),
            (
                ToolIdentity::Docs,
                serde_json::json!({
                    "query": "Items",
                    "source": "platform-help"
                }),
                "sections",
            ),
            (
                ToolIdentity::Apply,
                serde_json::json!({
                    "at": "main:Catalog.Items",
                    "ops": [{"op": "props.set", "args": {"values": {"Comment": "Preview"}}}],
                }),
                "validated",
            ),
        ];
        for (tool, arguments, expected_data_key) in cases {
            let result = call(tool, arguments);
            assert!(
                result.ok,
                "{} useful mode failed: {} {:?}",
                tool.catalog_name(),
                result.summary,
                result.diagnostics
            );
            assert!(
                result
                    .data
                    .as_ref()
                    .and_then(|data| data.get(expected_data_key))
                    .is_some(),
                "{} omitted data.{expected_data_key}: {:?}",
                tool.catalog_name(),
                result.data
            );
        }
        let run_dictionary = call(ToolIdentity::Run, serde_json::json!({}));
        let operations = run_dictionary
            .data
            .as_ref()
            .and_then(|data| data.get("operations"))
            .and_then(serde_json::Value::as_array)
            .expect("run dictionary has operations");
        // Проектный файл заводит человек, поэтому операции с таким именем в
        // словаре нет вовсе — ни реализованной, ни объявленной.
        assert!(
            operations
                .iter()
                .all(|operation| operation["op"] != "workspace.initialize"),
            "{operations:?}"
        );
        assert!(
            operations.iter().all(|operation| !matches!(
                operation["op"].as_str(),
                Some("syntax.check" | "test.run" | "query.execute" | "source.create")
            )),
            "v0.13 Run discovery must omit deferred check/test/query execution: {operations:?}"
        );
        let object_search = call(
            ToolIdentity::Search,
            serde_json::json!({
                "query": "UniqueSearchNeedle",
                "scope": "main:Catalog.Items"
            }),
        );
        let first_match = &object_search.data.as_ref().unwrap()["matches"][0];
        assert_eq!(first_match["scope"], "main:Catalog.Items");
        assert!(first_match.get("file").is_none());
        // Область поиска и читаемый адрес совпадения — разные поля (#1298).
        assert_eq!(
            first_match["at"],
            "main:Catalog.Items.Module.Object.Method.Проверка.Body#L2"
        );
        assert!(
            !std::fs::read_to_string(source.join("Catalogs/Items.xml"))
                .unwrap()
                .contains("<Comment>Preview</Comment>"),
            "dryRun must use the real planner without publishing its postimage"
        );
        // Второй свод того же вопроса: имена и синонимы метаданных. Ответ
        // несёт `at`, `kind`, `title` — доказательство совпадения по имени, —
        // и по-прежнему ни одного пути.
        let by_name = call(
            ToolIdentity::Search,
            serde_json::json!({"query": "Items", "corpus": "names"}),
        );
        assert!(by_name.ok, "{by_name:?}");
        let named = &by_name.data.as_ref().unwrap()["matches"][0];
        assert_eq!(named["at"], "main:Catalog.Items");
        assert!(named.get("kind").is_some(), "{named}");
        assert!(
            named.get("path").is_none() && named.get("file").is_none(),
            "путь в частом ответе зовёт обойти адресное пространство: {named}"
        );
        assert_eq!(
            by_name.data.as_ref().unwrap()["approximate"],
            serde_json::json!(false),
            "точное совпадение догадкой не является: {by_name:?}"
        );

        // Терпимость к опечатке переехала сюда из локатора и названа явно:
        // читатель обязан отличать «нашлось» от «похоже на».
        let mistyped = call(
            ToolIdentity::Search,
            serde_json::json!({"query": "Itmes", "corpus": "names"}),
        );
        assert!(mistyped.ok, "{mistyped:?}");
        assert_eq!(
            mistyped.data.as_ref().unwrap()["approximate"],
            serde_json::json!(true),
            "совпадение по близости обязано называться догадкой: {mistyped:?}"
        );

        let unknown_corpus = call(
            ToolIdentity::Search,
            serde_json::json!({"query": "Items", "corpus": "paths"}),
        );
        assert_eq!(unknown_corpus.diagnostics[0]["code"], "bad_value");
        assert!(
            unknown_corpus
                .next
                .iter()
                .any(|entry| entry["args"]["corpus"] == "names"),
            "отказ обязан назвать доступный свод: {unknown_corpus:?}"
        );

        // Применение связано с предпросмотром забором ревизии, поэтому путь
        // через провод всегда двухшаговый.
        let previewed_apply = call(
            ToolIdentity::Apply,
            serde_json::json!({
                "at": "main:Catalog.Items",
                "ops": [{
                    "op": "props.set",
                    "args": {"values": {"Comment": "Published through v0.13"}}
                }],
            }),
        );
        assert!(previewed_apply.ok, "{previewed_apply:?}");
        let published_apply = call(
            ToolIdentity::Apply,
            serde_json::json!({"executionToken": previewed_apply.data.as_ref().unwrap()["executionToken"]}),
        );
        assert!(
            published_apply.ok,
            "supported metadata apply must publish: {published_apply:?}"
        );
        assert!(published_apply.rev.is_some());
        assert_eq!(
            published_apply
                .data
                .as_ref()
                .and_then(|data| data.get("mode")),
            Some(&serde_json::json!("published"))
        );
        assert!(std::fs::read_to_string(source.join("Catalogs/Items.xml"))
            .unwrap()
            .contains("<Comment>Published through v0.13</Comment>"));
        let rejected_batch = call(
            ToolIdentity::Apply,
            serde_json::json!({
                "at": "main:Catalog.Items",
                "ops": [
                    {
                        "op": "props.set",
                        "args": {"values": {"Comment": "must not publish"}}
                    },
                    {"op": "object.create", "args": {"values": {"kind": "Catalog", "name": "Ghost"}}}
                ],
            }),
        );
        // `object.create` addresses the configuration root, so naming it on
        // a catalog is a caller mistake; the batch must still publish nothing.
        assert!(!rejected_batch.ok);
        assert_eq!(rejected_batch.diagnostics[0]["code"], "bad_value");
        let after_rejected_batch =
            std::fs::read_to_string(source.join("Catalogs/Items.xml")).unwrap();
        assert!(after_rejected_batch.contains("<Comment>Published through v0.13</Comment>"));
        assert!(!after_rejected_batch.contains("must not publish"));
        let unsupported_cases = [
            (
                ToolIdentity::Run,
                serde_json::json!({"op": "syntax.check", "args": {"mode": "shell"}}),
                "unsupported_operation",
            ),
            // `launch` реализован: пустые аргументы — ошибка вызова, а не
            // нереализованная операция.
            (
                ToolIdentity::Run,
                serde_json::json!({"op": "launch", "args": {}}),
                "bad_value",
            ),
            // `make` реализован: пустые аргументы — ошибка вызова.
            (
                ToolIdentity::Run,
                serde_json::json!({"op": "make", "args": {}}),
                "bad_value",
            ),
            (
                ToolIdentity::Run,
                serde_json::json!({"op": "query.execute", "args": {}}),
                "unsupported_operation",
            ),
            (
                ToolIdentity::Search,
                serde_json::json!({"query": "needle", "regex": "yes"}),
                "bad_value",
            ),
            (
                ToolIdentity::Search,
                serde_json::json!({
                    "query": "needle",
                    "scope": "main:Catalog.Items.Attribute.Code"
                }),
                "unsupported_scope",
            ),
            // Validator selection follows the node kind, not a caller filter.
            (
                ToolIdentity::Check,
                serde_json::json!({"filter": {"severity": "warning"}}),
                "bad_value",
            ),
            (
                ToolIdentity::Check,
                serde_json::json!({
                    "at": "main:Catalog.Items",
                    "filter": {"validation": {"profile": "form"}}
                }),
                "bad_value",
            ),
            (
                ToolIdentity::Diff,
                serde_json::json!({
                    "left": "main:Catalog.Items",
                    "right": "main:Catalog.Items",
                    "cursor": "opaque"
                }),
                // A cursor no first page issued continues nothing (#977).
                "invalid_cursor",
            ),
            (
                ToolIdentity::Diff,
                serde_json::json!({
                    "left": "main:Catalog.Items",
                    "right": "main:Catalog.Items",
                    "filter": "changes"
                }),
                "bad_value",
            ),
            (
                ToolIdentity::Docs,
                serde_json::json!({"query": "Items", "source": "unknown"}),
                "unsupported_source",
            ),
            (
                ToolIdentity::Docs,
                serde_json::json!({
                    "query": "Items",
                    "source": "configuration-documentation"
                }),
                "unsupported_source",
            ),
        ];
        for (tool, arguments, expected_code) in unsupported_cases {
            let unsupported = call(tool, arguments);
            assert!(
                !unsupported.ok,
                "{} must reject honestly",
                tool.catalog_name()
            );
            let diagnostic = unsupported.diagnostics.first().unwrap_or_else(|| {
                panic!(
                    "{} rejected without a diagnostic: {unsupported:?}",
                    tool.catalog_name()
                )
            });
            assert_eq!(
                diagnostic["code"],
                expected_code,
                "{} returned a misleading diagnostic: {unsupported:?}",
                tool.catalog_name()
            );
        }
    }

    /// A daemon over a workspace with two catalogs of different structure and
    /// a module that matches the search needle twice.
    fn empty_cursor_and_diff_fixture() -> (
        tempfile::TempDir,
        tempfile::TempDir,
        V5CanonicalInvocationRuntime,
        String,
    ) {
        let state_root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        std::fs::create_dir_all(source.join("Catalogs/Items/Ext")).unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects><Catalog>Items</Catalog><Catalog>Other</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>Items</Name><Synonym/><Comment>first</Comment></Properties><ChildObjects/></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Catalogs/Other.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="cccccccc-cccc-4ccc-8ccc-cccccccccccc"><Properties><Name>Other</Name><Synonym/><Comment>second</Comment></Properties><ChildObjects/></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Catalogs/Items/Ext/ObjectModule.bsl"),
            "Процедура Проверка()\n    PagedNeedle = 1;\n    PagedNeedle = 2;\nКонецПроцедуры\n",
        )
        .unwrap();
        let config = DaemonServerConfig::new(
            std::fs::canonicalize(state_root.path()).unwrap(),
            CoreIdentity::production(),
            Duration::from_millis(50),
        );
        let runtime =
            V5CanonicalInvocationRuntime::new(config.invocation_service, Arc::new(TokioClock));
        let workspace_hint = std::fs::canonicalize(workspace.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        (state_root, workspace, runtime, workspace_hint)
    }

    /// #1216: a form client sends an untouched optional string as `""`. For
    /// every tool that publishes `cursor`, the empty string asks for the
    /// first page exactly like an omitted cursor, and a first page that has
    /// more still issues a real continuation.
    #[test]
    fn empty_cursor_reads_the_first_page_of_every_tool_that_publishes_one() {
        struct PlatformDocsStandIn;

        impl unica_documentation::model::DocumentationProvider for PlatformDocsStandIn {
            fn id(&self) -> unica_documentation::model::DocumentationProviderId {
                unica_documentation::model::DocumentationProviderId::new("platform-test")
            }

            fn corpora(&self) -> Vec<unica_documentation::model::DocumentationCorpus> {
                vec![unica_documentation::model::DocumentationCorpus {
                    id: "platform-test".to_string(),
                    source_kind: unica_documentation::model::SourceKind::PlatformHelp,
                    authority: unica_documentation::model::Authority::Vendor,
                }]
            }

            fn needs_network(&self) -> bool {
                false
            }

            fn search(
                &self,
                request: &unica_documentation::model::DocumentationSearchRequest,
                _: &unica_documentation::model::DocumentationContext,
            ) -> Vec<unica_documentation::model::DocumentationSection> {
                vec![unica_documentation::model::DocumentationSection::empty(
                    self.id(),
                    "platform-test",
                    unica_documentation::model::SourceKind::PlatformHelp,
                    unica_documentation::model::Authority::Vendor,
                    &request.language,
                )]
            }
        }

        let _docs =
            crate::infrastructure::application_ports::install_documentation_registry_stand_in(
                Arc::new(PlatformDocsStandIn),
            );
        let (_state, _workspace, runtime, workspace_hint) = empty_cursor_and_diff_fixture();
        let call = |tool, arguments| {
            let request =
                InvocationRequest::new(tool, arguments, workspace_hint.as_str(), 7_000).unwrap();
            direct_v5(&runtime, request).unwrap()
        };
        let planned = call(
            ToolIdentity::Apply,
            serde_json::json!({
                "at": "main:Catalog.Items",
                "ops": [{"op": "props.set", "args": {"values": {"Comment": "saved empty cursor"}}}]
            }),
        );
        assert!(planned.ok, "{planned:?}");
        let token = planned.data.as_ref().unwrap()["executionToken"].clone();
        assert!(token.as_str().is_some_and(|token| !token.is_empty()));
        let cases = [
            (ToolIdentity::View, serde_json::json!({}), false),
            (
                ToolIdentity::View,
                serde_json::json!({"at": "main:Catalog.Items.Module.Object.Event", "limit": 1}),
                true,
            ),
            (
                ToolIdentity::Search,
                serde_json::json!({"query": "PagedNeedle", "limit": 1}),
                true,
            ),
            (
                ToolIdentity::Diff,
                serde_json::json!({"executionToken": token, "limit": 1}),
                true,
            ),
            (
                ToolIdentity::Diff,
                serde_json::json!({
                    "left": "main:Catalog.Items",
                    "right": "main:Catalog.Other",
                    "limit": 1
                }),
                true,
            ),
            (ToolIdentity::Check, serde_json::json!({}), false),
            (
                ToolIdentity::Check,
                serde_json::json!({"at": "main:Catalog.Items"}),
                false,
            ),
            (
                ToolIdentity::Docs,
                serde_json::json!({"query": "Items", "source": "platform-help"}),
                false,
            ),
        ];
        // A tool that starts publishing `cursor` must join the cases above.
        let mut covered = cases
            .iter()
            .map(|(tool, _, _)| tool.catalog_name())
            .collect::<Vec<_>>();
        covered.sort_unstable();
        covered.dedup();
        let mut expected = crate::application::v13::tool_catalog::catalog_for(
            crate::application::tool_contracts::SurfaceRelease::V13,
        )
        .unwrap()
        .tools
        .into_iter()
        .filter(|contract| contract.input_schema["properties"].get("cursor").is_some())
        .map(|contract| contract.name)
        .collect::<Vec<_>>();
        expected.sort_unstable();
        assert_eq!(covered, expected, "tools that publish cursor");
        for (tool, arguments, continues) in cases {
            let omitted = call(tool, arguments.clone());
            let mut with_empty = arguments.clone();
            with_empty
                .as_object_mut()
                .unwrap()
                .insert("cursor".into(), serde_json::json!(""));
            let empty = call(tool, with_empty);
            let name = tool.catalog_name();
            assert!(omitted.ok, "{name} {arguments}: {omitted:?}");
            assert!(empty.ok, "{name} {arguments} with cursor \"\": {empty:?}");
            assert_eq!(empty.data, omitted.data, "{name} {arguments}");
            assert_eq!(empty.page, omitted.page, "{name} {arguments}");
            assert_eq!(
                empty.cursor.is_some(),
                continues,
                "{name} {arguments}: first page continuation"
            );
        }
    }

    #[test]
    fn saved_plan_diff_default_limit_pages_twenty_chunks_and_replays() {
        let (_state, _workspace, runtime, workspace_hint) = empty_cursor_and_diff_fixture();
        let source = std::path::Path::new(&workspace_hint).join("src");
        let before = unica_workspace::test_support::tree_snapshot(&source);
        let call = |tool, arguments| {
            let request =
                InvocationRequest::new(tool, arguments, workspace_hint.as_str(), 7_000).unwrap();
            direct_v5(&runtime, request).unwrap()
        };
        // A real metadata edit produces more than twenty 2048-byte source chunks.
        // Twenty ASCII chunks still fit the 64 KiB encoded-page bound.
        let planned = call(
            ToolIdentity::Apply,
            serde_json::json!({
                "at": "main:Catalog.Items",
                "ops": [{"op": "props.set", "args": {"values": {"Comment": "x".repeat(50 * 1024)}}}]
            }),
        );
        assert!(planned.ok, "{planned:?}");
        assert_eq!(planned.data.as_ref().unwrap()["mode"], "preview");
        let token = planned.data.as_ref().unwrap()["executionToken"].clone();
        assert!(token.as_str().is_some_and(|token| !token.is_empty()));
        let first_args = serde_json::json!({"executionToken": token.clone()});
        let omitted = call(ToolIdentity::Diff, first_args.clone());
        let explicit_twenty = call(
            ToolIdentity::Diff,
            serde_json::json!({"executionToken": token.clone(), "limit": 20}),
        );
        assert!(omitted.ok, "{omitted:?}");
        assert!(explicit_twenty.ok, "{explicit_twenty:?}");
        assert_eq!(
            serde_json::to_value(&omitted).unwrap(),
            serde_json::to_value(&explicit_twenty).unwrap(),
            "omitting limit uses the same real twenty-chunk boundary"
        );
        let first = omitted.data.as_ref().unwrap();
        assert_eq!(first["chunks"].as_array().unwrap().len(), 20);
        assert_eq!(first["truncated"], true);
        assert_eq!(first["planIdentity"].as_str(), planned.rev.as_deref());
        assert_eq!(omitted.page.as_ref().unwrap()["stoppedBy"], "limit");
        let cursor = omitted.cursor.as_deref().expect("more than twenty chunks");
        assert_eq!(first["cursor"], cursor);
        assert_eq!(omitted.next.len(), 1);
        assert_eq!(omitted.next[0]["tool"], "unica.diff");
        assert_eq!(omitted.next[0]["arguments"]["executionToken"], token);
        assert_eq!(omitted.next[0]["arguments"]["limit"], 20);
        assert_eq!(omitted.next[0]["arguments"]["cursor"], cursor);
        let first_replay = call(ToolIdentity::Diff, first_args);
        assert_eq!(
            serde_json::to_value(&first_replay).unwrap(),
            serde_json::to_value(&omitted).unwrap()
        );
        // Keep limit omitted when following the issued cursor as well.
        let continuation_args = serde_json::json!({"executionToken": token, "cursor": cursor});
        let last = call(ToolIdentity::Diff, continuation_args.clone());
        assert!(last.ok, "{last:?}");
        let tail = last.data.as_ref().unwrap();
        let tail_len = tail["chunks"].as_array().unwrap().len();
        assert!(tail_len > 0 && tail_len <= 20);
        assert!(first["chunks"].as_array().unwrap().len() + tail_len > 20);
        assert_eq!(tail["planIdentity"], first["planIdentity"]);
        assert_eq!(tail["truncated"], false);
        assert!(tail["cursor"].is_null());
        assert!(last.cursor.is_none());
        assert!(last.next.is_empty());
        assert_eq!(last.page.as_ref().unwrap()["stoppedBy"], "complete");
        let last_replay = call(ToolIdentity::Diff, continuation_args);
        assert_eq!(
            serde_json::to_value(&last_replay).unwrap(),
            serde_json::to_value(&last).unwrap()
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&source),
            before,
            "planning and all saved-source page reads stay write-free"
        );
    }

    #[test]
    fn saved_plan_diff_projects_its_continuation_in_the_canonical_envelope() {
        let workspace = subsystem_picture_workspace("", true);
        let source = workspace.path().join("src");
        let before = unica_workspace::test_support::tree_snapshot(&source);
        let runtime = bootstrap_runtime();
        let preview = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            serde_json::json!({"at": "main:Subsystem.Sales", "ops": [
                {"op": "props.set", "args": {"values": {"Comment": "saved cursor projection"}}}
            ]}),
        );
        assert!(preview.ok, "{preview:?}");
        let token = preview.data.as_ref().unwrap()["executionToken"].clone();
        let first = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Diff,
            serde_json::json!({"executionToken": token.clone(), "limit": 1}),
        );
        assert!(first.ok, "{first:?}");
        let data = first.data.as_ref().unwrap();
        let payload_cursor = data["cursor"]
            .as_str()
            .expect("two saved sides need a continuation");
        assert_eq!(
            first.cursor.as_deref(),
            Some(payload_cursor),
            "the canonical envelope must publish the saved-plan continuation"
        );
        assert_eq!(data["planIdentity"].as_str(), preview.rev.as_deref());
        assert_eq!(data["chunks"].as_array().unwrap().len(), 1);
        assert_eq!(data["truncated"], true);
        assert_eq!(first.page.as_ref().unwrap()["stoppedBy"], "limit");
        assert_eq!(first.next.len(), 1);
        assert_eq!(first.next[0]["arguments"]["cursor"], data["cursor"]);
        assert_eq!(first.next[0]["arguments"]["executionToken"], token);
        let cursor = first.cursor.clone().unwrap();
        let continuation_args =
            serde_json::json!({"executionToken": token.clone(), "limit": 1, "cursor": cursor});
        let terminal = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Diff,
            continuation_args.clone(),
        );
        assert!(terminal.ok, "{terminal:?}");
        assert!(
            terminal.cursor.is_none(),
            "terminal envelope has no continuation"
        );
        assert!(terminal.data.as_ref().unwrap()["cursor"].is_null());
        assert_eq!(terminal.data.as_ref().unwrap()["truncated"], false);
        assert_eq!(terminal.page.as_ref().unwrap()["stoppedBy"], "complete");
        assert!(terminal.next.is_empty());
        let repeated = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Diff,
            continuation_args,
        );
        assert_eq!(
            serde_json::to_value(&repeated).unwrap(),
            serde_json::to_value(&terminal).unwrap()
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&source),
            before,
            "planning and cursor reads must not publish source edits"
        );
        let applied = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            serde_json::json!({"executionToken": token.clone()}),
        );
        assert!(applied.ok, "{applied:?}");
        let published = unica_workspace::test_support::tree_snapshot(&source);
        assert_ne!(published, before);
        let replay = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            serde_json::json!({"executionToken": token}),
        );
        assert_eq!(
            serde_json::to_value(&replay).unwrap(),
            serde_json::to_value(&applied).unwrap()
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&source),
            published,
            "applying the retained token again must replay without another source write"
        );
    }

    /// #977: a cut left/right comparison continues on the canonical route.
    /// Pages walk the whole difference once, in order; the cursor answers
    /// only the same pair and page size and keeps the pair the first page
    /// read; nothing is published.
    #[test]
    fn diff_pages_continue_the_saved_pair_on_the_canonical_route() {
        let (_state, workspace, runtime, workspace_hint) = empty_cursor_and_diff_fixture();
        let source = workspace.path().join("src");
        let call = |arguments| {
            let request = InvocationRequest::new(
                ToolIdentity::Diff,
                arguments,
                workspace_hint.as_str(),
                7_000,
            )
            .unwrap();
            direct_v5(&runtime, request).unwrap()
        };
        let before = unica_workspace::test_support::tree_snapshot(&source);
        let pair = serde_json::json!({
            "left": "main:Catalog.Items",
            "right": "main:Catalog.Other"
        });
        let whole = call(pair.clone());
        assert!(whole.ok, "{whole:?}");
        assert_eq!(whole.data.as_ref().unwrap()["truncated"], false);
        assert!(whole.cursor.is_none(), "{whole:?}");
        let all = whole.data.as_ref().unwrap()["changes"]
            .as_array()
            .unwrap()
            .clone();
        assert!(all.len() > 2, "fixture must differ in more than two places");

        let mut first_args = pair.clone();
        first_args["limit"] = serde_json::json!(1);
        let first = call(first_args.clone());
        assert!(first.ok, "{first:?}");
        assert_eq!(first.data.as_ref().unwrap()["truncated"], true);
        assert_eq!(first.data.as_ref().unwrap()["equal"], false);
        assert_eq!(first.page.as_ref().unwrap()["stoppedBy"], "limit");
        let first_cursor = first.cursor.clone().expect("a cut page issues a cursor");
        assert_eq!(first.next[0]["tool"], "unica.diff");
        assert_eq!(first.next[0]["arguments"]["cursor"], first_cursor.as_str());
        assert_eq!(first.next[0]["arguments"]["limit"], 1);

        let mut empty = first_args.clone();
        empty["cursor"] = serde_json::json!("");
        let restarted = call(empty);
        assert_eq!(
            restarted.data.as_ref().unwrap()["changes"],
            first.data.as_ref().unwrap()["changes"]
        );

        let mut paged = Vec::new();
        let mut cursors = Vec::new();
        let mut page = first.clone();
        loop {
            paged.extend(
                page.data.as_ref().unwrap()["changes"]
                    .as_array()
                    .unwrap()
                    .clone(),
            );
            let Some(cursor) = page.cursor.clone() else {
                assert_eq!(page.data.as_ref().unwrap()["truncated"], false);
                assert_eq!(page.page.as_ref().unwrap()["stoppedBy"], "complete");
                break;
            };
            assert!(paged.len() <= all.len(), "pages repeat: {paged:?}");
            cursors.push(cursor.clone());
            let mut next = first_args.clone();
            next["cursor"] = serde_json::json!(cursor);
            page = call(next);
            assert!(page.ok, "{page:?}");
        }
        assert_eq!(
            paged, all,
            "pages must walk the whole difference once, in order"
        );

        let mut over = pair.clone();
        over["limit"] = serde_json::json!(51);
        let refused = call(over);
        assert_eq!(refused.diagnostics[0]["code"], "bad_value", "{refused:?}");
        for forged in ["opaque".to_owned(), format!("{first_cursor}0")] {
            let mut replay = first_args.clone();
            replay["cursor"] = serde_json::json!(forged);
            let refused = call(replay);
            assert_eq!(
                refused.diagnostics[0]["code"], "invalid_cursor",
                "{forged}: {refused:?}"
            );
        }
        for (field, value) in [
            ("limit", serde_json::json!(2)),
            ("right", serde_json::json!("main:Catalog.Items")),
            ("left", serde_json::json!("main:Catalog.Other")),
        ] {
            let mut replay = first_args.clone();
            replay[field] = value;
            replay["cursor"] = serde_json::json!(first_cursor.as_str());
            let refused = call(replay);
            assert!(!refused.ok, "{field}: {refused:?}");
            assert_eq!(
                refused.diagnostics[0]["code"], "invalid_cursor",
                "{field}: {refused:?}"
            );
        }
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&source),
            before,
            "comparison must not publish source edits"
        );

        // The continuation into the comment the edit below replaces: one that
        // read the sources again would answer `edited`.
        let comment = all
            .iter()
            .position(|change| change["path"] == "/props/Comment")
            .expect("fixture differs in its comment");
        assert!(comment > 0, "{all:?}");
        assert_eq!(all[comment]["right"], "second", "{all:?}");
        let into_comment = cursors[comment - 1].clone();
        let other = source.join("Catalogs/Other.xml");
        let edited = std::fs::read_to_string(&other)
            .unwrap()
            .replace("<Comment>second</Comment>", "<Comment>edited</Comment>");
        std::fs::write(&other, edited).unwrap();
        let mut continued = first_args.clone();
        continued["cursor"] = serde_json::json!(into_comment);
        let saved = call(continued);
        assert!(saved.ok, "{saved:?}");
        assert_eq!(
            saved.data.as_ref().unwrap()["changes"],
            serde_json::json!([all[comment].clone()]),
            "a continuation reads the pair saved by its first page"
        );
        // A cursor answers only the workspace that asked the first page.
        let (_other_state, _other_workspace, _other_runtime, other_hint) =
            empty_cursor_and_diff_fixture();
        let mut elsewhere = first_args.clone();
        elsewhere["cursor"] = serde_json::json!(first_cursor.as_str());
        let request =
            InvocationRequest::new(ToolIdentity::Diff, elsewhere, other_hint.as_str(), 7_000)
                .unwrap();
        let foreign = direct_v5(&runtime, request).unwrap();
        assert_eq!(
            foreign.diagnostics[0]["code"], "invalid_cursor",
            "{foreign:?}"
        );
        let fresh = call(pair);
        assert!(fresh.ok, "{fresh:?}");
        assert!(
            fresh.data.as_ref().unwrap().to_string().contains("edited"),
            "a new first page reads the edited source: {fresh:?}"
        );
    }

    /// #977: nodes of different kinds are not compared, and an unreadable
    /// side is a refusal, never an empty node.
    #[test]
    fn diff_refuses_incomparable_kinds_and_an_unreadable_side() {
        let (_state, _workspace, runtime, workspace_hint) = empty_cursor_and_diff_fixture();
        let call = |arguments| {
            let request = InvocationRequest::new(
                ToolIdentity::Diff,
                arguments,
                workspace_hint.as_str(),
                7_000,
            )
            .unwrap();
            direct_v5(&runtime, request).unwrap()
        };
        let incomparable = call(serde_json::json!({
            "left": "main:Catalog.Items",
            "right": "main:Catalog.Items.Module.Object"
        }));
        assert!(!incomparable.ok, "{incomparable:?}");
        assert_eq!(
            incomparable.diagnostics[0]["code"], "incomparable_nodes",
            "{incomparable:?}"
        );
        let missing = call(serde_json::json!({
            "left": "main:Catalog.Items",
            "right": "main:Catalog.Missing"
        }));
        assert!(!missing.ok, "{missing:?}");
        assert!(missing.data.is_none(), "{missing:?}");
        assert_eq!(missing.diagnostics[0]["code"], "not_found", "{missing:?}");
    }

    /// `filter` снят с `view` и `diff` (#1374). Вызов с ним — отказ
    /// неизвестного аргумента, но `next` ведёт к адресу, который заменил то,
    /// что фильтр делал: к контракту операции `<узел>.Can.<op>`, к самому
    /// узлу с его `can`, к сравнению без фильтра.
    #[test]
    fn a_retired_read_filter_routes_next_to_the_address_that_replaced_it() {
        let request =
            |tool, arguments| InvocationRequest::new(tool, arguments, "/workspace", 7_000).unwrap();
        let view = super::retired_read_filter(&request(
            ToolIdentity::View,
            serde_json::json!({
                "at": "cf:Report.F05Report.Template.F05Schema.DataSet.F05Data",
                "filter": {"sections": ["can"], "can": {"op": "query.set"}}
            }),
        ))
        .expect("a view filter is refused");
        assert!(!view.ok);
        assert_eq!(view.diagnostics[0]["code"], "bad_value");
        assert!(view.diagnostics[0]["message"]
            .as_str()
            .is_some_and(|message| message.contains("unknown argument `filter`")));
        let targets = view
            .next
            .iter()
            .map(|entry| (entry["tool"].clone(), entry["args"]["at"].clone()))
            .collect::<Vec<_>>();
        assert_eq!(
            targets,
            [
                (
                    serde_json::json!("unica.view"),
                    serde_json::json!(
                        "cf:Report.F05Report.Template.F05Schema.DataSet.F05Data.Can.query.set"
                    )
                ),
                (
                    serde_json::json!("unica.view"),
                    serde_json::json!("cf:Report.F05Report.Template.F05Schema.DataSet.F05Data")
                ),
            ]
        );
        let sections_only = super::retired_read_filter(&request(
            ToolIdentity::View,
            serde_json::json!({"at": "main:Catalog.Валюты", "filter": {"sections": ["props"]}}),
        ))
        .expect("a sections filter is refused");
        assert_eq!(sections_only.next.len(), 1);
        assert_eq!(
            sections_only.next[0]["args"],
            serde_json::json!({"at": "main:Catalog.Валюты"})
        );

        let diff = super::retired_read_filter(&request(
            ToolIdentity::Diff,
            serde_json::json!({
                "left": "main:Enum.A",
                "right": "main:Enum.B",
                "filter": {"paths": ["/props/synonym"]}
            }),
        ))
        .expect("a diff filter is refused");
        assert_eq!(diff.diagnostics[0]["code"], "bad_value");
        assert_eq!(
            diff.next[0]["args"],
            serde_json::json!({"left": "main:Enum.A", "right": "main:Enum.B"})
        );

        let workspace = super::retired_read_filter(&request(
            ToolIdentity::View,
            serde_json::json!({"filter": {"sections": ["props"]}}),
        ))
        .expect("a filter without at is refused too");
        assert_eq!(workspace.next[0]["args"], serde_json::json!({}));

        for (tool, arguments) in [
            (
                ToolIdentity::View,
                serde_json::json!({"at": "main:Catalog.Валюты"}),
            ),
            (
                ToolIdentity::Check,
                serde_json::json!({"at": "main:Catalog.Валюты", "filter": {}}),
            ),
        ] {
            assert!(
                super::retired_read_filter(&request(tool, arguments)).is_none(),
                "{}: only a view or diff filter is retired here",
                tool.catalog_name()
            );
        }
    }

    /// Адрес операции словаря читает только `view`: `check`, `resolve`,
    /// `apply` (в том числе адрес внутри операции) и область `search`
    /// отвечают одним отказом с переходом к этому чтению.
    #[test]
    fn an_operation_address_outside_view_routes_next_to_view() {
        let can = "main:Catalog.Валюты.Can.attribute.add";
        let request =
            |tool, arguments| InvocationRequest::new(tool, arguments, "/workspace", 7_000).unwrap();
        for (tool, arguments, location) in [
            (ToolIdentity::Check, serde_json::json!({"at": can}), "at"),
            (ToolIdentity::Resolve, serde_json::json!({"at": can}), "at"),
            (
                ToolIdentity::Search,
                serde_json::json!({"query": "x", "scope": can}),
                "scope",
            ),
            (
                ToolIdentity::Apply,
                serde_json::json!({"at": can, "ops": [{"op": "props.set"}]}),
                "at",
            ),
            (
                ToolIdentity::Apply,
                serde_json::json!({"at": "main:Catalog.Валюты", "ops": [
                    {"op": "props.set", "args": {"values": {}}},
                    {"op": "attribute.add", "args": {"at": can, "items": []}}
                ]}),
                "ops[1].args.at",
            ),
        ] {
            let name = tool.catalog_name();
            let refused = super::operation_address_outside_view(&request(tool, arguments))
                .unwrap_or_else(|| panic!("{name}: a Can address is refused"));
            assert!(!refused.ok, "{name}");
            assert_eq!(refused.diagnostics[0]["code"], "bad_value", "{name}");
            assert_eq!(refused.diagnostics[0]["outcome"], "fixCall", "{name}");
            assert_eq!(refused.at.as_deref(), Some(location), "{name}");
            assert_eq!(
                refused.diagnostics[0]["message"],
                format!("`{can}` is an operation-dictionary address; only unica.view reads it"),
                "{name}"
            );
            assert_eq!(
                refused.next,
                vec![serde_json::json!({
                    "tool": "unica.view",
                    "args": {"at": can},
                    "reason": "контракт операции читает view",
                })],
                "{name}"
            );
        }
        for (tool, arguments) in [
            (
                ToolIdentity::Check,
                serde_json::json!({"at": "main:Catalog.Валюты"}),
            ),
            (
                ToolIdentity::Check,
                serde_json::json!({"at": "main:Catalog.Can"}),
            ),
            (ToolIdentity::View, serde_json::json!({"at": can})),
            (
                ToolIdentity::Diff,
                serde_json::json!({"left": can, "right": can}),
            ),
        ] {
            assert!(
                super::operation_address_outside_view(&request(tool, arguments)).is_none(),
                "{}: not refused here",
                tool.catalog_name()
            );
        }
    }

    /// INV.WIRE.V13-REFUSAL-CHANNEL: every canonical refusal answers through
    /// `diagnostics[0]` with a code from the closed set and a message; a stale
    /// saved apply plan has its own conflict code instead of `provider_unavailable`;
    /// and an admitted logical scope without a source subtree is an empty
    /// result rather than a refusal or a raw OS error.
    #[test]
    fn canonical_refusals_answer_one_diagnostics_channel_from_the_closed_code_set() {
        const CLOSED_REFUSAL_CODES: &[&str] = &[
            "bad_value",
            "not_found",
            "stale_revision",
            "unsupported_operation",
            "unsupported_scope",
            "unsupported_cursor",
            "unsupported_source",
            "unsupported_section",
            "target_kind_unsupported",
            "invalid_state",
            "invalid_source",
            "source_selection_changed",
            "rollback_incomplete",
            "provider_unavailable",
        ];

        let state_root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        std::fs::create_dir_all(source.join("Catalogs")).unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects><Catalog>Bare</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        // `Bare` exists as a logical object but ships no `Catalogs/Bare/`
        // source subtree, so a scoped search has nothing to walk.
        std::fs::write(
            source.join("Catalogs/Bare.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"><Properties><Name>Bare</Name><Synonym/><Comment/></Properties><ChildObjects/></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        let config = DaemonServerConfig::new(
            std::fs::canonicalize(state_root.path()).unwrap(),
            CoreIdentity::production(),
            Duration::from_millis(50),
        );
        let runtime =
            V5CanonicalInvocationRuntime::new(config.invocation_service, Arc::new(TokioClock));
        let workspace_hint = std::fs::canonicalize(workspace.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let call = |tool, arguments| {
            let request =
                InvocationRequest::new(tool, arguments, workspace_hint.as_str(), 7_000).unwrap();
            direct_v5(&runtime, request).unwrap()
        };

        let refusals = [
            (
                ToolIdentity::View,
                serde_json::json!({
                    "at": "main:Catalog.Bare",
                    "filter": {"sections": ["can"]}
                }),
                "bad_value",
            ),
            (
                ToolIdentity::Apply,
                serde_json::json!({
                    "at": "main:Catalog.Bare",
                    "ops": [{"op": "frobnicate"}],
                }),
                "unsupported_operation",
            ),
            (
                ToolIdentity::Apply,
                serde_json::json!({
                    "executionToken": "unknown-plan"
                }),
                "bad_value",
            ),
            (
                ToolIdentity::Run,
                serde_json::json!({"op": "syntax.check", "args": {}}),
                "unsupported_operation",
            ),
            (
                ToolIdentity::View,
                serde_json::json!({"at": "main:Catalog.Bare.Can.frobnicate"}),
                "bad_value",
            ),
            (
                ToolIdentity::View,
                serde_json::json!({"at": "main:Catalog.Bare.Can.enumValue.add"}),
                "bad_value",
            ),
            (
                ToolIdentity::Diff,
                serde_json::json!({
                    "left": "main:Catalog.Bare",
                    "right": "main:Catalog.Bare",
                    "filter": {"paths": ["/props"]}
                }),
                "bad_value",
            ),
            (
                ToolIdentity::Diff,
                serde_json::json!({
                    "left": "main:Catalog.Bare.Can.props.set",
                    "right": "main:Catalog.Bare.Can.props.set"
                }),
                "target_kind_unsupported",
            ),
            (
                ToolIdentity::Apply,
                serde_json::json!({
                    "at": "main:Catalog.Bare",
                    "ops": [{"op": "props.set", "args": {"props": {"Comment": "x"}}}],
                }),
                "bad_value",
            ),
        ];
        for (tool, arguments, expected_code) in refusals {
            let refusal = call(tool, arguments);
            assert!(!refusal.ok, "{} must refuse", tool.catalog_name());
            let diagnostic = refusal.diagnostics.first().unwrap_or_else(|| {
                panic!(
                    "{} refused outside the diagnostics channel: {refusal:?}",
                    tool.catalog_name()
                )
            });
            assert_eq!(
                diagnostic["code"],
                expected_code,
                "{}: {refusal:?}",
                tool.catalog_name()
            );
            assert!(
                CLOSED_REFUSAL_CODES
                    .iter()
                    .any(|code| diagnostic["code"] == *code),
                "{} answered outside the closed code set: {refusal:?}",
                tool.catalog_name()
            );
            assert!(
                diagnostic["message"]
                    .as_str()
                    .is_some_and(|m| !m.is_empty()),
                "{} refused without a message: {refusal:?}",
                tool.catalog_name()
            );
            assert!(
                refusal
                    .data
                    .as_ref()
                    .and_then(|data| data.get("code"))
                    .is_none(),
                "{} leaked a second code channel through data: {refusal:?}",
                tool.catalog_name()
            );
        }

        let current = call(
            ToolIdentity::View,
            serde_json::json!({"at": "main:Catalog.Bare"}),
        );
        assert!(current.ok, "{current:?}");
        assert!(current.rev.is_none(), "reads do not carry an apply fence");
        let planned = call(
            ToolIdentity::Apply,
            serde_json::json!({
                "at": "main:Catalog.Bare",
                "ops": [{"op": "props.set", "args": {"values": {"Comment": "x"}}}]
            }),
        );
        assert!(planned.ok, "{planned:?}");
        let bare_path = source.join("Catalogs/Bare.xml");
        let previous = std::fs::read_to_string(&bare_path).unwrap();
        std::fs::write(
            &bare_path,
            previous.replace("<Comment/>", "<Comment>external change</Comment>"),
        )
        .unwrap();
        let updated = call(
            ToolIdentity::View,
            serde_json::json!({"at": "main:Catalog.Bare"}),
        );
        let stale = call(
            ToolIdentity::Apply,
            serde_json::json!({"executionToken": planned.data.as_ref().unwrap()["executionToken"]}),
        );
        assert!(!stale.ok, "{stale:?}");
        assert_eq!(stale.diagnostics[0]["code"], "stale_revision");
        let stale_message = stale.diagnostics[0]["message"].as_str().unwrap();
        assert!(!stale_message.is_empty());
        assert!(updated.ok && updated.rev.is_none(), "{updated:?}");

        let bare_scope = call(
            ToolIdentity::Search,
            serde_json::json!({"query": "needle", "scope": "main:Catalog.Bare"}),
        );
        assert!(
            bare_scope.ok,
            "an admitted scope without a source subtree is not a failure: {bare_scope:?}"
        );
        assert_eq!(
            bare_scope
                .data
                .as_ref()
                .and_then(|data| data["matches"].as_array())
                .map(Vec::len),
            Some(0),
            "{bare_scope:?}"
        );
        assert!(bare_scope.diagnostics.is_empty(), "{bare_scope:?}");

        // The apply operation dictionary is part of every node answer: it is
        // computed from the one closed registry that also validates calls,
        // with the Run-dictionary honesty flag.
        let viewed_can = call(
            ToolIdentity::View,
            serde_json::json!({"at": "main:Catalog.Bare"}),
        );
        assert!(viewed_can.ok, "{viewed_can:?}");
        let can = viewed_can
            .data
            .as_ref()
            .and_then(|data| data["can"].as_array())
            .expect("the node answer carries its dictionary");
        let entry = |op: &str| {
            can.iter()
                .find(|entry| entry["op"] == op)
                .unwrap_or_else(|| panic!("missing `{op}` in {can:?}"))
                .clone()
        };
        assert_eq!(
            entry("props.set"),
            serde_json::json!({"op": "props.set", "args": "values"})
        );
        // Новый объект создаёт его родитель — корень конфигурации.
        assert!(
            can.iter().all(|entry| entry["op"] != "object.create"),
            "object.create belongs to the configuration root: {can:?}"
        );
        assert_eq!(
            entry("form.add"),
            serde_json::json!({"op": "form.add", "args": "items"})
        );
        assert!(
            can.iter().all(|entry| entry["op"] != "enumValue.add"),
            "a Catalog node must not advertise Enum-only operations: {can:?}"
        );
        // One operation's argument contract is read by its own address.
        let contract = call(
            ToolIdentity::View,
            serde_json::json!({"at": "main:Catalog.Bare.Can.attribute.add"}),
        );
        assert!(contract.ok, "{:?}", contract.diagnostics);
        assert_eq!(
            contract.at.as_deref(),
            Some("main:Catalog.Bare.Can.attribute.add")
        );
        let contract = contract.data.expect("operation node data");
        assert_eq!(contract["at"], "main:Catalog.Bare.Can.attribute.add");
        assert_eq!(contract["kind"], "Can");
        assert_eq!(contract["props"]["op"], "attribute.add");
        assert!(contract["props"].get("implemented").is_none());
        assert!(
            contract["props"]["contract"]["argsSchema"].is_object(),
            "the contract publishes its argument schema"
        );
        assert!(contract.get("can").is_none());
        // Неизвестная, неприменимая операция и `Can` без имени отказывают
        // `bad_value`, а `next` ведёт к словарю узла.
        for (at, code) in [
            ("main:Catalog.Bare.Can.frobnicate", "bad_value"),
            ("main:Catalog.Bare.Can.enumValue.add", "bad_value"),
            ("main:Catalog.Bare.Can", "bad_value"),
        ] {
            let refused = call(ToolIdentity::View, serde_json::json!({"at": at}));
            assert!(!refused.ok, "{at}");
            assert_eq!(refused.diagnostics[0]["code"], code, "{at}");
            assert_eq!(refused.at.as_deref(), Some(at));
            assert_eq!(
                refused.next.first().map(|entry| entry["args"].clone()),
                Some(serde_json::json!({"at": "main:Catalog.Bare"})),
                "{at}"
            );
        }

        // Адрес операции читает только `view`: остальные инструменты на
        // проводе отвечают одним отказом с переходом к нему.
        let can = "main:Catalog.Bare.Can.attribute.add";
        for (tool, arguments) in [
            (ToolIdentity::Check, serde_json::json!({"at": can})),
            (ToolIdentity::Resolve, serde_json::json!({"at": can})),
            (
                ToolIdentity::Search,
                serde_json::json!({"query": "needle", "scope": can}),
            ),
            (
                ToolIdentity::Apply,
                serde_json::json!({"at": can, "ops": [{"op": "props.set", "args": {"values": {}}}]}),
            ),
            (
                ToolIdentity::Apply,
                serde_json::json!({"at": "main:Catalog.Bare", "ops": [
                    {"op": "attribute.add", "args": {"at": can, "items": []}}
                ]}),
            ),
        ] {
            let name = tool.catalog_name();
            let refused = call(tool, arguments);
            assert!(!refused.ok, "{name}");
            assert_eq!(refused.diagnostics[0]["code"], "bad_value", "{name}");
            assert!(
                refused.diagnostics[0]["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("operation-dictionary address")),
                "{name}"
            );
            assert_eq!(
                refused.next.first().map(|entry| entry["args"].clone()),
                Some(serde_json::json!({"at": can})),
                "{name}"
            );
            assert_eq!(
                refused.next.first().map(|entry| entry["tool"].clone()),
                Some(serde_json::json!("unica.view")),
                "{name}"
            );
        }

        let wrong_args = call(
            ToolIdentity::Apply,
            serde_json::json!({
                "at": "main:Catalog.Bare",
                "ops": [{"op": "props.set", "args": {"props": {"Comment": "x"}}}],
            }),
        );
        assert!(
            wrong_args.diagnostics[0]["message"]
                .as_str()
                .is_some_and(|message| message.contains("`props.set` expects `values`")),
            "an argument refusal names the expected skeleton: {wrong_args:?}"
        );

        // Отказ по операции называет маршрут к словарю узла. Перечислять
        // операции в тексте не нужно — ответ узла уже несёт `can`, и агенту
        // нужен путь к нему. Без маршрута пробел словаря не всплывает: агент
        // повторяет вслепую.
        let dictionary_route = |result: &DomainResult| {
            result
                .next
                .iter()
                .find(|entry| entry["tool"] == "unica.view" && entry["args"]["at"].is_string())
                .cloned()
        };

        let unknown_operation = call(
            ToolIdentity::Apply,
            serde_json::json!({
                "at": "main:Catalog.Bare",
                "ops": [{"op": "object.levitate", "args": {"values": {}}}],
            }),
        );
        assert_eq!(
            unknown_operation.diagnostics[0]["code"], "unsupported_operation",
            "{unknown_operation:?}"
        );
        assert_eq!(
            unknown_operation.diagnostics[0]["outcome"], "goElsewhere",
            "исход зовёт взять альтернативу: {unknown_operation:?}"
        );
        assert_eq!(
            dictionary_route(&unknown_operation)
                .as_ref()
                .map(|entry| entry["args"]["at"].clone()),
            Some(serde_json::json!("main:Catalog.Bare")),
            "неизвестная операция называет словарь своего узла: {unknown_operation:?}"
        );

        let wrong_kind = call(
            ToolIdentity::Apply,
            serde_json::json!({
                "at": "main:Catalog.Bare",
                "ops": [{"op": "enumValue.add", "args": {"items": []}}],
            }),
        );
        assert!(
            dictionary_route(&wrong_kind).is_some(),
            "операция не того вида тоже называет словарь узла: {wrong_kind:?}"
        );
    }

    #[test]
    pub(crate) fn subsequent_daemon_invocation_after_same_root_kind_change_gets_new_actor_identity()
    {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        std::fs::create_dir_all(&source).unwrap();
        let project = workspace.path().join("v8project.yaml");
        let write_project = |kind: &str| {
            std::fs::write(
                &project,
                format!(
                    "format: DESIGNER\nsource-set:\n  - name: main\n    type: {kind}\n    path: src\n"
                ),
            )
            .unwrap();
        };
        write_project("CONFIGURATION");
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(
                crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(),
            ),
            Arc::new(TokioClock),
        );
        let request = InvocationRequest::new(
            ToolIdentity::View,
            serde_json::json!({"at": "main:Catalog.Items"}),
            std::fs::canonicalize(workspace.path())
                .unwrap()
                .to_string_lossy(),
            7_000,
        )
        .unwrap();
        let bind = || {
            bind_workspace_invocation(
                &request,
                &runtime.workspace_actors,
                Arc::clone(&runtime.deliveries),
                Arc::clone(&runtime.provider_hosts),
                Arc::clone(&runtime.runtime_resources),
                None,
                runtime.capture_response_deadline_for_test(),
            )
            .unwrap()
        };
        let configuration = bind();
        write_project("EXTENSION");
        let extension = bind();

        assert!(
            !Arc::ptr_eq(configuration.actor_for_test(), extension.actor_for_test()),
            "subsequent semantic kind change reused the live actor"
        );
        assert_ne!(
            configuration.workspace_identity_hash(),
            extension.workspace_identity_hash(),
            "durable daemon workspace identity ignored the changed source kind"
        );
    }

    fn source_selection_read_fixture() -> (tempfile::TempDir, std::path::PathBuf) {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        std::fs::create_dir_all(source.join("Catalogs")).unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects><Catalog>Items</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>Items</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        (workspace, source)
    }

    fn v13_runtime_for_borrowing_test() -> V5CanonicalInvocationRuntime {
        V5CanonicalInvocationRuntime::new(canonical_v13_service(), Arc::new(TokioClock))
    }

    fn borrowing_view_fixture(parent_state: &str) -> tempfile::TempDir {
        let (workspace, source) = source_selection_read_fixture();
        let extension = workspace.path().join("ext");
        std::fs::create_dir_all(extension.join("Catalogs")).unwrap();
        let config = std::fs::read_to_string(source.join("Configuration.xml")).unwrap();
        std::fs::write(extension.join("Configuration.xml"), config.replace("<Name>Store</Name>", "<ObjectBelonging>Adopted</ObjectBelonging><Name>Extension</Name><NamePrefix>Ext_</NamePrefix>")).unwrap();
        std::fs::write(extension.join("Catalogs/Items.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"><Properties><ObjectBelonging>Adopted</ObjectBelonging><Name>Items</Name><ExtendedConfigurationObject>aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa</ExtendedConfigurationObject></Properties><ChildObjects/></Catalog></MetaDataObject>"#).unwrap();
        let mut yaml = "format: DESIGNER\nsource-set:\n  - name: parent\n    type: CONFIGURATION\n    path: src\n  - name: ext\n    type: EXTENSION\n    path: ext\n".to_string();
        if parent_state == "duplicate" {
            std::fs::create_dir_all(workspace.path().join("copy/Catalogs")).unwrap();
            std::fs::copy(
                source.join("Configuration.xml"),
                workspace.path().join("copy/Configuration.xml"),
            )
            .unwrap();
            std::fs::copy(
                source.join("Catalogs/Items.xml"),
                workspace.path().join("copy/Catalogs/Items.xml"),
            )
            .unwrap();
            yaml.push_str("  - name: copy\n    type: CONFIGURATION\n    path: copy\n");
        } else if parent_state == "missing_second" {
            yaml.push_str("  - name: missing\n    type: CONFIGURATION\n    path: absent\n");
        } else if parent_state == "unregistered" {
            std::fs::write(
                source.join("Configuration.xml"),
                config.replace("<Catalog>Items</Catalog>", ""),
            )
            .unwrap();
        } else if parent_state == "different_uuid" {
            let path = source.join("Catalogs/Items.xml");
            let text = std::fs::read_to_string(&path).unwrap().replace(
                "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
                "cccccccc-cccc-4ccc-8ccc-cccccccccccc",
            );
            std::fs::write(path, text).unwrap();
        }
        std::fs::write(workspace.path().join("v8project.yaml"), yaml).unwrap();
        workspace
    }

    #[test]
    fn borrowing_view_resolves_registered_parent_and_preserves_unresolved_extension_facts() {
        for (case, expected) in [
            ("normal", "resolved"),
            ("duplicate", "ambiguous"),
            ("missing_second", "unavailable"),
            ("unregistered", "not_found"),
            ("different_uuid", "not_found"),
        ] {
            let workspace = borrowing_view_fixture(case);
            let runtime = v13_runtime_for_borrowing_test();
            let result = submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::View,
                serde_json::json!({"at":"ext:Catalog.Items"}),
            );
            assert!(result.ok, "{case}: {result:?}");
            let props = &result.data.as_ref().unwrap()["props"];
            assert_eq!(props["belonging"], "borrowed", "{case}");
            assert_eq!(props["parentStatus"], expected, "{case}: {result:?}");
            if expected == "resolved" {
                assert_eq!(props["extends"], "parent:Catalog.Items");
                let parent = submit_canonical(
                    &runtime,
                    workspace.path(),
                    ToolIdentity::View,
                    serde_json::json!({"at":props["extends"]}),
                );
                assert!(parent.ok, "parent must be readable: {parent:?}");
            } else {
                assert!(
                    props.get("extends").is_none(),
                    "{case}: never guess a parent"
                );
            }
        }
    }

    #[test]
    fn borrowing_view_bounds_override_props_for_metadata_and_specialized_readers() {
        for (kind, directory) in [("Catalog", "Catalogs"), ("CommonModule", "CommonModules")] {
            let workspace = borrowing_view_fixture("normal");
            let states = (0..300).map(|index| format!("<xr:PropertyState><xr:Property>Property{index:04}</xr:Property><xr:State>Notify</xr:State></xr:PropertyState>")).collect::<String>();
            for source in ["src", "ext"] {
                let root = workspace.path().join(source);
                let owner = root.join("Configuration.xml");
                let config = std::fs::read_to_string(&owner)
                    .unwrap()
                    .replace("Catalog>", &format!("{kind}>"));
                std::fs::write(owner, config).unwrap();
                let original = root.join("Catalogs/Items.xml");
                let mut xml = std::fs::read_to_string(&original)
                    .unwrap()
                    .replace("Catalog ", &format!("{kind} "))
                    .replace("</Catalog>", &format!("</{kind}>"));
                if kind == "CommonModule" {
                    std::fs::create_dir_all(root.join("CommonModules/Items/Ext")).unwrap();
                    std::fs::write(root.join("CommonModules/Items/Ext/Module.bsl"), "").unwrap();
                    let privileged = if source == "src" {
                        "<Privileged>false</Privileged>"
                    } else {
                        ""
                    };
                    xml = xml.replace("</Properties>", &format!("<Global>false</Global><ClientManagedApplication>false</ClientManagedApplication><Server>true</Server><ExternalConnection>false</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>false</ServerCall>{privileged}<ReturnValuesReuse>DontUse</ReturnValuesReuse></Properties>"));
                }
                if source == "ext" {
                    xml = xml
                        .replace(
                            "<MetaDataObject ",
                            "<MetaDataObject xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" ",
                        )
                        .replace(
                            "<Properties>",
                            &format!("<InternalInfo>{states}</InternalInfo><Properties>"),
                        );
                }
                std::fs::create_dir_all(root.join(directory)).unwrap();
                std::fs::write(root.join(directory).join("Items.xml"), xml).unwrap();
            }
            let runtime = v13_runtime_for_borrowing_test();
            let result = submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::View,
                serde_json::json!({"at":format!("ext:{kind}.Items")}),
            );
            assert!(result.ok, "{kind}: {result:?}");
            let props = &result.data.as_ref().unwrap()["props"];
            assert_eq!(props["overridesCount"], 300, "{kind}: {result:?}");
            assert_eq!(props["overridesComplete"], false, "{kind}: {result:?}");
            assert!(
                props.get("overrides").is_none(),
                "oversized scalar must not escape: {kind}"
            );
            assert_eq!(
                props["extends"],
                format!("parent:{kind}.Items"),
                "{result:?}"
            );
            if kind == "CommonModule" {
                assert_eq!(props["commonModule"]["privileged"], serde_json::Value::Null);
            }
        }
    }

    #[test]
    fn canonical_object_borrow_previews_publishes_and_rejects_changed_parent_revision() {
        let workspace = borrowing_view_fixture("normal");
        let owner = workspace.path().join("ext/Configuration.xml");
        let config = std::fs::read_to_string(&owner).unwrap();
        std::fs::write(&owner, config.replace("<Catalog>Items</Catalog>", "")).unwrap();
        std::fs::remove_file(workspace.path().join("ext/Catalogs/Items.xml")).unwrap();
        let runtime = v13_runtime_for_borrowing_test();
        let skill = include_str!("../../../../../plugins/unica/skills/cfe-borrow/SKILL.md");
        let parse_examples = |source: &str| {
            source
                .split("```json")
                .skip(1)
                .map(|block| {
                    serde_json::from_str::<serde_json::Value>(block.split("```").next().unwrap())
                        .unwrap()
                })
                .collect::<Vec<_>>()
        };
        let examples = parse_examples(skill);
        let windows_skill = skill.replace("\r\n", "\n").replace('\n', "\r\n");
        assert_eq!(examples, parse_examples(&windows_skill));
        assert_eq!(
            examples.len(),
            3,
            "the skill must keep its three public examples"
        );
        let preview_args = examples[0]["params"]["arguments"].clone();
        let preview = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            preview_args.clone(),
        );
        assert!(preview.ok, "{preview:?}");
        assert!(!workspace.path().join("ext/Catalogs/Items.xml").exists());
        let mut apply_args =
            serde_json::json!({"executionToken": preview.data.as_ref().unwrap()["executionToken"]});
        let parent = workspace.path().join("src/Catalogs/Items.xml");
        let text = std::fs::read_to_string(&parent).unwrap();
        std::fs::write(
            &parent,
            text.replace(
                "<Name>Items</Name>",
                "<Name>Items</Name><Comment>changed</Comment>",
            ),
        )
        .unwrap();
        let stale = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            apply_args.clone(),
        );
        assert!(!stale.ok, "parent change must invalidate preview");
        assert!(
            stale
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic["code"] == "stale_revision"),
            "{stale:?}"
        );
        assert!(!workspace.path().join("ext/Catalogs/Items.xml").exists());
        let preview = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            preview_args.clone(),
        );
        assert!(preview.ok, "{preview:?}");
        apply_args["executionToken"] = preview.data.as_ref().unwrap()["executionToken"].clone();
        let applied = submit_canonical(&runtime, workspace.path(), ToolIdentity::Apply, apply_args);
        assert!(applied.ok, "{applied:?}");
        let read = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::View,
            examples[1]["params"]["arguments"].clone(),
        );
        assert!(read.ok, "{read:?}");
        assert_eq!(
            read.data.as_ref().unwrap()["props"]["extends"],
            "parent:Catalog.Items"
        );
        let repeat = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            preview_args,
        );
        assert!(repeat.ok, "{repeat:?}");
        assert!(
            repeat.changed.is_empty(),
            "unchanged repeat is not an update"
        );

        // The form example borrows a managed form into the borrowed owner.
        let forms = workspace.path().join("src/Catalogs/Items/Forms");
        std::fs::create_dir_all(&forms).unwrap();
        std::fs::write(
            forms.join("ItemForm.xml"),
            std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
                    "../../tests/fixtures/platform_8_3_27/cfe_borrow/parent-catalog-form.xml",
                ),
            )
            .unwrap()
            .replace("<Name>CorpusForm</Name>", "<Name>ItemForm</Name>"),
        )
        .unwrap();
        let text = std::fs::read_to_string(&parent).unwrap();
        std::fs::write(
            &parent,
            text.replace(
                "<ChildObjects/>",
                "<ChildObjects><Form>ItemForm</Form></ChildObjects>",
            ),
        )
        .unwrap();
        let form_args = examples[2]["params"]["arguments"].clone();
        let form_preview =
            submit_canonical(&runtime, workspace.path(), ToolIdentity::Apply, form_args);
        assert!(form_preview.ok, "{form_preview:?}");
        let borrowed = workspace.path().join("ext/Catalogs/Items/Forms/ItemForm");
        assert!(!borrowed.exists(), "form preview must not write");
        let form_applied = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            serde_json::json!({"executionToken": form_preview.data.as_ref().unwrap()["executionToken"]}),
        );
        assert!(form_applied.ok, "{form_applied:?}");
        assert!(borrowed.join("Ext/Form.xml").exists());
        let form = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::View,
            serde_json::json!({"at":"ext:Catalog.Items.Form.ItemForm"}),
        );
        assert!(form.ok, "{form:?}");
        assert!(
            form.data.as_ref().unwrap()["props"]
                .get("content")
                .is_none(),
            "{form:?}"
        );
    }

    #[test]
    fn borrowing_view_publishes_saved_parent_facts_and_new_read_rechecks_parent() {
        let workspace = borrowing_view_fixture("normal");
        let runtime = v13_runtime_for_borrowing_test();
        let request = InvocationRequest::new(
            ToolIdentity::View,
            serde_json::json!({"at":"ext:Catalog.Items"}),
            std::fs::canonicalize(workspace.path())
                .unwrap()
                .to_string_lossy(),
            7_000,
        )
        .unwrap();
        let invocation = bind_workspace_invocation(
            &request,
            &runtime.workspace_actors,
            Arc::clone(&runtime.deliveries),
            Arc::clone(&runtime.provider_hosts),
            Arc::clone(&runtime.runtime_resources),
            None,
            runtime.capture_response_deadline_for_test(),
        )
        .unwrap();
        let cancellation = CancellationToken::new();
        let execution = invocation.begin_execution(&cancellation).unwrap();
        let result = canonical_v13_service()
            .execute(&execution, cancellation.clone())
            .unwrap();
        assert_eq!(
            result.data.as_ref().unwrap()["props"]["parentStatus"],
            "resolved"
        );
        let parent = workspace.path().join("src/Catalogs/Items.xml");
        let text = std::fs::read_to_string(&parent).unwrap().replace(
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
            "cccccccc-cccc-4ccc-8ccc-cccccccccccc",
        );
        std::fs::write(parent, text).unwrap();
        let published = execution
            .publish(Ok(result), &cancellation)
            .unwrap()
            .unwrap();
        assert!(published.ok, "{published:?}");
        assert_eq!(
            published.data.as_ref().unwrap()["props"]["parentStatus"],
            "resolved"
        );
        assert_eq!(
            published.data.as_ref().unwrap()["props"]["extends"],
            "parent:Catalog.Items"
        );
        let current = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::View,
            serde_json::json!({"at":"ext:Catalog.Items"}),
        );
        assert!(current.ok, "{current:?}");
        let props = &current.data.as_ref().unwrap()["props"];
        assert_eq!(props["parentStatus"], "not_found", "{current:?}");
        assert!(
            props.get("extends").is_none(),
            "a new read must not reuse the old parent proof"
        );
    }

    #[test]
    pub(crate) fn view_find_admitted_snapshot_may_finish_after_map_change() {
        let (workspace, source) = source_selection_read_fixture();
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(
                crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(),
            ),
            Arc::new(TokioClock),
        );
        let request = |tool, arguments| {
            InvocationRequest::new(
                tool,
                arguments,
                std::fs::canonicalize(workspace.path())
                    .unwrap()
                    .to_string_lossy(),
                7_000,
            )
            .unwrap()
        };
        let bind = |request: &InvocationRequest| {
            bind_workspace_invocation(
                request,
                &runtime.workspace_actors,
                Arc::clone(&runtime.deliveries),
                Arc::clone(&runtime.provider_hosts),
                Arc::clone(&runtime.runtime_resources),
                None,
                runtime.capture_response_deadline_for_test(),
            )
            .unwrap()
        };
        let view_request = request(
            ToolIdentity::View,
            serde_json::json!({"at": "main:Catalog.Items"}),
        );
        let find_request = request(
            ToolIdentity::Resolve,
            serde_json::json!({"path": "src/Catalogs/Items.xml"}),
        );
        let view = bind(&view_request);
        let find = bind(&find_request);
        assert!(Arc::ptr_eq(view.actor_for_test(), find.actor_for_test()));
        let admitted_actor = Arc::clone(view.actor_for_test());
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: EXTENSION\n    path: src\n",
        )
        .unwrap();

        let service =
            crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default();
        for invocation in [view, find] {
            let cancellation = CancellationToken::new();
            let execution = invocation.begin_execution(&cancellation).unwrap();
            let result = service
                .execute(&execution, cancellation.clone())
                .expect("already-admitted retained read execution");
            let published = execution
                .publish(Ok(result), &cancellation)
                .unwrap()
                .unwrap();
            assert!(published.ok, "retained read publication must succeed");
        }

        let subsequent = bind(&view_request);
        assert!(
            !Arc::ptr_eq(&admitted_actor, subsequent.actor_for_test()),
            "a subsequent invocation ignored the changed map"
        );
        assert_eq!(
            subsequent.provider_root_for_test().source_kind(),
            SourceSetKind::Extension
        );
        assert!(source.join("Catalogs/Items.xml").exists());
    }

    #[test]
    pub(crate) fn semantically_equivalent_map_edit_reuses_actor_identity() {
        let (workspace, _) = source_selection_read_fixture();
        let dependency = workspace.path().join("dep");
        std::fs::create_dir_all(&dependency).unwrap();
        std::fs::write(
            dependency.join("Configuration.xml"),
            "<MetaDataObject><Configuration/></MetaDataObject>",
        )
        .unwrap();
        let project = workspace.path().join("v8project.yaml");
        std::fs::write(
            &project,
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: main\n    type: CONFIGURATION\n    path: src\n",
                "  - name: dep\n    type: CONFIGURATION\n    path: dep\n",
            ),
        )
        .unwrap();
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(
                crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(),
            ),
            Arc::new(TokioClock),
        );
        let request = InvocationRequest::new(
            ToolIdentity::View,
            serde_json::json!({"at": "main:Catalog.Items"}),
            std::fs::canonicalize(workspace.path())
                .unwrap()
                .to_string_lossy(),
            7_000,
        )
        .unwrap();
        let bind = || {
            bind_workspace_invocation(
                &request,
                &runtime.workspace_actors,
                Arc::clone(&runtime.deliveries),
                Arc::clone(&runtime.provider_hosts),
                Arc::clone(&runtime.runtime_resources),
                None,
                runtime.capture_response_deadline_for_test(),
            )
            .unwrap()
        };
        let first = bind();
        std::fs::write(
            &project,
            concat!(
                "# order and comments are semantically irrelevant\n",
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: dep\n    type: CONFIGURATION\n    path: dep\n",
                "  - name: main\n    type: CONFIGURATION\n    path: src\n",
            ),
        )
        .unwrap();
        let second = bind();

        assert!(Arc::ptr_eq(first.actor_for_test(), second.actor_for_test()));
        assert_eq!(
            first.workspace_identity_hash(),
            second.workspace_identity_hash(),
            "comments/order-only edit changed actor state scope"
        );
    }

    #[test]
    fn unpublished_apply_execution_rejects_success_changed_and_revision_without_actor_publication()
    {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(
                crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(),
            ),
            Arc::new(TokioClock),
        );
        let request = InvocationRequest::new(
            ToolIdentity::Apply,
            serde_json::json!({
                "at": "main:Configuration",
                "ops": [{"op": "object.create", "args": {}}],
            }),
            std::fs::canonicalize(workspace.path())
                .unwrap()
                .to_string_lossy(),
            7_000,
        )
        .unwrap();
        let bind = || {
            bind_workspace_invocation(
                &request,
                &runtime.workspace_actors,
                Arc::clone(&runtime.deliveries),
                Arc::clone(&runtime.provider_hosts),
                Arc::clone(&runtime.runtime_resources),
                None,
                runtime.capture_response_deadline_for_test(),
            )
            .unwrap()
        };
        let mut changed = failed_domain_result("unpublished changed Apply");
        changed.changed.push(serde_json::json!({"path": "src"}));
        let mut revision = failed_domain_result("unpublished revision-bearing Apply");
        revision.rev = Some("unpublished-revision".to_string());
        let cases = [
            ("success", DomainResult::success("unpublished Apply")),
            ("changed", changed),
            ("revision", revision),
        ];
        let mut accepted = Vec::new();
        for (label, result) in cases {
            let cancellation = CancellationToken::new();
            let execution = bind().begin_execution(&cancellation).unwrap();
            if execution.publish(Ok(result), &cancellation).is_ok() {
                accepted.push(label);
            }
        }

        assert!(
            accepted.is_empty(),
            "unpublished Apply accepted result classes requiring actor publication: {accepted:?}"
        );
    }

    #[test]
    fn non_apply_execution_rejects_prepared_apply_before_actor_publication() {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        std::fs::create_dir_all(source.join("Documents")).unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects><Document>Order</Document></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let descriptor = source.join("Documents/Order.xml");
        let preimage = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>Order</Name><Synonym/><Comment/></Properties><ChildObjects/></Document></MetaDataObject>"#;
        std::fs::write(&descriptor, preimage).unwrap();
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(
                crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(),
            ),
            Arc::new(TokioClock),
        );
        let workspace_hint = std::fs::canonicalize(workspace.path()).unwrap();
        let view = InvocationRequest::new(
            ToolIdentity::View,
            serde_json::json!({"at": "main:Document.Order"}),
            workspace_hint.to_string_lossy(),
            7_000,
        )
        .unwrap();
        let cancellation = CancellationToken::new();
        let execution = bind_workspace_invocation(
            &view,
            &runtime.workspace_actors,
            Arc::clone(&runtime.deliveries),
            Arc::clone(&runtime.provider_hosts),
            Arc::clone(&runtime.runtime_resources),
            None,
            runtime.capture_response_deadline_for_test(),
        )
        .unwrap()
        .begin_execution(&cancellation)
        .unwrap();
        let apply_arguments = serde_json::json!({
            "at": "main:Document.Order",
            "ops": [{"op": "props.set", "args": {"values": {"Comment": "forbidden"}}}],
            "dryRun": false,
            "ifRev": "unica-source-sha256-v1:1:ignored-before-admission",
        });
        let request = crate::application::v13::apply::parse_request(
            apply_arguments.as_object().unwrap(),
            &["main"],
        )
        .unwrap();
        let binding = execution.provider_root_for_test().clone();
        let admission = execution
            .actor_for_test()
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let (staged, effects) =
            crate::infrastructure::native_operations::apply_families::plan_hidden_v13_apply(
                &request, &binding, &admission,
            )
            .unwrap();
        let prepared = admission.prepare_with_effects(staged, effects).unwrap();

        let error = execution.publish_prepared_apply(prepared).unwrap_err();

        assert_eq!(
            error.kind(),
            unica_workspace::actor::ApplyPublicationErrorKind::Invariant
        );
        assert_eq!(std::fs::read_to_string(descriptor).unwrap(), preimage);
    }

    /// The canonical removal reports its cache impact in the same result:
    /// preview and publication both carry the cache mode, the event and the
    /// invalidation lists, so no second call is needed to learn what moved.
    #[test]
    fn canonical_object_remove_reports_typed_cache_impact_in_preview_and_publication() {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        std::fs::create_dir_all(source.join("Documents")).unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects><Document>Order</Document></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let descriptor = source.join("Documents/Order.xml");
        std::fs::write(
            &descriptor,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20"><Document uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>Order</Name><Synonym/><Comment/></Properties><ChildObjects/></Document></MetaDataObject>"#,
        )
        .unwrap();
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(
                crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(),
            ),
            Arc::new(TokioClock),
        );
        let workspace_hint = std::fs::canonicalize(workspace.path()).unwrap();
        let call = |token: Option<&str>| {
            let arguments = match token {
                None => {
                    serde_json::json!({"at": "main:Document.Order", "ops": [{"op": "object.remove", "args": {}}]})
                }
                Some(token) => serde_json::json!({"executionToken": token}),
            };
            let request = InvocationRequest::new(
                ToolIdentity::Apply,
                arguments,
                workspace_hint.to_string_lossy(),
                7_000,
            )
            .unwrap();
            let result = direct_v5(&runtime, request).unwrap();
            assert!(result.ok, "object.remove failed: {result:?}");
            result
        };
        let assert_cache_impact = |result: &DomainResult, mode: &str| {
            let data = result.data.as_ref().unwrap();
            assert_eq!(data["mode"], mode);
            assert!(data["effects"].as_u64().unwrap() >= 1);
            let cache = &data["cache"];
            assert!(cache["mode"].is_string(), "{cache}");
            assert!(cache["events"]
                .as_array()
                .unwrap()
                .iter()
                .any(|event| event == "MetadataChanged"));
            assert!(!cache["invalidated"].as_array().unwrap().is_empty());
            assert!(result.changed.iter().any(|change| {
                change.get("at").and_then(serde_json::Value::as_str) == Some("main:Document.Order")
            }));
        };
        let preview = call(None);
        assert!(
            descriptor.is_file(),
            "preview must not remove the descriptor"
        );
        assert_cache_impact(&preview, "preview");
        let published = call(Some(
            preview.data.as_ref().unwrap()["executionToken"]
                .as_str()
                .unwrap(),
        ));
        assert!(!descriptor.exists(), "publication removes the descriptor");
        assert_cache_impact(&published, "published");
    }

    #[test]
    fn saved_apply_token_survives_eight_other_workspace_profiles_and_replays_exactly() {
        let workspace = subsystem_picture_workspace("", true);
        let source = workspace.path().join("src");
        let before = unica_workspace::test_support::tree_snapshot(&source);
        let runtime = bootstrap_runtime();
        let arguments = serde_json::json!({
            "at": "main:Subsystem.Sales",
            "ops": [{"op": "props.set", "args": {"values": {"Comment": "retained exact plan"}}}]
        });
        let preview = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            arguments.clone(),
        );
        assert!(preview.ok, "{preview:?}");
        let token = preview.data.as_ref().unwrap()["executionToken"].clone();
        let others = (0..8)
            .map(|_| subsystem_picture_workspace("", true))
            .collect::<Vec<_>>();
        let admit_others = || {
            for other in &others {
                let plan = submit_canonical(
                    &runtime,
                    other.path(),
                    ToolIdentity::Apply,
                    arguments.clone(),
                );
                assert!(plan.ok, "{plan:?}");
            }
        };
        // These are complete public plan calls, not held actor leases. Their
        // eight distinct profiles cross the production warm cache's LRU edge.
        admit_others();
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&source),
            before
        );
        let executed = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            serde_json::json!({"executionToken": token}),
        );
        assert!(
            executed.ok,
            "an issued plan was lost after unrelated profiles: {executed:?}"
        );
        let published = unica_workspace::test_support::tree_snapshot(&source);
        assert!(std::fs::read_to_string(source.join("Subsystems/Sales.xml"))
            .unwrap()
            .contains("retained exact plan"));

        admit_others();
        let replayed = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            serde_json::json!({"executionToken": token}),
        );
        assert_eq!(
            serde_json::to_value(&replayed).unwrap(),
            serde_json::to_value(&executed).unwrap()
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&source),
            published,
            "replay changed the published source"
        );
    }

    #[test]
    fn saved_apply_token_survives_temporary_named_root_validation_failure() {
        let workspace = subsystem_picture_workspace("", true);
        let source = workspace.path().join("src");
        let before = unica_workspace::test_support::tree_snapshot(&source);
        let runtime = bootstrap_runtime();
        let preview = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            serde_json::json!({
                "at": "main:Subsystem.Sales",
                "ops": [{"op": "props.set", "args": {"values": {"Comment": "retained after temporary root failure"}}}]
            }),
        );
        assert!(preview.ok, "{preview:?}");
        let token = preview.data.as_ref().unwrap()["executionToken"].clone();

        runtime
            .workspace_actors
            .fail_next_named_root_validation_for_test(std::io::ErrorKind::PermissionDenied);
        let unavailable = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            serde_json::json!({"executionToken": token}),
        );
        assert!(!unavailable.ok, "{unavailable:?}");
        assert_eq!(
            unavailable.diagnostics[0]["code"], "provider_unavailable",
            "временная ошибка открытия корня не должна превращать токен в неизвестный: {unavailable:?}"
        );
        assert_eq!(
            unavailable.diagnostics[0]["detailCode"],
            "source_temporarily_unavailable"
        );
        assert_eq!(unavailable.diagnostics[0]["outcome"], "retry");
        assert_eq!(unavailable.next[0]["tool"], "unica.apply");
        assert_eq!(unavailable.next[0]["args"]["executionToken"], token);
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&source),
            before,
            "отказ допуска не должен публиковать сохранённый план"
        );

        let executed = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            serde_json::json!({"executionToken": token}),
        );
        assert!(
            executed.ok,
            "повтор после временной ошибки должен исполнить прежний токен: {executed:?}"
        );
        assert!(std::fs::read_to_string(source.join("Subsystems/Sales.xml"))
            .unwrap()
            .contains("retained after temporary root failure"));
    }

    fn saved_apply_survives_retention_age(age: Duration) {
        use unica_workspace::actor::with_actor_retention_time_for_test;

        let workspace = subsystem_picture_workspace("", true);
        let source = workspace.path().join("src");
        let before = unica_workspace::test_support::tree_snapshot(&source);
        let runtime = bootstrap_runtime();
        let started = Instant::now();
        let preview = with_actor_retention_time_for_test(started, || {
            submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::Apply,
                serde_json::json!({"at": "main:Subsystem.Sales", "ops": [{"op": "props.set", "args": {"values": {"Comment": "retained after former time limits"}}}]}),
            )
        });
        assert!(preview.ok, "{preview:?}");
        let token = preview.data.as_ref().unwrap()["executionToken"].clone();
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&source),
            before
        );
        let executed = with_actor_retention_time_for_test(started + age, || {
            submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::Apply,
                serde_json::json!({"executionToken": token}),
            )
        });
        assert!(
            executed.ok,
            "accepted plan was lost after {age:?}: {executed:?}"
        );
        let published = unica_workspace::test_support::tree_snapshot(&source);
        assert!(std::fs::read_to_string(source.join("Subsystems/Sales.xml"))
            .unwrap()
            .contains("retained after former time limits"));
        let replayed =
            with_actor_retention_time_for_test(started + age + Duration::from_secs(601), || {
                submit_canonical(
                    &runtime,
                    workspace.path(),
                    ToolIdentity::Apply,
                    serde_json::json!({"executionToken": token}),
                )
            });
        assert_eq!(
            serde_json::to_value(&replayed).unwrap(),
            serde_json::to_value(&executed).unwrap()
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&source),
            published
        );
    }

    #[test]
    fn saved_apply_plan_survives_former_three_hundred_second_expiry() {
        saved_apply_survives_retention_age(Duration::from_secs(301));
    }

    #[test]
    fn saved_apply_plan_survives_former_six_hundred_second_actor_idle_window() {
        saved_apply_survives_retention_age(Duration::from_secs(601));
    }

    #[test]
    fn saved_apply_refusal_replays_after_former_actor_idle_window_without_replanning() {
        use unica_workspace::actor::with_actor_retention_time_for_test;
        let workspace = subsystem_picture_workspace("", true);
        let source = workspace.path().join("src");
        let runtime = bootstrap_runtime();
        let started = Instant::now();
        let preview = with_actor_retention_time_for_test(started, || {
            submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::Apply,
                serde_json::json!({"at": "main:Subsystem.Sales", "ops": [{"op": "props.set", "args": {"values": {"Comment": "saved refusal"}}}]}),
            )
        });
        assert!(preview.ok, "{preview:?}");
        let token = preview.data.as_ref().unwrap()["executionToken"].clone();
        let descriptor = source.join("Subsystems/Sales.xml");
        let mut changed = std::fs::read(&descriptor).unwrap();
        changed.extend_from_slice(b"\n");
        std::fs::write(&descriptor, changed).unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&source);
        let refusal = with_actor_retention_time_for_test(started + Duration::from_secs(1), || {
            submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::Apply,
                serde_json::json!({"executionToken": token}),
            )
        });
        assert!(
            !refusal.ok,
            "changed inputs unexpectedly published: {refusal:?}"
        );
        assert_eq!(refusal.diagnostics[0]["code"], "stale_revision");
        let replay = with_actor_retention_time_for_test(started + Duration::from_secs(602), || {
            submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::Apply,
                serde_json::json!({"executionToken": token}),
            )
        });
        assert_eq!(
            serde_json::to_value(&replay).unwrap(),
            serde_json::to_value(&refusal).unwrap()
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&source),
            before
        );
    }

    #[derive(Clone, Copy)]
    enum CleanupBeforePromise {
        Idle,
        Release,
    }

    fn cleanup_before_live_invocation_creates_promise(
        runtime: &V5CanonicalInvocationRuntime,
        started: Instant,
        cleanup: CleanupBeforePromise,
    ) {
        use unica_workspace::actor::with_actor_retention_time_for_test;
        match cleanup {
            CleanupBeforePromise::Idle => {
                with_actor_retention_time_for_test(started + Duration::from_secs(601), || {
                    runtime.workspace_actors.evict_idle_warm_actors().unwrap();
                });
            }
            CleanupBeforePromise::Release => {
                runtime.workspace_actors.release_warm_actors().unwrap()
            }
        }
    }

    fn saved_apply_token_survives_cleanup_before_live_invocation_creates_plan(
        cleanup: CleanupBeforePromise,
    ) {
        use unica_workspace::actor::with_actor_retention_time_for_test;
        let workspace = subsystem_picture_workspace("", true);
        let source = workspace.path().join("src");
        let before = unica_workspace::test_support::tree_snapshot(&source);
        let runtime = bootstrap_runtime();
        let started = Instant::now();
        let request = InvocationRequest::new(
            ToolIdentity::Apply,
            serde_json::json!({"at": "main:Subsystem.Sales", "ops": [{"op": "props.set", "args": {"values": {"Comment": "plan created after cleanup"}}}]}),
            workspace.path().to_string_lossy(),
            7_000,
        ).unwrap();
        // Admission holds the actual actor while its promise tables are still
        // empty. Cleanup occurs before the real canonical handler saves a plan.
        let bound = with_actor_retention_time_for_test(started, || runtime.bind(request)).unwrap();
        let observed = match &bound.0 {
            ActorBoundCanonicalInvocationState::Workspace { invocation, .. } => {
                Arc::downgrade(invocation.actor_for_test())
            }
            _ => panic!("apply must bind its workspace actor"),
        };
        cleanup_before_live_invocation_creates_promise(&runtime, started, cleanup);
        let preview = bound
            .prepare()
            .unwrap()
            .execute(CancellationToken::new())
            .unwrap();
        assert!(preview.ok, "{preview:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&source),
            before,
            "preview must not write"
        );
        let token = preview.data.as_ref().unwrap()["executionToken"].clone();
        // execute consumes and drops the admitted invocation above. No test
        // Arc keeps that actor alive when the public token request rebinds it.
        let executed = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            serde_json::json!({"executionToken": token}),
        );
        assert!(
            executed.ok,
            "cleanup lost a plan created by a live invocation: {executed:?}"
        );
        assert!(
            observed.upgrade().is_some(),
            "the promised plan retains its original actor"
        );
        let published = unica_workspace::test_support::tree_snapshot(&source);
        assert!(std::fs::read_to_string(source.join("Subsystems/Sales.xml"))
            .unwrap()
            .contains("plan created after cleanup"));
        let replay = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            serde_json::json!({"executionToken": token}),
        );
        assert_eq!(
            serde_json::to_value(&replay).unwrap(),
            serde_json::to_value(&executed).unwrap()
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&source),
            published,
            "exact replay must not write again"
        );
    }

    #[test]
    fn saved_apply_token_survives_idle_cleanup_before_live_invocation_creates_plan() {
        saved_apply_token_survives_cleanup_before_live_invocation_creates_plan(
            CleanupBeforePromise::Idle,
        );
    }

    #[test]
    fn saved_apply_token_survives_warm_release_before_live_invocation_creates_plan() {
        saved_apply_token_survives_cleanup_before_live_invocation_creates_plan(
            CleanupBeforePromise::Release,
        );
    }

    fn index_work_survives_last_call_and_actor_idle_window(
        completed_leased: bool,
        cleanup_before_creation: Option<CleanupBeforePromise>,
    ) {
        use unica_workspace::actor::with_actor_retention_time_for_test;

        struct ReleaseProducer(mpsc::Sender<()>);
        impl Drop for ReleaseProducer {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let workspace = subsystem_picture_workspace("", true);
        let runtime = bootstrap_runtime();
        let started = Instant::now();
        let request = || {
            InvocationRequest::new(
                ToolIdentity::View,
                serde_json::json!({"at": "main:Subsystem.Sales"}),
                workspace.path().to_string_lossy(),
                7_000,
            )
            .unwrap()
        };
        let bound =
            with_actor_retention_time_for_test(started, || runtime.bind(request())).unwrap();
        let ActorBoundCanonicalInvocationState::Workspace { invocation, .. } = &bound.0 else {
            panic!("initial call did not bind its actor");
        };
        let actor = invocation.actor_for_test();
        let observed = Arc::downgrade(actor);
        if let Some(cleanup) = cleanup_before_creation {
            cleanup_before_live_invocation_creates_promise(&runtime, started, cleanup);
        }
        let binding = actor
            .bind_provider_root("main", &workspace.path().join("src"))
            .unwrap();
        let producers = Arc::new(AtomicUsize::new(0));
        let first_count = Arc::clone(&producers);
        let (entered, entered_wait) = mpsc::channel();
        let (exited, exited_wait) = mpsc::channel();
        let (release, release_wait) = mpsc::channel();
        let release = ReleaseProducer(release);
        let (first_key, first_lease) = actor
            .join_index_work(
                &binding,
                "retention-generation",
                "rlm",
                "bsl-1",
                move |_| {
                    first_count.fetch_add(1, Ordering::SeqCst);
                    entered.send(()).unwrap();
                    release_wait.recv().unwrap();
                    exited.send(()).unwrap();
                    Ok(())
                },
            )
            .unwrap();
        entered_wait.recv_timeout(Duration::from_secs(2)).unwrap();
        let retained_lease = if completed_leased {
            release.0.send(()).unwrap();
            assert!(matches!(
                first_lease.wait_timeout(Duration::from_secs(2)),
                unica_work_control::shared_work::SharedWorkSnapshot::Ready(_)
            ));
            Some(first_lease)
        } else {
            drop(first_lease);
            None
        };
        drop(bound);
        let second = with_actor_retention_time_for_test(started + Duration::from_secs(601), || {
            runtime.bind(request())
        })
        .unwrap();
        let ActorBoundCanonicalInvocationState::Workspace { invocation, .. } = &second.0 else {
            panic!("later call did not bind its actor");
        };
        let second_actor = invocation.actor_for_test();
        let same_actor = observed
            .upgrade()
            .is_some_and(|first| Arc::ptr_eq(&first, second_actor));
        let binding = second_actor
            .bind_provider_root("main", &workspace.path().join("src"))
            .unwrap();
        let second_count = Arc::clone(&producers);
        let (second_key, second_lease) = second_actor
            .join_index_work(
                &binding,
                "retention-generation",
                "rlm",
                "bsl-1",
                move |_| {
                    second_count.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                },
            )
            .unwrap();
        let started_duplicate = second_lease.started_here();
        drop(release);
        exited_wait.recv_timeout(Duration::from_secs(2)).unwrap();
        let outcome = second_lease.wait_timeout(Duration::from_secs(2));
        assert!(matches!(
            outcome,
            unica_work_control::shared_work::SharedWorkSnapshot::Ready(_)
        ));
        assert_eq!(
            first_key, second_key,
            "the same exact generation changed key"
        );
        assert!(
            !started_duplicate,
            "idle cleanup lost a live shared-work registry and started another producer"
        );
        assert!(same_actor, "shared-work ownership did not retain its actor");
        assert_eq!(producers.load(Ordering::SeqCst), 1);
        drop(retained_lease);
    }

    #[test]
    fn running_unowned_index_work_survives_last_call_and_actor_idle_window() {
        index_work_survives_last_call_and_actor_idle_window(false, None);
    }

    #[test]
    fn completed_leased_index_work_survives_last_call_and_actor_idle_window() {
        index_work_survives_last_call_and_actor_idle_window(true, None);
    }

    #[test]
    fn running_index_work_survives_idle_cleanup_before_live_invocation_creates_work() {
        index_work_survives_last_call_and_actor_idle_window(
            false,
            Some(CleanupBeforePromise::Idle),
        );
    }

    #[test]
    fn running_index_work_survives_warm_release_before_live_invocation_creates_work() {
        index_work_survives_last_call_and_actor_idle_window(
            false,
            Some(CleanupBeforePromise::Release),
        );
    }

    #[test]
    fn saved_apply_more_than_256_plans_keep_earlier_tokens_without_writes() {
        let workspace = subsystem_picture_workspace("", true);
        let source = workspace.path().join("src");
        let before = unica_workspace::test_support::tree_snapshot(&source);
        let runtime = bootstrap_runtime();
        let arguments = serde_json::json!({
            "at": "main:Subsystem.Sales",
            "ops": [{"op": "props.set", "args": {"values": {"Comment": "capacity probe"}}}]
        });
        let mut first_token = None;
        for _ in 0..256 {
            let plan = submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::Apply,
                arguments.clone(),
            );
            assert!(plan.ok, "{plan:?}");
            first_token
                .get_or_insert_with(|| plan.data.as_ref().unwrap()["executionToken"].clone());
        }
        let additional =
            submit_canonical(&runtime, workspace.path(), ToolIdentity::Apply, arguments);
        assert!(additional.ok, "{additional:?}");
        assert!(additional
            .data
            .as_ref()
            .and_then(|data| data.get("executionToken"))
            .is_some());
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&source),
            before
        );
        let executed = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            serde_json::json!({"executionToken": first_token.unwrap()}),
        );
        assert!(executed.ok, "{executed:?}");
        assert!(std::fs::read_to_string(source.join("Subsystems/Sales.xml"))
            .unwrap()
            .contains("capacity probe"));
    }

    /// Два плана на одной ревизии: второй не должен уничтожить первый.
    ///
    /// Это единственное место контракта, где ошибка невидима. Замер до правки:
    /// оба применения проходили, и запись первого агента исчезала без единой
    /// диагностики — потерянное обновление в чистом виде. Пример на один план
    /// такого не показывает, поэтому сценарий строится на двух.
    #[test]
    fn conflicting_preview_plans_cannot_both_publish() {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        std::fs::create_dir_all(source.join("Documents")).unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects><Document>Order</Document></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let descriptor = source.join("Documents/Order.xml");
        std::fs::write(
            &descriptor,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20"><Document uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>Order</Name><Synonym/><Comment/></Properties><ChildObjects/></Document></MetaDataObject>"#,
        )
        .unwrap();
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(
                crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(),
            ),
            Arc::new(TokioClock),
        );
        let workspace_hint = std::fs::canonicalize(workspace.path()).unwrap();
        let call = |arguments| {
            let request = InvocationRequest::new(
                ToolIdentity::Apply,
                arguments,
                workspace_hint.to_string_lossy(),
                7_000,
            )
            .unwrap();
            direct_v5(&runtime, request).unwrap()
        };
        let plan = |comment: &str| {
            serde_json::json!({
                "at": "main:Document.Order",
                "ops": [{"op": "props.set", "args": {"values": {"Comment": comment}}}],
            })
        };
        let preview = |comment: &str| {
            let result = call(plan(comment));
            assert!(result.ok, "{result:?}");
            (
                result.rev.unwrap(),
                result.data.as_ref().unwrap()["executionToken"]
                    .as_str()
                    .unwrap()
                    .to_string(),
            )
        };
        let publish = |token: &str| call(serde_json::json!({"executionToken": token}));

        // Оба агента планируют по одному и тому же состоянию исходников.
        let first_fence = preview("от первого");
        let second_fence = preview("от второго");
        assert_ne!(
            first_fence.0, second_fence.0,
            "distinct plans bind distinct postimages"
        );
        // Changes outside the plan inputs do not invalidate its preview.
        std::fs::write(source.join("unrelated.bsl"), "unrelated change").unwrap();
        let mixed = call(
            serde_json::json!({"executionToken": first_fence.1, "at": "main:Document.Order", "ops": plan("подмена операции")["ops"]}),
        );
        assert!(
            !mixed.ok && mixed.diagnostics[0]["code"] == "bad_value",
            "{mixed:?}"
        );

        let first = publish(&first_fence.1);
        assert!(first.ok, "{first:?}");
        assert!(std::fs::read_to_string(&descriptor)
            .unwrap()
            .contains("от первого"));

        // Второй план опоздал. До правки он публиковался и уничтожал первую
        // запись; теперь он назван устаревшим, и правка первого цела.
        let second = publish(&second_fence.1);
        assert!(!second.ok, "{second:?}");
        assert_eq!(second.diagnostics[0]["code"], "stale_revision");
        let published = std::fs::read_to_string(&descriptor).unwrap();
        assert!(published.contains("от первого"), "{published}");
        assert!(!published.contains("от второго"), "{published}");

        // Забор не бывает необязательным: применение без него отказывает до
        // всякой записи, а не полагается на дисциплину вызывающего.
        let mut unfenced = plan("без забора");
        unfenced["dryRun"] = serde_json::Value::Bool(false);
        let refused = call(unfenced);
        assert!(!refused.ok, "{refused:?}");
        assert_eq!(refused.diagnostics[0]["code"], "bad_value");
        assert!(refused.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("dryRun"));
        assert_eq!(std::fs::read_to_string(&descriptor).unwrap(), published);

        // Второй агент перечитывает ревизию и повторяет план — это и есть
        // названный путь восстановления.
        let retry_fence = preview("от второго");
        assert_ne!(retry_fence, second_fence);
        let retry = publish(&retry_fence.1);
        assert!(retry.ok, "{retry:?}");
        assert!(std::fs::read_to_string(&descriptor)
            .unwrap()
            .contains("от второго"));

        let committed = unica_workspace::test_support::tree_snapshot(&source);
        let replay = publish(&retry_fence.1);
        assert_eq!(
            serde_json::to_value(&retry).unwrap(),
            serde_json::to_value(&replay).unwrap()
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&source),
            committed
        );

        let pending = preview("после перезапуска");
        let restarted = V5CanonicalInvocationRuntime::new(
            Arc::new(
                crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(),
            ),
            Arc::new(TokioClock),
        );
        let unavailable = submit_canonical(
            &restarted,
            workspace.path(),
            ToolIdentity::Apply,
            serde_json::json!({"executionToken": pending.1}),
        );
        assert!(
            !unavailable.ok,
            "another actor must not reconstruct a lost plan"
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&source),
            committed
        );
        let fresh = submit_canonical(
            &restarted,
            workspace.path(),
            ToolIdentity::Apply,
            plan("после перезапуска"),
        );
        assert!(fresh.ok, "{fresh:?}");
        assert_ne!(
            fresh.data.as_ref().unwrap()["executionToken"],
            serde_json::json!(pending.1)
        );
        let executed = submit_canonical(
            &restarted,
            workspace.path(),
            ToolIdentity::Apply,
            serde_json::json!({"executionToken": fresh.data.as_ref().unwrap()["executionToken"]}),
        );
        assert!(executed.ok, "{executed:?}");
        assert!(std::fs::read_to_string(&descriptor)
            .unwrap()
            .contains("после перезапуска"));
    }

    fn scheduled_handler_workspace(source_kind: &str, handler: &str) -> tempfile::TempDir {
        let (workspace, source) = source_selection_read_fixture();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            format!("format: DESIGNER\nsource-set:\n  - name: main\n    type: {source_kind}\n    path: src\n"),
        )
        .unwrap();
        let extension_properties = if source_kind == "EXTENSION" {
            "<ObjectBelonging>Adopted</ObjectBelonging><NamePrefix/>"
        } else {
            ""
        };
        std::fs::write(
            source.join("Configuration.xml"),
            format!(r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name>{extension_properties}<DefaultLanguage>Language.Русский</DefaultLanguage></Properties><ChildObjects><Language>Русский</Language><Catalog>Items</Catalog><CommonModule>Handlers</CommonModule></ChildObjects></Configuration></MetaDataObject>"#),
        )
        .unwrap();
        std::fs::create_dir_all(source.join("Languages")).unwrap();
        std::fs::write(
            source.join("Languages/Русский.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="33333333-3333-4333-8333-333333333333"><Properties><Name>Русский</Name><Comment/><LanguageCode>ru</LanguageCode></Properties></Language></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::create_dir_all(source.join("CommonModules/Handlers/Ext")).unwrap();
        std::fs::write(
            source.join("CommonModules/Handlers.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>Handlers</Name><Synonym/><Comment/><Global>false</Global><ClientManagedApplication>false</ClientManagedApplication><Server>true</Server><ExternalConnection>false</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>true</ServerCall><Privileged>false</Privileged><ReturnValuesReuse>DontUse</ReturnValuesReuse></Properties></CommonModule></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("CommonModules/Handlers/Ext/Module.bsl"),
            handler,
        )
        .unwrap();
        workspace
    }

    fn scheduled_job_create_plan() -> serde_json::Value {
        serde_json::json!({
            "at": "main:Configuration",
            "ops": [{"op": "object.create", "args": {"values": {
                "kind": "ScheduledJob", "name": "Nightly"
            }}}]
        })
    }

    #[test]
    fn canonical_scheduled_job_defaulted_handler_saved_plan_round_trips_cf_and_cfe() {
        for (source_kind, handler, method) in [
            ("CONFIGURATION", "Procedure Run(Profile = Undefined, Manual = False) Export\nEndProcedure\n", "Run"),
            ("EXTENSION", "Процедура Запустить(\nПрофиль = Неопределено,\nРучнойЗапуск = Ложь\n) Экспорт\nКонецПроцедуры\n", "Запустить"),
        ] {
            let workspace = scheduled_handler_workspace(source_kind, handler);
            let source = workspace.path().join("src");
            let before = unica_workspace::test_support::tree_snapshot(&source);
            let runtime = bootstrap_runtime();
            let planned = submit_canonical(&runtime, workspace.path(), ToolIdentity::Apply, scheduled_job_create_plan());
            assert!(planned.ok, "{source_kind}: {planned:?}");
            assert_eq!(unica_workspace::test_support::tree_snapshot(&source), before);
            let plan = planned.data.as_ref().unwrap();
            assert!(plan["executionToken"].as_str().is_some_and(|token| !token.is_empty()));
            let applied = submit_canonical(
                &runtime, workspace.path(), ToolIdentity::Apply,
                serde_json::json!({"executionToken": plan["executionToken"]}),
            );
            assert!(applied.ok, "{source_kind}: {applied:?}");
            assert_eq!(applied.data.as_ref().unwrap()["planHash"], plan["planHash"]);
            let viewed = submit_canonical(
                &runtime, workspace.path(), ToolIdentity::View,
                serde_json::json!({"at": "main:ScheduledJob.Nightly"}),
            );
            assert!(viewed.ok, "{source_kind}: {viewed:?}");
            let props = &viewed.data.as_ref().unwrap()["props"];
            assert_eq!(props["handlerModule"], "CommonModule.Handlers");
            assert_eq!(props["handlerMethod"], method);
            let descriptor = std::fs::read_to_string(source.join("ScheduledJobs/Nightly.xml")).unwrap();
            let parsed = roxmltree::Document::parse(descriptor.trim_start_matches('\u{feff}')).unwrap();
            let selected = parsed.descendants().find(|node| node.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "MethodName"))).unwrap();
            assert_eq!(selected.text(), Some(format!("CommonModule.Handlers.{method}").as_str()));
            assert_eq!(std::fs::read_to_string(source.join("CommonModules/Handlers/Ext/Module.bsl")).unwrap(), handler);
        }
    }

    #[test]
    fn canonical_scheduled_job_required_handler_refuses_plan_without_writes() {
        for source_kind in ["CONFIGURATION", "EXTENSION"] {
            let workspace = scheduled_handler_workspace(
                source_kind,
                "Procedure Run(Required, Optional = False) Export\nEndProcedure\n",
            );
            let source = workspace.path().join("src");
            let before = unica_workspace::test_support::tree_snapshot(&source);
            let runtime = bootstrap_runtime();
            let refused = submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::Apply,
                scheduled_job_create_plan(),
            );
            assert!(!refused.ok, "{source_kind}: {refused:?}");
            assert_eq!(refused.diagnostics[0]["code"], "bad_value", "{refused:?}");
            assert!(refused
                .data
                .as_ref()
                .and_then(|data| data.get("executionToken"))
                .is_none());
            assert!(refused.changed.is_empty());
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&source),
                before
            );
            assert!(!source.join("ScheduledJobs/Nightly.xml").exists());
        }
    }

    #[test]
    fn canonical_scheduled_job_saved_plan_refuses_changed_handler_without_publication() {
        for source_kind in ["CONFIGURATION", "EXTENSION"] {
            let workspace = scheduled_handler_workspace(
                source_kind,
                "Procedure Run(Manual = False) Export\nEndProcedure\n",
            );
            let source = workspace.path().join("src");
            let owner_before = std::fs::read(source.join("Configuration.xml")).unwrap();
            let runtime = bootstrap_runtime();
            let planned = submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::Apply,
                scheduled_job_create_plan(),
            );
            assert!(planned.ok, "{source_kind}: {planned:?}");
            // Still callable: only the retained input changed, not its admissibility.
            let foreign = "Procedure Run(Manual = True) Export\nEndProcedure\n";
            let module = source.join("CommonModules/Handlers/Ext/Module.bsl");
            std::fs::write(&module, foreign).unwrap();
            let before_execution = unica_workspace::test_support::tree_snapshot(&source);
            let refused = submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::Apply,
                serde_json::json!({"executionToken": planned.data.as_ref().unwrap()["executionToken"]}),
            );
            assert!(!refused.ok, "{source_kind}: {refused:?}");
            assert_eq!(
                refused.diagnostics[0]["code"], "stale_revision",
                "{refused:?}"
            );
            assert!(refused.changed.is_empty());
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&source),
                before_execution
            );
            assert_eq!(std::fs::read_to_string(module).unwrap(), foreign);
            assert_eq!(
                std::fs::read(source.join("Configuration.xml")).unwrap(),
                owner_before
            );
            assert!(!source.join("ScheduledJobs").exists());
        }
    }

    #[test]
    fn canonical_apply_view_round_trip_common_module_ordinary_client() {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        std::fs::create_dir_all(source.join("CommonModules/OrdinaryClient/Ext")).unwrap();
        std::fs::create_dir_all(source.join("Documents")).unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        ).unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects><CommonModule>OrdinaryClient</CommonModule><Document>Order</Document></ChildObjects></Configuration></MetaDataObject>"#,
        ).unwrap();
        let descriptor = source.join("CommonModules/OrdinaryClient.xml");
        std::fs::write(
            &descriptor,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>OrdinaryClient</Name><Synonym/><Comment/><Global>false</Global><ClientManagedApplication>false</ClientManagedApplication><Server>true</Server><ExternalConnection>false</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>true</ServerCall><Privileged>false</Privileged><ReturnValuesReuse>DontUse</ReturnValuesReuse></Properties></CommonModule></MetaDataObject>"#,
        ).unwrap();
        std::fs::write(
            source.join("CommonModules/OrdinaryClient/Ext/Module.bsl"),
            "",
        )
        .unwrap();
        let document = source.join("Documents/Order.xml");
        std::fs::write(
            &document,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document uuid="22222222-2222-4222-8222-222222222222"><Properties><Name>Order</Name><Synonym/><Comment/></Properties><ChildObjects/></Document></MetaDataObject>"#,
        ).unwrap();
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(
                crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(),
            ),
            Arc::new(TokioClock),
        );
        let workspace_hint = std::fs::canonicalize(workspace.path()).unwrap();
        let call = |tool, arguments| {
            let request =
                InvocationRequest::new(tool, arguments, workspace_hint.to_string_lossy(), 7_000)
                    .unwrap();
            direct_v5(&runtime, request).unwrap()
        };
        let view = |at| {
            let result = call(ToolIdentity::View, serde_json::json!({"at": at}));
            assert!(result.ok, "{result:?}");
            result
        };
        let at = "main:CommonModule.OrdinaryClient";
        let plan = |at: &str, value: serde_json::Value| {
            serde_json::json!({
                "at": at,
                "ops": [{"op": "props.set", "args": {"values": {"ClientOrdinaryApplication": value}}}],
            })
        };
        assert_eq!(
            view(at).data.as_ref().unwrap()["props"]["commonModule"]["clientOrdinaryApplication"],
            false
        );
        for value in [true, false] {
            let before = std::fs::read(&descriptor).unwrap();
            let observed = view(at);
            let mut args = plan(at, serde_json::json!(value));
            let preview = call(ToolIdentity::Apply, args.clone());
            assert!(preview.ok, "props.set preview failed: {preview:?}");
            assert_eq!(std::fs::read(&descriptor).unwrap(), before);
            let after_preview = view(at);
            assert!(after_preview.rev.is_none());
            assert_eq!(after_preview.data, observed.data);
            args = serde_json::json!({"executionToken": preview.data.as_ref().unwrap()["executionToken"]});
            let applied = call(ToolIdentity::Apply, args);
            assert!(applied.ok, "props.set apply failed: {applied:?}");
            assert_eq!(
                preview.data.as_ref().unwrap()["planHash"],
                applied.data.as_ref().unwrap()["planHash"]
            );
            let xml = std::fs::read_to_string(&descriptor).unwrap();
            let parsed = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
            let properties: Vec<_> = parsed
                .descendants()
                .filter(|node| node.has_tag_name("ClientOrdinaryApplication"))
                .collect();
            assert_eq!(properties.len(), 1, "{xml}");
            assert_eq!(
                properties[0].text(),
                Some(if value { "true" } else { "false" })
            );
            assert_eq!(
                view(at).data.as_ref().unwrap()["props"]["commonModule"]
                    ["clientOrdinaryApplication"],
                value
            );
        }
        for (target, value, reason) in [
            (at, serde_json::json!("true"), "boolean"),
            (
                "main:Document.Order",
                serde_json::json!(true),
                "not supported for Document",
            ),
        ] {
            let before_module = std::fs::read(&descriptor).unwrap();
            let before_document = std::fs::read(&document).unwrap();
            let before_view = view(target);
            {
                let args = plan(target, value.clone());
                let refused = call(ToolIdentity::Apply, args);
                assert!(!refused.ok, "{refused:?}");
                assert_eq!(refused.diagnostics[0]["code"], "bad_value", "{refused:?}");
                assert!(
                    refused.diagnostics[0]["message"]
                        .as_str()
                        .unwrap()
                        .contains(reason),
                    "{refused:?}"
                );
                assert_eq!(std::fs::read(&descriptor).unwrap(), before_module);
                assert_eq!(std::fs::read(&document).unwrap(), before_document);
                let unchanged = view(target);
                assert!(unchanged.rev.is_none());
                assert_eq!(unchanged.data, before_view.data);
            }
        }
    }

    #[test]
    fn canonical_html_field_creation_binding_and_neighbor_edit_preserve_form_contract() {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        std::fs::create_dir_all(source.join("Documents")).unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name><DefaultLanguage>Language.Русский</DefaultLanguage></Properties><ChildObjects><Language>Русский</Language><Document>Order</Document></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::create_dir_all(source.join("Languages")).unwrap();
        std::fs::write(
            source.join("Languages/Русский.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="33333333-3333-4333-8333-333333333333"><Properties><Name>Русский</Name><Comment/><LanguageCode>ru</LanguageCode></Properties></Language></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Documents/Order.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20"><Document uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>Order</Name><Synonym/><Comment/></Properties><ChildObjects/></Document></MetaDataObject>"#,
        )
        .unwrap();
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(
                crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(),
            ),
            Arc::new(TokioClock),
        );
        let workspace_hint = std::fs::canonicalize(workspace.path()).unwrap();
        let call = |tool, arguments| {
            let request =
                InvocationRequest::new(tool, arguments, workspace_hint.to_string_lossy(), 7_000)
                    .unwrap();
            direct_v5(&runtime, request).unwrap()
        };
        let apply_pair = |at: &str, ops: serde_json::Value| {
            let before = unica_workspace::test_support::tree_snapshot(&source);
            let preview = call(
                ToolIdentity::Apply,
                serde_json::json!({"at": at, "ops": ops}),
            );
            assert!(preview.ok, "preview failed: {preview:?}");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&source),
                before
            );
            let token = &preview.data.as_ref().unwrap()["executionToken"];
            assert!(token.is_string(), "{preview:?}");
            let published = call(
                ToolIdentity::Apply,
                serde_json::json!({"executionToken": token}),
            );
            assert!(published.ok, "publication failed: {published:?}");
            let plan_hash = &preview.data.as_ref().unwrap()["planHash"];
            assert!(plan_hash.is_string(), "{preview:?}");
            assert_eq!(plan_hash, &published.data.as_ref().unwrap()["planHash"]);
        };
        let at = "main:Document.Order.Form.Main";
        let item_at = "main:Document.Order.Form.Main.Item.Content.Item.Html";
        apply_pair(
            "main:Document.Order",
            serde_json::json!([{"op": "form.add", "args": {
                "items": [{"name": "Main", "type": "ObjectForm"}]
            }}]),
        );
        apply_pair(
            at,
            serde_json::json!([
                {"op": "attribute.add", "args": {"items": [
                    {"name": "HtmlSource", "type": "String"}
                ]}},
                {"op": "item.add", "args": {"items": [
                    {"name": "Content", "type": "Group"}
                ]}},
                {"op": "item.add", "args": {"at": "main:Document.Order.Form.Main.Item.Content", "items": [{
                    "name": "Html", "type": "HTMLDocumentField", "path": "HtmlSource",
                    "titleLocation": "None", "width": 1, "height": 1,
                    "autoMaxWidth": false, "skipOnInput": true
                }]}}
            ]),
        );
        // События нового элемента привязывает `event.bind` на их адресах с
        // явным именем обработчика (Р2, Р4 #1447).
        apply_pair(
            item_at,
            serde_json::json!([
                {"op": "event.bind", "args": {
                    "at": format!("{item_at}.Event.DocumentComplete"),
                    "values": {"handler": "HtmlDocumentComplete"}
                }},
                {"op": "event.bind", "args": {
                    "at": format!("{item_at}.Event.OnClick"),
                    "values": {"handler": "HtmlOnClick"}
                }}
            ]),
        );

        let form_path = source.join("Documents/Order/Forms/Main/Ext/Form.xml");
        let xml = std::fs::read_to_string(&form_path).unwrap();
        let xml = xml.trim_start_matches('\u{feff}');
        let parsed = roxmltree::Document::parse(xml).unwrap();
        let html = parsed
            .descendants()
            .find(|node| {
                node.has_tag_name("HTMLDocumentField") && node.attribute("name") == Some("Html")
            })
            .expect("typed item.add creates the platform HTML element");
        let group = html.parent().unwrap().parent().unwrap();
        assert!(group.has_tag_name("UsualGroup"), "{xml}");
        assert_eq!(group.attribute("name"), Some("Content"));
        for (property, value) in [
            ("DataPath", "HtmlSource"),
            ("TitleLocation", "None"),
            ("Width", "1"),
            ("Height", "1"),
            ("AutoMaxWidth", "false"),
            ("SkipOnInput", "true"),
        ] {
            assert_eq!(
                html.children()
                    .find(|node| node.has_tag_name(property))
                    .and_then(|node| node.text()),
                Some(value),
                "{property}: {xml}"
            );
        }
        for companion in ["ContextMenu", "ExtendedTooltip"] {
            let nodes: Vec<_> = html
                .children()
                .filter(|node| node.has_tag_name(companion))
                .collect();
            assert_eq!(nodes.len(), 1, "{companion}: {xml}");
            assert!(nodes[0].attribute("id").is_some(), "{xml}");
        }
        let events: Vec<_> = html
            .descendants()
            .filter(|node| node.has_tag_name("Event"))
            .collect();
        assert_eq!(events.len(), 2, "{xml}");
        assert!(
            events
                .iter()
                .any(|node| node.attribute("name") == Some("OnClick")
                    && node.text() == Some("HtmlOnClick")),
            "{xml}"
        );
        assert!(
            events
                .iter()
                .any(|node| node.attribute("name") == Some("DocumentComplete")
                    && node.text() == Some("HtmlDocumentComplete")),
            "{xml}"
        );
        let preserved_html = xml[html.range()].to_string();

        for target in [at, item_at] {
            let viewed = call(ToolIdentity::View, serde_json::json!({"at": target}));
            assert!(viewed.ok, "{viewed:?}");
            assert_eq!(viewed.at.as_deref(), Some(target));
            if target == item_at {
                assert_eq!(
                    viewed.data.as_ref().unwrap()["props"]["tag"],
                    "[HTMLDocumentField]"
                );
            }
        }
        let checked = call(ToolIdentity::Check, serde_json::json!({"at": at}));
        assert!(checked.ok, "{checked:?}");
        assert_eq!(
            checked.data.as_ref().unwrap()["diagnosticCount"],
            0,
            "{checked:?}"
        );

        // A later operation must not discard an HTML field it did not create.
        apply_pair(
            "main:Document.Order.Form.Main.Item.Content",
            serde_json::json!([{"op": "item.add", "args": {"items": [
                {"name": "Neighbor", "type": "InputField", "path": "HtmlSource", "after": "Html"}
            ]}}]),
        );
        let with_neighbor = std::fs::read_to_string(&form_path).unwrap();
        let with_neighbor = with_neighbor.trim_start_matches('\u{feff}');
        let parsed = roxmltree::Document::parse(with_neighbor).unwrap();
        let html = parsed
            .descendants()
            .find(|node| node.has_tag_name("HTMLDocumentField"))
            .unwrap();
        assert_eq!(&with_neighbor[html.range()], preserved_html);
        assert_eq!(
            html.next_sibling_element().unwrap().attribute("name"),
            Some("Neighbor")
        );
        let child_items = parsed
            .descendants()
            .find(|node| node.has_tag_name("ChildItems"))
            .unwrap();
        let ids: Vec<_> = child_items
            .descendants()
            .filter_map(|node| node.attribute("id"))
            .collect();
        let unique: std::collections::BTreeSet<_> = ids.iter().copied().collect();
        assert_eq!(
            ids.len(),
            unique.len(),
            "UI elements and companions have distinct IDs: {with_neighbor}"
        );

        // Poison a batch after a valid addition: neither operation may leak to disk.
        let before = unica_workspace::test_support::tree_snapshot(&source);
        {
            let refused = call(
                ToolIdentity::Apply,
                serde_json::json!({
                    "at": at,
                    "ops": [
                        {"op": "item.add", "args": {"items": [{"name": "MustNotAppear", "type": "InputField", "path": "HtmlSource"}]}},
                        {"op": "event.bind", "args": {"at": format!("{item_at}.Event.OnChange"), "values": {"handler": "InvalidHandler"}}}
                    ]
                }),
            );
            assert!(!refused.ok, "{refused:?}");
            assert!(refused
                .data
                .as_ref()
                .is_none_or(|data| data.get("executionToken").is_none()));
            assert!(
                refused
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.to_string().contains("FORM_EVENT_NOT_ALLOWED")),
                "{refused:?}"
            );
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&source),
                before
            );
        }
        for invalid_item in [
            serde_json::json!({"name": "MixedInput", "type": "InputField", "html": "MixedInput", "path": "HtmlSource"}),
            serde_json::json!({"name": "MixedGroup", "type": "Group", "html": "MixedGroup"}),
            serde_json::json!({"name": "MixedBar", "type": "CommandBar", "html": "MixedBar"}),
            serde_json::json!({"name": "LegacyHtml", "html": "LegacyHtml", "path": "HtmlSource"}),
            serde_json::json!({"name": "RepeatedHtml", "type": "HTMLDocumentField", "html": "RepeatedHtml"}),
            serde_json::json!({"name": "InvalidHtml", "type": "HTMLDocumentField", "on": ["OnChange"]}),
            serde_json::json!({"name": "InvalidHtml", "type": "HTMLDocumentField", "width": -1}),
            serde_json::json!({"name": "InvalidGroup", "type": "Group", "children": [
                {"name": "InvalidHtml", "type": "HTMLDocumentField"}
            ]}),
        ] {
            {
                let refused = call(
                    ToolIdentity::Apply,
                    serde_json::json!({
                        "at": at,
                        "ops": [{"op": "item.add", "args": {"items": [invalid_item]}}]
                    }),
                );
                assert!(
                    !refused.ok,
                    "invalid HTML definition was accepted: {refused:?}"
                );
                assert!(!refused.diagnostics.is_empty(), "{refused:?}");
                assert!(refused
                    .data
                    .as_ref()
                    .is_none_or(|data| data.get("executionToken").is_none()));
                assert_eq!(
                    unica_workspace::test_support::tree_snapshot(&source),
                    before
                );
            }
        }

        let original = std::fs::read_to_string(&form_path).unwrap();
        let parsed = roxmltree::Document::parse(original.trim_start_matches('\u{feff}')).unwrap();
        let imported_id = parsed
            .descendants()
            .find(|node| node.attribute("name") == Some("Html"))
            .unwrap()
            .attribute("id")
            .unwrap();
        let marker = format!("name=\"Html\" id=\"{imported_id}\"");
        assert_eq!(original.matches(&marker).count(), 1);
        for highest in [usize::MAX, usize::MAX - 1, usize::MAX - 2] {
            let exhausted = original.replace(&marker, &format!("name=\"Html\" id=\"{highest}\""));
            std::fs::write(&form_path, exhausted).unwrap();
            let before = unica_workspace::test_support::tree_snapshot(&source);
            let refused = call(
                ToolIdentity::Apply,
                serde_json::json!({
                    "at": at,
                    "ops": [{"op": "item.add", "args": {"items": [{
                        "name": "OverflowHtml", "type": "HTMLDocumentField", "path": "HtmlSource"
                    }]}}]
                }),
            );
            assert!(!refused.ok, "highest={highest}: {refused:?}");
            assert!(
                refused
                    .data
                    .as_ref()
                    .is_none_or(|data| data.get("executionToken").is_none()),
                "{refused:?}"
            );
            assert!(
                refused
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.to_string().contains("Form ID space exhausted")),
                "{refused:?}"
            );
            assert!(refused.changed.is_empty(), "{refused:?}");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&source),
                before
            );
        }
    }

    #[test]
    fn canonical_apply_constant_type_create_edit_view_and_atomic_refusal() {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        ).unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name><DefaultLanguage>Language.Русский</DefaultLanguage></Properties><ChildObjects><Language>Русский</Language></ChildObjects></Configuration></MetaDataObject>"#,
        ).unwrap();
        std::fs::create_dir_all(source.join("Languages")).unwrap();
        std::fs::write(
            source.join("Languages/Русский.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="33333333-3333-4333-8333-333333333333"><Properties><Name>Русский</Name><Comment/><LanguageCode>ru</LanguageCode></Properties></Language></MetaDataObject>"#,
        )
        .unwrap();
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(
                crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(),
            ),
            Arc::new(TokioClock),
        );
        let workspace_hint = std::fs::canonicalize(workspace.path()).unwrap();
        let call = |tool, arguments| {
            let request =
                InvocationRequest::new(tool, arguments, workspace_hint.to_string_lossy(), 7_000)
                    .unwrap();
            direct_v5(&runtime, request).unwrap()
        };
        let publish = |ops: serde_json::Value| {
            let before = unica_workspace::test_support::tree_snapshot(workspace.path());
            let creates_object = ops[0]["op"] == "object.create";
            let at = if creates_object {
                "main:Configuration"
            } else {
                ops[0]["args"]["at"].as_str().expect("edit target")
            };
            let preview = call(
                ToolIdentity::Apply,
                serde_json::json!({"at": at, "ops": ops}),
            );
            assert!(preview.ok, "{preview:?}");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(workspace.path()),
                before,
                "planning writes nothing"
            );
            let token = &preview.data.as_ref().unwrap()["executionToken"];
            assert!(token.as_str().is_some_and(|token| !token.is_empty()));
            let applied = call(
                ToolIdentity::Apply,
                serde_json::json!({"executionToken": token}),
            );
            assert!(applied.ok, "{applied:?}");
            let preview_data = preview.data.as_ref().unwrap();
            let applied_data = applied.data.as_ref().unwrap();
            assert_eq!(
                preview_data["operationCount"],
                applied_data["operationCount"]
            );
            assert_eq!(preview_data["effects"], applied_data["effects"]);
            assert_eq!(preview.changed, applied.changed);
            assert_eq!(preview_data["planHash"], applied_data["planHash"]);
            applied
        };
        let view_type = || {
            let viewed = call(
                ToolIdentity::View,
                serde_json::json!({"at": "main:Constant.Enabled"}),
            );
            assert!(viewed.ok, "{viewed:?}");
            let props = &viewed.data.as_ref().unwrap()["props"];
            assert!(
                props.get("type").is_none(),
                "type has one public projection: {props}"
            );
            assert!(
                props["Type"].is_object(),
                "type is nested JSON, not a string: {props}"
            );
            props["Type"].clone()
        };
        let assert_xml_type = |expected: &str| {
            let xml = std::fs::read_to_string(source.join("Constants/Enabled.xml")).unwrap();
            let document = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
            let types: Vec<_> = document
                .descendants()
                .filter(|node| node.has_tag_name(("http://v8.1c.ru/8.1/data/core", "Type")))
                .collect();
            assert_eq!(types.len(), 1, "{xml}");
            let (prefix, local) = types[0].text().unwrap().split_once(':').unwrap();
            assert_eq!(local, expected);
            assert_eq!(
                types[0].lookup_namespace_uri(Some(prefix)),
                Some(if expected == "boolean" {
                    "http://www.w3.org/2001/XMLSchema"
                } else {
                    "http://v8.1c.ru/8.1/data/enterprise/current-config"
                })
            );
            assert!(
                !document
                    .descendants()
                    .any(|node| node
                        .has_tag_name(("http://v8.1c.ru/8.1/data/core", "StringQualifiers"))),
                "old string qualifiers survived: {xml}"
            );
        };
        publish(serde_json::json!([
            {"op": "object.create", "args": {"values": {"kind": "Constant", "name": "Enabled"}}},
            {"op": "props.set", "args": {"at": "main:Constant.Enabled", "values": {"Type": {"variants": [{"kind": "boolean"}]}}}}
        ]));
        assert_xml_type("boolean");
        assert_eq!(view_type()["variants"][0]["kind"], "boolean");
        publish(serde_json::json!([
            {"op": "object.create", "args": {"values": {"kind": "Catalog", "name": "Products"}}}
        ]));
        let reference = serde_json::json!([
            {"op": "props.set", "args": {"at": "main:Constant.Enabled", "values": {"Type": {"variants": [{"kind": "reference", "metadataPath": "Catalog.Products"}]}}}}
        ]);
        publish(reference.clone());
        assert_xml_type("CatalogRef.Products");
        let observed = view_type();
        assert_eq!(observed["variants"][0]["kind"], "reference");
        assert_eq!(observed["variants"][0]["metadataPath"], "Catalog.Products");
        let before_noop = unica_workspace::test_support::tree_snapshot(&source);
        let noop = publish(reference);
        assert_eq!(noop.data.as_ref().unwrap()["effects"], 0);
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&source),
            before_noop
        );

        for (name, invalid_arguments) in [("Enabled", false), ("NewInvalidConst", true)] {
            let before = unica_workspace::test_support::tree_snapshot(workspace.path());
            let mut create_args = serde_json::json!({"values": {"kind": "Constant", "name": name}});
            if invalid_arguments {
                create_args["bogus"] = serde_json::json!(true);
            }
            let refused = call(
                ToolIdentity::Apply,
                serde_json::json!({
                    "at": "main:Configuration",
                    "ops": [
                        {"op": "object.create", "args": create_args},
                        {"op": "props.set", "args": {"at": format!("main:Constant.{name}"), "values": {"Comment": "must not publish"}}}
                    ]
                }),
            );
            assert!(
                !refused.ok,
                "a failed creation cannot authorize configuration: {refused:?}"
            );
            assert!(
                refused
                    .data
                    .as_ref()
                    .and_then(|data| data.get("executionToken"))
                    .is_none(),
                "failed planning must not mint an executable token: {refused:?}"
            );
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(workspace.path()),
                before,
                "{name}: failed create batch must not publish"
            );
        }

        // A valid first operation must not leak when a later type is invalid.
        let before = unica_workspace::test_support::tree_snapshot(workspace.path());
        let refused = call(
            ToolIdentity::Apply,
            serde_json::json!({
                "at": "main:Constant.Enabled",
                "ops": [
                    {"op": "props.set", "args": {"at": "main:Constant.Enabled", "values": {"Comment": "must not publish"}}},
                    {"op": "props.set", "args": {"at": "main:Constant.Enabled", "values": {"Type": {"variants": [{"kind": "boolean", "unknownQualifier": true}]}}}}
                ]
            }),
        );
        assert!(!refused.ok, "{refused:?}");
        assert_eq!(refused.diagnostics[0]["code"], "bad_value", "{refused:?}");
        assert!(
            refused
                .data
                .as_ref()
                .and_then(|data| data.get("executionToken"))
                .is_none(),
            "invalid batch must not mint an executable token: {refused:?}"
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(workspace.path()),
            before,
            "invalid batch must be atomic"
        );
    }

    #[test]
    fn public_metadata_apply_preview_is_index_write_free_and_matches_real_plan() {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        std::fs::create_dir_all(source.join("Documents")).unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name><DefaultLanguage>Language.Русский</DefaultLanguage></Properties><ChildObjects><Language>Русский</Language><Document>Order</Document></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::create_dir_all(source.join("Languages")).unwrap();
        std::fs::write(
            source.join("Languages/Русский.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="33333333-3333-4333-8333-333333333333"><Properties><Name>Русский</Name><Comment/><LanguageCode>ru</LanguageCode></Properties></Language></MetaDataObject>"#,
        )
        .unwrap();
        let descriptor = source.join("Documents/Order.xml");
        std::fs::write(
            &descriptor,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20"><Document uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>Order</Name><Synonym/><Comment/></Properties><ChildObjects/></Document></MetaDataObject>"#,
        )
        .unwrap();
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(
                crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(),
            ),
            Arc::new(TokioClock),
        );
        let workspace_hint = std::fs::canonicalize(workspace.path()).unwrap();
        let index_context = unica_source_contract::workspace::WorkspaceContext {
            cwd: workspace_hint.clone(),
            workspace_root: workspace_hint.clone(),
            cache_root: workspace_hint.join(".build/unica"),
            workspace_epoch: 0,
        };
        let index_state_root =
            unica_adapter_rlm::index::rlm_provider_state_root(&index_context, &source).unwrap();
        let index_status = unica_adapter_rlm::index::status_path(&index_context, &source).unwrap();
        let index_lock = index_state_root.join("locks/rlm-bsl/index-v17/bsl_index.lock");
        std::fs::create_dir_all(index_status.parent().unwrap()).unwrap();
        std::fs::create_dir_all(index_lock.parent().unwrap()).unwrap();
        std::fs::write(
            &index_status,
            serde_json::to_vec_pretty(&serde_json::json!({
                "status": "building",
                "source_root": source.display().to_string(),
                "db_path": null,
                "message": "preview state trap",
                "failure_class": null,
                "build_id": null,
                "next_action": null,
                "updated_at": 0,
                "last_run": null
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            &index_lock,
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema_version": 1,
                "lock_id": "preview-state-trap",
                "owner_pid": std::process::id(),
                "action": "build",
                "source_root": source.display().to_string(),
                "started_at": 0,
                "updated_at": 0,
                "state": "active"
            }))
            .unwrap(),
        )
        .unwrap();
        // Эта ловушка изменилась бы при любой проверке готовности индекса:
        // устаревшая блокировка стала бы recovered, а состояние — failed.
        let index_state_before = unica_workspace::test_support::tree_snapshot(&index_state_root);

        struct IndexBuildObserverReset;
        impl Drop for IndexBuildObserverReset {
            fn drop(&mut self) {
                unica_adapter_rlm::index::set_isolated_build_start_observer_for_test(None);
            }
        }
        let call = |arguments| {
            let request = InvocationRequest::new(
                ToolIdentity::Apply,
                arguments,
                workspace_hint.to_string_lossy(),
                7_000,
            )
            .unwrap();
            direct_v5(&runtime, request).unwrap()
        };
        let apply_pair = |operation: serde_json::Value| {
            let before = std::fs::read(&descriptor).unwrap();
            let index_starts = std::rc::Rc::new(std::cell::Cell::new(0));
            let observed_starts = std::rc::Rc::clone(&index_starts);
            let (dry, repeated_dry) = {
                // Наблюдатель стоит на первой строке start_isolated_build и
                // видит вызов даже до раннего отказа доставки RLM или записи.
                unica_adapter_rlm::index::set_isolated_build_start_observer_for_test(Some(
                    Box::new(move |_| observed_starts.set(observed_starts.get() + 1)),
                ));
                let _reset = IndexBuildObserverReset;
                (
                    call(serde_json::json!({
                        "at": "main:Document.Order",
                        "ops": [operation.clone()],
                    })),
                    call(serde_json::json!({
                        "at": "main:Document.Order",
                        "ops": [operation.clone()],
                    })),
                )
            };
            assert!(dry.ok, "dry-run failed: {dry:?}");
            assert_eq!(std::fs::read(&descriptor).unwrap(), before);
            assert_eq!(index_starts.get(), 0, "preview started RLM indexing");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&index_state_root),
                index_state_before,
                "preview wrote RLM index state"
            );
            assert_eq!(
                dry.data.as_ref().unwrap()["planHash"],
                repeated_dry.data.as_ref().unwrap()["planHash"],
                "dry-run planning must be deterministic"
            );
            let real = call(
                serde_json::json!({"executionToken": dry.data.as_ref().unwrap()["executionToken"]}),
            );
            assert!(real.ok, "real apply failed: {real:?}");
            assert_ne!(std::fs::read(&descriptor).unwrap(), before);
            assert_eq!(
                dry.data.as_ref().unwrap()["operationCount"],
                real.data.as_ref().unwrap()["operationCount"]
            );
            assert_eq!(
                dry.data.as_ref().unwrap()["effects"],
                real.data.as_ref().unwrap()["effects"]
            );
            assert_eq!(
                dry.data.as_ref().unwrap()["planHash"],
                real.data.as_ref().unwrap()["planHash"]
            );
            assert_eq!(dry.changed, real.changed);
        };

        apply_pair(serde_json::json!({
            "op": "props.set",
            "args": {"values": {"Comment": "planned"}}
        }));
        apply_pair(serde_json::json!({
            "op": "attribute.add",
            "args": {"items": [{"name": "Total"}]}
        }));
        apply_pair(serde_json::json!({
            "op": "attribute.set",
            "args": {
                "at": "main:Document.Order.Attribute.Total",
                "values": {"comment": "updated"}
            }
        }));
        apply_pair(serde_json::json!({
            "op": "attribute.remove",
            "args": {"at": "main:Document.Order.Attribute.Total"}
        }));

        let before_rejected = std::fs::read(&descriptor).unwrap();
        let rejected = call(serde_json::json!({
            "at": "main:Document.Order",
            "ops": [
                {"op": "props.set", "args": {"values": {"Comment": "must not publish"}}},
                {"op": "object.create", "args": {"values": {"kind": "Catalog", "name": "Ghost"}}}
            ],
        }));
        assert!(!rejected.ok);
        assert_eq!(rejected.diagnostics[0]["code"], "bad_value");
        assert_eq!(std::fs::read(&descriptor).unwrap(), before_rejected);

        let before_reverted = std::fs::read(&descriptor).unwrap();
        let net_zero_preview = call(serde_json::json!({
            "at": "main:Document.Order",
            "ops": [
                {"op": "props.set", "args": {"values": {"Comment": "transient"}}},
                {"op": "props.set", "args": {"values": {"Comment": "planned"}}}
            ],
        }));
        assert!(net_zero_preview.ok, "{net_zero_preview:?}");
        let reverted = call(
            serde_json::json!({"executionToken": net_zero_preview.data.as_ref().unwrap()["executionToken"]}),
        );
        assert!(reverted.ok, "net-zero apply failed: {reverted:?}");
        assert!(reverted.changed.is_empty());
        assert_eq!(reverted.data.as_ref().unwrap()["effects"], 0);
        assert_eq!(std::fs::read(&descriptor).unwrap(), before_reverted);
    }

    /// A task recovered as Working belongs to an attempt that died with its
    /// process: recovery terminalizes it as uncertain without any domain
    /// callback — apply is never re-executed to resume it.
    #[test]
    pub(crate) fn working_task_recovery_is_resume_unsupported_without_apply_reexecution() {
        use std::str::FromStr;
        use unica_execution::invocation_store::EpochMillisClock;
        use unica_execution::invocation_store_v5::{
            InvocationStoreV5, NewV5InvocationRecord, RecoveryTerminalReason, V5SafeFailureReason,
            V5StoredTask, V5TaskIdentity,
        };
        use unica_execution::receipt_ledger::ReceiptKeyDigest;
        use unica_execution::task_store::FileInvocationStoreV5;
        use unica_work_control::provider_deadline::ProviderDeadline;

        struct FixedEpoch(u64);
        impl EpochMillisClock for FixedEpoch {
            fn now_epoch_millis(&self) -> u64 {
                self.0
            }
        }

        let root = tempfile::tempdir().unwrap();
        let state_root = std::fs::canonicalize(root.path()).unwrap();
        let state = DaemonStateDirectory::open(&state_root, &CoreIdentity::production()).unwrap();
        let tasks = state.create_private_retained_subdirectory("tasks").unwrap();
        let deadline = ProviderDeadline::from_budget(Duration::from_secs(7));
        let (store, _) = FileInvocationStoreV5::open_retained_directory_inspect_only(
            tasks,
            Arc::new(FixedEpoch(7_500)),
            deadline,
        )
        .unwrap();
        let apply_executions = AtomicUsize::new(0);

        let working = store
            .create_exact(
                NewV5InvocationRecord::new(
                    V5TaskIdentity::new(
                        unica_execution_contract::invocation::TaskId::new(),
                        unica_execution_contract::invocation::InvocationId::new(),
                        ReceiptKeyDigest::from_str(&"91".repeat(32)).unwrap(),
                    ),
                    V5ToolIdentity::Apply,
                    unica_execution_contract::invocation::NormalizedArgumentsHash::from_sha256(
                        [0x91; 32],
                    ),
                    unica_execution_contract::invocation::SafeIdentityHash::from_sha256([0x92; 32]),
                    250,
                    60_000,
                )
                .for_recovered_begun(false),
                deadline,
            )
            .unwrap();
        assert_eq!(working.task, V5StoredTask::Working);

        let recovered = store
            .terminalize_recovered_exact(
                &working.identity(),
                working.version,
                RecoveryTerminalReason::OutcomeUncertain,
                deadline,
            )
            .unwrap();

        assert!(
            matches!(
                recovered.task,
                V5StoredTask::Failed {
                    reason: V5SafeFailureReason::OutcomeUncertain,
                    ..
                }
            ),
            "a working task recovers as a closed uncertain failure: {:?}",
            recovered.task
        );
        assert_eq!(recovered.version, working.version + 1);
        assert_eq!(
            store
                .terminalize_recovered_exact(
                    &working.identity(),
                    working.version,
                    RecoveryTerminalReason::OutcomeUncertain,
                    deadline,
                )
                .unwrap(),
            recovered,
            "repeated recovery is idempotent"
        );
        assert_eq!(
            apply_executions.load(Ordering::SeqCst),
            0,
            "resume is unsupported: recovery never re-executes apply"
        );
    }

    #[test]
    pub(crate) fn v13_daemon_rejects_unproved_edt_invalid_or_empty_platform_fallback() {
        let empty = tempfile::tempdir().unwrap();
        let edt = tempfile::tempdir().unwrap();
        let invalid = tempfile::tempdir().unwrap();

        let edt_source = edt.path().join("src");
        std::fs::create_dir_all(&edt_source).unwrap();
        std::fs::write(
            edt.path().join("v8project.yaml"),
            "format: EDT\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();

        let invalid_source = invalid.path().join("src");
        std::fs::create_dir_all(&invalid_source).unwrap();
        std::fs::write(
            invalid.path().join("v8project.yaml"),
            "source-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            invalid_source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(invalid_source.join(".project"), "edt marker").unwrap();

        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(
                crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(),
            ),
            Arc::new(TokioClock),
        );
        let mut admitted = Vec::new();
        for (label, workspace) in [
            ("empty", empty.path()),
            ("edt", edt.path()),
            ("invalid", invalid.path()),
        ] {
            let request = InvocationRequest::new(
                ToolIdentity::View,
                serde_json::json!({"at": "main:Catalog.Items"}),
                std::fs::canonicalize(workspace).unwrap().to_string_lossy(),
                7_000,
            )
            .unwrap();
            if bind_workspace_invocation(
                &request,
                &runtime.workspace_actors,
                Arc::clone(&runtime.deliveries),
                Arc::clone(&runtime.provider_hosts),
                Arc::clone(&runtime.runtime_resources),
                None,
                runtime.capture_response_deadline_for_test(),
            )
            .is_ok()
            {
                admitted.push(label);
            }
        }

        assert!(
            admitted.is_empty(),
            "unproved source maps entered canonical Platform XML admission: {admitted:?}"
        );
    }

    #[test]
    fn logical_read_of_one_object_opens_only_its_own_files() {
        assert_logical_read_files_do_not_grow_with_unrelated_objects(false);
    }

    /// The configuration root proves only `Configuration.xml`: it opens no
    /// object descriptor and lists no collection, however many are registered.
    #[test]
    fn configuration_root_view_opens_no_object_descriptor() {
        let (workspace, source) = source_selection_read_fixture();
        add_unrelated_catalogs(&source, 6);
        let runtime = bootstrap_runtime();
        let reads = RetainedReadProbe::watch(&source);

        let root = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::View,
            serde_json::json!({"at": "main:Configuration"}),
        );

        assert!(root.ok, "the configuration root view must succeed");
        let root_reads = reads.take();
        assert!(
            root_reads
                .iter()
                .any(|read| *read == RetainedRead::Opened("Configuration.xml".into())),
            "the probe must observe the root read: {root_reads:?}"
        );
        assert!(
            root_reads.iter().all(|read| match read {
                RetainedRead::Opened(path) => !path.starts_with("Catalogs"),
                RetainedRead::Listed(_) => false,
            }),
            "the configuration root read object descriptors: {root_reads:?}"
        );
    }

    /// The streamed root of a `Configuration.xml` above 8 MiB proves only the
    /// file itself, like the ordinary root.
    #[test]
    fn streamed_configuration_root_view_opens_no_object_descriptor() {
        let (workspace, source) = source_selection_read_fixture();
        add_unrelated_catalogs(&source, 6);
        let configuration = source.join("Configuration.xml");
        let text = std::fs::read_to_string(&configuration).unwrap();
        std::fs::write(
            &configuration,
            text.replace(
                "</MetaDataObject>",
                &format!("<!--{}--></MetaDataObject>", "x".repeat(9 * 1024 * 1024)),
            ),
        )
        .unwrap();
        let runtime = bootstrap_runtime();
        let reads = RetainedReadProbe::watch(&source);

        let root = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::View,
            serde_json::json!({"at": "main:Configuration"}),
        );

        assert!(root.ok, "the streamed configuration root view must succeed");
        assert_eq!(root.data.as_ref().unwrap()["props"]["totalObjects"], 7);
        let root_reads = reads.take();
        assert!(
            root_reads.iter().all(|read| match read {
                RetainedRead::Opened(path) => !path.starts_with("Catalogs"),
                RetainedRead::Listed(_) => false,
            }),
            "the streamed configuration root read object descriptors: {root_reads:?}"
        );
    }

    #[test]
    fn borrowed_logical_read_admits_and_publishes_without_unrelated_objects() {
        assert_logical_read_files_do_not_grow_with_unrelated_objects(true);
    }

    /// Registers `count` catalogs the read never addresses, each with its own
    /// descriptor, so a tree scan or a read of every descriptor grows with it.
    fn add_unrelated_catalogs(source: &std::path::Path, count: usize) {
        let configuration = source.join("Configuration.xml");
        let registrations = (0..count)
            .map(|index| format!("<Catalog>Unrelated{index}</Catalog>"))
            .collect::<String>();
        let text = std::fs::read_to_string(&configuration).unwrap();
        assert!(text.contains("<ChildObjects>"), "{text}");
        std::fs::write(
            &configuration,
            text.replacen(
                "<ChildObjects>",
                &format!("<ChildObjects>{registrations}"),
                1,
            ),
        )
        .unwrap();
        for index in 0..count {
            std::fs::write(
                source.join(format!("Catalogs/Unrelated{index}.xml")),
                format!(
                    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="dddddddd-dddd-4ddd-8ddd-{index:012}"><Properties><Name>Unrelated{index}</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#
                ),
            )
            .unwrap();
        }
    }

    struct LogicalReadPhases {
        admission: Vec<RetainedRead>,
        execution: Vec<RetainedRead>,
        publication: Vec<RetainedRead>,
    }

    /// Runs one logical `view` through production admission, execution and
    /// publication and records the source files each phase opens or lists.
    fn observe_logical_read(parent_publication: bool, unrelated: usize) -> LogicalReadPhases {
        let workspace = if parent_publication {
            borrowing_view_fixture("normal")
        } else {
            source_selection_read_fixture().0
        };
        add_unrelated_catalogs(&workspace.path().join("src"), unrelated);
        if parent_publication {
            add_unrelated_catalogs(&workspace.path().join("ext"), unrelated);
        }
        let runtime = bootstrap_runtime();
        let probe = RetainedReadProbe::watch(workspace.path());
        let request = InvocationRequest::new(
            ToolIdentity::View,
            serde_json::json!({"at": if parent_publication { "ext:Catalog.Items" } else { "main:Catalog.Items" }}),
            std::fs::canonicalize(workspace.path())
                .unwrap()
                .to_string_lossy(),
            7_000,
        )
        .unwrap();
        let invocation = bind_workspace_invocation(
            &request,
            &runtime.workspace_actors,
            Arc::clone(&runtime.deliveries),
            Arc::clone(&runtime.provider_hosts),
            Arc::clone(&runtime.runtime_resources),
            None,
            runtime.capture_response_deadline_for_test(),
        )
        .unwrap();
        let cancellation = CancellationToken::new();
        let execution = invocation.begin_execution(&cancellation).unwrap();
        let admission = probe.take();
        let service =
            crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default();
        let staged = service.execute(&execution, cancellation.clone()).unwrap();
        assert!(staged.ok, "{staged:?}");
        assert!(
            staged.data.is_some(),
            "the reader must stage actual source data"
        );
        if parent_publication {
            assert_eq!(
                staged.data.as_ref().unwrap()["props"]["parentStatus"],
                "resolved"
            );
        }
        let execution_reads = probe.take();
        let result = execution
            .publish(Ok(staged), &cancellation)
            .unwrap()
            .expect("a logical read publishes a domain result");
        let publication = probe.take();
        assert!(result.ok, "{result:?}");
        assert!(
            result.data.is_some(),
            "successful read must keep source data"
        );
        LogicalReadPhases {
            admission,
            execution: execution_reads,
            publication,
        }
    }

    /// A read admits, stages and publishes from the addressed node: the set of
    /// opened and listed source entries is the same for one and for many
    /// unrelated registered objects, and never touches their descriptors.
    /// A borrowed read still resolves its parent by reading the parent's
    /// descriptors while staging (#1457); its admission and publication
    /// remain bounded by the addressed nodes.
    fn assert_logical_read_files_do_not_grow_with_unrelated_objects(parent_publication: bool) {
        let few = observe_logical_read(parent_publication, 1);
        let many = observe_logical_read(parent_publication, 6);
        let mut phases = vec![
            ("admission", &few.admission, &many.admission),
            ("publication", &few.publication, &many.publication),
        ];
        if !parent_publication {
            phases.push(("execution", &few.execution, &many.execution));
        }
        for (phase, few, many) in phases {
            assert_eq!(
                few, many,
                "{phase} source work grows with unrelated objects"
            );
            assert!(
                !many
                    .iter()
                    .any(|read| format!("{read:?}").contains("Unrelated")),
                "{phase} read an unaddressed object: {many:?}"
            );
            assert!(
                !many
                    .iter()
                    .any(|read| matches!(read, RetainedRead::Listed(_))),
                "{phase} enumerated a source directory: {many:?}"
            );
        }
        assert!(
            few.execution
                .iter()
                .any(|read| format!("{read:?}").contains("Items.xml")),
            "the probe must observe the addressed descriptor read: {:?}",
            few.execution
        );
        // A borrowed read revalidates the source selection at publication:
        // the project file and each selected root marker, never an object.
        assert!(
            few.publication.iter().all(|read| {
                parent_publication
                    && matches!(read, RetainedRead::Opened(path)
                        if path.ends_with("v8project.yaml") || path.ends_with("Configuration.xml"))
            }),
            "publication must not reopen source files: {:?}",
            few.publication
        );
    }

    /// #1251, criterion 1: the default paths of `view`, an addressed `check`
    /// of a module body and an `apply` preview reach their providers through
    /// the daemon's real admission and invocation service without any
    /// operation deadline.
    #[test]
    fn default_operation_paths_reach_providers_without_a_deadline() {
        let workspace =
            scheduled_handler_workspace("CONFIGURATION", "Procedure Run() Export\nEndProcedure\n");
        let runtime = bootstrap_runtime();
        let service =
            crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default();
        let cancellation = CancellationToken::new();
        let admit = |tool, arguments| {
            let request = InvocationRequest::new(
                tool,
                arguments,
                std::fs::canonicalize(workspace.path())
                    .unwrap()
                    .to_string_lossy(),
                7_000,
            )
            .unwrap();
            bind_workspace_invocation(
                &request,
                &runtime.workspace_actors,
                Arc::clone(&runtime.deliveries),
                Arc::clone(&runtime.provider_hosts),
                Arc::clone(&runtime.runtime_resources),
                None,
                runtime.capture_response_deadline_for_test(),
            )
            .unwrap()
            .begin_execution(&cancellation)
            .unwrap()
        };

        let view = admit(
            ToolIdentity::View,
            serde_json::json!({"at": "main:Catalog.Items"}),
        );
        let reader_deadlines = view
            .read_sources()
            .unwrap()
            .iter()
            .map(|source| source.deadline().remaining())
            .collect::<Vec<_>>();
        assert!(!reader_deadlines.is_empty());
        assert!(
            reader_deadlines.iter().all(Option::is_none),
            "view readers received an operation deadline: {reader_deadlines:?}"
        );
        let viewed = service.execute(&view, cancellation.clone()).unwrap();
        let viewed = view.publish(Ok(viewed), &cancellation).unwrap().unwrap();
        assert!(viewed.ok, "{viewed:?}");

        let (provider, analyzer) =
            unica_adapter_bsl_analyzer::diagnostics::analyze_timeout_probe::recording_provider();
        crate::infrastructure::application_ports::DIAGNOSTIC_PROVIDER_OVERRIDE
            .with(|slot| *slot.borrow_mut() = Some(provider));
        let check = admit(
            ToolIdentity::Check,
            serde_json::json!({"at": "main:CommonModule.Handlers.Body"}),
        );
        let checked = service.execute(&check, cancellation.clone()).unwrap();
        crate::infrastructure::application_ports::DIAGNOSTIC_PROVIDER_OVERRIDE
            .with(|slot| *slot.borrow_mut() = None);
        assert_eq!(
            analyzer.timeouts(),
            [None],
            "the analyzer of an addressed check received a timeout: {checked:?}"
        );

        super::admission_test_control::take_apply_deadlines();
        let apply = admit(
            ToolIdentity::Apply,
            serde_json::json!({
                "at": "main:CommonModule.Handlers",
                "ops": [{"op": "code.replace", "args": {
                    "at": "main:CommonModule.Handlers.Body",
                    "text": "Procedure Run() Export\n    // planned\nEndProcedure\n",
                }}],
            }),
        );
        let planned = service.execute(&apply, cancellation.clone()).unwrap();
        assert!(planned.ok, "{planned:?}");
        assert_eq!(
            super::admission_test_control::take_apply_deadlines(),
            [None],
            "the apply preview admission received an operation deadline"
        );
    }

    /// The saved-plan adapter carries the existing cutoff into real actor
    /// publication. A finite cutoff must not become a fresh relative budget,
    /// and an elapsed finite cutoff must not become absence or consume the plan.
    #[test]
    fn saved_apply_adapter_preserves_the_existing_cutoff_and_absence() {
        use unica_work_control::operation_deadline::OperationDeadline;
        use unica_workspace::actor::{ApplyPublicationErrorKind, SavedApplyExecutionError};

        let now = Instant::now();
        for deadline in [
            OperationDeadline::Finite(now + Duration::from_secs(3_600)),
            OperationDeadline::NoDeadline,
            OperationDeadline::Finite(now - Duration::from_secs(1)),
        ] {
            let workspace = subsystem_picture_workspace("", true);
            let source = workspace.path().join("src");
            let before = unica_workspace::test_support::tree_snapshot(&source);
            let runtime = bootstrap_runtime();
            let preview = submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::Apply,
                serde_json::json!({
                    "at": "main:Subsystem.Sales",
                    "ops": [{"op": "props.set", "args": {"values": {"Comment": "same saved cutoff"}}}],
                }),
            );
            assert!(preview.ok, "{preview:?}");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&source),
                before
            );
            let token = preview.data.as_ref().unwrap()["executionToken"]
                .as_str()
                .unwrap();
            let request = InvocationRequest::new(
                ToolIdentity::Apply,
                serde_json::json!({"executionToken": token}),
                std::fs::canonicalize(workspace.path())
                    .unwrap()
                    .to_string_lossy(),
                7_000,
            )
            .unwrap();
            let cancellation = CancellationToken::new();
            let execution = bind_workspace_invocation(
                &request,
                &runtime.workspace_actors,
                Arc::clone(&runtime.deliveries),
                Arc::clone(&runtime.provider_hosts),
                Arc::clone(&runtime.runtime_resources),
                None,
                runtime.capture_response_deadline_for_test(),
            )
            .unwrap()
            .begin_execution(&cancellation)
            .unwrap();
            let expired = deadline.is_elapsed_at(Instant::now());
            let expected = match deadline {
                OperationDeadline::NoDeadline => ProviderDeadline::no_deadline(),
                OperationDeadline::Finite(cutoff) => ProviderDeadline::new(cutoff),
            };
            super::admission_test_control::take_apply_deadlines();
            let result = execution.execute_saved_apply(
                token,
                &cancellation,
                deadline,
                |preview, publication| {
                    publication.expect("a live cutoff publishes the actual saved plan");
                    preview
                },
            );
            let observed = super::admission_test_control::take_apply_deadline_values();
            assert_eq!(observed.len(), 1);
            if expired {
                assert_eq!(observed[0].remaining(), Some(Duration::ZERO));
                assert!(matches!(
                    result,
                    Err(SavedApplyExecutionError::Publication(error))
                        if error.kind() == ApplyPublicationErrorKind::Deadline
                ));
                assert_eq!(
                    unica_workspace::test_support::tree_snapshot(&source),
                    before
                );
                let retried = execution
                    .execute_saved_apply(
                        token,
                        &cancellation,
                        OperationDeadline::NoDeadline,
                        |preview, publication| {
                            publication.expect("expiry must leave the saved plan pending");
                            preview
                        },
                    )
                    .unwrap();
                assert!(retried.ok, "{retried:?}");
                assert_eq!(
                    super::admission_test_control::take_apply_deadline_values(),
                    [ProviderDeadline::no_deadline()]
                );
            } else {
                assert_eq!(
                    observed,
                    [expected],
                    "saved execution must preserve the input endpoint or its absence"
                );
                assert!(result.unwrap().ok);
            }
            assert!(std::fs::read_to_string(source.join("Subsystems/Sales.xml"))
                .unwrap()
                .contains("same saved cutoff"));
        }
    }

    /// #1338: only a binding that left the workspace actor is a changed
    /// workspace the caller may repeat; an invariant failure at the
    /// execution boundary stays an internal failure, and neither carries the
    /// boundary's text.
    #[test]
    fn execution_boundary_names_a_changed_workspace_only_for_a_lost_binding() {
        use super::invocation_service::ExecutionBoundaryError;
        use unica_execution::invocation_store_v5::V5SafeFailureReason;
        for (error, reason) in [
            (
                ExecutionBoundaryError::binding_changed(
                    "provider root binding belongs to another workspace actor".to_string(),
                ),
                V5SafeFailureReason::WorkspaceChanged,
            ),
            (
                ExecutionBoundaryError::from(
                    "unpublished Apply result requires real actor apply publication",
                ),
                V5SafeFailureReason::InvocationFailed,
            ),
            (
                ExecutionBoundaryError::from("parent read leases are poisoned"),
                V5SafeFailureReason::InvocationFailed,
            ),
        ] {
            let failure = super::boundary_failure(error);
            assert_eq!(V5SafeFailureReason::of_execution_failure(&failure), reason);
            assert!(!failure.message.contains("Apply"), "{failure:?}");
            assert!(!failure.message.contains("poisoned"), "{failure:?}");
        }
    }

    /// #1338: an addressed module `check` whose analyzer gave no result
    /// names the engine, what happened to it and how to recover, through the
    /// daemon's real admission and invocation service. An absent engine is
    /// `provider_absent`; the engine's own text never reaches the answer.
    #[test]
    fn module_check_names_why_the_analyzer_gave_no_result() {
        use unica_adapter_bsl_analyzer::adapter::AnalyzerRunFailureKind;
        let workspace =
            scheduled_handler_workspace("CONFIGURATION", "Procedure Run() Export\nEndProcedure\n");
        let runtime = bootstrap_runtime();
        let service =
            crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default();
        let cancellation = CancellationToken::new();
        for (kind, detail, cause) in [
            (
                AnalyzerRunFailureKind::EngineMissing,
                Some("provider_absent"),
                "bsl-analyzer is not available for darwin-arm64: its file is not on this machine; build it from third-party/tools.lock.json",
            ),
            (
                AnalyzerRunFailureKind::StartFailed,
                Some("provider_absent"),
                "analysis engine could not be started",
            ),
            (
                AnalyzerRunFailureKind::TimedOut,
                None,
                "analysis exceeded its time limit",
            ),
            (
                AnalyzerRunFailureKind::Exited,
                None,
                "analysis engine exited with an error before producing a result. Repeat the call",
            ),
        ] {
            let request = InvocationRequest::new(
                ToolIdentity::Check,
                serde_json::json!({"at": "main:CommonModule.Handlers.Body"}),
                std::fs::canonicalize(workspace.path())
                    .unwrap()
                    .to_string_lossy(),
                7_000,
            )
            .unwrap();
            let check = bind_workspace_invocation(
                &request,
                &runtime.workspace_actors,
                Arc::clone(&runtime.deliveries),
                Arc::clone(&runtime.provider_hosts),
                Arc::clone(&runtime.runtime_resources),
                None,
                runtime.capture_response_deadline_for_test(),
            )
            .unwrap()
            .begin_execution(&cancellation)
            .unwrap();
            crate::infrastructure::application_ports::DIAGNOSTIC_PROVIDER_OVERRIDE.with(|slot| {
                *slot.borrow_mut() =
                    Some(unica_adapter_bsl_analyzer::diagnostics::analyze_timeout_probe::failing_provider(kind))
            });
            let checked = service.execute(&check, cancellation.clone()).unwrap();
            crate::infrastructure::application_ports::DIAGNOSTIC_PROVIDER_OVERRIDE
                .with(|slot| *slot.borrow_mut() = None);
            assert!(!checked.ok, "{kind:?}");
            assert_eq!(checked.diagnostics.len(), 1, "{kind:?}");
            let diagnostic = &checked.diagnostics[0];
            assert_eq!(diagnostic["code"], "provider_unavailable", "{kind:?}");
            assert_eq!(
                diagnostic.get("detailCode").and_then(serde_json::Value::as_str),
                detail,
                "{kind:?}"
            );
            let message = diagnostic["message"].as_str().unwrap();
            assert!(
                message.starts_with(
                    "BSL analysis did not complete, so the module is unproven: bsl-analyzer"
                ),
                "{kind:?}"
            );
            assert!(message.contains(cause), "{kind:?}");
            assert!(!checked_text_leaks(&checked), "{kind:?}");
        }
    }

    /// #1966, INV.APP.DIAGNOSTIC-FOCUS, INV.SAFETY.DIAGNOSTIC-LOCATION-BOUNDARY:
    /// `check` справочника спрашивает находки у поставщика, который объявил цель
    /// «объект метаданных». Находка на свойстве реквизита приходит цепочкой
    /// коллекция → элемент → свойство и язык; недоказанный элемент — находкой
    /// объекта целиком; файл внутри объекта без логического адреса — путём
    /// с причиной и владельцем; находка модуля объекту не принадлежит.
    /// Анализатор модулей такой цели не объявляет, и без подмены шага нет.
    #[test]
    fn object_check_shows_the_metadata_focus_chain_of_a_provider_finding() {
        use unica_diagnostics::model::{
            DiagnosticAction, DiagnosticContext, DiagnosticFocusKind, DiagnosticObservation,
            DiagnosticObservationFocus, DiagnosticObservationLocation, DiagnosticProvider,
            DiagnosticProviderDescriptor, DiagnosticProviderId, DiagnosticProviderOutcome,
            DiagnosticProviderRequest, DiagnosticProviderStatus, DiagnosticSeverity, MetadataFocus,
        };
        use unica_metadata::MetadataElement;
        use unica_work_control::provider_deadline::ProviderDeadline;

        static DESCRIPTOR: DiagnosticProviderDescriptor = DiagnosticProviderDescriptor {
            id: DiagnosticProviderId::new_const("metadata-provider"),
            actions: &[DiagnosticAction::Findings],
            findings_target_kinds: &[
                unica_address::source_target::TargetKind::MetadataObject,
                unica_address::source_target::TargetKind::SourceRoot,
            ],
            emits_focus_kinds: &[DiagnosticFocusKind::Target, DiagnosticFocusKind::Metadata],
        };
        struct MetadataProvider(Vec<DiagnosticObservation>);
        impl DiagnosticProvider for MetadataProvider {
            fn descriptor(&self) -> &'static DiagnosticProviderDescriptor {
                &DESCRIPTOR
            }
            fn execute(
                &self,
                _request: &DiagnosticProviderRequest,
                _context: &DiagnosticContext,
                _deadline: ProviderDeadline,
                _cancellation: &CancellationToken,
            ) -> DiagnosticProviderOutcome {
                DiagnosticProviderOutcome {
                    observations: self.0.clone(),
                    ..DiagnosticProviderOutcome::empty(DiagnosticProviderStatus::Completed)
                }
            }
        }
        let finding = |location, focus| DiagnosticObservation::Diagnostic {
            provider: DESCRIPTOR.id,
            location,
            focus,
            code: "MetadataRule".to_string(),
            severity: DiagnosticSeverity::Warning,
            message: "finding".to_string(),
            tags: Vec::new(),
        };
        let object = || DiagnosticObservationLocation::Logical {
            metadata_path: Some(
                unica_address::source_target::MetadataAddress::parse(
                    unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
                    "Catalog.Items",
                )
                .unwrap(),
            ),
        };
        let attribute = |name: &str| MetadataFocus {
            element_path: vec![MetadataElement {
                collection: "attributes".to_string(),
                name: name.to_string(),
            }],
            property: Some("Synonym".to_string()),
            language: Some("ru".to_string()),
        };

        let workspace =
            scheduled_handler_workspace("CONFIGURATION", "Procedure Run()\nEndProcedure\n");
        let source = workspace.path().join("src");
        std::fs::write(
            source.join("Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20"><Catalog uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>Items</Name></Properties><ChildObjects><Attribute uuid="bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"><Properties><Name>Номенклатура</Name><Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Номенклатура</v8:content></v8:item></Synonym></Properties></Attribute></ChildObjects></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::create_dir_all(source.join("Catalogs/Items/Ext")).unwrap();
        std::fs::write(source.join("Catalogs/Items/Ext/Unknown.xml"), "<unknown/>").unwrap();
        let provider = MetadataProvider(vec![
            finding(
                DiagnosticObservationLocation::Logical {
                    metadata_path: None,
                },
                DiagnosticObservationFocus::Target,
            ),
            finding(
                object(),
                DiagnosticObservationFocus::Metadata(attribute("Номенклатура")),
            ),
            finding(
                object(),
                DiagnosticObservationFocus::Metadata(attribute("Missing")),
            ),
            finding(
                DiagnosticObservationLocation::Resource {
                    handle: source
                        .join("Catalogs/Items/Ext/Unknown.xml")
                        .to_string_lossy()
                        .into_owned(),
                },
                DiagnosticObservationFocus::Target,
            ),
            finding(
                DiagnosticObservationLocation::Resource {
                    handle: source
                        .join("CommonModules/Handlers/Ext/Module.bsl")
                        .to_string_lossy()
                        .into_owned(),
                },
                DiagnosticObservationFocus::Target,
            ),
        ]);

        let runtime = bootstrap_runtime();
        let service =
            crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default();
        let cancellation = CancellationToken::new();
        let check = |at: &str, override_provider: Option<Arc<dyn DiagnosticProvider>>| {
            let request = InvocationRequest::new(
                ToolIdentity::Check,
                serde_json::json!({"at": at}),
                std::fs::canonicalize(workspace.path())
                    .unwrap()
                    .to_string_lossy(),
                7_000,
            )
            .unwrap();
            let check = bind_workspace_invocation(
                &request,
                &runtime.workspace_actors,
                Arc::clone(&runtime.deliveries),
                Arc::clone(&runtime.provider_hosts),
                Arc::clone(&runtime.runtime_resources),
                None,
                runtime.capture_response_deadline_for_test(),
            )
            .unwrap()
            .begin_execution(&cancellation)
            .unwrap();
            crate::infrastructure::application_ports::DIAGNOSTIC_PROVIDER_OVERRIDE
                .with(|slot| *slot.borrow_mut() = override_provider);
            let checked = service.execute(&check, cancellation.clone());
            crate::infrastructure::application_ports::DIAGNOSTIC_PROVIDER_OVERRIDE
                .with(|slot| *slot.borrow_mut() = None);
            let checked = checked.unwrap();
            assert!(checked.ok, "object check must answer a verdict");
            checked.data.unwrap()
        };

        let provider: Arc<dyn DiagnosticProvider> = Arc::new(provider);
        let data = check("main:Catalog.Items", Some(Arc::clone(&provider)));
        assert_eq!(
            data["validators"],
            serde_json::json!(["meta", "bsl"]),
            "the provider finding keeps its location"
        );
        let provided = data["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["provider"] == "metadata-provider")
            .collect::<Vec<_>>();
        assert_eq!(
            provided.len(),
            3,
            "neither the module nor the root finding is the object's"
        );
        let addressed = serde_json::json!({
            "kind": "addressed",
            "sourceSet": "main",
            "metadataPath": "Catalog.Items",
            "targetKind": "metadataObject",
        });
        assert!(
            provided.iter().any(|item| item["location"] == addressed
                && item["focus"]
                    == serde_json::json!({
                        "kind": "metadata",
                        "elementPath": [{"collection": "attributes", "name": "Номенклатура"}],
                        "property": "Synonym",
                        "language": "ru",
                    })),
            "the provider finding keeps its location"
        );
        assert!(
            provided.iter().any(|item| item["location"] == addressed
                && item["focus"] == serde_json::json!({"kind": "target"})),
            "an unproven element weakens to the object"
        );
        assert!(
            provided
                .iter()
                .any(|item| item["location"]["kind"] == "unaddressable"
                    && item["location"]["ownerMetadataPath"] == "Catalog.Items"
                    && item["location"]["path"] == "Catalogs/Items/Ext/Unknown.xml"
                    && item["locationReason"] == "resourceNotAddressable"),
            "the provider finding keeps its location"
        );
        assert!(
            !data.to_string().contains(&*source.to_string_lossy()),
            "the provider finding keeps its location"
        );

        let plain = check("main:Catalog.Items", None);
        assert_eq!(
            plain["validators"],
            serde_json::json!(["meta"]),
            "no step without a provider"
        );

        // Корень принимает только свою находку: объекты и модули — свои цели.
        let root = check("main:Configuration", Some(provider));
        let provided = root["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["provider"] == "metadata-provider")
            .collect::<Vec<_>>();
        assert_eq!(provided.len(), 1, "the root keeps only its own finding");
        assert_eq!(
            provided[0]["location"],
            serde_json::json!({"kind": "addressed", "sourceSet": "main", "targetKind": "sourceRoot"}),
            "the root finding is addressed to the root"
        );
        let plain = check("main:Configuration", None);
        assert_eq!(
            plain["validators"],
            serde_json::json!(["cf"]),
            "no step without a provider"
        );
    }

    /// #2177: порт символов на настоящей службе. Справочник и метод модуля
    /// отвечает сам Unica: читатель и текст модуля; остальное — поставщик,
    /// которого тест подменяет без изменения службы. Ответ называет места
    /// адресами Unica и не несёт путей поставщика; изменённый буфер
    /// отвечает своим текстом, а ответ поставщика помечен `stale`.
    #[test]
    fn symbol_port_answers_in_unica_places_with_a_substituted_provider() {
        use std::sync::Mutex;
        use unica_code_intelligence::symbols::{
            CallDirection, ProviderFailure, ProviderPlace, ProviderPlaces, ProviderSymbol,
            SymbolKind, SymbolPosition, SymbolProvider, SymbolQuery, SymbolRange,
            SymbolRequestContext,
        };

        const MODULE: &str = "Процедура Run() Export\n\tСсылка = Справочники.Items.ПустаяСсылка();\n\tПомощник();\n\tВнешний.Метод();\nEndProcedure\n\nПроцедура Помощник()\nEndProcedure\n";

        #[derive(Default)]
        struct Stub(Mutex<Vec<SymbolQuery>>);
        impl SymbolProvider for Stub {
            fn symbol(
                &self,
                query: &SymbolQuery,
                _context: &SymbolRequestContext<'_>,
            ) -> Result<Option<ProviderSymbol>, ProviderFailure> {
                self.0.lock().unwrap().push(query.clone());
                let at = |line| SymbolRange {
                    start: SymbolPosition { line, column: 10 },
                    end: SymbolPosition { line, column: 18 },
                };
                Ok(Some(ProviderSymbol {
                    name: "Метод".to_string(),
                    kind: SymbolKind::Method,
                    signature: Some("Процедура Метод()".to_string()),
                    return_type: None,
                    definitions: vec![
                        ProviderPlace::Code {
                            path: "CommonModules/Handlers/Ext/Module.bsl".to_string(),
                            range: at(6),
                        },
                        ProviderPlace::Metadata {
                            kind: "Catalog".to_string(),
                            object: "Items".to_string(),
                            member: None,
                        },
                    ],
                    complete: true,
                }))
            }
            fn references(
                &self,
                _query: &SymbolQuery,
                _context: &SymbolRequestContext<'_>,
            ) -> Result<ProviderPlaces, ProviderFailure> {
                Ok(ProviderPlaces {
                    places: Vec::new(),
                    complete: true,
                })
            }
            fn calls(
                &self,
                _query: &SymbolQuery,
                _direction: CallDirection,
                _context: &SymbolRequestContext<'_>,
            ) -> Result<ProviderPlaces, ProviderFailure> {
                Ok(ProviderPlaces {
                    places: Vec::new(),
                    complete: true,
                })
            }
        }

        let workspace = scheduled_handler_workspace("CONFIGURATION", MODULE);
        let runtime = bootstrap_runtime();
        let service =
            crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default();
        let cancellation = CancellationToken::new();
        let stub = Arc::new(Stub::default());
        struct ResetOverride;
        impl Drop for ResetOverride {
            fn drop(&mut self) {
                crate::infrastructure::application_ports::SYMBOL_PROVIDER_OVERRIDE
                    .with(|slot| *slot.borrow_mut() = None);
            }
        }
        let _reset = ResetOverride;
        crate::infrastructure::application_ports::SYMBOL_PROVIDER_OVERRIDE
            .with(|slot| *slot.borrow_mut() = Some(stub.clone()));
        let position = |text: &str, needle: &str| {
            let offset = text.find(needle).unwrap();
            let (line, column) = unica_bsl_parser::symbols::LineIndex::new(text).position(offset);
            SymbolPosition { line, column }
        };
        let ask =
            |buffer: Option<&str>,
             needle: &str,
             operation: crate::infrastructure::daemon::v13_service::SymbolOperation| {
                let request = InvocationRequest::new(
                    ToolIdentity::View,
                    serde_json::json!({"at": "main:CommonModule.Handlers"}),
                    std::fs::canonicalize(workspace.path())
                        .unwrap()
                        .to_string_lossy(),
                    7_000,
                )
                .unwrap();
                let execution = bind_workspace_invocation(
                    &request,
                    &runtime.workspace_actors,
                    Arc::clone(&runtime.deliveries),
                    Arc::clone(&runtime.provider_hosts),
                    Arc::clone(&runtime.runtime_resources),
                    None,
                    runtime.capture_response_deadline_for_test(),
                )
                .unwrap()
                .begin_execution(&cancellation)
                .unwrap();
                let answer = service.symbols(
                    &execution,
                    crate::infrastructure::daemon::v13_service::SymbolRequest {
                        module: unica_address::address::QualifiedAddress::parse(
                            "main:CommonModule.Handlers",
                        )
                        .unwrap(),
                        buffer,
                        position: position(buffer.unwrap_or(MODULE), needle),
                        operation,
                    },
                    &cancellation,
                );
                assert!(answer.ok, "the symbol port answers");
                answer.data.unwrap()
            };
        use crate::infrastructure::daemon::v13_service::SymbolOperation;

        let catalog = ask(None, "Items", SymbolOperation::Definition);
        assert_eq!(
            catalog,
            serde_json::json!({"value": [{"at": "main:Catalog.Items"}], "complete": true})
        );
        let helper = ask(None, "Помощник();", SymbolOperation::Definition);
        assert_eq!(helper["complete"], true);
        assert_eq!(helper["value"][0]["range"]["start"]["line"], 6);
        // A procedure no other module sees is answered by the module alone.
        let references = ask(None, "Помощник();", SymbolOperation::References);
        assert_eq!(references["value"].as_array().unwrap().len(), 2);
        assert_eq!(references["complete"], true);
        // An exported one is looked for in other modules by the set's name
        // index: built in the background, not ready on the first request.
        let export = ask(None, "Run()", SymbolOperation::References);
        assert_eq!(export["value"].as_array().unwrap().len(), 1);
        assert_eq!(
            export["reasons"],
            serde_json::json!([{"kind": "namesNotIndexed"}]),
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let export = loop {
            let answer = ask(None, "Run()", SymbolOperation::References);
            if answer["complete"] == true || std::time::Instant::now() > deadline {
                break answer;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(export["complete"], true, "{export}");
        assert!(stub.0.lock().unwrap().is_empty(), "Unica answered alone");

        let external = ask(None, "Метод", SymbolOperation::Definition);
        let places: Vec<_> = external["value"]
            .as_array()
            .unwrap()
            .iter()
            .map(|place| place["at"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(places.len(), 2, "both provider places are addressed");
        assert!(places[0].starts_with("main:CommonModule.Handlers"));
        assert_eq!(places[1], "main:Catalog.Items");
        assert_eq!(external["complete"], true);
        let wire = external.to_string();
        assert!(!wire.contains(".bsl"), "no provider path on the wire");

        let edited = format!("{MODULE}\nПроцедура Вызвать()\n\tПомощник();\nEndProcedure\n");
        let stale = ask(Some(&edited), "Метод", SymbolOperation::Hover);
        assert_eq!(stale["complete"], false);
        assert_eq!(stale["reasons"], serde_json::json!([{"kind": "stale"}]));
        let callers = ask(
            Some(&edited),
            "Помощник()\nEnd",
            SymbolOperation::IncomingCalls,
        );
        assert_eq!(
            callers["value"].as_array().unwrap().len(),
            2,
            "the buffer's own caller counts"
        );
        assert!(matches!(
            stub.0.lock().unwrap().last(),
            Some(SymbolQuery::Named { .. })
        ));
    }

    /// #1460: платформа не компилирует модуль, где `Перем` стоит после
    /// методов, а анализатор такой модуль пропускает. `check` модуля через
    /// настоящий сервис вызова проваливается находкой с текстом платформы и
    /// местом объявления; тот же модуль с объявлением в начале проходит.
    #[test]
    fn module_check_fails_a_variable_declared_after_methods() {
        let checked = |source: &str| {
            let workspace = scheduled_handler_workspace("CONFIGURATION", source);
            let runtime = bootstrap_runtime();
            let service =
                crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default();
            let cancellation = CancellationToken::new();
            let request = InvocationRequest::new(
                ToolIdentity::Check,
                serde_json::json!({"at": "main:CommonModule.Handlers.Body"}),
                std::fs::canonicalize(workspace.path())
                    .unwrap()
                    .to_string_lossy(),
                7_000,
            )
            .unwrap();
            let check = bind_workspace_invocation(
                &request,
                &runtime.workspace_actors,
                Arc::clone(&runtime.deliveries),
                Arc::clone(&runtime.provider_hosts),
                Arc::clone(&runtime.runtime_resources),
                None,
                runtime.capture_response_deadline_for_test(),
            )
            .unwrap()
            .begin_execution(&cancellation)
            .unwrap();
            let (provider, _analyzer) =
                unica_adapter_bsl_analyzer::diagnostics::analyze_timeout_probe::recording_provider(
                );
            crate::infrastructure::application_ports::DIAGNOSTIC_PROVIDER_OVERRIDE
                .with(|slot| *slot.borrow_mut() = Some(provider));
            let checked = service.execute(&check, cancellation.clone());
            crate::infrastructure::application_ports::DIAGNOSTIC_PROVIDER_OVERRIDE
                .with(|slot| *slot.borrow_mut() = None);
            let checked = checked.unwrap();
            // Сообщения проверок — постоянные строки: ответ несёт данные
            // поддержки, и CodeQL считает их вывод утечкой.
            assert!(checked.ok, "module check must answer a verdict");
            checked.data.unwrap()
        };

        let late = checked("Procedure Run() Export\n    Cache = 1;\nEndProcedure\n\nVar Cache;\n");
        assert!(
            late["status"] == "failed",
            "a late declaration fails the check"
        );
        let diagnostics = late["diagnostics"].as_array().unwrap();
        assert!(
            diagnostics.len() == 1,
            "exactly one finding names the declaration"
        );
        let finding = &diagnostics[0];
        assert!(
            finding["validator"] == "bsl",
            "the finding belongs to the bsl step"
        );
        assert!(
            finding["code"] == "VariableDeclarationPlacement",
            "the finding names the placement rule"
        );
        assert!(finding["severity"] == "error", "the finding is an error");
        assert!(
            finding["message"]
                == "Объявления переменных должны быть расположены в начале модуля, процедуры или функции",
            "the finding carries the platform message"
        );
        assert!(
            finding["location"]["targetKind"] == "module",
            "the finding belongs to the module"
        );
        assert!(
            finding["focus"]["range"]["startLine"] == 4,
            "the finding points at the declaration line"
        );

        let early = checked("Var Cache;\n\nProcedure Run() Export\n    Cache = 1;\nEndProcedure\n");
        assert!(early["status"] == "passed", "an early declaration passes");
        assert!(
            early["diagnostics"] == serde_json::json!([]),
            "an early declaration has no findings"
        );
    }

    /// #1338: an addressed module `check` whose analyzer is still being
    /// delivered answers through the daemon's real invocation service with a
    /// retryable `working` delivery state, not with an analysis verdict. The
    /// delivery is already in flight, so this call joins it as a follower and
    /// returns at once.
    #[test]
    fn module_check_answers_working_while_the_analyzer_is_being_delivered() {
        const CHILD: &str = "UNICA_CHECK_DELIVERY_WORKING_CHILD";
        if std::env::var_os(CHILD).is_some() {
            use unica_artifact_delivery::{BootstrapError, DownloadObserver, Downloader};

            /// Holds the only download until the test releases it, so the
            /// delivery stays in flight while `check` runs.
            struct HeldDownload {
                entered: Mutex<mpsc::Sender<()>>,
                release: Mutex<mpsc::Receiver<()>>,
            }
            impl Downloader for HeldDownload {
                fn download(
                    &self,
                    _url: &str,
                    _destination: &std::path::Path,
                    _observer: &dyn DownloadObserver,
                ) -> Result<(), BootstrapError> {
                    let _ = self.entered.lock().unwrap().send(());
                    let _ = self
                        .release
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(30));
                    Err(std::io::Error::other("held download released").into())
                }
            }

            let (entered, entered_wait) = mpsc::channel();
            let (release, released) = mpsc::channel();
            let runtime =
                bootstrap_runtime().with_delivery_downloader_for_test(Arc::new(HeldDownload {
                    entered: Mutex::new(entered),
                    release: Mutex::new(released),
                }));
            let plugin = std::path::PathBuf::from(std::env::var_os("UNICA_PLUGIN_ROOT").unwrap());
            let owner = {
                let deliveries = Arc::clone(&runtime.deliveries);
                thread::spawn(move || {
                    crate::infrastructure::engine_delivery::deliver_if_missing(
                        &deliveries,
                        &plugin,
                        "bsl-analyzer",
                        &CancellationToken::new(),
                        &unica_work_control::progress::NoopProgressSink,
                    )
                })
            };
            entered_wait
                .recv_timeout(Duration::from_secs(10))
                .expect("the analyzer delivery is in flight");

            let workspace = scheduled_handler_workspace(
                "CONFIGURATION",
                "Procedure Run() Export\nEndProcedure\n",
            );
            let service =
                crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default();
            let cancellation = CancellationToken::new();
            let request = InvocationRequest::new(
                ToolIdentity::Check,
                serde_json::json!({"at": "main:CommonModule.Handlers.Body"}),
                std::fs::canonicalize(workspace.path())
                    .unwrap()
                    .to_string_lossy(),
                7_000,
            )
            .unwrap();
            let check = bind_workspace_invocation(
                &request,
                &runtime.workspace_actors,
                Arc::clone(&runtime.deliveries),
                Arc::clone(&runtime.provider_hosts),
                Arc::clone(&runtime.runtime_resources),
                None,
                runtime.capture_response_deadline_for_test(),
            )
            .unwrap()
            .begin_execution(&cancellation)
            .unwrap();
            assert!(
                crate::infrastructure::application_ports::DIAGNOSTIC_PROVIDER_OVERRIDE
                    .with(|slot| slot.borrow().is_none()),
                "the delivery path is skipped under a provider override"
            );
            let checked = service.execute(&check, cancellation.clone()).unwrap();

            release.send(()).unwrap();
            assert!(
                !matches!(
                    owner.join().unwrap(),
                    unica_artifact_delivery::shared::EngineDeliveryState::Ready(_)
                ),
                "the held download never produces the analyzer"
            );

            assert!(
                !checked.ok,
                "check must not pass while the analyzer is delivered"
            );
            assert_eq!(checked.diagnostics.len(), 1);
            let diagnostic = &checked.diagnostics[0];
            assert_eq!(diagnostic["code"], "provider_unavailable");
            assert_eq!(diagnostic["detailCode"], "delivery_in_progress");
            assert_eq!(diagnostic["outcome"], "retry");
            assert_eq!(
                checked.at.as_deref(),
                Some("main:CommonModule.Handlers.Body")
            );
            let delivery = &checked.data.as_ref().expect("delivery state")["delivery"];
            assert_eq!(delivery["artifact"], "bsl-analyzer");
            assert_eq!(delivery["status"], "working");
            assert!(delivery["retryAfterMs"].as_u64().is_some());
            return;
        }

        // A one-test child owns the delivery environment, as in
        // `canonical_selected_engine_routes_request_delivery_before_provider_start`.
        let root = tempfile::tempdir().unwrap();
        let plugin = root.path().join("plugin");
        std::fs::create_dir_all(plugin.join("third-party")).unwrap();
        let target = crate::infrastructure::platform::current_target_id().unwrap();
        let exe = std::env::consts::EXE_SUFFIX;
        std::fs::write(
            plugin.join("third-party/manifest.json"),
            serde_json::json!({
                "schemaVersion": 2,
                "tools": [{
                    "name": "bsl-analyzer",
                    "version": "0.2.67",
                    "binaryPath": format!("bin/{target}/bsl-analyzer{exe}"),
                    "sha256": "0".repeat(64),
                }],
            })
            .to_string(),
        )
        .unwrap();
        let version = env!("CARGO_PKG_VERSION");
        let mut core_targets = serde_json::Map::new();
        let mut analyzer_targets = serde_json::Map::new();
        for host in unica_artifact_delivery::HostTarget::ALL {
            let host = host.as_str();
            let suffix = if host == "win-x64" { ".exe" } else { "" };
            let core_asset = format!("unica-runtime-{host}.tar.gz");
            let core_path = format!("bin/{host}/unica{suffix}");
            core_targets.insert(host.to_owned(), serde_json::json!({
                "asset": {"name": core_asset, "url": format!("https://github.com/IngvarConsulting/unica/releases/download/v{version}/{core_asset}"), "mediaType": "application/gzip", "sha256": "1".repeat(64)},
                "files": [{"path": core_path, "sha256": "1".repeat(64), "executable": true}],
                "entrypoint": core_path,
            }));
            let asset = format!("bsl-analyzer-{host}{suffix}");
            analyzer_targets.insert(host.to_owned(), serde_json::json!({
                "asset": {"name": asset, "url": format!("https://github.com/IngvarConsulting/unica-toolchain/releases/download/bsl-analyzer-v0.2.67-build.1/{asset}"), "mediaType": "application/octet-stream", "sha256": "2".repeat(64)},
                "files": [{"path": format!("bin/{host}/bsl-analyzer{suffix}"), "sha256": "2".repeat(64), "executable": true}],
            }));
        }
        let release = root.path().join("runtime-manifest.json");
        std::fs::write(
            &release,
            serde_json::json!({
                "schemaVersion": 2,
                "pluginVersion": version,
                "source": {"repository": "https://github.com/IngvarConsulting/unica", "commit": "0".repeat(40)},
                "release": {"repository": "https://github.com/IngvarConsulting/unica", "tag": format!("v{version}")},
                "artifacts": {
                    "unica": {"version": version, "role": "core", "targets": core_targets},
                    "bsl-analyzer": {"version": "0.2.67", "role": "engine", "targets": analyzer_targets},
                }
            })
            .to_string(),
        )
        .unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "infrastructure::daemon::server::actor_capacity_tests::module_check_answers_working_while_the_analyzer_is_being_delivered",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("UNICA_PLUGIN_ROOT", &plugin)
            .env("UNICA_ARTIFACT_CACHE", root.path().join("cache"))
            .env("UNICA_RUNTIME_MANIFEST", &release)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stdout.contains("running 1 test"), "{stdout}\n{stderr}");
        assert!(output.status.success(), "{stdout}\n{stderr}");
    }

    fn checked_text_leaks(result: &unica_execution_contract::invocation::DomainResult) -> bool {
        let text = serde_json::to_string(result).unwrap();
        text.contains("private-token") || text.contains("/private/engine")
    }

    /// Граф вызовов читается только адресом ветви. Обычное чтение метода
    /// называет ветви `Caller` и `Callee` без счёта и анализатор не спрашивает;
    /// секции `callGraph` нет; чтение ветви спрашивает граф в обоих направлениях.
    #[test]
    fn method_view_names_call_graph_branches_without_asking_the_analyzer() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use unica_code_intelligence::model::CallEdgeProvenance;
        use unica_code_intelligence::model::CallGraphDirection;
        use unica_code_intelligence::model::CallGraphEdge;
        use unica_code_intelligence::model::CallGraphResult;
        use unica_code_intelligence::model::CallGraphState;
        use unica_code_intelligence::model::CodeIntelligenceContext;
        use unica_code_intelligence::model::CodeIntelligenceProvider;
        use unica_code_intelligence::model::CodeIntelligenceReadData;
        use unica_code_intelligence::model::CodeIntelligenceReadRequest;
        use unica_code_intelligence::model::ProviderCapability;
        use unica_code_intelligence::model::ProviderIdentity;
        use unica_code_intelligence::model::ProviderReadOutcome;
        use unica_code_intelligence::model::ProviderRole;
        use unica_code_intelligence::model::ProviderSearchSection;
        use unica_code_intelligence::model::SearchRequest;

        struct CountingCallGraph(AtomicUsize);
        impl CodeIntelligenceProvider for CountingCallGraph {
            fn identity(&self) -> ProviderIdentity {
                ProviderIdentity::new(ProviderRole::Symbol, "counting-call-graph")
            }
            fn capabilities(&self) -> &[ProviderCapability] {
                &[ProviderCapability::CallGraph]
            }
            fn search(
                &self,
                _request: &SearchRequest,
                _context: &CodeIntelligenceContext,
                _deadline: ProviderDeadline,
                _cancellation: &CancellationToken,
            ) -> ProviderSearchSection {
                unreachable!("the call graph test never searches")
            }
            fn read(
                &self,
                request: &CodeIntelligenceReadRequest,
                _context: &CodeIntelligenceContext,
                _deadline: ProviderDeadline,
                _cancellation: &CancellationToken,
            ) -> Result<ProviderReadOutcome, String> {
                self.0.fetch_add(1, Ordering::SeqCst);
                let CodeIntelligenceReadRequest::CallGraph { id, direction, .. } = request else {
                    return Err("only the call graph is read".to_string());
                };
                assert_eq!(id, "method/common/Handlers/Run");
                let edges = match direction {
                    CallGraphDirection::Callers => vec![CallGraphEdge {
                        id: "method/common/Handlers/Run".to_string(),
                        provenance: CallEdgeProvenance::Resolved,
                    }],
                    CallGraphDirection::Callees => Vec::new(),
                };
                Ok(ProviderReadOutcome {
                    provider: self.identity(),
                    ok: true,
                    summary: "call graph".to_string(),
                    warnings: Vec::new(),
                    errors: Vec::new(),
                    artifacts: Vec::new(),
                    stdout: None,
                    stderr: None,
                    data: Some(CodeIntelligenceReadData::CallGraph(CallGraphResult {
                        state: CallGraphState::Ready,
                        total: Some(edges.len() as u64),
                        edges,
                        revision: Some(7),
                        stale: Some(false),
                        complete: true,
                    })),
                })
            }
        }

        let workspace =
            scheduled_handler_workspace("CONFIGURATION", "Procedure Run() Export\nEndProcedure\n");
        let runtime = bootstrap_runtime();
        let service =
            crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default();
        let cancellation = CancellationToken::new();
        let provider = Arc::new(CountingCallGraph(AtomicUsize::new(0)));
        // Сбрасывает подмену на любом выходе из теста, в том числе при панике.
        struct ResetOverride;
        impl Drop for ResetOverride {
            fn drop(&mut self) {
                crate::infrastructure::application_ports::CODE_INTELLIGENCE_PROVIDER_OVERRIDE
                    .with(|slot| *slot.borrow_mut() = None);
            }
        }
        let _reset_override = ResetOverride;
        crate::infrastructure::application_ports::CODE_INTELLIGENCE_PROVIDER_OVERRIDE
            .with(|slot| *slot.borrow_mut() = Some(vec![provider.clone()]));
        let view = |arguments: serde_json::Value| {
            let request = InvocationRequest::new(
                ToolIdentity::View,
                arguments,
                std::fs::canonicalize(workspace.path())
                    .unwrap()
                    .to_string_lossy(),
                7_000,
            )
            .unwrap();
            let execution = bind_workspace_invocation(
                &request,
                &runtime.workspace_actors,
                Arc::clone(&runtime.deliveries),
                Arc::clone(&runtime.provider_hosts),
                Arc::clone(&runtime.runtime_resources),
                None,
                runtime.capture_response_deadline_for_test(),
            )
            .unwrap()
            .begin_execution(&cancellation)
            .unwrap();
            let result = service.execute(&execution, cancellation.clone()).unwrap();
            execution
                .publish(Ok(result), &cancellation)
                .unwrap()
                .unwrap()
        };
        let method = "main:CommonModule.Handlers.Method.Run";

        let plain = view(serde_json::json!({"at": method}));
        assert!(plain.ok, "a plain method view succeeds");
        let data = plain.data.as_ref().unwrap();
        assert!(
            data["props"].get("callGraph").is_none(),
            "the method node carries no call graph summary"
        );
        let branches = data["branches"].as_array().unwrap();
        for kind in ["Caller", "Callee"] {
            let branch = format!("{method}.{kind}");
            assert!(
                branches
                    .iter()
                    .any(|declared| *declared == serde_json::json!({"at": branch})),
                "{kind} is named without a count"
            );
        }
        assert_eq!(
            provider.0.load(Ordering::SeqCst),
            0,
            "a plain method view asked the analyzer"
        );

        let caller = view(serde_json::json!({"at": format!("{method}.Caller")}));
        assert!(caller.ok, "the Caller branch is read");
        let page = caller.data.as_ref().unwrap();
        assert_eq!(page["kind"], "Caller");
        assert_eq!(page["props"]["callGraph"], "ready");
        assert_eq!(page["props"]["total"], 1, "the branch carries its total");
        assert_eq!(
            page["items"][0]["at"], method,
            "the recursive call is its own caller"
        );
        let callee = view(serde_json::json!({"at": format!("{method}.Callee")}));
        assert!(callee.ok, "the Callee branch is read");
        assert_eq!(
            callee.data.as_ref().unwrap()["props"]["total"],
            0,
            "a ready empty branch proves the absence of calls"
        );
        assert_eq!(
            provider.0.load(Ordering::SeqCst),
            4,
            "each branch read asks both directions"
        );
    }
    #[test]
    fn saved_call_graph_pages_survive_deleted_owner_without_analyzer_reads() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use unica_code_intelligence::model::CallEdgeProvenance;
        use unica_code_intelligence::model::CallGraphEdge;
        use unica_code_intelligence::model::CallGraphResult;
        use unica_code_intelligence::model::CallGraphState;
        use unica_code_intelligence::model::CodeIntelligenceContext;
        use unica_code_intelligence::model::CodeIntelligenceProvider;
        use unica_code_intelligence::model::CodeIntelligenceReadData;
        use unica_code_intelligence::model::CodeIntelligenceReadRequest;
        use unica_code_intelligence::model::ProviderCapability;
        use unica_code_intelligence::model::ProviderIdentity;
        use unica_code_intelligence::model::ProviderReadOutcome;
        use unica_code_intelligence::model::ProviderRole;
        use unica_code_intelligence::model::ProviderSearchSection;
        use unica_code_intelligence::model::SearchRequest;

        struct CountingCallGraph(AtomicUsize);
        impl CodeIntelligenceProvider for CountingCallGraph {
            fn identity(&self) -> ProviderIdentity {
                ProviderIdentity::new(ProviderRole::Symbol, "counting-call-graph")
            }
            fn capabilities(&self) -> &[ProviderCapability] {
                &[ProviderCapability::CallGraph]
            }
            fn search(
                &self,
                _request: &SearchRequest,
                _context: &CodeIntelligenceContext,
                _deadline: ProviderDeadline,
                _cancellation: &CancellationToken,
            ) -> ProviderSearchSection {
                unreachable!("the call graph test never searches")
            }
            fn read(
                &self,
                request: &CodeIntelligenceReadRequest,
                _context: &CodeIntelligenceContext,
                _deadline: ProviderDeadline,
                _cancellation: &CancellationToken,
            ) -> Result<ProviderReadOutcome, String> {
                self.0.fetch_add(1, Ordering::SeqCst);
                let CodeIntelligenceReadRequest::CallGraph { id, .. } = request else {
                    return Err("only the call graph is read".to_string());
                };
                assert_eq!(id, "method/common/Handlers/Run");
                let edges = ["Run", "Next", "Last"]
                    .into_iter()
                    .map(|method| CallGraphEdge {
                        id: format!("method/common/Handlers/{method}"),
                        provenance: CallEdgeProvenance::Resolved,
                    })
                    .collect::<Vec<_>>();
                Ok(ProviderReadOutcome {
                    provider: self.identity(),
                    ok: true,
                    summary: "call graph".to_string(),
                    warnings: Vec::new(),
                    errors: Vec::new(),
                    artifacts: Vec::new(),
                    stdout: None,
                    stderr: None,
                    data: Some(CodeIntelligenceReadData::CallGraph(CallGraphResult {
                        state: CallGraphState::Ready,
                        total: Some(edges.len() as u64),
                        edges,
                        revision: Some(7),
                        stale: Some(false),
                        complete: true,
                    })),
                })
            }
        }

        let workspace =
            scheduled_handler_workspace("CONFIGURATION", "Procedure Run() Export\nEndProcedure\nProcedure Next() Export\nEndProcedure\nProcedure Last() Export\nEndProcedure\n");
        let runtime = bootstrap_runtime();
        let service =
            crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default();
        let cancellation = CancellationToken::new();
        let provider = Arc::new(CountingCallGraph(AtomicUsize::new(0)));
        // Сбрасывает подмену на любом выходе из теста, в том числе при панике.
        struct ResetOverride;
        impl Drop for ResetOverride {
            fn drop(&mut self) {
                crate::infrastructure::application_ports::CODE_INTELLIGENCE_PROVIDER_OVERRIDE
                    .with(|slot| *slot.borrow_mut() = None);
            }
        }
        let _reset_override = ResetOverride;
        crate::infrastructure::application_ports::CODE_INTELLIGENCE_PROVIDER_OVERRIDE
            .with(|slot| *slot.borrow_mut() = Some(vec![provider.clone()]));
        let view = |arguments: serde_json::Value| {
            let request = InvocationRequest::new(
                ToolIdentity::View,
                arguments,
                std::fs::canonicalize(workspace.path())
                    .unwrap()
                    .to_string_lossy(),
                7_000,
            )
            .unwrap();
            let execution = bind_workspace_invocation(
                &request,
                &runtime.workspace_actors,
                Arc::clone(&runtime.deliveries),
                Arc::clone(&runtime.provider_hosts),
                Arc::clone(&runtime.runtime_resources),
                None,
                runtime.capture_response_deadline_for_test(),
            )
            .unwrap()
            .begin_execution(&cancellation)
            .unwrap();
            let result = service.execute(&execution, cancellation.clone()).unwrap();
            execution
                .publish(Ok(result), &cancellation)
                .unwrap()
                .unwrap()
        };
        let method = "main:CommonModule.Handlers.Method.Run";
        let mut saved = Vec::new();
        for kind in ["Caller", "Callee"] {
            let at = format!("{method}.{kind}");
            let first = view(serde_json::json!({"at": at, "limit": 1}));
            assert!(first.ok, "the initial saved call-graph page succeeds");
            let cursor = first.cursor.unwrap();
            let second = view(serde_json::json!({"at": at, "limit": 1, "cursor": cursor}));
            assert!(
                second.ok && second.cursor.is_some(),
                "saved successor succeeds and continues"
            );
            saved.push((at, cursor, second));
        }
        assert_eq!(provider.0.load(Ordering::SeqCst), 4);
        std::fs::remove_file(
            workspace
                .path()
                .join("src/CommonModules/Handlers/Ext/Module.bsl"),
        )
        .unwrap();
        let fresh = view(serde_json::json!({"at": method}));
        assert!(
            !fresh.ok,
            "a fresh read must observe that the owner source was deleted"
        );
        for (at, cursor, second) in saved {
            let repeated = view(serde_json::json!({"at": at, "limit": 1, "cursor": cursor}));
            assert_eq!(
                repeated, second,
                "saved page and successor survive deletion of the owning method"
            );
            let final_page =
                view(serde_json::json!({"at": at, "limit": 1, "cursor": second.cursor}));
            assert!(
                final_page.ok && final_page.cursor.is_none(),
                "the final saved call-graph page succeeds and finishes pagination"
            );
            assert_eq!(
                final_page.data.as_ref().unwrap()["items"][0]["at"],
                "main:CommonModule.Handlers.Method.Last"
            );
        }
        assert_eq!(
            provider.0.load(Ordering::SeqCst),
            4,
            "saved pages must not read the graph again"
        );
    }

    /// #1373: ветвь графа вызовов, чей анализатор ещё поставляется, отвечает
    /// через настоящую службу вызова повторяемым состоянием поставки
    /// `working`, а не графом. Поставка уже идёт, поэтому чтение ветви
    /// присоединяется к ней ведомым и возвращается сразу.
    #[test]
    fn call_graph_branch_answers_working_while_the_analyzer_is_being_delivered() {
        const CHILD: &str = "UNICA_CALL_GRAPH_DELIVERY_WORKING_CHILD";
        if std::env::var_os(CHILD).is_some() {
            use unica_artifact_delivery::{BootstrapError, DownloadObserver, Downloader};

            /// Holds the only download until the test releases it, so the
            /// delivery stays in flight while the branch view runs.
            struct HeldDownload {
                entered: Mutex<mpsc::Sender<()>>,
                release: Mutex<mpsc::Receiver<()>>,
            }
            impl Downloader for HeldDownload {
                fn download(
                    &self,
                    _url: &str,
                    _destination: &std::path::Path,
                    _observer: &dyn DownloadObserver,
                ) -> Result<(), BootstrapError> {
                    let _ = self.entered.lock().unwrap().send(());
                    let _ = self
                        .release
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(30));
                    Err(std::io::Error::other("held download released").into())
                }
            }

            let (entered, entered_wait) = mpsc::channel();
            let (release, released) = mpsc::channel();
            let runtime =
                bootstrap_runtime().with_delivery_downloader_for_test(Arc::new(HeldDownload {
                    entered: Mutex::new(entered),
                    release: Mutex::new(released),
                }));
            let plugin = std::path::PathBuf::from(std::env::var_os("UNICA_PLUGIN_ROOT").unwrap());
            let owner = {
                let deliveries = Arc::clone(&runtime.deliveries);
                thread::spawn(move || {
                    crate::infrastructure::engine_delivery::deliver_if_missing(
                        &deliveries,
                        &plugin,
                        "bsl-analyzer",
                        &CancellationToken::new(),
                        &unica_work_control::progress::NoopProgressSink,
                    )
                })
            };
            entered_wait
                .recv_timeout(Duration::from_secs(10))
                .expect("the analyzer delivery is in flight");

            let workspace = scheduled_handler_workspace(
                "CONFIGURATION",
                "Procedure Run() Export\nEndProcedure\n",
            );
            let service =
                crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default();
            let cancellation = CancellationToken::new();
            let branch = "main:CommonModule.Handlers.Method.Run.Caller";
            let request = InvocationRequest::new(
                ToolIdentity::View,
                serde_json::json!({"at": branch}),
                std::fs::canonicalize(workspace.path())
                    .unwrap()
                    .to_string_lossy(),
                7_000,
            )
            .unwrap();
            let view = bind_workspace_invocation(
                &request,
                &runtime.workspace_actors,
                Arc::clone(&runtime.deliveries),
                Arc::clone(&runtime.provider_hosts),
                Arc::clone(&runtime.runtime_resources),
                None,
                runtime.capture_response_deadline_for_test(),
            )
            .unwrap()
            .begin_execution(&cancellation)
            .unwrap();
            assert!(
                crate::infrastructure::application_ports::CODE_INTELLIGENCE_PROVIDER_OVERRIDE
                    .with(|slot| slot.borrow().is_none()),
                "the branch must reach the real analyzer registry"
            );
            let viewed = service.execute(&view, cancellation.clone()).unwrap();

            release.send(()).unwrap();
            assert!(
                !matches!(
                    owner.join().unwrap(),
                    unica_artifact_delivery::shared::EngineDeliveryState::Ready(_)
                ),
                "the held download never produces the analyzer"
            );

            assert!(
                !viewed.ok,
                "the branch must not answer a graph while the analyzer is delivered"
            );
            assert_eq!(
                viewed.diagnostics.len(),
                1,
                "the branch names exactly one delivery refusal"
            );
            let diagnostic = &viewed.diagnostics[0];
            assert_eq!(diagnostic["code"], "provider_unavailable");
            assert_eq!(diagnostic["detailCode"], "delivery_in_progress");
            assert_eq!(diagnostic["outcome"], "retry");
            assert_eq!(viewed.at.as_deref(), Some(branch));
            let delivery = &viewed.data.as_ref().expect("delivery state")["delivery"];
            assert_eq!(delivery["artifact"], "bsl-analyzer");
            assert_eq!(delivery["status"], "working");
            assert!(delivery["retryAfterMs"].as_u64().is_some());
            return;
        }

        // A one-test child owns the delivery environment, as in
        // `canonical_selected_engine_routes_request_delivery_before_provider_start`.
        let root = tempfile::tempdir().unwrap();
        let plugin = root.path().join("plugin");
        std::fs::create_dir_all(plugin.join("third-party")).unwrap();
        let target = crate::infrastructure::platform::current_target_id().unwrap();
        let exe = std::env::consts::EXE_SUFFIX;
        std::fs::write(
            plugin.join("third-party/manifest.json"),
            serde_json::json!({
                "schemaVersion": 2,
                "tools": [{
                    "name": "bsl-analyzer",
                    "version": "0.2.67",
                    "binaryPath": format!("bin/{target}/bsl-analyzer{exe}"),
                    "sha256": "0".repeat(64),
                }],
            })
            .to_string(),
        )
        .unwrap();
        let version = env!("CARGO_PKG_VERSION");
        let mut core_targets = serde_json::Map::new();
        let mut analyzer_targets = serde_json::Map::new();
        for host in unica_artifact_delivery::HostTarget::ALL {
            let host = host.as_str();
            let suffix = if host == "win-x64" { ".exe" } else { "" };
            let core_asset = format!("unica-runtime-{host}.tar.gz");
            let core_path = format!("bin/{host}/unica{suffix}");
            core_targets.insert(host.to_owned(), serde_json::json!({
                "asset": {"name": core_asset, "url": format!("https://github.com/IngvarConsulting/unica/releases/download/v{version}/{core_asset}"), "mediaType": "application/gzip", "sha256": "1".repeat(64)},
                "files": [{"path": core_path, "sha256": "1".repeat(64), "executable": true}],
                "entrypoint": core_path,
            }));
            let asset = format!("bsl-analyzer-{host}{suffix}");
            analyzer_targets.insert(host.to_owned(), serde_json::json!({
                "asset": {"name": asset, "url": format!("https://github.com/IngvarConsulting/unica-toolchain/releases/download/bsl-analyzer-v0.2.67-build.1/{asset}"), "mediaType": "application/octet-stream", "sha256": "2".repeat(64)},
                "files": [{"path": format!("bin/{host}/bsl-analyzer{suffix}"), "sha256": "2".repeat(64), "executable": true}],
            }));
        }
        let release = root.path().join("runtime-manifest.json");
        std::fs::write(
            &release,
            serde_json::json!({
                "schemaVersion": 2,
                "pluginVersion": version,
                "source": {"repository": "https://github.com/IngvarConsulting/unica", "commit": "0".repeat(40)},
                "release": {"repository": "https://github.com/IngvarConsulting/unica", "tag": format!("v{version}")},
                "artifacts": {
                    "unica": {"version": version, "role": "core", "targets": core_targets},
                    "bsl-analyzer": {"version": "0.2.67", "role": "engine", "targets": analyzer_targets},
                }
            })
            .to_string(),
        )
        .unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "infrastructure::daemon::server::actor_capacity_tests::call_graph_branch_answers_working_while_the_analyzer_is_being_delivered",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("UNICA_PLUGIN_ROOT", &plugin)
            .env("UNICA_ARTIFACT_CACHE", root.path().join("cache"))
            .env("UNICA_RUNTIME_MANIFEST", &release)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("running 1 test"),
            "the child must run exactly the call graph delivery test"
        );
        assert!(
            output.status.success(),
            "the child call graph delivery test failed; rerun it with its environment"
        );
    }

    struct ManualInvocationClock(Mutex<Instant>);

    impl ManualInvocationClock {
        fn new(now: Instant) -> Self {
            Self(Mutex::new(now))
        }

        fn advance(&self, duration: Duration) {
            let mut now = self.0.lock().expect("manual clock lock");
            *now += duration;
        }
    }

    impl Clock for ManualInvocationClock {
        fn now(&self) -> Instant {
            *self.0.lock().unwrap()
        }
    }

    #[test]
    pub(crate) fn logical_reads_run_without_deadline_tree_scans_or_mutation_lane_wait() {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        let sibling = workspace.path().join("dep");
        std::fs::create_dir_all(source.join("Catalogs")).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: dep\n    type: CONFIGURATION\n    path: dep\n",
        )
        .unwrap();
        std::fs::write(
            sibling.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Dependency</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects><Catalog>Items</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>Items</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        add_unrelated_catalogs(&source, 3);
        let sibling_reads = RetainedReadProbe::watch(&sibling);
        let selected_reads = RetainedReadProbe::watch(&source);
        let assert_no_unrelated_object_or_listing = |label: &str| {
            let reads = selected_reads.take();
            assert!(
                !reads.iter().any(|read| {
                    matches!(read, RetainedRead::Listed(_))
                        || format!("{read:?}").contains("Unrelated")
                }),
                "{label} scanned the selected source tree: {reads:?}"
            );
        };
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(
                crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(),
            ),
            Arc::new(TokioClock),
        );
        let request = InvocationRequest::new(
            ToolIdentity::View,
            serde_json::json!({"at": "main:Catalog.Items"}),
            std::fs::canonicalize(workspace.path())
                .unwrap()
                .to_string_lossy(),
            7_000,
        )
        .unwrap();
        let invocation = bind_workspace_invocation(
            &request,
            &runtime.workspace_actors,
            Arc::clone(&runtime.deliveries),
            Arc::clone(&runtime.provider_hosts),
            Arc::clone(&runtime.runtime_resources),
            None,
            runtime.capture_response_deadline_for_test(),
        )
        .unwrap();
        let cancellation = CancellationToken::new();
        let execution = invocation.begin_execution(&cancellation).unwrap();
        assert!(execution.layout_read_source("main").is_err());
        let sibling_admission = sibling_reads.take();
        assert!(
            sibling_admission
                .iter()
                .all(|read| *read == RetainedRead::Opened("Configuration.xml".into())),
            "source selection may prove the sibling root marker, but a qualified view must not read the unrelated source set: {sibling_admission:?}"
        );
        std::fs::write(
            sibling.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>ChangedDependency</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
        )
        .unwrap();

        let sources = execution.read_sources().unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(
            sources[0].deadline().remaining(),
            None,
            "a logical read has no operation deadline, before or after the handoff window"
        );
        let service =
            crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default();
        let result = service
            .execute(&execution, cancellation.clone())
            .expect("canonical v0.13 execution");
        assert!(result.ok, "canonical v0.13 execution must succeed");
        assert!(result.rev.is_none());
        assert_no_unrelated_object_or_listing("view admission and execution");
        for (tool, arguments) in [
            (
                ToolIdentity::Resolve,
                serde_json::json!({"at":"main:Catalog.Items"}),
            ),
            (
                ToolIdentity::Search,
                serde_json::json!({"query":"Items","scope":"main:Configuration"}),
            ),
            (
                ToolIdentity::Check,
                serde_json::json!({"at":"main:Catalog.Items"}),
            ),
            (
                ToolIdentity::Diff,
                serde_json::json!({"left":"main:Catalog.Items","right":"main:Catalog.Items"}),
            ),
        ] {
            let read_request = InvocationRequest::new(
                tool,
                arguments,
                std::fs::canonicalize(workspace.path())
                    .unwrap()
                    .to_string_lossy(),
                7_000,
            )
            .unwrap();
            let read_invocation = bind_workspace_invocation(
                &read_request,
                &runtime.workspace_actors,
                Arc::clone(&runtime.deliveries),
                Arc::clone(&runtime.provider_hosts),
                Arc::clone(&runtime.runtime_resources),
                None,
                runtime.capture_response_deadline_for_test(),
            )
            .unwrap();
            let read_execution = read_invocation.begin_execution(&cancellation).unwrap();
            let read_result = service
                .execute(&read_execution, cancellation.clone())
                .unwrap();
            assert!(read_result.ok, "{tool:?}: {read_result:?}");
            assert!(
                read_result.rev.is_none(),
                "{tool:?} must not expose a source revision"
            );
            let published = read_execution
                .publish(Ok(read_result), &cancellation)
                .unwrap()
                .unwrap();
            assert!(published.ok);
            match tool {
                // A text search reads its scope by contract.
                ToolIdentity::Search => {
                    selected_reads.take();
                }
                // Resolve proves the target's place: it may enumerate the
                // directories on the target's own path to match names, but it
                // opens no other descriptor (#1457).
                ToolIdentity::Resolve => {
                    let reads = selected_reads.take();
                    assert!(
                        reads.iter().all(|read| match read {
                            RetainedRead::Listed(path) => {
                                path.as_os_str().is_empty()
                                    || path == std::path::Path::new("Catalogs")
                            }
                            RetainedRead::Opened(path) => {
                                path == std::path::Path::new("Configuration.xml")
                                    || path == std::path::Path::new("Catalogs/Items.xml")
                            }
                        }),
                        "Resolve read beyond the addressed object: {reads:?}"
                    );
                }
                _ => assert_no_unrelated_object_or_listing(&format!("{tool:?}")),
            }
            let sibling = sibling_reads.take();
            assert!(
                sibling
                    .iter()
                    .all(|read| *read == RetainedRead::Opened("Configuration.xml".into())),
                "{tool:?} must not read the unrelated source tree beyond its root marker: {sibling:?}"
            );
        }
        let actor = Arc::clone(execution.actor_for_test());
        let held_lane = actor.hold_mutation_lane_for_test();
        let (published_tx, published_rx) = mpsc::channel();
        let publish_cancellation = cancellation.clone();
        std::thread::spawn(move || {
            published_tx
                .send(execution.publish(Ok(result), &publish_cancellation))
                .unwrap();
        });
        let published = published_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("logical publication must not wait on the mutation lane")
            .unwrap()
            .unwrap();
        drop(held_lane);
        assert!(published.ok);
        assert!(published.rev.is_none());
        assert_eq!(
            selected_reads.take(),
            Vec::new(),
            "read publication must not reopen the selected source tree"
        );
        assert_eq!(
            sibling_reads.take(),
            Vec::new(),
            "an unrelated source mutation must not invalidate the qualified view"
        );

        let find_request = InvocationRequest::new(
            ToolIdentity::Resolve,
            serde_json::json!({"path": "src/Catalogs/Items.xml"}),
            std::fs::canonicalize(workspace.path())
                .unwrap()
                .to_string_lossy(),
            7_000,
        )
        .unwrap();
        let find_invocation = bind_workspace_invocation(
            &find_request,
            &runtime.workspace_actors,
            Arc::clone(&runtime.deliveries),
            Arc::clone(&runtime.provider_hosts),
            Arc::clone(&runtime.runtime_resources),
            None,
            runtime.capture_response_deadline_for_test(),
        )
        .unwrap();
        let find_execution = find_invocation.begin_execution(&cancellation).unwrap();
        assert_eq!(
            find_execution.layout_sources().unwrap().len(),
            2,
            "the find directory must admit every workspace source set"
        );
        assert!(find_execution.layout_read_source("absent").is_err());
        let layout_capability = find_execution.layout_read_source("main").unwrap();
        assert!(layout_capability
            .logical_view_read_authority(&cancellation)
            .is_ok());
        let find_result = service
            .execute(&find_execution, cancellation.clone())
            .expect("canonical v0.13 find execution");
        assert!(
            find_result.ok,
            "canonical v0.13 find execution must succeed"
        );
        // A layout directory publishes no revision state: `find` confirms
        // nothing and never waits for the actor mutation lane, so it publishes
        // while a competing apply holds that lane.
        let find_actor = Arc::clone(find_execution.actor_for_test());
        let held_lane = find_actor.hold_mutation_lane_for_test();
        let (published_tx, published_rx) = mpsc::channel();
        let publish_cancellation = cancellation.clone();
        std::thread::spawn(move || {
            published_tx
                .send(find_execution.publish(Ok(find_result), &publish_cancellation))
                .unwrap();
        });
        let published = published_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("find publication must not wait on the mutation lane")
            .expect("find publication")
            .expect("find result");
        drop(held_lane);
        assert!(published.ok);
        assert!(
            published.rev.is_none(),
            "a find directory is not a revision snapshot and must publish no rev"
        );

        let find_invocation = bind_workspace_invocation(
            &find_request,
            &runtime.workspace_actors,
            Arc::clone(&runtime.deliveries),
            Arc::clone(&runtime.provider_hosts),
            Arc::clone(&runtime.runtime_resources),
            None,
            runtime.capture_response_deadline_for_test(),
        )
        .unwrap();
        let find_execution = find_invocation.begin_execution(&cancellation).unwrap();
        let find_result = service
            .execute(&find_execution, cancellation.clone())
            .expect("second canonical v0.13 find execution");
        std::fs::write(
            sibling.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>ChangedAgain</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        assert!(
            find_execution
                .publish(Ok(find_result), &cancellation)
                .is_ok(),
            "a directory answer stands even when a source set changes after it"
        );

        let invocation = bind_workspace_invocation(
            &request,
            &runtime.workspace_actors,
            Arc::clone(&runtime.deliveries),
            Arc::clone(&runtime.provider_hosts),
            Arc::clone(&runtime.runtime_resources),
            None,
            runtime.capture_response_deadline_for_test(),
        )
        .unwrap();
        let execution = invocation.begin_execution(&cancellation).unwrap();
        let result = service
            .execute(&execution, cancellation.clone())
            .expect("second canonical v0.13 execution");
        std::fs::write(
            source.join("Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>ItemsChanged</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        assert!(
            execution.publish(Ok(result), &cancellation).is_ok(),
            "a completed read remains usable after source content changes"
        );
        let layout_execution = || {
            bind_workspace_invocation(
                &find_request,
                &runtime.workspace_actors,
                Arc::clone(&runtime.deliveries),
                Arc::clone(&runtime.provider_hosts),
                Arc::clone(&runtime.runtime_resources),
                None,
                runtime.capture_response_deadline_for_test(),
            )
            .unwrap()
            .begin_execution(&cancellation)
            .unwrap()
        };
        let cancelled_execution = layout_execution();
        assert!(cancelled_execution.layout_read_source("main").is_ok());
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let result = cancelled_execution
            .publish(
                Ok(DomainResult::success("staged resource location")),
                &cancelled,
            )
            .unwrap()
            .unwrap();
        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["code"], "cancelled");
        if unica_workspace::test_support::supports_retained_root_replacement_test() {
            let remapped_execution = layout_execution();
            assert!(remapped_execution.layout_read_source("main").is_ok());
            std::fs::rename(&source, workspace.path().join("retained-src")).unwrap();
            std::fs::create_dir_all(&source).unwrap();
            assert!(remapped_execution.layout_read_source("main").is_err());
            assert!(remapped_execution
                .publish(
                    Ok(DomainResult::success("staged resource location")),
                    &cancellation,
                )
                .is_err());
        }
    }

    fn rejected_hidden_v13_view_execution(
        at: &str,
    ) -> (tempfile::TempDir, ActorBoundExecution, RetainedReadProbe) {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(
                crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(),
            ),
            Arc::new(TokioClock),
        );
        let request = InvocationRequest::new(
            ToolIdentity::View,
            serde_json::json!({"at": at}),
            std::fs::canonicalize(workspace.path())
                .unwrap()
                .to_string_lossy(),
            7_000,
        )
        .unwrap();
        let invocation = bind_workspace_invocation(
            &request,
            &runtime.workspace_actors,
            Arc::clone(&runtime.deliveries),
            Arc::clone(&runtime.provider_hosts),
            Arc::clone(&runtime.runtime_resources),
            None,
            runtime.capture_response_deadline_for_test(),
        )
        .unwrap();
        let reads = RetainedReadProbe::watch(&source);
        let cancellation = CancellationToken::new();

        let execution = invocation
            .begin_execution(&cancellation)
            .expect("ordinary rejected address must reach the typed view handler");
        assert_eq!(
            reads.take(),
            Vec::new(),
            "an address rejected before source selection must not read a source corpus"
        );
        (workspace, execution, reads)
    }

    fn execute_rejected_hidden_v13_view_address(at: &str) -> DomainResult {
        let (_workspace, execution, reads) = rejected_hidden_v13_view_execution(at);
        let cancellation = CancellationToken::new();
        let result = crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()
            .execute(&execution, cancellation.clone())
            .expect("typed input error is a DomainResult");
        let result = execution
            .publish(Ok(result), &cancellation)
            .unwrap()
            .unwrap();
        assert_eq!(
            reads.take(),
            Vec::new(),
            "zero-fence typed rejection must not read a source corpus"
        );
        result
    }

    fn zero_fence_accepts_forged_result(at: &str, mutate: impl FnOnce(&mut DomainResult)) -> bool {
        let (_workspace, execution, reads) = rejected_hidden_v13_view_execution(at);
        let cancellation = CancellationToken::new();
        let mut result =
            crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()
                .execute(&execution, cancellation.clone())
                .expect("typed input error is a DomainResult");
        mutate(&mut result);
        let accepted = execution.publish(Ok(result), &cancellation).is_ok();
        assert_eq!(
            reads.take(),
            Vec::new(),
            "forged rejected-path result must not read a source corpus"
        );
        accepted
    }

    #[test]
    pub(crate) fn review_invalid_logical_address_reaches_typed_bad_value_result() {
        let result = execute_rejected_hidden_v13_view_address("not-qualified");

        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["code"], "bad_value");
    }

    #[test]
    pub(crate) fn valid_unknown_source_reaches_typed_provider_unavailable_without_scanning() {
        let result = execute_rejected_hidden_v13_view_address("missing:Catalog.Items");

        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["code"], "provider_unavailable");
    }

    #[test]
    pub(crate) fn zero_fence_view_rejection_accepts_only_the_exact_canonical_envelope() {
        let mut accepted = Vec::new();
        for at in ["not-qualified", "missing:Catalog.Items"] {
            macro_rules! expect_rejected {
                ($label:literal, $mutate:expr) => {
                    if zero_fence_accepts_forged_result(at, $mutate) {
                        accepted.push(format!("{at}:{}", $label));
                    }
                };
            }
            expect_rejected!("data", |result: &mut DomainResult| {
                result.data = Some(serde_json::json!({"unfenced": "payload"}));
            });
            expect_rejected!("rev", |result: &mut DomainResult| {
                result.rev = Some("unfenced-revision".to_string());
            });
            expect_rejected!("cursor", |result: &mut DomainResult| {
                result.cursor = Some("unfenced-cursor".to_string());
            });
            expect_rejected!("changed", |result: &mut DomainResult| {
                result.changed.push(serde_json::json!({"path": "unfenced"}));
            });
            expect_rejected!("warnings", |result: &mut DomainResult| {
                result.warnings.push(serde_json::json!({"raw": "provider"}));
            });
            expect_rejected!("artifacts", |result: &mut DomainResult| {
                result
                    .artifacts
                    .push(serde_json::json!({"path": "unfenced"}));
            });
            expect_rejected!("next", |result: &mut DomainResult| {
                result.next.push(serde_json::json!({"op": "unfenced"}));
            });
            expect_rejected!("extra diagnostic", |result: &mut DomainResult| {
                result
                    .diagnostics
                    .push(serde_json::json!({"code": "bad_value", "message": "extra"}));
            });
            expect_rejected!("diagnostic key", |result: &mut DomainResult| {
                result.diagnostics[0]["raw"] = serde_json::json!("provider");
            });
            expect_rejected!("at", |result: &mut DomainResult| {
                result.at = Some("other:Catalog.Items".to_string());
            });
            expect_rejected!("summary", |result: &mut DomainResult| {
                result.summary = "summary disagrees with diagnostic".to_string();
            });
            expect_rejected!("diagnostic message", |result: &mut DomainResult| {
                result.diagnostics[0]["message"] =
                    serde_json::json!("diagnostic disagrees with summary");
            });
            expect_rejected!(
                "consistent alternate message",
                |result: &mut DomainResult| {
                    result.summary = "alternate but internally consistent".to_string();
                    result.diagnostics[0]["message"] =
                        serde_json::json!("alternate but internally consistent");
                }
            );

            let (_workspace, execution, reads) = rejected_hidden_v13_view_execution(at);
            let cancellation = CancellationToken::new();
            if execution
                .publish(
                    Err(InvocationFailure::new("cancelled", "forged failure")),
                    &cancellation,
                )
                .is_ok()
            {
                accepted.push(format!("{at}:InvocationFailure"));
            }
            assert_eq!(reads.take(), Vec::new());

            let (_workspace, execution, reads) = rejected_hidden_v13_view_execution(at);
            if execution
                .publish(Ok(DomainResult::success("forged success")), &cancellation)
                .is_ok()
            {
                accepted.push(format!("{at}:success"));
            }
            assert_eq!(reads.take(), Vec::new());
        }
        assert!(
            accepted.is_empty(),
            "zero-fence publication accepted forbidden envelopes: {accepted:?}"
        );
    }

    /// An inline execution that outlives the handoff window and finishes
    /// only once it is released.
    struct ReleasedHandoffService {
        executions: Arc<AtomicUsize>,
        started: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl CanonicalInvocationService for ReleasedHandoffService {
        fn prepare(
            &self,
            _invocation: &ActorBoundInvocation,
        ) -> Result<ExecutionClass, Box<DomainResult>> {
            Ok(ExecutionClass::InlineCandidate)
        }

        fn execute(
            &self,
            _invocation: &ActorBoundExecution,
            _cancellation: CancellationToken,
        ) -> Result<DomainResult, InvocationFailure> {
            self.executions.fetch_add(1, Ordering::SeqCst);
            self.started.send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
            Ok(DomainResult::success("find completed after handoff"))
        }
    }

    /// An unfinished attempt outlives the seven-second Task handoff: the
    /// daemon hands it off as a Task at its cutoff, the same attempt keeps
    /// running and completes exactly once.
    pub(crate) fn assert_unfinished_work_survives_handoff_and_completes_once() {
        let workspace = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(workspace.path()).unwrap();
        ensure_platform_xml_workspace(&root.to_string_lossy());
        let clock = Arc::new(ManualInvocationClock::new(Instant::now()));
        let executions = Arc::new(AtomicUsize::new(0));
        let (started, started_wait) = mpsc::channel();
        let (release, release_wait) = mpsc::channel();
        let service: Arc<dyn CanonicalInvocationService> = Arc::new(ReleasedHandoffService {
            executions: Arc::clone(&executions),
            started,
            release: Mutex::new(release_wait),
        });
        let canonical = Arc::new(V5CanonicalInvocationRuntime::new(
            Arc::clone(&service),
            Arc::clone(&clock) as Arc<dyn Clock>,
        ));
        let daemon = LiveV5Daemon::over(canonical, service);
        let owner = daemon.owner();
        let request = InvocationRequest::new(
            ToolIdentity::Run,
            serde_json::json!({"op": "test.long-work", "args": {}}),
            root.to_string_lossy(),
            7_000,
        )
        .unwrap();

        let task_id = thread::scope(|scope| {
            let submission = scope.spawn(|| daemon.submit(&owner, &request));
            started_wait
                .recv_timeout(Duration::from_secs(10))
                .expect("the inline execution starts under the handoff window");
            // The seventh second by the daemon's own clock: the attempt is not
            // done, so the daemon hands it off instead of waiting for it.
            clock.advance(INVOCATION_HANDOFF_WINDOW);
            match submission.join().expect("submission thread") {
                V5Submission::Task(task_id) => task_id,
                other => panic!("unfinished inline work did not hand off as Task: {other:?}"),
            }
        });
        release.send(()).unwrap();
        let terminal = daemon.wait_terminal(&owner, task_id, Duration::from_secs(10));
        assert_eq!(terminal.status(), InvocationStatus::Completed);
        assert_eq!(
            terminal.completed_result().unwrap().summary,
            "find completed after handoff"
        );
        assert_eq!(executions.load(Ordering::SeqCst), 1);
        daemon.finish(owner);
    }

    struct BlockingService {
        entered: mpsc::Sender<()>,
    }

    struct RejectingAfterActorAdmissionService;

    struct NonCooperativeActorService {
        entered: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    struct CountingPrepareService {
        preparations: Arc<AtomicUsize>,
    }

    struct SharedDeliveryService {
        key: DeliveryWorkKey,
        ready_root: std::path::PathBuf,
        producers: Arc<AtomicUsize>,
        producer_entered: mpsc::Sender<()>,
        joined: mpsc::Sender<usize>,
        release: Arc<(Mutex<bool>, Condvar)>,
    }

    #[derive(Clone)]
    enum LongCapabilityKind {
        Index,
        Provider(ProviderHostKey),
        Runtime { lease_id: String },
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum JoinedCapabilityIdentity {
        Index(IndexWorkIdentity),
        Provider(ProviderHostKey),
        Runtime(String),
    }

    struct SharedLongCapabilityService {
        kind: LongCapabilityKind,
        producers: Arc<AtomicUsize>,
        producer_entered: mpsc::Sender<()>,
        joined: mpsc::Sender<JoinedCapabilityIdentity>,
        release: Arc<(Mutex<bool>, Condvar)>,
    }

    struct GenerationChangingIndexService {
        producers: Arc<AtomicUsize>,
        producer_entered: mpsc::Sender<()>,
        joined: mpsc::Sender<IndexWorkIdentity>,
        release: Arc<(Mutex<bool>, Condvar)>,
        first_execution: AtomicBool,
        mark_dirty: Mutex<mpsc::Receiver<()>>,
        dirty_done: mpsc::Sender<()>,
    }

    impl CanonicalInvocationService for BlockingService {
        fn prepare(
            &self,
            _invocation: &ActorBoundInvocation,
        ) -> Result<ExecutionClass, Box<DomainResult>> {
            Ok(ExecutionClass::KnownLong(KnownLongReason::ExternalProcess))
        }

        fn execute(
            &self,
            _invocation: &ActorBoundExecution,
            cancellation: CancellationToken,
        ) -> Result<DomainResult, InvocationFailure> {
            self.entered.send(()).unwrap();
            while !cancellation.is_cancelled() {
                std::thread::yield_now();
            }
            Err(InvocationFailure::new("cancelled", "test cancellation"))
        }
    }

    impl CanonicalInvocationService for RejectingAfterActorAdmissionService {
        fn prepare(
            &self,
            invocation: &ActorBoundInvocation,
        ) -> Result<ExecutionClass, Box<DomainResult>> {
            assert_eq!(invocation.tool(), ToolIdentity::Run);
            Err(Box::new(failed_domain_result(
                "test rejection after actor admission",
            )))
        }

        fn execute(
            &self,
            _invocation: &ActorBoundExecution,
            _cancellation: CancellationToken,
        ) -> Result<DomainResult, InvocationFailure> {
            panic!("rejected preparation must not reach execution")
        }
    }

    impl CanonicalInvocationService for NonCooperativeActorService {
        fn prepare(
            &self,
            _invocation: &ActorBoundInvocation,
        ) -> Result<ExecutionClass, Box<DomainResult>> {
            Ok(ExecutionClass::KnownLong(KnownLongReason::ExternalProcess))
        }

        fn execute(
            &self,
            _invocation: &ActorBoundExecution,
            _cancellation: CancellationToken,
        ) -> Result<DomainResult, InvocationFailure> {
            self.entered.send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
            Ok(DomainResult::success(
                "non-cooperative staged actor result must stay hidden",
            ))
        }
    }

    impl CanonicalInvocationService for CountingPrepareService {
        fn prepare(
            &self,
            _invocation: &ActorBoundInvocation,
        ) -> Result<ExecutionClass, Box<DomainResult>> {
            self.preparations.fetch_add(1, Ordering::SeqCst);
            Ok(ExecutionClass::InlineCandidate)
        }

        fn execute(
            &self,
            _invocation: &ActorBoundExecution,
            _cancellation: CancellationToken,
        ) -> Result<DomainResult, InvocationFailure> {
            Ok(DomainResult::success("counted execution"))
        }
    }

    #[test]
    fn v5_actor_binding_does_not_enter_domain_prepare_before_explicit_begin() {
        let workspace = tempfile::tempdir().expect("temporary workspace");
        let source = workspace.path().join("src");
        std::fs::create_dir_all(&source).expect("create source root");
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .expect("write workspace descriptor");
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
        )
        .expect("write configuration root");
        let preparations = Arc::new(AtomicUsize::new(0));
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(CountingPrepareService {
                preparations: Arc::clone(&preparations),
            }),
            Arc::new(TokioClock),
        );
        let request = InvocationRequest::new(
            ToolIdentity::View,
            serde_json::json!({"at": "main:Configuration"}),
            std::fs::canonicalize(workspace.path())
                .expect("canonical workspace")
                .to_string_lossy(),
            7_000,
        )
        .expect("valid request");

        let bound = runtime.bind(request).expect("bind workspace actor");
        assert_eq!(preparations.load(Ordering::SeqCst), 0);
        let prepared = bound.prepare().expect("enter domain prepare explicitly");
        assert_eq!(preparations.load(Ordering::SeqCst), 1);
        assert_eq!(prepared.execution_class(), &ExecutionClass::InlineCandidate);
    }

    #[test]
    fn v5_view_without_at_prepares_root_inspection_without_source_actor_admission() {
        let workspace = tempfile::tempdir().expect("temporary empty workspace");
        let preparations = Arc::new(AtomicUsize::new(0));
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(CountingPrepareService {
                preparations: Arc::clone(&preparations),
            }),
            Arc::new(TokioClock),
        );
        let request = InvocationRequest::new(
            ToolIdentity::View,
            serde_json::json!({}),
            std::fs::canonicalize(workspace.path())
                .expect("canonical workspace")
                .to_string_lossy(),
            7_000,
        )
        .expect("valid bootstrap request");

        let bound = runtime.bind(request).expect("prepare root inspection");
        assert!(matches!(
            bound.0,
            ActorBoundCanonicalInvocationState::WorkspaceInspection { .. }
        ));
        assert_eq!(runtime.workspace_actors.entry_len_for_test().unwrap(), 0);
        assert_eq!(preparations.load(Ordering::SeqCst), 0);
        let prepared = bound.prepare().expect("prepare actor-free root operation");
        assert_eq!(prepared.execution_class(), &ExecutionClass::InlineCandidate);
        let result = prepared
            .execute(CancellationToken::new())
            .expect("execute root inspection");

        assert!(result.ok, "v5 bootstrap was misclassified: {result:?}");
        assert_eq!(result.data.as_ref().unwrap()["config"]["state"], "missing");
        assert_eq!(preparations.load(Ordering::SeqCst), 0);
        assert_eq!(runtime.workspace_actors.entry_len_for_test().unwrap(), 0);
    }

    #[test]
    fn v5_run_dictionary_returns_directly_before_workspace_actor_admission() {
        let workspace = tempfile::tempdir().expect("temporary empty workspace");
        let preparations = Arc::new(AtomicUsize::new(0));
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(CountingPrepareService {
                preparations: Arc::clone(&preparations),
            }),
            Arc::new(TokioClock),
        );
        let request = InvocationRequest::new(
            ToolIdentity::Run,
            serde_json::json!({}),
            std::fs::canonicalize(workspace.path())
                .expect("canonical workspace")
                .to_string_lossy(),
            7_000,
        )
        .expect("valid run dictionary request");

        let result = match runtime.bind(request) {
            Err(V5CanonicalPrepareError::Direct(result)) => result,
            Ok(_) => panic!("run dictionary must not enter actor admission"),
            Err(other) => panic!("run dictionary returned infrastructure failure: {other:?}"),
        };

        assert!(result.ok, "v5 run dictionary was misclassified: {result:?}");
        let catalog = crate::application::v13::tool_catalog::catalog_for(SurfaceRelease::V13)
            .expect("canonical catalog");
        let operations = result.data.as_ref().unwrap()["operations"]
            .as_array()
            .expect("published operations");
        let mut published = std::collections::BTreeMap::new();
        for operation in operations {
            let name = operation["op"].as_str().expect("operation name");
            assert!(
                published.insert(name, operation).is_none(),
                "duplicate operation {name}"
            );
        }
        // Only available operations are listed; an unavailable one keeps
        // its refusal on call.
        let available = catalog
            .run_dictionary
            .iter()
            .filter(|operation| operation.implemented)
            .collect::<Vec<_>>();
        assert_eq!(published.len(), available.len());
        for operation in available {
            let name = operation.name();
            let entry = published.get(name).expect("catalog operation is published");
            assert_eq!(entry["description"], operation.description(), "{name}");
            assert_eq!(
                entry["argsSchema"],
                serde_json::json!(operation.args_schema()),
                "{name}"
            );
            assert_eq!(entry["execution"], operation.execution(), "{name}");
            assert_eq!(
                entry["effects"],
                serde_json::json!(operation.effects()),
                "{name}"
            );
            assert!(entry.get("implemented").is_none(), "{name}");
            let preview_apply = operation.execution() == "previewApply";
            assert_eq!(entry["dryRunRequired"], preview_apply, "{name}");
            assert_eq!(entry["previewRequired"], false, "{name}");
            assert!(entry.get("ifRevRequiredOnApply").is_none(), "{name}");
        }
        assert_eq!(preparations.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn v5_run_rejects_if_rev_before_workspace_or_provider_admission() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("v8project.yaml"), "not: [valid yaml").unwrap();
        let preparations = Arc::new(AtomicUsize::new(0));
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(CountingPrepareService {
                preparations: Arc::clone(&preparations),
            }),
            Arc::new(TokioClock),
        );
        let catalog =
            crate::application::v13::tool_catalog::catalog_for(SurfaceRelease::V13).unwrap();
        let contract = catalog
            .tools
            .iter()
            .find(|tool| tool.name == "run")
            .unwrap();
        assert!(contract.input_schema["properties"].get("ifRev").is_none());
        assert_eq!(contract.input_schema["additionalProperties"], false);
        for operation in &catalog.run_dictionary {
            let request = InvocationRequest::new(
                ToolIdentity::Run,
                serde_json::json!({
                    "op": operation.name(), "args": {}, "dryRun": false, "ifRev": "old-preview"
                }),
                workspace.path().display().to_string(),
                7_000,
            )
            .unwrap();
            let result = match runtime.bind(request) {
                Err(V5CanonicalPrepareError::Rejected(result)) => result,
                Ok(_) => panic!("{} accepted the removed ifRev argument", operation.name()),
                Err(other) => panic!("request reached workspace admission: {other:?}"),
            };
            assert_eq!(result.diagnostics[0]["code"], "bad_value", "{result:?}");
            assert!(result.rev.is_none());
        }
        assert_eq!(preparations.load(Ordering::SeqCst), 0);
        assert_eq!(runtime.workspace_actors.entry_len_for_test().unwrap(), 0);
        assert!(!workspace.path().join(".build").exists());
    }

    #[test]
    fn v5_infobase_exports_prepare_before_source_admission_and_run_without_a_revision_gate() {
        let workspace = tempfile::tempdir().expect("temporary infobase workspace");
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "infobase:\n",
                "  connection:\n",
                "    type: file\n",
                "    path: .build/infobase\n",
            ),
        )
        .expect("write infobase-only workspace descriptor");
        let preparations = Arc::new(AtomicUsize::new(0));
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(CountingPrepareService {
                preparations: Arc::clone(&preparations),
            }),
            Arc::new(TokioClock),
        );
        let request = |op: &str, args: serde_json::Value, dry_run: bool| {
            let arguments = serde_json::json!({
                "op": op,
                "args": args,
                "dryRun": dry_run,
            });
            InvocationRequest::new(
                ToolIdentity::Run,
                arguments,
                std::fs::canonicalize(workspace.path())
                    .expect("canonical workspace")
                    .to_string_lossy(),
                7_000,
            )
            .expect("valid infobase export request")
        };

        for (op, args) in [
            (
                "download",
                serde_json::json!({"state": "working", "output": "dist/main.cf"}),
            ),
            (
                "infobase.dump",
                serde_json::json!({"output": "dist/main.dt"}),
            ),
        ] {
            let preview = runtime
                .bind(request(op, args.clone(), true))
                .expect("preview binds without a PlatformXml source set")
                .prepare()
                .expect("preview preparation");
            assert_eq!(
                preview.execution_class(),
                &ExecutionClass::KnownLong(KnownLongReason::ExternalProcess)
            );

            let apply = runtime
                .bind(request(op, args, false))
                .expect("execution binds without a prior preview or revision")
                .prepare()
                .expect("apply preparation");
            assert_eq!(
                apply.execution_class(),
                &ExecutionClass::KnownLong(KnownLongReason::ExternalProcess)
            );
        }

        assert_eq!(
            preparations.load(Ordering::SeqCst),
            0,
            "infobase exports must not enter the PlatformXml service"
        );
    }

    #[test]
    fn v5_client_run_binds_before_source_admission_without_a_revision_gate() {
        // Вариант А развилки A-1 (#871): терминальная операция запускается
        // сразу, превью необязательно, `ifRev` не принимается.
        let workspace = tempfile::tempdir().expect("temporary infobase workspace");
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\ninfobase:\n  connection: \"File=.build/infobase\"\n",
        )
        .expect("write infobase-only workspace descriptor");
        let preparations = Arc::new(AtomicUsize::new(0));
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(CountingPrepareService {
                preparations: Arc::clone(&preparations),
            }),
            Arc::new(TokioClock),
        );
        let request = |arguments: serde_json::Value| {
            InvocationRequest::new(
                ToolIdentity::Run,
                arguments,
                std::fs::canonicalize(workspace.path())
                    .expect("canonical workspace")
                    .to_string_lossy(),
                7_000,
            )
            .expect("valid launch request")
        };

        for arguments in [
            serde_json::json!({"op": "launch", "args": {"clientMode": "designer"}, "dryRun": true}),
            serde_json::json!({"op": "launch", "args": {"clientMode": "thin"}}),
        ] {
            let bound = runtime
                .bind(request(arguments))
                .expect("launch binds without a PlatformXml source set");
            assert!(matches!(
                bound.0,
                ActorBoundCanonicalInvocationState::ClientRun { .. }
            ));
            let prepared = bound.prepare().expect("launch preparation");
            assert_eq!(
                prepared.execution_class(),
                &ExecutionClass::KnownLong(KnownLongReason::ExternalProcess)
            );
        }

        let fenced = match runtime.bind(request(serde_json::json!({
            "op": "launch",
            "args": {"clientMode": "thin"},
            "ifRev": "unica-infobase-export-sha256-v1:test",
        }))) {
            Err(V5CanonicalPrepareError::Rejected(result)) => result,
            Ok(_) => panic!("a terminal operation accepted ifRev"),
            Err(other) => panic!("ifRev on launch returned infrastructure failure: {other:?}"),
        };
        assert_eq!(fenced.diagnostics[0]["code"], "bad_value");
        assert!(runtime.workspace_actors.entry_len_for_test().unwrap() == 0);
        assert_eq!(
            preparations.load(Ordering::SeqCst),
            0,
            "launch must not enter the PlatformXml service"
        );
    }

    fn assert_development_cycle_admission(selected: &str) {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("v8project.yaml"),
            "format: DESIGNER\ninfobase:\n  connection: 'File=base'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n").unwrap();
        std::fs::create_dir(workspace.path().join("src")).unwrap();
        std::fs::write(workspace.path().join("main.cf"), b"fixture").unwrap();
        let preparations = Arc::new(AtomicUsize::new(0));
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(CountingPrepareService {
                preparations: Arc::clone(&preparations),
            }),
            Arc::new(TokioClock),
        );
        for (op, args) in [
            ("infobase.create", serde_json::json!({})),
            ("push", serde_json::json!({"sourceSet":"main","force":true})),
            ("pull", serde_json::json!({"sourceSet":"main","force":true})),
        ] {
            if op != selected {
                continue;
            }
            for dry_run in [false, true] {
                let arguments = serde_json::json!({"op":op,"args":args,"dryRun":dry_run});
                let request = InvocationRequest::new(
                    ToolIdentity::Run,
                    arguments,
                    workspace.path().display().to_string(),
                    7000,
                )
                .unwrap();
                let bound = runtime
                    .bind(request)
                    .unwrap_or_else(|e| panic!("{op}: {e:?}"));
                let prepared = bound.prepare().unwrap_or_else(|e| panic!("{op}: {e:?}"));
                assert_eq!(
                    prepared.execution_class(),
                    &ExecutionClass::KnownLong(KnownLongReason::ExternalProcess)
                );
            }
        }
        assert_eq!(preparations.load(Ordering::SeqCst), 0);
    }

    /// Адаптер раннера 0.15 не загружает CF/CFE без применения и не исполняет `apply`
    /// и `reset` (#1246): операции остаются в словаре недоступными, и вызов получает
    /// типизированный отказ при bind — до рабочего пространства, PlatformXml и
    /// раннера.
    #[test]
    fn v5_unavailable_runner_operations_are_refused_at_bind_without_a_runner() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("v8project.yaml"),
            "format: DESIGNER\ninfobase:\n  connection: 'File=base'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n").unwrap();
        std::fs::write(workspace.path().join("main.cf"), b"fixture").unwrap();
        let preparations = Arc::new(AtomicUsize::new(0));
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(CountingPrepareService {
                preparations: Arc::clone(&preparations),
            }),
            Arc::new(TokioClock),
        );
        for (op, args) in [
            ("upload", serde_json::json!({"input":"main.cf"})),
            ("apply", serde_json::json!({})),
            ("reset", serde_json::json!({"force":true})),
        ] {
            for dry_run in [false, true] {
                let request = InvocationRequest::new(
                    ToolIdentity::Run,
                    serde_json::json!({"op":op,"args":args,"dryRun":dry_run}),
                    workspace.path().display().to_string(),
                    7000,
                )
                .unwrap();
                let refused = match runtime.bind(request) {
                    Err(V5CanonicalPrepareError::Direct(result)) => result,
                    Ok(_) => panic!("{op} reached a runner adapter"),
                    Err(other) => panic!("{op}: {other:?}"),
                };
                assert!(!refused.ok);
                assert_eq!(refused.diagnostics[0]["code"], "unsupported_operation");
                assert!(
                    refused.summary.contains("issues/1246"),
                    "{op}: {}",
                    refused.summary
                );
            }
        }
        assert_eq!(preparations.load(Ordering::SeqCst), 0);
        assert!(runtime.workspace_actors.entry_len_for_test().unwrap() == 0);
    }

    #[test]
    fn v5_infobase_create_prepares_before_source_admission_and_runs_without_a_revision_gate() {
        assert_development_cycle_admission("infobase.create");
    }

    #[test]
    fn v5_source_import_prepares_before_source_admission_and_runs_without_a_revision_gate() {
        assert_development_cycle_admission("push");
    }

    #[test]
    fn v5_source_export_prepares_before_source_admission_and_runs_without_a_revision_gate() {
        assert_development_cycle_admission("pull");
    }

    #[test]
    fn v5_artifact_build_prepares_before_source_admission_and_runs_without_a_revision_gate() {
        let workspace = tempfile::tempdir().expect("temporary make workspace");
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\ninfobase:\n  connection: \"File=.build/infobase\"\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .expect("write a workspace descriptor with one source set");
        let preparations = Arc::new(AtomicUsize::new(0));
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(CountingPrepareService {
                preparations: Arc::clone(&preparations),
            }),
            Arc::new(TokioClock),
        );
        let request = |arguments: serde_json::Value| {
            InvocationRequest::new(
                ToolIdentity::Run,
                arguments,
                std::fs::canonicalize(workspace.path())
                    .expect("canonical workspace")
                    .to_string_lossy(),
                7_000,
            )
            .expect("valid make request")
        };

        for arguments in [
            serde_json::json!({"op": "make", "args": {"output": "dist/main.cf"}, "dryRun": true}),
            serde_json::json!({"op": "make", "args": {"output": "dist/main.cf", "sourceSet": "main"}, "dryRun": false}),
        ] {
            let bound = runtime
                .bind(request(arguments))
                .expect("make binds without a PlatformXml source set");
            assert!(matches!(
                bound.0,
                ActorBoundCanonicalInvocationState::ArtifactBuild { .. }
            ));
            let prepared = bound.prepare().expect("make preparation");
            assert_eq!(
                prepared.execution_class(),
                &ExecutionClass::KnownLong(KnownLongReason::ExternalProcess)
            );
        }

        let external = match runtime.bind(request(serde_json::json!({
            "op": "make",
            "args": {"output": "dist/report.epf"},
            "dryRun": true,
        }))) {
            Err(V5CanonicalPrepareError::Rejected(result)) => result,
            Ok(_) => panic!("an .epf output without its external source set was accepted"),
            Err(other) => {
                panic!(".epf on make returned infrastructure failure: {other:?}")
            }
        };
        // EPF/ERF build only from a named external source set (#1866).
        assert_eq!(external.diagnostics[0]["code"], "bad_value");
        assert!(
            external.diagnostics[0]["message"]
                .as_str()
                .is_some_and(|message| message.contains("sourceSet")),
            "{external:?}"
        );
        assert_eq!(
            preparations.load(Ordering::SeqCst),
            0,
            "make must not enter the PlatformXml service"
        );
    }

    #[test]
    fn v5_documentation_answers_before_source_admission_without_an_actor_lease() {
        // Каталог без `v8project.yaml` и без корней 1С — тот, из которого
        // новичок и спрашивает, как рабочее пространство завести.
        let workspace = tempfile::tempdir().expect("temporary bare workspace");
        std::fs::write(workspace.path().join("README.md"), "not a 1C workspace\n")
            .expect("write a file that is not a 1C source root");
        let preparations = Arc::new(AtomicUsize::new(0));
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(CountingPrepareService {
                preparations: Arc::clone(&preparations),
            }),
            Arc::new(TokioClock),
        );
        let request = |arguments: serde_json::Value| {
            InvocationRequest::new(
                ToolIdentity::Docs,
                arguments,
                std::fs::canonicalize(workspace.path())
                    .expect("canonical workspace")
                    .to_string_lossy(),
                7_000,
            )
            .expect("valid docs request")
        };

        let prepared = runtime
            .bind(request(serde_json::json!({
                "query": "как завести рабочее пространство",
                "source": "configuration-documentation"
            })))
            .expect("docs binds without a PlatformXml source set")
            .prepare()
            .expect("docs preparation");
        // Справка не длинная по построению: локальное попадание отвечает
        // сразу, а сетевое уходит в Task на общем cutoff.
        assert_eq!(prepared.execution_class(), &ExecutionClass::InlineCandidate);

        // Единственный свод, которому рабочее пространство нужно, отвечает
        // своим типизированным отказом, а не общим отказом допуска.
        let result = prepared
            .execute(CancellationToken::new())
            .expect("docs executes without an actor");
        assert!(!result.ok, "docs must refuse the workspace corpus honestly");
        assert_eq!(result.diagnostics[0]["code"], "unsupported_source");
        assert_eq!(
            preparations.load(Ordering::SeqCst),
            0,
            "docs must not enter the PlatformXml service"
        );
    }

    impl CanonicalInvocationService for SharedDeliveryService {
        fn prepare(
            &self,
            _invocation: &ActorBoundInvocation,
        ) -> Result<ExecutionClass, Box<DomainResult>> {
            Ok(ExecutionClass::KnownLong(KnownLongReason::MissingEngine))
        }

        fn execute(
            &self,
            invocation: &ActorBoundExecution,
            _cancellation: CancellationToken,
        ) -> Result<DomainResult, InvocationFailure> {
            let desk = invocation.delivery_work();
            let key = self.key.clone();
            let producers = Arc::clone(&self.producers);
            let producer_entered = self.producer_entered.clone();
            let release = Arc::clone(&self.release);
            let ready_root = self.ready_root.clone();
            let lease = desk.join(self.key.clone(), move |_| {
                producers.fetch_add(1, Ordering::SeqCst);
                producer_entered.send(()).expect("producer observation");
                let (released, wake) = &*release;
                let mut released = released.lock().expect("delivery release");
                while !*released {
                    released = wake.wait(released).expect("delivery release wait");
                }
                ArtifactReady::new(key, ready_root)
            });
            self.joined
                .send(desk as *const unica_artifact_delivery::desk::DeliveryDesk as usize)
                .expect("join observation");
            lease
                .wait()
                .map_err(|_| InvocationFailure::new("delivery_failed", "delivery failed"))?;
            Ok(DomainResult::success("shared daemon delivery ready"))
        }
    }

    impl CanonicalInvocationService for SharedLongCapabilityService {
        fn prepare(
            &self,
            _invocation: &ActorBoundInvocation,
        ) -> Result<ExecutionClass, Box<DomainResult>> {
            let reason = match self.kind {
                LongCapabilityKind::Index => KnownLongReason::ColdIndex,
                LongCapabilityKind::Provider(_) => KnownLongReason::ProviderStartup,
                LongCapabilityKind::Runtime { .. } => KnownLongReason::ExternalProcess,
            };
            Ok(ExecutionClass::KnownLong(reason))
        }

        fn execute(
            &self,
            invocation: &ActorBoundExecution,
            _cancellation: CancellationToken,
        ) -> Result<DomainResult, InvocationFailure> {
            fn make_work<E>(
                service: &SharedLongCapabilityService,
            ) -> impl FnOnce(unica_work_control::shared_work::SharedWorkProducer) -> Result<(), E>
                   + Send
                   + 'static {
                let producers = Arc::clone(&service.producers);
                let producer_entered = service.producer_entered.clone();
                let release = Arc::clone(&service.release);
                move |_| {
                    producers.fetch_add(1, Ordering::SeqCst);
                    producer_entered.send(()).expect("producer observation");
                    let (released, wake) = &*release;
                    let mut released = released.lock().expect("long-work release");
                    while !*released {
                        released = wake.wait(released).expect("long-work release wait");
                    }
                    Ok(())
                }
            }
            fn settle<K: unica_work_control::shared_work::WorkKey, E>(
                joined: &mpsc::Sender<JoinedCapabilityIdentity>,
                key: JoinedCapabilityIdentity,
                lease: unica_work_control::shared_work::SharedWorkLease<K, (), E>,
            ) -> Result<(), InvocationFailure> {
                joined.send(key).expect("join observation");
                lease
                    .wait()
                    .map(|_| ())
                    .map_err(|_| InvocationFailure::new("long_work_failed", "work unavailable"))
            }
            match &self.kind {
                LongCapabilityKind::Index => {
                    let (key, lease) = invocation
                        .join_index_work("rlm", "bsl-1", "test-build-1", make_work(self))
                        .map_err(|_| InvocationFailure::new("index_failed", "index unavailable"))?;
                    settle(&self.joined, JoinedCapabilityIdentity::Index(key), lease)
                }
                LongCapabilityKind::Provider(key) => {
                    let lease = invocation.join_provider_host(key.clone(), make_work(self));
                    settle(
                        &self.joined,
                        JoinedCapabilityIdentity::Provider(key.clone()),
                        lease,
                    )
                }
                LongCapabilityKind::Runtime { lease_id } => {
                    let lease = invocation
                        .join_runtime_resource(lease_id, make_work(self))
                        .map_err(|_| {
                            InvocationFailure::new("runtime_failed", "runtime unavailable")
                        })?;
                    settle(
                        &self.joined,
                        JoinedCapabilityIdentity::Runtime(lease_id.clone()),
                        lease,
                    )
                }
            }?;
            let marker = invocation
                .read_relative_file(std::path::Path::new("marker.txt"), 64)
                .map_err(|_| InvocationFailure::new("root_failed", "root unavailable"))?;
            Ok(DomainResult::success(
                String::from_utf8(marker).expect("marker is utf8"),
            ))
        }
    }

    impl CanonicalInvocationService for GenerationChangingIndexService {
        fn prepare(
            &self,
            _invocation: &ActorBoundInvocation,
        ) -> Result<ExecutionClass, Box<DomainResult>> {
            Ok(ExecutionClass::KnownLong(KnownLongReason::ColdIndex))
        }

        fn execute(
            &self,
            invocation: &ActorBoundExecution,
            _cancellation: CancellationToken,
        ) -> Result<DomainResult, InvocationFailure> {
            let producers = Arc::clone(&self.producers);
            let producer_entered = self.producer_entered.clone();
            let release = Arc::clone(&self.release);
            let (key, lease) = invocation
                .join_index_work(
                    "rlm",
                    "bsl-1",
                    if self.first_execution.load(Ordering::SeqCst) {
                        "test-build-2"
                    } else {
                        "test-build-1"
                    },
                    move |_| {
                        producers.fetch_add(1, Ordering::SeqCst);
                        producer_entered
                            .send(())
                            .expect("index producer observation");
                        let (released, wake) = &*release;
                        let mut released = released.lock().expect("index release");
                        while !*released {
                            released = wake.wait(released).expect("index release wait");
                        }
                        Ok(())
                    },
                )
                .map_err(|_| InvocationFailure::new("index_failed", "index unavailable"))?;
            self.joined.send(key).expect("index join observation");
            if !self.first_execution.swap(true, Ordering::SeqCst) {
                self.mark_dirty
                    .lock()
                    .expect("dirty signal")
                    .recv()
                    .expect("dirty request");
                self.dirty_done.send(()).expect("dirty acknowledgement");
            }
            lease
                .wait()
                .map_err(|_| InvocationFailure::new("index_failed", "index unavailable"))?;
            let marker = invocation
                .read_relative_file(std::path::Path::new("marker.txt"), 64)
                .map_err(|_| InvocationFailure::new("root_failed", "root unavailable"))?;
            Ok(DomainResult::success(
                String::from_utf8(marker).expect("marker is utf8"),
            ))
        }
    }

    type SharedCapabilityFixture = (
        Arc<SharedLongCapabilityService>,
        Arc<AtomicUsize>,
        mpsc::Receiver<()>,
        mpsc::Receiver<JoinedCapabilityIdentity>,
        Arc<(Mutex<bool>, Condvar)>,
    );

    const OWNERSHIP_CONTRACT_WAIT_MS: u64 = 15_000;

    fn shared_capability_service(kind: LongCapabilityKind) -> SharedCapabilityFixture {
        let producers = Arc::new(AtomicUsize::new(0));
        let (producer_entered, producer_wait) = mpsc::channel();
        let (joined, joined_wait) = mpsc::channel();
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        (
            Arc::new(SharedLongCapabilityService {
                kind,
                producers: Arc::clone(&producers),
                producer_entered,
                joined,
                release: Arc::clone(&release),
            }),
            producers,
            producer_wait,
            joined_wait,
            release,
        )
    }

    pub(crate) fn ensure_platform_xml_workspace(workspace_hint: &str) {
        let workspace = std::path::Path::new(workspace_hint);
        let project = workspace.join("v8project.yaml");
        if project.exists() {
            return;
        }
        std::fs::write(
            project,
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: .\n",
        )
        .unwrap();
        std::fs::write(
            workspace.join("Configuration.xml"),
            "<MetaDataObject><Configuration/></MetaDataObject>",
        )
        .unwrap();
    }

    /// A real executable stands in for the pinned runner. Its apply call waits
    /// after entering the mutating step, so the test cancels at a known point.
    /// Compiling it in the test keeps the same path on Unix and Windows.
    pub(crate) fn install_cancellable_create_runner(root: &std::path::Path) {
        install_runner_probe(
            root,
            r##"
use std::{env, fs, io::Write, thread, time::{Duration, Instant}};
fn main() {
    let cwd = env::current_dir().unwrap();
    let dry_run = env::args().any(|arg| arg == "--dry-run");
    let created = cwd.join("created.marker");
    if !dry_run {
        let mut dispatches = fs::OpenOptions::new().create(true).append(true).open(cwd.join("apply-dispatch.log")).unwrap();
        writeln!(dispatches, "apply").unwrap();
        dispatches.sync_all().unwrap();
        fs::write(cwd.join("entered.marker"), "runner entered mutation").unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        while !cwd.join("release.marker").exists() {
            if Instant::now() >= deadline { panic!("runner was never released"); }
            thread::sleep(Duration::from_millis(5));
        }
        fs::write(&created, "provider created infobase").unwrap();
    }
    let status = if !dry_run { "ok" } else if created.exists() { "skipped" } else { "planned" };
    println!(r#"{{"ok":true,"command":"infobase create","duration_ms":0,"data":{{"ok":true,"provider_dispatched":{},"steps":[{{"target":"infobase","action":"create","status":"{}","message":"probe","duration_ms":0}},{{"target":"edt_workspace","action":"import","status":"skipped","message":"probe","duration_ms":0}}],"duration_ms":0}},"warnings":[],"steps":[]}}"#, !dry_run, status);
}
"##,
        );
    }

    /// Compiles `probe` as the pinned v8-runner of a plugin rooted at `root`.
    pub(crate) fn install_runner_probe(root: &std::path::Path, probe: &str) {
        use sha2::{Digest, Sha256};

        for name in ["UNICA_PLUGIN_ROOT", "UNICA_ARTIFACT_CACHE"] {
            assert!(
                std::env::var_os(name).is_none(),
                "the runner fixture must not resolve a binary from {name}"
            );
        }

        let plugin = root.join("plugins/unica");
        let target = crate::infrastructure::platform::current_target_id().unwrap();
        let binary_name = format!("v8-runner{}", std::env::consts::EXE_SUFFIX);
        let relative = format!("bin/{target}/{binary_name}");
        let binary = plugin.join(&relative);
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::create_dir_all(plugin.join("third-party")).unwrap();
        std::fs::create_dir_all(plugin.join("skills")).unwrap();
        let source = root.join("runner-probe.rs");
        std::fs::write(&source, probe).unwrap();
        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let compiled = std::process::Command::new(rustc)
            .arg("--edition=2021")
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .output()
            .expect("compile runner probe");
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let digest = format!("{:x}", Sha256::digest(std::fs::read(&binary).unwrap()));
        std::fs::write(
            plugin.join("third-party/manifest.json"),
            serde_json::json!({
                "sourceManifest": true,
                "tools": [{
                    "name": "v8-runner",
                    "version": RUNNER_VERSION,
                    "binaries": {target: {"binaryPath": relative, "sha256": digest}}
                }]
            })
            .to_string(),
        )
        .unwrap();
    }

    #[test]
    fn mutating_runner_cancel_keeps_the_factual_receipt_over_the_v5_daemon_wire() {
        let root = tempfile::tempdir().unwrap();
        install_cancellable_create_runner(root.path());
        let workspace = root.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\ninfobase:\n  connection: 'File=build/ib'\n",
        )
        .unwrap();
        let workspace = std::fs::canonicalize(workspace).unwrap();
        let daemon = LiveV5Daemon::start(canonical_v13_service());
        let owner = daemon.owner();
        // A separate workspace executes its first public call directly. The
        // provider's internal preflight and factual receipt remain required.
        let direct_workspace = root.path().join("direct-workspace");
        std::fs::create_dir(&direct_workspace).unwrap();
        std::fs::copy(
            workspace.join("v8project.yaml"),
            direct_workspace.join("v8project.yaml"),
        )
        .unwrap();
        std::fs::write(
            direct_workspace.join("release.marker"),
            "allow direct execution",
        )
        .unwrap();
        let direct_request = InvocationRequest::new(
            ToolIdentity::Run,
            serde_json::json!({"op":"infobase.create", "args":{}, "dryRun":false}),
            std::fs::canonicalize(&direct_workspace)
                .unwrap()
                .to_string_lossy(),
            7_000,
        )
        .unwrap();
        let direct_task = daemon.task_id(&owner, &direct_request);
        match daemon.wait_terminal(&owner, direct_task, Duration::from_secs(20)) {
            V5DaemonTaskSnapshot::Completed { result, .. } => {
                assert!(result.ok, "{result:?}");
                assert!(result.rev.is_none());
                assert_eq!(result.data.as_ref().unwrap()["state"], "created");
            }
            other => panic!("execution without a prior preview did not complete: {other:?}"),
        }
        assert!(direct_workspace.join("created.marker").exists());
        let request = |dry_run: bool| {
            let args = serde_json::json!({"op":"infobase.create","args":{},"dryRun":dry_run});
            InvocationRequest::new(ToolIdentity::Run, args, workspace.to_string_lossy(), 7_000)
                .unwrap()
        };
        let preview = daemon.task_id(&owner, &request(true));
        match daemon.wait_terminal(&owner, preview, Duration::from_secs(20)) {
            V5DaemonTaskSnapshot::Completed { result, .. } => {
                assert!(result.ok, "{result:?}");
                assert!(result.rev.is_none());
                let next = result
                    .next
                    .iter()
                    .find(|next| next["tool"] == "unica.run")
                    .unwrap();
                assert_eq!(next["args"]["dryRun"], false);
                assert!(next["args"].get("ifRev").is_none());
            }
            other => panic!("preview did not complete: {other:?}"),
        };

        assert!(!workspace.join("entered.marker").exists());
        let task_id = daemon.task_id(&owner, &request(false));
        let entered = workspace.join("entered.marker");
        let deadline = Instant::now() + Duration::from_secs(20);
        while !entered.exists() {
            assert!(
                Instant::now() < deadline,
                "mutating runner did not start: {:?}",
                daemon.get(&owner, task_id)
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let cancelled = daemon.cancel(&owner, task_id);
        assert!(
            matches!(
                cancelled,
                V5DaemonTaskSnapshot::Working {
                    cancel_requested: true,
                    ..
                }
            ),
            "{cancelled:?}"
        );
        assert!(!workspace.join("created.marker").exists());
        std::fs::write(workspace.join("release.marker"), "finish mutation").unwrap();
        let terminal = daemon.wait_terminal(&owner, task_id, Duration::from_secs(20));
        match terminal {
            V5DaemonTaskSnapshot::Completed {
                cancel_requested: true,
                result,
                ..
            } => {
                assert!(result.ok, "{result:?}");
                assert_eq!(result.data.as_ref().unwrap()["state"], "created");
                assert_eq!(
                    result.data.as_ref().unwrap()["receipt"],
                    "repeated preview reports nothing left to create"
                );
            }
            other => panic!("late cancellation hid the provider receipt: {other:?}"),
        }
        assert!(workspace.join("created.marker").exists());
        assert!(matches!(
            daemon.get(&owner, task_id),
            V5DaemonTaskSnapshot::Completed {
                cancel_requested: true,
                ..
            }
        ));
        daemon.finish(owner);
    }

    #[test]
    fn daemon_shared_delivery_releases_request_admission_before_wait_and_shares_across_worktrees() {
        let workspace_parent = tempfile::tempdir().unwrap();
        let roots = (0..2)
            .map(|index| {
                let root = workspace_parent
                    .path()
                    .join(format!("delivery-workspace-{index}"));
                std::fs::create_dir(&root).unwrap();
                let root = std::fs::canonicalize(root).unwrap();
                ensure_platform_xml_workspace(&root.to_string_lossy());
                root
            })
            .collect::<Vec<_>>();
        let producers = Arc::new(AtomicUsize::new(0));
        let (producer_entered, producer_wait) = mpsc::channel();
        let (joined, joined_wait) = mpsc::channel();
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let service = Arc::new(SharedDeliveryService {
            key: DeliveryWorkKey::new(
                "rlm-tools-bsl",
                "1.33.0",
                "darwin-arm64",
                "a".repeat(64),
                DeliveryFormIdentity::Archive,
            )
            .unwrap(),
            ready_root: workspace_parent.path().join("shared-daemon-cache"),
            producers: Arc::clone(&producers),
            producer_entered,
            joined,
            release: Arc::clone(&release),
        });
        let daemon = LiveV5Daemon::start(service);
        let owner = daemon.owner();
        let request = |root: &std::path::Path| {
            InvocationRequest::new(
                ToolIdentity::Run,
                serde_json::json!({"op": "test.long-work", "args": {}}),
                root.to_string_lossy(),
                7_000,
            )
            .unwrap()
        };

        let first = daemon.task_id(&owner, &request(&roots[0]));
        producer_wait
            .recv_timeout(Duration::from_secs(10))
            .expect("first task entered delivery producer");
        let second = daemon.task_id(&owner, &request(&roots[1]));
        let first_desk = joined_wait
            .recv_timeout(Duration::from_secs(10))
            .expect("first task joined delivery");
        let second_desk = joined_wait
            .recv_timeout(Duration::from_secs(10))
            .expect("second task joined delivery");

        assert_eq!(
            first_desk, second_desk,
            "both actors use one daemon registry"
        );
        assert_eq!(
            first_desk,
            Arc::as_ptr(&daemon.runtime().deliveries) as usize,
            "actor bindings retain the daemon-owned DeliveryDesk"
        );
        assert_eq!(
            daemon
                .runtime()
                .workspace_actors
                .live_len_for_test()
                .unwrap(),
            2
        );
        assert_eq!(producers.load(Ordering::SeqCst), 1);
        for task_id in [first, second] {
            assert_eq!(
                daemon.get(&owner, task_id).status(),
                InvocationStatus::Working
            );
        }

        // Both request admissions have already returned durable TaskIds while the only
        // producer is still blocked: joining/waiting consumes task ownership, not request
        // admission, and the two actor-bound executions do not create duplicate delivery.
        let (released, wake) = &*release;
        *released.lock().unwrap() = true;
        wake.notify_all();
        for task_id in [first, second] {
            let terminal = daemon.wait_terminal(
                &owner,
                task_id,
                Duration::from_millis(OWNERSHIP_CONTRACT_WAIT_MS),
            );
            assert_eq!(terminal.status(), InvocationStatus::Completed);
            assert_eq!(
                terminal.completed_result().unwrap().summary,
                "shared daemon delivery ready"
            );
        }
        assert_eq!(producers.load(Ordering::SeqCst), 1);
        daemon.finish(owner);
    }

    #[test]
    fn daemon_long_work_capabilities_handoff_before_wait_and_preserve_exact_ownership() {
        let provider_key = ProviderHostKey::new(
            "bsl-analyzer",
            "aarch64-apple-darwin",
            std::collections::BTreeSet::from([
                "diagnostics".to_string(),
                "navigation".to_string(),
                "search".to_string(),
            ]),
        )
        .unwrap();
        let runtime_authority = tempfile::tempdir().unwrap();
        let runtime_lease = RuntimeJobService::enqueue(
            runtime_authority.path(),
            &RuntimeJobRequest::new(
                RuntimeJobOperation::Test,
                Vec::new(),
                "workspace:test".to_string(),
                None,
            ),
        )
        .unwrap();
        let runtime_service = Arc::new(RuntimeJobService::new(
            runtime_authority.path(),
            Arc::new(unica_adapter_v8_runner::backend::CoordinationOnlyRunner),
        ));

        for kind in [
            LongCapabilityKind::Index,
            LongCapabilityKind::Provider(provider_key),
            LongCapabilityKind::Runtime {
                lease_id: runtime_lease.id,
            },
        ] {
            let workspace_parent = tempfile::tempdir().unwrap();
            let roots = if matches!(kind, LongCapabilityKind::Index) {
                let root = workspace_parent.path().join("index-workspace");
                std::fs::create_dir(&root).unwrap();
                std::fs::write(root.join("marker.txt"), "index").unwrap();
                let root = std::fs::canonicalize(root).unwrap();
                vec![root.clone(), root]
            } else {
                (0..2)
                    .map(|index| {
                        let root = workspace_parent.path().join(format!("workspace-{index}"));
                        std::fs::create_dir(&root).unwrap();
                        std::fs::write(root.join("marker.txt"), format!("root-{index}")).unwrap();
                        std::fs::canonicalize(root).unwrap()
                    })
                    .collect::<Vec<_>>()
            };
            for root in &roots {
                ensure_platform_xml_workspace(&root.to_string_lossy());
            }
            let (service, producers, producer_wait, joined_wait, release) =
                shared_capability_service(kind.clone());
            let canonical = V5CanonicalInvocationRuntime::new(
                Arc::clone(&service) as Arc<dyn CanonicalInvocationService>,
                Arc::new(TokioClock),
            );
            let canonical = if matches!(kind, LongCapabilityKind::Runtime { .. }) {
                canonical.with_runtime_service_for_test(Arc::clone(&runtime_service))
            } else {
                canonical
            };
            let daemon = LiveV5Daemon::over(Arc::new(canonical), service);
            let owner = daemon.owner();
            let request = |root: &std::path::Path| {
                InvocationRequest::new(
                    ToolIdentity::Run,
                    serde_json::json!({"op": "test.long-work", "args": {}}),
                    root.to_string_lossy(),
                    7_000,
                )
                .unwrap()
            };

            let first = daemon.task_id(&owner, &request(&roots[0]));
            producer_wait
                .recv_timeout(Duration::from_secs(10))
                .expect("first task entered long-work producer");
            let second = daemon.task_id(&owner, &request(&roots[1]));
            let first_key = joined_wait
                .recv_timeout(Duration::from_secs(10))
                .expect("first task joined long work");
            let second_key = joined_wait
                .recv_timeout(Duration::from_secs(10))
                .expect("second task joined long work");

            assert_eq!(first_key, second_key);
            assert_eq!(producers.load(Ordering::SeqCst), 1);
            for task_id in [first, second] {
                assert_eq!(
                    daemon.get(&owner, task_id).status(),
                    InvocationStatus::Working
                );
            }
            assert_eq!(
                daemon
                    .runtime()
                    .workspace_actors
                    .live_len_for_test()
                    .unwrap(),
                if matches!(kind, LongCapabilityKind::Index) {
                    1
                } else {
                    2
                }
            );
            let (released, wake) = &*release;
            *released.lock().unwrap() = true;
            wake.notify_all();
            let first_result = daemon
                .wait_terminal(
                    &owner,
                    first,
                    Duration::from_millis(OWNERSHIP_CONTRACT_WAIT_MS),
                )
                .completed_result()
                .cloned()
                .unwrap();
            let second_result = daemon
                .wait_terminal(
                    &owner,
                    second,
                    Duration::from_millis(OWNERSHIP_CONTRACT_WAIT_MS),
                )
                .completed_result()
                .cloned()
                .unwrap();
            if matches!(kind, LongCapabilityKind::Index) {
                assert_eq!(first_result.summary, "index");
                assert_eq!(second_result.summary, "index");
            } else {
                assert_eq!(first_result.summary, "root-0");
                assert_eq!(second_result.summary, "root-1");
            }
            daemon.finish(owner);
        }
    }

    #[test]
    fn daemon_index_work_separates_worktrees_and_build_generations() {
        // Distinct actor identities intentionally cannot join one Index key.
        let workspace_parent = tempfile::tempdir().unwrap();
        let roots = (0..2)
            .map(|index| {
                let root = workspace_parent.path().join(format!("index-root-{index}"));
                std::fs::create_dir(&root).unwrap();
                std::fs::write(root.join("marker.txt"), format!("root-{index}")).unwrap();
                let root = std::fs::canonicalize(root).unwrap();
                ensure_platform_xml_workspace(&root.to_string_lossy());
                root
            })
            .collect::<Vec<_>>();
        let (service, producers, producer_wait, joined_wait, release) =
            shared_capability_service(LongCapabilityKind::Index);
        let daemon = LiveV5Daemon::start(service);
        let owner = daemon.owner();
        let request = |root: &std::path::Path| {
            InvocationRequest::new(
                ToolIdentity::Run,
                serde_json::json!({"op": "test.long-work", "args": {}}),
                root.to_string_lossy(),
                7_000,
            )
            .unwrap()
        };
        let first = daemon.task_id(&owner, &request(&roots[0]));
        producer_wait
            .recv_timeout(Duration::from_secs(10))
            .expect("first index producer");
        let second = daemon.task_id(&owner, &request(&roots[1]));
        producer_wait
            .recv_timeout(Duration::from_secs(10))
            .expect("second index producer");
        let first_key = joined_wait.recv_timeout(Duration::from_secs(10)).unwrap();
        let second_key = joined_wait.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_ne!(first_key, second_key);
        assert_eq!(producers.load(Ordering::SeqCst), 2);
        let (released, wake) = &*release;
        *released.lock().unwrap() = true;
        wake.notify_all();
        assert_eq!(
            daemon
                .wait_terminal(
                    &owner,
                    first,
                    Duration::from_millis(OWNERSHIP_CONTRACT_WAIT_MS)
                )
                .status(),
            InvocationStatus::Completed
        );
        assert_eq!(
            daemon
                .wait_terminal(
                    &owner,
                    second,
                    Duration::from_millis(OWNERSHIP_CONTRACT_WAIT_MS)
                )
                .status(),
            InvocationStatus::Completed
        );
        daemon.finish(owner);

        // A new build generation starts a separate producer; source edits do not
        // invalidate an already admitted build result.
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("marker.txt"), "old").unwrap();
        std::fs::write(
            workspace.path().join("Module.bsl"),
            "Procedure Old()\nEndProcedure",
        )
        .unwrap();
        let root = std::fs::canonicalize(workspace.path()).unwrap();
        ensure_platform_xml_workspace(&root.to_string_lossy());
        let producers = Arc::new(AtomicUsize::new(0));
        let (producer_entered, producer_wait) = mpsc::channel();
        let (joined, joined_wait) = mpsc::channel();
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let (mark_dirty, dirty_request) = mpsc::channel();
        let (dirty_done, dirty_wait) = mpsc::channel();
        let service = Arc::new(GenerationChangingIndexService {
            producers: Arc::clone(&producers),
            producer_entered,
            joined,
            release: Arc::clone(&release),
            first_execution: AtomicBool::new(false),
            mark_dirty: Mutex::new(dirty_request),
            dirty_done,
        });
        let daemon = LiveV5Daemon::start(service);
        let owner = daemon.owner();
        let first = daemon.task_id(&owner, &request(&root));
        producer_wait.recv_timeout(Duration::from_secs(10)).unwrap();
        let old_key = joined_wait.recv_timeout(Duration::from_secs(10)).unwrap();
        std::fs::write(root.join("marker.txt"), "new").unwrap();
        std::fs::write(root.join("Module.bsl"), "Procedure New()\nEndProcedure").unwrap();
        mark_dirty.send(()).unwrap();
        dirty_wait.recv_timeout(Duration::from_secs(10)).unwrap();
        let second = daemon.task_id(&owner, &request(&root));
        producer_wait.recv_timeout(Duration::from_secs(10)).unwrap();
        let new_key = joined_wait.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_ne!(old_key, new_key);
        assert_eq!(producers.load(Ordering::SeqCst), 2);
        let (released, wake) = &*release;
        *released.lock().unwrap() = true;
        wake.notify_all();
        assert_eq!(
            daemon
                .wait_terminal(
                    &owner,
                    first,
                    Duration::from_millis(OWNERSHIP_CONTRACT_WAIT_MS)
                )
                .status(),
            InvocationStatus::Completed
        );
        let second = daemon.wait_terminal(
            &owner,
            second,
            Duration::from_millis(OWNERSHIP_CONTRACT_WAIT_MS),
        );
        assert_eq!(second.status(), InvocationStatus::Completed);
        assert_eq!(second.completed_result().unwrap().summary, "new");
        daemon.finish(owner);
    }

    #[test]
    fn daemon_long_work_rejects_replaced_actor_root_before_reuse_or_publication() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("marker.txt"), "old").unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        ensure_platform_xml_workspace(&root.to_string_lossy());
        let (service, _, producer_wait, joined_wait, release) =
            shared_capability_service(LongCapabilityKind::Index);
        let daemon = LiveV5Daemon::start(service);
        let owner = daemon.owner();
        let request = || {
            InvocationRequest::new(
                ToolIdentity::Run,
                serde_json::json!({"op": "test.long-work", "args": {}}),
                root.to_string_lossy(),
                7_000,
            )
            .unwrap()
        };
        let first = daemon.task_id(&owner, &request());
        producer_wait.recv_timeout(Duration::from_secs(10)).unwrap();
        joined_wait.recv_timeout(Duration::from_secs(10)).unwrap();
        let moved = parent.path().join("workspace-old");
        std::fs::rename(&root, moved).unwrap();
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("marker.txt"), "replacement").unwrap();
        ensure_platform_xml_workspace(&root.to_string_lossy());
        let V5Submission::Direct(rejected) = daemon.submit(&owner, &request()) else {
            panic!("a replaced actor root must be refused before reuse");
        };
        assert!(!rejected.ok, "replaced root was admitted: {rejected:?}");
        let (released, wake) = &*release;
        *released.lock().unwrap() = true;
        wake.notify_all();
        assert_eq!(
            daemon
                .wait_terminal(
                    &owner,
                    first,
                    Duration::from_millis(OWNERSHIP_CONTRACT_WAIT_MS)
                )
                .status(),
            InvocationStatus::Failed
        );
        daemon.finish(owner);
    }

    fn run_long_work_contract_obligation(test_name: &str) {
        // These ownership oracles exercise independent global runtime fixtures.
        // A larger child thread stack still shares the parent test process and
        // can terminate that process on Windows before the aggregate reports a
        // failure. Give each obligation an independent process boundary.
        let output = std::process::Command::new(
            std::env::current_exe().expect("current test executable should resolve"),
        )
        .args([
            "--exact",
            test_name,
            "--nocapture",
            "--test-threads=1",
            "--color",
            "never",
        ])
        // The parent harness has tests that temporarily change its process-wide
        // current directory. Never inherit a fixture cwd that can disappear
        // while this independent test process is starting.
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("long-work contract obligation process should start");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "long-work contract obligation {test_name} failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(
            stdout.contains("running 1 test"),
            "long-work contract obligation {test_name} matched no unique test\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
    }

    #[test]
    fn daemon_exact_long_work_ownership_contract() {
        run_long_work_contract_obligation(
            "infrastructure::daemon::server::actor_capacity_tests::daemon_long_work_capabilities_handoff_before_wait_and_preserve_exact_ownership",
        );
        run_long_work_contract_obligation(
            "infrastructure::daemon::server::actor_capacity_tests::daemon_index_work_separates_worktrees_and_build_generations",
        );
        run_long_work_contract_obligation(
            "infrastructure::daemon::server::actor_capacity_tests::daemon_long_work_rejects_replaced_actor_root_before_reuse_or_publication",
        );
        run_long_work_contract_obligation(
            "infrastructure::runtime_jobs_integration::tests::runtime_resource_tree_lease_contract",
        );
    }

    #[test]
    fn default_actor_registry_admits_sixty_five_profiles_and_retains_exact_live_owners() {
        let workspace_parent = tempfile::tempdir().unwrap();
        let roots = (0..65)
            .map(|index| {
                let root = workspace_parent.path().join(format!("workspace-{index}"));
                std::fs::create_dir(&root).unwrap();
                let root = std::fs::canonicalize(root).unwrap();
                ensure_platform_xml_workspace(&root.to_string_lossy());
                root
            })
            .collect::<Vec<_>>();
        let (entered, _entered_wait) = mpsc::channel();
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(BlockingService { entered }),
            Arc::new(TokioClock),
        );
        let request = |root: &std::path::Path| {
            InvocationRequest::new(
                ToolIdentity::Run,
                serde_json::json!({"op": "test.long-work", "args": {}}),
                root.to_string_lossy(),
                7_000,
            )
            .unwrap()
        };
        let mut bound = Vec::new();
        let mut observed = Vec::new();
        for (index, root) in roots.iter().enumerate() {
            let admitted = runtime.bind(request(root)).unwrap_or_else(|error| {
                panic!(
                    "profile {} was rejected by the default registry: {error:?}",
                    index + 1
                )
            });
            let ActorBoundCanonicalInvocationState::Workspace { invocation, .. } = &admitted.0
            else {
                panic!("the supported profile did not bind its workspace actor");
            };
            observed.push(Arc::downgrade(invocation.actor_for_test()));
            bound.push(admitted);
        }
        assert_eq!(runtime.workspace_actors.live_len_for_test().unwrap(), 65);
        runtime.workspace_actors.release_warm_actors().unwrap();
        assert!(
            observed.iter().all(|actor| actor.upgrade().is_some()),
            "warm cleanup released an actor owned by an admitted invocation"
        );
        let alias = runtime.bind(request(&roots[0].join("."))).unwrap();
        let ActorBoundCanonicalInvocationState::Workspace { invocation, .. } = &alias.0 else {
            panic!("the alias did not bind its workspace actor");
        };
        assert!(Arc::ptr_eq(
            &observed[0].upgrade().unwrap(),
            invocation.actor_for_test()
        ));
        assert_eq!(runtime.workspace_actors.live_len_for_test().unwrap(), 65);

        runtime.workspace_actors.release_warm_actors().unwrap();
        drop(alias);
        drop(bound);
        runtime.workspace_actors.release_warm_actors().unwrap();
        assert!(
            observed.iter().all(|actor| actor.upgrade().is_none()),
            "completed admissions retained hidden actor ownership"
        );
        let _fresh = runtime.bind(request(&roots[0])).unwrap();
        assert_eq!(
            runtime.workspace_actors.entry_len_for_test().unwrap(),
            1,
            "new admission did not prune dead identities"
        );
    }

    fn live_actor_admission_reuses_alias_and_accepts_distinct_roots() {
        let workspace_parent = tempfile::tempdir().unwrap();
        let roots = (0..3)
            .map(|index| {
                let root = workspace_parent.path().join(format!("workspace-{index}"));
                std::fs::create_dir(&root).unwrap();
                let root = std::fs::canonicalize(root).unwrap();
                ensure_platform_xml_workspace(&root.to_string_lossy());
                root
            })
            .collect::<Vec<_>>();
        let (entered, _entered_wait) = mpsc::channel();
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(BlockingService { entered }),
            Arc::new(TokioClock),
        );
        let request = |root: &std::path::Path| {
            InvocationRequest::new(
                ToolIdentity::Run,
                serde_json::json!({"op": "test.long-work", "args": {}}),
                root.to_string_lossy(),
                7_000,
            )
            .unwrap()
        };

        // A bound invocation is the durable admission point: its actor lease is
        // retained before any worker is scheduled, so ownership evidence must not
        // depend on runner scheduling.
        let first = runtime
            .bind(request(&roots[0]))
            .expect("first root admitted");
        let second = runtime
            .bind(request(&roots[1]))
            .expect("second root admitted");
        let alias = runtime
            .bind(request(&roots[0].join(".")))
            .expect("an alias of a live root reuses its actor");
        let third = runtime
            .bind(request(&roots[2]))
            .expect("a distinct third root is admitted");
        let ActorBoundCanonicalInvocationState::Workspace {
            invocation: first_invocation,
            ..
        } = &first.0
        else {
            panic!("first root did not bind a workspace actor");
        };
        let ActorBoundCanonicalInvocationState::Workspace {
            invocation: alias_invocation,
            ..
        } = &alias.0
        else {
            panic!("alias did not bind a workspace actor");
        };
        assert!(Arc::ptr_eq(
            first_invocation.actor_for_test(),
            alias_invocation.actor_for_test()
        ));
        assert_eq!(runtime.workspace_actors.entry_len_for_test().unwrap(), 3);

        drop((first, second, alias, third));
    }

    fn concurrent_same_identity_admission_creates_one_actor() {
        let workspace = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(workspace.path()).unwrap();
        let context = discover_workspace(Some(root.clone())).unwrap();
        let registry = Arc::new(crate::infrastructure::source_workspace::actor_registry());
        let barrier = Arc::new(Barrier::new(9));
        let mut admissions = Vec::new();
        for _ in 0..8 {
            let registry = Arc::clone(&registry);
            let barrier = Arc::clone(&barrier);
            let context = context.clone();
            let root = root.clone();
            admissions.push(std::thread::spawn(move || {
                barrier.wait();
                registry
                    .get_or_create(
                        &context,
                        [WorkspaceSourceSetInput::new(
                            "main",
                            &root,
                            SourceSetKind::Configuration,
                            SourceFormat::PlatformXml,
                            SourceProfile::platform_xml_8_3_27_format_2_20(),
                        )],
                        "canonical-v0.13",
                    )
                    .unwrap()
            }));
        }
        barrier.wait();
        let actors = admissions
            .into_iter()
            .map(|admission| admission.join().unwrap())
            .collect::<Vec<_>>();
        for actor in actors.iter().skip(1) {
            assert!(Arc::ptr_eq(&actors[0], actor));
        }
        assert_eq!(registry.entry_len_for_test().unwrap(), 1);
    }

    fn poisoned_registry_is_a_closed_internal_error_and_admits_nothing() {
        let workspace = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(workspace.path()).unwrap();
        ensure_platform_xml_workspace(&root.to_string_lossy());
        let (entered, _entered_wait) = mpsc::channel();
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(BlockingService { entered }),
            Arc::new(TokioClock),
        );
        runtime.workspace_actors.poison_for_test();

        let error = runtime
            .bind(
                InvocationRequest::new(
                    ToolIdentity::Run,
                    serde_json::json!({"op": "test.long-work", "args": {}}),
                    root.to_string_lossy(),
                    7_000,
                )
                .unwrap(),
            )
            .err()
            .expect("a poisoned registry admits nothing");
        assert!(matches!(
            error,
            V5CanonicalPrepareError::WorkspaceRegistryFailed
        ));
    }

    fn sequential_direct_admissions_release_actor_capabilities_and_prune_dead_entries() {
        let workspaces = tempfile::tempdir().unwrap();
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(RejectingAfterActorAdmissionService),
            Arc::new(TokioClock),
        );
        let request = |workspace: &std::path::Path| {
            let root = std::fs::canonicalize(workspace).unwrap();
            ensure_platform_xml_workspace(&root.to_string_lossy());
            InvocationRequest::new(
                ToolIdentity::Run,
                serde_json::json!({"op": "test.long-work", "args": {}}),
                root.to_string_lossy(),
                7_000,
            )
            .unwrap()
        };

        // Within the idle interval, later profiles do not evict earlier actors.
        // None of these rejected calls issued a saved plan.
        for index in 0..80 {
            let workspace = workspaces.path().join(format!("workspace-{index}"));
            std::fs::create_dir(&workspace).unwrap();
            let result = direct_v5(&runtime, request(&workspace)).unwrap();
            assert!(
                !result.ok && result.summary == "test rejection after actor admission",
                "unexpected outcome: {result:?}"
            );
            assert_eq!(
                runtime.workspace_actors.entry_len_for_test().unwrap(),
                index + 1
            );
        }
        assert_eq!(runtime.workspace_actors.entry_len_for_test().unwrap(), 80);

        // With the warm TTL elapsed every completed workspace releases its
        // actor capability and the registry prunes the dead entry.
        let runtime = V5CanonicalInvocationRuntime::with_workspace_actors_for_test(
            Arc::new(RejectingAfterActorAdmissionService),
            Arc::new(TokioClock),
            WorkspaceActorRegistry::with_idle_policy_for_test(Duration::ZERO),
        );
        for index in 0..20 {
            let workspace = workspaces.path().join(format!("expired-{index}"));
            std::fs::create_dir(&workspace).unwrap();
            direct_v5(&runtime, request(&workspace)).unwrap();
            runtime.workspace_actors.evict_idle_warm_actors().unwrap();
            assert!(runtime.workspace_actors.live_len_for_test().unwrap() <= 1);
        }
        assert_eq!(runtime.workspace_actors.live_len_for_test().unwrap(), 0);
    }

    #[test]
    pub(crate) fn restart_request_does_not_claim_noncooperative_actor_released_in_process() {
        let workspace = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(workspace.path()).unwrap();
        ensure_platform_xml_workspace(&root.to_string_lossy());
        let (entered, entered_wait) = mpsc::channel();
        let (release, release_wait) = mpsc::channel();
        let service: Arc<dyn CanonicalInvocationService> = Arc::new(NonCooperativeActorService {
            entered,
            release: Mutex::new(release_wait),
        });
        // A zero warm TTL: once an execution returns its actor, nothing but
        // the warm set could keep it, and an eviction pass shows the truth.
        let canonical = V5CanonicalInvocationRuntime::with_workspace_actors_for_test(
            Arc::clone(&service),
            Arc::new(TokioClock),
            WorkspaceActorRegistry::with_idle_policy_for_test(Duration::ZERO),
        );
        let daemon = LiveV5Daemon::over(Arc::new(canonical), service);
        let owner = daemon.owner();
        let request = InvocationRequest::new(
            ToolIdentity::Run,
            serde_json::json!({"op": "test.long-work", "args": {}}),
            root.to_string_lossy(),
            7_000,
        )
        .unwrap();
        let task_id = daemon.task_id(&owner, &request);
        entered_wait.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(
            daemon
                .runtime()
                .workspace_actors
                .live_len_for_test()
                .unwrap(),
            1
        );

        // Cancellation is a request to the executing attempt, never a claim on
        // its actor: a non-cooperative execution keeps the actor until it
        // returns, and only then is the capability released in process.
        let cancelled = daemon.cancel(&owner, task_id);
        assert_ne!(cancelled.status(), InvocationStatus::Completed);
        daemon
            .runtime()
            .workspace_actors
            .evict_idle_warm_actors()
            .unwrap();
        assert_eq!(
            daemon
                .runtime()
                .workspace_actors
                .live_len_for_test()
                .unwrap(),
            1,
            "the executing ActorBoundExecution still owns the actor until it returns"
        );

        release.send(()).unwrap();
        let terminal = daemon.wait_terminal(&owner, task_id, Duration::from_secs(10));
        assert_ne!(
            terminal.status(),
            InvocationStatus::Working,
            "a returned non-cooperative execution settles the Task: {terminal:?}"
        );
        assert!(
            terminal.completed_result().is_none(),
            "a cancelled attempt publishes no staged result: {terminal:?}"
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            daemon
                .runtime()
                .workspace_actors
                .evict_idle_warm_actors()
                .unwrap();
            if daemon
                .runtime()
                .workspace_actors
                .live_len_for_test()
                .unwrap()
                == 0
                || Instant::now() >= deadline
            {
                break;
            }
            std::thread::yield_now();
        }
        assert_eq!(
            daemon
                .runtime()
                .workspace_actors
                .live_len_for_test()
                .unwrap(),
            0
        );
        daemon.finish(owner);
    }

    #[test]
    fn daemon_workspace_actor_admission_is_concurrent_and_fail_closed() {
        live_actor_admission_reuses_alias_and_accepts_distinct_roots();
        concurrent_same_identity_admission_creates_one_actor();
        poisoned_registry_is_a_closed_internal_error_and_admits_nothing();
        sequential_direct_admissions_release_actor_capabilities_and_prune_dead_entries();
        unica_workspace::actor::retention_contract_tests::warm_registry_reuses_the_same_actor_across_sequential_admissions();
        unica_workspace::actor::retention_contract_tests::warm_actor_expires_after_the_idle_ttl_and_is_rebuilt();
        unica_workspace::actor::retention_contract_tests::warm_actor_whose_named_root_was_replaced_is_rebuilt();
        unica_workspace::actor::retention_contract_tests::warm_actors_do_not_block_a_distinct_identity(
        );
    }
    const OWN_REMOVAL_AT: &str = "main:Report.OwnRemovalReport.Template.OwnDiagram";
    const OWN_REMOVAL_OWNER_AT: &str = "main:Report.OwnRemovalReport";
    const OWN_REMOVAL_OWNER: &str = "Reports/OwnRemovalReport.xml";
    const OWN_REMOVAL_BASE: &str = "Reports/OwnRemovalReport/Templates/OwnDiagram";
    const OWN_REMOVAL_BODY: &str = "Reports/OwnRemovalReport/Templates/OwnDiagram/Ext/Template.xml";
    const OWN_REMOVAL_DESCRIPTOR: &str = "Reports/OwnRemovalReport/Templates/OwnDiagram.xml";
    const OWN_REMOVAL_RED: &str =
        "Reports/OwnRemovalReport/Templates/OwnDiagram/Ext/Template/Items/RedSquare/Picture.png";
    const OWN_REMOVAL_FILES: &[&str] = &[
        "v8project.yaml",
        "src/Configuration.xml",
        "src/Languages/English.xml",
        "src/Reports/OwnRemovalReport.xml",
        "src/Reports/OwnRemovalReport/Templates/OwnDiagram.xml",
        "src/Reports/OwnRemovalReport/Templates/KeepDiagram.xml",
        "src/Reports/OwnRemovalReport/Templates/OwnDiagram/Ext/Template.xml",
        "src/Reports/OwnRemovalReport/Templates/KeepDiagram/Ext/Template.xml",
        "src/Reports/OwnRemovalReport/Templates/OwnDiagram/Ext/Template/Items/RedSquare/Picture.png",
        "src/Reports/OwnRemovalReport/Templates/OwnDiagram/Ext/Template/Items/BlueSquare/Picture.png",
    ];

    fn own_removal_workspace() -> tempfile::TempDir {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-whole-template-removal");
        let workspace = tempfile::tempdir().unwrap();
        for relative in OWN_REMOVAL_FILES {
            let source = fixture.join(relative);
            assert!(std::fs::symlink_metadata(&source)
                .unwrap()
                .file_type()
                .is_file());
            let target = workspace.path().join(relative);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, std::fs::read(source).unwrap()).unwrap();
        }
        workspace
    }

    // Includes empty directories and links without following them. This helper is
    // only an oracle for write-free preview/refusal, never the public preview.
    #[derive(Debug, PartialEq, Eq)]
    enum OwnRemovalEntry {
        Directory,
        File(Vec<u8>),
        Link(std::path::PathBuf),
    }

    fn own_removal_snapshot(
        root: &std::path::Path,
    ) -> std::collections::BTreeMap<std::path::PathBuf, OwnRemovalEntry> {
        fn visit(
            root: &std::path::Path,
            current: &std::path::Path,
            entries: &mut std::collections::BTreeMap<std::path::PathBuf, OwnRemovalEntry>,
        ) {
            let root_metadata = match std::fs::symlink_metadata(current) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
                Err(error) => panic!("snapshot root metadata: {error}"),
            };
            assert!(root_metadata.is_dir());
            assert!(!unica_workspace::paths::metadata_is_link_or_reparse_point(
                &root_metadata
            ));
            for entry in std::fs::read_dir(current).unwrap() {
                let path = entry.unwrap().path();
                let metadata = std::fs::symlink_metadata(&path).unwrap();
                let kind = metadata.file_type();
                let relative = path.strip_prefix(root).unwrap().to_path_buf();
                if unica_workspace::paths::metadata_is_link_or_reparse_point(&metadata) {
                    entries.insert(
                        relative,
                        OwnRemovalEntry::Link(std::fs::read_link(path).unwrap()),
                    );
                } else if kind.is_dir() {
                    entries.insert(relative, OwnRemovalEntry::Directory);
                    visit(root, &path, entries);
                } else {
                    assert!(
                        kind.is_file(),
                        "unhandled fixture entry: {}",
                        path.display()
                    );
                    entries.insert(
                        relative,
                        OwnRemovalEntry::File(std::fs::read(path).unwrap()),
                    );
                }
            }
        }
        let mut entries = std::collections::BTreeMap::new();
        visit(root, root, &mut entries);
        entries
    }

    fn own_removal_file_bytes(
        entries: &std::collections::BTreeMap<std::path::PathBuf, OwnRemovalEntry>,
    ) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
        entries
            .iter()
            .filter_map(|(path, entry)| match entry {
                OwnRemovalEntry::File(bytes) => Some((path.clone(), bytes.clone())),
                OwnRemovalEntry::Directory | OwnRemovalEntry::Link(_) => None,
            })
            .collect()
    }

    fn own_removal_plan(top_at: &str) -> serde_json::Value {
        serde_json::json!({
            "at": top_at,
            "ops": [{"op": "template.remove", "args": {"at": OWN_REMOVAL_AT}}],
        })
    }

    fn own_removal_assert_refused(result: &DomainResult) {
        assert!(!result.ok, "damaged/changed selection admitted: {result:?}");
        assert!(
            result.changed.is_empty(),
            "refusal reports committed changes: {result:?}"
        );
        assert!(result
            .data
            .as_ref()
            .and_then(|data| data.get("executionToken"))
            .is_none());
    }

    #[test]
    fn canonical_whole_graphical_template_removal_preserves_the_exact_neighbor_and_replays() {
        // Both are existing canonical address forms. This also detects an owner-top
        // request whose operation target would receive weaker guards.
        for top_at in [OWN_REMOVAL_AT, OWN_REMOVAL_OWNER_AT] {
            let workspace = own_removal_workspace();
            let source = workspace.path().join("src");
            let before = own_removal_snapshot(&source);
            let mut expected = own_removal_file_bytes(&before);
            let original_owner =
                String::from_utf8(expected[std::path::Path::new(OWN_REMOVAL_OWNER)].clone())
                    .unwrap();
            let selected_line = "      <Template>OwnDiagram</Template>\n";
            assert_eq!(original_owner.matches(selected_line).count(), 1);
            let expected_owner = original_owner.replace(selected_line, "").into_bytes();
            expected.insert(OWN_REMOVAL_OWNER.into(), expected_owner);
            for relative in [
                OWN_REMOVAL_DESCRIPTOR,
                OWN_REMOVAL_BODY,
                OWN_REMOVAL_RED,
                "Reports/OwnRemovalReport/Templates/OwnDiagram/Ext/Template/Items/BlueSquare/Picture.png",
            ] {
                assert!(expected.remove(std::path::Path::new(relative)).is_some());
            }
            let runtime = bootstrap_runtime();
            let preview = submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::Apply,
                own_removal_plan(top_at),
            );
            assert!(preview.ok, "{top_at}: {preview:?}");
            assert_eq!(preview.data.as_ref().unwrap()["mode"], "preview");
            assert!(preview
                .changed
                .iter()
                .any(|change| change["at"] == OWN_REMOVAL_AT));
            assert_eq!(
                own_removal_snapshot(&source),
                before,
                "preview changed source entries"
            );
            let token = preview.data.as_ref().unwrap()["executionToken"].clone();
            assert!(!token.as_str().unwrap().is_empty());
            let applied = submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::Apply,
                serde_json::json!({"executionToken": token}),
            );
            assert!(applied.ok, "{top_at}: {applied:?}");
            assert_eq!(applied.data.as_ref().unwrap()["mode"], "published");
            let post = own_removal_snapshot(&source);
            assert_eq!(own_removal_file_bytes(&post), expected);
            // The selected payload leaves no directory behind; the collection
            // still holds the neighbor (#1997).
            assert!(post.keys().all(|path| !path.starts_with(OWN_REMOVAL_BASE)));
            assert_eq!(
                post.get(std::path::Path::new("Reports/OwnRemovalReport/Templates")),
                Some(&OwnRemovalEntry::Directory)
            );
            for (path, entry) in &post {
                assert!(before.contains_key(path), "new postimage entry: {path:?}");
                match entry {
                    OwnRemovalEntry::Directory => assert_eq!(before.get(path), Some(entry)),
                    OwnRemovalEntry::Link(_) => panic!("new link/reparse point: {path:?}"),
                    OwnRemovalEntry::File(_) => {}
                }
                if !path.starts_with(OWN_REMOVAL_BASE)
                    && path != std::path::Path::new(OWN_REMOVAL_OWNER)
                {
                    assert_eq!(before.get(path), Some(entry));
                }
            }
            for (path, entry) in &before {
                if !path.starts_with(OWN_REMOVAL_BASE)
                    && path != std::path::Path::new(OWN_REMOVAL_DESCRIPTOR)
                    && path != std::path::Path::new(OWN_REMOVAL_OWNER)
                {
                    assert_eq!(
                        post.get(path),
                        Some(entry),
                        "lost unrelated entry: {path:?}"
                    );
                }
            }
            let replay = submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::Apply,
                serde_json::json!({"executionToken": token}),
            );
            assert_eq!(
                serde_json::to_value(&replay).unwrap(),
                serde_json::to_value(&applied).unwrap()
            );
            assert_eq!(own_removal_snapshot(&source), post);
        }
    }

    fn own_removal_assert_damaged_selection_refused(top_at: &str, case: &str) {
        let workspace = own_removal_workspace();
        let source = workspace.path().join("src");
        match case {
            "missing-wrapper" => std::fs::remove_file(source.join(OWN_REMOVAL_DESCRIPTOR)).unwrap(),
            "malformed-wrapper" => {
                std::fs::write(source.join(OWN_REMOVAL_DESCRIPTOR), "<MetaDataObject").unwrap()
            }
            "wrong-name" => {
                let path = source.join(OWN_REMOVAL_DESCRIPTOR);
                let text = std::fs::read_to_string(&path).unwrap();
                std::fs::write(
                    path,
                    text.replace("<Name>OwnDiagram</Name>", "<Name>OtherDiagram</Name>"),
                )
                .unwrap();
            }
            "duplicate-registration" => {
                let path = source.join(OWN_REMOVAL_OWNER);
                let text = std::fs::read_to_string(&path).unwrap();
                std::fs::write(
                    path,
                    text.replace(
                        "<Template>OwnDiagram</Template>",
                        "<Template>OwnDiagram</Template><Template>OwnDiagram</Template>",
                    ),
                )
                .unwrap();
            }
            "missing-primary" => {
                std::fs::remove_file(source.join(OWN_REMOVAL_BODY)).unwrap();
            }
            "wrong-wrapper-kind" => {
                let path = source.join(OWN_REMOVAL_DESCRIPTOR);
                let text = std::fs::read_to_string(&path).unwrap();
                assert_eq!(text.matches("<Template uuid=").count(), 1);
                std::fs::write(
                    path,
                    text.replace("<Template uuid=", "<Form uuid=")
                        .replace("</Template>", "</Form>"),
                )
                .unwrap();
            }
            "wrong-wrapper-namespace" => {
                let path = source.join(OWN_REMOVAL_DESCRIPTOR);
                let text = std::fs::read_to_string(&path).unwrap();
                let namespace = "xmlns=\"http://v8.1c.ru/8.3/MDClasses\"";
                assert_eq!(text.matches(namespace).count(), 1);
                std::fs::write(
                    path,
                    text.replace(namespace, "xmlns=\"urn:foreign-template-wrapper\""),
                )
                .unwrap();
            }
            "invalid-wrapper-uuid" => {
                let path = source.join(OWN_REMOVAL_DESCRIPTOR);
                let text = std::fs::read_to_string(&path).unwrap();
                assert_eq!(text.matches("uuid=\"").count(), 1);
                std::fs::write(path, text.replacen("uuid=\"", "uuid=\"invalid-", 1)).unwrap();
            }
            "malformed-body" => {
                std::fs::write(source.join(OWN_REMOVAL_BODY), "<GraphicalSchema").unwrap()
            }
            "old-body-version" => {
                let path = source.join(OWN_REMOVAL_BODY);
                let text = std::fs::read_to_string(&path).unwrap();
                std::fs::write(path, text.replace("version=\"2.20\"", "version=\"2.19\"")).unwrap();
            }
            _ => unreachable!(),
        }
        let before = own_removal_snapshot(&source);
        let runtime = bootstrap_runtime();
        let result = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            own_removal_plan(top_at),
        );
        own_removal_assert_refused(&result);
        assert!(
            ["invalid_source", "not_found", "bad_value"].iter().any(|code| result.diagnostics[0]["code"] == *code),
            "{top_at}: {case}: unrelated/transient refusal does not prove source validation: {result:?}"
        );
        if case == "old-body-version" {
            assert_eq!(
                result.diagnostics[0]["code"], "invalid_source",
                "{top_at}: {result:?}"
            );
            assert_eq!(
                result.diagnostics[0]["outcome"], "fixSource",
                "{top_at}: {result:?}"
            );
        }
        assert_eq!(own_removal_snapshot(&source), before, "{top_at}: {case}");
    }

    macro_rules! own_removal_refusal_case {
        ($name:ident, $top_at:expr, $case:literal) => {
            #[test]
            fn $name() {
                own_removal_assert_damaged_selection_refused($top_at, $case);
            }
        };
    }

    own_removal_refusal_case!(
        canonical_whole_graphical_template_removal_refuses_missing_wrapper_at_template,
        OWN_REMOVAL_AT,
        "missing-wrapper"
    );
    own_removal_refusal_case!(
        canonical_whole_graphical_template_removal_refuses_missing_wrapper_at_owner,
        OWN_REMOVAL_OWNER_AT,
        "missing-wrapper"
    );
    own_removal_refusal_case!(
        canonical_whole_graphical_template_removal_refuses_malformed_wrapper_at_template,
        OWN_REMOVAL_AT,
        "malformed-wrapper"
    );
    own_removal_refusal_case!(
        canonical_whole_graphical_template_removal_refuses_malformed_wrapper_at_owner,
        OWN_REMOVAL_OWNER_AT,
        "malformed-wrapper"
    );
    own_removal_refusal_case!(
        canonical_whole_graphical_template_removal_refuses_wrong_name_at_template,
        OWN_REMOVAL_AT,
        "wrong-name"
    );
    own_removal_refusal_case!(
        canonical_whole_graphical_template_removal_refuses_wrong_name_at_owner,
        OWN_REMOVAL_OWNER_AT,
        "wrong-name"
    );
    own_removal_refusal_case!(
        canonical_whole_graphical_template_removal_refuses_duplicate_registration_at_template,
        OWN_REMOVAL_AT,
        "duplicate-registration"
    );
    own_removal_refusal_case!(
        canonical_whole_graphical_template_removal_refuses_duplicate_registration_at_owner,
        OWN_REMOVAL_OWNER_AT,
        "duplicate-registration"
    );
    own_removal_refusal_case!(
        canonical_whole_graphical_template_removal_refuses_malformed_body_at_template,
        OWN_REMOVAL_AT,
        "malformed-body"
    );
    own_removal_refusal_case!(
        canonical_whole_graphical_template_removal_refuses_malformed_body_at_owner,
        OWN_REMOVAL_OWNER_AT,
        "malformed-body"
    );
    own_removal_refusal_case!(
        canonical_whole_graphical_template_removal_refuses_old_body_version_at_template,
        OWN_REMOVAL_AT,
        "old-body-version"
    );
    own_removal_refusal_case!(
        canonical_whole_graphical_template_removal_refuses_old_body_version_at_owner,
        OWN_REMOVAL_OWNER_AT,
        "old-body-version"
    );

    own_removal_refusal_case!(
        canonical_whole_graphical_template_removal_refuses_missing_primary_at_template,
        OWN_REMOVAL_AT,
        "missing-primary"
    );

    own_removal_refusal_case!(
        canonical_whole_graphical_template_removal_refuses_missing_primary_at_owner,
        OWN_REMOVAL_OWNER_AT,
        "missing-primary"
    );

    own_removal_refusal_case!(
        canonical_whole_graphical_template_removal_refuses_wrong_wrapper_kind_at_template,
        OWN_REMOVAL_AT,
        "wrong-wrapper-kind"
    );

    own_removal_refusal_case!(
        canonical_whole_graphical_template_removal_refuses_wrong_wrapper_kind_at_owner,
        OWN_REMOVAL_OWNER_AT,
        "wrong-wrapper-kind"
    );

    own_removal_refusal_case!(
        canonical_whole_graphical_template_removal_refuses_wrong_wrapper_namespace_at_template,
        OWN_REMOVAL_AT,
        "wrong-wrapper-namespace"
    );

    own_removal_refusal_case!(
        canonical_whole_graphical_template_removal_refuses_wrong_wrapper_namespace_at_owner,
        OWN_REMOVAL_OWNER_AT,
        "wrong-wrapper-namespace"
    );

    own_removal_refusal_case!(
        canonical_whole_graphical_template_removal_refuses_invalid_wrapper_uuid_at_template,
        OWN_REMOVAL_AT,
        "invalid-wrapper-uuid"
    );

    own_removal_refusal_case!(
        canonical_whole_graphical_template_removal_refuses_invalid_wrapper_uuid_at_owner,
        OWN_REMOVAL_OWNER_AT,
        "invalid-wrapper-uuid"
    );

    #[test]
    fn canonical_whole_graphical_template_removal_refuses_changed_payload_membership_and_owner() {
        for case in [
            "picture-bytes",
            "new-payload-file",
            "new-empty-directory",
            "owner-registration",
        ] {
            let workspace = own_removal_workspace();
            let source = workspace.path().join("src");
            let runtime = bootstrap_runtime();
            let preview = submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::Apply,
                own_removal_plan(OWN_REMOVAL_AT),
            );
            assert!(preview.ok, "{preview:?}");
            let token = preview.data.as_ref().unwrap()["executionToken"].clone();
            match case {
                "picture-bytes" => {
                    let path = source.join(OWN_REMOVAL_RED);
                    // Hold the original object alive so an identity comparison cannot
                    // pass through inode reuse after a replacement.
                    let held = std::fs::File::open(&path).unwrap();
                    let identity_before =
                        unica_workspace::state_fs::test_support::file_identity(&held).unwrap();
                    let original = std::fs::read(&path).unwrap();
                    let mut bytes = original.clone();
                    // External concurrent bytes: same length, same inode, different content.
                    let last = bytes.len() - 1;
                    bytes[last] ^= 1;
                    std::fs::write(&path, &bytes).unwrap();
                    let named_after = std::fs::File::open(&path).unwrap();
                    let identity_after =
                        unica_workspace::state_fs::test_support::file_identity(&named_after)
                            .unwrap();
                    assert_eq!(identity_before, identity_after);
                    assert_eq!(bytes.len(), original.len());
                    assert_ne!(bytes, original);
                    assert_eq!(std::fs::read(&path).unwrap(), bytes);
                }
                "new-payload-file" => std::fs::write(
                    source.join(OWN_REMOVAL_BASE).join("Ext/foreign.txt"),
                    "concurrent own sentinel",
                )
                .unwrap(),
                "new-empty-directory" => {
                    std::fs::create_dir(source.join(OWN_REMOVAL_BASE).join("Ext/foreign-empty"))
                        .unwrap()
                }
                "owner-registration" => {
                    let path = source.join(OWN_REMOVAL_OWNER);
                    let text = std::fs::read_to_string(&path).unwrap();
                    std::fs::write(
                        path,
                        text.replace(
                            "<Template>OwnDiagram</Template>",
                            "<Template>OtherDiagram</Template>",
                        ),
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
            // Refusal must preserve the concurrent image, not restore pre-preview bytes.
            let concurrent = own_removal_snapshot(&source);
            let result = submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::Apply,
                serde_json::json!({"executionToken": token}),
            );
            own_removal_assert_refused(&result);
            assert_eq!(
                result.diagnostics[0]["code"], "stale_revision",
                "{case}: {result:?}"
            );
            assert_eq!(
                own_removal_snapshot(&source),
                concurrent,
                "{case}: overwrote foreign change"
            );
        }
    }
    fn template_removal_assert_active_document_payload_absence_refused(
        top_at: &str,
        keep_empty_root: bool,
    ) {
        let workspace = own_removal_workspace();
        let source = workspace.path().join("src");
        let descriptor = source.join(OWN_REMOVAL_DESCRIPTOR);
        let image = std::fs::read_to_string(&descriptor).unwrap();
        let original_kind = "<TemplateType>GraphicalSchema</TemplateType>";
        assert_eq!(image.matches(original_kind).count(), 1);
        // An explicitly damaged input variation. This does not claim a valid
        // ActiveDocument body, a primary filename, or native acceptance.
        std::fs::write(
            descriptor,
            image.replacen(
                original_kind,
                "<TemplateType>ActiveDocument</TemplateType>",
                1,
            ),
        )
        .unwrap();
        let payload = source.join(OWN_REMOVAL_BASE);
        std::fs::remove_dir_all(&payload).unwrap();
        if keep_empty_root {
            std::fs::create_dir(&payload).unwrap();
            assert_eq!(std::fs::read_dir(&payload).unwrap().count(), 0);
        } else {
            assert_eq!(
                std::fs::symlink_metadata(&payload).unwrap_err().kind(),
                std::io::ErrorKind::NotFound
            );
        }
        let before = own_removal_snapshot(&source);
        let runtime = bootstrap_runtime();
        let result = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            own_removal_plan(top_at),
        );
        own_removal_assert_refused(&result);
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_source",
            "{top_at}: empty root={keep_empty_root}: {result:?}"
        );
        assert_eq!(
            result.diagnostics[0]["outcome"], "fixSource",
            "{top_at}: empty root={keep_empty_root}: {result:?}"
        );
        assert_eq!(
            own_removal_snapshot(&source),
            before,
            "{top_at}: empty root={keep_empty_root}"
        );
    }

    macro_rules! active_document_payload_absence_case {
        ($name:ident, $top_at:expr, $keep_empty_root:expr) => {
            #[test]
            fn $name() {
                template_removal_assert_active_document_payload_absence_refused(
                    $top_at,
                    $keep_empty_root,
                );
            }
        };
    }

    active_document_payload_absence_case!(
        canonical_template_removal_active_document_absent_payload_root_at_template,
        OWN_REMOVAL_AT,
        false
    );
    active_document_payload_absence_case!(
        canonical_template_removal_active_document_absent_payload_root_at_owner,
        OWN_REMOVAL_OWNER_AT,
        false
    );
    active_document_payload_absence_case!(
        canonical_template_removal_active_document_empty_payload_root_at_template,
        OWN_REMOVAL_AT,
        true
    );
    active_document_payload_absence_case!(
        canonical_template_removal_active_document_empty_payload_root_at_owner,
        OWN_REMOVAL_OWNER_AT,
        true
    );

    const BORROWED_REMOVAL_AT: &str = "ext:DataProcessor.Обработка.Template.Макет";
    const BORROWED_REMOVAL_OWNER_AT: &str = "ext:DataProcessor.Обработка";
    const BORROWED_REMOVAL_OWNER: &str = "ext/DataProcessors/Обработка.xml";
    const BORROWED_REMOVAL_DESCRIPTOR: &str = "ext/DataProcessors/Обработка/Templates/Макет.xml";
    const BORROWED_REMOVAL_PAYLOAD: &str = "ext/DataProcessors/Обработка/Templates/Макет";

    fn borrowed_removal_workspace_without_local_type() -> tempfile::TempDir {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-borrowed");
        let fixture_entries = own_removal_snapshot(&fixture);
        let workspace = tempfile::tempdir().unwrap();
        // Copy the complete two-source fixture, including empty directories,
        // without following links or silently omitting registered neighbors.
        for (relative, entry) in &fixture_entries {
            let destination = workspace.path().join(relative);
            match entry {
                OwnRemovalEntry::Directory => std::fs::create_dir_all(destination).unwrap(),
                OwnRemovalEntry::File(bytes) => {
                    std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
                    std::fs::write(destination, bytes).unwrap();
                }
                OwnRemovalEntry::Link(_) => {
                    panic!("borrowed fixture must be a closed regular tree")
                }
            }
        }
        assert_eq!(own_removal_snapshot(workspace.path()), fixture_entries);
        let descriptor = workspace
            .path()
            .join("src")
            .join(BORROWED_REMOVAL_DESCRIPTOR);
        let original = std::fs::read_to_string(&descriptor).unwrap();
        let type_line = "\t\t\t<TemplateType>SpreadsheetDocument</TemplateType>\r\n";
        assert_eq!(original.matches(type_line).count(), 1);
        // Deliberate input variation under borrowed-object-inherited-content.md.
        // The stored native export has an explicit default TemplateType. The
        // platform accepts an omitted input property and adds its default on
        // export; this edited copy is not claimed to be another native export.
        std::fs::write(descriptor, original.replacen(type_line, "", 1)).unwrap();
        let source = workspace.path().join("src");
        assert!(!own_removal_snapshot(&source)
            .contains_key(std::path::Path::new(BORROWED_REMOVAL_PAYLOAD)));
        workspace
    }

    fn borrowed_removal_plan(top_at: &str) -> serde_json::Value {
        serde_json::json!({
            "at": top_at,
            "ops": [{"op": "template.remove", "args": {"at": BORROWED_REMOVAL_AT}}],
        })
    }

    fn borrowed_removal_assert_source_refused(result: &DomainResult) {
        own_removal_assert_refused(result);
        assert_eq!(result.diagnostics[0]["code"], "invalid_source");
        assert_eq!(result.diagnostics[0]["outcome"], "fixSource");
    }

    #[test]
    fn canonical_borrowed_template_without_type_removes_only_local_registration_and_descriptor() {
        for top_at in [BORROWED_REMOVAL_AT, BORROWED_REMOVAL_OWNER_AT] {
            let workspace = borrowed_removal_workspace_without_local_type();
            let source = workspace.path().join("src");
            let before = own_removal_snapshot(&source);
            let mut expected = own_removal_snapshot(&source);
            let before_files = own_removal_file_bytes(&before);
            let owner_before = String::from_utf8(
                before_files[std::path::Path::new(BORROWED_REMOVAL_OWNER)].clone(),
            )
            .unwrap();
            let selected_line = "\t\t\t<Template>Макет</Template>\r\n";
            assert_eq!(owner_before.matches(selected_line).count(), 1);
            assert!(owner_before.starts_with('\u{feff}'));
            let expected_owner = owner_before.replacen(selected_line, "", 1).into_bytes();
            expected.insert(
                BORROWED_REMOVAL_OWNER.into(),
                OwnRemovalEntry::File(expected_owner),
            );
            assert!(matches!(
                expected.remove(std::path::Path::new(BORROWED_REMOVAL_DESCRIPTOR)),
                Some(OwnRemovalEntry::File(_))
            ));
            let runtime = bootstrap_runtime();
            let preview = submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::Apply,
                borrowed_removal_plan(top_at),
            );
            assert!(preview.ok, "borrowed descriptor-only preview succeeds");
            assert_eq!(preview.data.as_ref().unwrap()["mode"], "preview");
            assert_eq!(own_removal_snapshot(&source), before);
            let token = preview.data.as_ref().unwrap()["executionToken"].clone();
            assert!(!token.as_str().unwrap().is_empty());
            let applied = submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::Apply,
                serde_json::json!({"executionToken": token.clone()}),
            );
            assert!(applied.ok, "borrowed descriptor-only publication succeeds");
            assert_eq!(applied.data.as_ref().unwrap()["mode"], "published");
            let after = own_removal_snapshot(&source);
            // Full entry equality keeps all parent sources, neighbor bytes and
            // directory membership; no local payload directory may be created.
            assert_eq!(after, expected);
            let replay = submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::Apply,
                serde_json::json!({"executionToken": token}),
            );
            assert_eq!(
                serde_json::to_value(&replay).unwrap(),
                serde_json::to_value(&applied).unwrap()
            );
            assert_eq!(own_removal_snapshot(&source), after);
        }
    }

    #[test]
    fn canonical_borrowed_sole_template_without_payload_takes_its_empty_collection() {
        // Without the borrowed neighbor the selected descriptor is the last
        // entry of `Templates`, so the collection goes with it (#1997); the
        // sibling `Forms` collection and the owner directory stay.
        let workspace = borrowed_removal_workspace_without_local_type();
        let source = workspace.path().join("src");
        let templates = source.join("ext/DataProcessors/Обработка/Templates");
        std::fs::remove_file(templates.join("СхемаКомпоновки.xml")).unwrap();
        let owner = source.join(BORROWED_REMOVAL_OWNER);
        let text = std::fs::read_to_string(&owner).unwrap();
        let neighbor_line = "\t\t\t<Template>СхемаКомпоновки</Template>\r\n";
        assert_eq!(text.matches(neighbor_line).count(), 1);
        std::fs::write(&owner, text.replacen(neighbor_line, "", 1)).unwrap();
        let before = own_removal_snapshot(&source);
        let mut expected = own_removal_snapshot(&source);
        let selected_line = "\t\t\t<Template>Макет</Template>\r\n";
        let owner_text = std::fs::read_to_string(&owner).unwrap();
        assert_eq!(owner_text.matches(selected_line).count(), 1);
        expected.insert(
            BORROWED_REMOVAL_OWNER.into(),
            OwnRemovalEntry::File(owner_text.replacen(selected_line, "", 1).into_bytes()),
        );
        assert!(expected
            .remove(std::path::Path::new(BORROWED_REMOVAL_DESCRIPTOR))
            .is_some());
        assert!(matches!(
            expected.remove(std::path::Path::new(
                "ext/DataProcessors/Обработка/Templates"
            )),
            Some(OwnRemovalEntry::Directory)
        ));
        let runtime = bootstrap_runtime();
        let preview = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            borrowed_removal_plan(BORROWED_REMOVAL_AT),
        );
        assert!(preview.ok, "{preview:?}");
        assert_eq!(own_removal_snapshot(&source), before);
        let applied = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            serde_json::json!({"executionToken": preview.data.as_ref().unwrap()["executionToken"]}),
        );
        assert!(applied.ok, "{applied:?}");
        assert_eq!(own_removal_snapshot(&source), expected);
        assert!(source.join("ext/DataProcessors/Обработка/Forms").is_dir());
    }

    #[test]
    fn canonical_borrowed_template_empty_payload_root_is_not_inherited_absence() {
        for top_at in [BORROWED_REMOVAL_AT, BORROWED_REMOVAL_OWNER_AT] {
            let workspace = borrowed_removal_workspace_without_local_type();
            let source = workspace.path().join("src");
            std::fs::create_dir(source.join(BORROWED_REMOVAL_PAYLOAD)).unwrap();
            let before = own_removal_snapshot(&source);
            let runtime = bootstrap_runtime();
            let refused = submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::Apply,
                borrowed_removal_plan(top_at),
            );
            borrowed_removal_assert_source_refused(&refused);
            assert_eq!(own_removal_snapshot(&source), before);
        }
    }

    #[test]
    fn canonical_borrowed_template_saved_absence_refuses_new_empty_payload_root() {
        for top_at in [BORROWED_REMOVAL_AT, BORROWED_REMOVAL_OWNER_AT] {
            let workspace = borrowed_removal_workspace_without_local_type();
            let source = workspace.path().join("src");
            let before = own_removal_snapshot(&source);
            let runtime = bootstrap_runtime();
            let preview = submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::Apply,
                borrowed_removal_plan(top_at),
            );
            assert!(preview.ok, "absent inherited payload admits a saved plan");
            assert_eq!(preview.data.as_ref().unwrap()["mode"], "preview");
            assert_eq!(own_removal_snapshot(&source), before);
            let token = preview.data.as_ref().unwrap()["executionToken"].clone();
            std::fs::create_dir(source.join(BORROWED_REMOVAL_PAYLOAD)).unwrap();
            let concurrent = own_removal_snapshot(&source);
            let refused = submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::Apply,
                serde_json::json!({"executionToken": token}),
            );
            own_removal_assert_refused(&refused);
            assert_eq!(refused.diagnostics[0]["code"], "stale_revision");
            assert_eq!(refused.diagnostics[0]["outcome"], "fixCall");
            assert_eq!(own_removal_snapshot(&source), concurrent);
        }
    }

    fn borrowed_removal_corrupt_marker(
        relative: &str,
        property: &str,
        value: &str,
        duplicate: bool,
        top_at: &str,
    ) {
        let workspace = borrowed_removal_workspace_without_local_type();
        let source = workspace.path().join("src");
        let path = source.join(relative);
        let original = std::fs::read_to_string(&path).unwrap();
        let marker = format!("<{property}>{value}</{property}>");
        assert_eq!(original.matches(marker.as_str()).count(), 1);
        let replacement = if duplicate {
            format!("{marker}{marker}")
        } else {
            format!("<foreign:{property} xmlns:foreign=\"urn:unica-borrowed-removal-test\">{value}</foreign:{property}>")
        };
        std::fs::write(
            path,
            original.replacen(marker.as_str(), replacement.as_str(), 1),
        )
        .unwrap();
        let before = own_removal_snapshot(&source);
        let runtime = bootstrap_runtime();
        let refused = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            borrowed_removal_plan(top_at),
        );
        borrowed_removal_assert_source_refused(&refused);
        assert_eq!(own_removal_snapshot(&source), before);
    }

    macro_rules! borrowed_removal_marker_case {
        ($test:ident, $relative:expr, $property:literal, $value:literal, $duplicate:literal, $top_at:expr) => {
            #[test]
            fn $test() {
                borrowed_removal_corrupt_marker($relative, $property, $value, $duplicate, $top_at);
            }
        };
    }

    borrowed_removal_marker_case!(
        canonical_borrowed_template_foreign_owner_parent_at_template,
        BORROWED_REMOVAL_OWNER,
        "ExtendedConfigurationObject",
        "1f0e0000-0000-4000-8000-000000000103",
        false,
        BORROWED_REMOVAL_AT
    );
    borrowed_removal_marker_case!(
        canonical_borrowed_template_foreign_owner_parent_at_owner,
        BORROWED_REMOVAL_OWNER,
        "ExtendedConfigurationObject",
        "1f0e0000-0000-4000-8000-000000000103",
        false,
        BORROWED_REMOVAL_OWNER_AT
    );
    borrowed_removal_marker_case!(
        canonical_borrowed_template_duplicate_owner_parent_at_template,
        BORROWED_REMOVAL_OWNER,
        "ExtendedConfigurationObject",
        "1f0e0000-0000-4000-8000-000000000103",
        true,
        BORROWED_REMOVAL_AT
    );
    borrowed_removal_marker_case!(
        canonical_borrowed_template_duplicate_owner_parent_at_owner,
        BORROWED_REMOVAL_OWNER,
        "ExtendedConfigurationObject",
        "1f0e0000-0000-4000-8000-000000000103",
        true,
        BORROWED_REMOVAL_OWNER_AT
    );
    borrowed_removal_marker_case!(
        canonical_borrowed_template_foreign_child_parent_at_template,
        BORROWED_REMOVAL_DESCRIPTOR,
        "ExtendedConfigurationObject",
        "1f0e0000-0000-4000-8000-000000000105",
        false,
        BORROWED_REMOVAL_AT
    );
    borrowed_removal_marker_case!(
        canonical_borrowed_template_foreign_child_parent_at_owner,
        BORROWED_REMOVAL_DESCRIPTOR,
        "ExtendedConfigurationObject",
        "1f0e0000-0000-4000-8000-000000000105",
        false,
        BORROWED_REMOVAL_OWNER_AT
    );
    borrowed_removal_marker_case!(
        canonical_borrowed_template_duplicate_child_parent_at_template,
        BORROWED_REMOVAL_DESCRIPTOR,
        "ExtendedConfigurationObject",
        "1f0e0000-0000-4000-8000-000000000105",
        true,
        BORROWED_REMOVAL_AT
    );
    borrowed_removal_marker_case!(
        canonical_borrowed_template_duplicate_child_parent_at_owner,
        BORROWED_REMOVAL_DESCRIPTOR,
        "ExtendedConfigurationObject",
        "1f0e0000-0000-4000-8000-000000000105",
        true,
        BORROWED_REMOVAL_OWNER_AT
    );
    borrowed_removal_marker_case!(
        canonical_borrowed_template_foreign_extension_purpose_at_template,
        "ext/Configuration.xml",
        "ConfigurationExtensionPurpose",
        "Customization",
        false,
        BORROWED_REMOVAL_AT
    );
    borrowed_removal_marker_case!(
        canonical_borrowed_template_foreign_extension_purpose_at_owner,
        "ext/Configuration.xml",
        "ConfigurationExtensionPurpose",
        "Customization",
        false,
        BORROWED_REMOVAL_OWNER_AT
    );
    borrowed_removal_marker_case!(
        canonical_borrowed_template_duplicate_extension_purpose_at_template,
        "ext/Configuration.xml",
        "ConfigurationExtensionPurpose",
        "Customization",
        true,
        BORROWED_REMOVAL_AT
    );
    borrowed_removal_marker_case!(
        canonical_borrowed_template_duplicate_extension_purpose_at_owner,
        "ext/Configuration.xml",
        "ConfigurationExtensionPurpose",
        "Customization",
        true,
        BORROWED_REMOVAL_OWNER_AT
    );
    fn template_removal_workspace_from_fixture(relative: &str) -> tempfile::TempDir {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(relative);
        let entries = own_removal_snapshot(&fixture);
        let workspace = tempfile::tempdir().unwrap();
        for (relative, entry) in &entries {
            let destination = workspace.path().join(relative);
            match entry {
                OwnRemovalEntry::Directory => std::fs::create_dir_all(destination).unwrap(),
                OwnRemovalEntry::File(bytes) => {
                    std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
                    std::fs::write(destination, bytes).unwrap();
                }
                OwnRemovalEntry::Link(_) => panic!("test fixture must be a closed regular tree"),
            }
        }
        assert_eq!(own_removal_snapshot(workspace.path()), entries);
        workspace
    }

    fn template_removal_assert_postimage(
        before: &std::collections::BTreeMap<std::path::PathBuf, OwnRemovalEntry>,
        post: &std::collections::BTreeMap<std::path::PathBuf, OwnRemovalEntry>,
        expected: &std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
        payload_root: &std::path::Path,
        descriptor: &std::path::Path,
        owner: &std::path::Path,
    ) {
        assert_eq!(&own_removal_file_bytes(post), expected);
        for (path, entry) in post {
            assert!(before.contains_key(path), "new postimage entry: {path:?}");
            match entry {
                OwnRemovalEntry::Directory => assert_eq!(before.get(path), Some(entry)),
                OwnRemovalEntry::Link(_) => panic!("postimage contains link/reparse: {path:?}"),
                OwnRemovalEntry::File(_) => {}
            }
        }
        for (path, entry) in before {
            if !path.starts_with(payload_root) && path != descriptor && path != owner {
                assert_eq!(
                    post.get(path),
                    Some(entry),
                    "lost unrelated entry: {path:?}"
                );
            }
        }
    }

    fn template_removal_owner_representation(case: &str, top_at: &str) {
        let workspace = own_removal_workspace();
        let source = workspace.path().join("src");
        let owner_path = source.join(OWN_REMOVAL_OWNER);
        let original = std::fs::read_to_string(&owner_path).unwrap();
        let selected_line = "      <Template>OwnDiagram</Template>\n";
        let marker = "__OWN_REMOVAL_SELECTED_REGISTRATION__";
        assert_eq!(original.matches(selected_line).count(), 1);
        assert!(!original.contains(marker));
        let template = original.replacen(selected_line, marker, 1);
        let (owner, expected_owner) = match case {
            "compact-shadow-comment" => {
                let compact = template.replace('\n', "");
                let shadow = "<!--До:<Template>OwnDiagram</Template>--><!--сохранить-->";
                (
                    compact.replacen(
                        marker,
                        &format!("{shadow}<Template>OwnDiagram</Template>"),
                        1,
                    ),
                    compact.replacen(marker, shadow, 1),
                )
            }
            "alternate-prefix" => {
                let selected = "      <owned:Template xmlns:owned=\"http://v8.1c.ru/8.3/MDClasses\">OwnDiagram</owned:Template>\n";
                (
                    template.replacen(marker, selected, 1),
                    template.replacen(marker, "", 1),
                )
            }
            "encoded-name" => {
                let descriptor = source.join(OWN_REMOVAL_DESCRIPTOR);
                let bytes = std::fs::read_to_string(&descriptor).unwrap();
                let name = "<Name>OwnDiagram</Name>";
                assert_eq!(bytes.matches(name).count(), 1);
                std::fs::write(
                    descriptor,
                    bytes.replacen(name, "<Name>OwnDiag&#114;am</Name>", 1),
                )
                .unwrap();
                let selected = "      <Template>OwnDiag&#114;am</Template>\n";
                (
                    template.replacen(marker, selected, 1),
                    template.replacen(marker, "", 1),
                )
            }
            "comment-shadow-reference" => {
                let empty = "      <MainDataCompositionSchema/>\n";
                let comment = concat!(
                    "      <!--<DefaultTemplate>Report.OwnRemovalReport.Template.OwnDiagram</DefaultTemplate>-->\n",
                    "      <!--<MainDataCompositionSchema>Report.OwnRemovalReport.Template.OwnDiagram</MainDataCompositionSchema>-->\n",
                );
                assert_eq!(template.matches(empty).count(), 1);
                let template = template.replacen(empty, &format!("{comment}{empty}"), 1);
                (
                    template.replacen(marker, selected_line, 1),
                    template.replacen(marker, "", 1),
                )
            }
            _ => unreachable!(),
        };
        std::fs::write(&owner_path, &owner).unwrap();
        let before = own_removal_snapshot(&source);
        let mut expected = own_removal_file_bytes(&before);
        expected.insert(OWN_REMOVAL_OWNER.into(), expected_owner.into_bytes());
        for path in [
            OWN_REMOVAL_DESCRIPTOR, OWN_REMOVAL_BODY, OWN_REMOVAL_RED,
            "Reports/OwnRemovalReport/Templates/OwnDiagram/Ext/Template/Items/BlueSquare/Picture.png",
        ] {
            assert!(expected.remove(std::path::Path::new(path)).is_some());
        }
        let runtime = bootstrap_runtime();
        let preview = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            own_removal_plan(top_at),
        );
        assert!(preview.ok, "{case}: {top_at}: {preview:?}");
        assert_eq!(preview.data.as_ref().unwrap()["mode"], "preview");
        assert_eq!(
            own_removal_snapshot(&source),
            before,
            "preview source writes: {case}"
        );
        let token = preview.data.as_ref().unwrap()["executionToken"].clone();
        assert!(!token.as_str().unwrap().is_empty());
        let applied = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            serde_json::json!({"executionToken":token.clone()}),
        );
        assert!(applied.ok, "{case}: {top_at}: {applied:?}");
        assert_eq!(applied.data.as_ref().unwrap()["mode"], "published");
        let post = own_removal_snapshot(&source);
        template_removal_assert_postimage(
            &before,
            &post,
            &expected,
            std::path::Path::new(OWN_REMOVAL_BASE),
            std::path::Path::new(OWN_REMOVAL_DESCRIPTOR),
            std::path::Path::new(OWN_REMOVAL_OWNER),
        );
        let replay = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::Apply,
            serde_json::json!({"executionToken":token}),
        );
        assert_eq!(
            serde_json::to_value(replay).unwrap(),
            serde_json::to_value(applied).unwrap()
        );
        assert_eq!(own_removal_snapshot(&source), post);
    }

    macro_rules! template_removal_owner_case {
        ($name:ident, $case:literal, $top_at:expr) => {
            #[test]
            fn $name() {
                template_removal_owner_representation($case, $top_at);
            }
        };
    }

    template_removal_owner_case!(
        canonical_template_removal_owner_compact_shadow_comment_at_template,
        "compact-shadow-comment",
        OWN_REMOVAL_AT
    );
    template_removal_owner_case!(
        canonical_template_removal_owner_compact_shadow_comment_at_owner,
        "compact-shadow-comment",
        OWN_REMOVAL_OWNER_AT
    );
    template_removal_owner_case!(
        canonical_template_removal_owner_alternate_prefix_at_template,
        "alternate-prefix",
        OWN_REMOVAL_AT
    );
    template_removal_owner_case!(
        canonical_template_removal_owner_alternate_prefix_at_owner,
        "alternate-prefix",
        OWN_REMOVAL_OWNER_AT
    );
    template_removal_owner_case!(
        canonical_template_removal_owner_encoded_name_at_template,
        "encoded-name",
        OWN_REMOVAL_AT
    );
    template_removal_owner_case!(
        canonical_template_removal_owner_encoded_name_at_owner,
        "encoded-name",
        OWN_REMOVAL_OWNER_AT
    );
    template_removal_owner_case!(
        canonical_template_removal_owner_comment_shadow_reference_at_template,
        "comment-shadow-reference",
        OWN_REMOVAL_AT
    );
    template_removal_owner_case!(
        canonical_template_removal_owner_comment_shadow_reference_at_owner,
        "comment-shadow-reference",
        OWN_REMOVAL_OWNER_AT
    );

    #[test]
    fn canonical_dcs_template_removal_clears_only_the_real_reference_and_replays() {
        const TEMPLATE_AT: &str = "cf:Report.F05Report.Template.F05Schema";
        const OWNER_AT: &str = "cf:Report.F05Report";
        const OWNER: &str = "cf/Reports/F05Report.xml";
        const DESCRIPTOR: &str = "cf/Reports/F05Report/Templates/F05Schema.xml";
        const PAYLOAD: &str = "cf/Reports/F05Report/Templates/F05Schema";
        const TEMPLATES: &str = "cf/Reports/F05Report/Templates";
        const BODY: &str = "cf/Reports/F05Report/Templates/F05Schema/Ext/Template.xml";
        const PROPERTIES_START: &str = "<Properties><Name>F05Report</Name>";
        const REGISTRATION: &str = "<Template>F05Schema</Template>";
        const FILLED_REFERENCE: &str = "<MainDataCompositionSchema>Report.F05Report.Template.F05Schema</MainDataCompositionSchema>";

        for top_at in [TEMPLATE_AT, OWNER_AT] {
            let workspace = template_removal_workspace_from_fixture("acceptance/workspace-dcs");
            let source = workspace.path().join("src");
            let owner_path = source.join(OWNER);
            let original_owner = std::fs::read_to_string(&owner_path).unwrap();
            assert_eq!(original_owner.matches(PROPERTIES_START).count(), 1);
            assert_eq!(original_owner.matches(REGISTRATION).count(), 1);
            assert_eq!(original_owner.matches(FILLED_REFERENCE).count(), 0);

            // The complete synthetic DCS fixture already owns this real DCS
            // wrapper and payload. Adding a matching owner reference is a
            // deliberate test-input variation, not a native-export claim.
            // The preceding comment carries the same spelling and must survive.
            let comment_shadow = format!("<!-- preserve-reference-shadow {FILLED_REFERENCE} -->");
            let prepared_properties =
                format!("{PROPERTIES_START}{comment_shadow}{FILLED_REFERENCE}");
            let expected_properties =
                format!("{PROPERTIES_START}{comment_shadow}<MainDataCompositionSchema/>");
            let prepared_owner =
                original_owner.replacen(PROPERTIES_START, prepared_properties.as_str(), 1);
            // The only registration leaves the collection in the native empty
            // spelling, as the 8.3.27 export writes it.
            let expected_owner = original_owner
                .replacen(PROPERTIES_START, expected_properties.as_str(), 1)
                .replacen(
                    &format!("<ChildObjects>{REGISTRATION}</ChildObjects>"),
                    "<ChildObjects/>",
                    1,
                )
                .into_bytes();
            std::fs::write(&owner_path, prepared_owner.as_bytes()).unwrap();

            let before = own_removal_snapshot(&source);
            let mut expected_files = own_removal_file_bytes(&before);
            expected_files.insert(OWNER.into(), expected_owner);
            assert!(expected_files
                .remove(std::path::Path::new(DESCRIPTOR))
                .is_some());
            assert!(expected_files.remove(std::path::Path::new(BODY)).is_some());

            let runtime = bootstrap_runtime();
            let preview = submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::Apply,
                serde_json::json!({
                    "at": top_at,
                    "ops": [{"op": "template.remove", "args": {"at": TEMPLATE_AT}}],
                }),
            );
            assert!(preview.ok, "{top_at}: {preview:?}");
            assert_eq!(preview.data.as_ref().unwrap()["mode"], "preview");
            assert_eq!(own_removal_snapshot(&source), before);
            let token = preview.data.as_ref().unwrap()["executionToken"].clone();
            assert!(!token.as_str().unwrap().is_empty());

            let applied = submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::Apply,
                serde_json::json!({"executionToken": token.clone()}),
            );
            assert!(applied.ok, "{top_at}: {applied:?}");
            assert_eq!(applied.data.as_ref().unwrap()["mode"], "published");
            let after = own_removal_snapshot(&source);
            assert_eq!(own_removal_file_bytes(&after), expected_files);
            // The payload directories and the `Templates` collection the sole
            // template leaves empty go with it (#1997); the report directory
            // and every other entry outside them stay.
            assert!(!after.contains_key(std::path::Path::new(PAYLOAD)));
            assert!(!after.contains_key(std::path::Path::new(TEMPLATES)));
            assert_eq!(
                after.get(std::path::Path::new("cf/Reports/F05Report")),
                Some(&OwnRemovalEntry::Directory)
            );
            for (path, entry) in &after {
                assert!(before.contains_key(path), "new postimage entry: {path:?}");
                match entry {
                    OwnRemovalEntry::Directory => {
                        assert_eq!(before.get(path), Some(entry));
                    }
                    OwnRemovalEntry::Link(_) => {
                        panic!("unexpected link/reparse point: {path:?}");
                    }
                    OwnRemovalEntry::File(_) => {}
                }
            }
            for (path, entry) in &before {
                if !path.starts_with(PAYLOAD)
                    && path != std::path::Path::new(TEMPLATES)
                    && path != std::path::Path::new(DESCRIPTOR)
                    && path != std::path::Path::new(OWNER)
                {
                    assert_eq!(
                        after.get(path),
                        Some(entry),
                        "changed unrelated source entry: {path:?}"
                    );
                }
            }

            let replay = submit_canonical(
                &runtime,
                workspace.path(),
                ToolIdentity::Apply,
                serde_json::json!({"executionToken": token}),
            );
            assert_eq!(
                serde_json::to_value(&replay).unwrap(),
                serde_json::to_value(&applied).unwrap()
            );
            assert_eq!(own_removal_snapshot(&source), after);
        }
    }
}

/// #1251: no production operation path may start a finite `ProviderDeadline`
/// again, under its own name or an alias, outside the listed per-step limits.
#[cfg(test)]
mod finite_deadline_guard {

    /// Constructors that start a finite operation deadline.
    const FINITE_DEADLINE_CONSTRUCTORS: [&str; 3] = ["from_budget", "new", "from_started_at"];

    /// Production places that may construct a finite `ProviderDeadline`
    /// (#1251). Each entry is a per-step safety bound, a deadline the user set
    /// explicitly, an adapter of an existing caller cutoff, a path the v0.13
    /// wire does not reach, or search work moved to #1254. An existing-cutoff
    /// adapter must have a test that rejects rearming its input as a budget.
    /// A new constructor anywhere, or one more in a listed file, fails here.
    const FINITE_DEADLINE_ALLOW_LIST: &[(&str, usize, &str)] = &[
        (
            "crates/unica-code-intelligence/src/coordinator.rs",
            2,
            "search and provider reads: #1254",
        ),
        (
            "crates/unica-diagnostics/src/coordinator.rs",
            2,
            "explicit analyze timeout; resident actions of the retired code.diagnostics",
        ),
        (
            "crates/unica-coder/src/application/mod.rs",
            1,
            "PUBLIC_INVOCATION_DEADLINE: legacy v0.12 surface, unreachable from the v0.13 wire",
        ),
        (
            "crates/unica-coder/src/infrastructure/application_ports.rs",
            1,
            "NATIVE_TYPED_INVOCATION_DEADLINE: legacy v0.12 surface",
        ),
        (
            "crates/unica-coder/src/infrastructure/daemon/invocation_service.rs",
            1,
            "saved apply adapts the existing caller cutoff; saved_apply_adapter_preserves_the_existing_cutoff_and_absence rejects rearming",
        ),
        (
            "crates/unica-execution/src/receipt_runtime.rs",
            27,
            "per-step Task store and lifecycle-link I/O bounds",
        ),
    ];

    /// `cfg(test)`, `cfg(all(test, ..))` or the receipt-ledger harness
    /// feature; a condition such as `cfg(any(test, feature = ".."))` also
    /// compiles outside tests and stays production code.
    fn is_test_only(attrs: &[syn::Attribute]) -> bool {
        attrs.iter().any(|attr| {
            let syn::Meta::List(list) = &attr.meta else {
                return false;
            };
            let condition = list.tokens.to_string();
            attr.path().is_ident("cfg")
                && (condition == "test"
                    || condition.starts_with("all (test ,")
                    || condition == "feature = \"receipt-ledger-test-support\"")
        })
    }

    /// Calls `<ProviderDeadline or an alias of it>::<finite constructor>(..)`
    /// in production code: test-only items and modules are skipped.
    #[derive(Default)]
    struct FiniteDeadlineCalls {
        aliases: Vec<String>,
        calls: usize,
    }

    impl<'ast> syn::visit::Visit<'ast> for FiniteDeadlineCalls {
        fn visit_item(&mut self, item: &'ast syn::Item) {
            let attrs = match item {
                syn::Item::Fn(item) => &item.attrs,
                syn::Item::Mod(item) => &item.attrs,
                syn::Item::Impl(item) => &item.attrs,
                syn::Item::Const(item) => &item.attrs,
                syn::Item::Static(item) => &item.attrs,
                syn::Item::Struct(item) => &item.attrs,
                syn::Item::Use(item) => &item.attrs,
                syn::Item::Macro(item) => &item.attrs,
                _ => return syn::visit::visit_item(self, item),
            };
            if !is_test_only(attrs) {
                syn::visit::visit_item(self, item);
            }
        }

        fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
            if !is_test_only(&item.attrs) {
                syn::visit::visit_impl_item_fn(self, item);
            }
        }

        fn visit_stmt(&mut self, stmt: &'ast syn::Stmt) {
            let attrs = match stmt {
                syn::Stmt::Local(local) => &local.attrs,
                syn::Stmt::Expr(syn::Expr::If(expr), _) => &expr.attrs,
                syn::Stmt::Expr(syn::Expr::Call(expr), _) => &expr.attrs,
                syn::Stmt::Expr(syn::Expr::MethodCall(expr), _) => &expr.attrs,
                syn::Stmt::Macro(stmt) => &stmt.attrs,
                _ => return syn::visit::visit_stmt(self, stmt),
            };
            if !is_test_only(attrs) {
                syn::visit::visit_stmt(self, stmt);
            }
        }

        fn visit_use_rename(&mut self, rename: &'ast syn::UseRename) {
            if rename.ident == "ProviderDeadline" {
                self.aliases.push(rename.rename.to_string());
            }
        }

        fn visit_expr_path(&mut self, expr: &'ast syn::ExprPath) {
            let segments = expr
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>();
            if let [.., owner, constructor] = segments.as_slice() {
                if (owner == "ProviderDeadline" || self.aliases.contains(owner))
                    && FINITE_DEADLINE_CONSTRUCTORS.contains(&constructor.as_str())
                {
                    self.calls += 1;
                }
            }
            syn::visit::visit_expr_path(self, expr);
        }
    }

    fn finite_deadline_calls(source: &str) -> usize {
        let file = syn::parse_file(source).expect("production source parses");
        if is_test_only(&file.attrs) {
            return 0;
        }
        let mut calls = FiniteDeadlineCalls::default();
        syn::visit::Visit::visit_file(&mut calls, &file);
        calls.calls
    }

    /// Files that compile only into tests or the receipt-ledger test harness.
    fn is_test_source(relative: &str) -> bool {
        let name = relative.rsplit('/').next().unwrap_or(relative);
        name == "tests.rs"
            || name.ends_with("_tests.rs")
            || name == "test_support.rs"
            || name == "test_control.rs"
            || name == "testing.rs"
            || relative.contains("/tests/")
            || relative.contains("receipt_scenario_v5")
    }

    #[test]
    fn production_operation_paths_start_no_new_finite_deadline() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut pending = std::fs::read_dir(root.join("crates"))
            .unwrap()
            .map(|entry| entry.unwrap().path().join("src"))
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        let mut found = std::collections::BTreeMap::new();
        while let Some(directory) = pending.pop() {
            for entry in std::fs::read_dir(&directory).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                    continue;
                }
                if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
                    continue;
                }
                let relative = path
                    .strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                if is_test_source(&relative) {
                    continue;
                }
                let source = std::fs::read_to_string(&path).unwrap();
                // Вызов возможен только через `ProviderDeadline` или его
                // псевдоним из `use` того же файла: без этого имени разбирать
                // файл нечего, а разбор всего workspace стоил секунды (#1920).
                if !source.contains("ProviderDeadline") {
                    continue;
                }
                let calls = finite_deadline_calls(&source);
                if calls > 0 {
                    found.insert(relative, calls);
                }
            }
        }
        let allowed = FINITE_DEADLINE_ALLOW_LIST
            .iter()
            .map(|(file, calls, _)| (file.to_string(), *calls))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            found, allowed,
            "a production path starts a finite operation deadline outside the per-step allow-list"
        );
    }

    #[test]
    fn finite_deadline_guard_honors_whole_file_cfg_without_hiding_operation_paths() {
        for source in [
            r#"#![cfg(feature = "receipt-ledger-test-support")]
fn contract() { let _ = ProviderDeadline::new(deadline); }"#,
            r#"#![cfg(test)]
fn fixture() { let _ = ProviderDeadline::from_budget(B); }"#,
            r#"#![cfg(all(test, target_os = "macos"))]
fn fixture() { let _ = ProviderDeadline::from_budget(B); }"#,
        ] {
            assert_eq!(
                finite_deadline_calls(source),
                0,
                "the file's compiler gate applies to every item: {source}"
            );
        }
        for source in [
            "fn contract() { let _ = ProviderDeadline::new(deadline); }",
            r#"#![cfg(any(test, feature = "receipt-ledger-test-support"))]
fn contract() { let _ = ProviderDeadline::new(deadline); }"#,
            r#"#![cfg(not(test))]
fn operation() { let _ = ProviderDeadline::new(deadline); }"#,
            r#"#![cfg(feature = "other")]
fn operation() { let _ = ProviderDeadline::new(deadline); }"#,
            r#"#![cfg(not(feature = "receipt-ledger-test-support"))]
fn operation() { let _ = ProviderDeadline::new(deadline); }"#,
            r#"#![cfg(feature = "other")]
use unica_work_control::provider_deadline::ProviderDeadline as Budget;
fn operation() { let _ = Budget::from_budget(B); }"#,
        ] {
            assert_eq!(
                finite_deadline_calls(source),
                1,
                "an unguarded or conservatively production file stays checked: {source}"
            );
        }
    }

    #[test]
    fn finite_deadline_guard_recognizes_aliases_and_skips_only_test_code() {
        assert_eq!(
            finite_deadline_calls(
                "fn read() { let _ = ProviderDeadline::from_budget(Duration::from_secs(120)); }"
            ),
            1
        );
        assert_eq!(
            finite_deadline_calls(
                "fn read() { let _ = unica_work_control::provider_deadline::ProviderDeadline::new(Instant::now()); }"
            ),
            1
        );
        assert_eq!(
            finite_deadline_calls(
                "use unica_work_control::provider_deadline::ProviderDeadline as Budget;\nfn read() { let _ = Budget::from_started_at(Instant::now(), BUDGET); }"
            ),
            1,
            "a renamed import must not hide a new deadline"
        );
        assert_eq!(
            finite_deadline_calls("fn read() { let _ = ProviderDeadline::no_deadline(); }"),
            0
        );
        assert_eq!(
            finite_deadline_calls(
                "#[cfg(test)]\nmod tests { fn fixture() { let _ = ProviderDeadline::from_budget(B); } }"
            ),
            0
        );
        assert_eq!(
            finite_deadline_calls(
                "#[cfg(any(test, feature = \"hostile\"))]\nfn escape() { let _ = ProviderDeadline::from_budget(B); }"
            ),
            1,
            "a condition that also compiles outside tests stays production code"
        );
        assert_eq!(
            finite_deadline_calls(
                "#[cfg(not(test))]\nfn production() { let _ = ProviderDeadline::from_budget(B); }"
            ),
            1
        );
        assert_eq!(
            finite_deadline_calls(
                "#[cfg(all(test, target_os = \"macos\"))]\nmod tests { fn f() { let _ = ProviderDeadline::from_budget(B); } }"
            ),
            0
        );
    }
}
