use super::super::protocol::InvocationRequest;
use crate::application::invocation::InvocationResponseDeadline;
use crate::application::operation_descriptors::ExecutionClass;
use crate::application::shared_work::{
    LongWorkFailure, ProviderHostKey, ProviderHostOwner, SharedWorkLease, SharedWorkProducer,
};
use crate::application::v13::LOGICAL_READ_OPERATION_BUDGET;
use crate::domain::address::QualifiedAddress;
use crate::domain::apply::ApplyRequest;
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::invocation::{DomainResult, InvocationFailure, SafeIdentityHash};
use crate::domain::project_sources::{SourceFormat, SourceProfile, SourceSetKind};
use crate::infrastructure::runtime_jobs::{RuntimeJobService, RuntimeResourceOwner};
use crate::infrastructure::source_selection_evidence::discover_project_source_admission;
use crate::infrastructure::workspace::discover_workspace;
use crate::infrastructure::workspace_actor::{
    ApplyAdmission, IndexWorkIdentity, ProviderRootBinding, WorkspaceActor, WorkspaceActorRegistry,
    WorkspaceActorRegistryError, WorkspaceLogicalReadFence, WorkspaceRevisionFence,
    WorkspaceSourceSetInput,
};
use std::sync::Arc;
use std::time::Duration;

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
pub(in crate::infrastructure::daemon) struct ActorReadSourceCapability {
    binding: ProviderRootBinding,
    identity: String,
    fence: WorkspaceLogicalReadFence,
    deadline: ProviderDeadline,
}

#[allow(dead_code)]
impl ActorReadSourceCapability {
    pub(in crate::infrastructure::daemon) fn source_set_name(&self) -> &str {
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

    pub(in crate::infrastructure::daemon) const fn deadline(&self) -> ProviderDeadline {
        self.deadline
    }

    pub(in crate::infrastructure::daemon) fn logical_view_read_authority<'a>(
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

#[cfg(test)]
pub(super) fn actor_read_source_capability_for_test(
    binding: ProviderRootBinding,
    identity: String,
    fence: WorkspaceLogicalReadFence,
    deadline: ProviderDeadline,
) -> ActorReadSourceCapability {
    ActorReadSourceCapability {
        binding,
        identity,
        fence,
        deadline,
    }
}

#[cfg(test)]
pub(super) fn actor_read_source_metadata_for_test(
    capability: &ActorReadSourceCapability,
) -> (SourceSetKind, SourceFormat, SourceProfile) {
    (
        capability.source_kind(),
        capability.source_format(),
        capability.source_profile(),
    )
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

    pub(super) fn response_deadline(&self) -> &InvocationResponseDeadline {
        &self.response_deadline
    }

    #[cfg(test)]
    pub(super) fn actor_for_test(&self) -> &Arc<WorkspaceActor> {
        &self.actor
    }

    #[cfg(test)]
    pub(super) fn provider_root_for_test(&self) -> &ProviderRootBinding {
        &self.provider_root
    }

    #[cfg(test)]
    pub(super) fn read_source_binding_for_test(
        &self,
        source_set_name: &str,
    ) -> Option<&ProviderRootBinding> {
        self.read_sources
            .iter()
            .find(|source| source.binding.source_set_name() == source_set_name)
            .map(|source| &source.binding)
    }

    #[cfg(test)]
    pub(super) fn begin_execution_with_logical_deadline_for_test(
        self,
        cancellation: &CancellationToken,
        logical_deadline: ProviderDeadline,
    ) -> Result<ActorBoundExecution, String> {
        self.begin_execution_with_logical_deadline(cancellation, logical_deadline)
    }

    pub(super) fn begin_execution(
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

    pub(in crate::infrastructure::daemon) fn admitted_source_set_names(&self) -> Vec<&str> {
        self.invocation
            .read_sources
            .iter()
            .map(|source| source.binding.source_set_name())
            .collect()
    }

    pub(in crate::infrastructure::daemon) fn admit_apply(
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
    pub(in crate::infrastructure::daemon) fn read_sources(
        &self,
    ) -> Result<Vec<ActorReadSourceCapability>, String> {
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
    pub(super) fn actor_for_test(&self) -> &Arc<WorkspaceActor> {
        self.invocation.actor_for_test()
    }

    #[cfg(test)]
    pub(super) fn provider_root_for_test(&self) -> &ProviderRootBinding {
        self.invocation.provider_root_for_test()
    }

    #[cfg(test)]
    pub(super) fn mark_source_revision_dirty_for_test(&self) {
        self.invocation.actor.mark_source_revisions_dirty();
    }

    pub(super) fn publish(
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
pub(super) fn bind_workspace_invocation(
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

#[cfg(test)]
pub(super) struct ActorInvocationResourcesForTest(ActorInvocationResources);

#[cfg(test)]
impl ActorInvocationResourcesForTest {
    pub(super) fn new(
        deliveries: Arc<crate::infrastructure::engine_delivery::DeliveryDesk>,
        provider_hosts: Arc<ProviderHostOwner>,
        runtime_resources: Arc<RuntimeResourceOwner>,
        runtime_service: Option<Arc<RuntimeJobService>>,
    ) -> Self {
        Self(ActorInvocationResources {
            deliveries,
            provider_hosts,
            runtime_resources,
            runtime_service,
        })
    }
}

#[cfg(test)]
pub(super) fn bind_workspace_invocation_with_source_override_for_test(
    request: &InvocationRequest,
    actors: &WorkspaceActorRegistry,
    resources: ActorInvocationResourcesForTest,
    response_deadline: InvocationResponseDeadline,
    source_kind: SourceSetKind,
    source_format: SourceFormat,
    source_profile: SourceProfile,
) -> Result<ActorBoundInvocation, WorkspaceAdmissionError> {
    bind_workspace_invocation_controlled(
        request,
        actors,
        resources.0,
        response_deadline,
        |sources| {
            sources[0].kind = source_kind;
            sources[0].source_format = source_format;
            sources[0].source_profile = source_profile;
        },
    )
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
pub(super) enum WorkspaceAdmissionError {
    Capacity,
    RegistryFailed,
    Invalid,
}
