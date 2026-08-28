use super::identity::{CoreIdentity, DaemonStateDirectory};
use super::protocol::{
    parse_request, read_bounded_request_line, read_bounded_request_line_before, ClientRequest,
    DaemonErrorCode, DaemonTaskSnapshot, EndpointRecord, InvocationRequest, InvocationResponse,
    ServerResponse, DAEMON_PROTOCOL_VERSION,
};
use crate::application::invocation::{
    normalized_arguments_hash, InvocationExecutor, InvocationExecutorError,
    InvocationResponseDeadline, PreparedDaemonInvocation, RESPONSE_SERIALIZATION_MARGIN_MS,
    TASK_RECONCILIATION_BUDGET,
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
use crate::domain::apply::ApplyRequest;
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::invocation::{
    DomainResult, InvocationFailure, InvocationOutcome, SafeIdentityHash,
};
use crate::domain::project_sources::{SourceFormat, SourceProfile, SourceSetKind};
use crate::infrastructure::runtime_jobs::{RuntimeJobService, RuntimeResourceOwner};
use crate::infrastructure::source_selection_evidence::discover_project_source_admission;
use crate::infrastructure::workspace::discover_workspace;
use crate::infrastructure::workspace_actor::{
    ApplyAdmission, IndexWorkIdentity, ProviderRootBinding, WorkspaceActor, WorkspaceActorRegistry,
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
        let source_profile = self.binding.source_profile();
        let platform_profile = source_profile.platform_profile().ok_or_else(|| {
            "actor-bound logical source has no supported platform profile".to_string()
        })?;
        let read =
            crate::infrastructure::v13_read_port::ProviderReadAuthority::new_with_revision_lease(
                self.binding.source_set_name(),
                self.identity.clone(),
                self.binding.source_kind(),
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
            crate::application::invocation_store::ToolIdentity::Apply => {
                ActorExecutionRevision::UnpublishedApply
            }
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
    UnpublishedApply,
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

    pub(super) fn admitted_source_set_names(&self) -> Vec<&str> {
        self.invocation
            .read_sources
            .iter()
            .map(|source| source.binding.source_set_name())
            .collect()
    }

    pub(super) fn admit_apply(
        &self,
        request: &ApplyRequest,
        cancellation: &CancellationToken,
    ) -> Result<(ProviderRootBinding, ApplyAdmission), String> {
        let binding = self
            .invocation
            .read_sources
            .iter()
            .find(|source| source.binding.source_set_name() == request.at().source_set())
            .map(|source| source.binding.clone())
            .ok_or_else(|| {
                "apply source set was not admitted by the workspace actor".to_string()
            })?;
        self.invocation.actor.validate_binding(&binding)?;
        let admission = self.invocation.actor.admit_apply(
            &binding,
            request.if_rev(),
            request.dry_run(),
            ProviderDeadline::from_budget(ACTOR_OPERATION_BUDGET),
            cancellation,
        )?;
        Ok((binding, admission))
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
            ActorExecutionRevision::UnpublishedApply => {
                let requires_actor_publication = match &staged {
                    Ok(result) => result.ok || !result.changed.is_empty() || result.rev.is_some(),
                    Err(_) => false,
                };
                if requires_actor_publication {
                    return Err(
                        "unpublished Apply result requires real actor apply publication"
                            .to_string(),
                    );
                }
                Ok(staged)
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
    retained_identity: crate::infrastructure::platform::filesystem::FileIdentity,
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
    let mut checkpoint = || {
        response_deadline
            .checkpoint_actor_admission()
            .map_err(str::to_string)
    };
    let source_admission =
        discover_project_source_admission(&context.workspace_root, &mut checkpoint)
            .map_err(|_| WorkspaceAdmissionError::Invalid)?;
    let mut admitted_sources = source_admission
        .map()
        .source_sets
        .iter()
        .filter(|source| source.source_format == SourceFormat::PlatformXml)
        .map(|source| {
            let relative = closed_daemon_source_relative_path(&source.path)
                .map_err(|_| WorkspaceAdmissionError::Invalid)?;
            let retained_identity = source_admission
                .source_root_identity(&relative)
                .ok_or(WorkspaceAdmissionError::Invalid)?;
            let root = context.workspace_root.join(&relative);
            Ok(DiscoveredActorSource {
                name: source.name.clone(),
                kind: source.kind,
                source_format: source.source_format,
                source_profile: SourceProfile::platform_xml_8_3_27_format_2_20(),
                root,
                retained_identity,
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
                .and_then(|binding| {
                    if binding.retained_root().identity() != source.retained_identity {
                        return Err("actor binding does not match retained source-map admission"
                            .to_string());
                    }
                    Ok(ActorReadSourceBinding { binding })
                })
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

fn closed_daemon_source_relative_path(path: &str) -> Result<std::path::PathBuf, String> {
    let mut relative = std::path::PathBuf::new();
    for component in std::path::Path::new(path).components() {
        match component {
            std::path::Component::Normal(name) => relative.push(name),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => {
                return Err("source-map route is not workspace-relative".to_string());
            }
        }
    }
    Ok(relative)
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
    startup_pause: Option<Arc<HandshakePause>>,
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
            startup_pause: None,
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
    pub(crate) fn with_startup_pause(mut self, pause: &HandshakePauseGuard) -> Self {
        self.startup_pause = Some(Arc::clone(&pause.pause));
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
    #[cfg(test)]
    pause_test_thread_if_configured(config.startup_pause.clone());
    let active_leases = Arc::new(LeaseRegistry::default());
    let admitted_connections = Arc::new(AtomicUsize::new(0));
    let shutting_down = Arc::new(AtomicBool::new(false));
    let mut handlers = Vec::new();
    let mut idle_since = Instant::now();
    let mut restart_requested = false;

    loop {
        match listener.accept() {
            Ok((stream, address)) if address.ip().is_loopback() => {
                let connection = AcceptedConnection {
                    stream,
                    handshake_deadline: Instant::now() + HANDSHAKE_READ_TIMEOUT,
                };
                match ConnectionSlot::acquire(Arc::clone(&admitted_connections)) {
                    Some(slot) => handlers.push(spawn_connection_handler(
                        connection,
                        record.clone(),
                        Arc::clone(&active_leases),
                        Arc::clone(&shutting_down),
                        Arc::clone(&invocation_runtime),
                        slot,
                        #[cfg(test)]
                        config.handshake_pause.clone(),
                    )),
                    None => reject_overloaded_connection(connection),
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

struct AcceptedConnection {
    stream: TcpStream,
    handshake_deadline: Instant,
}

fn spawn_connection_handler(
    connection: AcceptedConnection,
    record: EndpointRecord,
    active_leases: Arc<LeaseRegistry>,
    shutting_down: Arc<AtomicBool>,
    invocation_runtime: Arc<DaemonInvocationRuntime>,
    slot: ConnectionSlot,
    #[cfg(test)] handshake_pause: Option<Arc<HandshakePause>>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        #[cfg(test)]
        pause_test_thread_if_configured(handshake_pause);
        let _ = handle_connection(
            connection.stream,
            connection.handshake_deadline,
            &record,
            &active_leases,
            &shutting_down,
            &invocation_runtime,
            slot,
        );
    })
}

enum HandshakeRequestError {
    Invalid,
    Transport,
}

fn read_handshake_request(
    reader: &mut BufReader<TcpStream>,
    handshake_deadline: Instant,
) -> Result<ClientRequest, HandshakeRequestError> {
    let bytes = read_bounded_request_line_before(reader, |reader| {
        let remaining = handshake_deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| io::Error::from(io::ErrorKind::TimedOut))?;
        reader.get_ref().set_read_timeout(Some(remaining))
    });
    if Instant::now() >= handshake_deadline {
        return Err(HandshakeRequestError::Transport);
    }
    let request = match bytes {
        Ok(bytes) => parse_request(&bytes).map_err(|_| HandshakeRequestError::Invalid),
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            Err(HandshakeRequestError::Invalid)
        }
        Err(_) => Err(HandshakeRequestError::Transport),
    };
    if Instant::now() >= handshake_deadline {
        return Err(HandshakeRequestError::Transport);
    }
    request
}

fn handle_connection(
    mut stream: TcpStream,
    handshake_deadline: Instant,
    record: &EndpointRecord,
    active_leases: &Arc<LeaseRegistry>,
    shutting_down: &AtomicBool,
    invocation_runtime: &DaemonInvocationRuntime,
    handshake_slot: ConnectionSlot,
) -> Result<(), String> {
    let reader_stream = stream
        .try_clone()
        .map_err(|error| daemon_io_error("clone daemon client stream", error))?;
    let mut reader = BufReader::new(reader_stream);
    let request = match read_handshake_request(&mut reader, handshake_deadline) {
        Ok(request) => request,
        Err(HandshakeRequestError::Invalid) => {
            write_response_before(
                &mut stream,
                &ServerResponse::error(DaemonErrorCode::InvalidRequest),
                handshake_deadline,
            )?;
            return Ok(());
        }
        Err(HandshakeRequestError::Transport) => return Ok(()),
    };

    let ClientRequest::Hello {
        protocol_version,
        token,
        core_identity,
        owner_lease,
    } = request
    else {
        write_response_before(
            &mut stream,
            &ServerResponse::error(DaemonErrorCode::HandshakeRequired),
            handshake_deadline,
        )?;
        return Ok(());
    };
    if protocol_version != DAEMON_PROTOCOL_VERSION {
        write_response_before(
            &mut stream,
            &ServerResponse::error(DaemonErrorCode::ProtocolMismatch),
            handshake_deadline,
        )?;
        return Ok(());
    }
    if core_identity != *record.core_identity() {
        write_response_before(
            &mut stream,
            &ServerResponse::error(DaemonErrorCode::CoreMismatch),
            handshake_deadline,
        )?;
        return Ok(());
    }
    if !tokens_equal(&token, record.token()) {
        write_response_before(
            &mut stream,
            &ServerResponse::error(DaemonErrorCode::Unauthorized),
            handshake_deadline,
        )?;
        return Ok(());
    }

    let _owner = match active_leases.acquire(owner_lease)? {
        LeaseAdmission::Acquired(owner) => owner,
        LeaseAdmission::Duplicate => {
            write_response_before(
                &mut stream,
                &ServerResponse::error(DaemonErrorCode::DuplicateLease),
                handshake_deadline,
            )?;
            return Ok(());
        }
        LeaseAdmission::Capacity => {
            write_response_before(
                &mut stream,
                &ServerResponse::error(DaemonErrorCode::OwnerCapacity),
                handshake_deadline,
            )?;
            return Ok(());
        }
    };
    // The owner lease becomes the lifecycle fence before the pre-authentication admission
    // permit is released, so idle shutdown observes at least one of them throughout handoff.
    drop(handshake_slot);
    write_response_before(
        &mut stream,
        &ServerResponse::ready(record),
        handshake_deadline,
    )?;
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
                let deadline = task_response_deadline(Duration::ZERO);
                write_task_response_before(&mut stream, invocation_runtime.get(task_id), deadline)?
            }
            ClientRequest::WaitTask { task_id, wait_ms } => {
                let deadline = task_response_deadline(Duration::from_millis(wait_ms));
                write_task_response_before(
                    &mut stream,
                    invocation_runtime.wait(task_id, wait_ms),
                    deadline,
                )?
            }
            ClientRequest::CancelTask { task_id } => {
                let deadline = task_response_deadline(Duration::ZERO);
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

fn task_response_deadline(wait_budget: Duration) -> Instant {
    // The frontend owns the one absolute task-operation cutoff and closes a late session.
    // Protocol v3 does not transmit that cutoff, so the daemon must not manufacture a fresh
    // 125 ms operation window here. The daemon-side bound covers the executor's canonical store
    // reconciliation allowance and response margin, capped by the independent session safety
    // limit; it does not replenish the frontend cutoff.
    let operation_budget = wait_budget
        .saturating_add(TASK_RECONCILIATION_BUDGET)
        .saturating_add(Duration::from_millis(RESPONSE_SERIALIZATION_MARGIN_MS))
        .min(OWNER_RESPONSE_WRITE_TIMEOUT);
    Instant::now() + operation_budget
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
        NewInvocationRecord, SafeFailureReason, SafeStatusMessage, StoredInvocationRecord,
        TaskTransition, ToolIdentity,
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
        use quote::ToTokens;
        use syn::visit::Visit;

        const CAPABILITY: &str = "ActorReadSourceCapability";

        fn tokens(node: &impl ToTokens) -> String {
            node.to_token_stream().to_string()
        }

        fn is_pub_super(visibility: &syn::Visibility) -> bool {
            matches!(
                visibility,
                syn::Visibility::Restricted(restricted)
                    if restricted.in_token.is_none() && restricted.path.is_ident("super")
            )
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
            let syn::Expr::MethodCall(from_profile) = to_result.receiver.as_ref() else {
                return Err(
                    "platform profile must be derived from the bound source profile".to_string(),
                );
            };
            let Some(error_closure) = to_result.args.first() else {
                return Err("platform profile requires one exact error closure".to_string());
            };
            if from_profile.method != "platform_profile"
                || !from_profile.args.is_empty()
                || !expression_is(from_profile.receiver.as_ref(), "source_profile")
                || !expression_is(
                    error_closure,
                    r#"|| {
                        "actor-bound logical source has no supported platform profile".to_string()
                    }"#,
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
                "crate::infrastructure::v13_read_port::ProviderReadAuthority::new_with_revision_lease",
            )?;
            let expected_read_args = [
                "self.binding.source_set_name()",
                "self.identity.clone()",
                "self.binding.source_kind()",
                "self.binding.retained_root()",
                "self.fence.revision()",
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
                "crate::infrastructure::v13_read::LogicalViewReadAuthority::with_read_authority",
            )?;
            let expected_authority_args =
                ["cancellation", "read", "platform_profile", "self.deadline"];
            if authority_args.len() != expected_authority_args.len()
                || authority_args
                    .iter()
                    .zip(expected_authority_args)
                    .any(|(actual, expected)| !expression_is(actual, expected))
            {
                return Err(format!(
                    "logical authority must preserve cancellation/read/profile/deadline exactly; found `{}`",
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
                |name: &syn::Ident| PROVEN_ATTRIBUTE_NAMES.contains(&name.to_string().as_str());
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
                if segment.ident == CAPABILITY {
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
                syn::UseTree::Name(name) => name.ident == CAPABILITY,
                syn::UseTree::Rename(rename) => {
                    rename.ident == CAPABILITY || rename.rename == CAPABILITY
                }
                syn::UseTree::Path(path) => {
                    path.ident == CAPABILITY || use_tree_mentions_capability(path.tree.as_ref())
                }
                syn::UseTree::Group(group) => group.items.iter().any(use_tree_mentions_capability),
                syn::UseTree::Glob(_) => false,
            }
        }

        fn use_tree_can_shadow_inert_macro(tree: &syn::UseTree) -> bool {
            const INERT_MACROS: [&str; 3] = ["format", "matches", "vec"];
            match tree {
                syn::UseTree::Name(name) => INERT_MACROS.contains(&name.ident.to_string().as_str()),
                syn::UseTree::Rename(rename) => {
                    INERT_MACROS.contains(&rename.ident.to_string().as_str())
                        || INERT_MACROS.contains(&rename.rename.to_string().as_str())
                }
                syn::UseTree::Path(path) => {
                    INERT_MACROS.contains(&path.ident.to_string().as_str())
                        || use_tree_can_shadow_inert_macro(path.tree.as_ref())
                }
                syn::UseTree::Group(group) => {
                    group.items.iter().any(use_tree_can_shadow_inert_macro)
                }
                syn::UseTree::Glob(_) => true,
            }
        }

        fn inert_macro_payload_is_closed(mac: &syn::Macro) -> bool {
            const FORBIDDEN_IDENTIFIERS: [&str; 14] = [
                CAPABILITY,
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
            let payload = tokens(&mac.tokens);
            !payload.contains('!')
                && !payload
                    .split_whitespace()
                    .any(|token| FORBIDDEN_IDENTIFIERS.contains(&token))
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

        struct CapabilityFunctionReference {
            owner_impl: Option<String>,
            signature: String,
        }

        #[derive(Default)]
        struct CapabilityItems<'ast> {
            declarations: Vec<&'ast syn::ItemStruct>,
            implementations: Vec<&'ast syn::ItemImpl>,
            aliases: Vec<String>,
            macro_references: Vec<String>,
            attribute_references: Vec<String>,
            item_references: Vec<String>,
            function_references: Vec<CapabilityFunctionReference>,
            constructions: Vec<CapabilityConstruction<'ast>>,
            capability_path_references: usize,
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
                if item.ident == CAPABILITY {
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
                if PROVEN_ATTRIBUTE_NAMES.contains(&item.ident.to_string().as_str())
                    || item.rename.as_ref().is_some_and(|(_, rename)| {
                        PROVEN_ATTRIBUTE_NAMES.contains(&rename.to_string().as_str())
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
                    .any(|segment| segment.ident == CAPABILITY);
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

            fn visit_path_segment(&mut self, segment: &'ast syn::PathSegment) {
                if segment.ident == CAPABILITY {
                    self.capability_path_references += 1;
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
        if found.capability_path_references != 3 {
            return Err(format!(
                "actor read capability must have exactly three production path references (impl, read_sources return, singleton construction), found {}",
                found.capability_path_references
            ));
        }
        if !found.item_references.is_empty() {
            return Err(format!(
                "actor read capability must not escape through another production item: {:?}",
                found.item_references
            ));
        }
        let expected_read_sources_signature = tokens(
            &syn::parse_str::<syn::Signature>(
                "fn read_sources(&self) -> Result<Vec<ActorReadSourceCapability>, String>",
            )
            .expect("the one actor capability return signature parses"),
        );
        let [read_sources_reference] = found.function_references.as_slice() else {
            return Err(format!(
                "actor read capability must appear in exactly one production function signature, found {}",
                found.function_references.len()
            ));
        };
        if read_sources_reference.owner_impl.as_deref() != Some("ActorBoundExecution")
            || read_sources_reference.signature != expected_read_sources_signature
        {
            return Err(format!(
                "only ActorBoundExecution::read_sources may return the capability; found owner={:?}, signature=`{}`",
                read_sources_reference.owner_impl, read_sources_reference.signature
            ));
        }

        let [construction] = found.constructions.as_slice() else {
            return Err(format!(
                "actor read capability must have exactly one production construction, found {}",
                found.constructions.len()
            ));
        };
        let expression = construction.expression;
        if construction.owner_impl.as_deref() != Some("ActorBoundExecution")
            || construction.function.as_deref() != Some("read_sources")
            || expression.qself.is_some()
            || !expression.path.is_ident(CAPABILITY)
            || !expression.attrs.is_empty()
            || expression.rest.is_some()
        {
            return Err(format!(
                "only ActorBoundExecution::read_sources may construct the capability; found owner={:?}, function={:?}, expression=`{}`",
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
            ("deadline", "lease.deadline"),
            ("fence", "source.fence.clone()"),
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
                "actor capability construction must use exact binding/identity/fence/deadline dataflow; found `{}`",
                tokens(expression)
            ));
        }
        let [declaration] = found.declarations.as_slice() else {
            return Err(format!(
                "actor read capability must have exactly one named declaration, found {}",
                found.declarations.len()
            ));
        };
        if !is_pub_super(&declaration.vis)
            || !declaration.generics.params.is_empty()
            || declaration.generics.where_clause.is_some()
            || !is_exact_dead_code_allow(&declaration.attrs)
        {
            return Err("actor read capability declaration must be exact `pub(super)` with no derives or generics".to_string());
        }
        let syn::Fields::Named(named) = &declaration.fields else {
            return Err("actor read capability must have four named private fields".to_string());
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
            ("fence".to_string(), "WorkspaceLogicalReadFence".to_string()),
            ("identity".to_string(), "String".to_string()),
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
                    false,
                    "const fn source_kind(&self) -> SourceSetKind",
                    "self.binding.source_kind()",
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
                || (*sibling_visible && !is_pub_super(&method.vis))
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

    #[test]
    pub(crate) fn actor_read_source_capability_is_sealed_after_binding() {
        let source = include_str!("server.rs");
        audit_actor_read_source_capability_api(source).unwrap_or_else(|error| panic!("{error}"));
        actor_read_source_capability_sibling_field_privacy_is_enforced_by_rustc();
        actor_read_source_capability_audit_rejects_sibling_visible_forge_and_mutator();
        actor_read_source_capability_ast_audit_rejects_split_sibling_visibility();
        actor_read_source_capability_ast_audit_rejects_multiline_derive_clone();
        actor_read_source_capability_ast_audit_rejects_multiline_manual_clone();
        actor_read_source_capability_ast_audit_rejects_multiline_extra_inherent_impl();
        actor_read_source_capability_ast_audit_rejects_macro_generated_api();
        actor_read_source_capability_ast_audit_rejects_opaque_item_macro_invocation();
        actor_read_source_capability_ast_audit_rejects_opaque_const_expression_macro();
        actor_read_source_capability_ast_audit_rejects_no_arg_opaque_const_expression_macro();
        actor_read_source_capability_ast_audit_rejects_opaque_static_and_statement_macros();
        actor_read_source_capability_ast_audit_rejects_opaque_macro_nested_in_inert_allowlist();
        actor_read_source_capability_ast_audit_rejects_inert_macro_import_rename_or_glob_shadow();
        review_audit_rejects_opaque_attribute_macro_invocation();
        actor_read_source_capability_ast_audit_rejects_custom_derive_macro();
        actor_read_source_capability_ast_audit_rejects_attribute_macro_evasion_shapes();
        actor_read_source_capability_procedural_macros_can_reopen_sibling_api();
        actor_read_source_capability_ast_audit_rejects_type_alias_impl();
        actor_read_source_capability_ast_audit_rejects_defaulted_generic_alias_impl();
        actor_read_source_capability_ast_audit_rejects_sibling_visible_free_factory();
        actor_read_source_capability_ast_audit_rejects_nested_production_factory();
        actor_read_source_capability_ast_audit_rejects_foreign_impl_signature_escape();
        actor_read_source_capability_ast_audit_rejects_associated_type_escape();
        actor_read_source_capability_ast_audit_rejects_item_field_escape();
        actor_read_source_capability_ast_audit_rejects_hardcoded_platform_profile();
        actor_read_source_capability_ast_audit_rejects_hardcoded_source_kind();
        actor_read_source_capability_ast_audit_rejects_replenished_deadline();
        actor_read_source_capability_ast_audit_rejects_side_effecting_profile_error_closure();
        actor_read_source_capability_ast_audit_rejects_substituted_construction_fields();
        actor_read_source_capability_ast_audit_skips_only_exact_cfg_test_scope();
    }

    #[test]
    fn actor_read_source_capability_sibling_field_privacy_is_enforced_by_rustc() {
        use quote::ToTokens;

        let source = include_str!("server.rs");
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
            .to_token_stream()
            .to_string();
        let fixture = format!(
            r#"
mod daemon {{
    mod owner {{
        struct ProviderRootBinding;
        struct WorkspaceLogicalReadFence;
        struct ProviderDeadline;
        {declaration}
    }}
    mod sibling {{
        fn inspect(capability: &super::owner::ActorReadSourceCapability) {{
            let _ = &capability.binding;
        }}
    }}
}}
fn main() {{}}
"#
        );
        let directory = tempfile::tempdir().unwrap();
        let fixture_path = directory.path().join("actor-capability-sibling-privacy.rs");
        std::fs::write(&fixture_path, fixture).unwrap();
        let output = std::process::Command::new("rustc")
            .arg("--edition=2021")
            .arg("--emit=metadata")
            .arg("--out-dir")
            .arg(directory.path())
            .arg(&fixture_path)
            .output()
            .expect("rustc is available to the Rust test suite");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success() && stderr.contains("E0616") && stderr.contains("binding"),
            "a sibling module accessed the audited private binding; status={:?}, stderr={stderr}",
            output.status.code()
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
    fn actor_read_source_capability_ast_audit_rejects_split_sibling_visibility() {
        let source = include_str!("server.rs");
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
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "split `pub (super)` visibility escaped the capability API audit"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_multiline_derive_clone() {
        let source = include_str!("server.rs");
        let multiline_derive = source.replacen(
            "#[allow(dead_code)]\npub(super) struct ActorReadSourceCapability {",
            "#[allow(dead_code)]\n#[derive(\n    Clone\n)]\npub(super) struct ActorReadSourceCapability {",
            1,
        );
        assert!(
            audit_actor_read_source_capability_api(&multiline_derive).is_err(),
            "multiline derive Clone escaped the capability API audit"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_multiline_manual_clone() {
        let source = include_str!("server.rs");
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
        assert!(
            audit_actor_read_source_capability_api(&manual_clone).is_err(),
            "multiline manual Clone escaped the capability API audit"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_multiline_extra_inherent_impl() {
        let source = include_str!("server.rs");
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
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "multiline extra inherent impl escaped the capability API audit"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_macro_generated_api() {
        let source = include_str!("server.rs");
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
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "macro-generated sibling API escaped the capability API audit"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_opaque_item_macro_invocation() {
        let source = include_str!("server.rs");
        let hostile = source.replacen(
            "struct ActorLogicalReadLease {",
            "reopen_sealed_capability!();\n\nstruct ActorLogicalReadLease {",
            1,
        );
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

        let source = include_str!("server.rs");
        let hostile = source.replacen(
            "struct ActorLogicalReadLease {",
            "const _: () = external_reopen_capability!(ActorReadSourceCapability);\n\nstruct ActorLogicalReadLease {",
            1,
        );
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

        let source = include_str!("server.rs");
        let hostile = source.replacen(
            "struct ActorLogicalReadLease {",
            "const _: () = external_reopen_capability!();\n\nstruct ActorLogicalReadLease {",
            1,
        );
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "a no-argument opaque const-expression macro generated a sibling capability forge"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_opaque_static_and_statement_macros() {
        let source = include_str!("server.rs");
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
        let source = include_str!("server.rs");
        let hostile = source.replacen(
            "struct ActorLogicalReadLease {",
            r#"fn nested_opaque_macro() {
    let _ = format!("{}", external_reopen_capability!());
}

struct ActorLogicalReadLease {"#,
            1,
        );
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "an opaque macro hid inside the inert standard-macro allowlist"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_inert_macro_import_rename_or_glob_shadow() {
        let source = include_str!("server.rs");
        let hostile_imports = [
            "use hostile_macros::reopen as format;\n",
            "use hostile_macros::matches;\n",
            "use hostile_macros::*;\n",
        ];
        let mut accepted = Vec::new();
        for import in hostile_imports {
            let hostile = source.replacen(
                "use super::identity::{CoreIdentity, DaemonStateDirectory};",
                &format!("{import}use super::identity::{{CoreIdentity, DaemonStateDirectory}};"),
                1,
            );
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
        let source = include_str!("server.rs");
        let hostile = source.replacen(
            "struct ActorLogicalReadLease {",
            "#[external_reopen_capability]\nconst ATTRIBUTE_MACRO_TRIGGER: () = ();\n\nstruct ActorLogicalReadLease {",
            1,
        );
        assert_ne!(
            hostile, source,
            "hostile attribute fixture was not inserted"
        );
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "an opaque procedural attribute could generate a sibling capability API"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_custom_derive_macro() {
        let source = include_str!("server.rs");
        let hostile = source.replacen(
            "struct ActorLogicalReadLease {",
            "#[derive(ExternalReopenCapability)]\nstruct DERIVE_MACRO_TRIGGER;\n\nstruct ActorLogicalReadLease {",
            1,
        );
        assert_ne!(hostile, source, "hostile derive fixture was not inserted");
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "a custom derive could generate a sibling capability API"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_attribute_macro_evasion_shapes() {
        let source = include_str!("server.rs");
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
            if audit_actor_read_source_capability_api(&hostile).is_ok() {
                accepted.push(item);
            }
        }
        for import in hostile_imports {
            let hostile = source.replacen(
                "use super::identity::{CoreIdentity, DaemonStateDirectory};",
                &format!("{import}use super::identity::{{CoreIdentity, DaemonStateDirectory}};"),
                1,
            );
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
        let source = include_str!("server.rs");
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

        let source = include_str!("server.rs");
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

        let source = include_str!("server.rs");
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
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "a sibling-visible owner-module free factory escaped the capability audit"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_nested_production_factory() {
        let source = include_str!("server.rs");
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
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "a nested production module exported a capability factory"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_foreign_impl_signature_escape() {
        let source = include_str!("server.rs");
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
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "a foreign impl accepted and returned the capability"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_associated_type_escape() {
        let source = include_str!("server.rs");
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
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "an associated type exported the capability outside the closed surface"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_item_field_escape() {
        let source = include_str!("server.rs");
        let hostile = source.replacen(
            "struct ActorLogicalReadLease {",
            r#"pub(super) struct CapabilityEscape {
    pub(super) capability: ActorReadSourceCapability,
}

struct ActorLogicalReadLease {"#,
            1,
        );
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "a sibling-visible item field exported the capability"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_side_effecting_profile_error_closure() {
        let source = include_str!("server.rs");
        let hostile = source.replacen(
            r#"|| {
            "actor-bound logical source has no supported platform profile".to_string()
        }"#,
            r#"|| {
            PROFILE_ERROR_SIDE_EFFECT.store(true, Ordering::SeqCst);
            "actor-bound logical source has no supported platform profile".to_string()
        }"#,
            1,
        );
        assert!(
            audit_actor_read_source_capability_api(&hostile).is_err(),
            "a side-effecting unsupported-profile closure escaped the exact builder audit"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_substituted_construction_fields() {
        let source = include_str!("server.rs");
        let hostile_fields = [
            (
                "Ok(ActorReadSourceCapability {\n                    binding: source.binding.clone(),",
                "Ok(ActorReadSourceCapability {\n                    binding: self.invocation.provider_root.clone(),",
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
                "fence: source.fence.clone(),",
                "fence: lease.sources[0].fence.clone(),",
            ),
            (
                "deadline: lease.deadline,",
                "deadline: ProviderDeadline::from_budget(LOGICAL_READ_OPERATION_BUDGET),",
            ),
        ];
        let mut accepted = Vec::new();
        for (original, substituted) in hostile_fields {
            let hostile = source.replacen(original, substituted, 1);
            assert_ne!(
                hostile, source,
                "hostile construction fixture did not replace `{original}`"
            );
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
    fn actor_read_source_capability_ast_audit_skips_only_exact_cfg_test_scope() {
        let source = include_str!("server.rs");
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
        audit_actor_read_source_capability_api(&exact_test_only)
            .expect("the production audit deliberately excludes exact cfg(test) modules");

        let partly_production = exact_test_only.replacen(
            "#[cfg(test)]\nmod test_only_escape",
            "#[cfg(any(test, feature = \"hostile-production\"))]\nmod test_only_escape",
            1,
        );
        assert!(
            audit_actor_read_source_capability_api(&partly_production).is_err(),
            "a module that can compile outside cfg(test) escaped the production audit"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_hardcoded_platform_profile() {
        let source = include_str!("server.rs");
        let hardcoded_profile = source.replacen(
            "source_profile.platform_profile()",
            "SourceProfile::platform_xml_8_3_27_format_2_20().platform_profile()",
            1,
        );
        assert!(
            audit_actor_read_source_capability_api(&hardcoded_profile).is_err(),
            "hardcoded platform profile escaped the authority-construction dataflow audit"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_hardcoded_source_kind() {
        let source = include_str!("server.rs");
        let hardcoded_kind = source.replacen(
            "self.binding.source_kind(),\n                self.binding.retained_root(),",
            "SourceSetKind::Configuration,\n                self.binding.retained_root(),",
            1,
        );
        assert!(
            audit_actor_read_source_capability_api(&hardcoded_kind).is_err(),
            "hardcoded source kind escaped the authority-construction dataflow audit"
        );
    }

    #[test]
    fn actor_read_source_capability_ast_audit_rejects_replenished_deadline() {
        let source = include_str!("server.rs");
        let replenished_deadline = source.replacen(
            "platform_profile,\n                self.deadline,",
            "platform_profile,\n                ProviderDeadline::from_budget(LOGICAL_READ_OPERATION_BUDGET),",
            1,
        );
        assert!(
            audit_actor_read_source_capability_api(&replenished_deadline).is_err(),
            "replenished deadline escaped the authority-construction dataflow audit"
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
        let actor = WorkspaceActorRegistry::default()
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
        let cancellation = CancellationToken::new();
        let fence = actor
            .capture_logical_read_revision(&binding, deadline, &cancellation)
            .unwrap();
        let identity = format!(
            "{}:{}",
            actor.safe_identity_hash().unwrap().as_str(),
            binding.source_set_name()
        );
        let capability = ActorReadSourceCapability {
            binding,
            identity,
            fence,
            deadline,
        };
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
            Duration::from_secs(19),
            "the authority builder replenished the captured operation deadline"
        );
    }

    #[test]
    fn actor_read_authority_builder_uses_only_actor_bound_semantics() {
        actor_read_authority_builder_rejects_actor_bound_unsupported_profile();
        actor_read_authority_builder_preserves_actor_bound_source_kind();
        actor_read_authority_builder_preserves_non_replenishing_deadline();
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

        let capability_aggregate_declaration = [
            "pub(crate) fn actor_authenticated_source_capability_contract_is_complete",
            "()",
        ]
        .concat();
        let (_, after_capability_aggregate) = source
            .split_once(&capability_aggregate_declaration)
            .expect("capability aggregate remains available to the architecture witness");
        let (capability_aggregate, _) = after_capability_aggregate
            .split_once("\n    }\n")
            .expect("capability aggregate remains structurally bounded");
        assert!(
            capability_aggregate
                .contains("actor_read_source_capability_is_sealed_after_binding();"),
            "capability aggregate omits the complete AST and sibling-privacy witness"
        );
        assert!(
            capability_aggregate
                .contains("actor_read_authority_builder_uses_only_actor_bound_semantics();"),
            "capability aggregate omits bound profile/kind/deadline behavior"
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

    #[test]
    pub(crate) fn view_find_admitted_snapshot_may_finish_after_map_change() {
        let task_root = tempfile::tempdir().unwrap();
        let (workspace, source) = source_selection_read_fixture();
        let (store, _) =
            FileInvocationStore::open(task_root.path(), Arc::new(SystemEpochMillisClock)).unwrap();
        let runtime = DaemonInvocationRuntime::new(
            Arc::new(store),
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
                runtime.capture_response_deadline(),
            )
            .unwrap()
        };
        let view_request = request(
            ToolIdentity::View,
            serde_json::json!({"at": "main:Catalog.Items"}),
        );
        let find_request = request(ToolIdentity::Find, serde_json::json!({"query": "Items"}));
        let view = bind(&view_request);
        let find = bind(&find_request);
        assert!(Arc::ptr_eq(&view.actor, &find.actor));
        let admitted_actor = Arc::clone(&view.actor);
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
            !Arc::ptr_eq(&admitted_actor, &subsequent.actor),
            "a subsequent invocation ignored the changed map"
        );
        assert_eq!(
            subsequent.provider_root.source_kind(),
            SourceSetKind::Extension
        );
        assert!(source.join("Catalogs/Items.xml").exists());
    }

    #[test]
    pub(crate) fn semantically_equivalent_map_edit_reuses_actor_identity() {
        let task_root = tempfile::tempdir().unwrap();
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

        assert!(Arc::ptr_eq(&first.actor, &second.actor));
        assert_eq!(
            first.workspace_identity_hash, second.workspace_identity_hash,
            "comments/order-only edit changed actor state scope"
        );
    }

    #[test]
    fn unpublished_apply_execution_rejects_success_changed_and_revision_without_actor_publication()
    {
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
            ToolIdentity::Apply,
            serde_json::json!({
                "at": "main:Configuration",
                "ops": [{"op": "object.create", "args": {}}],
                "dryRun": false,
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
                runtime.capture_response_deadline(),
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
    pub(crate) fn working_task_recovery_is_resume_unsupported_without_apply_reexecution() {
        let task_root = tempfile::tempdir().unwrap();
        let (store, _) =
            FileInvocationStore::open(task_root.path(), Arc::new(SystemEpochMillisClock)).unwrap();
        let record = NewInvocationRecord::new(
            crate::domain::invocation::InvocationId::new(),
            ToolIdentity::Apply,
            crate::domain::invocation::NormalizedArgumentsHash::from_sha256([0x91; 32]),
            crate::domain::invocation::SafeIdentityHash::from_sha256([0x92; 32]),
            SafeStatusMessage::Working,
            250,
            60_000,
            Some(crate::domain::invocation::ResumeDescriptor::Delivery(
                crate::domain::invocation::DeliveryResume::new(
                    crate::domain::invocation::SafeIdentityHash::from_sha256([0x93; 32]),
                ),
            )),
        );
        let working = store.create_working(record).unwrap();
        drop(store);
        let apply_executions = AtomicUsize::new(0);

        let (reopened, report) =
            FileInvocationStore::open(task_root.path(), Arc::new(SystemEpochMillisClock)).unwrap();
        let recovered = reopened.get(working.task_id).unwrap();

        assert_eq!(recovered.status, InvocationStatus::Failed);
        assert_eq!(
            recovered.failure_reason,
            Some(SafeFailureReason::ResumeUnsupported)
        );
        assert!(recovered.result.is_none());
        assert_eq!(apply_executions.load(Ordering::SeqCst), 0);
        assert!(report.classifications.iter().any(|classification| matches!(
            classification,
            crate::infrastructure::task_store::RecoveryClassification::UnsupportedResume { task_id }
                if *task_id == working.task_id
        )));
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
        actor_read_authority_builder_uses_only_actor_bound_semantics();
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
        assert!(result.ok, "hidden V13 execution must succeed");
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
        assert!(find_result.ok, "hidden V13 find execution must succeed");
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

    struct RevisionChangingIndexService {
        producers: Arc<AtomicUsize>,
        producer_entered: mpsc::Sender<()>,
        joined: mpsc::Sender<IndexWorkIdentity>,
        release: Arc<(Mutex<bool>, Condvar)>,
        first_execution: AtomicBool,
        mark_dirty: Mutex<mpsc::Receiver<()>>,
        dirty_done: mpsc::Sender<()>,
    }

    struct UnavailableCancelStore {
        inner: FileInvocationStore,
    }

    impl InvocationStore for UnavailableCancelStore {
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
            _task_id: crate::domain::invocation::TaskId,
            _status_message: SafeStatusMessage,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            Err(InvocationStoreError::ActorUnavailable)
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
        std::fs::write(
            workspace.join("Configuration.xml"),
            "<MetaDataObject><Configuration/></MetaDataObject>",
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
            ready_root: workspace_parent.path().join("shared-daemon-cache"),
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
        crate::infrastructure::runtime_jobs::reset_runtime_resource_contract_executions_for_test();

        daemon_long_work_capabilities_handoff_before_wait_and_preserve_exact_ownership();
        daemon_index_work_separates_worktrees_and_rejects_stale_revision_publication();
        daemon_long_work_rejects_replaced_actor_root_before_reuse_or_publication();
        crate::infrastructure::runtime_jobs::run_runtime_resource_tree_contract_for_test();

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
        // Trigger fail-stop at the cancel boundary itself. A shortened reconciliation budget
        // also bounds the preceding create_working store call and makes this a scheduler-speed
        // test instead of a process-owned resource-lifetime test.
        let runtime = DaemonInvocationRuntime::new(
            Arc::new(UnavailableCancelStore { inner: store }),
            Arc::new(NonCooperativeActorService {
                entered,
                release: Mutex::new(release_wait),
            }),
            Arc::new(TokioClock),
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

fn reject_overloaded_connection(connection: AcceptedConnection) {
    let mut stream = connection.stream;
    let _ = write_response_before(
        &mut stream,
        &ServerResponse::error(DaemonErrorCode::Overloaded),
        connection
            .handshake_deadline
            .min(Instant::now() + Duration::from_millis(100)),
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
pub(crate) fn install_startup_pause() -> HandshakePauseGuard {
    install_handshake_pause()
}

#[cfg(test)]
fn pause_test_thread_if_configured(pause: Option<Arc<HandshakePause>>) {
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
