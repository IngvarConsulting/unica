use super::identity::{CoreIdentity, DaemonStateDirectory};
use super::protocol::{
    parse_request, read_bounded_request_line, ClientRequest, DaemonErrorCode, DaemonTaskSnapshot,
    EndpointRecord, InvocationRequest, InvocationResponse, ServerResponse, DAEMON_PROTOCOL_VERSION,
};
use crate::application::invocation::{
    normalized_arguments_hash, InvocationExecutor, InvocationExecutorError,
    InvocationResponseDeadline, PreparedDaemonInvocation, RESPONSE_SERIALIZATION_MARGIN_MS,
};
use crate::application::invocation_store::{InvocationStore, InvocationStoreError};
use crate::application::operation_descriptors::ExecutionClass;
use crate::application::ports::{Clock, TokioClock};
use crate::application::shared_work::{
    LongWorkFailure, ProviderHostKey, ProviderHostOwner, SharedWorkLease, SharedWorkProducer,
};
use crate::application::tool_contracts::SurfaceRelease;
use crate::application::v13::LOGICAL_READ_OPERATION_BUDGET;
use crate::composition::open_daemon_invocation_store_from_directory;
use crate::domain::address::QualifiedAddress;
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::invocation::{
    DomainResult, InvocationFailure, InvocationOutcome, SafeIdentityHash,
};
use crate::domain::project_sources::{SourceFormat, SourceProfile, SourceSetKind};
use crate::infrastructure::project_sources::discover_project_source_map;
use crate::infrastructure::runtime_jobs::{RuntimeJobService, RuntimeResourceOwner};
use crate::infrastructure::source_roots::normalize_contained_source_root;
use crate::infrastructure::workspace::discover_workspace;
use crate::infrastructure::workspace_actor::{
    IndexWorkIdentity, ProviderRootBinding, WorkspaceActor, WorkspaceActorRegistry,
    WorkspaceActorRegistryError, WorkspaceLogicalReadFence, WorkspaceRevisionFence,
    WorkspaceSourceSetInput,
};
use std::collections::HashSet;
use std::io::{self, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(20);
const CONNECTION_READ_TIMEOUT: Duration = Duration::from_millis(100);
const HANDSHAKE_READ_TIMEOUT: Duration = Duration::from_secs(2);
const OWNER_RESPONSE_WRITE_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const MAX_HANDSHAKES: usize = 8;
pub(crate) const MAX_OWNER_SESSIONS: usize = 64;
const ACTOR_OPERATION_BUDGET: Duration = Duration::from_secs(7);

#[derive(Clone)]
pub(crate) struct ActorBoundInvocation {
    tool: crate::application::invocation_store::ToolIdentity,
    arguments: serde_json::Map<String, serde_json::Value>,
    response_deadline: InvocationResponseDeadline,
    actor: Arc<WorkspaceActor>,
    provider_root: ProviderRootBinding,
    #[allow(dead_code)] // consumed only by the injected Task 14 service before Task 22
    read_sources: Arc<[ActorReadSourceBinding]>,
    workspace_identity_hash: SafeIdentityHash,
    deliveries: Arc<crate::infrastructure::engine_delivery::DeliveryDesk>,
    provider_hosts: Arc<ProviderHostOwner>,
    runtime_resources: Arc<RuntimeResourceOwner>,
    runtime_service: Option<Arc<RuntimeJobService>>,
}

#[allow(dead_code)]
#[derive(Clone)]
struct ActorReadSourceBinding {
    binding: ProviderRootBinding,
}

#[allow(dead_code)]
pub(super) struct ActorReadSourceCapability {
    binding: ProviderRootBinding,
    identity: String,
    fence: WorkspaceLogicalReadFence,
    deadline: ProviderDeadline,
}

#[allow(dead_code)]
impl ActorReadSourceCapability {
    pub(super) fn source_set_name(&self) -> &str {
        self.binding.source_set_name()
    }

    const fn source_kind(&self) -> SourceSetKind {
        self.binding.source_kind()
    }

    const fn source_format(&self) -> SourceFormat {
        self.binding.source_format()
    }

    const fn source_profile(&self) -> SourceProfile {
        self.binding.source_profile()
    }

    pub(super) const fn deadline(&self) -> ProviderDeadline {
        self.deadline
    }

    pub(super) fn logical_view_read_authority<'a>(
        &self,
        cancellation: &'a CancellationToken,
    ) -> Result<crate::infrastructure::v13_read::LogicalViewReadAuthority<'a>, String> {
        let platform_profile = self.source_profile().platform_profile().ok_or_else(|| {
            "actor-bound logical source has no supported platform profile".to_string()
        })?;
        let read =
            crate::infrastructure::v13_read_port::ProviderReadAuthority::new_with_revision_lease(
                self.source_set_name(),
                self.identity.clone(),
                self.source_kind(),
                self.binding.retained_root(),
                self.fence.revision(),
            );
        Ok(
            crate::infrastructure::v13_read::LogicalViewReadAuthority::with_read_authority(
                cancellation,
                read,
                platform_profile,
                self.deadline,
            ),
        )
    }
}

struct ActorLogicalReadSourceLease {
    binding: ProviderRootBinding,
    fence: WorkspaceLogicalReadFence,
}

struct ActorLogicalReadLease {
    deadline: ProviderDeadline,
    sources: Vec<ActorLogicalReadSourceLease>,
    route: ActorLogicalReadRoute,
}

enum ActorLogicalReadRoute {
    Admitted,
    Rejected(Box<DomainResult>),
}

impl ActorBoundInvocation {
    pub(crate) fn tool(&self) -> crate::application::invocation_store::ToolIdentity {
        self.tool
    }

    pub(crate) fn arguments(&self) -> &serde_json::Map<String, serde_json::Value> {
        &self.arguments
    }

    pub(crate) fn workspace_identity_hash(&self) -> &SafeIdentityHash {
        &self.workspace_identity_hash
    }

    fn begin_execution(
        self,
        cancellation: &CancellationToken,
    ) -> Result<ActorBoundExecution, String> {
        self.begin_execution_with_logical_deadline(
            cancellation,
            ProviderDeadline::from_budget(LOGICAL_READ_OPERATION_BUDGET),
        )
    }

    fn begin_execution_with_logical_deadline(
        self,
        cancellation: &CancellationToken,
        logical_deadline: ProviderDeadline,
    ) -> Result<ActorBoundExecution, String> {
        let revision = match self.tool {
            crate::application::invocation_store::ToolIdentity::View
            | crate::application::invocation_store::ToolIdentity::Find => {
                let (selected, route) = match self.tool {
                    crate::application::invocation_store::ToolIdentity::View => {
                        match self.arguments.get("at").and_then(serde_json::Value::as_str) {
                            Some(at) => match QualifiedAddress::parse(at) {
                                Ok(address) => match self
                                .read_sources
                                .iter()
                                .find(|source| {
                                    source.binding.source_set_name() == address.source_set()
                                })
                            {
                                Some(source) => (vec![source], ActorLogicalReadRoute::Admitted),
                                    None => (
                                        Vec::new(),
                                        ActorLogicalReadRoute::Rejected(Box::new(
                                            DomainResult::canonical_rejection(
                                                Some(at.to_string()),
                                                "provider_unavailable",
                                                "view source set was not admitted by the workspace actor",
                                            ),
                                        )),
                                    ),
                                },
                                Err(error) => (
                                    Vec::new(),
                                    ActorLogicalReadRoute::Rejected(Box::new(
                                        DomainResult::canonical_rejection(
                                            Some(at.to_string()),
                                            "bad_value",
                                            error.to_string(),
                                        ),
                                    )),
                                ),
                            },
                            None => (
                                Vec::new(),
                                ActorLogicalReadRoute::Rejected(Box::new(
                                    DomainResult::canonical_rejection(
                                        None,
                                        "bad_value",
                                        "view requires string argument `at`",
                                    ),
                                )),
                            ),
                        }
                    }
                    _ => (
                        self.read_sources.iter().collect(),
                        ActorLogicalReadRoute::Admitted,
                    ),
                };
                if matches!(route, ActorLogicalReadRoute::Admitted) && selected.is_empty() {
                    return Err(
                        "logical read admission selected no actor-owned source sets".to_string()
                    );
                }
                let mut sources = Vec::with_capacity(selected.len());
                for source in selected {
                    let fence = self.actor.capture_logical_read_revision(
                        &source.binding,
                        logical_deadline,
                        cancellation,
                    )?;
                    sources.push(ActorLogicalReadSourceLease {
                        binding: source.binding.clone(),
                        fence,
                    });
                }
                ActorExecutionRevision::LogicalRead(ActorLogicalReadLease {
                    deadline: logical_deadline,
                    sources,
                    route,
                })
            }
            _ => ActorExecutionRevision::Legacy(self.actor.capture_revision(
                &self.provider_root,
                ProviderDeadline::from_budget(ACTOR_OPERATION_BUDGET),
                cancellation,
            )?),
        };
        Ok(ActorBoundExecution {
            invocation: self,
            revision,
        })
    }
}

pub(crate) struct ActorBoundExecution {
    invocation: ActorBoundInvocation,
    revision: ActorExecutionRevision,
}

enum ActorExecutionRevision {
    Legacy(WorkspaceRevisionFence),
    LogicalRead(ActorLogicalReadLease),
}

impl ActorBoundExecution {
    // The hidden V13 profile installs real consumers in Tasks 10-21. Until the
    // Task 22 cutover, only injected canonical services exercise this narrow
    // capability surface.
    #[allow(dead_code)]
    pub(crate) fn tool(&self) -> crate::application::invocation_store::ToolIdentity {
        self.invocation.tool()
    }

    #[allow(dead_code)]
    pub(crate) fn arguments(&self) -> &serde_json::Map<String, serde_json::Value> {
        self.invocation.arguments()
    }

    #[allow(dead_code)]
    pub(crate) fn workspace_identity_hash(&self) -> &SafeIdentityHash {
        self.invocation.workspace_identity_hash()
    }

    #[allow(dead_code)]
    pub(super) fn read_sources(&self) -> Result<Vec<ActorReadSourceCapability>, String> {
        let ActorExecutionRevision::LogicalRead(lease) = &self.revision else {
            return Err("logical read sources are unavailable to a legacy invocation".to_string());
        };
        lease
            .sources
            .iter()
            .map(|source| {
                self.invocation.actor.validate_binding(&source.binding)?;
                Ok(ActorReadSourceCapability {
                    binding: source.binding.clone(),
                    identity: format!(
                        "{}:{}",
                        self.invocation.workspace_identity_hash.as_str(),
                        source.binding.source_set_name()
                    ),
                    fence: source.fence.clone(),
                    deadline: lease.deadline,
                })
            })
            .collect()
    }

    pub(crate) fn rejected_logical_read_result(&self) -> Option<DomainResult> {
        let ActorExecutionRevision::LogicalRead(lease) = &self.revision else {
            return None;
        };
        match &lease.route {
            ActorLogicalReadRoute::Admitted => None,
            ActorLogicalReadRoute::Rejected(result) => Some(result.as_ref().clone()),
        }
    }

    /// Daemon-owned exact delivery capability. Canonical work may wait here
    /// only after request admission has become an Invocation or durable Task.
    #[allow(dead_code)]
    pub(crate) fn delivery_work(&self) -> &crate::infrastructure::engine_delivery::DeliveryDesk {
        &self.invocation.deliveries
    }

    #[allow(dead_code)]
    pub(crate) fn join_index_work<W>(
        &self,
        provider: &str,
        profile: &str,
        work: W,
    ) -> Result<(IndexWorkIdentity, SharedWorkLease<(), LongWorkFailure>), String>
    where
        W: FnOnce(SharedWorkProducer) -> Result<(), LongWorkFailure> + Send + 'static,
    {
        let ActorExecutionRevision::Legacy(revision) = &self.revision else {
            return Err("index work is unavailable to a logical read invocation".to_string());
        };
        self.invocation.actor.join_index_work(
            &self.invocation.provider_root,
            revision,
            provider,
            profile,
            work,
        )
    }

    #[allow(dead_code)]
    pub(crate) fn join_provider_host<W>(
        &self,
        key: ProviderHostKey,
        work: W,
    ) -> SharedWorkLease<(), LongWorkFailure>
    where
        W: FnOnce(SharedWorkProducer) -> Result<(), LongWorkFailure> + Send + 'static,
    {
        self.invocation.provider_hosts.join_or_start(key, work)
    }

    #[allow(dead_code)]
    pub(crate) fn join_runtime_resource<W>(
        &self,
        lease_id: &str,
        work: W,
    ) -> Result<SharedWorkLease<(), LongWorkFailure>, String>
    where
        W: FnOnce(SharedWorkProducer) -> Result<(), LongWorkFailure> + Send + 'static,
    {
        let service = self.invocation.runtime_service.as_ref().ok_or_else(|| {
            "runtime resource capability is not admitted for this daemon invocation".to_string()
        })?;
        service.join_shared_work(lease_id, &self.invocation.runtime_resources, work)
    }

    #[allow(dead_code)]
    pub(crate) fn read_relative_file(
        &self,
        relative: &std::path::Path,
        max_bytes: usize,
    ) -> Result<Vec<u8>, String> {
        self.invocation.actor.read_relative_file(
            &self.invocation.provider_root,
            relative,
            max_bytes,
        )
    }

    #[cfg(test)]
    fn mark_source_revision_dirty_for_test(&self) {
        self.invocation.actor.mark_source_revisions_dirty();
    }

    fn publish(
        self,
        staged: Result<DomainResult, InvocationFailure>,
        cancellation: &CancellationToken,
    ) -> Result<Result<DomainResult, InvocationFailure>, String> {
        match self.revision {
            ActorExecutionRevision::Legacy(revision) => self
                .invocation
                .actor
                .begin_publication(
                    &revision,
                    ProviderDeadline::from_budget(ACTOR_OPERATION_BUDGET),
                    cancellation,
                )?
                .publish(
                    staged,
                    ProviderDeadline::from_budget(ACTOR_OPERATION_BUDGET),
                    cancellation,
                ),
            ActorExecutionRevision::LogicalRead(lease) => {
                if let ActorLogicalReadRoute::Rejected(expected) = lease.route {
                    let is_closed_typed_rejection = staged.as_ref() == Ok(expected.as_ref());
                    return is_closed_typed_rejection.then_some(staged).ok_or_else(|| {
                        "logical view input rejection produced an unexpected terminal result"
                            .to_string()
                    });
                }
                let fences = lease
                    .sources
                    .into_iter()
                    .map(|source| source.fence)
                    .collect::<Vec<_>>();
                self.invocation.actor.publish_logical_read(
                    &fences,
                    staged,
                    lease.deadline,
                    cancellation,
                )
            }
        }
    }
}

pub(crate) trait CanonicalInvocationService: Send + Sync {
    fn prepare(
        &self,
        invocation: &ActorBoundInvocation,
    ) -> Result<ExecutionClass, Box<DomainResult>>;

    fn execute(
        &self,
        invocation: &ActorBoundExecution,
        cancellation: CancellationToken,
    ) -> Result<DomainResult, InvocationFailure>;
}

struct DormantCanonicalV13Service;

impl CanonicalInvocationService for DormantCanonicalV13Service {
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
        Ok(failed_domain_result(
            "canonical v0.13 handler is not installed before the Task 22 cutover",
        ))
    }
}

fn bind_workspace_invocation(
    request: &InvocationRequest,
    actors: &WorkspaceActorRegistry,
    deliveries: Arc<crate::infrastructure::engine_delivery::DeliveryDesk>,
    provider_hosts: Arc<ProviderHostOwner>,
    runtime_resources: Arc<RuntimeResourceOwner>,
    runtime_service: Option<Arc<RuntimeJobService>>,
    response_deadline: InvocationResponseDeadline,
) -> Result<ActorBoundInvocation, WorkspaceAdmissionError> {
    bind_workspace_invocation_controlled(
        request,
        actors,
        ActorInvocationResources {
            deliveries,
            provider_hosts,
            runtime_resources,
            runtime_service,
        },
        response_deadline,
        |_| {},
    )
}

struct ActorInvocationResources {
    deliveries: Arc<crate::infrastructure::engine_delivery::DeliveryDesk>,
    provider_hosts: Arc<ProviderHostOwner>,
    runtime_resources: Arc<RuntimeResourceOwner>,
    runtime_service: Option<Arc<RuntimeJobService>>,
}

#[derive(Clone)]
struct DiscoveredActorSource {
    name: String,
    kind: SourceSetKind,
    source_format: SourceFormat,
    source_profile: SourceProfile,
    root: std::path::PathBuf,
}

fn bind_workspace_invocation_controlled(
    request: &InvocationRequest,
    actors: &WorkspaceActorRegistry,
    resources: ActorInvocationResources,
    response_deadline: InvocationResponseDeadline,
    after_actor_admission: impl FnOnce(&mut [DiscoveredActorSource]),
) -> Result<ActorBoundInvocation, WorkspaceAdmissionError> {
    let context = discover_workspace(Some(std::path::PathBuf::from(request.workspace_hint())))
        .map_err(|_| WorkspaceAdmissionError::Invalid)?;
    let source_map = discover_project_source_map(&context.workspace_root)
        .map_err(|_| WorkspaceAdmissionError::Invalid)?;
    let mut admitted_sources = source_map
        .source_sets
        .into_iter()
        .filter(|source| source.source_format == SourceFormat::PlatformXml)
        .map(|source| {
            let root = normalize_contained_source_root(&context.workspace_root, &source.path)
                .map_err(|_| WorkspaceAdmissionError::Invalid)?;
            Ok(DiscoveredActorSource {
                name: source.name,
                kind: source.kind,
                source_format: source.source_format,
                source_profile: SourceProfile::platform_xml_8_3_27_format_2_20(),
                root,
            })
        })
        .collect::<Result<Vec<_>, WorkspaceAdmissionError>>()?;
    if admitted_sources.is_empty() {
        return Err(WorkspaceAdmissionError::Invalid);
    }
    admitted_sources.sort_by(|left, right| left.name.cmp(&right.name));
    let actor = actors
        .get_or_create(
            &context,
            admitted_sources.iter().map(|source| {
                WorkspaceSourceSetInput::new(
                    source.name.clone(),
                    source.root.clone(),
                    source.kind,
                    source.source_format,
                    source.source_profile,
                )
            }),
            "canonical-v0.13",
        )
        .map_err(|error| match error {
            WorkspaceActorRegistryError::Capacity { .. } => WorkspaceAdmissionError::Capacity,
            WorkspaceActorRegistryError::InvalidIdentity(_) => WorkspaceAdmissionError::Invalid,
            WorkspaceActorRegistryError::Poisoned => WorkspaceAdmissionError::RegistryFailed,
        })?;
    after_actor_admission(&mut admitted_sources);
    let read_sources = admitted_sources
        .iter()
        .map(|source| {
            actor
                .bind_provider_root(&source.name, &source.root)
                .map(|binding| ActorReadSourceBinding { binding })
                .map_err(|_| WorkspaceAdmissionError::Invalid)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let provider_root = read_sources
        .iter()
        .find(|source| source.binding.source_set_name() == "main")
        .or_else(|| read_sources.first())
        .map(|source| source.binding.clone())
        .ok_or(WorkspaceAdmissionError::Invalid)?;
    let workspace_identity_hash = actor
        .safe_identity_hash()
        .map_err(|_| WorkspaceAdmissionError::Invalid)?;
    Ok(ActorBoundInvocation {
        tool: request.tool(),
        arguments: request.arguments().clone(),
        response_deadline,
        actor,
        provider_root,
        read_sources: Arc::from(read_sources),
        workspace_identity_hash,
        deliveries: resources.deliveries,
        provider_hosts: resources.provider_hosts,
        runtime_resources: resources.runtime_resources,
        runtime_service: resources.runtime_service,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkspaceAdmissionError {
    Capacity,
    RegistryFailed,
    Invalid,
}

#[derive(Debug)]
enum DaemonInvocationError {
    WorkspaceCapacity,
    WorkspaceRegistryFailed,
    Executor(InvocationExecutorError),
}

impl From<InvocationExecutorError> for DaemonInvocationError {
    fn from(error: InvocationExecutorError) -> Self {
        Self::Executor(error)
    }
}

impl DaemonInvocationError {
    fn protocol_code(&self) -> DaemonErrorCode {
        match self {
            Self::WorkspaceCapacity => DaemonErrorCode::WorkspaceCapacity,
            Self::WorkspaceRegistryFailed => DaemonErrorCode::WorkspaceRegistryFailed,
            Self::Executor(InvocationExecutorError::Store(InvocationStoreError::NotFound)) => {
                DaemonErrorCode::TaskNotFound
            }
            Self::Executor(InvocationExecutorError::Store(InvocationStoreError::Expired)) => {
                DaemonErrorCode::TaskExpired
            }
            Self::Executor(InvocationExecutorError::Store(InvocationStoreError::Capacity {
                ..
            })) => DaemonErrorCode::TaskCapacity,
            Self::Executor(InvocationExecutorError::Store(
                InvocationStoreError::ResultTooLarge { .. },
            ))
            | Self::Executor(InvocationExecutorError::ResultTooLarge) => {
                DaemonErrorCode::ResultTooLarge
            }
            Self::Executor(InvocationExecutorError::Store(_)) => DaemonErrorCode::StoreFailed,
            Self::Executor(InvocationExecutorError::ExecutionFailed) => {
                DaemonErrorCode::InvocationFailed
            }
            Self::Executor(InvocationExecutorError::DeadlineAuthorityMismatch) => {
                DaemonErrorCode::InvocationFailed
            }
            Self::Executor(InvocationExecutorError::StatePoisoned) => {
                DaemonErrorCode::InvocationFailed
            }
            Self::Executor(InvocationExecutorError::RestartRequested) => {
                DaemonErrorCode::DurabilityUncertain
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn workspace_capacity_protocol_code_for_test() -> DaemonErrorCode {
    DaemonInvocationError::WorkspaceCapacity.protocol_code()
}

fn failed_domain_result(summary: &str) -> DomainResult {
    let mut result = DomainResult::success(summary);
    result.ok = false;
    result
}

fn validate_hidden_v13_request(request: &InvocationRequest) -> Result<(), String> {
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
    let expected = schema.get("type").and_then(serde_json::Value::as_str);
    let type_matches = match expected {
        Some("string") => value.is_string(),
        Some("boolean") => value.is_boolean(),
        Some("integer") => value.as_i64().is_some() || value.as_u64().is_some(),
        Some("object") => value.is_object(),
        Some("array") => value.is_array(),
        Some(_) | None => true,
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

pub(crate) struct DaemonInvocationRuntime {
    executor: Arc<InvocationExecutor>,
    service: Arc<dyn CanonicalInvocationService>,
    workspace_actors: WorkspaceActorRegistry,
    deliveries: Arc<crate::infrastructure::engine_delivery::DeliveryDesk>,
    provider_hosts: Arc<ProviderHostOwner>,
    runtime_resources: Arc<RuntimeResourceOwner>,
    runtime_service: Option<Arc<RuntimeJobService>>,
}

impl DaemonInvocationRuntime {
    fn new(
        store: Arc<dyn InvocationStore>,
        service: Arc<dyn CanonicalInvocationService>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            executor: Arc::new(InvocationExecutor::new(store, clock)),
            service,
            workspace_actors: WorkspaceActorRegistry::default(),
            deliveries: Arc::new(crate::infrastructure::engine_delivery::DeliveryDesk::default()),
            provider_hosts: Arc::new(ProviderHostOwner::default()),
            runtime_resources: Arc::new(RuntimeResourceOwner::default()),
            runtime_service: None,
        }
    }

    #[cfg(test)]
    fn new_with_reconciliation_budget_for_test(
        store: Arc<dyn InvocationStore>,
        service: Arc<dyn CanonicalInvocationService>,
        clock: Arc<dyn Clock>,
        budget: Duration,
    ) -> Self {
        Self {
            executor: Arc::new(InvocationExecutor::new_with_reconciliation_budget_for_test(
                store, clock, budget,
            )),
            service,
            workspace_actors: WorkspaceActorRegistry::default(),
            deliveries: Arc::new(crate::infrastructure::engine_delivery::DeliveryDesk::default()),
            provider_hosts: Arc::new(ProviderHostOwner::default()),
            runtime_resources: Arc::new(RuntimeResourceOwner::default()),
            runtime_service: None,
        }
    }

    #[cfg(test)]
    fn with_runtime_service_for_test(mut self, service: Arc<RuntimeJobService>) -> Self {
        self.runtime_service = Some(service);
        self
    }

    fn capture_response_deadline(&self) -> InvocationResponseDeadline {
        self.executor.capture_response_deadline()
    }

    fn submit(
        &self,
        request: InvocationRequest,
        response_deadline: InvocationResponseDeadline,
    ) -> Result<InvocationResponse, DaemonInvocationError> {
        let actor_bound = match validate_hidden_v13_request(&request) {
            Ok(()) => {
                match bind_workspace_invocation(
                    &request,
                    &self.workspace_actors,
                    Arc::clone(&self.deliveries),
                    Arc::clone(&self.provider_hosts),
                    Arc::clone(&self.runtime_resources),
                    self.runtime_service.clone(),
                    response_deadline.clone(),
                ) {
                    Ok(bound) => Ok(bound),
                    Err(WorkspaceAdmissionError::Capacity) => {
                        return Err(DaemonInvocationError::WorkspaceCapacity)
                    }
                    Err(WorkspaceAdmissionError::RegistryFailed) => {
                        return Err(DaemonInvocationError::WorkspaceRegistryFailed)
                    }
                    Err(WorkspaceAdmissionError::Invalid) => {
                        Err(failed_domain_result("workspace actor admission failed"))
                    }
                }
            }
            Err(summary) => Err(failed_domain_result(&summary)),
        };
        let prepared = match &actor_bound {
            Ok(invocation) => self.service.prepare(invocation).map_err(|result| *result),
            Err(result) => Err(result.clone()),
        }
        .map(|class| {
            let invocation = actor_bound
                .as_ref()
                .expect("successful preparation retains actor-bound invocation");
            PreparedDaemonInvocation::new(
                invocation.tool(),
                normalized_arguments_hash(invocation.arguments()),
                invocation.workspace_identity_hash().clone(),
                class,
                invocation.response_deadline.clone(),
            )
            .with_resource_lease(Arc::new(invocation.clone()))
        });
        let service = Arc::clone(&self.service);
        let execute_invocation = actor_bound.ok();
        self.executor
            .submit_prepared(response_deadline, prepared, move |cancellation| {
                let invocation = execute_invocation.ok_or_else(|| {
                    InvocationFailure::new(
                        "workspace_admission_failed",
                        "workspace actor admission failed",
                    )
                })?;
                let execution = invocation.begin_execution(&cancellation).map_err(|_| {
                    InvocationFailure::new(
                        "workspace_changed",
                        "workspace actor capability changed before execution",
                    )
                })?;
                let outcome = service.execute(&execution, cancellation.clone());
                execution.publish(outcome, &cancellation).map_err(|_| {
                    InvocationFailure::new(
                        "workspace_changed",
                        "workspace actor capability changed before publication",
                    )
                })?
            })
            .map(|outcome| match outcome {
                InvocationOutcome::Direct(result) => InvocationResponse::Direct(result),
                InvocationOutcome::Task(task) => {
                    InvocationResponse::Task(DaemonTaskSnapshot::from_domain(task))
                }
            })
            .map_err(DaemonInvocationError::from)
    }

    fn get(
        &self,
        task_id: crate::domain::invocation::TaskId,
    ) -> Result<DaemonTaskSnapshot, DaemonInvocationError> {
        self.executor
            .get_task(task_id)
            .map(DaemonTaskSnapshot::from_domain)
            .map_err(DaemonInvocationError::from)
    }

    fn wait(
        &self,
        task_id: crate::domain::invocation::TaskId,
        wait_ms: u64,
    ) -> Result<DaemonTaskSnapshot, DaemonInvocationError> {
        self.executor
            .wait_task(task_id, Duration::from_millis(wait_ms))
            .map(DaemonTaskSnapshot::from_domain)
            .map_err(DaemonInvocationError::from)
    }

    fn cancel(
        &self,
        task_id: crate::domain::invocation::TaskId,
    ) -> Result<DaemonTaskSnapshot, DaemonInvocationError> {
        self.executor
            .cancel_task(task_id)
            .map(DaemonTaskSnapshot::from_domain)
            .map_err(DaemonInvocationError::from)
    }

    fn has_active_invocations(&self) -> bool {
        self.executor.has_active_invocations()
    }

    fn restart_requested(&self) -> bool {
        self.executor.restart_requested()
    }
}

#[derive(Clone)]
pub(crate) struct DaemonServerConfig {
    pub(crate) state_root: std::path::PathBuf,
    pub(crate) core_identity: CoreIdentity,
    pub(crate) idle_grace: Duration,
    invocation_service: Arc<dyn CanonicalInvocationService>,
    #[cfg(test)]
    invocation_store: Option<Arc<dyn InvocationStore>>,
    #[cfg(test)]
    reconciliation_budget: Option<Duration>,
    #[cfg(test)]
    invocation_clock: Option<Arc<dyn Clock>>,
    #[cfg(test)]
    handshake_pause: Option<Arc<HandshakePause>>,
}

impl DaemonServerConfig {
    pub(crate) fn new(
        state_root: std::path::PathBuf,
        core_identity: CoreIdentity,
        idle_grace: Duration,
    ) -> Self {
        Self {
            state_root,
            core_identity,
            idle_grace,
            invocation_service: Arc::new(DormantCanonicalV13Service),
            #[cfg(test)]
            invocation_store: None,
            #[cfg(test)]
            reconciliation_budget: None,
            #[cfg(test)]
            invocation_clock: None,
            #[cfg(test)]
            handshake_pause: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_invocation_service(
        mut self,
        service: Arc<dyn CanonicalInvocationService>,
    ) -> Self {
        self.invocation_service = service;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_invocation_store_for_test(
        mut self,
        store: Arc<dyn InvocationStore>,
    ) -> Self {
        self.invocation_store = Some(store);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_reconciliation_budget_for_test(mut self, budget: Duration) -> Self {
        self.reconciliation_budget = Some(budget);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_invocation_clock_for_test(mut self, clock: Arc<dyn Clock>) -> Self {
        self.invocation_clock = Some(clock);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_handshake_pause(mut self, pause: &HandshakePauseGuard) -> Self {
        self.handshake_pause = Some(Arc::clone(&pause.pause));
        self
    }
}

pub(crate) fn run_daemon(config: DaemonServerConfig) -> Result<(), String> {
    if config.idle_grace.is_zero() {
        return Err("daemon idle grace must be positive".to_string());
    }
    let state = DaemonStateDirectory::open(&config.state_root, &config.core_identity)?;
    if let Some(existing) = state.read_endpoint_record()? {
        if existing.core_identity() != &config.core_identity {
            return Err("daemon endpoint record belongs to a foreign core identity".to_string());
        }
    }

    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .map_err(|error| daemon_io_error("bind daemon loopback endpoint", error))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| daemon_io_error("configure daemon listener", error))?;
    let port = listener
        .local_addr()
        .map_err(|error| daemon_io_error("inspect daemon listener", error))?
        .port();

    #[cfg(test)]
    let invocation_store = if let Some(store) = config.invocation_store.clone() {
        store
    } else {
        let task_store_directory = state.create_private_subdirectory("tasks")?;
        let opened_store = open_daemon_invocation_store_from_directory(task_store_directory)?;
        // Recovery belongs to this daemon before routing starts. Keeping the report beside the
        // sole-writer store prevents the stdio frontend from consuming it early.
        let _recovery_classifications = opened_store.recovery.classifications.len();
        opened_store.store
    };
    #[cfg(not(test))]
    let invocation_store = {
        let task_store_directory = state.create_private_subdirectory("tasks")?;
        let opened_store = open_daemon_invocation_store_from_directory(task_store_directory)?;
        let _recovery_classifications = opened_store.recovery.classifications.len();
        opened_store.store
    };
    #[cfg(test)]
    let invocation_clock = config
        .invocation_clock
        .clone()
        .unwrap_or_else(|| Arc::new(TokioClock));
    #[cfg(test)]
    let invocation_runtime = Arc::new(match config.reconciliation_budget {
        Some(budget) => DaemonInvocationRuntime::new_with_reconciliation_budget_for_test(
            invocation_store,
            Arc::clone(&config.invocation_service),
            Arc::clone(&invocation_clock),
            budget,
        ),
        None => DaemonInvocationRuntime::new(
            invocation_store,
            Arc::clone(&config.invocation_service),
            invocation_clock,
        ),
    });
    #[cfg(not(test))]
    let invocation_runtime = Arc::new(DaemonInvocationRuntime::new(
        invocation_store,
        Arc::clone(&config.invocation_service),
        Arc::new(TokioClock),
    ));

    let record = EndpointRecord::new(config.core_identity.clone(), port);
    let published = state.publish_endpoint_record(&record)?;
    let active_leases = Arc::new(LeaseRegistry::default());
    let admitted_connections = Arc::new(AtomicUsize::new(0));
    let shutting_down = Arc::new(AtomicBool::new(false));
    let mut handlers = Vec::new();
    let mut idle_since = Instant::now();
    let mut restart_requested = false;

    loop {
        match listener.accept() {
            Ok((stream, address)) if address.ip().is_loopback() => {
                match ConnectionSlot::acquire(Arc::clone(&admitted_connections)) {
                    Some(slot) => handlers.push(spawn_connection_handler(
                        stream,
                        record.clone(),
                        Arc::clone(&active_leases),
                        Arc::clone(&shutting_down),
                        Arc::clone(&invocation_runtime),
                        slot,
                        #[cfg(test)]
                        config.handshake_pause.clone(),
                    )),
                    None => reject_overloaded_connection(stream),
                }
            }
            Ok((_stream, _)) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => {
                shutting_down.store(true, Ordering::Release);
                join_handlers(handlers);
                let _ = state.remove_endpoint_if_owned(&published);
                return Err(daemon_io_error("accept daemon connection", error));
            }
        }

        handlers = reap_finished_handlers(handlers);
        if invocation_runtime.restart_requested() {
            restart_requested = true;
            break;
        }
        if active_leases.is_empty()?
            && admitted_connections.load(Ordering::Acquire) == 0
            && !invocation_runtime.has_active_invocations()
        {
            if idle_since.elapsed() >= config.idle_grace {
                break;
            }
        } else {
            idle_since = Instant::now();
        }
        thread::sleep(ACCEPT_POLL_INTERVAL);
    }

    shutting_down.store(true, Ordering::Release);
    drop(listener);
    if restart_requested {
        // Process death, not an in-process map or thread join, is the authority
        // that releases a stalled store syscall or non-cooperative execution.
        // Keep the PID-bound endpoint for stale-owner cleanup by the successor.
        return Ok(());
    }
    join_handlers(handlers);
    state.remove_endpoint_if_owned(&published)?;
    Ok(())
}

fn spawn_connection_handler(
    stream: TcpStream,
    record: EndpointRecord,
    active_leases: Arc<LeaseRegistry>,
    shutting_down: Arc<AtomicBool>,
    invocation_runtime: Arc<DaemonInvocationRuntime>,
    slot: ConnectionSlot,
    #[cfg(test)] handshake_pause: Option<Arc<HandshakePause>>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        #[cfg(test)]
        pause_before_handshake_if_configured(handshake_pause);
        let _ = handle_connection(
            stream,
            &record,
            &active_leases,
            &shutting_down,
            &invocation_runtime,
            slot,
        );
    })
}

fn handle_connection(
    mut stream: TcpStream,
    record: &EndpointRecord,
    active_leases: &Arc<LeaseRegistry>,
    shutting_down: &AtomicBool,
    invocation_runtime: &DaemonInvocationRuntime,
    handshake_slot: ConnectionSlot,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(HANDSHAKE_READ_TIMEOUT))
        .map_err(|error| daemon_io_error("configure daemon handshake timeout", error))?;
    stream
        .set_write_timeout(Some(HANDSHAKE_READ_TIMEOUT))
        .map_err(|error| daemon_io_error("configure daemon response timeout", error))?;
    let reader_stream = stream
        .try_clone()
        .map_err(|error| daemon_io_error("clone daemon client stream", error))?;
    let mut reader = BufReader::new(reader_stream);
    let request = match read_bounded_request_line(&mut reader).and_then(|bytes| {
        parse_request(&bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }) {
        Ok(request) => request,
        Err(_) => {
            write_response(
                &mut stream,
                &ServerResponse::error(DaemonErrorCode::InvalidRequest),
            )?;
            return Ok(());
        }
    };

    let ClientRequest::Hello {
        protocol_version,
        token,
        core_identity,
        owner_lease,
    } = request
    else {
        write_response(
            &mut stream,
            &ServerResponse::error(DaemonErrorCode::HandshakeRequired),
        )?;
        return Ok(());
    };
    if protocol_version != DAEMON_PROTOCOL_VERSION {
        write_response(
            &mut stream,
            &ServerResponse::error(DaemonErrorCode::ProtocolMismatch),
        )?;
        return Ok(());
    }
    if core_identity != *record.core_identity() {
        write_response(
            &mut stream,
            &ServerResponse::error(DaemonErrorCode::CoreMismatch),
        )?;
        return Ok(());
    }
    if !tokens_equal(&token, record.token()) {
        write_response(
            &mut stream,
            &ServerResponse::error(DaemonErrorCode::Unauthorized),
        )?;
        return Ok(());
    }

    let _owner = match active_leases.acquire(owner_lease)? {
        LeaseAdmission::Acquired(owner) => owner,
        LeaseAdmission::Duplicate => {
            write_response(
                &mut stream,
                &ServerResponse::error(DaemonErrorCode::DuplicateLease),
            )?;
            return Ok(());
        }
        LeaseAdmission::Capacity => {
            write_response(
                &mut stream,
                &ServerResponse::error(DaemonErrorCode::OwnerCapacity),
            )?;
            return Ok(());
        }
    };
    // The owner lease becomes the lifecycle fence before the pre-authentication admission
    // permit is released, so idle shutdown observes at least one of them throughout handoff.
    drop(handshake_slot);
    write_response(&mut stream, &ServerResponse::ready(record))?;
    stream
        .set_write_timeout(Some(OWNER_RESPONSE_WRITE_TIMEOUT))
        .map_err(|error| daemon_io_error("configure daemon owner response timeout", error))?;
    stream
        .set_read_timeout(Some(CONNECTION_READ_TIMEOUT))
        .map_err(|error| daemon_io_error("configure daemon owner timeout", error))?;
    while !shutting_down.load(Ordering::Acquire) {
        let bytes = match read_bounded_request_line(&mut reader) {
            Ok(bytes) => bytes,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                continue
            }
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(_) => break,
        };
        // Capture before strict request/schema validation and before any actor
        // discovery or dynamic service preparation. The parsed frontend budget
        // can only narrow this already-running daemon-receipt deadline.
        let response_deadline = invocation_runtime.capture_response_deadline();
        let request = match parse_request(&bytes) {
            Ok(request) => request,
            Err(_) => {
                write_response(
                    &mut stream,
                    &ServerResponse::error(DaemonErrorCode::InvalidRequest),
                )?;
                break;
            }
        };
        match request {
            ClientRequest::Ping {} => write_response_before(
                &mut stream,
                &ServerResponse::Pong,
                session_response_deadline(Duration::ZERO),
            )?,
            ClientRequest::Release {} => {
                write_response_before(
                    &mut stream,
                    &ServerResponse::Released,
                    session_response_deadline(Duration::ZERO),
                )?;
                break;
            }
            ClientRequest::Hello { .. } => {
                write_response(
                    &mut stream,
                    &ServerResponse::error(DaemonErrorCode::InvalidRequest),
                )?;
                break;
            }
            ClientRequest::SubmitInvocation { invocation } => {
                let response_deadline = response_deadline.restrict_to_frontend_budget(
                    Duration::from_millis(invocation.response_budget_ms()),
                );
                match invocation_runtime.submit(invocation, response_deadline.clone()) {
                    Ok(outcome) => write_invocation_response_before(
                        &mut stream,
                        &ServerResponse::invocation(outcome),
                        &response_deadline,
                    )?,
                    Err(error) => write_invocation_response_before(
                        &mut stream,
                        &ServerResponse::error(error.protocol_code()),
                        &response_deadline,
                    )?,
                }
            }
            ClientRequest::GetTask { task_id } => {
                let deadline = session_response_deadline(Duration::ZERO);
                write_task_response_before(&mut stream, invocation_runtime.get(task_id), deadline)?
            }
            ClientRequest::WaitTask { task_id, wait_ms } => {
                let deadline = session_response_deadline(Duration::from_millis(wait_ms));
                write_task_response_before(
                    &mut stream,
                    invocation_runtime.wait(task_id, wait_ms),
                    deadline,
                )?
            }
            ClientRequest::CancelTask { task_id } => {
                let deadline = session_response_deadline(Duration::ZERO);
                write_task_response_before(
                    &mut stream,
                    invocation_runtime.cancel(task_id),
                    deadline,
                )?
            }
        }
    }
    Ok(())
}

fn write_task_response_before(
    stream: &mut TcpStream,
    result: Result<DaemonTaskSnapshot, DaemonInvocationError>,
    deadline: Instant,
) -> Result<(), String> {
    match result {
        Ok(snapshot) => write_response_before(stream, &ServerResponse::task(snapshot), deadline),
        Err(error) => write_response_before(
            stream,
            &ServerResponse::error(error.protocol_code()),
            deadline,
        ),
    }
}

fn session_response_deadline(operation_budget: Duration) -> Instant {
    Instant::now() + operation_budget + Duration::from_millis(RESPONSE_SERIALIZATION_MARGIN_MS)
}

fn write_response(stream: &mut TcpStream, response: &ServerResponse) -> Result<(), String> {
    write_response_before(
        stream,
        response,
        Instant::now() + OWNER_RESPONSE_WRITE_TIMEOUT,
    )
}

fn write_response_before(
    stream: &mut TcpStream,
    response: &ServerResponse,
    session_deadline: Instant,
) -> Result<(), String> {
    write_response_before_with_now(stream, response, session_deadline, Instant::now)
}

fn write_invocation_response_before(
    stream: &mut TcpStream,
    response: &ServerResponse,
    response_deadline: &InvocationResponseDeadline,
) -> Result<(), String> {
    write_response_before_with_now(stream, response, response_deadline.response_at(), || {
        response_deadline.now()
    })
}

fn write_response_before_with_now<N>(
    stream: &mut TcpStream,
    response: &ServerResponse,
    session_deadline: Instant,
    mut now: N,
) -> Result<(), String>
where
    N: FnMut() -> Instant,
{
    let deadline = session_deadline.min(now() + OWNER_RESPONSE_WRITE_TIMEOUT);
    if now() >= deadline {
        return Err("write daemon response: session response deadline elapsed".to_string());
    }
    let mut bytes = match serialize_response_bounded(response) {
        Ok(bytes) => bytes,
        Err(ResponseSerializationError::TooLarge) => {
            serialize_response_bounded(&ServerResponse::error(DaemonErrorCode::ResultTooLarge))
                .map_err(|_| "bounded daemon error response could not be serialized".to_string())?
        }
        Err(ResponseSerializationError::Invalid) => {
            return Err("daemon response could not be serialized".to_string())
        }
    };
    if now() >= deadline {
        return Err("write daemon response: session response deadline elapsed".to_string());
    }
    bytes.push(b'\n');
    write_bytes_before(stream, &bytes, deadline, now, |stream, remaining| {
        stream.set_write_timeout(Some(remaining))
    })
}

pub(super) fn write_bytes_before<W, N, C>(
    writer: &mut W,
    bytes: &[u8],
    deadline: Instant,
    mut now: N,
    mut configure_timeout: C,
) -> Result<(), String>
where
    W: Write,
    N: FnMut() -> Instant,
    C: FnMut(&mut W, Duration) -> io::Result<()>,
{
    let mut written = 0;
    while written < bytes.len() {
        let remaining = deadline.saturating_duration_since(now());
        if remaining.is_zero() {
            return Err("write daemon response: bounded response deadline elapsed".to_string());
        }
        configure_timeout(writer, remaining)
            .map_err(|error| daemon_io_error("configure daemon response timeout", error))?;
        match writer.write(&bytes[written..]) {
            Ok(0) => {
                return Err("write daemon response: connection closed before response".to_string())
            }
            Ok(count) => written += count,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                std::thread::sleep(Duration::from_millis(1).min(remaining));
            }
            Err(error) => return Err(daemon_io_error("write daemon response", error)),
        }
    }
    writer
        .flush()
        .map_err(|error| daemon_io_error("flush daemon response", error))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponseSerializationError {
    TooLarge,
    Invalid,
}

struct BoundedResponseWriter {
    bytes: Vec<u8>,
    max_bytes: usize,
    too_large: bool,
}

impl Write for BoundedResponseWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let Some(next) = self.bytes.len().checked_add(buffer.len()) else {
            self.too_large = true;
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "response too large",
            ));
        };
        if next > self.max_bytes {
            self.too_large = true;
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "response too large",
            ));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn serialize_response_bounded(
    response: &ServerResponse,
) -> Result<Vec<u8>, ResponseSerializationError> {
    let mut writer = BoundedResponseWriter {
        bytes: Vec::new(),
        max_bytes: super::protocol::MAX_DAEMON_RESPONSE_LINE_BYTES.saturating_sub(1),
        too_large: false,
    };
    let serialized = serde_json::to_writer(&mut writer, response);
    if writer.too_large {
        return Err(ResponseSerializationError::TooLarge);
    }
    if serialized.is_err() {
        return Err(ResponseSerializationError::Invalid);
    }
    Ok(writer.bytes)
}

#[cfg(test)]
pub(crate) mod actor_capacity_tests {
    use super::*;
    use crate::application::invocation_store::{
        NewInvocationRecord, SafeStatusMessage, StoredInvocationRecord, TaskTransition,
        ToolIdentity,
    };
    use crate::application::operation_descriptors::KnownLongReason;
    use crate::application::shared_work::{
        ArtifactReady, DeliveryFormIdentity, DeliveryWorkKey, ProviderHostKey,
    };
    use crate::domain::invocation::InvocationStatus;
    use crate::infrastructure::runtime_jobs::{
        RuntimeJobOperation, RuntimeJobRequest, RuntimeJobService,
    };
    use crate::infrastructure::task_store::{FileInvocationStore, SystemEpochMillisClock};
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Barrier, Condvar};

    thread_local! {
        static LOGICAL_READ_NOW: RefCell<Option<Instant>> = const { RefCell::new(None) };
    }

    fn logical_read_now() -> Instant {
        LOGICAL_READ_NOW.with(|now| now.borrow().expect("logical read clock is initialized"))
    }

    fn set_logical_read_now(now: Instant) {
        LOGICAL_READ_NOW.with(|current| *current.borrow_mut() = Some(now));
    }

    fn audit_actor_read_source_capability_api(source: &str) -> Result<(), String> {
        let lines = source.lines().collect::<Vec<_>>();
        let declarations = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.trim() == "pub(super) struct ActorReadSourceCapability {")
            .collect::<Vec<_>>();
        if declarations.len() != 1 {
            return Err(format!(
                "actor read capability must have exactly one sibling-visible declaration, found {}",
                declarations.len()
            ));
        }
        let declaration_line = declarations[0].0;
        let mut attribute_line = declaration_line;
        while attribute_line > 0 && lines[attribute_line - 1].trim().starts_with("#[") {
            attribute_line -= 1;
        }
        if lines[attribute_line..declaration_line]
            .iter()
            .any(|line| line.contains("Clone"))
        {
            return Err("actor read capability must not implement Clone".to_string());
        }

        let (_, after_declaration) = source
            .split_once("pub(super) struct ActorReadSourceCapability {")
            .expect("the unique declaration was found above");
        let (body, _) = after_declaration.split_once("\n}").ok_or_else(|| {
            "actor read capability declaration remains structurally bounded".to_string()
        })?;
        let mut fields = body
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        let mut expected_fields = vec![
            "binding: ProviderRootBinding,",
            "identity: String,",
            "fence: WorkspaceLogicalReadFence,",
            "deadline: ProviderDeadline,",
        ];
        fields.sort_unstable();
        expected_fields.sort_unstable();
        if fields != expected_fields {
            return Err(format!(
                "actor read capability fields must remain the exact private sealed shape; found {fields:?}"
            ));
        }

        let impl_declarations = lines
            .iter()
            .filter(|line| line.trim() == "impl ActorReadSourceCapability {")
            .count();
        let other_impls = lines
            .iter()
            .filter(|line| {
                let line = line.trim();
                line.starts_with("impl ")
                    && line.contains("ActorReadSourceCapability")
                    && line != "impl ActorReadSourceCapability {"
            })
            .map(|line| line.trim())
            .collect::<Vec<_>>();
        if impl_declarations != 1 || !other_impls.is_empty() {
            return Err(format!(
                "actor read capability must have exactly one inherent impl and no trait/extra impls; inherent={impl_declarations}, extra={other_impls:?}"
            ));
        }

        let (_, after_impl) = source
            .split_once("impl ActorReadSourceCapability {\n")
            .expect("the unique inherent impl was found above");
        let (impl_body, _) = after_impl
            .split_once("\n}\n\nstruct ActorLogicalReadSourceLease")
            .ok_or_else(|| {
                "actor read capability implementation remains structurally bounded".to_string()
            })?;
        let mut sibling_items = Vec::new();
        let impl_lines = impl_body.lines().collect::<Vec<_>>();
        let mut line_index = 0;
        while line_index < impl_lines.len() {
            let line = impl_lines[line_index].trim();
            if line.starts_with("pub(") || line.starts_with("pub ") {
                let mut declaration = line.to_string();
                while !declaration.contains('{') && !declaration.ends_with(';') {
                    line_index += 1;
                    let continuation = impl_lines.get(line_index).ok_or_else(|| {
                        "sibling-visible capability item has no body or terminator".to_string()
                    })?;
                    declaration.push(' ');
                    declaration.push_str(continuation.trim());
                }
                sibling_items.push(declaration.split_whitespace().collect::<Vec<_>>().join(" "));
            }
            line_index += 1;
        }

        let mut expected_sibling_items = vec![
            "pub(super) fn source_set_name(&self) -> &str {".to_string(),
            "pub(super) const fn deadline(&self) -> ProviderDeadline {".to_string(),
            "pub(super) fn logical_view_read_authority<'a>( &self, cancellation: &'a CancellationToken, ) -> Result<crate::infrastructure::v13_read::LogicalViewReadAuthority<'a>, String> {".to_string(),
        ];
        sibling_items.sort_unstable();
        expected_sibling_items.sort_unstable();
        if sibling_items != expected_sibling_items {
            return Err(format!(
                "actor read capability sibling API must be the exact read-only allowlist; found {sibling_items:?}"
            ));
        }
        Ok(())
    }

    #[test]
    pub(crate) fn actor_read_source_capability_is_sealed_after_binding() {
        let source = include_str!("server.rs");
        audit_actor_read_source_capability_api(source).unwrap_or_else(|error| panic!("{error}"));
    }

    #[test]
    fn actor_read_source_capability_audit_rejects_sibling_visible_forge_and_mutator() {
        let source = include_str!("server.rs");
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
                "#[allow(dead_code)]\npub(super) struct ActorReadSourceCapability {",
                "#[allow(dead_code)]\n#[derive(Clone)]\npub(super) struct ActorReadSourceCapability {",
                1,
            ),
            source.replacen(
                "pub(super) const fn deadline(&self) -> ProviderDeadline {",
                "pub(super) const fn deadline(&mut self) -> ProviderDeadline {",
                1,
            ),
        ];
        for hostile in hostile_sources {
            assert!(
                audit_actor_read_source_capability_api(&hostile).is_err(),
                "hostile sibling-visible capability shape escaped the closed API audit"
            );
        }
    }

    #[test]
    pub(crate) fn actor_authenticated_source_architecture_names_complete_witnesses() {
        fn front_matter_value<'a>(document: &'a str, key: &str) -> &'a str {
            document
                .lines()
                .find_map(|line| line.strip_prefix(key))
                .map(str::trim)
                .unwrap_or_else(|| panic!("architecture record has no `{key}` field"))
        }

        let capability = include_str!(
            "../../../../../arch/invariants/INV.APP.ACTOR-AUTHENTICATED-SOURCE-CAPABILITIES.md"
        );
        assert_eq!(
            front_matter_value(capability, "check:"),
            "crates/unica-coder/src/infrastructure/daemon/server.rs::actor_authenticated_source_capability_contract_is_complete",
            "capability invariant points at a witness that omits daemon no-substitution"
        );

        let decision = include_str!(
            "../../../../../arch/decisions/2026-08-26-actor-authenticated-source-profile-slice.md"
        );
        assert_eq!(
            front_matter_value(decision, "realized:"),
            "crates/unica-coder/src/infrastructure/daemon/server.rs::actor_authenticated_source_profile_contract_is_complete"
        );

        let source = include_str!("server.rs");
        let aggregate_declaration = [
            "pub(crate) fn actor_authenticated_source_profile_contract_is_complete",
            "()",
        ]
        .concat();
        let (_, after_aggregate) = source
            .split_once(&aggregate_declaration)
            .expect("decision aggregate remains available to the architecture witness");
        let (aggregate, _) = after_aggregate
            .split_once("\n    }\n")
            .expect("decision aggregate remains structurally bounded");
        assert!(
            aggregate.contains("actor_authenticated_source_capability_contract_is_complete();"),
            "decision aggregate omits the complete actor capability witness"
        );
        assert!(
            aggregate.contains(
                "remapped_names_and_profiles_do_not_share_revision_index_or_coordination_state();"
            ),
            "decision aggregate omits actor state-scope separation"
        );
        assert!(
            aggregate.contains("duplicate_source_set_names_with_distinct_roots_are_rejected();"),
            "decision aggregate omits duplicate source-set name rejection"
        );
    }

    #[test]
    pub(crate) fn provider_binding_and_actor_bound_invocation_cannot_substitute_kind_or_profile() {
        let task_root = tempfile::tempdir().unwrap();
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
        let (store, _) =
            FileInvocationStore::open(task_root.path(), Arc::new(SystemEpochMillisClock)).unwrap();
        let runtime = DaemonInvocationRuntime::new(
            Arc::new(store),
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
        let invocation = bind_workspace_invocation_controlled(
            &request,
            &runtime.workspace_actors,
            ActorInvocationResources {
                deliveries: Arc::clone(&runtime.deliveries),
                provider_hosts: Arc::clone(&runtime.provider_hosts),
                runtime_resources: Arc::clone(&runtime.runtime_resources),
                runtime_service: None,
            },
            runtime.capture_response_deadline(),
            |sources| {
                sources[0].kind = SourceSetKind::Extension;
                sources[0].source_format = SourceFormat::Edt;
                sources[0].source_profile = SourceProfile::TestPlatform8_3_28Format2_20;
            },
        )
        .unwrap();
        let binding = invocation.read_sources[0].binding.clone();
        let execution = invocation
            .begin_execution(&CancellationToken::new())
            .unwrap();
        let capability = execution.read_sources().unwrap().remove(0);

        assert_eq!(capability.source_set_name(), binding.source_set_name());
        assert_eq!(capability.source_kind(), binding.source_kind());
        assert_eq!(capability.source_format(), binding.source_format());
        assert_eq!(capability.source_profile(), binding.source_profile());
        assert_eq!(
            capability.source_profile().platform_profile(),
            binding.source_profile().platform_profile(),
            "reader profile must be derived from the actor-issued source profile"
        );
    }

    #[test]
    pub(crate) fn subsequent_daemon_invocation_after_same_root_kind_change_gets_new_actor_identity()
    {
        let task_root = tempfile::tempdir().unwrap();
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
        let (store, _) =
            FileInvocationStore::open(task_root.path(), Arc::new(SystemEpochMillisClock)).unwrap();
        let runtime = DaemonInvocationRuntime::new(
            Arc::new(store),
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
                runtime.capture_response_deadline(),
            )
            .unwrap()
        };
        let configuration = bind();
        write_project("EXTENSION");
        let extension = bind();

        assert!(
            !Arc::ptr_eq(&configuration.actor, &extension.actor),
            "subsequent semantic kind change reused the live actor"
        );
        assert_ne!(
            configuration.workspace_identity_hash, extension.workspace_identity_hash,
            "durable daemon workspace identity ignored the changed source kind"
        );
    }

    #[test]
    pub(crate) fn v13_daemon_rejects_unproved_edt_invalid_or_empty_platform_fallback() {
        let task_root = tempfile::tempdir().unwrap();
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

        let (store, _) =
            FileInvocationStore::open(task_root.path(), Arc::new(SystemEpochMillisClock)).unwrap();
        let runtime = DaemonInvocationRuntime::new(
            Arc::new(store),
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
                runtime.capture_response_deadline(),
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
    pub(crate) fn actor_authenticated_source_profile_contract_is_complete() {
        crate::infrastructure::workspace_actor::tests::same_name_root_changed_kind_rotates_actor_and_state_scope();
        crate::infrastructure::workspace_actor::tests::same_name_root_changed_format_or_platform_profile_rotates_actor();
        crate::infrastructure::workspace_actor::tests::workspace_actor_registry_keys_exact_identity_and_separates_worktrees_and_source_roots();
        crate::infrastructure::workspace_actor::tests::duplicate_physical_root_names_are_rejected_as_ambiguous();
        crate::infrastructure::workspace_actor::tests::duplicate_source_set_names_with_distinct_roots_are_rejected();
        actor_authenticated_source_capability_contract_is_complete();
        crate::infrastructure::workspace_actor::tests::remapped_names_and_profiles_do_not_share_revision_index_or_coordination_state();
        subsequent_daemon_invocation_after_same_root_kind_change_gets_new_actor_identity();
        v13_daemon_rejects_unproved_edt_invalid_or_empty_platform_fallback();
        hidden_v13_logical_lease_survives_the_handoff_window_and_confirms_once();
    }

    #[test]
    pub(crate) fn actor_authenticated_source_capability_contract_is_complete() {
        actor_read_source_capability_is_sealed_after_binding();
        provider_binding_and_actor_bound_invocation_cannot_substitute_kind_or_profile();
        crate::infrastructure::workspace_actor::tests::capabilities_do_not_cross_distinct_actor_instances_with_equal_identity();
        crate::infrastructure::workspace_actor::tests::workspace_actor_capabilities_enforce_identity_physical_and_bounded_publication();
    }

    struct ManualInvocationClock(Mutex<Instant>);

    impl ManualInvocationClock {
        fn new(now: Instant) -> Self {
            Self(Mutex::new(now))
        }

        fn advance(&self, duration: Duration) {
            *self.0.lock().unwrap() += duration;
        }
    }

    impl Clock for ManualInvocationClock {
        fn now(&self) -> Instant {
            *self.0.lock().unwrap()
        }
    }

    #[test]
    pub(crate) fn hidden_v13_logical_lease_survives_the_handoff_window_and_confirms_once() {
        let task_root = tempfile::tempdir().unwrap();
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
        let (store, _) =
            FileInvocationStore::open(task_root.path(), Arc::new(SystemEpochMillisClock)).unwrap();
        let runtime = DaemonInvocationRuntime::new(
            Arc::new(store),
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
            runtime.capture_response_deadline(),
        )
        .unwrap();
        let sibling_binding = invocation
            .read_sources
            .iter()
            .find(|source| source.binding.source_set_name() == "dep")
            .unwrap()
            .binding
            .clone();
        let sibling_revisions = invocation
            .actor
            .source_revision_service(&sibling_binding)
            .unwrap();
        let started = Instant::now();
        set_logical_read_now(started);
        let deadline =
            ProviderDeadline::with_clock(started + LOGICAL_READ_OPERATION_BUDGET, logical_read_now);
        let cancellation = CancellationToken::new();
        let execution = invocation
            .begin_execution_with_logical_deadline(&cancellation, deadline)
            .unwrap();
        assert_eq!(
            sibling_revisions.retained_scan_count(),
            0,
            "qualified view must not admit or scan an unrelated source set"
        );
        std::fs::write(
            sibling.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>ChangedDependency</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
        )
        .unwrap();

        set_logical_read_now(started + Duration::from_secs(8));
        let sources = execution.read_sources().unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(
            sources[0].deadline().remaining(),
            LOGICAL_READ_OPERATION_BUDGET - Duration::from_secs(8),
            "the handoff window must not replenish or cap the logical-read deadline"
        );
        let service =
            crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default();
        let result = service
            .execute(&execution, cancellation.clone())
            .expect("hidden V13 execution");
        assert!(result.ok, "{} {:?}", result.summary, result.diagnostics);
        let actor = Arc::clone(&execution.invocation.actor);
        let legacy_fence = actor
            .capture_revision(&execution.invocation.provider_root, deadline, &cancellation)
            .expect("capture a fence for the actor mutation lane");
        let held_publication = actor
            .begin_publication(&legacy_fence, deadline, &cancellation)
            .expect("hold the actor mutation lane");
        let (published_tx, published_rx) = mpsc::channel();
        let publish_cancellation = cancellation.clone();
        std::thread::spawn(move || {
            set_logical_read_now(started + Duration::from_secs(8));
            published_tx
                .send(execution.publish(Ok(result), &publish_cancellation))
                .unwrap();
        });
        assert!(
            matches!(
                published_rx.recv_timeout(Duration::from_millis(100)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "logical read publication must wait on the actor mutation lane"
        );
        drop(held_publication);
        let published = published_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("logical publication resumes after mutation lane release")
            .unwrap()
            .unwrap();
        assert!(published.ok);
        assert_eq!(
            sibling_revisions.retained_scan_count(),
            0,
            "an unrelated source mutation must not invalidate the qualified view"
        );

        let find_request = InvocationRequest::new(
            ToolIdentity::Find,
            serde_json::json!({"query": "Items"}),
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
            runtime.capture_response_deadline(),
        )
        .unwrap();
        let find_execution = find_invocation
            .begin_execution_with_logical_deadline(&cancellation, deadline)
            .unwrap();
        assert_eq!(
            find_execution.read_sources().unwrap().len(),
            2,
            "aggregate find must admit every workspace source set"
        );
        let find_result = service
            .execute(&find_execution, cancellation.clone())
            .expect("hidden V13 find execution");
        assert!(find_result.ok, "{:?}", find_result.diagnostics);
        let find_actor = Arc::clone(&find_execution.invocation.actor);
        let legacy_fence = find_actor
            .capture_revision(
                &find_execution.invocation.provider_root,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let (first_confirmed_tx, first_confirmed_rx) = mpsc::channel();
        let (release_confirm_tx, release_confirm_rx) = mpsc::channel();
        let (logical_done_tx, logical_done_rx) = mpsc::channel();
        let logical_cancellation = cancellation.clone();
        std::thread::spawn(move || {
            set_logical_read_now(started + Duration::from_secs(8));
            crate::infrastructure::workspace_actor::set_logical_publication_after_confirmation_hook(
                move || {
                    first_confirmed_tx.send(()).unwrap();
                    release_confirm_rx.recv().unwrap();
                },
            );
            logical_done_tx
                .send(find_execution.publish(Ok(find_result), &logical_cancellation))
                .unwrap();
        });
        first_confirmed_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("first aggregate source was confirmed under the lane");
        let (competitor_tx, competitor_rx) = mpsc::channel();
        let competing_actor = Arc::clone(&find_actor);
        let competing_cancellation = cancellation.clone();
        std::thread::spawn(move || {
            let acquired = competing_actor.begin_publication(
                &legacy_fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &competing_cancellation,
            );
            competitor_tx.send(acquired.map(drop)).unwrap();
        });
        assert!(
            matches!(
                competitor_rx.recv_timeout(Duration::from_millis(100)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "the mutation lane must remain held between aggregate source confirmations"
        );
        release_confirm_tx.send(()).unwrap();
        assert!(
            logical_done_rx
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .unwrap()
                .unwrap()
                .ok
        );
        competitor_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("competing publication resumes after every source confirmation")
            .unwrap();

        let find_invocation = bind_workspace_invocation(
            &find_request,
            &runtime.workspace_actors,
            Arc::clone(&runtime.deliveries),
            Arc::clone(&runtime.provider_hosts),
            Arc::clone(&runtime.runtime_resources),
            None,
            runtime.capture_response_deadline(),
        )
        .unwrap();
        let find_execution = find_invocation
            .begin_execution_with_logical_deadline(&cancellation, deadline)
            .unwrap();
        let find_result = service
            .execute(&find_execution, cancellation.clone())
            .expect("second hidden V13 find execution");
        std::fs::write(
            sibling.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>ChangedAgain</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        assert!(
            find_execution
                .publish(Ok(find_result), &cancellation)
                .is_err(),
            "find must confirm every admitted source set"
        );

        let invocation = bind_workspace_invocation(
            &request,
            &runtime.workspace_actors,
            Arc::clone(&runtime.deliveries),
            Arc::clone(&runtime.provider_hosts),
            Arc::clone(&runtime.runtime_resources),
            None,
            runtime.capture_response_deadline(),
        )
        .unwrap();
        set_logical_read_now(started + Duration::from_secs(9));
        let execution = invocation
            .begin_execution_with_logical_deadline(&cancellation, deadline)
            .unwrap();
        let result = service
            .execute(&execution, cancellation.clone())
            .expect("second hidden V13 execution");
        std::fs::write(
            source.join("Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>ItemsChanged</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        assert!(
            execution.publish(Ok(result), &cancellation).is_err(),
            "a selected-source mutation must fail final retained confirmation"
        );
    }

    fn rejected_hidden_v13_view_execution(
        at: &str,
    ) -> (
        tempfile::TempDir,
        ActorBoundExecution,
        Arc<crate::infrastructure::source_revision::SourceRevisionService>,
        usize,
    ) {
        let task_root = tempfile::tempdir().unwrap();
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
        let (store, _) =
            FileInvocationStore::open(task_root.path(), Arc::new(SystemEpochMillisClock)).unwrap();
        let runtime = DaemonInvocationRuntime::new(
            Arc::new(store),
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
            runtime.capture_response_deadline(),
        )
        .unwrap();
        let revisions = invocation
            .actor
            .source_revision_service(&invocation.read_sources[0].binding)
            .unwrap();
        let scans_before = revisions.retained_scan_count();
        let cancellation = CancellationToken::new();

        let execution = invocation
            .begin_execution(&cancellation)
            .expect("ordinary rejected address must reach the typed view handler");
        assert_eq!(
            revisions.retained_scan_count(),
            scans_before,
            "an address rejected before source selection must not scan a source corpus"
        );
        (workspace, execution, revisions, scans_before)
    }

    fn execute_rejected_hidden_v13_view_address(at: &str) -> DomainResult {
        let (_workspace, execution, revisions, scans_before) =
            rejected_hidden_v13_view_execution(at);
        let cancellation = CancellationToken::new();
        let result = crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()
            .execute(&execution, cancellation.clone())
            .expect("typed input error is a DomainResult");
        let result = execution
            .publish(Ok(result), &cancellation)
            .unwrap()
            .unwrap();
        assert_eq!(
            revisions.retained_scan_count(),
            scans_before,
            "zero-fence typed rejection publication must not scan a source corpus"
        );
        result
    }

    fn zero_fence_accepts_forged_result(at: &str, mutate: impl FnOnce(&mut DomainResult)) -> bool {
        let (_workspace, execution, revisions, scans_before) =
            rejected_hidden_v13_view_execution(at);
        let cancellation = CancellationToken::new();
        let mut result =
            crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()
                .execute(&execution, cancellation.clone())
                .expect("typed input error is a DomainResult");
        mutate(&mut result);
        let accepted = execution.publish(Ok(result), &cancellation).is_ok();
        assert_eq!(
            revisions.retained_scan_count(),
            scans_before,
            "forged rejected-path result must not scan a source corpus"
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

            let (_workspace, execution, revisions, scans_before) =
                rejected_hidden_v13_view_execution(at);
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
            assert_eq!(revisions.retained_scan_count(), scans_before);

            let (_workspace, execution, revisions, scans_before) =
                rejected_hidden_v13_view_execution(at);
            if execution
                .publish(Ok(DomainResult::success("forged success")), &cancellation)
                .is_ok()
            {
                accepted.push(format!("{at}:success"));
            }
            assert_eq!(revisions.retained_scan_count(), scans_before);
        }
        assert!(
            accepted.is_empty(),
            "zero-fence publication accepted forbidden envelopes: {accepted:?}"
        );
    }

    #[test]
    pub(crate) fn hidden_v13_logical_publication_contract_is_complete() {
        crate::infrastructure::source_revision::tests::retained_final_confirmation_stabilization_contract_is_complete();
        hidden_v13_logical_lease_survives_the_handoff_window_and_confirms_once();
        review_invalid_logical_address_reaches_typed_bad_value_result();
        valid_unknown_source_reaches_typed_provider_unavailable_without_scanning();
        zero_fence_view_rejection_accepts_only_the_exact_canonical_envelope();
        crate::infrastructure::workspace_actor::tests::logical_read_publication_lane_wait_honors_existing_cancellation_and_deadline();
    }

    struct BlockingService {
        entered: mpsc::Sender<()>,
    }

    struct RejectingAfterActorAdmissionService;

    struct NonCooperativeActorService {
        entered: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    struct DelayedPrepareService {
        clock: Arc<ManualInvocationClock>,
        delay: Duration,
        executions: Arc<AtomicUsize>,
    }

    struct SharedDeliveryService {
        key: DeliveryWorkKey,
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

    struct RevisionChangingIndexService {
        producers: Arc<AtomicUsize>,
        producer_entered: mpsc::Sender<()>,
        joined: mpsc::Sender<IndexWorkIdentity>,
        release: Arc<(Mutex<bool>, Condvar)>,
        first_execution: AtomicBool,
        mark_dirty: Mutex<mpsc::Receiver<()>>,
        dirty_done: mpsc::Sender<()>,
    }

    struct UncertainCancelStore {
        inner: FileInvocationStore,
    }

    impl InvocationStore for UncertainCancelStore {
        fn create(
            &self,
            record: NewInvocationRecord,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            self.inner.create(record)
        }

        fn create_working(
            &self,
            record: NewInvocationRecord,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            self.inner.create_working(record)
        }

        fn get(
            &self,
            task_id: crate::domain::invocation::TaskId,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            self.inner.get(task_id)
        }

        fn update(
            &self,
            task_id: crate::domain::invocation::TaskId,
            transition: TaskTransition,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            self.inner.update(task_id, transition)
        }

        fn cancel(
            &self,
            task_id: crate::domain::invocation::TaskId,
            _status_message: SafeStatusMessage,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            Err(InvocationStoreError::CommitUncertain {
                task_id,
                operation: crate::application::invocation_store::CommitOperation::Cancel,
            })
        }
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

    impl CanonicalInvocationService for DelayedPrepareService {
        fn prepare(
            &self,
            _invocation: &ActorBoundInvocation,
        ) -> Result<ExecutionClass, Box<DomainResult>> {
            self.clock.advance(self.delay);
            Ok(ExecutionClass::InlineCandidate)
        }

        fn execute(
            &self,
            _invocation: &ActorBoundExecution,
            _cancellation: CancellationToken,
        ) -> Result<DomainResult, InvocationFailure> {
            self.executions.fetch_add(1, Ordering::SeqCst);
            Ok(DomainResult::success("deadline-bound prepare result"))
        }
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
            let lease = desk.join(self.key.clone(), move |_| {
                producers.fetch_add(1, Ordering::SeqCst);
                producer_entered.send(()).expect("producer observation");
                let (released, wake) = &*release;
                let mut released = released.lock().expect("delivery release");
                while !*released {
                    released = wake.wait(released).expect("delivery release wait");
                }
                ArtifactReady::new(key, std::path::PathBuf::from("/cache/shared-daemon"))
            });
            self.joined
                .send(desk as *const crate::infrastructure::engine_delivery::DeliveryDesk as usize)
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
            let make_work = || {
                let producers = Arc::clone(&self.producers);
                let producer_entered = self.producer_entered.clone();
                let release = Arc::clone(&self.release);
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
            };
            let (key, lease) = match &self.kind {
                LongCapabilityKind::Index => invocation
                    .join_index_work("rlm", "bsl-1", make_work())
                    .map(|(key, lease)| (JoinedCapabilityIdentity::Index(key), lease))
                    .map_err(|_| InvocationFailure::new("index_failed", "index unavailable"))?,
                LongCapabilityKind::Provider(key) => (
                    JoinedCapabilityIdentity::Provider(key.clone()),
                    invocation.join_provider_host(key.clone(), make_work()),
                ),
                LongCapabilityKind::Runtime { lease_id } => (
                    JoinedCapabilityIdentity::Runtime(lease_id.clone()),
                    invocation
                        .join_runtime_resource(lease_id, make_work())
                        .map_err(|_| {
                            InvocationFailure::new("runtime_failed", "runtime unavailable")
                        })?,
                ),
            };
            self.joined.send(key).expect("join observation");
            lease
                .wait()
                .map_err(|_| InvocationFailure::new("long_work_failed", "work unavailable"))?;
            let marker = invocation
                .read_relative_file(std::path::Path::new("marker.txt"), 64)
                .map_err(|_| InvocationFailure::new("root_failed", "root unavailable"))?;
            Ok(DomainResult::success(
                String::from_utf8(marker).expect("marker is utf8"),
            ))
        }
    }

    impl CanonicalInvocationService for RevisionChangingIndexService {
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
                .join_index_work("rlm", "bsl-1", move |_| {
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
                })
                .map_err(|_| InvocationFailure::new("index_failed", "index unavailable"))?;
            self.joined.send(key).expect("index join observation");
            if !self.first_execution.swap(true, Ordering::SeqCst) {
                self.mark_dirty
                    .lock()
                    .expect("dirty signal")
                    .recv()
                    .expect("dirty request");
                invocation.mark_source_revision_dirty_for_test();
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

    #[test]
    pub(crate) fn daemon_receipt_deadline_is_not_replenished_after_delayed_prepare() {
        for delay in [Duration::from_millis(110), Duration::from_millis(226)] {
            let task_root = tempfile::tempdir().unwrap();
            let workspace = tempfile::tempdir().unwrap();
            let (store, _) =
                FileInvocationStore::open(task_root.path(), Arc::new(SystemEpochMillisClock))
                    .unwrap();
            let clock = Arc::new(ManualInvocationClock::new(Instant::now()));
            let executions = Arc::new(AtomicUsize::new(0));
            let runtime = DaemonInvocationRuntime::new(
                Arc::new(store),
                Arc::new(DelayedPrepareService {
                    clock: Arc::clone(&clock),
                    delay,
                    executions: Arc::clone(&executions),
                }),
                clock,
            );
            let response = submit_at_receipt(
                &runtime,
                InvocationRequest::new(
                    ToolIdentity::Run,
                    serde_json::json!({}),
                    std::fs::canonicalize(workspace.path())
                        .unwrap()
                        .to_string_lossy(),
                    100,
                )
                .unwrap(),
            )
            .unwrap();
            let task_id = task_id(response);
            let terminal = runtime.wait(task_id, 7_000).unwrap();
            assert_eq!(terminal.status, InvocationStatus::Completed);
            assert_eq!(
                terminal.result.unwrap().summary,
                "deadline-bound prepare result"
            );
            assert_eq!(executions.load(Ordering::SeqCst), 1);
        }

        let task_root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let (store, _) =
            FileInvocationStore::open(task_root.path(), Arc::new(SystemEpochMillisClock)).unwrap();
        let started = Instant::now();
        let clock = Arc::new(ManualInvocationClock::new(started));
        let executions = Arc::new(AtomicUsize::new(0));
        let runtime = DaemonInvocationRuntime::new(
            Arc::new(store),
            Arc::new(DelayedPrepareService {
                clock: Arc::clone(&clock),
                delay: Duration::from_millis(226),
                executions: Arc::clone(&executions),
            }),
            clock.clone(),
        );
        let invalid = submit_at_receipt(
            &runtime,
            InvocationRequest::new(
                ToolIdentity::Run,
                serde_json::json!({"unknown": true}),
                std::fs::canonicalize(workspace.path())
                    .unwrap()
                    .to_string_lossy(),
                100,
            )
            .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            invalid,
            InvocationResponse::Direct(result)
                if !result.ok && result.summary.contains("unknown argument")
        ));
        assert_eq!(clock.now(), started);
        assert_eq!(executions.load(Ordering::SeqCst), 0);
    }

    fn task_id(response: InvocationResponse) -> crate::domain::invocation::TaskId {
        match response {
            InvocationResponse::Task(snapshot) => snapshot.task_id,
            other => panic!("expected durable task: {other:?}"),
        }
    }

    fn submit_at_receipt(
        runtime: &DaemonInvocationRuntime,
        request: InvocationRequest,
    ) -> Result<InvocationResponse, DaemonInvocationError> {
        ensure_platform_xml_workspace(request.workspace_hint());
        let response_deadline = runtime
            .capture_response_deadline()
            .restrict_to_frontend_budget(Duration::from_millis(request.response_budget_ms()));
        runtime.submit(request, response_deadline)
    }

    fn ensure_platform_xml_workspace(workspace_hint: &str) {
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
    }

    #[test]
    fn daemon_shared_delivery_releases_request_admission_before_wait_and_shares_across_worktrees() {
        let task_root = tempfile::tempdir().unwrap();
        let workspace_parent = tempfile::tempdir().unwrap();
        let roots = (0..2)
            .map(|index| {
                let root = workspace_parent
                    .path()
                    .join(format!("delivery-workspace-{index}"));
                std::fs::create_dir(&root).unwrap();
                std::fs::canonicalize(root).unwrap()
            })
            .collect::<Vec<_>>();
        let (store, _) =
            FileInvocationStore::open(task_root.path(), Arc::new(SystemEpochMillisClock)).unwrap();
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
            producers: Arc::clone(&producers),
            producer_entered,
            joined,
            release: Arc::clone(&release),
        });
        let runtime = DaemonInvocationRuntime::new(Arc::new(store), service, Arc::new(TokioClock));
        let request = |root: &std::path::Path| {
            InvocationRequest::new(
                ToolIdentity::Run,
                serde_json::json!({}),
                root.to_string_lossy(),
                0,
            )
            .unwrap()
        };

        let first = task_id(submit_at_receipt(&runtime, request(&roots[0])).unwrap());
        producer_wait
            .recv_timeout(Duration::from_secs(10))
            .expect("first task entered delivery producer");
        let second = task_id(submit_at_receipt(&runtime, request(&roots[1])).unwrap());
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
            Arc::as_ptr(&runtime.deliveries) as usize,
            "actor bindings retain the daemon-owned DeliveryDesk"
        );
        assert_eq!(runtime.workspace_actors.live_len_for_test().unwrap(), 2);
        assert_eq!(producers.load(Ordering::SeqCst), 1);
        for task_id in [first, second] {
            assert_eq!(
                runtime.get(task_id).unwrap().status,
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
            let terminal = runtime.wait(task_id, 7_000).unwrap();
            assert_eq!(terminal.status, InvocationStatus::Completed);
            assert_eq!(
                terminal.result.unwrap().summary,
                "shared daemon delivery ready"
            );
        }
        assert_eq!(producers.load(Ordering::SeqCst), 1);
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
        let runtime_service = Arc::new(RuntimeJobService::coordination_only_for_test(
            runtime_authority.path(),
        ));

        for kind in [
            LongCapabilityKind::Index,
            LongCapabilityKind::Provider(provider_key),
            LongCapabilityKind::Runtime {
                lease_id: runtime_lease.id,
            },
        ] {
            let task_root = tempfile::tempdir().unwrap();
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
            let (store, _) =
                FileInvocationStore::open(task_root.path(), Arc::new(SystemEpochMillisClock))
                    .unwrap();
            let (service, producers, producer_wait, joined_wait, release) =
                shared_capability_service(kind.clone());
            let runtime =
                DaemonInvocationRuntime::new(Arc::new(store), service, Arc::new(TokioClock));
            let runtime = if matches!(kind, LongCapabilityKind::Runtime { .. }) {
                runtime.with_runtime_service_for_test(Arc::clone(&runtime_service))
            } else {
                runtime
            };
            let request = |root: &std::path::Path| {
                InvocationRequest::new(
                    ToolIdentity::Run,
                    serde_json::json!({}),
                    root.to_string_lossy(),
                    0,
                )
                .unwrap()
            };

            let first = task_id(submit_at_receipt(&runtime, request(&roots[0])).unwrap());
            producer_wait
                .recv_timeout(Duration::from_secs(10))
                .expect("first task entered long-work producer");
            let second = task_id(submit_at_receipt(&runtime, request(&roots[1])).unwrap());
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
                    runtime.get(task_id).unwrap().status,
                    InvocationStatus::Working
                );
            }
            assert_eq!(
                runtime.workspace_actors.live_len_for_test().unwrap(),
                if matches!(kind, LongCapabilityKind::Index) {
                    1
                } else {
                    2
                }
            );
            let (released, wake) = &*release;
            *released.lock().unwrap() = true;
            wake.notify_all();
            let first_result = runtime.wait(first, 7_000).unwrap().result.unwrap();
            let second_result = runtime.wait(second, 7_000).unwrap().result.unwrap();
            if matches!(kind, LongCapabilityKind::Index) {
                assert_eq!(first_result.summary, "index");
                assert_eq!(second_result.summary, "index");
            } else {
                assert_eq!(first_result.summary, "root-0");
                assert_eq!(second_result.summary, "root-1");
            }
        }
    }

    #[test]
    fn daemon_index_work_separates_worktrees_and_rejects_stale_revision_publication() {
        // Distinct actor identities intentionally cannot join one Index key.
        let task_root = tempfile::tempdir().unwrap();
        let workspace_parent = tempfile::tempdir().unwrap();
        let roots = (0..2)
            .map(|index| {
                let root = workspace_parent.path().join(format!("index-root-{index}"));
                std::fs::create_dir(&root).unwrap();
                std::fs::write(root.join("marker.txt"), format!("root-{index}")).unwrap();
                std::fs::canonicalize(root).unwrap()
            })
            .collect::<Vec<_>>();
        let (store, _) =
            FileInvocationStore::open(task_root.path(), Arc::new(SystemEpochMillisClock)).unwrap();
        let (service, producers, producer_wait, joined_wait, release) =
            shared_capability_service(LongCapabilityKind::Index);
        let runtime = DaemonInvocationRuntime::new(Arc::new(store), service, Arc::new(TokioClock));
        let request = |root: &std::path::Path| {
            InvocationRequest::new(
                ToolIdentity::Run,
                serde_json::json!({}),
                root.to_string_lossy(),
                0,
            )
            .unwrap()
        };
        let first = task_id(submit_at_receipt(&runtime, request(&roots[0])).unwrap());
        producer_wait
            .recv_timeout(Duration::from_secs(10))
            .expect("first index producer");
        let second = task_id(submit_at_receipt(&runtime, request(&roots[1])).unwrap());
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
            runtime.wait(first, 7_000).unwrap().status,
            InvocationStatus::Completed
        );
        assert_eq!(
            runtime.wait(second, 7_000).unwrap().status,
            InvocationStatus::Completed
        );

        // A new trusted source revision starts a new producer, and the result
        // staged under the prior revision cannot cross the actor publication fence.
        let task_root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("marker.txt"), "old").unwrap();
        std::fs::write(
            workspace.path().join("Module.bsl"),
            "Procedure Old()\nEndProcedure",
        )
        .unwrap();
        let root = std::fs::canonicalize(workspace.path()).unwrap();
        let (store, _) =
            FileInvocationStore::open(task_root.path(), Arc::new(SystemEpochMillisClock)).unwrap();
        let producers = Arc::new(AtomicUsize::new(0));
        let (producer_entered, producer_wait) = mpsc::channel();
        let (joined, joined_wait) = mpsc::channel();
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let (mark_dirty, dirty_request) = mpsc::channel();
        let (dirty_done, dirty_wait) = mpsc::channel();
        let service = Arc::new(RevisionChangingIndexService {
            producers: Arc::clone(&producers),
            producer_entered,
            joined,
            release: Arc::clone(&release),
            first_execution: AtomicBool::new(false),
            mark_dirty: Mutex::new(dirty_request),
            dirty_done,
        });
        let runtime = DaemonInvocationRuntime::new(Arc::new(store), service, Arc::new(TokioClock));
        let first = task_id(submit_at_receipt(&runtime, request(&root)).unwrap());
        producer_wait.recv_timeout(Duration::from_secs(10)).unwrap();
        let old_key = joined_wait.recv_timeout(Duration::from_secs(10)).unwrap();
        std::fs::write(root.join("marker.txt"), "new").unwrap();
        std::fs::write(root.join("Module.bsl"), "Procedure New()\nEndProcedure").unwrap();
        mark_dirty.send(()).unwrap();
        dirty_wait.recv_timeout(Duration::from_secs(10)).unwrap();
        let second = task_id(submit_at_receipt(&runtime, request(&root)).unwrap());
        producer_wait.recv_timeout(Duration::from_secs(10)).unwrap();
        let new_key = joined_wait.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_ne!(old_key, new_key);
        assert_eq!(producers.load(Ordering::SeqCst), 2);
        let (released, wake) = &*release;
        *released.lock().unwrap() = true;
        wake.notify_all();
        assert_eq!(
            runtime.wait(first, 7_000).unwrap().status,
            InvocationStatus::Failed
        );
        let second = runtime.wait(second, 7_000).unwrap();
        assert_eq!(second.status, InvocationStatus::Completed);
        assert_eq!(second.result.unwrap().summary, "new");
    }

    #[test]
    fn daemon_long_work_rejects_replaced_actor_root_before_reuse_or_publication() {
        let task_root = tempfile::tempdir().unwrap();
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("marker.txt"), "old").unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let (store, _) =
            FileInvocationStore::open(task_root.path(), Arc::new(SystemEpochMillisClock)).unwrap();
        let (service, _, producer_wait, joined_wait, release) =
            shared_capability_service(LongCapabilityKind::Index);
        let runtime = DaemonInvocationRuntime::new(Arc::new(store), service, Arc::new(TokioClock));
        let request = || {
            InvocationRequest::new(
                ToolIdentity::Run,
                serde_json::json!({}),
                root.to_string_lossy(),
                0,
            )
            .unwrap()
        };
        let first = task_id(submit_at_receipt(&runtime, request()).unwrap());
        producer_wait.recv_timeout(Duration::from_secs(10)).unwrap();
        joined_wait.recv_timeout(Duration::from_secs(10)).unwrap();
        let moved = parent.path().join("workspace-old");
        std::fs::rename(&root, moved).unwrap();
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("marker.txt"), "replacement").unwrap();
        let rejected = submit_at_receipt(&runtime, request()).unwrap();
        assert!(matches!(rejected, InvocationResponse::Direct(result) if !result.ok));
        let (released, wake) = &*release;
        *released.lock().unwrap() = true;
        wake.notify_all();
        assert_eq!(
            runtime.wait(first, 7_000).unwrap().status,
            InvocationStatus::Failed
        );
    }

    #[test]
    fn daemon_exact_long_work_ownership_contract() {
        daemon_long_work_capabilities_handoff_before_wait_and_preserve_exact_ownership();
        daemon_index_work_separates_worktrees_and_rejects_stale_revision_publication();
        daemon_long_work_rejects_replaced_actor_root_before_reuse_or_publication();
        crate::infrastructure::runtime_jobs::run_runtime_resource_tree_contract_for_test();
    }

    #[test]
    fn daemon_named_contract_executes_runtime_resource_tree_evidence() {
        crate::infrastructure::runtime_jobs::reset_runtime_resource_contract_executions_for_test();

        daemon_exact_long_work_ownership_contract();

        assert_eq!(
            crate::infrastructure::runtime_jobs::runtime_resource_contract_executions_for_test(),
            1,
            "daemon CTR named check did not execute its runtime-tree obligations"
        );
    }

    fn live_actor_capacity_reuses_alias_and_rejects_only_a_distinct_third_root() {
        let task_root = tempfile::tempdir().unwrap();
        let workspace_parent = tempfile::tempdir().unwrap();
        let roots = (0..3)
            .map(|index| {
                let root = workspace_parent.path().join(format!("workspace-{index}"));
                std::fs::create_dir(&root).unwrap();
                std::fs::canonicalize(root).unwrap()
            })
            .collect::<Vec<_>>();
        let (store, _) =
            FileInvocationStore::open(task_root.path(), Arc::new(SystemEpochMillisClock)).unwrap();
        let (entered, _entered_wait) = mpsc::channel();
        let runtime = DaemonInvocationRuntime {
            executor: Arc::new(InvocationExecutor::new(
                Arc::new(store),
                Arc::new(TokioClock),
            )),
            service: Arc::new(BlockingService { entered }),
            workspace_actors: WorkspaceActorRegistry::with_capacity_for_test(2),
            deliveries: Arc::new(crate::infrastructure::engine_delivery::DeliveryDesk::default()),
            provider_hosts: Arc::new(ProviderHostOwner::default()),
            runtime_resources: Arc::new(RuntimeResourceOwner::default()),
            runtime_service: None,
        };
        let request = |root: &std::path::Path| {
            InvocationRequest::new(
                ToolIdentity::Run,
                serde_json::json!({}),
                root.to_string_lossy(),
                0,
            )
            .unwrap()
        };

        let first = task_id(submit_at_receipt(&runtime, request(&roots[0])).unwrap());
        let second = task_id(submit_at_receipt(&runtime, request(&roots[1])).unwrap());
        let alias = task_id(submit_at_receipt(&runtime, request(&roots[0].join("."))).unwrap());
        // A returned task snapshot is the durable admission point. Its actor lease is already
        // retained before the background worker is scheduled, so capacity evidence must not
        // depend on runner scheduling.
        let rejected = submit_at_receipt(&runtime, request(&roots[2])).unwrap_err();
        assert_eq!(rejected.protocol_code(), DaemonErrorCode::WorkspaceCapacity);
        assert_eq!(runtime.workspace_actors.entry_len_for_test().unwrap(), 2);

        for task_id in [first, second, alias] {
            assert_eq!(
                runtime.cancel(task_id).unwrap().status,
                crate::domain::invocation::InvocationStatus::Cancelled
            );
        }
    }

    fn concurrent_same_identity_admission_creates_one_actor() {
        let workspace = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(workspace.path()).unwrap();
        let context = discover_workspace(Some(root.clone())).unwrap();
        let registry = Arc::new(WorkspaceActorRegistry::default());
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
        let task_root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let (store, _) =
            FileInvocationStore::open(task_root.path(), Arc::new(SystemEpochMillisClock)).unwrap();
        let (entered, _entered_wait) = mpsc::channel();
        let runtime = DaemonInvocationRuntime::new(
            Arc::new(store),
            Arc::new(BlockingService { entered }),
            Arc::new(TokioClock),
        );
        runtime.workspace_actors.poison_for_test();

        let error = submit_at_receipt(
            &runtime,
            InvocationRequest::new(
                ToolIdentity::Run,
                serde_json::json!({}),
                std::fs::canonicalize(workspace.path())
                    .unwrap()
                    .to_string_lossy(),
                0,
            )
            .unwrap(),
        )
        .unwrap_err();
        assert_eq!(
            error.protocol_code(),
            DaemonErrorCode::WorkspaceRegistryFailed
        );
    }

    fn sequential_direct_admissions_release_actor_capabilities_and_prune_dead_entries() {
        let task_root = tempfile::tempdir().unwrap();
        let workspaces = tempfile::tempdir().unwrap();
        let (store, _) =
            FileInvocationStore::open(task_root.path(), Arc::new(SystemEpochMillisClock)).unwrap();
        let runtime = DaemonInvocationRuntime::new(
            Arc::new(store),
            Arc::new(RejectingAfterActorAdmissionService),
            Arc::new(TokioClock),
        );

        for index in 0..80 {
            let workspace = workspaces.path().join(format!("workspace-{index}"));
            std::fs::create_dir(&workspace).unwrap();
            let response = submit_at_receipt(
                &runtime,
                InvocationRequest::new(
                    ToolIdentity::Run,
                    serde_json::json!({}),
                    std::fs::canonicalize(&workspace).unwrap().to_string_lossy(),
                    0,
                )
                .unwrap(),
            )
            .unwrap();
            assert!(matches!(
                response,
                InvocationResponse::Direct(result)
                    if !result.ok && result.summary == "test rejection after actor admission"
            ));
            assert!(runtime.workspace_actors.entry_len_for_test().unwrap() <= 1);
        }
        assert_eq!(runtime.workspace_actors.entry_len_for_test().unwrap(), 1);
    }

    #[test]
    pub(crate) fn restart_request_does_not_claim_noncooperative_actor_released_in_process() {
        let task_root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let (store, _) =
            FileInvocationStore::open(task_root.path(), Arc::new(SystemEpochMillisClock)).unwrap();
        let (entered, entered_wait) = mpsc::channel();
        let (release, release_wait) = mpsc::channel();
        let runtime = DaemonInvocationRuntime::new_with_reconciliation_budget_for_test(
            Arc::new(UncertainCancelStore { inner: store }),
            Arc::new(NonCooperativeActorService {
                entered,
                release: Mutex::new(release_wait),
            }),
            Arc::new(TokioClock),
            Duration::from_millis(100),
        );
        let request = InvocationRequest::new(
            ToolIdentity::Run,
            serde_json::json!({}),
            std::fs::canonicalize(workspace.path())
                .unwrap()
                .to_string_lossy(),
            0,
        )
        .unwrap();
        let task_id = task_id(submit_at_receipt(&runtime, request).unwrap());
        entered_wait.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(runtime.workspace_actors.live_len_for_test().unwrap(), 1);

        assert!(matches!(
            runtime.cancel(task_id),
            Err(DaemonInvocationError::Executor(
                InvocationExecutorError::RestartRequested
            ))
        ));
        assert!(runtime.restart_requested());
        assert_eq!(
            runtime.workspace_actors.live_len_for_test().unwrap(),
            1,
            "the executing ActorBoundExecution still owns the actor until process death"
        );

        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while runtime.workspace_actors.live_len_for_test().unwrap() != 0
            && Instant::now() < deadline
        {
            std::thread::yield_now();
        }
        assert_eq!(runtime.workspace_actors.live_len_for_test().unwrap(), 0);
    }

    #[test]
    fn daemon_workspace_actor_admission_is_concurrent_bounded_and_fail_closed() {
        live_actor_capacity_reuses_alias_and_rejects_only_a_distinct_third_root();
        concurrent_same_identity_admission_creates_one_actor();
        poisoned_registry_is_a_closed_internal_error_and_admits_nothing();
        sequential_direct_admissions_release_actor_capabilities_and_prune_dead_entries();
    }

    #[test]
    fn typed_executor_errors_map_to_closed_protocol_codes_without_text_matching() {
        let storage = DaemonInvocationError::Executor(InvocationExecutorError::Store(
            InvocationStoreError::Storage(
                "SECRET runtime prose /private/path must not classify".into(),
            ),
        ));
        assert_eq!(storage.protocol_code(), DaemonErrorCode::StoreFailed);
        assert_eq!(
            DaemonInvocationError::Executor(InvocationExecutorError::Store(
                InvocationStoreError::NotFound,
            ))
            .protocol_code(),
            DaemonErrorCode::TaskNotFound
        );
        assert_eq!(
            DaemonInvocationError::Executor(InvocationExecutorError::Store(
                InvocationStoreError::Capacity { max_records: 4_096 },
            ))
            .protocol_code(),
            DaemonErrorCode::TaskCapacity
        );
        assert_eq!(
            DaemonInvocationError::Executor(InvocationExecutorError::Store(
                InvocationStoreError::Expired,
            ))
            .protocol_code(),
            DaemonErrorCode::TaskExpired
        );
        assert_eq!(
            DaemonInvocationError::Executor(InvocationExecutorError::ExecutionFailed)
                .protocol_code(),
            DaemonErrorCode::InvocationFailed
        );
        assert_eq!(
            DaemonInvocationError::Executor(InvocationExecutorError::RestartRequested)
                .protocol_code(),
            DaemonErrorCode::DurabilityUncertain
        );
        assert_eq!(
            DaemonInvocationError::WorkspaceRegistryFailed.protocol_code(),
            DaemonErrorCode::WorkspaceRegistryFailed
        );
    }
}

fn tokens_equal(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.bytes()
        .zip(right.bytes())
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

#[derive(Default)]
struct LeaseRegistry {
    leases: Mutex<HashSet<String>>,
}

impl LeaseRegistry {
    fn acquire(self: &Arc<Self>, lease: String) -> Result<LeaseAdmission, String> {
        let mut leases = self
            .leases
            .lock()
            .map_err(|_| "daemon owner lease registry is poisoned".to_string())?;
        if leases.contains(&lease) {
            return Ok(LeaseAdmission::Duplicate);
        }
        if leases.len() >= MAX_OWNER_SESSIONS {
            return Ok(LeaseAdmission::Capacity);
        }
        leases.insert(lease.clone());
        drop(leases);
        Ok(LeaseAdmission::Acquired(LeaseGuard {
            registry: Arc::clone(self),
            lease,
        }))
    }

    fn is_empty(&self) -> Result<bool, String> {
        self.leases
            .lock()
            .map(|leases| leases.is_empty())
            .map_err(|_| "daemon owner lease registry is poisoned".to_string())
    }
}

enum LeaseAdmission {
    Acquired(LeaseGuard),
    Duplicate,
    Capacity,
}

struct LeaseGuard {
    registry: Arc<LeaseRegistry>,
    lease: String,
}

impl Drop for LeaseGuard {
    fn drop(&mut self) {
        if let Ok(mut leases) = self.registry.leases.lock() {
            leases.remove(&self.lease);
        }
    }
}

struct ConnectionSlot {
    admitted: Arc<AtomicUsize>,
}

impl ConnectionSlot {
    fn acquire(admitted: Arc<AtomicUsize>) -> Option<Self> {
        admitted
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < MAX_HANDSHAKES).then_some(current + 1)
            })
            .ok()
            .map(|_| Self { admitted })
    }
}

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        self.admitted.fetch_sub(1, Ordering::AcqRel);
    }
}

fn reject_overloaded_connection(mut stream: TcpStream) {
    let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
    let _ = write_response(
        &mut stream,
        &ServerResponse::error(DaemonErrorCode::Overloaded),
    );
}

fn reap_finished_handlers(handlers: Vec<JoinHandle<()>>) -> Vec<JoinHandle<()>> {
    let mut active = Vec::with_capacity(handlers.len());
    for handler in handlers {
        if handler.is_finished() {
            let _ = handler.join();
        } else {
            active.push(handler);
        }
    }
    active
}

fn join_handlers(handlers: Vec<JoinHandle<()>>) {
    for handler in handlers {
        let _ = handler.join();
    }
}

fn daemon_io_error(operation: &str, error: io::Error) -> String {
    format!("{operation}: {error}")
}

#[cfg(test)]
#[derive(Debug, Default)]
struct HandshakePauseState {
    entered: bool,
    released: bool,
}

#[cfg(test)]
#[derive(Debug)]
struct HandshakePause {
    state: Mutex<HandshakePauseState>,
    wake: std::sync::Condvar,
}

#[cfg(test)]
pub(crate) struct HandshakePauseGuard {
    pause: Arc<HandshakePause>,
}

#[cfg(test)]
impl HandshakePauseGuard {
    pub(crate) fn wait_until_entered(&self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut state = self.pause.state.lock().expect("handshake pause state");
        while !state.entered {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "daemon handler did not reach handshake pause"
            );
            let (next, timeout) = self
                .pause
                .wake
                .wait_timeout(state, remaining)
                .expect("handshake pause wait");
            state = next;
            assert!(
                !timeout.timed_out() || state.entered,
                "daemon handler pause timed out"
            );
        }
    }

    pub(crate) fn release(&self) {
        let mut state = self.pause.state.lock().expect("handshake pause state");
        state.released = true;
        self.pause.wake.notify_all();
    }
}

#[cfg(test)]
impl Drop for HandshakePauseGuard {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
pub(crate) fn install_handshake_pause() -> HandshakePauseGuard {
    let pause = Arc::new(HandshakePause {
        state: Mutex::new(HandshakePauseState::default()),
        wake: std::sync::Condvar::new(),
    });
    HandshakePauseGuard { pause }
}

#[cfg(test)]
fn pause_before_handshake_if_configured(pause: Option<Arc<HandshakePause>>) {
    let Some(pause) = pause else {
        return;
    };
    let mut state = pause.state.lock().expect("handshake pause state");
    state.entered = true;
    pause.wake.notify_all();
    while !state.released {
        state = pause.wake.wait(state).expect("handshake pause wait");
    }
}
