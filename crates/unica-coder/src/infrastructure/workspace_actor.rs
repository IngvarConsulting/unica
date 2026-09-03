use crate::application::shared_work::{
    LongWorkFailure, SharedWork, SharedWorkKey, SharedWorkLease, SharedWorkLifetime,
    SharedWorkProducer,
};
use crate::domain::cache::CacheAccess;
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::events::DomainEvent;
use crate::domain::invocation::SafeIdentityHash;
use crate::domain::project_sources::{SourceFormat, SourceProfile, SourceSetKind};
use crate::domain::source_revision::SourceRevision;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::deadline_lock::{
    DeadlineLock, DeadlineLockError, DeadlineLockErrorKind, FailClosed,
};
use crate::infrastructure::native_operations::apply::{
    ApplyPlanError, ApplyPlanErrorKind, ApplyStagedState, ApplyStagingError, ApplyStagingErrorKind,
};
use crate::infrastructure::native_operations::compile_transaction::{
    CompileTransaction, RetainedApplyRevisionTransients,
};
use crate::infrastructure::native_operations::event::PlannedApplyEffects;
use crate::infrastructure::platform::filesystem::{
    path_starts_with_host_root, stable_path_identity_bytes, RetainedChildCapability,
    RetainedDirectoryCapability,
};
use crate::infrastructure::source_revision::{
    PreparedRevisionReconciliation, RetainedRevisionError, RetainedRevisionErrorKind,
    RetainedRevisionLease, SourceRevisionService, WorkspaceStateScope,
};
use crate::infrastructure::source_roots::{normalize_path_identity, GENERATED_DIR_NAME};
use crate::infrastructure::source_selection_evidence::{
    discover_project_source_admission, ResolvedProjectSourceAdmission,
    RetainedSourceSelectionEvidence, SourceSelectionEvidenceError,
    SourceSelectionEvidenceErrorKind,
};
use crate::infrastructure::support_policy_evidence::{
    RetainedSupportPolicyEvidence, SupportPolicyEvidenceError, SupportPolicyEvidenceErrorKind,
    SupportPolicyMode,
};
use crate::infrastructure::workspace_index::{IndexRunner, WorkspaceIndexService};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::{OsStr, OsString};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

#[cfg(test)]
thread_local! {
    static LOGICAL_PUBLICATION_AFTER_CONFIRMATION_HOOK: std::cell::RefCell<
        Option<Box<dyn FnOnce()>>,
    > = std::cell::RefCell::new(None);
    static APPLY_DRY_RUN_AFTER_CONFIRMATION_HOOK: std::cell::RefCell<
        Option<Box<dyn FnOnce()>>,
    > = std::cell::RefCell::new(None);
    static REVISION_SERVICE_AFTER_BINDING_VALIDATION_HOOK: std::cell::RefCell<
        Option<Box<dyn FnOnce()>>,
    > = std::cell::RefCell::new(None);
}

#[cfg(test)]
pub(crate) fn set_logical_publication_after_confirmation_hook(hook: impl FnOnce() + 'static) {
    LOGICAL_PUBLICATION_AFTER_CONFIRMATION_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

#[cfg(test)]
fn run_logical_publication_after_confirmation_hook() {
    LOGICAL_PUBLICATION_AFTER_CONFIRMATION_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(test)]
fn set_apply_dry_run_after_confirmation_hook(hook: impl FnOnce() + 'static) {
    APPLY_DRY_RUN_AFTER_CONFIRMATION_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

#[cfg(test)]
fn run_apply_dry_run_after_confirmation_hook() {
    APPLY_DRY_RUN_AFTER_CONFIRMATION_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(test)]
fn set_revision_service_after_binding_validation_hook(hook: impl FnOnce() + 'static) {
    REVISION_SERVICE_AFTER_BINDING_VALIDATION_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

#[cfg(test)]
fn run_revision_service_after_binding_validation_hook() {
    REVISION_SERVICE_AFTER_BINDING_VALIDATION_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

pub(crate) const MAX_ACTIVE_WORKSPACE_ACTORS: usize = 64;
/// Recently used actors the daemon keeps alive between invocations so their
/// trusted source revision and platform fence survive to the next call.
pub(crate) const WARM_WORKSPACE_ACTORS: usize = 8;
/// Idle time after which a warm actor is released together with its retained
/// root descriptors and fence.
pub(crate) const WARM_WORKSPACE_ACTOR_TTL: Duration = Duration::from_secs(600);
const INDEX_FENCE_BUDGET: Duration = Duration::from_secs(7);
static APPLY_WRITER_AUTHORITY_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct IndexWorkIdentity([u8; 32]);

macro_rules! assert_not_impl_production {
    ($type:ty: $trait:path) => {
        const _: fn() = || {
            trait AmbiguousIfImpl<Marker> {
                fn check() {}
            }
            struct ImplementsTrait;
            impl<T: ?Sized> AmbiguousIfImpl<()> for T {}
            impl<T: ?Sized + $trait> AmbiguousIfImpl<ImplementsTrait> for T {}
            let _ = <$type as AmbiguousIfImpl<_>>::check;
        };
    };
}

assert_not_impl_production!(ResolvedProjectSourceAdmission: Clone);
assert_not_impl_production!(ResolvedProjectSourceAdmission: serde::Serialize);
assert_not_impl_production!(ResolvedProjectSourceAdmission: serde::de::DeserializeOwned);
assert_not_impl_production!(RetainedSourceSelectionEvidence: Clone);
assert_not_impl_production!(RetainedSourceSelectionEvidence: serde::Serialize);
assert_not_impl_production!(RetainedSourceSelectionEvidence: serde::de::DeserializeOwned);
assert_not_impl_production!(CodeApplyAuthority<'static>: Clone);
assert_not_impl_production!(CodeApplyAuthority<'static>: serde::Serialize);
assert_not_impl_production!(CodeApplyAuthority<'static>: serde::de::DeserializeOwned);
assert_not_impl_production!(XdtoApplyAuthority<'static>: Clone);
assert_not_impl_production!(XdtoApplyAuthority<'static>: serde::Serialize);
assert_not_impl_production!(XdtoApplyAuthority<'static>: serde::de::DeserializeOwned);
assert_not_impl_production!(MetadataApplyAuthority<'static>: Clone);
assert_not_impl_production!(MetadataApplyAuthority<'static>: serde::Serialize);
assert_not_impl_production!(MetadataApplyAuthority<'static>: serde::de::DeserializeOwned);
assert_not_impl_production!(FormResourceApplyAuthority<'static>: Clone);
assert_not_impl_production!(FormResourceApplyAuthority<'static>: serde::Serialize);
assert_not_impl_production!(FormResourceApplyAuthority<'static>: serde::de::DeserializeOwned);
assert_not_impl_production!(DcsMxlApplyAuthority<'static>: Clone);
assert_not_impl_production!(DcsMxlApplyAuthority<'static>: serde::Serialize);
assert_not_impl_production!(DcsMxlApplyAuthority<'static>: serde::de::DeserializeOwned);
assert_not_impl_production!(ActorRevisionServiceAuthority: Clone);
assert_not_impl_production!(ActorRevisionServiceAuthority: serde::Serialize);
assert_not_impl_production!(ActorRevisionServiceAuthority: serde::de::DeserializeOwned);
assert_not_impl_production!(RetainedApplyRevisionTransients<'static>: Clone);
assert_not_impl_production!(RetainedApplyRevisionTransients<'static>: serde::Serialize);
assert_not_impl_production!(RetainedApplyRevisionTransients<'static>: serde::de::DeserializeOwned);
assert_not_impl_production!(PlannedApplyEffects: Clone);
assert_not_impl_production!(PlannedApplyEffects: serde::Serialize);
assert_not_impl_production!(PlannedApplyEffects: serde::de::DeserializeOwned);

/// Exact daemon-local ownership key for mutable workspace state.
///
/// A Git repository is intentionally absent. Linked worktrees have separate
/// canonical roots, revisions and publication lanes even when `.git` points at
/// the same repository metadata.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct WorkspaceIdentity {
    workspace_root: PathBuf,
    source_sets: Vec<WorkspaceSourceSetIdentity>,
    provider_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct WorkspaceSourceSetIdentity {
    name: String,
    root: PathBuf,
    kind: SourceSetKind,
    source_format: SourceFormat,
    source_profile: SourceProfile,
}

/// Complete typed source-set input. The registry canonicalizes and retains its
/// root before it becomes actor authority.
#[derive(Debug, Clone)]
pub(crate) struct WorkspaceSourceSetInput {
    name: String,
    root: PathBuf,
    kind: SourceSetKind,
    source_format: SourceFormat,
    source_profile: SourceProfile,
}

impl WorkspaceSourceSetInput {
    pub(crate) fn new(
        name: impl Into<String>,
        root: impl Into<PathBuf>,
        kind: SourceSetKind,
        source_format: SourceFormat,
        source_profile: SourceProfile,
    ) -> Self {
        Self {
            name: name.into(),
            root: root.into(),
            kind,
            source_format,
            source_profile,
        }
    }
}

impl WorkspaceIdentity {
    pub(crate) fn new<I>(
        context: &WorkspaceContext,
        source_sets: I,
        provider_profile: &str,
    ) -> Result<Self, String>
    where
        I: IntoIterator<Item = WorkspaceSourceSetInput>,
    {
        if provider_profile.trim().is_empty() || provider_profile.chars().any(char::is_control) {
            return Err("workspace provider profile must be non-empty text".to_string());
        }
        let workspace_root = normalize_path_identity(&context.workspace_root)?;
        let mut source_sets = source_sets
            .into_iter()
            .map(|source_set| {
                let name = source_set.name;
                if name.trim().is_empty() || name.chars().any(char::is_control) {
                    return Err("workspace source-set name must be non-empty text".to_string());
                }
                Ok(WorkspaceSourceSetIdentity {
                    name,
                    root: normalize_path_identity(&source_set.root)?,
                    kind: source_set.kind,
                    source_format: source_set.source_format,
                    source_profile: source_set.source_profile,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if source_sets.is_empty() {
            return Err("workspace actor requires at least one source-set root".to_string());
        }
        if let Some(outside) = source_sets
            .iter()
            .find(|source_set| !path_starts_with_host_root(&source_set.root, &workspace_root))
        {
            return Err(format!(
                "workspace actor source-set root is outside its canonical workspace root: {}",
                outside.root.display()
            ));
        }
        source_sets.sort();
        if source_sets
            .windows(2)
            .any(|pair| pair[0].name == pair[1].name)
        {
            return Err("workspace actor source-set names must be unique".to_string());
        }
        let mut physical_roots = HashSet::new();
        if source_sets
            .iter()
            .any(|source_set| !physical_roots.insert(source_set.root.clone()))
        {
            return Err(
                "workspace actor source-set roots must be physically unambiguous".to_string(),
            );
        }
        Ok(Self {
            workspace_root,
            source_sets,
            provider_profile: provider_profile.to_string(),
        })
    }

    pub(crate) fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub(crate) fn source_sets(&self) -> &[WorkspaceSourceSetIdentity] {
        &self.source_sets
    }

    pub(crate) fn provider_profile(&self) -> &str {
        &self.provider_profile
    }

    fn state_scope_hash(&self) -> Result<[u8; 32], String> {
        let mut digest = Sha256::new();
        digest.update(b"unica-workspace-actor-state-v2\0");
        update_digest_path(&mut digest, &self.workspace_root)?;
        digest.update((self.source_sets.len() as u64).to_le_bytes());
        for source_set in &self.source_sets {
            digest.update(b"source-set-name\0");
            update_digest_text(&mut digest, &source_set.name);
            digest.update(b"source-set-root\0");
            update_digest_path(&mut digest, &source_set.root)?;
            digest.update(b"source-set-kind\0");
            digest.update([source_set.kind.stable_discriminant()]);
            digest.update(b"source-format\0");
            digest.update([source_set.source_format.stable_discriminant()]);
            digest.update(b"source-profile\0");
            digest.update(source_set.source_profile.stable_discriminants());
        }
        digest.update(b"provider-profile\0");
        update_digest_text(&mut digest, &self.provider_profile);
        Ok(digest.finalize().into())
    }

    fn state_scope_digest(&self) -> Result<String, String> {
        use std::fmt::Write as _;

        let mut encoded = String::with_capacity(64);
        for byte in self.state_scope_hash()? {
            write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
        }
        Ok(encoded)
    }
}

fn update_digest_text(digest: &mut Sha256, value: &str) {
    digest.update((value.len() as u64).to_le_bytes());
    digest.update(value.as_bytes());
}

fn update_digest_path(digest: &mut Sha256, value: &Path) -> Result<(), String> {
    let bytes = stable_path_identity_bytes(value)?;
    digest.update((bytes.len() as u64).to_le_bytes());
    digest.update(bytes);
    Ok(())
}

#[derive(Clone, PartialEq, Eq)]
struct ActorInstanceId(uuid::Uuid);

impl ActorInstanceId {
    fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

#[derive(Clone)]
pub(crate) struct ProviderRootBinding {
    actor_identity: WorkspaceIdentity,
    actor_instance: ActorInstanceId,
    source_set: WorkspaceSourceSetIdentity,
    source_root: Arc<RetainedDirectoryCapability>,
}

impl std::fmt::Debug for ProviderRootBinding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderRootBinding")
            .field("actor_identity", &self.actor_identity)
            .field("source_set", &self.source_set)
            .field("source_root", &self.source_root.path())
            .finish_non_exhaustive()
    }
}

impl ProviderRootBinding {
    pub(crate) fn source_set_name(&self) -> &str {
        &self.source_set.name
    }

    pub(crate) const fn source_kind(&self) -> SourceSetKind {
        self.source_set.kind
    }

    pub(crate) const fn source_format(&self) -> SourceFormat {
        self.source_set.source_format
    }

    pub(crate) const fn source_profile(&self) -> SourceProfile {
        self.source_set.source_profile
    }

    pub(super) fn source_root(&self) -> &Path {
        self.source_root.path()
    }

    pub(super) fn retained_root(&self) -> Arc<RetainedDirectoryCapability> {
        Arc::clone(&self.source_root)
    }
}

/// One actor-issued construction authority for one scoped revision service.
/// Its private fields bind the retained root, actor state namespace and source
/// profile together; infrastructure consumers may use the proof but cannot
/// assemble or replay a mismatched tuple.
pub(in crate::infrastructure) struct ActorRevisionServiceAuthority {
    source_root: Arc<RetainedDirectoryCapability>,
    state_scope: WorkspaceStateScope,
    source_kind: SourceSetKind,
    source_format: SourceFormat,
    source_profile: SourceProfile,
}

impl ActorRevisionServiceAuthority {
    pub(super) fn source_root(&self) -> &Path {
        self.source_root.path()
    }

    pub(super) fn state_scope(&self) -> &WorkspaceStateScope {
        &self.state_scope
    }

    pub(super) const fn source_kind(&self) -> SourceSetKind {
        self.source_kind
    }

    pub(super) const fn source_format(&self) -> SourceFormat {
        self.source_format
    }

    pub(super) const fn source_profile(&self) -> SourceProfile {
        self.source_profile
    }

    pub(super) fn into_parts(
        self,
    ) -> (
        Arc<RetainedDirectoryCapability>,
        WorkspaceStateScope,
        SourceSetKind,
        SourceFormat,
        SourceProfile,
    ) {
        (
            self.source_root,
            self.state_scope,
            self.source_kind,
            self.source_format,
            self.source_profile,
        )
    }
}

/// Unforgeable admission token for the retained apply publisher. Low-level
/// transaction methods require the same token that created the staged state;
/// infrastructure callers can name the type but cannot construct a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::infrastructure) struct ApplyWriterAuthority(u64);

impl ApplyWriterAuthority {
    fn issue() -> Self {
        Self(APPLY_WRITER_AUTHORITY_SEQUENCE.fetch_add(1, Ordering::Relaxed))
    }
}

#[cfg(test)]
pub(in crate::infrastructure) fn apply_writer_authority_for_test() -> ApplyWriterAuthority {
    ApplyWriterAuthority::issue()
}

#[derive(Clone)]
pub(crate) struct WorkspaceRevisionFence {
    actor_identity: WorkspaceIdentity,
    actor_instance: ActorInstanceId,
    source_set: WorkspaceSourceSetIdentity,
    revision: SourceRevision,
}

/// Actor-issued retained revision authority for one V13 logical source set.
/// The caller can copy its revision identity into typed readers, but cannot
/// substitute a root, revision service or actor identity at publication.
#[derive(Clone)]
pub(crate) struct WorkspaceLogicalReadFence {
    actor_identity: WorkspaceIdentity,
    actor_instance: ActorInstanceId,
    source_set: WorkspaceSourceSetIdentity,
    revision: RetainedRevisionLease,
}

impl WorkspaceLogicalReadFence {
    pub(crate) fn revision(&self) -> RetainedRevisionLease {
        self.revision.clone()
    }
}

/// Why an apply admission was refused. A stale `ifRev` is a caller-visible
/// conflict with its own recovery strategy (re-read the revision and retry),
/// so it is distinguished from every infrastructure failure instead of
/// travelling as one more opaque string.
#[derive(Debug)]
pub(crate) enum ApplyAdmissionError {
    StaleRevision { expected: String, admitted: String },
    Other(String),
}

impl From<String> for ApplyAdmissionError {
    fn from(message: String) -> Self {
        Self::Other(message)
    }
}

impl From<ApplyAdmissionError> for String {
    fn from(error: ApplyAdmissionError) -> Self {
        error.to_string()
    }
}

impl std::fmt::Display for ApplyAdmissionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StaleRevision { expected, admitted } => write!(
                formatter,
                "apply ifRev is stale: expected {expected}, admitted {admitted}"
            ),
            Self::Other(message) => message.fmt(formatter),
        }
    }
}

impl std::error::Error for ApplyAdmissionError {}

/// Actor-issued authority for planning exactly one hidden-v0.13 apply batch.
/// All identity, root, revision, deadline and cancellation fields are closed;
/// callers can stage bytes but cannot substitute publication authority.
pub(crate) struct ApplyAdmission {
    actor_identity: WorkspaceIdentity,
    actor_instance: ActorInstanceId,
    source_set: WorkspaceSourceSetIdentity,
    source_root: Arc<RetainedDirectoryCapability>,
    revision_service: Arc<SourceRevisionService>,
    revision: RetainedRevisionLease,
    dry_run: bool,
    deadline: ProviderDeadline,
    cancellation: CancellationToken,
    writer_authority: ApplyWriterAuthority,
    workspace_cache: WorkspaceCachePublicationAuthority,
    support_policy: RetainedSupportPolicyEvidence,
    source_selection: RetainedSourceSelectionEvidence,
    context: WorkspaceContext,
}

/// Admission-sealed authority for dormant Code planning. It borrows the exact
/// actor binding and writer token admitted together with retained policy and
/// source-selection evidence; callers cannot construct or substitute fields.
pub(crate) struct CodeApplyAuthority<'a> {
    binding: &'a ProviderRootBinding,
    writer_authority: &'a ApplyWriterAuthority,
    profile: crate::domain::platform_profile::PlatformProfile,
    expected_format: &'static str,
    support_policy: SupportPolicyMode,
}

/// Admission-sealed authority for dormant XDTO planning. It binds the same
/// retained source, writer token, exact Platform XML profile and support
/// evidence as the staged state it is allowed to plan.
pub(crate) struct XdtoApplyAuthority<'a> {
    binding: &'a ProviderRootBinding,
    writer_authority: &'a ApplyWriterAuthority,
    expected_format: &'static str,
    support_policy: SupportPolicyMode,
}

struct PlatformXmlApplyAuthority<'a> {
    binding: &'a ProviderRootBinding,
    writer_authority: &'a ApplyWriterAuthority,
    profile: crate::domain::platform_profile::PlatformProfile,
    expected_format: &'static str,
    support_policy: SupportPolicyMode,
    context: &'a WorkspaceContext,
}

/// Admission-sealed authority for dormant metadata/property planning.
pub(crate) struct MetadataApplyAuthority<'a>(PlatformXmlApplyAuthority<'a>);

/// Admission-sealed authority for dormant form/resource planning.
pub(crate) struct FormResourceApplyAuthority<'a>(PlatformXmlApplyAuthority<'a>);

/// Admission-sealed authority for dormant DCS/MXL planning.
pub(crate) struct DcsMxlApplyAuthority<'a>(PlatformXmlApplyAuthority<'a>);

impl PlatformXmlApplyAuthority<'_> {
    fn owns_staged_state(&self, staged: &ApplyStagedState) -> bool {
        staged.retained_root_identity() == self.binding.source_root.identity()
            && staged.has_writer_authority(self.writer_authority)
    }
}

macro_rules! impl_platform_xml_apply_authority {
    ($authority:ident) => {
        impl $authority<'_> {
            pub(crate) fn source_set_name(&self) -> &str {
                self.0.binding.source_set_name()
            }

            pub(crate) const fn source_kind(&self) -> SourceSetKind {
                self.0.binding.source_kind()
            }

            pub(crate) const fn profile(&self) -> crate::domain::platform_profile::PlatformProfile {
                self.0.profile
            }

            pub(crate) const fn expected_format(&self) -> &str {
                self.0.expected_format
            }

            pub(crate) const fn support_policy_mode(&self) -> SupportPolicyMode {
                self.0.support_policy
            }

            pub(crate) fn owns_staged_state(&self, staged: &ApplyStagedState) -> bool {
                self.0.owns_staged_state(staged)
            }

            /// The admitted workspace, for planners that consult the source
            /// root beyond the staged files (templates, reference scans).
            pub(crate) fn workspace_context(&self) -> &WorkspaceContext {
                self.0.context
            }

            /// The physical root of the admitted source set.
            pub(crate) fn source_root(&self) -> &Path {
                self.0.binding.source_root()
            }
        }
    };
}

impl_platform_xml_apply_authority!(MetadataApplyAuthority);
impl_platform_xml_apply_authority!(FormResourceApplyAuthority);
impl_platform_xml_apply_authority!(DcsMxlApplyAuthority);

impl XdtoApplyAuthority<'_> {
    pub(crate) fn source_set_name(&self) -> &str {
        self.binding.source_set_name()
    }

    pub(crate) const fn source_kind(&self) -> SourceSetKind {
        self.binding.source_kind()
    }

    pub(crate) const fn expected_format(&self) -> &str {
        self.expected_format
    }

    pub(crate) const fn support_policy_mode(&self) -> SupportPolicyMode {
        self.support_policy
    }

    pub(crate) fn owns_staged_state(&self, staged: &ApplyStagedState) -> bool {
        staged.retained_root_identity() == self.binding.source_root.identity()
            && staged.has_writer_authority(self.writer_authority)
    }
}

impl CodeApplyAuthority<'_> {
    pub(crate) fn source_set_name(&self) -> &str {
        self.binding.source_set_name()
    }

    pub(crate) const fn source_kind(&self) -> SourceSetKind {
        self.binding.source_kind()
    }

    pub(crate) const fn profile(&self) -> crate::domain::platform_profile::PlatformProfile {
        self.profile
    }

    pub(crate) const fn expected_format(&self) -> &str {
        self.expected_format
    }

    pub(crate) const fn support_policy_mode(&self) -> SupportPolicyMode {
        self.support_policy
    }

    pub(crate) fn owns_staged_state(&self, staged: &ApplyStagedState) -> bool {
        staged.retained_root_identity() == self.binding.source_root.identity()
            && staged.has_writer_authority(self.writer_authority)
    }
}

#[derive(Clone)]
struct WorkspaceCachePublicationAuthority {
    logical_root: PathBuf,
    anchor: Arc<RetainedDirectoryCapability>,
    relative_root: PathBuf,
    participant: WorkspaceCacheParticipantAuthority,
}

/// Actor-issued proof that the second retained participant is the workspace
/// cache captured during admission, rather than an arbitrary transaction that
/// happens to share the writer authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::infrastructure) struct WorkspaceCacheParticipantAuthority {
    writer_authority: ApplyWriterAuthority,
    logical_root: PathBuf,
    anchor_identity: crate::infrastructure::platform::filesystem::FileIdentity,
}

impl WorkspaceCacheParticipantAuthority {
    pub(in crate::infrastructure) fn writer_authority(&self) -> &ApplyWriterAuthority {
        &self.writer_authority
    }

    pub(in crate::infrastructure) fn logical_root(&self) -> &Path {
        &self.logical_root
    }

    pub(in crate::infrastructure) fn anchor_identity(
        &self,
    ) -> crate::infrastructure::platform::filesystem::FileIdentity {
        self.anchor_identity
    }
}

#[cfg(test)]
pub(in crate::infrastructure) fn workspace_cache_participant_authority_for_test(
    writer_authority: ApplyWriterAuthority,
    root: &RetainedDirectoryCapability,
) -> WorkspaceCacheParticipantAuthority {
    WorkspaceCacheParticipantAuthority {
        writer_authority,
        logical_root: root.path().to_path_buf(),
        anchor_identity: root.identity(),
    }
}

impl std::fmt::Debug for ApplyAdmission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ApplyAdmission")
            .field("source_set", &self.source_set)
            .field("dry_run", &self.dry_run)
            .finish_non_exhaustive()
    }
}

impl ApplyAdmission {
    pub(crate) fn revision_identity(&self) -> String {
        self.revision.revision_identity()
    }

    pub(crate) fn staged_state(&self) -> Result<ApplyStagedState, ApplyStagingError> {
        apply_staging_checkpoint(self.deadline, &self.cancellation, "apply planning")?;
        Ok(ApplyStagedState::from_retained_root(
            Arc::clone(&self.source_root),
            self.deadline,
            self.cancellation.clone(),
            self.writer_authority.clone(),
        )
        .forbid_generated_subtree())
    }

    pub(crate) const fn support_policy_mode(&self) -> SupportPolicyMode {
        self.support_policy.mode()
    }

    pub(crate) fn code_planning_authority<'a>(
        &'a self,
        binding: &'a ProviderRootBinding,
    ) -> Result<CodeApplyAuthority<'a>, ApplyPlanError> {
        if binding.actor_identity != self.actor_identity
            || binding.actor_instance != self.actor_instance
            || binding.source_set != self.source_set
            || binding.source_root.path() != self.source_root.path()
            || binding.source_root.identity() != self.source_root.identity()
        {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidState,
                "code planning binding does not belong to this apply admission",
            ));
        }
        if binding.source_format() != SourceFormat::PlatformXml
            || !matches!(
                binding.source_kind(),
                SourceSetKind::Configuration | SourceSetKind::Extension
            )
        {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::ProviderUnavailable,
                "admitted source does not provide writable Platform XML code",
            ));
        }
        let profile = binding.source_profile().platform_profile().ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::ProviderUnavailable,
                "admitted source has no supported Platform XML profile",
            )
        })?;
        let expected_format = binding
            .source_profile()
            .serialization_format()
            .ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::ProviderUnavailable,
                    "admitted source has no exact serialization profile",
                )
            })?;
        Ok(CodeApplyAuthority {
            binding,
            writer_authority: &self.writer_authority,
            profile,
            expected_format,
            support_policy: self.support_policy.mode(),
        })
    }

    pub(crate) fn xdto_planning_authority<'a>(
        &'a self,
        binding: &'a ProviderRootBinding,
    ) -> Result<XdtoApplyAuthority<'a>, ApplyPlanError> {
        if binding.actor_identity != self.actor_identity
            || binding.actor_instance != self.actor_instance
            || binding.source_set != self.source_set
            || binding.source_root.path() != self.source_root.path()
            || binding.source_root.identity() != self.source_root.identity()
        {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidState,
                "XDTO planning binding does not belong to this apply admission",
            ));
        }
        if binding.source_format() != SourceFormat::PlatformXml
            || !matches!(
                binding.source_kind(),
                SourceSetKind::Configuration | SourceSetKind::Extension
            )
            || binding.source_profile() != SourceProfile::platform_xml_8_3_27_format_2_20()
        {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::ProviderUnavailable,
                "admitted source does not provide writable Platform XML XDTO",
            ));
        }
        let expected_format = binding
            .source_profile()
            .serialization_format()
            .ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::ProviderUnavailable,
                    "admitted source has no exact serialization profile",
                )
            })?;
        Ok(XdtoApplyAuthority {
            binding,
            writer_authority: &self.writer_authority,
            expected_format,
            support_policy: self.support_policy.mode(),
        })
    }

    fn platform_xml_family_authority<'a>(
        &'a self,
        binding: &'a ProviderRootBinding,
        family: &str,
    ) -> Result<PlatformXmlApplyAuthority<'a>, ApplyPlanError> {
        if binding.actor_identity != self.actor_identity
            || binding.actor_instance != self.actor_instance
            || binding.source_set != self.source_set
            || binding.source_root.path() != self.source_root.path()
            || binding.source_root.identity() != self.source_root.identity()
        {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidState,
                format!("{family} planning binding does not belong to this apply admission"),
            ));
        }
        if binding.source_format() != SourceFormat::PlatformXml
            || !matches!(
                binding.source_kind(),
                SourceSetKind::Configuration | SourceSetKind::Extension
            )
        {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::ProviderUnavailable,
                format!("admitted source does not provide writable Platform XML {family}"),
            ));
        }
        let profile = binding.source_profile().platform_profile().ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::ProviderUnavailable,
                "admitted source has no supported Platform XML profile",
            )
        })?;
        let expected_format = binding
            .source_profile()
            .serialization_format()
            .ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::ProviderUnavailable,
                    "admitted source has no exact serialization profile",
                )
            })?;
        Ok(PlatformXmlApplyAuthority {
            binding,
            writer_authority: &self.writer_authority,
            profile,
            expected_format,
            support_policy: self.support_policy.mode(),
            context: &self.context,
        })
    }

    pub(crate) fn metadata_planning_authority<'a>(
        &'a self,
        binding: &'a ProviderRootBinding,
    ) -> Result<MetadataApplyAuthority<'a>, ApplyPlanError> {
        self.platform_xml_family_authority(binding, "metadata")
            .map(MetadataApplyAuthority)
    }

    pub(crate) fn form_resource_planning_authority<'a>(
        &'a self,
        binding: &'a ProviderRootBinding,
    ) -> Result<FormResourceApplyAuthority<'a>, ApplyPlanError> {
        self.platform_xml_family_authority(binding, "form/resource")
            .map(FormResourceApplyAuthority)
    }

    pub(crate) fn dcs_mxl_planning_authority<'a>(
        &'a self,
        binding: &'a ProviderRootBinding,
    ) -> Result<DcsMxlApplyAuthority<'a>, ApplyPlanError> {
        self.platform_xml_family_authority(binding, "DCS/MXL")
            .map(DcsMxlApplyAuthority)
    }

    pub(crate) fn prepare(
        self,
        state: ApplyStagedState,
    ) -> Result<PreparedApplyBatch, ApplyStagingError> {
        self.prepare_with_effects(state, PlannedApplyEffects::default())
    }

    pub(crate) fn prepare_with_effects(
        self,
        state: ApplyStagedState,
        effects: PlannedApplyEffects,
    ) -> Result<PreparedApplyBatch, ApplyStagingError> {
        let events = effects.into_events();
        apply_staging_checkpoint(self.deadline, &self.cancellation, "apply preparation")?;
        if state.retained_root_identity() != self.source_root.identity() {
            return Err(ApplyStagingError::new(
                ApplyStagingErrorKind::ContainmentIdentity,
                "apply staged state belongs to another retained source root",
            ));
        }
        if !state.has_writer_authority(&self.writer_authority) {
            return Err(ApplyStagingError::new(
                ApplyStagingErrorKind::Invariant,
                "apply staged state belongs to another actor-issued authority",
            ));
        }
        let source_changes = state.planned_changes();
        let no_op = source_changes.is_empty() && events.is_empty();
        let source_transaction = state.finalize()?;
        let revision_reconciliation = self
            .revision_service
            .prepare_retained_apply_reconciliation(
                &self.source_root,
                &self.revision,
                &source_changes,
                self.deadline,
                &self.cancellation,
            )
            .map_err(retained_revision_staging_error)?;
        let mut cache_state = ApplyStagedState::from_retained_root(
            Arc::clone(&self.workspace_cache.anchor),
            self.deadline,
            self.cancellation.clone(),
            self.writer_authority.clone(),
        );
        let projected_cache_report =
            crate::infrastructure::workspace_state::WorkspaceStateRepository::new(&self.context)
                .stage_report_in_retained_apply(
                    &mut cache_state,
                    &self.workspace_cache.relative_root,
                    &self.context,
                    &events,
                    false,
                    CacheAccess::default(),
                )?;
        let record_path =
            normalize_path_identity(revision_reconciliation.record_path()).map_err(|error| {
                ApplyStagingError::new(ApplyStagingErrorKind::ContainmentIdentity, error)
            })?;
        let record_suffix = record_path
            .strip_prefix(&self.workspace_cache.logical_root)
            .map_err(|_| {
                ApplyStagingError::new(
                    ApplyStagingErrorKind::ContainmentIdentity,
                    "source revision record escaped the actor-owned cache root",
                )
            })?;
        let record_relative = self.workspace_cache.relative_root.join(record_suffix);
        match cache_state.read(&record_relative)? {
            Some(previous) => cache_state.replace(
                &record_relative,
                &previous,
                revision_reconciliation.record_bytes().to_vec(),
            )?,
            None => cache_state.create(
                &record_relative,
                revision_reconciliation.record_bytes().to_vec(),
            )?,
        }
        let cache_transaction = cache_state.finalize()?;
        let transaction = source_transaction
            .close_with_workspace_cache_participant(
                cache_transaction,
                &self.workspace_cache.participant,
            )
            .map_err(|error| ApplyStagingError::new(ApplyStagingErrorKind::Invariant, error))?;
        Ok(PreparedApplyBatch {
            actor_identity: self.actor_identity,
            actor_instance: self.actor_instance,
            source_set: self.source_set,
            source_root: self.source_root,
            revision_service: self.revision_service,
            revision: self.revision,
            dry_run: self.dry_run,
            no_op,
            deadline: self.deadline,
            cancellation: self.cancellation,
            transaction,
            writer_authority: self.writer_authority,
            workspace_cache: self.workspace_cache,
            support_policy: self.support_policy,
            source_selection: self.source_selection,
            effects: PreparedApplyEffectReceipt {
                events,
                cache: projected_cache_report,
            },
            revision_reconciliation,
        })
    }
}

#[cfg(test)]
impl ApplyAdmission {
    fn prepare_with_cache_effects(
        self,
        state: ApplyStagedState,
        events: &[DomainEvent],
    ) -> Result<PreparedApplyBatch, ApplyStagingError> {
        let mut effects = PlannedApplyEffects::default();
        for event in events {
            effects.append(event.clone());
        }
        self.prepare_with_effects(state, effects)
    }
}

pub(crate) struct PreparedApplyBatch {
    actor_identity: WorkspaceIdentity,
    actor_instance: ActorInstanceId,
    source_set: WorkspaceSourceSetIdentity,
    source_root: Arc<RetainedDirectoryCapability>,
    revision_service: Arc<SourceRevisionService>,
    revision: RetainedRevisionLease,
    dry_run: bool,
    no_op: bool,
    deadline: ProviderDeadline,
    cancellation: CancellationToken,
    transaction: CompileTransaction,
    writer_authority: ApplyWriterAuthority,
    workspace_cache: WorkspaceCachePublicationAuthority,
    support_policy: RetainedSupportPolicyEvidence,
    source_selection: RetainedSourceSelectionEvidence,
    effects: PreparedApplyEffectReceipt,
    revision_reconciliation: PreparedRevisionReconciliation,
}

#[derive(Debug)]
struct PreparedApplyEffectReceipt {
    events: Vec<DomainEvent>,
    cache: crate::domain::cache::CacheReport,
}

impl PreparedApplyEffectReceipt {
    fn into_terminal(self, disposition: ApplyEffectDisposition) -> ApplyEffectReceipt {
        ApplyEffectReceipt {
            disposition,
            events: self.events,
            cache: self.cache,
        }
    }
}

pub(in crate::infrastructure) struct RetainedApplyFinalGate<'a, R> {
    actor: &'a WorkspaceActor<R>,
    binding: ProviderRootBinding,
    deadline: ProviderDeadline,
    cancellation: CancellationToken,
    support_policy: RetainedSupportPolicyEvidence,
    source_selection: RetainedSourceSelectionEvidence,
}

impl<R> RetainedApplyFinalGate<'_, R> {
    pub(in crate::infrastructure) fn deadline(&self) -> ProviderDeadline {
        self.deadline
    }

    pub(in crate::infrastructure) fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }

    pub(in crate::infrastructure) fn checkpoint(
        &self,
        phase: &str,
    ) -> Result<(), ApplyPublicationError> {
        apply_publication_checkpoint(self.deadline, &self.cancellation, phase)
    }

    pub(in crate::infrastructure) fn validate(&self) -> Result<(), ApplyPublicationError> {
        self.validate_after_publication(&[])
    }

    /// Final gate after the transaction published its source files. Each
    /// published replacement travels as `(absolute path, post-image)`; the
    /// source-map evidence accepts a new identity for exactly those files
    /// when their bytes match and the selection semantics stay unchanged.
    pub(in crate::infrastructure) fn validate_after_publication(
        &self,
        published: &[(PathBuf, Vec<u8>)],
    ) -> Result<(), ApplyPublicationError> {
        self.actor
            .validate_binding(&self.binding)
            .map_err(|error| {
                ApplyPublicationError::new(ApplyPublicationErrorKind::ContainmentIdentity, error)
            })?;
        self.checkpoint("prepared apply final gate")?;
        self.support_policy
            .validate(self.deadline, &self.cancellation)
            .map_err(support_policy_publication_error)?;
        let workspace = self.source_selection.workspace_path();
        let published = published
            .iter()
            .filter_map(|(absolute, bytes)| {
                absolute
                    .strip_prefix(workspace)
                    .ok()
                    .map(|relative| (relative.to_path_buf(), bytes.clone()))
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        self.source_selection
            .validate_final_with_published(&published, self.deadline, &self.cancellation)
            .map_err(source_selection_publication_error)
    }
}

impl std::fmt::Debug for PreparedApplyBatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedApplyBatch")
            .field("source_set", &self.source_set)
            .field("dry_run", &self.dry_run)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApplyCleanupDiagnosticKind {
    RetainedRecoveryCleanupIncomplete,
}

/// Bounded actor-facing cleanup context. The target stays relative to the
/// selected source set and the artifact is only the fixed-format internal leaf;
/// neither field exposes the retained provider root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApplyCleanupDiagnostic {
    kind: ApplyCleanupDiagnosticKind,
    logical_target: PathBuf,
    last_known_artifact_name: OsString,
}

impl ApplyCleanupDiagnostic {
    pub(crate) const fn kind(&self) -> ApplyCleanupDiagnosticKind {
        self.kind
    }

    pub(crate) fn logical_target(&self) -> &Path {
        &self.logical_target
    }

    /// The transaction-generated recovery leaf before cleanup began. A
    /// concurrent namespace mutation may mean it no longer names the retained
    /// artifact, so callers must not treat this diagnostic as delete authority.
    pub(crate) fn last_known_artifact_name(&self) -> &OsStr {
        &self.last_known_artifact_name
    }
}

#[derive(Debug)]
pub(crate) struct ApplyPublicationResult {
    rev: String,
    effects: ApplyEffectReceipt,
    commit_count: usize,
    cleanup_diagnostics: Vec<ApplyCleanupDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApplyEffectDisposition {
    Projected,
    Committed,
}

#[derive(Debug)]
pub(crate) struct ApplyEffectReceipt {
    disposition: ApplyEffectDisposition,
    events: Vec<DomainEvent>,
    cache: crate::domain::cache::CacheReport,
}

impl ApplyEffectReceipt {
    pub(crate) const fn disposition(&self) -> ApplyEffectDisposition {
        self.disposition
    }

    pub(crate) fn events(&self) -> &[DomainEvent] {
        &self.events
    }

    pub(crate) const fn cache(&self) -> &crate::domain::cache::CacheReport {
        &self.cache
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApplyPublicationErrorKind {
    Cancelled,
    Deadline,
    ConcurrentRevision,
    ContainmentIdentity,
    ProviderPostvalidation,
    SourceSelectionChanged,
    RollbackIncomplete,
    Invariant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApplyPublicationError {
    kind: ApplyPublicationErrorKind,
    message: String,
}

impl ApplyPublicationError {
    pub(in crate::infrastructure) fn new(
        kind: ApplyPublicationErrorKind,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub(crate) fn kind(&self) -> ApplyPublicationErrorKind {
        self.kind
    }

    #[cfg(test)]
    fn contains(&self, pattern: &str) -> bool {
        self.message.contains(pattern)
    }
}

impl std::fmt::Display for ApplyPublicationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for ApplyPublicationError {}

impl ApplyPublicationResult {
    pub(crate) fn rev(&self) -> &str {
        &self.rev
    }

    pub(crate) fn cleanup_diagnostics(&self) -> &[ApplyCleanupDiagnostic] {
        &self.cleanup_diagnostics
    }

    pub(crate) const fn effects(&self) -> &ApplyEffectReceipt {
        &self.effects
    }

    #[cfg(test)]
    pub(crate) const fn commit_count_for_test(&self) -> usize {
        self.commit_count
    }
}

/// Exclusive, revision-fenced permission to make one staged mutation result
/// observable.  It intentionally exposes neither an ambient source-root path
/// nor a generic publication callback.  Task 15 can add descriptor-relative
/// writer operations to this capability when writers are routed through the
/// actor; until then there is no production validate-then-unchecked escape.
pub(crate) struct WorkspacePublicationLease<'actor, R> {
    actor: &'actor WorkspaceActor<R>,
    binding: ProviderRootBinding,
    issued_revision: SourceRevision,
    revision_service: Arc<SourceRevisionService>,
    _lane: MutexGuard<'actor, ()>,
}

/// Actor-owned runtime payloads expose only a payload-defined projection.
///
/// This prevents the generic actor from returning `&R` in production while
/// still allowing a compatibility adapter to define a module-private view of
/// the state it must bridge during migration.
pub(super) trait WorkspaceActorRuntimeProjection {
    type Projection<'a>
    where
        Self: 'a;

    fn project_for_actor(&self) -> Self::Projection<'_>;
}

#[cfg(test)]
pub(super) trait WorkspaceActorRuntimeTestProjection {
    type ProjectionMut<'a>
    where
        Self: 'a;

    fn project_mut_for_actor_test(&mut self) -> Self::ProjectionMut<'_>;
}

impl<R> std::fmt::Debug for WorkspacePublicationLease<'_, R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkspacePublicationLease")
            .field("binding", &self.binding)
            .finish_non_exhaustive()
    }
}

impl<R> WorkspacePublicationLease<'_, R> {
    pub(crate) fn publish<T>(
        self,
        staged_result: T,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<T, String> {
        self.actor.validate_binding(&self.binding)?;
        let current = self.revision_service.snapshot_retained(
            &self.binding.source_root,
            deadline,
            cancellation,
        );
        self.actor.validate_binding(&self.binding)?;
        let current = current?;
        if current != self.issued_revision {
            return Err("source revision changed before staged result publication".to_string());
        }
        Ok(staged_result)
    }
}

/// One daemon-owned coordination boundary for a canonical worktree.
///
/// Reads do not take the mutation lane. Read-only staged-result confirmation
/// is exclusive and rechecks the actor-owned source revision immediately
/// before that result is returned. Task 15 adds the writer boundary.
pub(crate) struct WorkspaceActor<R = ()> {
    identity: WorkspaceIdentity,
    instance_id: ActorInstanceId,
    context: WorkspaceContext,
    source_roots: HashMap<WorkspaceSourceSetIdentity, Arc<RetainedDirectoryCapability>>,
    state_scope: WorkspaceStateScope,
    mutation_lane: DeadlineLock<FailClosed>,
    source_revisions: Mutex<HashMap<WorkspaceSourceSetIdentity, Arc<SourceRevisionService>>>,
    index_work: SharedWork<(), LongWorkFailure>,
    runtime: R,
}

assert_not_impl_production!(WorkspaceActor<()>: std::ops::Deref);
assert_not_impl_production!(WorkspaceActor<()>: std::ops::DerefMut);

impl<R> std::fmt::Debug for WorkspaceActor<R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkspaceActor")
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

impl WorkspaceActor<()> {
    pub(crate) fn new(
        identity: WorkspaceIdentity,
        context: WorkspaceContext,
    ) -> Result<Self, String> {
        Self::with_runtime(identity, context, ())
    }
}

fn validate_legacy_workspace_identity(identity: &WorkspaceIdentity) -> Result<(), String> {
    let [source_set] = identity.source_sets.as_slice() else {
        return Err(
            "legacy workspace actor requires exactly one compatibility source set".to_string(),
        );
    };
    if source_set.kind != SourceSetKind::Configuration
        || source_set.source_format != SourceFormat::Unknown
        || source_set.source_profile != SourceProfile::legacy_workspace_service_compatibility()
    {
        return Err(
            "legacy workspace actor requires Configuration/Unknown/legacy-compatibility identity"
                .to_string(),
        );
    }
    Ok(())
}

impl<R> WorkspaceActor<R> {
    pub(crate) fn with_runtime(
        identity: WorkspaceIdentity,
        context: WorkspaceContext,
        runtime: R,
    ) -> Result<Self, String> {
        let state_scope = WorkspaceStateScope::scoped_sha256(identity.state_scope_digest()?)?;
        Self::with_runtime_scope(identity, context, runtime, state_scope)
    }

    pub(crate) fn with_legacy_runtime(
        identity: WorkspaceIdentity,
        context: WorkspaceContext,
        runtime: R,
    ) -> Result<Self, String> {
        validate_legacy_workspace_identity(&identity)?;
        Self::with_runtime_scope(
            identity,
            context,
            runtime,
            WorkspaceStateScope::LegacyPhysical,
        )
    }

    fn with_runtime_scope(
        identity: WorkspaceIdentity,
        mut context: WorkspaceContext,
        runtime: R,
        state_scope: WorkspaceStateScope,
    ) -> Result<Self, String> {
        let context_root = normalize_path_identity(&context.workspace_root)?;
        if context_root != identity.workspace_root {
            return Err(
                "workspace actor context does not match its canonical identity".to_string(),
            );
        }
        let mut source_roots = HashMap::new();
        let mut physical_identities = HashSet::new();
        for source_set in &identity.source_sets {
            let capability = RetainedDirectoryCapability::open(&source_set.root).map_err(|error| {
                format!(
                    "workspace actor source-set root cannot be retained without following links: {}: {error}",
                    source_set.root.display()
                )
            })?;
            if !physical_identities.insert(capability.identity()) {
                return Err(
                    "workspace actor source-set roots resolve to one ambiguous physical directory"
                        .to_string(),
                );
            }
            source_roots.insert(source_set.clone(), Arc::new(capability));
        }
        context.workspace_root = context_root.clone();
        context.cwd = context_root;
        Ok(Self {
            identity,
            instance_id: ActorInstanceId::new(),
            context,
            source_roots,
            state_scope,
            mutation_lane: DeadlineLock::fail_closed("workspace actor mutation lane is poisoned"),
            source_revisions: Mutex::new(HashMap::new()),
            index_work: SharedWork::new(SharedWorkLifetime::ProducerBound),
            runtime,
        })
    }

    pub(crate) fn identity(&self) -> &WorkspaceIdentity {
        &self.identity
    }

    /// Whether every retained source-set root is still the directory its
    /// name resolves to. A warm actor whose root was replaced while idle
    /// must not be handed to the next admission.
    pub(crate) fn retains_named_roots(&self) -> bool {
        self.source_roots
            .values()
            .all(|root| root.validate_named_identity().is_ok())
    }

    pub(crate) fn workspace_identity(&self) -> &WorkspaceIdentity {
        &self.identity
    }

    /// Closed daemon-safe identity for task persistence. It is derived from
    /// the actor's complete structural identity; no caller-provided path or
    /// repository label can forge it.
    pub(crate) fn safe_identity_hash(&self) -> Result<SafeIdentityHash, String> {
        self.identity
            .state_scope_hash()
            .map(SafeIdentityHash::from_sha256)
    }

    pub(super) fn runtime_projection(&self) -> R::Projection<'_>
    where
        R: WorkspaceActorRuntimeProjection,
    {
        self.runtime.project_for_actor()
    }

    #[cfg(test)]
    pub(super) fn runtime_projection_mut_for_test(&mut self) -> R::ProjectionMut<'_>
    where
        R: WorkspaceActorRuntimeTestProjection,
    {
        self.runtime.project_mut_for_actor_test()
    }

    pub(crate) fn context(&self) -> &WorkspaceContext {
        &self.context
    }

    pub(crate) fn bind_provider_root(
        &self,
        source_set_name: &str,
        requested_root: &Path,
    ) -> Result<ProviderRootBinding, String> {
        let requested_root = normalize_path_identity(requested_root)?;
        let source_set = self
            .identity
            .source_sets
            .iter()
            .find(|source_set| {
                source_set.name == source_set_name && source_set.root == requested_root
            })
            .cloned();
        let Some(source_set) = source_set else {
            return Err(format!(
                "provider root is not bound to source set `{source_set_name}` in this workspace actor: {}",
                requested_root.display()
            ));
        };
        let source_root = self
            .source_roots
            .get(&source_set)
            .cloned()
            .ok_or_else(|| "workspace actor retained root is unavailable".to_string())?;
        validate_physical_root(&source_root)?;
        Ok(ProviderRootBinding {
            actor_identity: self.identity.clone(),
            actor_instance: self.instance_id.clone(),
            source_set,
            source_root,
        })
    }

    pub(crate) fn read<T>(
        &self,
        binding: &ProviderRootBinding,
        read: impl FnOnce(&Path) -> Result<T, String>,
    ) -> Result<T, String> {
        self.validate_binding(binding)?;
        let result = read(binding.source_root.path());
        self.validate_binding(binding)?;
        result
    }

    pub(crate) fn read_relative_file(
        &self,
        binding: &ProviderRootBinding,
        relative: &Path,
        max_bytes: usize,
    ) -> Result<Vec<u8>, String> {
        self.validate_binding(binding)?;
        let result = binding
            .source_root
            .read_relative_regular_bounded(relative, max_bytes)
            .map_err(|error| format!("actor-bound relative read failed: {error}"));
        self.validate_binding(binding)?;
        result
    }

    pub(crate) fn capture_revision(
        &self,
        binding: &ProviderRootBinding,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceRevisionFence, String> {
        self.validate_binding(binding)?;
        let revision = self.source_revision_service(binding).and_then(|service| {
            service.snapshot_retained(&binding.source_root, deadline, cancellation)
        });
        self.validate_binding(binding)?;
        let revision = revision?;
        Ok(WorkspaceRevisionFence {
            actor_identity: self.identity.clone(),
            actor_instance: self.instance_id.clone(),
            source_set: binding.source_set.clone(),
            revision,
        })
    }

    pub(crate) fn capture_logical_read_revision(
        &self,
        binding: &ProviderRootBinding,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceLogicalReadFence, String> {
        self.validate_binding(binding)?;
        let revision = self
            .source_revision_service(binding)?
            .begin_retained_operation(&binding.source_root, deadline, cancellation);
        self.validate_binding(binding)?;
        Ok(WorkspaceLogicalReadFence {
            actor_identity: self.identity.clone(),
            actor_instance: self.instance_id.clone(),
            source_set: binding.source_set.clone(),
            revision: revision?,
        })
    }

    pub(crate) fn admit_apply(
        &self,
        binding: &ProviderRootBinding,
        if_rev: Option<&str>,
        dry_run: bool,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<ApplyAdmission, ApplyAdmissionError> {
        apply_checkpoint(deadline, cancellation, "apply admission")?;
        self.validate_binding(binding)?;
        let revision_service = self.source_revision_service(binding)?;
        let revision = revision_service.observe_retained_operation(
            &binding.source_root,
            deadline,
            cancellation,
        )?;
        self.validate_binding(binding)?;
        let revision_identity = revision.revision_identity();
        if if_rev.is_some_and(|expected| expected != revision_identity) {
            return Err(ApplyAdmissionError::StaleRevision {
                expected: if_rev.unwrap_or_default().to_string(),
                admitted: revision_identity,
            });
        }
        let source_selection = {
            let mut checkpoint =
                || apply_checkpoint(deadline, cancellation, "apply source-map admission");
            let resolved =
                discover_project_source_admission(&self.context.workspace_root, &mut checkpoint)?;
            self.validate_source_selection_admission(binding, &resolved)?;
            resolved.into_evidence()
        };
        let writer_authority = ApplyWriterAuthority::issue();
        let workspace_cache = retain_workspace_cache_publication_authority(
            &self.context,
            &binding.source_root,
            &writer_authority,
        )?;
        let support_policy = RetainedSupportPolicyEvidence::capture(
            &self.context.workspace_root,
            binding.source_root.path(),
            deadline,
            cancellation,
        )
        .map_err(|error| error.to_string())?;
        Ok(ApplyAdmission {
            actor_identity: self.identity.clone(),
            actor_instance: self.instance_id.clone(),
            source_set: binding.source_set.clone(),
            source_root: Arc::clone(&binding.source_root),
            revision_service,
            revision,
            dry_run,
            deadline,
            cancellation: cancellation.clone(),
            writer_authority,
            workspace_cache,
            support_policy,
            source_selection,
            context: self.context.clone(),
        })
    }

    fn validate_source_selection_admission(
        &self,
        binding: &ProviderRootBinding,
        admission: &ResolvedProjectSourceAdmission,
    ) -> Result<(), String> {
        let projection =
            supported_actor_source_projection(&self.identity.workspace_root, admission.map())?;
        if projection != self.identity.source_sets {
            return Err(
                "project source-map supported projection does not match the workspace actor"
                    .to_string(),
            );
        }
        let selected = projection
            .iter()
            .find(|source| source.name == binding.source_set.name)
            .ok_or_else(|| {
                "project source-map no longer contains the actor-selected source".to_string()
            })?;
        if selected != &binding.source_set {
            return Err(
                "project source-map selected row does not match the actor binding".to_string(),
            );
        }
        let selected_row = admission
            .map()
            .source_sets
            .iter()
            .find(|source| {
                source.name == selected.name
                    && source.kind == selected.kind
                    && source.source_format == selected.source_format
            })
            .ok_or_else(|| "project source-map selected semantic row is unavailable".to_string())?;
        let relative = closed_source_selection_relative_path(&selected_row.path)?;
        if admission.source_root_identity(&relative) != Some(binding.source_root.identity()) {
            return Err(
                "project source-map selected retained root does not match the actor binding"
                    .to_string(),
            );
        }
        Ok(())
    }

    pub(crate) fn publish_prepared_apply(
        &self,
        prepared: PreparedApplyBatch,
    ) -> Result<ApplyPublicationResult, ApplyPublicationError> {
        if prepared.actor_identity != self.identity
            || prepared.actor_instance != self.instance_id
            || !self.identity.source_sets.contains(&prepared.source_set)
        {
            return Err(ApplyPublicationError::new(
                ApplyPublicationErrorKind::Invariant,
                "prepared apply batch belongs to another workspace actor",
            ));
        }
        apply_publication_checkpoint(
            prepared.deadline,
            &prepared.cancellation,
            "prepared apply publication",
        )?;
        let _lane = self
            .mutation_lane
            .acquire_before(
                prepared.deadline,
                &prepared.cancellation,
                "workspace actor prepared apply wait",
            )
            .map_err(deadline_lock_publication_error)?;
        let binding = ProviderRootBinding {
            actor_identity: prepared.actor_identity.clone(),
            actor_instance: prepared.actor_instance.clone(),
            source_set: prepared.source_set.clone(),
            source_root: Arc::clone(&prepared.source_root),
        };
        self.validate_binding(&binding).map_err(|error| {
            ApplyPublicationError::new(ApplyPublicationErrorKind::ContainmentIdentity, error)
        })?;
        prepared
            .workspace_cache
            .anchor
            .validate_named_identity()
            .map_err(|error| {
                ApplyPublicationError::new(
                    ApplyPublicationErrorKind::ContainmentIdentity,
                    format!(
                        "workspace-cache retained authority changed before publication: {error}"
                    ),
                )
            })?;
        prepared
            .revision_service
            .confirm_retained_observation_typed(
                &prepared.source_root,
                &prepared.revision,
                prepared.deadline,
                &prepared.cancellation,
            )
            .map_err(retained_revision_publication_error)?;
        prepared
            .support_policy
            .validate(prepared.deadline, &prepared.cancellation)
            .map_err(support_policy_publication_error)?;
        prepared
            .source_selection
            .validate(prepared.deadline, &prepared.cancellation)
            .map_err(source_selection_publication_error)?;
        if prepared.dry_run || prepared.no_op {
            prepared
                .transaction
                .validate_retained_for_apply_typed()
                .map_err(apply_validation_publication_error)?;
            prepared
                .revision_service
                .confirm_retained_observation_typed(
                    &prepared.source_root,
                    &prepared.revision,
                    prepared.deadline,
                    &prepared.cancellation,
                )
                .map_err(retained_revision_publication_error)?;
            #[cfg(test)]
            run_apply_dry_run_after_confirmation_hook();
            prepared
                .support_policy
                .validate(prepared.deadline, &prepared.cancellation)
                .map_err(support_policy_publication_error)?;
            prepared
                .source_selection
                .validate_dry_result(prepared.deadline, &prepared.cancellation)
                .map_err(source_selection_publication_error)?;
            self.validate_binding(&binding).map_err(|error| {
                ApplyPublicationError::new(ApplyPublicationErrorKind::ContainmentIdentity, error)
            })?;
            apply_publication_checkpoint(
                prepared.deadline,
                &prepared.cancellation,
                "prepared apply result",
            )?;
            return Ok(ApplyPublicationResult {
                rev: prepared.revision.revision_identity(),
                effects: prepared.effects.into_terminal(if prepared.dry_run {
                    ApplyEffectDisposition::Projected
                } else {
                    ApplyEffectDisposition::Committed
                }),
                commit_count: 0,
                cleanup_diagnostics: Vec::new(),
            });
        }

        let final_gate = RetainedApplyFinalGate {
            actor: self,
            binding,
            deadline: prepared.deadline,
            cancellation: prepared.cancellation.clone(),
            support_policy: prepared.support_policy,
            source_selection: prepared.source_selection,
        };
        let (report, revision) = prepared.transaction.commit_retained_apply(
            prepared.writer_authority,
            prepared.revision_reconciliation,
            final_gate,
        )?;
        debug_assert_eq!(
            report.cleanup_warnings.len(),
            report.retained_apply_cleanup_diagnostics.len(),
            "every retained apply cleanup warning must have structured actor context"
        );
        let cleanup_diagnostics = report
            .retained_apply_cleanup_diagnostics
            .into_iter()
            .map(|diagnostic| {
                let (logical_target, last_known_artifact_name) = diagnostic.into_parts();
                ApplyCleanupDiagnostic {
                    kind: ApplyCleanupDiagnosticKind::RetainedRecoveryCleanupIncomplete,
                    logical_target,
                    last_known_artifact_name,
                }
            })
            .collect();
        Ok(ApplyPublicationResult {
            rev: format!(
                "{}:{}:{}",
                revision.algorithm, revision.generation, revision.digest
            ),
            effects: prepared
                .effects
                .into_terminal(ApplyEffectDisposition::Committed),
            commit_count: 1,
            cleanup_diagnostics,
        })
    }

    /// Makes one staged logical-read result observable while holding the
    /// actor's single mutation lane across validation of every selected source
    /// and every retained final confirmation.
    pub(crate) fn publish_logical_read<T>(
        &self,
        fences: &[WorkspaceLogicalReadFence],
        staged_result: T,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<T, String> {
        if fences.is_empty() {
            return Err("logical read publication requires an admitted source fence".to_string());
        }
        let _publication = self
            .mutation_lane
            .acquire_before(
                deadline,
                cancellation,
                "workspace actor logical read publication wait",
            )
            .map_err(|error| error.to_string())?;
        let mut confirmations = Vec::with_capacity(fences.len());
        let mut source_sets = HashSet::with_capacity(fences.len());
        for fence in fences {
            if fence.actor_identity != self.identity
                || fence.actor_instance != self.instance_id
                || !source_sets.insert(fence.source_set.clone())
            {
                return Err(
                    "logical read revision fence belongs to another or duplicate workspace source"
                        .to_string(),
                );
            }
            let source_root = self
                .source_roots
                .get(&fence.source_set)
                .cloned()
                .ok_or_else(|| "workspace actor retained root is unavailable".to_string())?;
            let binding = ProviderRootBinding {
                actor_identity: fence.actor_identity.clone(),
                actor_instance: fence.actor_instance.clone(),
                source_set: fence.source_set.clone(),
                source_root,
            };
            self.validate_binding(&binding)?;
            confirmations.push((
                self.source_revision_service(&binding)?,
                binding,
                &fence.revision,
            ));
        }
        for (revisions, binding, revision) in confirmations {
            revisions.confirm_retained_operation(
                &binding.source_root,
                revision,
                deadline,
                cancellation,
            )?;
            #[cfg(test)]
            run_logical_publication_after_confirmation_hook();
        }
        Ok(staged_result)
    }

    /// Exact actor-owned index readiness identity. The workspace component is
    /// derived from this actor's complete canonical identity; the source-set
    /// and revision come from capabilities issued by this same actor instance.
    pub(crate) fn join_index_work<W>(
        &self,
        binding: &ProviderRootBinding,
        fence: &WorkspaceRevisionFence,
        provider: &str,
        profile: &str,
        work: W,
    ) -> Result<(IndexWorkIdentity, SharedWorkLease<(), LongWorkFailure>), String>
    where
        W: FnOnce(SharedWorkProducer) -> Result<(), LongWorkFailure> + Send + 'static,
    {
        self.validate_binding(binding)?;
        if fence.actor_identity != self.identity
            || fence.actor_instance != self.instance_id
            || fence.source_set != binding.source_set
        {
            return Err("index revision fence belongs to another workspace actor".to_string());
        }
        if !closed_index_component(provider)
            || !closed_index_component(profile)
            || fence.revision.algorithm != crate::domain::source_revision::SOURCE_REVISION_ALGORITHM
            || fence.revision.generation == 0
            || !is_lowercase_sha256(&fence.revision.digest)
        {
            return Err("actor-owned index work identity is invalid".to_string());
        }
        let revision_service = self.source_revision_service(binding)?;
        let current = revision_service.snapshot_retained(
            &binding.source_root,
            ProviderDeadline::from_budget(INDEX_FENCE_BUDGET),
            &CancellationToken::new(),
        );
        self.validate_binding(binding)?;
        if current? != fence.revision {
            return Err("index revision fence changed before shared-work admission".to_string());
        }

        let mut digest = Sha256::new();
        digest.update(b"unica-index-work-v2\0");
        digest.update(self.identity.state_scope_digest()?);
        digest_index_component(&mut digest, binding.source_set.name.as_bytes())?;
        digest_index_component(&mut digest, fence.revision.algorithm.as_bytes())?;
        digest.update(fence.revision.generation.to_le_bytes());
        digest_index_component(&mut digest, fence.revision.digest.as_bytes())?;
        digest_index_component(&mut digest, provider.as_bytes())?;
        digest_index_component(&mut digest, profile.as_bytes())?;
        let identity = IndexWorkIdentity(digest.finalize().into());

        let retained_root = Arc::clone(&binding.source_root);
        let expected_revision = fence.revision.clone();
        let producer_revision_service = Arc::clone(&revision_service);
        let lease = self.index_work.join_or_start(
            SharedWorkKey::Index {
                identity: identity.0,
            },
            move |producer| {
                retained_root
                    .validate_named_identity()
                    .map_err(|_| LongWorkFailure::Invalidated)?;
                let current = producer_revision_service
                    .snapshot_retained(
                        &retained_root,
                        ProviderDeadline::from_budget(INDEX_FENCE_BUDGET),
                        &CancellationToken::new(),
                    )
                    .map_err(|_| LongWorkFailure::Invalidated)?;
                retained_root
                    .validate_named_identity()
                    .map_err(|_| LongWorkFailure::Invalidated)?;
                if current != expected_revision {
                    return Err(LongWorkFailure::Invalidated);
                }
                work(producer)
            },
        );
        Ok((identity, lease))
    }

    pub(crate) fn begin_publication(
        &self,
        fence: &WorkspaceRevisionFence,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<WorkspacePublicationLease<'_, R>, String> {
        let publication = self
            .mutation_lane
            .acquire_before(deadline, cancellation, "workspace actor mutation lane wait")
            .map_err(|error| error.to_string())?;
        if fence.actor_identity != self.identity
            || fence.actor_instance != self.instance_id
            || !self.identity.source_sets.contains(&fence.source_set)
        {
            return Err("source revision fence belongs to another workspace actor".to_string());
        }
        let binding = ProviderRootBinding {
            actor_identity: fence.actor_identity.clone(),
            actor_instance: fence.actor_instance.clone(),
            source_root: self
                .source_roots
                .get(&fence.source_set)
                .cloned()
                .ok_or_else(|| "workspace actor retained root is unavailable".to_string())?,
            source_set: fence.source_set.clone(),
        };
        self.validate_binding(&binding)?;
        let revision_service = self.source_revision_service(&binding)?;
        let current =
            revision_service.snapshot_retained(&binding.source_root, deadline, cancellation);
        self.validate_binding(&binding)?;
        let current = current?;
        if current != fence.revision {
            return Err("source revision changed before publication".to_string());
        }
        Ok(WorkspacePublicationLease {
            actor: self,
            binding,
            issued_revision: fence.revision.clone(),
            revision_service,
            _lane: publication,
        })
    }

    pub(crate) fn index_service<'a>(
        &self,
        binding: &ProviderRootBinding,
        runner: &'a dyn IndexRunner,
    ) -> Result<WorkspaceIndexService<'a>, String> {
        self.validate_binding(binding)?;
        Ok(WorkspaceIndexService::with_runner(runner)
            .with_source_revision_service(self.source_revision_service(binding)?)
            .with_bound_source_root(Arc::clone(&binding.source_root))
            .with_state_scope(self.state_scope.clone()))
    }

    pub(crate) fn source_revision_service(
        &self,
        binding: &ProviderRootBinding,
    ) -> Result<Arc<SourceRevisionService>, String> {
        self.validate_binding(binding)?;
        let mut revisions = self
            .source_revisions
            .lock()
            .map_err(|_| "workspace actor revision registry is poisoned".to_string())?;
        if let Some(service) = revisions.get(&binding.source_set) {
            return Ok(Arc::clone(service));
        }
        let service = Arc::new(match self.state_scope.scoped_digest() {
            None => SourceRevisionService::new(&self.context, binding.source_root.path())?,
            Some(_) => {
                let authority = self.issue_revision_service_authority(binding)?;
                #[cfg(test)]
                run_revision_service_after_binding_validation_hook();
                SourceRevisionService::new_actor(&self.context, authority)?
            }
        });
        revisions.insert(binding.source_set.clone(), Arc::clone(&service));
        Ok(service)
    }

    fn issue_revision_service_authority(
        &self,
        binding: &ProviderRootBinding,
    ) -> Result<ActorRevisionServiceAuthority, String> {
        self.validate_binding(binding)?;
        if self.state_scope.scoped_digest().is_none() {
            return Err("actor revision authority requires a scoped actor state".to_string());
        }
        Ok(ActorRevisionServiceAuthority {
            source_root: Arc::clone(&binding.source_root),
            state_scope: self.state_scope.clone(),
            source_kind: binding.source_kind(),
            source_format: binding.source_format(),
            source_profile: binding.source_profile(),
        })
    }

    pub(crate) fn mark_source_revisions_dirty(&self) {
        if let Ok(revisions) = self.source_revisions.lock() {
            for revision in revisions.values() {
                revision.mark_dirty();
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn install_source_revision_service_for_test(
        &self,
        binding: &ProviderRootBinding,
        service: Arc<SourceRevisionService>,
    ) -> Result<(), String> {
        self.validate_binding(binding)?;
        self.source_revisions
            .lock()
            .map_err(|_| "workspace actor revision registry is poisoned".to_string())?
            .insert(binding.source_set.clone(), service);
        Ok(())
    }

    pub(crate) fn validate_binding(&self, binding: &ProviderRootBinding) -> Result<(), String> {
        if binding.actor_identity != self.identity
            || binding.actor_instance != self.instance_id
            || !self.identity.source_sets.contains(&binding.source_set)
            || binding.source_set.root != binding.source_root.path()
        {
            return Err("provider root binding belongs to another workspace actor".to_string());
        }
        validate_physical_root(&binding.source_root)
    }
}

fn supported_actor_source_projection(
    workspace_root: &Path,
    map: &crate::domain::project_sources::ProjectSourceMap,
) -> Result<Vec<WorkspaceSourceSetIdentity>, String> {
    let mut projection = map
        .source_sets
        .iter()
        .filter(|source| source.source_format == SourceFormat::PlatformXml)
        .map(|source| {
            let relative = closed_source_selection_relative_path(&source.path)?;
            Ok(WorkspaceSourceSetIdentity {
                name: source.name.clone(),
                root: workspace_root.join(relative),
                kind: source.kind,
                source_format: source.source_format,
                source_profile: SourceProfile::platform_xml_8_3_27_format_2_20(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    projection.sort();
    Ok(projection)
}

fn closed_source_selection_relative_path(path: &str) -> Result<PathBuf, String> {
    let mut relative = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            std::path::Component::Normal(name) => relative.push(name),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => {
                return Err(format!(
                    "project source-map row is not a closed workspace-relative route: {path}"
                ));
            }
        }
    }
    Ok(relative)
}

fn retain_workspace_cache_publication_authority(
    context: &WorkspaceContext,
    source_root: &RetainedDirectoryCapability,
    writer_authority: &ApplyWriterAuthority,
) -> Result<WorkspaceCachePublicationAuthority, String> {
    let cache_root = normalize_path_identity(&context.cache_root)?;
    let source_root_path = normalize_path_identity(source_root.path())?;
    if source_root_path.starts_with(&cache_root) {
        return Err(
            "workspace cache/source overlap: roots must not contain one another".to_string(),
        );
    }
    if let Ok(relative_cache) = cache_root.strip_prefix(&source_root_path) {
        let expected = Path::new(GENERATED_DIR_NAME).join("unica");
        if relative_cache != expected {
            return Err(
                "workspace cache/source overlap is only allowed for exact .build/unica".to_string(),
            );
        }
    }
    let relative = cache_root
        .strip_prefix(&context.workspace_root)
        .map_err(|_| "workspace cache root escaped the actor workspace".to_string())?;
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err("workspace cache root is not a closed workspace-relative path".to_string());
    }
    let mut anchor = RetainedDirectoryCapability::open(&context.workspace_root)
        .map_err(|error| format!("workspace cache ancestor cannot be retained: {error}"))?;
    let components = relative
        .components()
        .map(|component| component.as_os_str().to_os_string())
        .collect::<Vec<_>>();
    let mut relative_root = PathBuf::new();
    for (index, name) in components.iter().enumerate() {
        match anchor.retain_immediate_child_nofollow(name) {
            Ok(RetainedChildCapability::Directory(directory)) => anchor = directory,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                relative_root.extend(components[index..].iter());
                break;
            }
            Ok(RetainedChildCapability::ReparsePoint) => {
                return Err("workspace cache authority encountered a link/reparse point".to_string())
            }
            Ok(_) => {
                return Err(
                    "workspace cache authority encountered a non-directory component".to_string(),
                )
            }
            Err(error) => {
                return Err(format!(
                    "workspace cache authority cannot retain its path: {error}"
                ))
            }
        }
    }
    if cache_root == source_root_path && anchor.identity() == source_root.identity() {
        return Err(
            "workspace cache and source roots resolve to one physical directory".to_string(),
        );
    }
    let participant = WorkspaceCacheParticipantAuthority {
        writer_authority: writer_authority.clone(),
        logical_root: cache_root.clone(),
        anchor_identity: anchor.identity(),
    };
    Ok(WorkspaceCachePublicationAuthority {
        logical_root: cache_root,
        anchor: Arc::new(anchor),
        relative_root,
        participant,
    })
}

fn apply_checkpoint(
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
    phase: &str,
) -> Result<(), String> {
    if cancellation.is_cancelled() {
        Err(format!("{phase} cancelled"))
    } else if deadline.remaining().is_zero() {
        Err(format!("{phase} deadline exceeded"))
    } else {
        Ok(())
    }
}

fn apply_publication_checkpoint(
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
    phase: &str,
) -> Result<(), ApplyPublicationError> {
    if cancellation.is_cancelled() {
        Err(ApplyPublicationError::new(
            ApplyPublicationErrorKind::Cancelled,
            format!("{phase} cancelled"),
        ))
    } else if deadline.remaining().is_zero() {
        Err(ApplyPublicationError::new(
            ApplyPublicationErrorKind::Deadline,
            format!("{phase} deadline exceeded"),
        ))
    } else {
        Ok(())
    }
}

fn deadline_lock_publication_error(error: DeadlineLockError) -> ApplyPublicationError {
    let kind = match error.kind() {
        DeadlineLockErrorKind::Cancelled => ApplyPublicationErrorKind::Cancelled,
        DeadlineLockErrorKind::Deadline => ApplyPublicationErrorKind::Deadline,
        DeadlineLockErrorKind::Poisoned => ApplyPublicationErrorKind::Invariant,
    };
    ApplyPublicationError::new(kind, error.to_string())
}

fn retained_revision_staging_error(error: RetainedRevisionError) -> ApplyStagingError {
    let kind = match error.kind() {
        RetainedRevisionErrorKind::Cancelled => ApplyStagingErrorKind::Cancelled,
        RetainedRevisionErrorKind::Deadline => ApplyStagingErrorKind::Deadline,
        RetainedRevisionErrorKind::ConcurrentRevision => ApplyStagingErrorKind::ConcurrentRevision,
        RetainedRevisionErrorKind::ContainmentIdentity => {
            ApplyStagingErrorKind::ContainmentIdentity
        }
        RetainedRevisionErrorKind::Provider => ApplyStagingErrorKind::UnsupportedProvider,
        RetainedRevisionErrorKind::Invariant => ApplyStagingErrorKind::Invariant,
    };
    ApplyStagingError::new(kind, error.to_string())
}

pub(in crate::infrastructure) fn retained_revision_publication_error(
    error: RetainedRevisionError,
) -> ApplyPublicationError {
    let kind = match error.kind() {
        RetainedRevisionErrorKind::Cancelled => ApplyPublicationErrorKind::Cancelled,
        RetainedRevisionErrorKind::Deadline => ApplyPublicationErrorKind::Deadline,
        RetainedRevisionErrorKind::ConcurrentRevision => {
            ApplyPublicationErrorKind::ConcurrentRevision
        }
        RetainedRevisionErrorKind::ContainmentIdentity => {
            ApplyPublicationErrorKind::ContainmentIdentity
        }
        RetainedRevisionErrorKind::Provider => ApplyPublicationErrorKind::ProviderPostvalidation,
        RetainedRevisionErrorKind::Invariant => ApplyPublicationErrorKind::Invariant,
    };
    ApplyPublicationError::new(kind, error.to_string())
}

fn apply_validation_publication_error(
    error: crate::infrastructure::native_operations::compile_transaction::RetainedApplyValidationError,
) -> ApplyPublicationError {
    use crate::infrastructure::native_operations::compile_transaction::RetainedApplyValidationErrorKind;

    let kind = match error.kind() {
        RetainedApplyValidationErrorKind::ContainmentIdentity
        | RetainedApplyValidationErrorKind::AbsentChainOccupied => {
            ApplyPublicationErrorKind::ContainmentIdentity
        }
        RetainedApplyValidationErrorKind::UnsupportedProvider => {
            ApplyPublicationErrorKind::ProviderPostvalidation
        }
        RetainedApplyValidationErrorKind::Invariant => ApplyPublicationErrorKind::Invariant,
    };
    ApplyPublicationError::new(kind, error.to_string())
}

fn support_policy_publication_error(error: SupportPolicyEvidenceError) -> ApplyPublicationError {
    let kind = match error.kind() {
        SupportPolicyEvidenceErrorKind::Cancelled => ApplyPublicationErrorKind::Cancelled,
        SupportPolicyEvidenceErrorKind::Deadline => ApplyPublicationErrorKind::Deadline,
        SupportPolicyEvidenceErrorKind::ContainmentIdentity => {
            ApplyPublicationErrorKind::ContainmentIdentity
        }
        SupportPolicyEvidenceErrorKind::Provider => {
            ApplyPublicationErrorKind::ProviderPostvalidation
        }
    };
    ApplyPublicationError::new(kind, error.to_string())
}

fn source_selection_publication_error(
    error: SourceSelectionEvidenceError,
) -> ApplyPublicationError {
    let kind = match error.kind() {
        SourceSelectionEvidenceErrorKind::Cancelled => ApplyPublicationErrorKind::Cancelled,
        SourceSelectionEvidenceErrorKind::Deadline => ApplyPublicationErrorKind::Deadline,
        SourceSelectionEvidenceErrorKind::Changed => {
            ApplyPublicationErrorKind::SourceSelectionChanged
        }
        SourceSelectionEvidenceErrorKind::Provider => {
            ApplyPublicationErrorKind::ProviderPostvalidation
        }
    };
    ApplyPublicationError::new(kind, error.to_string())
}

fn apply_staging_checkpoint(
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
    phase: &str,
) -> Result<(), ApplyStagingError> {
    if cancellation.is_cancelled() {
        Err(ApplyStagingError::new(
            ApplyStagingErrorKind::Cancelled,
            format!("{phase} cancelled"),
        ))
    } else if deadline.remaining().is_zero() {
        Err(ApplyStagingError::new(
            ApplyStagingErrorKind::Deadline,
            format!("{phase} deadline exceeded"),
        ))
    } else {
        Ok(())
    }
}

fn validate_physical_root(root: &RetainedDirectoryCapability) -> Result<(), String> {
    root.validate_named_identity().map_err(|error| {
        format!(
            "workspace actor source-set physical identity changed after admission: {}: {error}",
            root.path().display()
        )
    })
}

fn closed_index_component(value: &str) -> bool {
    !value.is_empty() && value.trim() == value && !value.bytes().any(|byte| byte == 0)
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn digest_index_component(digest: &mut Sha256, bytes: &[u8]) -> Result<(), String> {
    let length = u64::try_from(bytes.len())
        .map_err(|_| "actor-owned index work component is too large".to_string())?;
    digest.update(length.to_le_bytes());
    digest.update(bytes);
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkspaceActorRegistryError {
    Capacity { limit: usize },
    InvalidIdentity(String),
    Poisoned,
}

impl std::fmt::Display for WorkspaceActorRegistryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Capacity { limit } => {
                write!(
                    formatter,
                    "workspace actor capacity {limit} is fully leased"
                )
            }
            Self::InvalidIdentity(message) => formatter.write_str(message),
            Self::Poisoned => formatter.write_str("workspace actor registry is poisoned"),
        }
    }
}

impl std::error::Error for WorkspaceActorRegistryError {}

#[derive(Debug)]
pub(crate) struct WorkspaceActorRegistry {
    actors: Mutex<HashMap<WorkspaceIdentity, Weak<WorkspaceActor>>>,
    /// Bounded most-recently-used actors retained between invocations. The
    /// weak map above stays the admission authority; this set only keeps a
    /// few recent actors alive so the next call finds a trusted revision.
    warm: Mutex<VecDeque<WarmWorkspaceActor>>,
    warm_capacity: usize,
    warm_ttl: Duration,
    #[cfg(test)]
    max_active_override: Option<usize>,
}

#[derive(Debug)]
struct WarmWorkspaceActor {
    actor: Arc<WorkspaceActor>,
    last_used: Instant,
}

impl Default for WorkspaceActorRegistry {
    fn default() -> Self {
        Self {
            actors: Mutex::new(HashMap::new()),
            warm: Mutex::new(VecDeque::new()),
            warm_capacity: WARM_WORKSPACE_ACTORS,
            warm_ttl: WARM_WORKSPACE_ACTOR_TTL,
            #[cfg(test)]
            max_active_override: None,
        }
    }
}

impl WorkspaceActorRegistry {
    pub(crate) fn get_or_create<I>(
        &self,
        context: &WorkspaceContext,
        source_sets: I,
        provider_profile: &str,
    ) -> Result<Arc<WorkspaceActor>, WorkspaceActorRegistryError>
    where
        I: IntoIterator<Item = WorkspaceSourceSetInput>,
    {
        let identity = WorkspaceIdentity::new(context, source_sets, provider_profile)
            .map_err(WorkspaceActorRegistryError::InvalidIdentity)?;
        let now = Instant::now();
        let mut actors = self
            .actors
            .lock()
            .map_err(|_| WorkspaceActorRegistryError::Poisoned)?;
        // Lock order is always `actors` then `warm`.
        self.evict_expired_warm(now)?;
        actors.retain(|_, actor| actor.strong_count() > 0);
        if let Some(actor) = actors.get(&identity).and_then(Weak::upgrade) {
            if actor.retains_named_roots() {
                self.touch_warm(&actor, now)?;
                return Ok(actor);
            }
            // The named root was replaced while the actor idled. Forget the
            // warm copy; an actor still held by a live invocation keeps its
            // existing fail-closed behaviour.
            self.forget_warm(&identity)?;
            drop(actor);
            if let Some(active) = actors.get(&identity).and_then(Weak::upgrade) {
                return Ok(active);
            }
            actors.remove(&identity);
        }
        let max_active = self.max_active();
        if actors.len() >= max_active {
            // Idle warm actors are not active work: release them before the
            // registry refuses a distinct identity.
            self.clear_warm()?;
            actors.retain(|_, actor| actor.strong_count() > 0);
            if actors.len() >= max_active {
                return Err(WorkspaceActorRegistryError::Capacity { limit: max_active });
            }
        }
        let actor = Arc::new(
            WorkspaceActor::new(identity.clone(), context.clone())
                .map_err(WorkspaceActorRegistryError::InvalidIdentity)?,
        );
        actors.insert(identity, Arc::downgrade(&actor));
        self.touch_warm(&actor, now)?;
        // Warming the new actor may have released the oldest warm one; do not
        // leave its dead entry behind until the next admission.
        actors.retain(|_, actor| actor.strong_count() > 0);
        Ok(actor)
    }

    /// Releases every warm actor. A daemon that has requested its own restart
    /// keeps nothing alive beyond the invocations still executing.
    pub(crate) fn release_warm_actors(&self) -> Result<(), WorkspaceActorRegistryError> {
        let _actors = self
            .actors
            .lock()
            .map_err(|_| WorkspaceActorRegistryError::Poisoned)?;
        self.clear_warm()
    }

    /// Releases warm actors that idled past the TTL. The daemon calls this
    /// from its accept loop so retained descriptors do not outlive the idle
    /// window without a new admission.
    pub(crate) fn evict_idle_warm_actors(&self) -> Result<(), WorkspaceActorRegistryError> {
        let _actors = self
            .actors
            .lock()
            .map_err(|_| WorkspaceActorRegistryError::Poisoned)?;
        self.evict_expired_warm(Instant::now())
    }

    fn warm_lock(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, VecDeque<WarmWorkspaceActor>>, WorkspaceActorRegistryError>
    {
        self.warm
            .lock()
            .map_err(|_| WorkspaceActorRegistryError::Poisoned)
    }

    fn touch_warm(
        &self,
        actor: &Arc<WorkspaceActor>,
        now: Instant,
    ) -> Result<(), WorkspaceActorRegistryError> {
        if self.warm_capacity == 0 {
            return Ok(());
        }
        let mut warm = self.warm_lock()?;
        warm.retain(|entry| !Arc::ptr_eq(&entry.actor, actor));
        warm.push_back(WarmWorkspaceActor {
            actor: Arc::clone(actor),
            last_used: now,
        });
        while warm.len() > self.warm_capacity {
            warm.pop_front();
        }
        Ok(())
    }

    fn evict_expired_warm(&self, now: Instant) -> Result<(), WorkspaceActorRegistryError> {
        let mut warm = self.warm_lock()?;
        warm.retain(|entry| now.saturating_duration_since(entry.last_used) < self.warm_ttl);
        Ok(())
    }

    fn forget_warm(&self, identity: &WorkspaceIdentity) -> Result<(), WorkspaceActorRegistryError> {
        let mut warm = self.warm_lock()?;
        warm.retain(|entry| entry.actor.identity() != identity);
        Ok(())
    }

    fn clear_warm(&self) -> Result<(), WorkspaceActorRegistryError> {
        self.warm_lock()?.clear();
        Ok(())
    }

    #[cfg(test)]
    fn len(&self) -> Result<usize, String> {
        self.actors
            .lock()
            .map(|actors| {
                actors
                    .values()
                    .filter(|actor| actor.strong_count() > 0)
                    .count()
            })
            .map_err(|_| "workspace actor registry is poisoned".to_string())
    }

    #[cfg(test)]
    pub(crate) fn with_capacity_for_test(max_active: usize) -> Self {
        assert!(max_active > 0);
        Self {
            max_active_override: Some(max_active),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn with_warm_policy_for_test(warm_capacity: usize, warm_ttl: Duration) -> Self {
        Self {
            warm_capacity,
            warm_ttl,
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn with_capacity_and_warm_policy_for_test(
        max_active: usize,
        warm_capacity: usize,
        warm_ttl: Duration,
    ) -> Self {
        assert!(max_active > 0);
        Self {
            warm_capacity,
            warm_ttl,
            max_active_override: Some(max_active),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn warm_len_for_test(&self) -> Result<usize, String> {
        self.warm
            .lock()
            .map(|warm| warm.len())
            .map_err(|_| "workspace actor registry is poisoned".to_string())
    }

    fn max_active(&self) -> usize {
        #[cfg(test)]
        if let Some(max_active) = self.max_active_override {
            return max_active;
        }
        MAX_ACTIVE_WORKSPACE_ACTORS
    }

    #[cfg(test)]
    pub(crate) fn live_len_for_test(&self) -> Result<usize, String> {
        self.len()
    }

    #[cfg(test)]
    pub(crate) fn entry_len_for_test(&self) -> Result<usize, String> {
        self.actors
            .lock()
            .map(|actors| actors.len())
            .map_err(|_| "workspace actor registry is poisoned".to_string())
    }

    #[cfg(test)]
    pub(crate) fn poison_for_test(&self) {
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = self.actors.lock().unwrap();
            panic!("poison workspace actor registry for deterministic admission test");
        }));
        assert!(poisoned.is_err());
    }

    #[cfg(test)]
    fn prune_dead_for_test(&self) -> Result<(), String> {
        self.actors
            .lock()
            .map(|mut actors| actors.retain(|_, actor| actor.strong_count() > 0))
            .map_err(|_| "workspace actor registry is poisoned".to_string())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{
        set_apply_dry_run_after_confirmation_hook,
        set_revision_service_after_binding_validation_hook, ApplyAdmission, ApplyEffectDisposition,
        ApplyEffectReceipt, PreparedApplyBatch, WorkspaceActorRegistry,
        WorkspaceActorRegistryError, WorkspaceIdentity, WorkspaceSourceSetInput,
    };
    use crate::domain::address::QualifiedAddress;
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::ProviderDeadline;
    use crate::domain::events::DomainEventKind;
    use crate::domain::platform_profile::PlatformProfile;
    use crate::domain::project_sources::{SourceFormat, SourceProfile, SourceSetKind};
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::native_operations::apply::{
        ApplyStagedState, ApplyStagingErrorKind,
    };
    use crate::infrastructure::native_operations::event::{
        plan_event_implement_batch, EventImplementArgs, EventPlanError, PlannedApplyEffects,
    };
    use crate::infrastructure::platform::source_revision_fence::{
        expected_platform_fence_capability_for_test, FenceCapability, FenceOutcome,
        SourceRevisionFence,
    };
    use crate::infrastructure::platform::testing::{
        attempt_retained_directory_replacement_for_test,
        can_swap_named_child_behind_retained_handle_for_test, create_file_link_fixture_for_test,
        path_identity_for_test, set_unix_mode_for_test, FileLinkFixtureOutcome,
        RetainedDirectoryReplacementOutcome,
    };
    use crate::infrastructure::source_revision::SourceRevisionService;
    use crate::infrastructure::support_policy_evidence::SupportPolicyMode;
    use crate::infrastructure::support_policy_evidence::{
        set_support_policy_capture_hook, set_support_policy_read_chunk_hook_once,
        set_support_policy_validation_hook, SupportPolicyTestStateGuard,
    };
    use std::cell::Cell;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    fn source_input(name: &str, root: impl AsRef<Path>) -> WorkspaceSourceSetInput {
        WorkspaceSourceSetInput::new(
            name,
            root.as_ref(),
            SourceSetKind::Configuration,
            SourceFormat::PlatformXml,
            SourceProfile::platform_xml_8_3_27_format_2_20(),
        )
    }

    thread_local! {
        static SUPPORT_POLICY_ACTOR_TEST_NOW: Cell<Instant> = Cell::new(Instant::now());
    }

    fn support_policy_actor_test_now() -> Instant {
        SUPPORT_POLICY_ACTOR_TEST_NOW.get()
    }

    fn set_support_policy_actor_test_now(now: Instant) {
        SUPPORT_POLICY_ACTOR_TEST_NOW.set(now);
    }

    fn reset_support_policy_actor_test_now() {
        set_support_policy_actor_test_now(Instant::now());
    }

    #[test]
    fn index_coordination_has_no_raw_crate_level_constructor_or_owner_escape() {
        let shared_work = include_str!("../application/shared_work.rs");
        let actor = include_str!("workspace_actor.rs");
        assert!(
            !shared_work.contains("pub(crate) struct IndexWorkKey"),
            "raw IndexWorkKey remains constructible across the crate"
        );
        assert!(
            !shared_work.contains("IndexWorkOwner"),
            "raw IndexWorkOwner remains joinable outside the actor"
        );
        let raw_key_accessor = ["pub(crate) fn index_", "work_key"].concat();
        let raw_owner_accessor = ["pub(crate) fn index_", "work(&self)"].concat();
        assert!(
            !actor.contains(&raw_key_accessor) && !actor.contains(&raw_owner_accessor),
            "WorkspaceActor still exposes a movable key/owner pair"
        );
        assert!(
            actor.contains("pub(crate) fn join_index_work"),
            "the actor-owned exact revision join seam is absent"
        );
    }

    #[test]
    fn workspace_actor_serializes_mutation_publication() {
        let fixture = actor_fixture("serialized-mutations", &["src"]);
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let fence = fixture
            .actor
            .capture_revision(
                &binding,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (attempted_tx, attempted_rx) = mpsc::channel();
        let (first_release_tx, first_release_rx) = mpsc::channel();
        let (second_release_tx, second_release_rx) = mpsc::channel();

        let first_actor = Arc::clone(&fixture.actor);
        let first_fence = fence.clone();
        let first_entered = entered_tx.clone();
        let first = thread::spawn(move || {
            let lease = first_actor.begin_publication(
                &first_fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )?;
            first_entered.send("first").unwrap();
            first_release_rx.recv().unwrap();
            lease.publish(
                (),
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
        });
        assert_eq!(
            entered_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            "first"
        );

        let second_actor = Arc::clone(&fixture.actor);
        let second = thread::spawn(move || {
            attempted_tx.send(()).unwrap();
            let lease = second_actor.begin_publication(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )?;
            entered_tx.send("second").unwrap();
            second_release_rx.recv().unwrap();
            lease.publish(
                (),
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
        });
        attempted_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(
            entered_rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "a second mutation crossed the first actor publication lease"
        );
        first_release_tx.send(()).unwrap();
        assert_eq!(
            entered_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            "second"
        );
        second_release_tx.send(()).unwrap();
        first.join().unwrap().unwrap();
        second.join().unwrap().unwrap();
        fixture.cleanup();
    }

    #[test]
    fn workspace_actor_pre_cancelled_publication_does_not_wait_for_mutation_lane() {
        let fixture = actor_fixture("pre-cancelled-mutation-lane", &["src"]);
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let cancellation = CancellationToken::new();
        let fence = fixture
            .actor
            .capture_revision(
                &binding,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let owner = fixture
            .actor
            .begin_publication(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let waiter_cancellation = CancellationToken::new();
        waiter_cancellation.cancel();
        let waiter_actor = Arc::clone(&fixture.actor);
        let (result_tx, result_rx) = mpsc::channel();
        let waiter = thread::spawn(move || {
            let result = waiter_actor
                .begin_publication(
                    &fence,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &waiter_cancellation,
                )
                .map(drop);
            result_tx.send(result).unwrap();
        });

        let result_before_release = result_rx.recv_timeout(Duration::from_millis(250));
        let returned_before_release = result_before_release.is_ok();
        drop(owner);
        let result = result_before_release.unwrap_or_else(|_| {
            result_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("waiter must finish after the owner releases the lane")
        });
        waiter.join().unwrap();

        assert!(
            returned_before_release,
            "pre-cancelled publication waited for the held mutation lane"
        );
        assert!(result.unwrap_err().starts_with("cancelled:"));
        fixture.cleanup();
    }

    #[test]
    pub(crate) fn logical_read_publication_lane_wait_honors_existing_cancellation_and_deadline() {
        let fixture = actor_fixture("logical-read-lane-bounds", &["src"]);
        std::fs::write(
            fixture.roots[0].join("Configuration.xml"),
            "<Configuration/>",
        )
        .unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let logical_fence = fixture
            .actor
            .capture_logical_read_revision(
                &binding,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let legacy_fence = fixture
            .actor
            .capture_revision(
                &binding,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let owner = fixture
            .actor
            .begin_publication(
                &legacy_fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();

        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let cancelled = fixture.actor.publish_logical_read(
            std::slice::from_ref(&logical_fence),
            (),
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &cancellation,
        );
        assert!(cancelled.unwrap_err().starts_with("cancelled:"));

        let deadline = fixture.actor.publish_logical_read(
            &[logical_fence],
            (),
            ProviderDeadline::from_budget(Duration::ZERO),
            &CancellationToken::new(),
        );
        assert!(deadline.unwrap_err().ends_with("deadline exceeded"));
        drop(owner);
        fixture.cleanup();
    }

    #[test]
    fn workspace_actor_late_cancellation_stops_mutation_lane_wait() {
        let fixture = actor_fixture("late-cancelled-mutation-lane", &["src"]);
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let cancellation = CancellationToken::new();
        let fence = fixture
            .actor
            .capture_revision(
                &binding,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let owner = fixture
            .actor
            .begin_publication(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let waiter_cancellation = CancellationToken::new();
        let waiter_signal = waiter_cancellation.clone();
        let waiter_actor = Arc::clone(&fixture.actor);
        let (started_tx, started_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let waiter = thread::spawn(move || {
            started_tx.send(()).unwrap();
            let result = waiter_actor
                .begin_publication(
                    &fence,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &waiter_cancellation,
                )
                .map(drop);
            result_tx.send(result).unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(
            result_rx.recv_timeout(Duration::from_millis(50)).is_err(),
            "waiter unexpectedly crossed the held mutation lane"
        );
        waiter_signal.cancel();

        let result_before_release = result_rx.recv_timeout(Duration::from_millis(250));
        let returned_before_release = result_before_release.is_ok();
        drop(owner);
        let result = result_before_release.unwrap_or_else(|_| {
            result_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("waiter must finish after the owner releases the lane")
        });
        waiter.join().unwrap();

        assert!(
            returned_before_release,
            "late cancellation did not stop the held mutation-lane wait"
        );
        assert!(result.unwrap_err().starts_with("cancelled:"));
        fixture.cleanup();
    }

    #[test]
    fn workspace_actor_deadline_bounds_mutation_lane_wait() {
        for (label, budget) in [
            ("expired", Duration::ZERO),
            ("elapsed", Duration::from_millis(40)),
        ] {
            let fixture = actor_fixture(&format!("{label}-mutation-lane"), &["src"]);
            let binding = fixture
                .actor
                .bind_provider_root("src", &fixture.roots[0])
                .unwrap();
            let cancellation = CancellationToken::new();
            let fence = fixture
                .actor
                .capture_revision(
                    &binding,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &cancellation,
                )
                .unwrap();
            let owner = fixture
                .actor
                .begin_publication(
                    &fence,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &cancellation,
                )
                .unwrap();
            let waiter_actor = Arc::clone(&fixture.actor);
            let (result_tx, result_rx) = mpsc::channel();
            let waiter = thread::spawn(move || {
                let result = waiter_actor
                    .begin_publication(
                        &fence,
                        ProviderDeadline::from_budget(budget),
                        &CancellationToken::new(),
                    )
                    .map(drop);
                result_tx.send(result).unwrap();
            });

            let result_before_release = result_rx.recv_timeout(Duration::from_millis(250));
            let returned_before_release = result_before_release.is_ok();
            drop(owner);
            let result = result_before_release.unwrap_or_else(|_| {
                result_rx
                    .recv_timeout(Duration::from_secs(2))
                    .expect("waiter must finish after the owner releases the lane")
            });
            waiter.join().unwrap();

            assert!(
                returned_before_release,
                "{label} deadline did not bound the held mutation-lane wait"
            );
            assert!(result.unwrap_err().contains("deadline exceeded"));
            fixture.cleanup();
        }
    }

    #[test]
    fn prepared_apply_lock_deadline_is_not_reclassified_by_later_cancellation() {
        let fixture = actor_fixture("typed-deadline-lock-race", &["src"]);
        std::fs::write(fixture.roots[0].join("Module.bsl"), b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let cancellation = CancellationToken::new();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"before", b"after".to_vec())
            .unwrap();
        let mut prepared = admitted.prepare(state).unwrap();
        prepared.deadline = ProviderDeadline::from_budget(Duration::from_millis(40));
        let owner = fixture.actor.mutation_lane.hold_for_test();
        let cancel_after_deadline = cancellation.clone();
        crate::infrastructure::deadline_lock::set_after_deadline_error_hook_for_test(move || {
            std::thread::spawn(move || cancel_after_deadline.cancel())
                .join()
                .unwrap();
        });

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

        assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Deadline);
        drop(owner);
        fixture.cleanup();
    }

    #[test]
    fn workspace_actor_poisoned_mutation_lane_fails_closed_before_publication() {
        let fixture = actor_fixture("poisoned-mutation-lane", &["src"]);
        std::fs::write(fixture.roots[0].join("Module.bsl"), "test").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let fence_calls = Arc::new(AtomicUsize::new(0));
        let revision_service = Arc::new(
            SourceRevisionService::new_with_fence_for_test(
                fixture.actor.context(),
                &fixture.roots[0],
                fixture.actor.state_scope.clone(),
                Arc::new(CountingActorFence {
                    calls: Arc::clone(&fence_calls),
                }),
            )
            .unwrap(),
        );
        fixture
            .actor
            .install_source_revision_service_for_test(&binding, revision_service)
            .unwrap();
        let fence = fixture
            .actor
            .capture_revision(
                &binding,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let poison_actor = Arc::clone(&fixture.actor);
        let poison_fence = fence.clone();
        let poison = thread::spawn(move || {
            let _lease = poison_actor
                .begin_publication(
                    &poison_fence,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap();
            panic!("poison workspace actor mutation lane");
        });
        assert!(poison.join().is_err());
        let calls_after_poison = fence_calls.load(Ordering::Acquire);

        let result = fixture
            .actor
            .begin_publication(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .and_then(|lease| {
                lease.publish(
                    "POISONED-STAGED-TEXT",
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
            });
        let error = result.unwrap_err();

        assert_eq!(error, "workspace actor mutation lane is poisoned");
        assert!(!error.contains("POISONED-STAGED-TEXT"), "{error}");
        assert_eq!(
            fence_calls.load(Ordering::Acquire),
            calls_after_poison,
            "poisoned mutation lane executed a source revision fence"
        );
        fixture.cleanup();
    }

    struct CountingActorFence {
        calls: Arc<AtomicUsize>,
    }

    impl SourceRevisionFence for CountingActorFence {
        fn capability(&self) -> FenceCapability {
            FenceCapability::ProvenFast
        }

        fn flush(
            &self,
            _deadline: ProviderDeadline,
            _cancellation: &CancellationToken,
        ) -> Result<FenceOutcome, String> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            Ok(FenceOutcome::Proven {
                changed_paths: Vec::new(),
            })
        }
    }

    struct UnsupportedActorFence {
        flush_calls: Arc<AtomicUsize>,
    }

    impl SourceRevisionFence for UnsupportedActorFence {
        fn capability(&self) -> FenceCapability {
            FenceCapability::Unsupported
        }

        fn flush(
            &self,
            _deadline: ProviderDeadline,
            _cancellation: &CancellationToken,
        ) -> Result<FenceOutcome, String> {
            self.flush_calls.fetch_add(1, Ordering::AcqRel);
            Err("unsupported actor fence must never be flushed".to_string())
        }
    }

    #[test]
    fn actor_owned_legacy_revision_lifecycle_uses_retained_fallback_when_platform_fence_is_unsupported(
    ) {
        let fixture = actor_fixture("unsupported-actor-revision-fence", &["src"]);
        std::fs::write(fixture.roots[0].join("Module.bsl"), "test").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let flush_calls = Arc::new(AtomicUsize::new(0));
        let revision_service = Arc::new(
            SourceRevisionService::new_with_fence_for_test(
                fixture.actor.context(),
                &fixture.roots[0],
                fixture.actor.state_scope.clone(),
                Arc::new(UnsupportedActorFence {
                    flush_calls: Arc::clone(&flush_calls),
                }),
            )
            .unwrap(),
        );
        fixture
            .actor
            .install_source_revision_service_for_test(&binding, revision_service)
            .unwrap();
        let cancellation = CancellationToken::new();
        let revision = fixture
            .actor
            .capture_revision(
                &binding,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();

        let (producer_entered, producer_entered_wait) = mpsc::channel();
        let (_, index_work) = fixture
            .actor
            .join_index_work(&binding, &revision, "rlm", "bsl-1", move |_| {
                producer_entered.send(()).unwrap();
                Ok(())
            })
            .unwrap();
        producer_entered_wait
            .recv_timeout(Duration::from_secs(2))
            .expect("retained index producer must start within the bounded wait");
        assert!(matches!(
            index_work.wait_timeout(Duration::from_secs(2)),
            crate::application::shared_work::SharedWorkSnapshot::Ready(_)
        ));

        let publication = fixture
            .actor
            .begin_publication(
                &revision,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        assert_eq!(
            publication
                .publish(
                    "retained-publication",
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &cancellation,
                )
                .unwrap(),
            "retained-publication"
        );

        let second = super::WorkspaceActor::new(
            fixture.actor.identity.clone(),
            fixture.actor.context.clone(),
        )
        .unwrap();
        assert!(second.read(&binding, |_| Ok(())).is_err());
        assert!(second
            .begin_publication(
                &revision,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .is_err());
        assert_eq!(
            flush_calls.load(Ordering::Acquire),
            0,
            "an unsupported fence must not be flushed as a source of trust"
        );
        fixture.cleanup();
    }

    #[test]
    fn workspace_actor_allows_reads_to_overlap() {
        let fixture = actor_fixture("concurrent-reads", &["src"]);
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (first_release_tx, first_release_rx) = mpsc::channel();
        let (second_release_tx, second_release_rx) = mpsc::channel();

        let first_actor = Arc::clone(&fixture.actor);
        let first_binding = binding.clone();
        let first_entered = entered_tx.clone();
        let first = thread::spawn(move || {
            first_actor.read(&first_binding, |_| {
                first_entered.send("first").unwrap();
                first_release_rx.recv().unwrap();
                Ok(())
            })
        });
        assert_eq!(
            entered_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            "first"
        );

        let second_actor = Arc::clone(&fixture.actor);
        let second = thread::spawn(move || {
            second_actor.read(&binding, |_| {
                entered_tx.send("second").unwrap();
                second_release_rx.recv().unwrap();
                Ok(())
            })
        });
        assert_eq!(
            entered_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            "second",
            "reads through one actor must not share the exclusive mutation lane"
        );
        first_release_tx.send(()).unwrap();
        second_release_tx.send(()).unwrap();
        first.join().unwrap().unwrap();
        second.join().unwrap().unwrap();
        fixture.cleanup();
    }

    #[test]
    fn workspace_actor_rejects_a_stale_revision_before_publication() {
        let fixture = actor_fixture("revision-fence", &["src"]);
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let module = fixture.roots[0].join("Module.bsl");
        std::fs::write(&module, "Процедура До()\nКонецПроцедуры\n").unwrap();
        let fence = fixture
            .actor
            .capture_revision(
                &binding,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        std::fs::write(&module, "Процедура После()\nКонецПроцедуры\n").unwrap();
        let error = fixture
            .actor
            .begin_publication(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap_err();

        assert!(
            error.contains("source revision changed before publication"),
            "{error}"
        );
        fixture.cleanup();
    }

    #[test]
    fn workspace_actor_rejects_a_revision_changed_after_lease_before_publish() {
        let fixture = actor_fixture("revision-after-lease", &["src"]);
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let module = fixture.roots[0].join("Module.bsl");
        std::fs::write(&module, "Процедура До()\nКонецПроцедуры\n").unwrap();
        let cancellation = CancellationToken::new();
        let fence = fixture
            .actor
            .capture_revision(
                &binding,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let lease = fixture
            .actor
            .begin_publication(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        std::fs::write(&module, "Процедура После()\nКонецПроцедуры\n").unwrap();

        let error = lease
            .publish(
                "FOREIGN-STAGED-PROVIDER-TEXT",
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap_err();

        assert!(error.contains("source revision changed"), "{error}");
        assert!(!error.contains("FOREIGN-STAGED-PROVIDER-TEXT"), "{error}");
        fixture.cleanup();
    }

    #[test]
    fn workspace_actor_rejects_root_replacement_after_lease_before_publish() {
        let fixture = actor_fixture("root-after-lease", &["src"]);
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        std::fs::write(fixture.roots[0].join("Module.bsl"), "test").unwrap();
        let cancellation = CancellationToken::new();
        let fence = fixture
            .actor
            .capture_revision(
                &binding,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let lease = fixture
            .actor
            .begin_publication(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let displaced = fixture.root.join("src-displaced-after-lease");
        std::fs::rename(&fixture.roots[0], &displaced).unwrap();
        std::fs::create_dir_all(&fixture.roots[0]).unwrap();

        let error = lease
            .publish(
                "FOREIGN-STAGED-PROVIDER-TEXT",
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap_err();

        assert!(error.contains("physical identity changed"), "{error}");
        assert!(!error.contains("FOREIGN-STAGED-PROVIDER-TEXT"), "{error}");
        fixture.cleanup();
    }

    #[test]
    fn workspace_actor_publish_honors_cancellation_and_masks_staged_text() {
        let fixture = actor_fixture("cancelled-publish", &["src"]);
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        std::fs::write(fixture.roots[0].join("Module.bsl"), "test").unwrap();
        let cancellation = CancellationToken::new();
        let fence = fixture
            .actor
            .capture_revision(
                &binding,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let lease = fixture
            .actor
            .begin_publication(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        cancellation.cancel();

        let error = lease
            .publish(
                "FOREIGN-STAGED-PROVIDER-TEXT",
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap_err();

        assert!(error.starts_with("cancelled:"), "{error}");
        assert!(!error.contains("FOREIGN-STAGED-PROVIDER-TEXT"), "{error}");
        fixture.cleanup();
    }

    #[test]
    fn workspace_actor_publish_cancellation_bounds_revision_lane_contention() {
        let (error, returned_before_owner_release) =
            publish_while_revision_operation_is_held(Duration::from_secs(5), true);

        assert!(returned_before_owner_release);
        assert!(error.starts_with("cancelled:"), "{error}");
        assert!(!error.contains("FOREIGN-STAGED-PROVIDER-TEXT"), "{error}");
    }

    #[test]
    fn workspace_actor_publish_deadline_bounds_revision_lane_contention() {
        let (error, returned_before_owner_release) =
            publish_while_revision_operation_is_held(Duration::from_millis(40), false);

        assert!(returned_before_owner_release);
        assert!(error.contains("deadline exceeded"), "{error}");
        assert!(!error.contains("FOREIGN-STAGED-PROVIDER-TEXT"), "{error}");
    }

    fn publish_while_revision_operation_is_held(
        budget: Duration,
        cancel_before_publish: bool,
    ) -> (String, bool) {
        let fixture = actor_fixture("contended-publish-revision", &["src"]);
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        std::fs::write(fixture.roots[0].join("Module.bsl"), "test").unwrap();
        let cancellation = CancellationToken::new();
        let fence = fixture
            .actor
            .capture_revision(
                &binding,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let lease = fixture
            .actor
            .begin_publication(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let revision_service = fixture.actor.source_revision_service(&binding).unwrap();
        let owner_released = Arc::new(AtomicBool::new(false));
        let owner_released_signal = Arc::clone(&owner_released);
        let (owner_held_tx, owner_held_rx) = mpsc::channel();
        let (owner_release_tx, owner_release_rx) = mpsc::channel();
        let owner = thread::spawn(move || {
            let guard = revision_service.hold_operation_for_test();
            owner_held_tx.send(()).unwrap();
            owner_release_rx.recv().unwrap();
            owner_released_signal.store(true, Ordering::Release);
            drop(guard);
        });
        owner_held_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let (watchdog_done_tx, watchdog_done_rx) = mpsc::channel();
        let emergency_release = owner_release_tx.clone();
        let watchdog = thread::spawn(move || {
            if watchdog_done_rx
                .recv_timeout(Duration::from_millis(500))
                .is_err()
            {
                let _ = emergency_release.send(());
            }
        });
        if cancel_before_publish {
            cancellation.cancel();
        }

        let error = lease
            .publish(
                "FOREIGN-STAGED-PROVIDER-TEXT",
                ProviderDeadline::from_budget(budget),
                &cancellation,
            )
            .unwrap_err();
        let returned_before_owner_release = !owner_released.load(Ordering::Acquire);
        let _ = watchdog_done_tx.send(());
        let _ = owner_release_tx.send(());
        watchdog.join().unwrap();
        owner.join().unwrap();
        fixture.cleanup();
        (error, returned_before_owner_release)
    }

    #[test]
    fn retained_binding_rejects_a_root_replaced_by_another_source_set_link() {
        use crate::infrastructure::platform::testing::{
            create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let fixture = actor_fixture("root-link-replacement", &["A", "B"]);
        let relative = Path::new("CommonModules/Same/Ext/Module.bsl");
        for (root, contents) in fixture.roots.iter().zip(["root A", "root B"]) {
            std::fs::create_dir_all(root.join(relative).parent().unwrap()).unwrap();
            std::fs::write(root.join(relative), contents).unwrap();
        }
        let binding = fixture
            .actor
            .bind_provider_root("A", &fixture.roots[0])
            .unwrap();
        let displaced = fixture.root.join("A-displaced");
        std::fs::rename(&fixture.roots[0], &displaced).unwrap();
        match create_directory_link_fixture_for_test(&fixture.roots[1], &fixture.roots[0]).unwrap()
        {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
                let _ = std::fs::rename(&displaced, &fixture.roots[0]);
                fixture.cleanup();
                return;
            }
        }

        let result = fixture.actor.read_relative_file(&binding, relative, 1024);

        assert!(
            result.is_err(),
            "retained binding followed replacement: {result:?}"
        );
        fixture.cleanup();
    }

    #[test]
    fn retained_binding_rejects_a_same_path_directory_replacement() {
        let fixture = actor_fixture("root-directory-replacement", &["A"]);
        let relative = Path::new("Module.bsl");
        std::fs::write(fixture.roots[0].join(relative), "original").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("A", &fixture.roots[0])
            .unwrap();
        std::fs::rename(&fixture.roots[0], fixture.root.join("A-displaced")).unwrap();
        std::fs::create_dir_all(&fixture.roots[0]).unwrap();
        std::fs::write(fixture.roots[0].join(relative), "replacement").unwrap();

        let result = fixture.actor.read_relative_file(&binding, relative, 1024);

        assert!(
            result.is_err(),
            "retained binding accepted replacement: {result:?}"
        );
        fixture.cleanup();
    }

    #[test]
    fn descriptor_relative_read_never_follows_a_nested_directory_link() {
        use crate::infrastructure::platform::testing::{
            create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let fixture = actor_fixture("nested-read-link", &["A", "B"]);
        std::fs::write(fixture.roots[1].join("Secret.bsl"), "root B").unwrap();
        let nested = fixture.roots[0].join("Nested");
        match create_directory_link_fixture_for_test(&fixture.roots[1], &nested).unwrap() {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
                fixture.cleanup();
                return;
            }
        }
        let binding = fixture
            .actor
            .bind_provider_root("A", &fixture.roots[0])
            .unwrap();

        let result =
            fixture
                .actor
                .read_relative_file(&binding, Path::new("Nested/Secret.bsl"), 1024);

        assert!(result.is_err(), "nested link was followed: {result:?}");
        fixture.cleanup();
    }

    #[test]
    fn path_provider_discards_output_when_root_changes_mid_operation() {
        let fixture = actor_fixture("root-mid-operation-swap", &["A"]);
        let relative = Path::new("Module.bsl");
        std::fs::write(fixture.roots[0].join(relative), "before swap").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("A", &fixture.roots[0])
            .unwrap();
        let displaced = fixture.root.join("A-displaced");
        let replacement = fixture.roots[0].clone();

        let result = fixture.actor.read(&binding, |root| {
            let output =
                std::fs::read_to_string(root.join(relative)).map_err(|error| error.to_string())?;
            std::fs::rename(root, &displaced).map_err(|error| error.to_string())?;
            std::fs::create_dir_all(&replacement).map_err(|error| error.to_string())?;
            Ok(output)
        });

        assert!(
            result.is_err(),
            "provider published output after swap: {result:?}"
        );
        fixture.cleanup();
    }

    #[test]
    pub(crate) fn capabilities_do_not_cross_distinct_actor_instances_with_equal_identity() {
        let root = temp_root("actor-instance-capability");
        let source = root.join("src");
        std::fs::create_dir_all(&source).unwrap();
        let context = context(&root);
        let identity =
            WorkspaceIdentity::new(&context, [source_input("main", &source)], "profile").unwrap();
        let first = super::WorkspaceActor::new(identity.clone(), context.clone()).unwrap();
        let second = super::WorkspaceActor::new(identity, context).unwrap();
        let binding = first.bind_provider_root("main", &source).unwrap();
        let fence = first
            .capture_revision(
                &binding,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();

        assert!(second.read(&binding, |_| Ok(())).is_err());
        assert!(second
            .begin_publication(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    pub(crate) fn workspace_actor_capabilities_enforce_identity_physical_and_bounded_publication() {
        use crate::infrastructure::platform::testing::{
            create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let root = temp_root("active-capability-contract");
        let source = root.join("src");
        let outside = root.join("outside");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(source.join("Module.bsl"), "test").unwrap();
        std::fs::write(outside.join("Secret.bsl"), "outside").unwrap();
        let context = context(&root);
        assert!(WorkspaceIdentity::new(
            &context,
            [
                source_input("main", &source),
                source_input("alias", &source)
            ],
            "profile",
        )
        .is_err());
        let identity =
            WorkspaceIdentity::new(&context, [source_input("main", &source)], "profile").unwrap();
        let first = super::WorkspaceActor::new(identity.clone(), context.clone()).unwrap();
        let second = super::WorkspaceActor::new(identity, context).unwrap();
        let binding = first.bind_provider_root("main", &source).unwrap();
        let fence = first
            .capture_revision(
                &binding,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();

        assert!(second.read(&binding, |_| Ok(())).is_err());
        assert!(second
            .begin_publication(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .is_err());

        let publication = first
            .begin_publication(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        std::fs::write(source.join("Module.bsl"), "changed after lease").unwrap();
        assert!(publication
            .publish(
                "must not escape",
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .is_err());

        let nested = source.join("Nested");
        if matches!(
            create_directory_link_fixture_for_test(&outside, &nested).unwrap(),
            FileLinkFixtureOutcome::Created
        ) {
            assert!(first
                .read_relative_file(&binding, Path::new("Nested/Secret.bsl"), 1024)
                .is_err());
        }

        let displaced = root.join("src-displaced");
        let replacement = source.clone();
        let external = first.read(&binding, |root| {
            let output =
                std::fs::read(root.join("Module.bsl")).map_err(|error| error.to_string())?;
            std::fs::rename(root, &displaced).map_err(|error| error.to_string())?;
            std::fs::create_dir_all(&replacement).map_err(|error| error.to_string())?;
            Ok(output)
        });
        assert!(external.is_err());
        assert!(first
            .begin_publication(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .is_err());
        let _ = std::fs::remove_dir_all(root);

        workspace_actor_pre_cancelled_publication_does_not_wait_for_mutation_lane();
        workspace_actor_late_cancellation_stops_mutation_lane_wait();
        workspace_actor_deadline_bounds_mutation_lane_wait();

        for (budget, cancel_before_publish) in [
            (Duration::from_secs(5), true),
            (Duration::from_millis(40), false),
        ] {
            let (error, returned_before_owner_release) =
                publish_while_revision_operation_is_held(budget, cancel_before_publish);
            assert!(returned_before_owner_release, "{error}");
            assert!(!error.contains("FOREIGN-STAGED-PROVIDER-TEXT"), "{error}");
        }
    }

    #[test]
    pub(crate) fn duplicate_physical_root_names_are_rejected_as_ambiguous() {
        let root = temp_root("duplicate-physical-root");
        let source = root.join("src");
        std::fs::create_dir_all(&source).unwrap();
        let context = context(&root);

        let result = WorkspaceIdentity::new(
            &context,
            [
                source_input("main", &source),
                source_input("alias", &source),
            ],
            "profile",
        );

        assert!(result.is_err(), "duplicate physical root was accepted");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    pub(crate) fn duplicate_source_set_names_with_distinct_roots_are_rejected() {
        let root = temp_root("duplicate-source-set-name");
        let first_source = root.join("first");
        let second_source = root.join("second");
        std::fs::create_dir_all(&first_source).unwrap();
        std::fs::create_dir_all(&second_source).unwrap();
        let context = context(&root);

        let result = WorkspaceIdentity::new(
            &context,
            [
                source_input("main", &first_source),
                source_input("main", &second_source),
            ],
            "profile",
        );

        assert!(
            result.is_err(),
            "duplicate source-set name with distinct retained roots was accepted"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    pub(crate) fn remapped_names_and_profiles_do_not_share_revision_index_or_coordination_state() {
        let root = temp_root("state-scope-separation");
        let source = root.join("src");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("Module.bsl"), "test").unwrap();
        let context = context(&root);
        let deadline = || ProviderDeadline::from_budget(Duration::from_secs(5));
        let cancellation = CancellationToken::new();
        let mut state_roots = Vec::new();

        for (name, profile) in [
            ("main", "program"),
            ("renamed", "program"),
            ("main", "program-and-service"),
        ] {
            let identity =
                WorkspaceIdentity::new(&context, [source_input(name, &source)], profile).unwrap();
            let actor = super::WorkspaceActor::new(identity, context.clone()).unwrap();
            let binding = actor.bind_provider_root(name, &source).unwrap();
            actor
                .capture_revision(&binding, deadline(), &cancellation)
                .unwrap();
            let service = actor
                .index_service(
                    &binding,
                    &crate::infrastructure::workspace_index::SYSTEM_INDEX_RUNNER,
                )
                .unwrap();
            state_roots.push(
                service
                    .provider_state_root_for_test(&context, &source)
                    .unwrap(),
            );
        }
        let records = std::fs::read_dir(context.cache_root.join("source-revisions"))
            .unwrap()
            .count();

        assert_eq!(
            records, 3,
            "logical actor scopes collapsed persisted revisions"
        );
        assert_ne!(state_roots[0], state_roots[1]);
        assert_ne!(state_roots[0], state_roots[2]);
        assert_ne!(state_roots[1], state_roots[2]);

        let legacy_direct = crate::infrastructure::source_revision::SourceRevisionService::new_reconciling_for_test(
            &context,
            &source,
        )
        .unwrap();
        legacy_direct.snapshot(deadline(), &cancellation).unwrap();
        assert_eq!(
            std::fs::read_dir(context.cache_root.join("source-revisions"))
                .unwrap()
                .count(),
            records + 1,
            "direct legacy revision did not publish exactly one physical-scope record"
        );
        let legacy_identity = WorkspaceIdentity::new(
            &context,
            [WorkspaceSourceSetInput::new(
                "main",
                &source,
                SourceSetKind::Configuration,
                SourceFormat::Unknown,
                SourceProfile::legacy_workspace_service_compatibility(),
            )],
            "legacy-profile",
        )
        .unwrap();
        let legacy_actor =
            super::WorkspaceActor::with_legacy_runtime(legacy_identity, context.clone(), ())
                .unwrap();
        let legacy_binding = legacy_actor.bind_provider_root("main", &source).unwrap();
        legacy_actor
            .capture_revision(&legacy_binding, deadline(), &cancellation)
            .unwrap();
        assert_eq!(
            std::fs::read_dir(context.cache_root.join("source-revisions"))
                .unwrap()
                .count(),
            records + 1,
            "legacy adapter moved the v0.12 revision namespace"
        );
        let direct_index = crate::infrastructure::workspace_index::WorkspaceIndexService::new()
            .provider_state_root_for_test(&context, &source)
            .unwrap();
        let actor_index = legacy_actor
            .index_service(
                &legacy_binding,
                &crate::infrastructure::workspace_index::SYSTEM_INDEX_RUNNER,
            )
            .unwrap()
            .provider_state_root_for_test(&context, &source)
            .unwrap();
        assert_eq!(
            direct_index, actor_index,
            "legacy adapter moved the v0.12 index/provider namespace"
        );
        if let Some((first, second)) =
            crate::infrastructure::platform::filesystem::distinct_non_unicode_paths_for_test()
        {
            let identity = |source_root| WorkspaceIdentity {
                workspace_root: PathBuf::from("/workspace"),
                source_sets: vec![super::WorkspaceSourceSetIdentity {
                    name: "main".to_string(),
                    root: source_root,
                    kind: SourceSetKind::Configuration,
                    source_format: SourceFormat::PlatformXml,
                    source_profile: SourceProfile::platform_xml_8_3_27_format_2_20(),
                }],
                provider_profile: "profile".to_string(),
            };
            assert_ne!(
                identity(first).state_scope_digest().unwrap(),
                identity(second).state_scope_digest().unwrap(),
                "distinct native path identities collapsed actor state"
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn workspace_actor_exposes_no_raw_generic_runtime_payload() {
        let immutable_return = ["-> ", "&R"].concat();
        let mutable_return = ["-> ", "&mut R"].concat();
        let source = include_str!("workspace_actor.rs");

        assert!(
            !source.lines().any(|line| line.contains(&immutable_return)),
            "workspace actor exposes a raw immutable runtime payload"
        );
        assert!(
            !source.lines().any(|line| line.contains(&mutable_return)),
            "workspace actor exposes a raw mutable runtime payload"
        );
    }

    #[test]
    fn actor_state_scope_digest_is_fallible_and_bounded() {
        fn digest(identity: &WorkspaceIdentity) -> Result<String, String> {
            identity.state_scope_digest()
        }

        let identity = WorkspaceIdentity {
            workspace_root: PathBuf::from("/workspace"),
            source_sets: vec![super::WorkspaceSourceSetIdentity {
                name: "main".to_string(),
                root: PathBuf::from("/workspace/source"),
                kind: SourceSetKind::Configuration,
                source_format: SourceFormat::PlatformXml,
                source_profile: SourceProfile::platform_xml_8_3_27_format_2_20(),
            }],
            provider_profile: "profile".to_string(),
        };
        let digest = digest(&identity).unwrap();

        assert_eq!(digest.len(), 64);
        assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn actor_state_scope_distinguishes_native_non_unicode_paths() {
        let Some((first, second)) =
            crate::infrastructure::platform::filesystem::distinct_non_unicode_paths_for_test()
        else {
            return;
        };
        let identity = |root| WorkspaceIdentity {
            workspace_root: PathBuf::from("/workspace"),
            source_sets: vec![super::WorkspaceSourceSetIdentity {
                name: "main".to_string(),
                root,
                kind: SourceSetKind::Configuration,
                source_format: SourceFormat::PlatformXml,
                source_profile: SourceProfile::platform_xml_8_3_27_format_2_20(),
            }],
            provider_profile: "profile".to_string(),
        };

        assert_ne!(
            identity(first).state_scope_digest().unwrap(),
            identity(second).state_scope_digest().unwrap(),
            "distinct native non-Unicode roots collapsed to one actor scope"
        );
    }

    #[test]
    pub(crate) fn same_name_root_changed_kind_rotates_actor_and_state_scope() {
        let root = temp_root("typed-kind-identity");
        let source = root.join("src");
        std::fs::create_dir_all(&source).unwrap();
        let context = context(&root);
        let input = |kind| {
            WorkspaceSourceSetInput::new(
                "main",
                &source,
                kind,
                SourceFormat::PlatformXml,
                SourceProfile::platform_xml_8_3_27_format_2_20(),
            )
        };
        let configuration_identity =
            WorkspaceIdentity::new(&context, [input(SourceSetKind::Configuration)], "provider")
                .unwrap();
        let extension_identity =
            WorkspaceIdentity::new(&context, [input(SourceSetKind::Extension)], "provider")
                .unwrap();
        let registry = WorkspaceActorRegistry::default();
        let configuration = registry
            .get_or_create(&context, [input(SourceSetKind::Configuration)], "provider")
            .unwrap();
        let extension = registry
            .get_or_create(&context, [input(SourceSetKind::Extension)], "provider")
            .unwrap();

        let actor_was_reused = Arc::ptr_eq(&configuration, &extension);
        let scope_was_reused = configuration_identity.state_scope_digest().unwrap()
            == extension_identity.state_scope_digest().unwrap();
        assert!(
            !actor_was_reused && !scope_was_reused,
            "changed source kind reused actor={actor_was_reused}, state_scope={scope_was_reused}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    pub(crate) fn same_name_root_changed_format_or_platform_profile_rotates_actor() {
        let root = temp_root("typed-format-profile-identity");
        let source = root.join("src");
        std::fs::create_dir_all(&source).unwrap();
        let context = context(&root);
        let input = |source_format, source_profile| {
            WorkspaceSourceSetInput::new(
                "main",
                &source,
                SourceSetKind::Configuration,
                source_format,
                source_profile,
            )
        };
        let cases = [
            (
                "format",
                SourceFormat::Edt,
                SourceProfile::platform_xml_8_3_27_format_2_20(),
            ),
            (
                "platform",
                SourceFormat::PlatformXml,
                SourceProfile::TestPlatform8_3_28Format2_20,
            ),
            (
                "serialization",
                SourceFormat::PlatformXml,
                SourceProfile::TestPlatform8_3_27Format2_21,
            ),
        ];
        let baseline_identity = WorkspaceIdentity::new(
            &context,
            [input(
                SourceFormat::PlatformXml,
                SourceProfile::platform_xml_8_3_27_format_2_20(),
            )],
            "provider",
        )
        .unwrap();
        let registry = WorkspaceActorRegistry::default();
        let baseline = registry
            .get_or_create(
                &context,
                [input(
                    SourceFormat::PlatformXml,
                    SourceProfile::platform_xml_8_3_27_format_2_20(),
                )],
                "provider",
            )
            .unwrap();
        let mut aliases = Vec::new();
        for (label, source_format, source_profile) in cases {
            let changed_identity = WorkspaceIdentity::new(
                &context,
                [input(source_format, source_profile)],
                "provider",
            )
            .unwrap();
            let changed = registry
                .get_or_create(&context, [input(source_format, source_profile)], "provider")
                .unwrap();
            if Arc::ptr_eq(&baseline, &changed)
                || baseline_identity.state_scope_digest().unwrap()
                    == changed_identity.state_scope_digest().unwrap()
            {
                aliases.push(label);
            }
        }

        assert!(
            aliases.is_empty(),
            "planner-significant typed source fields aliased baseline identity: {aliases:?}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    pub(crate) fn warm_registry_reuses_the_same_actor_across_sequential_admissions() {
        let root = temp_root("warm-reuse");
        std::fs::create_dir_all(root.join("src")).unwrap();
        let registry = WorkspaceActorRegistry::default();
        let context = context(&root);
        let first = registry
            .get_or_create(&context, [source_input("main", root.join("src"))], "p")
            .unwrap();
        let first = {
            let weak = Arc::downgrade(&first);
            drop(first);
            weak
        };
        let second = registry
            .get_or_create(&context, [source_input("main", root.join("src"))], "p")
            .unwrap();
        let first = first
            .upgrade()
            .expect("a released actor must stay warm for the next sequential admission");
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(registry.warm_len_for_test().unwrap(), 1);
        assert_eq!(registry.live_len_for_test().unwrap(), 1);
    }

    #[test]
    pub(crate) fn warm_actor_expires_after_the_idle_ttl_and_is_rebuilt() {
        let root = temp_root("warm-ttl");
        std::fs::create_dir_all(root.join("src")).unwrap();
        let registry = WorkspaceActorRegistry::with_warm_policy_for_test(
            super::WARM_WORKSPACE_ACTORS,
            Duration::ZERO,
        );
        let context = context(&root);
        let first = registry
            .get_or_create(&context, [source_input("main", root.join("src"))], "p")
            .unwrap();
        let first = {
            let weak = Arc::downgrade(&first);
            drop(first);
            weak
        };
        registry.evict_idle_warm_actors().unwrap();
        assert_eq!(registry.warm_len_for_test().unwrap(), 0);
        assert_eq!(registry.live_len_for_test().unwrap(), 0);
        assert!(
            first.upgrade().is_none(),
            "an expired warm actor must be released, not handed to a later admission"
        );
        let second = registry
            .get_or_create(&context, [source_input("main", root.join("src"))], "p")
            .unwrap();
        assert_eq!(registry.warm_len_for_test().unwrap(), 1);
        drop(second);
    }

    #[test]
    pub(crate) fn warm_actor_whose_named_root_was_replaced_is_rebuilt() {
        let root = temp_root("warm-replaced-root");
        std::fs::create_dir_all(root.join("src")).unwrap();
        let registry = WorkspaceActorRegistry::default();
        let context = context(&root);
        let first = registry
            .get_or_create(&context, [source_input("main", root.join("src"))], "p")
            .unwrap();
        let first = {
            let weak = Arc::downgrade(&first);
            drop(first);
            weak
        };
        std::fs::rename(root.join("src"), root.join("src-old")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        assert!(
            first.upgrade().is_some(),
            "the stale actor is still warm before admission"
        );
        let second = registry
            .get_or_create(&context, [source_input("main", root.join("src"))], "p")
            .unwrap();
        assert!(
            first.upgrade().is_none(),
            "a warm actor whose root directory was replaced must be released and rebuilt"
        );
        assert!(second.retains_named_roots());
        assert_eq!(registry.warm_len_for_test().unwrap(), 1);
    }

    #[test]
    pub(crate) fn warm_actors_yield_capacity_to_a_distinct_identity() {
        let root = temp_root("warm-capacity");
        let registry = WorkspaceActorRegistry::with_capacity_for_test(2);
        let mut actors = Vec::new();
        for index in 0..3 {
            let workspace = root.join(format!("workspace-{index}"));
            std::fs::create_dir_all(workspace.join("src")).unwrap();
            let actor = registry
                .get_or_create(
                    &context(&workspace),
                    [source_input("main", workspace.join("src"))],
                    "p",
                )
                .unwrap_or_else(|error| {
                    panic!("warm actors must not consume admission capacity: {error}")
                });
            actors.push(Arc::downgrade(&actor));
        }
        assert!(registry.live_len_for_test().unwrap() <= 2);
        assert!(
            actors[2].upgrade().is_some(),
            "the newest admission stays warm after evicting older idle actors"
        );
    }

    #[test]
    pub(crate) fn workspace_actor_registry_keys_exact_identity_and_separates_worktrees_and_source_roots(
    ) {
        let root = temp_root("identity");
        let shared_git = root.join("repository/.git");
        let worktree_a = root.join("worktrees/a");
        let worktree_b = root.join("worktrees/b");
        std::fs::create_dir_all(&shared_git).unwrap();
        for worktree in [&worktree_a, &worktree_b] {
            std::fs::create_dir_all(worktree.join("src-a")).unwrap();
            std::fs::create_dir_all(worktree.join("src-b")).unwrap();
            std::fs::write(worktree.join("v8project.yaml"), "format: DESIGNER\n").unwrap();
            std::fs::write(
                worktree.join(".git"),
                format!("gitdir: {}\n", shared_git.display()),
            )
            .unwrap();
        }
        let registry = WorkspaceActorRegistry::default();
        let context_a = context(&worktree_a);
        let context_b = context(&worktree_b);
        let a = registry
            .get_or_create(
                &context_a,
                [
                    source_input("main", worktree_a.join("src-a")),
                    source_input("extension", worktree_a.join("src-b")),
                ],
                "bsl-ls:program",
            )
            .unwrap();
        let same_a = registry
            .get_or_create(
                &context_a,
                [
                    source_input("extension", worktree_a.join("src-b")),
                    source_input("main", worktree_a.join("src-a")),
                ],
                "bsl-ls:program",
            )
            .unwrap();
        let other_worktree = registry
            .get_or_create(
                &context_b,
                [
                    source_input("main", worktree_b.join("src-a")),
                    source_input("extension", worktree_b.join("src-b")),
                ],
                "bsl-ls:program",
            )
            .unwrap();
        let other_roots = registry
            .get_or_create(
                &context_a,
                [source_input("main", worktree_a.join("src-a"))],
                "bsl-ls:program",
            )
            .unwrap();
        let other_profile = registry
            .get_or_create(
                &context_a,
                [
                    source_input("main", worktree_a.join("src-a")),
                    source_input("extension", worktree_a.join("src-b")),
                ],
                "bsl-ls:program-and-service",
            )
            .unwrap();
        let remapped_names = registry
            .get_or_create(
                &context_a,
                [
                    source_input("main", worktree_a.join("src-b")),
                    source_input("extension", worktree_a.join("src-a")),
                ],
                "bsl-ls:program",
            )
            .unwrap();

        assert!(Arc::ptr_eq(&a, &same_a));
        assert!(!Arc::ptr_eq(&a, &other_worktree));
        assert!(!Arc::ptr_eq(&a, &other_roots));
        assert!(!Arc::ptr_eq(&a, &other_profile));
        assert!(!Arc::ptr_eq(&a, &remapped_names));
        assert_eq!(registry.len().unwrap(), 5);
        assert_ne!(
            a.identity().workspace_root(),
            other_worktree.identity().workspace_root()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn two_frontends_reuse_the_actor_for_one_canonical_worktree() {
        let root = temp_root("frontend-reuse");
        let source = root.join("src");
        let nested = root.join("frontend/cwd");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&nested).unwrap();
        let mut first_context = context(&root);
        first_context.cwd = root.clone();
        let mut second_context = context(&root.join("frontend/.."));
        second_context.cwd = nested;
        let registry = WorkspaceActorRegistry::default();

        let first = registry
            .get_or_create(
                &first_context,
                [source_input("main", &source)],
                "legacy-bsl-rlm",
            )
            .unwrap();
        let second = registry
            .get_or_create(
                &second_context,
                [source_input("main", root.join("frontend/../src"))],
                "legacy-bsl-rlm",
            )
            .unwrap();

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(registry.len().unwrap(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn daemon_actor_registry_prunes_dead_entries_and_bounds_sequential_roots() {
        let parent = temp_root("bounded-sequential");
        // The weak map is the admission authority; with the warm set disabled
        // every released actor must die and its entry must be pruned.
        let registry =
            WorkspaceActorRegistry::with_capacity_and_warm_policy_for_test(2, 0, Duration::ZERO);

        for index in 0..12 {
            let root = parent.join(format!("workspace-{index}"));
            let source = root.join("src");
            std::fs::create_dir_all(&source).unwrap();
            let actor = registry
                .get_or_create(
                    &context(&root),
                    [source_input("main", &source)],
                    "canonical-v0.13",
                )
                .unwrap();
            assert_eq!(registry.live_len_for_test().unwrap(), 1);
            drop(actor);
        }

        registry.prune_dead_for_test().unwrap();
        assert_eq!(registry.entry_len_for_test().unwrap(), 0);
        let _ = std::fs::remove_dir_all(parent);
    }

    #[test]
    fn daemon_actor_registry_rejects_only_when_all_capacity_entries_are_live() {
        let parent = temp_root("bounded-live");
        let registry = WorkspaceActorRegistry::with_capacity_for_test(2);
        let mut actors = Vec::new();
        for index in 0..2 {
            let root = parent.join(format!("workspace-{index}"));
            let source = root.join("src");
            std::fs::create_dir_all(&source).unwrap();
            actors.push(
                registry
                    .get_or_create(
                        &context(&root),
                        [source_input("main", &source)],
                        "canonical-v0.13",
                    )
                    .unwrap(),
            );
        }

        let rejected_root = parent.join("workspace-rejected");
        let rejected_source = rejected_root.join("src");
        std::fs::create_dir_all(&rejected_source).unwrap();
        assert_eq!(
            registry
                .get_or_create(
                    &context(&rejected_root),
                    [source_input("main", &rejected_source)],
                    "canonical-v0.13",
                )
                .unwrap_err(),
            WorkspaceActorRegistryError::Capacity { limit: 2 }
        );
        assert_eq!(registry.live_len_for_test().unwrap(), 2);

        drop(actors.pop());
        let admitted = registry
            .get_or_create(
                &context(&rejected_root),
                [source_input("main", &rejected_source)],
                "canonical-v0.13",
            )
            .unwrap();
        assert_eq!(registry.live_len_for_test().unwrap(), 2);
        drop(admitted);
        drop(actors);
        let _ = std::fs::remove_dir_all(parent);
    }

    #[test]
    fn active_alias_reuses_actor_and_dropped_actor_recreates_a_new_instance() {
        let root = temp_root("weak-alias-recreate");
        let source = root.join("src");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(root.join("nested")).unwrap();
        let registry = WorkspaceActorRegistry::with_capacity_and_warm_policy_for_test(
            1,
            super::WARM_WORKSPACE_ACTORS,
            Duration::ZERO,
        );
        let first = registry
            .get_or_create(
                &context(&root),
                [source_input("main", &source)],
                "canonical-v0.13",
            )
            .unwrap();
        let stale_binding = first.bind_provider_root("main", &source).unwrap();
        let alias = registry
            .get_or_create(
                &context(&root.join("nested/..")),
                [source_input("main", root.join("nested/../src"))],
                "canonical-v0.13",
            )
            .unwrap();
        assert!(Arc::ptr_eq(&first, &alias));

        drop(alias);
        drop(first);
        // Once the warm TTL has elapsed the released actor is gone and the
        // next admission builds a fresh instance with its own capability id.
        registry.evict_idle_warm_actors().unwrap();
        let replacement = registry
            .get_or_create(
                &context(&root),
                [source_input("main", &source)],
                "canonical-v0.13",
            )
            .unwrap();
        assert!(replacement.validate_binding(&stale_binding).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn daemon_actor_registry_is_bounded_weak_and_alias_safe() {
        daemon_actor_registry_prunes_dead_entries_and_bounds_sequential_roots();
        daemon_actor_registry_rejects_only_when_all_capacity_entries_are_live();
        active_alias_reuses_actor_and_dropped_actor_recreates_a_new_instance();
    }

    #[test]
    fn multiroot_provider_keeps_identical_relative_paths_bound_to_the_requesting_root() {
        let fixture = actor_fixture("multiroot", &["A", "B"]);
        let relative = Path::new("CommonModules/Same/Ext/Module.bsl");
        for (root, contents) in fixture.roots.iter().zip(["root A", "root B"]) {
            std::fs::create_dir_all(root.join(relative).parent().unwrap()).unwrap();
            std::fs::write(root.join(relative), contents).unwrap();
        }
        let binding_a = fixture
            .actor
            .bind_provider_root("A", &fixture.roots[0])
            .unwrap();
        let binding_b = fixture
            .actor
            .bind_provider_root("B", &fixture.roots[1])
            .unwrap();
        assert!(fixture
            .actor
            .bind_provider_root("A", &fixture.roots[1])
            .is_err());

        let from_a = String::from_utf8(
            fixture
                .actor
                .read_relative_file(&binding_a, relative, 1024)
                .unwrap(),
        )
        .unwrap();
        let from_b = String::from_utf8(
            fixture
                .actor
                .read_relative_file(&binding_b, relative, 1024)
                .unwrap(),
        )
        .unwrap();

        assert_eq!(from_a, "root A");
        assert_eq!(from_b, "root B");
        fixture.cleanup();
    }

    #[test]
    fn actor_bound_index_session_rejects_another_source_set_root() {
        let fixture = actor_fixture("index-root-binding", &["A", "B"]);
        let binding = fixture
            .actor
            .bind_provider_root("A", &fixture.roots[0])
            .unwrap();
        let runner = &crate::infrastructure::workspace_index::SYSTEM_INDEX_RUNNER;
        let service = fixture.actor.index_service(&binding, runner).unwrap();
        let args = serde_json::json!({
            "sourceDir": fixture.roots[1].display().to_string()
        })
        .as_object()
        .unwrap()
        .clone();

        let readiness = service.ready_index(fixture.actor.context(), &args);

        assert!(
            matches!(
                readiness,
                crate::infrastructure::workspace_index::IndexReadiness::Unavailable(ref message)
                    if message.contains("escaped its actor-bound source root")
            ),
            "{readiness:?}"
        );
        fixture.cleanup();
    }

    #[test]
    fn apply_admission_rejects_source_inside_cache() {
        let root = temp_root("source-inside-cache");
        let source = root.join("cache/src");
        let cache = root.join("cache");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: src\n    type: CONFIGURATION\n    path: cache/src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            "<MetaDataObject><Configuration/></MetaDataObject>",
        )
        .unwrap();
        let mut actor_context = context(&root);
        actor_context.cache_root = cache;
        let identity = WorkspaceIdentity::new(
            &actor_context,
            [source_input("src", &source)],
            "test-provider",
        )
        .unwrap();
        let actor = super::WorkspaceActor::new(identity, actor_context).unwrap();
        let binding = actor.bind_provider_root("src", &source).unwrap();

        let error = actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap_err();
        assert!(error.to_string().contains("overlap"), "{error}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn workspace_root_source_allows_exact_generated_cache_descendant() {
        let root = temp_root("workspace-root-source-cache-descendant");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: src\n    type: CONFIGURATION\n    path: .\n",
        )
        .unwrap();
        std::fs::write(
            root.join("Configuration.xml"),
            "<MetaDataObject><Configuration/></MetaDataObject>",
        )
        .unwrap();
        std::fs::write(root.join("Module.bsl"), b"source").unwrap();
        let actor_context = context(&root);
        let identity = WorkspaceIdentity::new(
            &actor_context,
            [source_input("src", &root)],
            "test-provider",
        )
        .unwrap();
        let actor = super::WorkspaceActor::new(identity, actor_context).unwrap();
        let binding = actor.bind_provider_root("src", &root).unwrap();

        let admitted = actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        let error = state
            .create(".build/unica/forged.json", b"forged".to_vec())
            .unwrap_err();
        assert_eq!(error.kind(), ApplyStagingErrorKind::ContainmentIdentity);
        let error = state
            .create("Nested/.build/forged.json", b"forged".to_vec())
            .unwrap_err();
        assert_eq!(error.kind(), ApplyStagingErrorKind::ContainmentIdentity);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn source_role_rejects_platform_equivalent_generated_components() {
        let root = temp_root("platform-equivalent-generated-source");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: src\n    type: CONFIGURATION\n    path: .\n",
        )
        .unwrap();
        std::fs::write(
            root.join("Configuration.xml"),
            "<MetaDataObject><Configuration/></MetaDataObject>",
        )
        .unwrap();
        let canonical_root = std::fs::canonicalize(&root).unwrap();
        if crate::infrastructure::platform::filesystem::host_filesystem_case_sensitive(
            &canonical_root,
        )
        .unwrap()
        {
            let _ = std::fs::remove_dir_all(root);
            return;
        }
        let actor_context = context(&root);
        let identity = WorkspaceIdentity::new(
            &actor_context,
            [source_input("src", &root)],
            "test-provider",
        )
        .unwrap();
        let actor = super::WorkspaceActor::new(identity, actor_context).unwrap();
        let binding = actor.bind_provider_root("src", &root).unwrap();
        let admitted = actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();

        for target in [".BUILD/unica/forged.json", "Nested/.BUILD/forged.json"] {
            let error = state
                .create(target, b"forged".to_vec())
                .expect_err("platform-equivalent generated component reached Source role");
            assert_eq!(error.kind(), ApplyStagingErrorKind::ContainmentIdentity);
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn workspace_root_source_and_missing_cache_publish_through_disjoint_shared_anchor() {
        let root = temp_root("workspace-root-shared-cache-anchor");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: src\n    type: CONFIGURATION\n    path: .\n",
        )
        .unwrap();
        std::fs::write(
            root.join("Configuration.xml"),
            "<MetaDataObject><Configuration/></MetaDataObject>",
        )
        .unwrap();
        std::fs::write(root.join("Module.bsl"), b"before").unwrap();
        let actor_context = context(&root);
        let identity = WorkspaceIdentity::new(
            &actor_context,
            [source_input("src", &root)],
            "test-provider",
        )
        .unwrap();
        let actor = super::WorkspaceActor::new(identity, actor_context).unwrap();
        let binding = actor.bind_provider_root("src", &root).unwrap();
        let admitted = actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"before", b"after".to_vec())
            .unwrap();

        actor
            .publish_prepared_apply(
                admitted
                    .prepare_with_cache_effects(
                        state,
                        &[crate::domain::events::DomainEvent::new(
                            crate::domain::events::DomainEventKind::MetadataChanged,
                            "Catalog.Products",
                        )],
                    )
                    .unwrap(),
            )
            .unwrap();

        assert_eq!(std::fs::read(root.join("Module.bsl")).unwrap(), b"after");
        assert!(root.join(".build/unica/state.json").is_file());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn retained_apply_closed_participant_contract_is_complete() {
        crate::infrastructure::native_operations::apply::tests::retained_transaction_roles_require_explicit_roots_and_cache_authority();
        crate::infrastructure::native_operations::apply::tests::arbitrary_second_transaction_cannot_masquerade_as_actor_cache_authority();
        crate::infrastructure::native_operations::apply::tests::closed_transaction_rejects_physical_alias_and_second_cache_participant();
        apply_admission_rejects_source_inside_cache();
        workspace_root_source_allows_exact_generated_cache_descendant();
        workspace_root_source_and_missing_cache_publish_through_disjoint_shared_anchor();
    }

    #[test]
    fn actor_scoped_logical_revision_service_keeps_the_platform_fence_capability() {
        let fixture = actor_fixture("actor-scoped-platform-fence", &["src"]);
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();

        assert_eq!(
            service.fence_capability_for_test(),
            expected_platform_fence_capability_for_test(&fixture.roots[0])
        );
        fixture.cleanup();
    }

    #[test]
    pub(crate) fn active_platform_actor_cannot_select_the_legacy_revision_corpus() {
        let fixture = actor_fixture("revision-active-legacy-bypass", &["src"]);
        let source = &fixture.roots[0];
        let package = source.join("XDTOPackages/Sample/Ext/Package.bin");
        std::fs::create_dir_all(package.parent().unwrap()).unwrap();
        std::fs::write(&package, b"before").unwrap();
        let identity = WorkspaceIdentity::new(
            &context(&fixture.root),
            [source_input("src", source)],
            "test-provider",
        )
        .unwrap();

        let bypass =
            super::WorkspaceActor::with_legacy_runtime(identity, context(&fixture.root), ());
        let mut failure = None;
        if let Ok(actor) = bypass {
            let binding = actor.bind_provider_root("src", source).unwrap();
            let service = actor.source_revision_service(&binding).unwrap();
            let root = binding.retained_root();
            let before = service
                .observe_retained_operation(
                    &root,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap()
                .revision_identity();
            std::fs::write(&package, b"after").unwrap();
            let after = service
                .observe_retained_operation(
                    &root,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap()
                .revision_identity();
            failure = Some(format!(
                "active Platform actor selected LegacyV12: Package.bin revisions {before} / {after}"
            ));
        }

        assert!(failure.is_none(), "{}", failure.unwrap_or_default());
        fixture.cleanup();
    }

    #[test]
    pub(crate) fn actor_revision_service_construction_retains_the_validated_root_across_substitution(
    ) {
        if !can_swap_named_child_behind_retained_handle_for_test() {
            return;
        }
        let fixture = actor_fixture("revision-service-root-substitution", &["src"]);
        let source = fixture.roots[0].clone();
        let displaced = fixture.root.join("src-displaced");
        let package_relative = Path::new("XDTOPackages/Sample/Ext/Package.bin");
        let package = source.join(package_relative);
        std::fs::create_dir_all(package.parent().unwrap()).unwrap();
        std::fs::write(&package, b"before").unwrap();
        let binding = fixture.actor.bind_provider_root("src", &source).unwrap();

        let source_for_race = source.clone();
        let displaced_for_race = displaced.clone();
        set_revision_service_after_binding_validation_hook(move || {
            std::fs::rename(&source_for_race, &displaced_for_race).unwrap();
            std::fs::create_dir_all(&source_for_race).unwrap();
            std::fs::write(source_for_race.join("Configuration.xml"), b"<replacement/>").unwrap();
        });
        let raced = fixture.actor.source_revision_service(&binding);

        std::fs::remove_dir_all(&source).unwrap();
        std::fs::rename(&displaced, &source).unwrap();
        let mut failures = Vec::new();
        if raced.is_ok() {
            failures.push(
                "root substitution registered a service for ambient replacement identity"
                    .to_string(),
            );
        }

        match fixture.actor.source_revision_service(&binding) {
            Ok(service) if raced.is_err() => {
                let root = binding.retained_root();
                let before = service
                    .observe_retained_operation(
                        &root,
                        ProviderDeadline::from_budget(Duration::from_secs(5)),
                        &CancellationToken::new(),
                    )
                    .unwrap()
                    .revision_identity();
                std::fs::write(&package, b"after").unwrap();
                let after = service
                    .observe_retained_operation(
                        &root,
                        ProviderDeadline::from_budget(Duration::from_secs(5)),
                        &CancellationToken::new(),
                    )
                    .unwrap()
                    .revision_identity();
                if before == after {
                    failures.push(
                        "restored actor service omitted Package.bin from its exact corpus"
                            .to_string(),
                    );
                }
            }
            Ok(_) => failures.push(
                "raced service remained registered after the original root was restored"
                    .to_string(),
            ),
            Err(error) => failures.push(format!(
                "validated retained root could not construct a service after restoration: {error}"
            )),
        }

        assert!(
            failures.is_empty(),
            "actor revision root authority failures: {failures:?}"
        );
        fixture.cleanup();
    }

    #[test]
    pub(crate) fn actor_revision_platform_resource_projection_matches_live_capture() {
        let fixture = actor_fixture("revision-package-projection", &["src"]);
        let package = fixture.roots[0].join("XDTOPackages/Sample/Ext/Package.bin");
        std::fs::create_dir_all(package.parent().unwrap()).unwrap();
        std::fs::write(&package, b"package-before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace(
                "XDTOPackages/Sample/Ext/Package.bin",
                b"package-before",
                b"package-after".to_vec(),
            )
            .unwrap();
        let result = fixture
            .actor
            .publish_prepared_apply(admitted.prepare(state).unwrap())
            .expect("classified Platform resource must publish through retained equality");
        let observed = fixture
            .actor
            .admit_apply(
                &binding,
                Some(result.rev()),
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();

        assert_eq!(std::fs::read(package).unwrap(), b"package-after");
        assert_eq!(observed.revision_identity(), result.rev());
        fixture.cleanup();
    }

    pub(crate) fn replacement_commit_at_entry_limit_survives_owned_backup() {
        let fixture = actor_fixture("revision-replacement-entry-limit", &["src"]);
        let relative = "XDTOPackages/Sample/Ext/Package.bin";
        let package = fixture.roots[0].join(relative);
        std::fs::create_dir_all(package.parent().unwrap()).unwrap();
        std::fs::write(&package, b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace(relative, b"before", b"after".to_vec())
            .unwrap();
        let result = fixture
            .actor
            .publish_prepared_apply(admitted.prepare(state).unwrap())
            .expect("journal-owned recovery must not consume final-tree entry capacity");
        let reproduced = fixture
            .actor
            .admit_apply(
                &binding,
                Some(result.rev()),
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();

        assert_eq!(std::fs::read(package).unwrap(), b"after");
        assert_eq!(reproduced.revision_identity(), result.rev());
        fixture.cleanup();
    }

    pub(crate) fn new_leaf_commit_at_entry_limit_survives_owned_backup() {
        let fixture = actor_fixture("revision-new-leaf-entry-limit", &["src"]);
        let first = "Catalogs/Items/Forms/Main/Ext/Form/Items/a.png";
        let second = "Catalogs/Items/Forms/Main/Ext/Form/Items/b.png";
        let first_path = fixture.roots[0].join(first);
        let second_path = fixture.roots[0].join(second);
        std::fs::create_dir_all(first_path.parent().unwrap()).unwrap();
        std::fs::write(&first_path, b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state.replace(first, b"before", b"after".to_vec()).unwrap();
        state.create(second, b"created".to_vec()).unwrap();
        let result = fixture
            .actor
            .publish_prepared_apply(admitted.prepare(state).unwrap())
            .expect("owned recovery must not make an exact-limit replace/create batch fail");
        let reproduced = fixture
            .actor
            .admit_apply(
                &binding,
                Some(result.rev()),
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();

        assert_eq!(std::fs::read(first_path).unwrap(), b"after");
        assert_eq!(std::fs::read(second_path).unwrap(), b"created");
        assert_eq!(reproduced.revision_identity(), result.rev());
        fixture.cleanup();
    }

    pub(crate) fn multiple_recoveries_across_parents_preserve_exact_entry_limit() {
        let fixture = actor_fixture("revision-multiple-recovery-entry-limit", &["src"]);
        let paths = [
            "Catalogs/Items/Forms/Main/Ext/Form/Items/a.png",
            "Catalogs/Items/Forms/Main/Ext/Form/Items/b.png",
            "XDTOPackages/Sample/Ext/Package.bin",
        ];
        for path in paths {
            let target = fixture.roots[0].join(path);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, b"before").unwrap();
        }
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        for path in paths {
            state
                .replace(path, b"before", format!("after-{path}").into_bytes())
                .unwrap();
        }
        let result = fixture
            .actor
            .publish_prepared_apply(admitted.prepare(state).unwrap())
            .expect("three journal recoveries must preserve the exact final-tree limit");
        let reproduced = fixture
            .actor
            .admit_apply(
                &binding,
                Some(result.rev()),
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();

        assert_eq!(reproduced.revision_identity(), result.rev());
        fixture.cleanup();
    }

    pub(crate) fn remove_create_batch_at_entry_limit_preserves_final_tree_accounting() {
        let fixture = actor_fixture("revision-remove-create-entry-limit", &["src"]);
        let first = "Catalogs/Items/Forms/Main/Ext/Form/Items/a.png";
        let second = "Catalogs/Items/Forms/Main/Ext/Form/Items/b.png";
        let created = "Catalogs/Items/Forms/Main/Ext/Form/Items/c.png";
        for path in [first, second] {
            let target = fixture.roots[0].join(path);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, b"before").unwrap();
        }
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state.remove(first, b"before").unwrap();
        state.create(created, b"created".to_vec()).unwrap();
        let result = fixture
            .actor
            .publish_prepared_apply(admitted.prepare(state).unwrap())
            .expect("one removal must fund one creation despite the live recovery sibling");
        let reproduced = fixture
            .actor
            .admit_apply(
                &binding,
                Some(result.rev()),
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();

        assert!(!fixture.roots[0].join(first).exists());
        assert_eq!(
            std::fs::read(fixture.roots[0].join(created)).unwrap(),
            b"created"
        );
        assert_eq!(reproduced.revision_identity(), result.rev());
        fixture.cleanup();
    }

    fn retained_apply_recovery_path(parent: &Path) -> PathBuf {
        std::fs::read_dir(parent)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .and_then(std::ffi::OsStr::to_str)
                    .is_some_and(|name| name.starts_with(".unica-apply-"))
            })
            .expect("source publication must retain one recovery sibling")
    }

    #[test]
    pub(crate) fn actor_revision_recovery_identity_swap_is_rejected_before_revision_install() {
        let fixture = actor_fixture("revision-recovery-identity-swap", &["src"]);
        let relative = "XDTOPackages/Sample/Ext/Package.bin";
        let target = fixture.roots[0].join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();
        let machine_before = service.machine_state_for_test();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace(relative, b"before", b"after".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        let parent = target.parent().unwrap().to_path_buf();
        let moved = parent.join("recovery-moved-aside");
        let hook_parent = parent.clone();
        let hook_moved = moved.clone();
        crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_revision_validation_hook(
            move || {
                let recovery = retained_apply_recovery_path(&hook_parent);
                std::fs::rename(&recovery, &hook_moved).unwrap();
                std::fs::write(&recovery, b"decoy").unwrap();
            },
        );

        let error = fixture
            .actor
            .publish_prepared_apply(prepared)
            .expect_err("a same-name recovery decoy returned a receipt");

        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::RollbackIncomplete
        );
        assert!(error.contains("recovery"), "{error}");
        assert_eq!(service.machine_state_for_test(), machine_before);
        assert!(
            moved.exists(),
            "identity-bound recovery evidence was erased"
        );
        fixture.cleanup();
    }

    #[test]
    pub(crate) fn actor_revision_recovery_hard_link_alias_is_never_discounted_or_restored() {
        let fixture = actor_fixture("revision-recovery-hard-link", &["src"]);
        let relative = "XDTOPackages/Sample/Ext/Package.bin";
        let target = fixture.roots[0].join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();
        let machine_before = service.machine_state_for_test();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace(relative, b"before", b"after".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        let parent = target.parent().unwrap().to_path_buf();
        let alias = fixture.root.join("recovery-alias.bin");
        let hook_parent = parent.clone();
        let hook_alias = alias.clone();
        crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_revision_validation_hook(
            move || {
                let recovery = retained_apply_recovery_path(&hook_parent);
                std::fs::hard_link(recovery, &hook_alias).unwrap();
            },
        );

        let error = fixture
            .actor
            .publish_prepared_apply(prepared)
            .expect_err("a hard-linked recovery returned a receipt");

        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::RollbackIncomplete
        );
        assert!(error.contains("hard-link"), "{error}");
        assert_eq!(service.machine_state_for_test(), machine_before);
        assert_eq!(std::fs::read(alias).unwrap(), b"before");
        fixture.cleanup();
    }

    pub(crate) fn exact_limit_late_failure_reaches_phase_and_rolls_back_without_receipt() {
        use crate::infrastructure::native_operations::compile_transaction::RetainedApplyFailpoint;

        for (label, failpoint) in [
            ("revision-record", RetainedApplyFailpoint::RevisionRecord),
            ("state-marker", RetainedApplyFailpoint::StateMarker),
            (
                "after-postimages",
                RetainedApplyFailpoint::AfterAllPostimages,
            ),
        ] {
            let fixture = actor_fixture(&format!("revision-exact-limit-late-{label}"), &["src"]);
            let relative = "XDTOPackages/Sample/Ext/Package.bin";
            let target = fixture.roots[0].join(relative);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(&target, b"before").unwrap();
            let binding = fixture
                .actor
                .bind_provider_root("src", &fixture.roots[0])
                .unwrap();
            let service = fixture.actor.source_revision_service(&binding).unwrap();
            let admitted = fixture
                .actor
                .admit_apply(
                    &binding,
                    None,
                    false,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap();
            let mut state = admitted.staged_state().unwrap();
            state
                .replace(relative, b"before", b"after".to_vec())
                .unwrap();
            let prepared = admitted.prepare(state).unwrap();
            let source_before = snapshot_tree(&fixture.roots[0]);
            let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
            let machine_before = service.machine_state_for_test();
            crate::infrastructure::native_operations::compile_transaction::set_retained_apply_failpoint(
                failpoint,
            );

            let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

            assert!(
                error.contains("injected retained apply failure"),
                "{label} stopped before its selected late phase: {error}"
            );
            assert_eq!(snapshot_tree(&fixture.roots[0]), source_before, "{label}");
            assert_eq!(
                snapshot_tree(&fixture.root.join(".build/unica")),
                cache_before,
                "{label}"
            );
            assert_eq!(service.machine_state_for_test(), machine_before, "{label}");
            fixture.cleanup();
        }
    }

    pub(crate) fn revision_transient_spoofs_still_consume_capacity() {
        for foreign_name in [".unica-apply-spoof", "ordinary-ignored.bin"] {
            let fixture = actor_fixture(
                &format!("revision-transient-spoof-{foreign_name}"),
                &["src"],
            );
            let relative = "XDTOPackages/Sample/Ext/Package.bin";
            let target = fixture.roots[0].join(relative);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(&target, b"before").unwrap();
            let binding = fixture
                .actor
                .bind_provider_root("src", &fixture.roots[0])
                .unwrap();
            let admitted = fixture
                .actor
                .admit_apply(
                    &binding,
                    None,
                    false,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap();
            let mut state = admitted.staged_state().unwrap();
            state
                .replace(relative, b"before", b"after".to_vec())
                .unwrap();
            let prepared = admitted.prepare(state).unwrap();
            let foreign = target.parent().unwrap().join(foreign_name);
            let hook_foreign = foreign.clone();
            crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_revision_validation_hook(
                move || std::fs::write(hook_foreign, b"foreign").unwrap(),
            );

            let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

            assert_eq!(
                error.kind(),
                super::ApplyPublicationErrorKind::ProviderPostvalidation,
                "{foreign_name}: {error}"
            );
            assert_eq!(std::fs::read(&target).unwrap(), b"before");
            assert_eq!(std::fs::read(&foreign).unwrap(), b"foreign");
            fixture.cleanup();
        }
    }

    pub(crate) fn revision_transient_create_only_and_restart_are_exact() {
        let fixture = actor_fixture("revision-transient-create-only", &["src"]);
        let relative = "XDTOPackages/Sample/Ext/Package.bin";
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state.create(relative, b"created".to_vec()).unwrap();
        let result = fixture
            .actor
            .publish_prepared_apply(admitted.prepare(state).unwrap())
            .expect("create-only publication must issue no recovery allowance");

        let restart_context = context(&fixture.root);
        let restart_identity = WorkspaceIdentity::new(
            &restart_context,
            [source_input("src", &fixture.roots[0])],
            "test-provider",
        )
        .unwrap();
        let restarted = super::WorkspaceActor::new(restart_identity, restart_context).unwrap();
        let restarted_binding = restarted
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let reproduced = restarted
            .admit_apply(
                &restarted_binding,
                Some(result.rev()),
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();

        assert_eq!(reproduced.revision_identity(), result.rev());
        assert_eq!(
            std::fs::read(fixture.roots[0].join(relative)).unwrap(),
            b"created"
        );
        fixture.cleanup();
    }

    pub(crate) fn revision_transient_cleanup_failure_does_not_persist_authority() {
        let fixture = actor_fixture("revision-transient-cleanup-lifetime", &["src"]);
        let relative = "XDTOPackages/Sample/Ext/Package.bin";
        let target = fixture.roots[0].join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace(relative, b"before", b"after".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        let parent = target.parent().unwrap().to_path_buf();
        let moved = parent.join("recovery-left-after-cleanup");
        let hook_parent = parent.clone();
        let hook_moved = moved.clone();
        crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_post_validation_hook(
            move || {
                let recovery = retained_apply_recovery_path(&hook_parent);
                std::fs::rename(&recovery, &hook_moved).unwrap();
                std::fs::write(recovery, b"decoy").unwrap();
            },
        );

        let result = fixture
            .actor
            .publish_prepared_apply(prepared)
            .expect("cleanup failure occurs after revision installation");

        assert_eq!(result.cleanup_diagnostics().len(), 1);
        assert!(moved.exists());
        assert!(fixture
            .actor
            .admit_apply(
                &binding,
                Some(result.rev()),
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .is_err());

        let restart_context = context(&fixture.root);
        let restart_identity = WorkspaceIdentity::new(
            &restart_context,
            [source_input("src", &fixture.roots[0])],
            "test-provider",
        )
        .unwrap();
        let restarted = super::WorkspaceActor::new(restart_identity, restart_context).unwrap();
        let restarted_binding = restarted
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        assert!(restarted
            .admit_apply(
                &restarted_binding,
                Some(result.rev()),
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .is_err());
        fixture.cleanup();
    }

    #[test]
    pub(crate) fn revision_transient_stop_causes_preserve_rollback() {
        for phase in [
            "before-enumeration",
            "after-enumeration",
            "between-captures",
        ] {
            let fixture = actor_fixture(&format!("revision-transient-cancel-{phase}"), &["src"]);
            let relative = "XDTOPackages/Sample/Ext/Package.bin";
            let target = fixture.roots[0].join(relative);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(&target, b"before").unwrap();
            let binding = fixture
                .actor
                .bind_provider_root("src", &fixture.roots[0])
                .unwrap();
            let service = fixture.actor.source_revision_service(&binding).unwrap();
            let cancellation = CancellationToken::new();
            let admitted = fixture
                .actor
                .admit_apply(
                    &binding,
                    None,
                    false,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &cancellation,
                )
                .unwrap();
            let mut state = admitted.staged_state().unwrap();
            state
                .replace(relative, b"before", b"after".to_vec())
                .unwrap();
            let prepared = admitted.prepare(state).unwrap();
            let source_before = snapshot_tree(&fixture.roots[0]);
            let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
            let machine_before = service.machine_state_for_test();
            let _scan_guard = match phase {
                "before-enumeration" => {
                    let cancel = cancellation.clone();
                    crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_revision_validation_hook(
                        move || cancel.cancel(),
                    );
                    None
                }
                "after-enumeration" => {
                    let cancel = cancellation.clone();
                    Some(crate::infrastructure::source_revision::set_retained_scan_test_mutation(
                        crate::infrastructure::source_revision::RetainedScanTestMutationPoint::AfterDirectoryEnumeration,
                        move || cancel.cancel(),
                    ))
                }
                "between-captures" => {
                    let cancel = cancellation.clone();
                    let scans = Arc::new(std::sync::atomic::AtomicUsize::new(0));
                    let observed = Arc::clone(&scans);
                    Some(crate::infrastructure::source_revision::set_repeating_retained_scan_test_mutation(
                        crate::infrastructure::source_revision::RetainedScanTestMutationPoint::ScanStart,
                        move || {
                            if observed.fetch_add(1, Ordering::AcqRel) == 1 {
                                cancel.cancel();
                            }
                        },
                    ))
                }
                _ => unreachable!(),
            };

            let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

            assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Cancelled);
            assert_eq!(snapshot_tree(&fixture.roots[0]), source_before, "{phase}");
            assert_eq!(
                snapshot_tree(&fixture.root.join(".build/unica")),
                cache_before,
                "{phase}"
            );
            assert_eq!(service.machine_state_for_test(), machine_before, "{phase}");
            fixture.cleanup();
        }

        let fixture = actor_fixture("revision-transient-deadline", &["src"]);
        let relative = "XDTOPackages/Sample/Ext/Package.bin";
        let target = fixture.roots[0].join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace(relative, b"before", b"after".to_vec())
            .unwrap();
        let mut prepared = admitted.prepare(state).unwrap();
        prepared.deadline = ProviderDeadline::from_budget(Duration::from_millis(100));
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        let machine_before = service.machine_state_for_test();
        let _scan_guard = crate::infrastructure::source_revision::set_retained_scan_test_mutation(
            crate::infrastructure::source_revision::RetainedScanTestMutationPoint::AfterDirectoryEnumeration,
            || std::thread::sleep(Duration::from_millis(150)),
        );

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

        assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Deadline);
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        assert_eq!(service.machine_state_for_test(), machine_before);
        fixture.cleanup();
    }

    #[test]
    pub(crate) fn actor_revision_external_resource_drift_rotates_subsequent_admission() {
        let content_rows = [
            ("xdto", "XDTOPackages/Sample/Ext/Package.bin"),
            ("support", "Ext/ParentConfigurations.bin"),
            (
                "template-bin",
                "Catalogs/Items/Templates/Print/Ext/Template.bin",
            ),
            (
                "template-txt",
                "Catalogs/Items/Templates/Text/Ext/Template.txt",
            ),
            ("help-html", "Catalogs/Items/Ext/Help/ru.html"),
            (
                "form-png",
                "Catalogs/Items/Forms/Main/Ext/Form/Items/logo.png",
            ),
            ("dcs-xml", "Reports/Sales/Templates/Dcs/Ext/Template.xml"),
            ("mxl-xml", "Reports/Sales/Templates/Print/Ext/Template.xml"),
            ("form-xml", "Catalogs/Items/Forms/Main/Ext/Form.xml"),
            ("form-bsl", "Catalogs/Items/Forms/Main/Ext/Form/Module.bsl"),
            ("role-xml", "Roles/Seller/Ext/Rights.xml"),
        ];
        let mut unchanged = Vec::new();
        for (label, relative) in content_rows {
            let fixture = actor_fixture(&format!("revision-drift-{label}"), &["src"]);
            let path = fixture.roots[0].join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"before").unwrap();
            let binding = fixture
                .actor
                .bind_provider_root("src", &fixture.roots[0])
                .unwrap();
            let before = fixture
                .actor
                .admit_apply(
                    &binding,
                    None,
                    true,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap()
                .revision_identity();
            std::fs::write(&path, b"after").unwrap();
            let after = fixture
                .actor
                .admit_apply(
                    &binding,
                    None,
                    true,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap()
                .revision_identity();
            if before == after {
                unchanged.push(label);
            }
            fixture.cleanup();
        }

        for (label, initially_present) in [("vendor-add", false), ("vendor-remove", true)] {
            let fixture = actor_fixture(&format!("revision-drift-{label}"), &["src"]);
            let vendor = fixture.roots[0].join("Ext/ParentConfigurations/Vendor.cf");
            std::fs::create_dir_all(vendor.parent().unwrap()).unwrap();
            if initially_present {
                std::fs::write(&vendor, b"vendor bytes are not hashed").unwrap();
            }
            let binding = fixture
                .actor
                .bind_provider_root("src", &fixture.roots[0])
                .unwrap();
            let before = fixture
                .actor
                .admit_apply(
                    &binding,
                    None,
                    true,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap()
                .revision_identity();
            if initially_present {
                std::fs::write(&vendor, b"different vendor bytes are still not hashed").unwrap();
                let after_byte_rewrite = fixture
                    .actor
                    .admit_apply(
                        &binding,
                        None,
                        true,
                        ProviderDeadline::from_budget(Duration::from_secs(5)),
                        &CancellationToken::new(),
                    )
                    .unwrap()
                    .revision_identity();
                assert_eq!(
                    before, after_byte_rewrite,
                    "presence-only vendor bytes rotated the revision"
                );
                std::fs::remove_file(&vendor).unwrap();
            } else {
                std::fs::write(&vendor, b"vendor bytes are not hashed").unwrap();
            }
            let after = fixture
                .actor
                .admit_apply(
                    &binding,
                    None,
                    true,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap()
                .revision_identity();
            if before == after {
                unchanged.push(label);
            }
            fixture.cleanup();
        }

        assert!(
            unchanged.is_empty(),
            "resource drift omitted from actor revision: {unchanged:?}"
        );
    }

    #[test]
    pub(crate) fn actor_revision_policy_migrates_old_scoped_record_once_then_is_restart_stable() {
        let fixture = actor_fixture("revision-record-migration", &["src"]);
        let package = fixture.roots[0].join("XDTOPackages/Sample/Ext/Package.bin");
        std::fs::create_dir_all(package.parent().unwrap()).unwrap();
        std::fs::write(&package, b"package-before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let old = crate::infrastructure::source_revision::seed_observed_revision_record_for_test(
            &fixture.actor.source_revision_service(&binding).unwrap(),
            &binding.retained_root(),
        )
        .unwrap();

        let rebuilt_identity = WorkspaceIdentity::new(
            &context(&fixture.root),
            [source_input("src", &fixture.roots[0])],
            "test-provider",
        )
        .unwrap();
        let rebuilt =
            Arc::new(super::WorkspaceActor::new(rebuilt_identity, context(&fixture.root)).unwrap());
        let rebuilt_binding = rebuilt
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let migrated = rebuilt
            .admit_apply(
                &rebuilt_binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap()
            .revision_identity();

        let admitted = rebuilt
            .admit_apply(
                &rebuilt_binding,
                Some(&migrated),
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace(
                "XDTOPackages/Sample/Ext/Package.bin",
                b"package-before",
                b"package-after".to_vec(),
            )
            .unwrap();
        let committed = rebuilt
            .publish_prepared_apply(admitted.prepare(state).unwrap())
            .map(|result| result.rev().to_string());

        let mut failures = Vec::new();
        if migrated == old {
            failures.push(
                "old scoped record did not rotate for newly classified Package.bin".to_string(),
            );
        }
        match committed {
            Ok(committed) => {
                let final_identity = WorkspaceIdentity::new(
                    &context(&fixture.root),
                    [source_input("src", &fixture.roots[0])],
                    "test-provider",
                )
                .unwrap();
                let final_actor = Arc::new(
                    super::WorkspaceActor::new(final_identity, context(&fixture.root)).unwrap(),
                );
                let final_binding = final_actor
                    .bind_provider_root("src", &fixture.roots[0])
                    .unwrap();
                let after_restart = final_actor
                    .admit_apply(
                        &final_binding,
                        None,
                        true,
                        ProviderDeadline::from_budget(Duration::from_secs(5)),
                        &CancellationToken::new(),
                    )
                    .unwrap()
                    .revision_identity();
                if committed != after_restart {
                    failures.push(format!(
                        "committed revision was not restart-stable: {committed} != {after_restart}"
                    ));
                }
            }
            Err(error) => failures.push(format!("classified Package.bin commit failed: {error}")),
        }

        assert!(
            failures.is_empty(),
            "revision migration failures: {failures:?}"
        );
        fixture.cleanup();
    }

    #[test]
    pub(crate) fn actor_revision_unknown_staged_artifact_is_rejected_before_publication() {
        let fixture = actor_fixture("revision-unknown-stage", &["src"]);
        let target = fixture.roots[0].join("Loose/random.bin");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Loose/random.bin", b"before", b"after".to_vec())
            .unwrap();
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        let machine_before = service.machine_state_for_test();

        let error = match admitted.prepare(state) {
            Err(error) => error,
            Ok(_) => panic!("unknown staged artifact crossed revision preparation"),
        };

        assert_eq!(error.kind(), ApplyStagingErrorKind::Invariant);
        assert!(
            error.to_string().contains("revision artifact policy"),
            "{error}"
        );
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        assert_eq!(service.machine_state_for_test(), machine_before);
        fixture.cleanup();
    }

    #[test]
    pub(crate) fn actor_revision_lookalike_resource_is_rejected_before_publication() {
        let fixture = actor_fixture("revision-lookalike-stage", &["src"]);
        let relative = "Loose/Templates/Junk/Ext/Template.bin";
        let target = fixture.roots[0].join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace(relative, b"before", b"after".to_vec())
            .unwrap();
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        let machine_before = service.machine_state_for_test();

        let error = match admitted.prepare(state) {
            Err(error) => error,
            Ok(_) => panic!("lookalike resource crossed revision preparation"),
        };

        assert_eq!(error.kind(), ApplyStagingErrorKind::Invariant);
        assert!(
            error.to_string().contains("revision artifact policy"),
            "{error}"
        );
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        assert_eq!(service.machine_state_for_test(), machine_before);
        fixture.cleanup();
    }

    #[test]
    pub(crate) fn actor_revision_late_failure_rolls_back_targeted_resource_without_receipt() {
        use crate::infrastructure::native_operations::compile_transaction::RetainedApplyFailpoint;

        let mut failures = Vec::new();
        for (label, failpoint) in [
            ("revision-record", RetainedApplyFailpoint::RevisionRecord),
            ("state-marker", RetainedApplyFailpoint::StateMarker),
            (
                "after-postimages",
                RetainedApplyFailpoint::AfterAllPostimages,
            ),
        ] {
            let fixture = actor_fixture(&format!("revision-targeted-rollback-{label}"), &["src"]);
            let package = fixture.roots[0].join("XDTOPackages/Sample/Ext/Package.bin");
            std::fs::create_dir_all(package.parent().unwrap()).unwrap();
            std::fs::write(&package, b"before").unwrap();
            let binding = fixture
                .actor
                .bind_provider_root("src", &fixture.roots[0])
                .unwrap();
            let service = fixture.actor.source_revision_service(&binding).unwrap();
            let admitted = fixture
                .actor
                .admit_apply(
                    &binding,
                    None,
                    false,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap();
            let mut state = admitted.staged_state().unwrap();
            state
                .replace(
                    "XDTOPackages/Sample/Ext/Package.bin",
                    b"before",
                    b"after".to_vec(),
                )
                .unwrap();
            let prepared = admitted.prepare(state).unwrap();
            let source_before = snapshot_tree(&fixture.roots[0]);
            let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
            let machine_before = service.machine_state_for_test();
            crate::infrastructure::native_operations::compile_transaction::set_retained_apply_failpoint(
                failpoint,
            );

            let error = match fixture.actor.publish_prepared_apply(prepared) {
                Err(error) => error,
                Ok(_) => panic!("{label} late failure returned a receipt"),
            };
            if !error.contains("injected retained apply failure") {
                failures.push(format!(
                    "{label} stopped before selected late phase: {error}"
                ));
            }
            assert_eq!(snapshot_tree(&fixture.roots[0]), source_before, "{label}");
            assert_eq!(
                snapshot_tree(&fixture.root.join(".build/unica")),
                cache_before,
                "{label}"
            );
            assert_eq!(service.machine_state_for_test(), machine_before, "{label}");
            fixture.cleanup();
        }

        assert!(
            failures.is_empty(),
            "targeted rollback phase failures: {failures:?}"
        );
    }

    #[test]
    pub(crate) fn actor_revision_platform_commit_preserves_legacy_and_surface_contracts() {
        let fixture = actor_fixture("revision-platform-compatibility", &["src"]);
        let package = fixture.roots[0].join("XDTOPackages/Sample/Ext/Package.bin");
        std::fs::create_dir_all(package.parent().unwrap()).unwrap();
        std::fs::write(&package, b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace(
                "XDTOPackages/Sample/Ext/Package.bin",
                b"before",
                b"after".to_vec(),
            )
            .unwrap();
        let platform_commit = fixture
            .actor
            .publish_prepared_apply(admitted.prepare(state).unwrap());

        let legacy_root = temp_root("revision-legacy-compatibility");
        let legacy_source = legacy_root.join("src");
        std::fs::create_dir_all(&legacy_source).unwrap();
        std::fs::write(legacy_source.join("Module.bsl"), b"module").unwrap();
        std::fs::write(legacy_source.join("scratch.bin"), b"one").unwrap();
        let legacy_context = context(&legacy_root);
        let legacy_service = SourceRevisionService::new(&legacy_context, &legacy_source).unwrap();
        let retained =
            crate::infrastructure::platform::filesystem::RetainedDirectoryCapability::open(
                &std::fs::canonicalize(&legacy_source).unwrap(),
            )
            .unwrap();
        let legacy_before = legacy_service
            .observe_retained_operation(
                &retained,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap()
            .revision_identity();
        std::fs::write(legacy_source.join("scratch.bin"), b"two").unwrap();
        let legacy_after = legacy_service
            .observe_retained_operation(
                &retained,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap()
            .revision_identity();
        assert_eq!(
            legacy_before, legacy_after,
            "legacy V12 digest widened to .bin"
        );

        let result = platform_commit
            .expect("classified Platform resource commit must precede compatibility guards");
        let replay = fixture
            .actor
            .admit_apply(
                &binding,
                Some(result.rev()),
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(replay.revision_identity(), result.rev());
        let _ = std::fs::remove_dir_all(legacy_root);
        fixture.cleanup();
    }

    #[test]
    fn prepared_apply_observer_sees_source_eager_revision_and_state_marker_order() {
        let fixture = actor_fixture("prepared-apply-publication-order", &["src"]);
        std::fs::write(fixture.roots[0].join("Module.bsl"), b"original").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"published".to_vec())
            .unwrap();
        let prepared = admitted
            .prepare_with_cache_effects(
                state,
                &[crate::domain::events::DomainEvent::new(
                    crate::domain::events::DomainEventKind::MetadataChanged,
                    "Catalog.Products",
                )],
            )
            .unwrap();
        let _ = crate::infrastructure::native_operations::compile_transaction::take_retained_apply_observed_events();

        fixture.actor.publish_prepared_apply(prepared).unwrap();

        use crate::infrastructure::native_operations::compile_transaction::RetainedApplyObservedEvent;
        let observed = crate::infrastructure::native_operations::compile_transaction::take_retained_apply_observed_events();
        assert!(
            matches!(
                observed.as_slice(),
                [
                    RetainedApplyObservedEvent::Source(_),
                    RetainedApplyObservedEvent::EagerMetadata(_),
                    RetainedApplyObservedEvent::EagerMetadata(_),
                    RetainedApplyObservedEvent::RevisionRecord(_),
                    RetainedApplyObservedEvent::StateMarker(_),
                ]
            ),
            "unexpected retained publication order: {observed:?}"
        );
        fixture.cleanup();
    }

    #[test]
    fn prepared_apply_success_publishes_source_cache_record_and_state_as_one_revision() {
        let fixture = actor_fixture("prepared-apply-cache-success", &["src"]);
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"original").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let cancellation = CancellationToken::new();
        let deadline = ProviderDeadline::from_budget(Duration::from_secs(5));

        let admitted = fixture
            .actor
            .admit_apply(&binding, None, false, deadline, &cancellation)
            .unwrap();
        let before_rev = admitted.revision_identity().to_string();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"published".to_vec())
            .unwrap();
        let prepared = admitted
            .prepare_with_cache_effects(
                state,
                &[crate::domain::events::DomainEvent::new(
                    crate::domain::events::DomainEventKind::MetadataChanged,
                    "Catalog.Products",
                )],
            )
            .unwrap();

        let result = fixture.actor.publish_prepared_apply(prepared).unwrap();
        let cache_root = fixture.root.join(".build/unica");
        assert_eq!(std::fs::read(&target).unwrap(), b"published");
        assert!(cache_root.join("caches/workspace_graph.json").is_file());
        assert!(cache_root.join("caches/metadata_graph.json").is_file());
        assert!(cache_root.join("state.json").is_file());
        assert_eq!(
            std::fs::read_dir(cache_root.join("source-revisions"))
                .unwrap()
                .count(),
            1
        );
        assert_ne!(result.rev(), before_rev);

        let observed = fixture
            .actor
            .admit_apply(
                &binding,
                Some(result.rev()),
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(observed.revision_identity(), result.rev());
        fixture.cleanup();
    }

    #[test]
    fn retained_apply_failures_restore_source_cache_and_revision_machine_exactly() {
        use crate::infrastructure::native_operations::compile_transaction::RetainedApplyFailpoint;

        for (name, failpoint) in [
            ("second-source", RetainedApplyFailpoint::Source(2)),
            ("eager-cache", RetainedApplyFailpoint::EagerMetadata(1)),
            ("revision-record", RetainedApplyFailpoint::RevisionRecord),
            ("state-marker", RetainedApplyFailpoint::StateMarker),
            ("postimages", RetainedApplyFailpoint::AfterAllPostimages),
        ] {
            let fixture = actor_fixture(&format!("retained-failure-{name}"), &["src"]);
            std::fs::write(fixture.roots[0].join("A.bsl"), b"a-before").unwrap();
            std::fs::write(fixture.roots[0].join("B.bsl"), b"b-before").unwrap();
            let binding = fixture
                .actor
                .bind_provider_root("src", &fixture.roots[0])
                .unwrap();
            let service = fixture.actor.source_revision_service(&binding).unwrap();
            let admitted = fixture
                .actor
                .admit_apply(
                    &binding,
                    None,
                    false,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap();
            let mut state = admitted.staged_state().unwrap();
            state
                .replace("A.bsl", b"a-before", b"a-after".to_vec())
                .unwrap();
            state
                .replace("B.bsl", b"b-before", b"b-after".to_vec())
                .unwrap();
            let prepared = admitted
                .prepare_with_cache_effects(
                    state,
                    &[crate::domain::events::DomainEvent::new(
                        crate::domain::events::DomainEventKind::MetadataChanged,
                        "Catalog.Products",
                    )],
                )
                .unwrap();
            let source_before = snapshot_tree(&fixture.roots[0]);
            let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
            let machine_before = service.machine_state_for_test();
            crate::infrastructure::native_operations::compile_transaction::set_retained_apply_failpoint(failpoint);

            let error = fixture
                .actor
                .publish_prepared_apply(prepared)
                .expect_err(&format!("{name} failpoint did not stop publication"));

            assert!(
                error.contains("injected retained apply failure"),
                "{name}: {error}"
            );
            assert_eq!(
                error.kind(),
                super::ApplyPublicationErrorKind::ProviderPostvalidation,
                "{name} failure lost its typed publication phase"
            );
            assert_eq!(snapshot_tree(&fixture.roots[0]), source_before, "{name}");
            assert_eq!(
                snapshot_tree(&fixture.root.join(".build/unica")),
                cache_before,
                "{name}"
            );
            assert_eq!(service.machine_state_for_test(), machine_before, "{name}");
            fixture.cleanup();
        }
    }

    #[test]
    fn retained_apply_final_cancellation_gate_rolls_back_all_participants() {
        let fixture = actor_fixture("retained-final-cancellation", &["src"]);
        std::fs::write(fixture.roots[0].join("Module.bsl"), b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();
        let cancellation = CancellationToken::new();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"before", b"after".to_vec())
            .unwrap();
        let prepared = admitted
            .prepare_with_cache_effects(
                state,
                &[crate::domain::events::DomainEvent::new(
                    crate::domain::events::DomainEventKind::MetadataChanged,
                    "Catalog.Products",
                )],
            )
            .unwrap();
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        let machine_before = service.machine_state_for_test();
        let cancel_at_gate = cancellation.clone();
        crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_post_validation_hook(
            move || cancel_at_gate.cancel(),
        );

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

        assert!(error.contains("cancel"), "{error}");
        assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Cancelled);
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        assert_eq!(service.machine_state_for_test(), machine_before);
        fixture.cleanup();
    }

    #[test]
    fn retained_apply_late_deadline_after_all_writes_rolls_back_all_participants() {
        use crate::infrastructure::native_operations::compile_transaction::RetainedApplyObservedEvent;

        let fixture = actor_fixture("retained-final-deadline", &["src"]);
        std::fs::write(fixture.roots[0].join("Module.bsl"), b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"before", b"after".to_vec())
            .unwrap();
        let mut prepared = admitted
            .prepare_with_cache_effects(
                state,
                &[crate::domain::events::DomainEvent::new(
                    crate::domain::events::DomainEventKind::MetadataChanged,
                    "Catalog.Products",
                )],
            )
            .unwrap();
        prepared.deadline = ProviderDeadline::from_budget(Duration::from_millis(500));
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        let machine_before = service.machine_state_for_test();
        let _ = crate::infrastructure::native_operations::compile_transaction::take_retained_apply_observed_events();
        crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_post_validation_hook(
            || std::thread::sleep(Duration::from_millis(550)),
        );

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

        assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Deadline);
        let observed = crate::infrastructure::native_operations::compile_transaction::take_retained_apply_observed_events();
        assert!(
            observed
                .iter()
                .any(|event| matches!(event, RetainedApplyObservedEvent::StateMarker(_))),
            "deadline fired before all participant writes: {observed:?}"
        );
        assert!(
            observed
                .iter()
                .any(|event| matches!(event, RetainedApplyObservedEvent::Rollback(_))),
            "late deadline did not enter rollback: {observed:?}"
        );
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        assert_eq!(service.machine_state_for_test(), machine_before);
        fixture.cleanup();
    }

    #[test]
    fn retained_apply_observer_sees_exact_reverse_rollback_after_state_marker() {
        use crate::infrastructure::native_operations::compile_transaction::{
            RetainedApplyFailpoint, RetainedApplyObservedEvent,
        };

        let fixture = actor_fixture("retained-reverse-rollback-order", &["src"]);
        std::fs::write(fixture.roots[0].join("A.bsl"), b"a-before").unwrap();
        std::fs::write(fixture.roots[0].join("B.bsl"), b"b-before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("A.bsl", b"a-before", b"a-after".to_vec())
            .unwrap();
        state
            .replace("B.bsl", b"b-before", b"b-after".to_vec())
            .unwrap();
        let prepared = admitted
            .prepare_with_cache_effects(
                state,
                &[crate::domain::events::DomainEvent::new(
                    crate::domain::events::DomainEventKind::MetadataChanged,
                    "Catalog.Products",
                )],
            )
            .unwrap();
        let _ = crate::infrastructure::native_operations::compile_transaction::take_retained_apply_observed_events();
        crate::infrastructure::native_operations::compile_transaction::set_retained_apply_failpoint(
            RetainedApplyFailpoint::StateMarker,
        );

        fixture.actor.publish_prepared_apply(prepared).unwrap_err();

        let observed = crate::infrastructure::native_operations::compile_transaction::take_retained_apply_observed_events();
        let forward = observed
            .iter()
            .take_while(|event| !matches!(event, RetainedApplyObservedEvent::Rollback(_)))
            .map(|event| match event {
                RetainedApplyObservedEvent::Source(path)
                | RetainedApplyObservedEvent::EagerMetadata(path)
                | RetainedApplyObservedEvent::RevisionRecord(path)
                | RetainedApplyObservedEvent::StateMarker(path) => path.clone(),
                RetainedApplyObservedEvent::Rollback(_) => unreachable!(),
            })
            .collect::<Vec<_>>();
        let rollback = observed
            .iter()
            .filter_map(|event| match event {
                RetainedApplyObservedEvent::Rollback(path) => Some(path.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(rollback, forward.into_iter().rev().collect::<Vec<_>>());
        fixture.cleanup();
    }

    #[test]
    fn retained_apply_deterministic_success_and_rollback_order_is_complete() {
        prepared_apply_observer_sees_source_eager_revision_and_state_marker_order();
        retained_apply_observer_sees_exact_reverse_rollback_after_state_marker();
    }

    #[test]
    pub(crate) fn retained_apply_transaction_foundation_contract_is_complete() {
        retained_apply_closed_participant_contract_is_complete();
        prepared_apply_success_publishes_source_cache_record_and_state_as_one_revision();
        retained_apply_failures_restore_source_cache_and_revision_machine_exactly();
        retained_apply_deterministic_success_and_rollback_order_is_complete();
        retained_apply_final_cancellation_gate_rolls_back_all_participants();
        retained_apply_late_deadline_after_all_writes_rolls_back_all_participants();
        retained_apply_trust_epoch_race_rolls_back_without_overwriting_foreign_state();
        apply_admission_and_dry_run_revision_observation_are_cache_tree_write_free();
        actor_scoped_logical_revision_service_keeps_the_platform_fence_capability();
        prepared_apply_cleanup_race_surfaces_a_relative_actor_diagnostic();
        rollback_incomplete_failure_has_a_typed_non_success_category();
    }

    #[test]
    fn retained_apply_trust_epoch_race_rolls_back_without_overwriting_foreign_state() {
        let fixture = actor_fixture("retained-trust-epoch-race", &["src"]);
        std::fs::write(fixture.roots[0].join("Module.bsl"), b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"before", b"after".to_vec())
            .unwrap();
        let prepared = admitted
            .prepare_with_cache_effects(
                state,
                &[crate::domain::events::DomainEvent::new(
                    crate::domain::events::DomainEventKind::MetadataChanged,
                    "Catalog.Products",
                )],
            )
            .unwrap();
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        let race_service = Arc::clone(&service);
        crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_post_validation_hook(
            move || race_service.mark_dirty(),
        );

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

        assert!(error.contains("trust epoch"), "{error}");
        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::ConcurrentRevision
        );
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        assert!(matches!(
            service.machine_state_for_test().state(),
            crate::domain::source_revision::SourceRevisionState::Untrusted {
                reason: crate::domain::source_revision::SourceRevisionTrustLoss::WatcherGap,
                ..
            }
        ));
        fixture.cleanup();
    }

    #[test]
    fn retained_apply_publication_preserves_exact_typed_causes_end_to_end() {
        use crate::infrastructure::source_revision::{
            set_repeating_retained_scan_test_mutation, RetainedScanTestMutationPoint,
        };

        let fixture = actor_fixture("typed-cause-cancelled-scan", &["src"]);
        std::fs::write(fixture.roots[0].join("Module.bsl"), b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let cancellation = CancellationToken::new();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"before", b"after".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        let cancel_during_scan = cancellation.clone();
        let cancel_scan_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let cancel_scan_observed = Arc::clone(&cancel_scan_count);
        let _mutation = set_repeating_retained_scan_test_mutation(
            RetainedScanTestMutationPoint::AfterDirectoryEnumeration,
            move || {
                if cancel_scan_observed.fetch_add(1, Ordering::AcqRel) == 2 {
                    cancel_during_scan.cancel();
                }
            },
        );
        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Cancelled);
        fixture.cleanup();

        let fixture = actor_fixture("typed-cause-deadline-scan", &["src"]);
        std::fs::write(fixture.roots[0].join("Module.bsl"), b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"before", b"after".to_vec())
            .unwrap();
        let mut prepared = admitted.prepare(state).unwrap();
        prepared.deadline = ProviderDeadline::from_budget(Duration::from_millis(200));
        let deadline_scan_ran = Arc::new(AtomicBool::new(false));
        let deadline_scan_observed = Arc::clone(&deadline_scan_ran);
        let deadline_scan_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let deadline_scan_counter = Arc::clone(&deadline_scan_count);
        let _mutation = set_repeating_retained_scan_test_mutation(
            RetainedScanTestMutationPoint::AfterDirectoryEnumeration,
            move || {
                if deadline_scan_counter.fetch_add(1, Ordering::AcqRel) == 2 {
                    deadline_scan_observed.store(true, Ordering::Release);
                    std::thread::sleep(Duration::from_millis(250));
                }
            },
        );
        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        assert!(deadline_scan_ran.load(Ordering::Acquire));
        assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Deadline);
        fixture.cleanup();

        let fixture = actor_fixture("typed-cause-provider-scan", &["src"]);
        std::fs::write(fixture.roots[0].join("Module.bsl"), b"before").unwrap();
        let nested = fixture.roots[0].join("Nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("NestedModule.bsl"), b"nested").unwrap();
        if !crate::infrastructure::platform::testing::set_unix_mode_for_test(&nested, 0o700)
            .unwrap()
        {
            fixture.cleanup();
            return;
        }
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"before", b"after".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        crate::infrastructure::platform::testing::set_unix_mode_for_test(&nested, 0o000).unwrap();
        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        crate::infrastructure::platform::testing::set_unix_mode_for_test(&nested, 0o700).unwrap();
        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::ProviderPostvalidation
        );
        fixture.cleanup();

        let fixture = actor_fixture("typed-cause-concurrent-scan", &["src"]);
        std::fs::write(fixture.roots[0].join("Module.bsl"), b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"before", b"after".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        let concurrent = fixture.roots[0].join("Concurrent.bsl");
        let concurrent_scan_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let concurrent_scan_counter = Arc::clone(&concurrent_scan_count);
        let _mutation = set_repeating_retained_scan_test_mutation(
            RetainedScanTestMutationPoint::AfterDirectoryEnumeration,
            move || {
                if concurrent_scan_counter.fetch_add(1, Ordering::AcqRel) == 2 {
                    std::fs::write(&concurrent, b"foreign").unwrap();
                }
            },
        );
        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::ConcurrentRevision
        );
        fixture.cleanup();

        let fixture = actor_fixture("typed-cause-containment", &["src"]);
        std::fs::write(fixture.roots[0].join("Module.bsl"), b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"before", b"after".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        std::fs::rename(&fixture.roots[0], fixture.root.join("source-displaced")).unwrap();
        std::fs::create_dir_all(&fixture.roots[0]).unwrap();
        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::ContainmentIdentity
        );
        fixture.cleanup();

        let fixture = actor_fixture("typed-cause-publish-provider", &["src"]);
        std::fs::write(fixture.roots[0].join("Module.bsl"), b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"before", b"after".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        let source_root = fixture.roots[0].clone();
        crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_provider_io_hook(
            move || {
                crate::infrastructure::platform::testing::set_unix_mode_for_test(
                    &source_root,
                    0o500,
                )
                .unwrap();
            },
        );
        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        crate::infrastructure::platform::testing::set_unix_mode_for_test(&fixture.roots[0], 0o700)
            .unwrap();
        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::ProviderPostvalidation
        );
        fixture.cleanup();
    }

    #[test]
    fn prepared_apply_cache_authority_diagnostic_never_exposes_absolute_root() {
        let (fixture, binding) = publish_cache_fixture("cache-authority-redaction");
        let cache_root = fixture.root.join(".build/unica");
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let state = admitted.staged_state().unwrap();
        let prepared = admitted.prepare(state).unwrap();
        let displaced = fixture.root.join("cache-displaced");
        let retained_identity = path_identity_for_test(&cache_root)
            .unwrap()
            .expect("cache root identity must be available on supported CI platforms");
        match attempt_retained_directory_replacement_for_test(&cache_root, &displaced).unwrap() {
            RetainedDirectoryReplacementOutcome::Replaced => {
                std::fs::create_dir_all(&cache_root).unwrap();
                let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

                assert_eq!(
                    error.kind(),
                    super::ApplyPublicationErrorKind::ContainmentIdentity
                );
                assert!(
                    !error
                        .to_string()
                        .contains(&cache_root.display().to_string()),
                    "cache authority diagnostic exposed its absolute root: {error}"
                );
            }
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
                eprintln!(
                    "[SKIPPED FIXTURE] cache-root replacement is prevented by its retained handle"
                );
                assert_eq!(
                    path_identity_for_test(&cache_root).unwrap().as_deref(),
                    Some(retained_identity.as_str())
                );
                assert!(!displaced.exists());
                fixture.actor.publish_prepared_apply(prepared).unwrap();
            }
        }
        fixture.cleanup();
    }

    fn publish_cache_fixture(name: &str) -> (ActorFixture, super::ProviderRootBinding) {
        let fixture = actor_fixture(name, &["src"]);
        std::fs::write(fixture.roots[0].join("Module.bsl"), b"baseline").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let state = admitted.staged_state().unwrap();
        fixture
            .actor
            .publish_prepared_apply(
                admitted
                    .prepare_with_cache_effects(
                        state,
                        &[crate::domain::events::DomainEvent::new(
                            crate::domain::events::DomainEventKind::MetadataChanged,
                            "Catalog.Products",
                        )],
                    )
                    .unwrap(),
            )
            .unwrap();
        (fixture, binding)
    }

    fn prepare_cache_fixture_apply(
        fixture: &ActorFixture,
        binding: &super::ProviderRootBinding,
    ) -> super::PreparedApplyBatch {
        let admitted = fixture
            .actor
            .admit_apply(
                binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"baseline", b"must-rollback".to_vec())
            .unwrap();
        admitted
            .prepare_with_cache_effects(
                state,
                &[crate::domain::events::DomainEvent::new(
                    crate::domain::events::DomainEventKind::MetadataChanged,
                    "Catalog.Products",
                )],
            )
            .unwrap()
    }

    #[test]
    fn cache_and_revision_preimage_races_fail_closed_and_preserve_foreign_names() {
        for case in ["cache-replaced", "revision-disappeared", "cache-hard-link"] {
            let (fixture, binding) = publish_cache_fixture(&format!("preimage-{case}"));
            let prepared = prepare_cache_fixture_apply(&fixture, &binding);
            let cache_root = fixture.root.join(".build/unica");
            let metadata = cache_root.join("caches/metadata_graph.json");
            let revision = std::fs::read_dir(cache_root.join("source-revisions"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            match case {
                "cache-replaced" => std::fs::write(&metadata, b"foreign-cache").unwrap(),
                "revision-disappeared" => std::fs::remove_file(&revision).unwrap(),
                "cache-hard-link" => {
                    let foreign = fixture.root.join("foreign-cache.json");
                    std::fs::write(&foreign, b"foreign-hard-link").unwrap();
                    std::fs::remove_file(&metadata).unwrap();
                    std::fs::hard_link(&foreign, &metadata).unwrap();
                }
                _ => unreachable!(),
            }

            let error = fixture
                .actor
                .publish_prepared_apply(prepared)
                .expect_err(&format!("{case} race was accepted"));

            assert!(
                error.contains("preimage")
                    || error.contains("identity")
                    || error.contains("hard-link"),
                "{case}: {error}"
            );
            assert_eq!(
                std::fs::read(fixture.roots[0].join("Module.bsl")).unwrap(),
                b"baseline"
            );
            match case {
                "cache-replaced" => assert_eq!(std::fs::read(metadata).unwrap(), b"foreign-cache"),
                "revision-disappeared" => assert!(!revision.exists()),
                "cache-hard-link" => {
                    assert_eq!(std::fs::read(metadata).unwrap(), b"foreign-hard-link")
                }
                _ => unreachable!(),
            }
            fixture.cleanup();
        }
    }

    #[test]
    fn cache_preimage_link_race_and_absent_chain_occupation_preserve_foreign_namespace() {
        let (fixture, binding) = publish_cache_fixture("preimage-cache-link");
        let prepared = prepare_cache_fixture_apply(&fixture, &binding);
        let state_path = fixture.root.join(".build/unica/state.json");
        let foreign = fixture.root.join("foreign-state.json");
        std::fs::write(&foreign, b"foreign-state").unwrap();
        std::fs::remove_file(&state_path).unwrap();
        match create_file_link_fixture_for_test(&foreign, &state_path).unwrap() {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
                fixture.cleanup();
                return;
            }
        }

        fixture.actor.publish_prepared_apply(prepared).unwrap_err();

        assert!(std::fs::symlink_metadata(&state_path)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read(&foreign).unwrap(), b"foreign-state");
        assert_eq!(
            std::fs::read(fixture.roots[0].join("Module.bsl")).unwrap(),
            b"baseline"
        );
        fixture.cleanup();

        let fixture = actor_fixture("preimage-cache-chain", &["src"]);
        std::fs::write(fixture.roots[0].join("Module.bsl"), b"baseline").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"baseline", b"must-rollback".to_vec())
            .unwrap();
        let prepared = admitted
            .prepare_with_cache_effects(
                state,
                &[crate::domain::events::DomainEvent::new(
                    crate::domain::events::DomainEventKind::MetadataChanged,
                    "Catalog.Products",
                )],
            )
            .unwrap();
        std::fs::write(fixture.root.join(".build"), b"foreign-occupant").unwrap();

        fixture.actor.publish_prepared_apply(prepared).unwrap_err();

        assert_eq!(
            std::fs::read(fixture.root.join(".build")).unwrap(),
            b"foreign-occupant"
        );
        assert_eq!(
            std::fs::read(fixture.roots[0].join("Module.bsl")).unwrap(),
            b"baseline"
        );
        fixture.cleanup();
    }

    #[test]
    fn malformed_state_fallback_restores_exact_bytes_when_later_gate_fails() {
        let fixture = actor_fixture("malformed-state-rollback", &["src"]);
        std::fs::write(fixture.roots[0].join("Module.bsl"), b"before").unwrap();
        std::fs::create_dir_all(fixture.root.join(".build/unica")).unwrap();
        let malformed = b"{malformed-state\0bytes".to_vec();
        std::fs::write(fixture.root.join(".build/unica/state.json"), &malformed).unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"before", b"after".to_vec())
            .unwrap();
        let prepared = admitted
            .prepare_with_cache_effects(
                state,
                &[crate::domain::events::DomainEvent::new(
                    crate::domain::events::DomainEventKind::MetadataChanged,
                    "Catalog.Products",
                )],
            )
            .unwrap();
        crate::infrastructure::native_operations::compile_transaction::set_retained_apply_failpoint(
            crate::infrastructure::native_operations::compile_transaction::RetainedApplyFailpoint::AfterAllPostimages,
        );

        fixture.actor.publish_prepared_apply(prepared).unwrap_err();

        assert_eq!(
            std::fs::read(fixture.root.join(".build/unica/state.json")).unwrap(),
            malformed
        );
        assert_eq!(
            std::fs::read(fixture.roots[0].join("Module.bsl")).unwrap(),
            b"before"
        );
        fixture.cleanup();
    }

    #[test]
    fn rollback_incomplete_failure_has_a_typed_non_success_category() {
        let fixture = actor_fixture("typed-rollback-incomplete", &["src"]);
        let target = fixture.roots[0].join("Module.bsl");
        let moved = fixture.roots[0].join("published-moved.bsl");
        std::fs::write(&target, b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let cancellation = CancellationToken::new();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"before", b"published".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        let raced_target = target.clone();
        let raced_moved = moved.clone();
        let cancel = cancellation.clone();
        crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_post_validation_hook(
            move || {
                std::fs::rename(&raced_target, &raced_moved).unwrap();
                std::fs::write(&raced_target, b"foreign").unwrap();
                cancel.cancel();
            },
        );

        let typed = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

        assert_eq!(
            typed.kind(),
            super::ApplyPublicationErrorKind::RollbackIncomplete
        );
        assert!(typed.contains("rollback encountered"));
        assert_eq!(std::fs::read(&target).unwrap(), b"foreign");
        assert_eq!(std::fs::read(&moved).unwrap(), b"published");
        fixture.cleanup();
    }

    #[test]
    fn apply_admission_and_dry_run_revision_observation_are_cache_tree_write_free() {
        let fixture = actor_fixture("apply-observation-write-free", &["src"]);
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"original").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let cache_root = fixture.root.join(".build/unica");
        assert!(!cache_root.exists());
        let before = snapshot_tree(&cache_root);

        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(snapshot_tree(&cache_root), before);

        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"dry-run".to_vec())
            .unwrap();
        fixture
            .actor
            .publish_prepared_apply(admitted.prepare(state).unwrap())
            .unwrap();

        assert_eq!(snapshot_tree(&cache_root), before);
        assert_eq!(std::fs::read(&target).unwrap(), b"original");
        fixture.cleanup();
    }

    #[test]
    fn prepared_apply_dry_run_is_byte_identical_and_real_apply_commits_once_with_new_revision() {
        let fixture = actor_fixture("prepared-apply", &["src"]);
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"original").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let cancellation = CancellationToken::new();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let admitted_rev = admitted.revision_identity().to_string();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"dry-run".to_vec())
            .unwrap();
        state
            .create("Ext/Form/Module.bsl", b"dry-run-create".to_vec())
            .unwrap();
        let dry_result = fixture
            .actor
            .publish_prepared_apply(admitted.prepare(state).unwrap())
            .unwrap();
        assert_eq!(dry_result.rev(), admitted_rev);
        assert_eq!(std::fs::read(&target).unwrap(), b"original");
        assert!(!fixture.roots[0].join("Ext").exists());
        assert_eq!(dry_result.commit_count_for_test(), 0);
        assert!(dry_result.cleanup_diagnostics().is_empty());

        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                Some(&admitted_rev),
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"published".to_vec())
            .unwrap();
        state
            .create("Ext/Form/Module.bsl", b"published-create".to_vec())
            .unwrap();
        let result = fixture
            .actor
            .publish_prepared_apply(admitted.prepare(state).unwrap())
            .unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"published");
        assert_eq!(
            std::fs::read(fixture.roots[0].join("Ext/Form/Module.bsl")).unwrap(),
            b"published-create"
        );
        assert_ne!(result.rev(), admitted_rev);
        assert_eq!(result.commit_count_for_test(), 1);
        assert!(result.cleanup_diagnostics().is_empty());
        fixture.cleanup();
    }

    #[test]
    fn apply_selection_rejects_v8project_kind_change_after_prepare() {
        let fixture = actor_fixture("selection-kind-change", &["src/cf"]);
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src/cf", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"before", b"after".to_vec())
            .unwrap();
        let prepared = admitted
            .prepare_with_cache_effects(
                state,
                &[crate::domain::events::DomainEvent::new(
                    crate::domain::events::DomainEventKind::MetadataChanged,
                    "Catalog.Products",
                )],
            )
            .unwrap();
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        let machine_before = service.machine_state_for_test();
        std::fs::write(
            fixture.root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: src/cf\n    type: EXTENSION\n    path: src/cf\n",
        )
        .unwrap();

        let error = fixture
            .actor
            .publish_prepared_apply(prepared)
            .expect_err("changed source-map kind published and returned a committed receipt");

        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::SourceSelectionChanged
        );
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        assert_eq!(service.machine_state_for_test(), machine_before);
        fixture.cleanup();
    }

    #[test]
    fn apply_selection_rejects_v8project_absence_to_appearance_after_prepare() {
        let fixture = actor_fixture_without_source_map("selection-map-appears", "src/cf");
        std::fs::write(
            fixture.roots[0].join("Configuration.xml"),
            b"<MetaDataObject/>",
        )
        .unwrap();
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"before", b"after".to_vec())
            .unwrap();
        let prepared = admitted
            .prepare_with_cache_effects(
                state,
                &[crate::domain::events::DomainEvent::new(
                    crate::domain::events::DomainEventKind::MetadataChanged,
                    "Catalog.Products",
                )],
            )
            .unwrap();
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        let machine_before = service.machine_state_for_test();
        std::fs::write(
            fixture.root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src/cf\n",
        )
        .unwrap();

        let error = fixture.actor.publish_prepared_apply(prepared).expect_err(
            "an appearing semantically equivalent source map published and returned a receipt",
        );

        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::SourceSelectionChanged
        );
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        assert_eq!(service.machine_state_for_test(), machine_before);
        fixture.cleanup();
    }

    #[test]
    fn apply_selection_rejects_autodetected_extension_membership_change() {
        let fixture = actor_fixture_without_source_map("selection-extension-membership", "src/cf");
        std::fs::write(
            fixture.roots[0].join("Configuration.xml"),
            b"<MetaDataObject/>",
        )
        .unwrap();
        std::fs::create_dir_all(fixture.root.join("src/cfe")).unwrap();
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"before", b"after".to_vec())
            .unwrap();
        let prepared = admitted
            .prepare_with_cache_effects(
                state,
                &[crate::domain::events::DomainEvent::new(
                    crate::domain::events::DomainEventKind::MetadataChanged,
                    "Catalog.Products",
                )],
            )
            .unwrap();
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        let machine_before = service.machine_state_for_test();
        std::fs::create_dir_all(fixture.root.join("src/cfe/late")).unwrap();
        std::fs::write(
            fixture.root.join("src/cfe/late/Configuration.xml"),
            b"<MetaDataObject/>",
        )
        .unwrap();

        let error = fixture.actor.publish_prepared_apply(prepared).expect_err(
            "changed autodetection membership published and returned a committed receipt",
        );

        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::SourceSelectionChanged
        );
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        assert_eq!(service.machine_state_for_test(), machine_before);
        fixture.cleanup();
    }

    #[test]
    fn apply_selection_rejects_unselected_declared_parent_appearance() {
        let fixture = actor_fixture_without_source_map("selection-parent-appears", "src/cf");
        std::fs::write(
            fixture.roots[0].join("Configuration.xml"),
            b"<MetaDataObject/>",
        )
        .unwrap();
        std::fs::write(
            fixture.root.join("v8project.yaml"),
            concat!(
                "format: EDT\n",
                "source-set:\n",
                "  - name: main\n",
                "    type: CONFIGURATION\n",
                "    path: src/cf\n",
                "  - name: optional-edt\n",
                "    type: CONFIGURATION\n",
                "    path: optional/edt\n",
            ),
        )
        .unwrap();
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"before").unwrap();
        assert!(!fixture.root.join("optional").exists());
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"before", b"after".to_vec())
            .unwrap();
        let prepared = admitted
            .prepare_with_cache_effects(
                state,
                &[crate::domain::events::DomainEvent::new(
                    crate::domain::events::DomainEventKind::MetadataChanged,
                    "Catalog.Products",
                )],
            )
            .unwrap();
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        let machine_before = service.machine_state_for_test();
        std::fs::create_dir_all(fixture.root.join("optional/edt")).unwrap();
        std::fs::write(
            fixture.root.join("optional/edt/Configuration.xml"),
            b"<MetaDataObject/>",
        )
        .unwrap();
        std::fs::write(fixture.root.join("optional/edt/.project"), b"project").unwrap();

        let error = fixture
            .actor
            .publish_prepared_apply(prepared)
            .expect_err("appearing intermediate full-map parent published and returned a receipt");

        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::SourceSelectionChanged
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"before");
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        assert_eq!(service.machine_state_for_test(), machine_before);
        fixture.cleanup();
    }

    #[test]
    fn apply_selection_rejects_unselected_non_platform_map_input_change() {
        let fixture = actor_fixture_without_source_map("selection-unselected-edt", "src/cf");
        std::fs::write(
            fixture.roots[0].join("Configuration.xml"),
            b"<MetaDataObject/>",
        )
        .unwrap();
        std::fs::create_dir_all(fixture.root.join("src/edt")).unwrap();
        std::fs::write(fixture.root.join("src/edt/.project"), b"project").unwrap();
        std::fs::write(
            fixture.root.join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: main\n",
                "    type: CONFIGURATION\n",
                "    path: src/cf\n",
                "  - name: edt\n",
                "    type: CONFIGURATION\n",
                "    path: src/edt\n",
            ),
        )
        .unwrap();
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"before", b"after".to_vec())
            .unwrap();
        let prepared = admitted
            .prepare_with_cache_effects(
                state,
                &[crate::domain::events::DomainEvent::new(
                    crate::domain::events::DomainEventKind::MetadataChanged,
                    "Catalog.Products",
                )],
            )
            .unwrap();
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        let machine_before = service.machine_state_for_test();
        std::fs::write(
            fixture.root.join("src/edt/Configuration.xml"),
            b"<MetaDataObject/>",
        )
        .unwrap();

        let error = fixture.actor.publish_prepared_apply(prepared).expect_err(
            "unselected EDT-to-invalid map change published and returned a committed receipt",
        );

        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::SourceSelectionChanged
        );
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        assert_eq!(service.machine_state_for_test(), machine_before);
        fixture.cleanup();
    }

    #[test]
    fn apply_selection_rejects_repaired_oversized_unselected_external_descriptor() {
        let fixture =
            actor_fixture_without_source_map("selection-repaired-oversized-external", "src/cf");
        std::fs::write(
            fixture.roots[0].join("Configuration.xml"),
            b"<MetaDataObject/>",
        )
        .unwrap();
        std::fs::create_dir_all(fixture.root.join("epf")).unwrap();
        std::fs::write(
            fixture.root.join("v8project.yaml"),
            concat!(
                "format: EDT\n",
                "source-set:\n",
                "  - name: main\n",
                "    type: CONFIGURATION\n",
                "    path: src/cf\n",
                "  - name: unselected-processors\n",
                "    type: EXTERNAL_DATA_PROCESSORS\n",
                "    path: epf\n",
            ),
        )
        .unwrap();
        let descriptor = fixture.root.join("epf/ConfigDumpInfo.xml");
        std::fs::write(&descriptor, vec![b'x'; 8 * 1024 * 1024 + 1]).unwrap();
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"before", b"after".to_vec())
            .unwrap();
        let prepared = admitted
            .prepare_with_cache_effects(
                state,
                &[crate::domain::events::DomainEvent::new(
                    crate::domain::events::DomainEventKind::MetadataChanged,
                    "Catalog.Products",
                )],
            )
            .unwrap();
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        let machine_before = service.machine_state_for_test();
        std::fs::write(
            descriptor,
            b"<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
        )
        .unwrap();

        let error = fixture.actor.publish_prepared_apply(prepared).expect_err(
            "repaired oversized unselected descriptor published and returned a receipt",
        );

        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::SourceSelectionChanged
        );
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        assert_eq!(service.machine_state_for_test(), machine_before);
        fixture.cleanup();
    }

    #[test]
    fn apply_selection_dry_run_rejects_late_map_change_without_receipt() {
        let fixture = actor_fixture("selection-dry-late", &["src/cf"]);
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src/cf", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"before", b"projected".to_vec())
            .unwrap();
        let prepared = admitted
            .prepare_with_cache_effects(
                state,
                &[crate::domain::events::DomainEvent::new(
                    crate::domain::events::DomainEventKind::MetadataChanged,
                    "Catalog.Products",
                )],
            )
            .unwrap();
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        let machine_before = service.machine_state_for_test();
        let map = fixture.root.join("v8project.yaml");
        set_apply_dry_run_after_confirmation_hook(move || {
            std::fs::write(
                map,
                "format: DESIGNER\nsource-set:\n  - name: src/cf\n    type: EXTENSION\n    path: src/cf\n",
            )
            .unwrap();
        });

        let error = fixture
            .actor
            .publish_prepared_apply(prepared)
            .expect_err("late dry-run source-map change returned a projected result and receipt");

        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::SourceSelectionChanged
        );
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        assert_eq!(service.machine_state_for_test(), machine_before);
        fixture.cleanup();
    }

    #[test]
    fn apply_selection_late_change_rolls_back_source_cache_revision_and_receipt() {
        let fixture = actor_fixture_without_source_map("selection-real-late", "src/cf");
        std::fs::write(
            fixture.roots[0].join("Configuration.xml"),
            b"<MetaDataObject/>",
        )
        .unwrap();
        std::fs::create_dir_all(fixture.root.join("src/edt")).unwrap();
        std::fs::write(fixture.root.join("src/edt/.project"), b"project").unwrap();
        std::fs::write(
            fixture.root.join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: main\n",
                "    type: CONFIGURATION\n",
                "    path: src/cf\n",
                "  - name: edt\n",
                "    type: CONFIGURATION\n",
                "    path: src/edt\n",
            ),
        )
        .unwrap();
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"before", b"published".to_vec())
            .unwrap();
        let prepared = admitted
            .prepare_with_cache_effects(
                state,
                &[crate::domain::events::DomainEvent::new(
                    crate::domain::events::DomainEventKind::MetadataChanged,
                    "Catalog.Products",
                )],
            )
            .unwrap();
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        let machine_before = service.machine_state_for_test();
        let late_marker = fixture.root.join("src/edt/Configuration.xml");
        crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_post_validation_hook(
            move || std::fs::write(late_marker, b"<MetaDataObject/>").unwrap(),
        );

        let error = fixture.actor.publish_prepared_apply(prepared).expect_err(
            "late full-map change committed source/cache/revision and returned a receipt",
        );

        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::SourceSelectionChanged
        );
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        assert_eq!(service.machine_state_for_test(), machine_before);
        fixture.cleanup();
    }

    #[test]
    fn apply_selection_rejects_autodetection_container_identity_replacement() {
        let fixture = actor_fixture_without_source_map("selection-container-swap", "src/cf");
        std::fs::write(
            fixture.roots[0].join("Configuration.xml"),
            b"<MetaDataObject/>",
        )
        .unwrap();
        let container = fixture.root.join("src/cfe");
        std::fs::create_dir_all(&container).unwrap();
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"before").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"before", b"after".to_vec())
            .unwrap();
        let prepared = admitted
            .prepare_with_cache_effects(
                state,
                &[crate::domain::events::DomainEvent::new(
                    crate::domain::events::DomainEventKind::MetadataChanged,
                    "Catalog.Products",
                )],
            )
            .unwrap();
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        let machine_before = service.machine_state_for_test();
        std::fs::rename(&container, fixture.root.join("src/cfe-displaced")).unwrap();
        std::fs::create_dir(&container).unwrap();

        let error = fixture.actor.publish_prepared_apply(prepared).expect_err(
            "replacement container with identical membership published and returned a receipt",
        );

        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::SourceSelectionChanged
        );
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        assert_eq!(service.machine_state_for_test(), machine_before);
        fixture.cleanup();
    }

    #[test]
    pub(crate) fn retained_source_selection_finality_contract_is_complete() {
        crate::infrastructure::source_selection_evidence::tests::actor_admission_rejects_aggregate_exact_byte_budget();
        crate::infrastructure::source_selection_evidence::tests::actor_admission_charges_repeated_exact_work_before_second_read();
        crate::infrastructure::source_selection_evidence::tests::actor_admission_bounds_unique_retained_directories_without_ulimit();
        crate::infrastructure::source_selection_evidence::tests::actor_admission_bounds_global_membership_across_external_source_sets();
        crate::infrastructure::source_selection_evidence::tests::actor_admission_counts_repeated_membership_enumeration_globally();
        crate::infrastructure::source_selection_evidence::tests::actor_admission_rejects_total_evidence_record_budget();
        crate::infrastructure::source_selection_evidence::tests::actor_admission_rejects_route_and_name_byte_budget();
        crate::infrastructure::source_selection_evidence::tests::retained_selection_pass_checks_membership_budget_before_enumeration();
        crate::infrastructure::source_selection_evidence::tests::retained_selection_pass_checks_remaining_record_capacity_before_enumeration();
        crate::infrastructure::source_selection_evidence::tests::retained_selection_pass_checks_remaining_name_capacity_before_enumeration();
        crate::infrastructure::source_selection_evidence::tests::retained_selection_pass_rejects_before_unseen_member_child_open();
        crate::infrastructure::source_selection_evidence::tests::membership_overflow_probe_never_retains_more_names_than_charged();
        crate::infrastructure::source_selection_evidence::tests::membership_zero_work_rejects_before_enumeration();
        crate::infrastructure::source_selection_evidence::tests::membership_child_record_cost_is_preflighted_before_open();
        crate::infrastructure::source_selection_evidence::tests::membership_child_route_cost_is_preflighted_before_open();
        crate::infrastructure::source_selection_evidence::tests::retained_exact_read_never_appends_a_growth_chunk_past_the_limit();
        crate::infrastructure::source_selection_evidence::tests::retained_selection_pass_checks_record_budget_before_regular_open();
        crate::infrastructure::source_selection_evidence::tests::actor_admission_comparison_honors_cancellation();
        crate::infrastructure::source_selection_evidence::tests::actor_admission_comparison_honors_deadline();
        crate::infrastructure::source_selection_evidence::tests::retained_selection_pass_deduplicates_repeated_observations();
        crate::infrastructure::source_selection_evidence::tests::retained_selection_pass_rejects_inconsistent_regular_repeat();
        crate::infrastructure::source_selection_evidence::tests::retained_selection_pass_rejects_inconsistent_directory_repeat();
        crate::infrastructure::source_selection_evidence::tests::retained_selection_pass_rejects_inconsistent_membership_repeat();
        apply_selection_rejects_v8project_kind_change_after_prepare();
        apply_selection_rejects_v8project_absence_to_appearance_after_prepare();
        apply_selection_rejects_autodetected_extension_membership_change();
        apply_selection_rejects_unselected_declared_parent_appearance();
        apply_selection_rejects_unselected_non_platform_map_input_change();
        apply_selection_rejects_repaired_oversized_unselected_external_descriptor();
        apply_selection_dry_run_rejects_late_map_change_without_receipt();
        apply_selection_late_change_rolls_back_source_cache_revision_and_receipt();
        apply_selection_rejects_autodetection_container_identity_replacement();
        prepared_apply_root_and_actor_capabilities_cannot_be_redirected_or_replayed();
        apply_policy_foreign_actor_and_sibling_worktree_replay_are_rejected();
        retained_binding_rejects_a_same_path_directory_replacement();
        active_alias_reuses_actor_and_dropped_actor_recreates_a_new_instance();
        crate::infrastructure::daemon::server::actor_capacity_tests::restart_request_does_not_claim_noncooperative_actor_released_in_process();
        crate::infrastructure::daemon::server::actor_capacity_tests::working_task_recovery_is_resume_unsupported_without_apply_reexecution();
        crate::infrastructure::daemon::server::actor_capacity_tests::view_find_admitted_snapshot_may_finish_after_map_change();
        crate::infrastructure::daemon::server::actor_capacity_tests::semantically_equivalent_map_edit_reuses_actor_identity();
        crate::infrastructure::project_sources::tests::actor_admission_preserves_declared_external_processor_and_report_map();
        crate::infrastructure::project_sources::tests::actor_admission_external_config_dump_info_content_change_invalidates_evidence();
        crate::infrastructure::project_sources::tests::actor_admission_external_descriptor_absence_to_appearance_invalidates_evidence();
        crate::infrastructure::project_sources::tests::external_actor_positive_witness_uses_no_process_global_counter();
    }

    #[test]
    fn apply_preparation_preserves_all_six_typed_staging_error_kinds() {
        let mut observed = Vec::new();

        let fixture = actor_fixture("prepare-kind-cancelled", &["src"]);
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let cancellation = CancellationToken::new();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let state = admitted.staged_state().unwrap();
        cancellation.cancel();
        observed.push((
            ApplyStagingErrorKind::Cancelled,
            admitted.prepare(state).unwrap_err().kind(),
        ));
        fixture.cleanup();

        let fixture = actor_fixture("prepare-kind-deadline", &["src"]);
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let mut admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let state = admitted.staged_state().unwrap();
        admitted.deadline = ProviderDeadline::from_budget(Duration::ZERO);
        observed.push((
            ApplyStagingErrorKind::Deadline,
            admitted.prepare(state).unwrap_err().kind(),
        ));
        fixture.cleanup();

        let fixture = actor_fixture("prepare-kind-containment", &["src"]);
        std::fs::write(fixture.roots[0].join("Module.bsl"), b"original").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let other_fixture = actor_fixture("prepare-kind-containment-other", &["src"]);
        std::fs::write(other_fixture.roots[0].join("Module.bsl"), b"original").unwrap();
        let other_binding = other_fixture
            .actor
            .bind_provider_root("src", &other_fixture.roots[0])
            .unwrap();
        let mut state = ApplyStagedState::from_retained_root(
            Arc::clone(&other_binding.source_root),
            admitted.deadline,
            admitted.cancellation.clone(),
            admitted.writer_authority.clone(),
        );
        state
            .replace("Module.bsl", b"original", b"replacement".to_vec())
            .unwrap();
        observed.push((
            ApplyStagingErrorKind::ContainmentIdentity,
            admitted.prepare(state).unwrap_err().kind(),
        ));
        other_fixture.cleanup();
        fixture.cleanup();

        let fixture = actor_fixture("prepare-kind-occupied", &["src"]);
        std::fs::create_dir(fixture.roots[0].join("Ext")).unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .create("Ext/Form/Module.bsl", b"created".to_vec())
            .unwrap();
        std::fs::create_dir(fixture.roots[0].join("Ext/Form")).unwrap();
        observed.push((
            ApplyStagingErrorKind::AbsentChainOccupied,
            admitted.prepare(state).unwrap_err().kind(),
        ));
        fixture.cleanup();

        let fixture = actor_fixture("prepare-kind-provider", &["src"]);
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state.create("Module.bsl", b"created".to_vec()).unwrap();
        crate::infrastructure::native_operations::compile_transaction::inject_retained_apply_validation_provider_failure_for_test();
        observed.push((
            ApplyStagingErrorKind::UnsupportedProvider,
            admitted.prepare(state).unwrap_err().kind(),
        ));
        fixture.cleanup();

        let fixture = actor_fixture("prepare-kind-invariant", &["src"]);
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .create("Malformed.xml", b"<not-closed".to_vec())
            .unwrap();
        observed.push((
            ApplyStagingErrorKind::Invariant,
            admitted.prepare(state).unwrap_err().kind(),
        ));
        fixture.cleanup();

        assert_eq!(
            observed.len(),
            6,
            "typed staging error matrix is incomplete"
        );
        for (expected, actual) in observed {
            assert_eq!(actual, expected, "actor preparation erased {expected:?}");
        }
    }

    #[test]
    fn apply_preparation_classifies_an_occupied_absent_final_file_by_cause() {
        let fixture = actor_fixture("prepare-occupied-absent-final", &["src"]);
        std::fs::create_dir(fixture.roots[0].join("Ext")).unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .create("Ext/Module.bsl", b"must not replace".to_vec())
            .unwrap();
        std::fs::write(
            fixture.roots[0].join("Ext/Module.bsl"),
            b"external regular file",
        )
        .unwrap();

        let error = admitted.prepare(state).unwrap_err();

        assert_eq!(error.kind(), ApplyStagingErrorKind::AbsentChainOccupied);
        assert_eq!(
            std::fs::read(fixture.roots[0].join("Ext/Module.bsl")).unwrap(),
            b"external regular file"
        );
        fixture.cleanup();
    }

    #[test]
    fn prepared_apply_cleanup_race_surfaces_a_relative_actor_diagnostic() {
        if !crate::infrastructure::platform::testing::can_swap_named_child_behind_retained_handle_for_test() {
            return;
        }
        let fixture = actor_fixture("prepared-cleanup-diagnostic", &["src"]);
        let parent = fixture.roots[0].join("Nested");
        std::fs::create_dir_all(&parent).unwrap();
        let target = parent.join("Module.bsl");
        let owned_recovery = parent.join("owned-recovery.bsl");
        std::fs::write(&target, b"original").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Nested/Module.bsl", b"original", b"published".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        let hook_parent = parent.clone();
        let hook_owned = owned_recovery.clone();
        crate::infrastructure::platform::filesystem::set_before_identity_bound_cleanup_mutation_hook(
            move || {
                let recovery = std::fs::read_dir(&hook_parent)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .find(|path| {
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| name.starts_with(".unica-apply-"))
                    })
                    .expect("retained apply recovery artifact");
                std::fs::rename(&recovery, &hook_owned).unwrap();
                std::fs::write(&recovery, b"concurrent-recovery").unwrap();
            },
        );

        let result = fixture.actor.publish_prepared_apply(prepared).unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"published");
        assert_eq!(std::fs::read(&owned_recovery).unwrap(), b"original");
        assert!(std::fs::read_dir(&parent).unwrap().any(|entry| {
            let path = entry.unwrap().path();
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".unica-apply-"))
                && std::fs::read(path).unwrap() == b"concurrent-recovery"
        }));
        let [diagnostic] = result.cleanup_diagnostics() else {
            panic!("successful actor result discarded the retained cleanup diagnostic: {result:?}");
        };
        assert_eq!(
            diagnostic.kind(),
            super::ApplyCleanupDiagnosticKind::RetainedRecoveryCleanupIncomplete
        );
        assert_eq!(diagnostic.logical_target(), Path::new("Nested/Module.bsl"));
        let artifact_name = diagnostic.last_known_artifact_name();
        assert!(artifact_name.to_string_lossy().starts_with(".unica-apply-"));
        assert_eq!(Path::new(artifact_name).components().count(), 1);
        assert!(!format!("{diagnostic:?}").contains(&fixture.root.display().to_string()));
        fixture.cleanup();
    }

    #[test]
    fn prepared_apply_rejects_stale_revision_source_change_cancellation_and_deadline_before_publication(
    ) {
        let fixture = actor_fixture("prepared-apply-fences", &["src"]);
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"original").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let cancellation = CancellationToken::new();
        assert!(fixture
            .actor
            .admit_apply(
                &binding,
                Some("stale-revision"),
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap_err()
            .to_string()
            .contains("ifRev"));

        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"must-not-publish".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        std::fs::write(&target, b"concurrent").unwrap();
        assert!(fixture
            .actor
            .publish_prepared_apply(prepared)
            .unwrap_err()
            .contains("revision"));
        assert_eq!(std::fs::read(&target).unwrap(), b"concurrent");

        let cancelled = CancellationToken::new();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancelled,
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"concurrent", b"cancelled".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        cancelled.cancel();
        assert!(fixture
            .actor
            .publish_prepared_apply(prepared)
            .unwrap_err()
            .contains("cancel"));
        assert_eq!(std::fs::read(&target).unwrap(), b"concurrent");

        assert!(fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::ZERO),
                &CancellationToken::new(),
            )
            .unwrap_err()
            .to_string()
            .contains("deadline"));
        fixture.cleanup();
    }

    #[test]
    fn prepared_apply_root_and_actor_capabilities_cannot_be_redirected_or_replayed() {
        let first = actor_fixture("prepared-capability-a", &["src"]);
        let second = actor_fixture("prepared-capability-b", &["src"]);
        let target = first.roots[0].join("Module.bsl");
        std::fs::write(&target, b"original").unwrap();
        let binding = first
            .actor
            .bind_provider_root("src", &first.roots[0])
            .unwrap();
        let admitted = first
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"must-not-publish".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        assert!(second
            .actor
            .publish_prepared_apply(prepared)
            .unwrap_err()
            .contains("another workspace actor"));
        assert_eq!(std::fs::read(&target).unwrap(), b"original");

        let admitted = first
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"redirected".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        let displaced = first.root.join("src-displaced");
        let retained_identity = path_identity_for_test(&first.roots[0])
            .unwrap()
            .expect("source root identity must be available on supported CI platforms");
        match attempt_retained_directory_replacement_for_test(&first.roots[0], &displaced).unwrap()
        {
            RetainedDirectoryReplacementOutcome::Replaced => {
                std::fs::create_dir_all(&first.roots[0]).unwrap();
                std::fs::write(first.roots[0].join("Module.bsl"), b"replacement-tree").unwrap();
                assert!(first
                    .actor
                    .publish_prepared_apply(prepared)
                    .unwrap_err()
                    .contains("physical identity"));
                assert_eq!(
                    std::fs::read(displaced.join("Module.bsl")).unwrap(),
                    b"original"
                );
                assert_eq!(
                    std::fs::read(first.roots[0].join("Module.bsl")).unwrap(),
                    b"replacement-tree"
                );
            }
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
                assert_eq!(
                    path_identity_for_test(&first.roots[0]).unwrap().as_deref(),
                    Some(retained_identity.as_str())
                );
                assert!(!displaced.exists());
                first.actor.publish_prepared_apply(prepared).unwrap();
                assert_eq!(std::fs::read(&target).unwrap(), b"redirected");
            }
        }
        first.cleanup();
        second.cleanup();
    }

    #[test]
    fn prepared_apply_root_replacement_during_commit_rolls_back_before_result() {
        let fixture = actor_fixture("prepared-root-race", &["src"]);
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"original").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"published-postimage".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        let named_root = fixture.roots[0].clone();
        let displaced = fixture.root.join("src-race-displaced");
        let hook_displaced = displaced.clone();
        let retained_identity = path_identity_for_test(&named_root)
            .unwrap()
            .expect("source root identity must be available on supported CI platforms");
        let (replacement_tx, replacement_rx) = mpsc::sync_channel(1);
        crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_post_validation_hook(
            move || {
                let replacement = attempt_retained_directory_replacement_for_test(
                    &named_root,
                    &hook_displaced,
                )
                .unwrap();
                if replacement == RetainedDirectoryReplacementOutcome::Replaced {
                    std::fs::create_dir_all(&named_root).unwrap();
                    std::fs::write(named_root.join("Module.bsl"), b"replacement-tree").unwrap();
                }
                replacement_tx.send(replacement).unwrap();
            },
        );

        let publication = fixture.actor.publish_prepared_apply(prepared);
        let replacement = replacement_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("post-validation replacement hook did not report its platform outcome");
        match replacement {
            RetainedDirectoryReplacementOutcome::Replaced => {
                let error = publication.unwrap_err();
                assert!(
                    error.contains("physical identity")
                        || error.contains("identity changed after admission"),
                    "{error}"
                );
                assert_eq!(
                    std::fs::read(displaced.join("Module.bsl")).unwrap(),
                    b"original"
                );
                assert_eq!(
                    std::fs::read(fixture.roots[0].join("Module.bsl")).unwrap(),
                    b"replacement-tree"
                );
            }
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
                assert_eq!(
                    path_identity_for_test(&fixture.roots[0])
                        .unwrap()
                        .as_deref(),
                    Some(retained_identity.as_str())
                );
                assert!(!displaced.exists());
                publication.unwrap();
                assert_eq!(std::fs::read(&target).unwrap(), b"published-postimage");
            }
        }
        fixture.cleanup();
    }

    #[test]
    fn postimage_validation_preserves_typed_root_containment_cause() {
        let fixture = actor_fixture("typed-postimage-root", &["src"]);
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"original").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"published".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        let named_root = fixture.roots[0].clone();
        let displaced = fixture.root.join("typed-postimage-root-displaced");
        let hook_displaced = displaced.clone();
        let retained_identity = path_identity_for_test(&named_root)
            .unwrap()
            .expect("source root identity must be available on supported CI platforms");
        let (replacement_tx, replacement_rx) = mpsc::sync_channel(1);
        crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_postimages_hook(
            move || {
                let replacement = attempt_retained_directory_replacement_for_test(
                    &named_root,
                    &hook_displaced,
                )
                .unwrap();
                if replacement == RetainedDirectoryReplacementOutcome::Replaced {
                    std::fs::create_dir_all(&named_root).unwrap();
                    std::fs::write(named_root.join("foreign.txt"), b"foreign").unwrap();
                }
                replacement_tx.send(replacement).unwrap();
            },
        );

        let publication = fixture.actor.publish_prepared_apply(prepared);
        let replacement = replacement_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("postimage replacement hook did not report its platform outcome");
        match replacement {
            RetainedDirectoryReplacementOutcome::Replaced => {
                let error = publication.unwrap_err();
                assert_eq!(
                    error.kind(),
                    super::ApplyPublicationErrorKind::ContainmentIdentity
                );
                assert_eq!(
                    std::fs::read(displaced.join("Module.bsl")).unwrap(),
                    b"original"
                );
                assert_eq!(
                    std::fs::read(fixture.roots[0].join("foreign.txt")).unwrap(),
                    b"foreign"
                );
            }
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
                assert_eq!(
                    path_identity_for_test(&fixture.roots[0])
                        .unwrap()
                        .as_deref(),
                    Some(retained_identity.as_str())
                );
                assert!(!displaced.exists());
                publication.unwrap();
                assert_eq!(std::fs::read(&target).unwrap(), b"published");
            }
        }
        fixture.cleanup();
    }

    #[test]
    fn postimage_validation_preserves_typed_absent_chain_containment_cause() {
        let fixture = actor_fixture("typed-postimage-absent-chain", &["src"]);
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .create("Nested/Module.bsl", b"published".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        let nested = fixture.roots[0].join("Nested");
        let displaced = fixture.roots[0].join("Nested-displaced");
        let hook_nested = nested.clone();
        let hook_displaced = displaced.clone();
        let (replacement_tx, replacement_rx) = mpsc::sync_channel(1);
        crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_postimages_hook(
            move || {
                let retained_identity = path_identity_for_test(&hook_nested)
                    .unwrap()
                    .expect("created parent identity must be available on supported CI platforms");
                let replacement = attempt_retained_directory_replacement_for_test(
                    &hook_nested,
                    &hook_displaced,
                )
                .unwrap();
                if replacement == RetainedDirectoryReplacementOutcome::Replaced {
                    std::fs::create_dir_all(&hook_nested).unwrap();
                    std::fs::write(hook_nested.join("foreign.txt"), b"foreign").unwrap();
                    let rollback_nested = hook_nested.clone();
                    let rollback_displaced = hook_displaced.clone();
                    crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_rollback_hook(
                        move || {
                            std::fs::remove_dir_all(&rollback_nested).unwrap();
                            std::fs::rename(&rollback_displaced, &rollback_nested).unwrap();
                        },
                    );
                }
                replacement_tx
                    .send((replacement, retained_identity))
                    .unwrap();
            },
        );

        let publication = fixture.actor.publish_prepared_apply(prepared);
        let (replacement, retained_identity) = replacement_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("absent-chain replacement hook did not report its platform outcome");
        match replacement {
            RetainedDirectoryReplacementOutcome::Replaced => {
                let error = publication.unwrap_err();
                assert_eq!(
                    error.kind(),
                    super::ApplyPublicationErrorKind::ContainmentIdentity
                );
                assert!(
                    !nested.exists(),
                    "transaction-created parent was not rolled back"
                );
                assert!(!displaced.exists(), "test race recovery name leaked");
            }
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
                assert_eq!(
                    path_identity_for_test(&nested).unwrap().as_deref(),
                    Some(retained_identity.as_str())
                );
                assert!(!displaced.exists());
                publication.unwrap();
                assert_eq!(
                    std::fs::read(nested.join("Module.bsl")).unwrap(),
                    b"published"
                );
            }
        }
        fixture.cleanup();
    }

    #[test]
    fn prepared_apply_create_post_rename_failure_rolls_back_the_published_name() {
        let fixture = actor_fixture("prepared-create-post-rename-failure", &["src"]);
        let target = fixture.roots[0].join("created.bsl");
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state.create("created.bsl", b"published".to_vec()).unwrap();
        let prepared = admitted.prepare(state).unwrap();
        crate::infrastructure::platform::filesystem::inject_post_rename_sync_failure_for_test();

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        assert!(error.contains("post-rename"), "{error}");
        assert!(
            !target.exists(),
            "reported failure left created bytes published"
        );
        fixture.cleanup();
    }

    #[test]
    fn apply_admission_refuses_staged_state_from_another_actor_issued_authority() {
        let fixture = actor_fixture("prepared-foreign-writer-authority", &["src"]);
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let cancellation = CancellationToken::new();
        let first = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let second = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let foreign_state = second.staged_state().unwrap();

        let error = first.prepare(foreign_state).unwrap_err();
        assert_eq!(error.kind(), ApplyStagingErrorKind::Invariant);
        fixture.cleanup();
    }

    #[test]
    fn prepared_apply_replace_post_rename_failure_restores_the_exact_preimage() {
        let fixture = actor_fixture("prepared-replace-post-rename-failure", &["src"]);
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"original").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"published".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        crate::infrastructure::platform::filesystem::inject_post_rename_sync_failure_for_test();

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        assert!(error.contains("post-rename"), "{error}");
        assert_eq!(std::fs::read(&target).unwrap(), b"original");
        fixture.cleanup();
    }

    #[test]
    fn prepared_apply_replace_preserves_a_destination_replaced_at_the_mutation_boundary() {
        let fixture = actor_fixture("prepared-replace-destination-race", &["src"]);
        let target = fixture.roots[0].join("Module.bsl");
        let displaced = fixture.roots[0].join("concurrent-old.bsl");
        std::fs::write(&target, b"original").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"apply-bytes".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        let raced_target = target.clone();
        let raced_displaced = displaced.clone();
        crate::infrastructure::platform::filesystem::set_before_identity_bound_no_replace_rename_hook(
            move || {
                std::fs::rename(&raced_target, &raced_displaced).unwrap();
                std::fs::write(&raced_target, b"concurrent").unwrap();
            },
        );

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        assert!(
            error.contains("preimage") || error.contains("identity"),
            "{error}"
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"concurrent");
        fixture.cleanup();
    }

    #[test]
    fn prepared_apply_remove_preserves_a_destination_replaced_at_the_mutation_boundary() {
        let fixture = actor_fixture("prepared-remove-destination-race", &["src"]);
        let target = fixture.roots[0].join("Module.bsl");
        let displaced = fixture.roots[0].join("concurrent-old.bsl");
        std::fs::write(&target, b"original").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state.remove("Module.bsl", b"original").unwrap();
        let prepared = admitted.prepare(state).unwrap();
        let raced_target = target.clone();
        let raced_displaced = displaced.clone();
        crate::infrastructure::platform::filesystem::set_before_identity_bound_no_replace_rename_hook(
            move || {
                std::fs::rename(&raced_target, &raced_displaced).unwrap();
                std::fs::write(&raced_target, b"concurrent").unwrap();
            },
        );

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        assert!(
            error.contains("preimage") || error.contains("identity"),
            "{error}"
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"concurrent");
        fixture.cleanup();
    }

    #[test]
    fn prepared_apply_dry_run_rejects_root_replacement_before_result() {
        let fixture = actor_fixture("prepared-dry-root-race", &["src"]);
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"original").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"dry-postimage".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        let named_root = fixture.roots[0].clone();
        let displaced = fixture.root.join("src-dry-race-displaced");
        let hook_displaced = displaced.clone();
        let retained_identity = path_identity_for_test(&named_root)
            .unwrap()
            .expect("source root identity must be available on supported CI platforms");
        let (replacement_tx, replacement_rx) = mpsc::sync_channel(1);
        set_apply_dry_run_after_confirmation_hook(move || {
            let replacement =
                attempt_retained_directory_replacement_for_test(&named_root, &hook_displaced)
                    .unwrap();
            if replacement == RetainedDirectoryReplacementOutcome::Replaced {
                std::fs::create_dir_all(&named_root).unwrap();
                std::fs::write(named_root.join("Module.bsl"), b"replacement-tree").unwrap();
            }
            replacement_tx.send(replacement).unwrap();
        });

        let publication = fixture.actor.publish_prepared_apply(prepared);
        let replacement = replacement_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("dry-run replacement hook did not report its platform outcome");
        match replacement {
            RetainedDirectoryReplacementOutcome::Replaced => {
                let error = publication.unwrap_err();
                assert!(error.contains("physical identity"), "{error}");
                assert_eq!(
                    std::fs::read(displaced.join("Module.bsl")).unwrap(),
                    b"original"
                );
                assert_eq!(
                    std::fs::read(fixture.roots[0].join("Module.bsl")).unwrap(),
                    b"replacement-tree"
                );
            }
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
                assert_eq!(
                    path_identity_for_test(&fixture.roots[0])
                        .unwrap()
                        .as_deref(),
                    Some(retained_identity.as_str())
                );
                assert!(!displaced.exists());
                publication.unwrap();
                assert_eq!(std::fs::read(&target).unwrap(), b"original");
            }
        }
        fixture.cleanup();
    }

    #[test]
    fn prepared_apply_effects_are_retained_from_planner_to_result() {
        let fixture = actor_fixture("effect-receipt-retained", &["main"]);
        write_actor_event_fixture(&fixture.roots[0]);
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let (state, effects) = plan_actor_events(
            &admitted,
            &[event_operation(
                "main:Catalog.Products.Form.Main.Event.OnOpen",
            )],
        )
        .unwrap();

        let result = fixture
            .actor
            .publish_prepared_apply(admitted.prepare_with_effects(state, effects).unwrap())
            .unwrap();
        let receipt = result.effects();

        assert_form_module_effect_subject(receipt, ApplyEffectDisposition::Projected);
        fixture.cleanup();
    }

    #[test]
    fn prepared_apply_dry_run_returns_projected_effect_receipt_without_any_write() {
        let fixture = actor_fixture("effect-receipt-dry-run", &["main"]);
        write_actor_event_fixture(&fixture.roots[0]);
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_root = fixture.root.join(".build/unica");
        let cache_before = snapshot_tree(&cache_root);
        let machine_before = service.machine_state_for_test();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let admitted_rev = admitted.revision_identity().to_string();
        let (state, effects) = plan_actor_events(
            &admitted,
            &[event_operation(
                "main:Catalog.Products.Form.Main.Event.OnOpen",
            )],
        )
        .unwrap();
        let prepared = admitted.prepare_with_effects(state, effects).unwrap();

        let result = fixture.actor.publish_prepared_apply(prepared).unwrap();
        let receipt = result.effects();

        assert_form_module_effect_subject(receipt, ApplyEffectDisposition::Projected);
        assert_eq!(result.rev(), admitted_rev);
        assert_eq!(result.commit_count_for_test(), 0);
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(snapshot_tree(&cache_root), cache_before);
        assert_eq!(service.machine_state_for_test(), machine_before);
        assert!(std::ptr::eq(receipt, result.effects()));
        assert!(
            !cache_root.exists(),
            "dry run created the absent cache root"
        );
        fixture.cleanup();
    }

    #[test]
    fn prepared_apply_success_returns_committed_effect_receipt_after_one_commit() {
        use crate::infrastructure::native_operations::compile_transaction::RetainedApplyObservedEvent;

        let fixture = actor_fixture("effect-receipt-commit", &["main"]);
        write_actor_event_fixture(&fixture.roots[0]);
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let admitted_rev = admitted.revision_identity().to_string();
        let (state, effects) = plan_actor_events(
            &admitted,
            &[event_operation(
                "main:Catalog.Products.Form.Main.Event.OnOpen",
            )],
        )
        .unwrap();
        let prepared = admitted.prepare_with_effects(state, effects).unwrap();
        let _ = crate::infrastructure::native_operations::compile_transaction::take_retained_apply_observed_events();

        let result = fixture.actor.publish_prepared_apply(prepared).unwrap();
        let receipt = result.effects();

        assert_form_module_effect_subject(receipt, ApplyEffectDisposition::Committed);
        assert_ne!(result.rev(), admitted_rev);
        assert_eq!(result.commit_count_for_test(), 1);
        assert!(fixture.roots[0]
            .join("Catalogs/Products/Forms/Main/Ext/Form/Module.bsl")
            .is_file());
        let observed = crate::infrastructure::native_operations::compile_transaction::take_retained_apply_observed_events();
        assert!(
            matches!(
                observed.as_slice(),
                [
                    RetainedApplyObservedEvent::Source(_),
                    RetainedApplyObservedEvent::Source(_),
                    RetainedApplyObservedEvent::Source(_),
                    RetainedApplyObservedEvent::Source(_),
                    RetainedApplyObservedEvent::Source(_),
                    RetainedApplyObservedEvent::EagerMetadata(_),
                    RetainedApplyObservedEvent::RevisionRecord(_),
                    RetainedApplyObservedEvent::StateMarker(_),
                ]
            ),
            "receipt became committed outside the one retained publication: {observed:?}"
        );
        fixture.cleanup();
    }

    #[test]
    fn event_implement_planner_integrates_with_actor_effect_publication_matrix() {
        for (name, target, missing, expected) in [
            (
                "platform-available",
                "main:Catalog.Products.Module.Object.Event.BeforeWrite",
                false,
                vec![DomainEventKind::ModuleChanged],
            ),
            (
                "property-available",
                "main:Catalog.Products.Form.Main.Event.OnOpen",
                false,
                vec![DomainEventKind::FormChanged, DomainEventKind::ModuleChanged],
            ),
            (
                "property-missing",
                "main:Catalog.Products.Form.Main.Event.OnOpen",
                true,
                vec![DomainEventKind::ModuleChanged],
            ),
        ] {
            let fixture = actor_fixture(&format!("effect-matrix-{name}"), &["main"]);
            write_actor_event_fixture(&fixture.roots[0]);
            if missing {
                write_missing_on_open_binding(&fixture.roots[0]);
            }
            let binding = fixture
                .actor
                .bind_provider_root("main", &fixture.roots[0])
                .unwrap();
            let admitted = fixture
                .actor
                .admit_apply(
                    &binding,
                    None,
                    true,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap();
            let (state, effects) =
                plan_actor_events(&admitted, &[event_operation(target)]).unwrap();
            let result = fixture
                .actor
                .publish_prepared_apply(admitted.prepare_with_effects(state, effects).unwrap())
                .unwrap();
            let receipt = result.effects();
            assert_eq!(
                receipt
                    .events()
                    .iter()
                    .map(|event| event.kind)
                    .collect::<Vec<_>>(),
                expected,
                "{name}"
            );
            fixture.cleanup();
        }

        let operations = [
            event_operation("main:Catalog.Products.Form.Main.Event.OnOpen"),
            event_operation("main:Catalog.Products.Form.Main.Event.OnClose"),
        ];
        let mut subjects = Vec::new();
        for dry_run in [true, false] {
            let fixture = actor_fixture(
                if dry_run {
                    "effect-matrix-dry-parity"
                } else {
                    "effect-matrix-real-parity"
                },
                &["main"],
            );
            write_actor_event_fixture(&fixture.roots[0]);
            let binding = fixture
                .actor
                .bind_provider_root("main", &fixture.roots[0])
                .unwrap();
            let admitted = fixture
                .actor
                .admit_apply(
                    &binding,
                    None,
                    dry_run,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap();
            let (state, effects) = plan_actor_events(&admitted, &operations).unwrap();
            let result = fixture
                .actor
                .publish_prepared_apply(admitted.prepare_with_effects(state, effects).unwrap())
                .unwrap();
            let receipt = result.effects();
            subjects.push((
                receipt.events().to_vec(),
                receipt.cache().events.clone(),
                receipt.cache().invalidated.clone(),
                receipt.cache().refreshed.clone(),
            ));
            fixture.cleanup();
        }
        assert_eq!(
            subjects[0], subjects[1],
            "dry/real receipt subjects diverged"
        );
        assert_eq!(
            subjects[0]
                .0
                .iter()
                .map(|event| event.kind)
                .collect::<Vec<_>>(),
            vec![DomainEventKind::FormChanged, DomainEventKind::ModuleChanged],
            "multi-op planner lost stable first-occurrence deduplication"
        );
    }

    #[test]
    fn event_implement_op_failure_returns_no_effect_receipt_and_preserves_all_state() {
        let control = actor_fixture("effect-poison-control", &["main"]);
        write_actor_event_fixture(&control.roots[0]);
        let control_binding = control
            .actor
            .bind_provider_root("main", &control.roots[0])
            .unwrap();
        let control_admission = control
            .actor
            .admit_apply(
                &control_binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let (control_state, control_effects) = plan_actor_events(
            &control_admission,
            &[event_operation(
                "main:Catalog.Products.Form.Main.Event.OnOpen",
            )],
        )
        .unwrap();
        let control_result = control
            .actor
            .publish_prepared_apply(
                control_admission
                    .prepare_with_effects(control_state, control_effects)
                    .unwrap(),
            )
            .unwrap();
        assert_form_module_effect_subject(
            control_result.effects(),
            ApplyEffectDisposition::Projected,
        );
        control.cleanup();

        for dry_run in [true, false] {
            let fixture = actor_fixture(
                if dry_run {
                    "effect-poison-dry"
                } else {
                    "effect-poison-real"
                },
                &["main"],
            );
            write_actor_event_fixture(&fixture.roots[0]);
            let binding = fixture
                .actor
                .bind_provider_root("main", &fixture.roots[0])
                .unwrap();
            let service = fixture.actor.source_revision_service(&binding).unwrap();
            let source_before = snapshot_tree(&fixture.roots[0]);
            let cache_root = fixture.root.join(".build/unica");
            let cache_before = snapshot_tree(&cache_root);
            let machine_before = service.machine_state_for_test();
            let admitted = fixture
                .actor
                .admit_apply(
                    &binding,
                    None,
                    dry_run,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap();
            let operations = [
                event_operation("main:Catalog.Products.Form.Main.Event.OnOpen"),
                event_operation("main:Catalog.Products.Form.Main.Event.DoesNotExist"),
            ];

            let error = plan_actor_events(&admitted, &operations)
                .expect_err("poisoned operation batch unexpectedly reached preparation");

            assert_eq!(
                error.kind(),
                crate::infrastructure::native_operations::event::EventPlanErrorKind::ProviderUnavailable
            );
            assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
            assert_eq!(snapshot_tree(&cache_root), cache_before);
            assert_eq!(service.machine_state_for_test(), machine_before);
            assert!(!cache_root.exists());
            fixture.cleanup();
        }
    }

    #[test]
    fn retained_apply_effect_failure_matrix_rolls_back_and_returns_no_receipt() {
        use crate::infrastructure::native_operations::compile_transaction::RetainedApplyFailpoint;

        for (name, failpoint) in [
            ("second-source", RetainedApplyFailpoint::Source(2)),
            ("eager-cache", RetainedApplyFailpoint::EagerMetadata(1)),
            ("revision-record", RetainedApplyFailpoint::RevisionRecord),
            ("state-marker", RetainedApplyFailpoint::StateMarker),
            (
                "final-validation",
                RetainedApplyFailpoint::AfterAllPostimages,
            ),
        ] {
            let fixture = actor_fixture(&format!("effect-failure-{name}"), &["main"]);
            write_actor_event_fixture(&fixture.roots[0]);
            let binding = fixture
                .actor
                .bind_provider_root("main", &fixture.roots[0])
                .unwrap();
            let service = fixture.actor.source_revision_service(&binding).unwrap();
            let prepared =
                prepare_property_effect_batch(&fixture, &binding, false, &CancellationToken::new());
            let prepared_receipt = &prepared.effects;
            assert_eq!(
                prepared_receipt
                    .events
                    .iter()
                    .map(|event| event.kind)
                    .collect::<Vec<_>>(),
                [DomainEventKind::FormChanged, DomainEventKind::ModuleChanged]
            );
            assert_eq!(prepared_receipt.cache.mode, "applied");
            let source_before = snapshot_tree(&fixture.roots[0]);
            let cache_root = fixture.root.join(".build/unica");
            let cache_before = snapshot_tree(&cache_root);
            let machine_before = service.machine_state_for_test();
            crate::infrastructure::native_operations::compile_transaction::set_retained_apply_failpoint(
                failpoint,
            );

            let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

            assert_eq!(
                error.kind(),
                super::ApplyPublicationErrorKind::ProviderPostvalidation,
                "{name}: {error}"
            );
            assert_eq!(snapshot_tree(&fixture.roots[0]), source_before, "{name}");
            assert_eq!(snapshot_tree(&cache_root), cache_before, "{name}");
            assert_eq!(service.machine_state_for_test(), machine_before, "{name}");
            fixture.cleanup();
        }

        let fixture = actor_fixture("effect-failure-final-cancel", &["main"]);
        write_actor_event_fixture(&fixture.roots[0]);
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let cancellation = CancellationToken::new();
        let prepared = prepare_property_effect_batch(&fixture, &binding, false, &cancellation);
        assert!(!prepared.effects.events.is_empty());
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_root = fixture.root.join(".build/unica");
        let cache_before = snapshot_tree(&cache_root);
        let cancel_at_final_gate = cancellation.clone();
        crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_post_validation_hook(
            move || cancel_at_final_gate.cancel(),
        );
        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Cancelled);
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(snapshot_tree(&cache_root), cache_before);
        fixture.cleanup();

        let fixture = actor_fixture("effect-failure-rollback-incomplete", &["main"]);
        write_actor_event_fixture(&fixture.roots[0]);
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let cancellation = CancellationToken::new();
        let prepared = prepare_property_effect_batch(&fixture, &binding, false, &cancellation);
        assert!(!prepared.effects.events.is_empty());
        let target = fixture.roots[0].join("Catalogs/Products/Forms/Main/Ext/Form.xml");
        let moved =
            fixture.roots[0].join("Catalogs/Products/Forms/Main/Ext/Form-published-moved.xml");
        let raced_target = target.clone();
        let raced_moved = moved.clone();
        let cancel = cancellation.clone();
        crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_post_validation_hook(
            move || {
                std::fs::rename(&raced_target, &raced_moved).unwrap();
                std::fs::write(&raced_target, b"foreign-form").unwrap();
                cancel.cancel();
            },
        );

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::RollbackIncomplete
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"foreign-form");
        assert!(
            moved.is_file(),
            "foreign replacement erased published evidence"
        );
        fixture.cleanup();
    }

    #[test]
    fn retained_apply_effect_races_never_publish_or_return_effects() {
        let stale = actor_fixture("effect-race-stale", &["main"]);
        write_actor_event_fixture(&stale.roots[0]);
        let stale_binding = stale
            .actor
            .bind_provider_root("main", &stale.roots[0])
            .unwrap();
        let stale_error = stale
            .actor
            .admit_apply(
                &stale_binding,
                Some("stale-revision"),
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap_err();
        assert!(stale_error.to_string().contains("ifRev"));
        assert!(!stale.root.join(".build/unica").exists());
        stale.cleanup();

        let fixture = actor_fixture("effect-race-revision", &["main"]);
        write_actor_event_fixture(&fixture.roots[0]);
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let prepared =
            prepare_property_effect_batch(&fixture, &binding, false, &CancellationToken::new());
        assert!(
            !prepared.effects.events.is_empty(),
            "racing prepared batch discarded the receipt subject before the gate"
        );
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        std::fs::write(fixture.roots[0].join("Concurrent.bsl"), b"foreign").unwrap();
        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::ConcurrentRevision
        );
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        assert_eq!(
            std::fs::read(fixture.roots[0].join("Concurrent.bsl")).unwrap(),
            b"foreign"
        );
        fixture.cleanup();

        let fixture = actor_fixture("effect-race-source-root", &["main"]);
        write_actor_event_fixture(&fixture.roots[0]);
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let prepared =
            prepare_property_effect_batch(&fixture, &binding, false, &CancellationToken::new());
        assert!(!prepared.effects.events.is_empty());
        let displaced = fixture.root.join("main-displaced");
        let retained_identity = path_identity_for_test(&fixture.roots[0])
            .unwrap()
            .expect("source root identity must be available on supported CI platforms");
        match attempt_retained_directory_replacement_for_test(&fixture.roots[0], &displaced)
            .unwrap()
        {
            RetainedDirectoryReplacementOutcome::Replaced => {
                std::fs::create_dir_all(&fixture.roots[0]).unwrap();
                std::fs::write(fixture.roots[0].join("foreign.txt"), b"foreign").unwrap();
                let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
                assert_eq!(
                    error.kind(),
                    super::ApplyPublicationErrorKind::ContainmentIdentity
                );
                assert_eq!(
                    std::fs::read(fixture.roots[0].join("foreign.txt")).unwrap(),
                    b"foreign"
                );
            }
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
                assert_eq!(
                    path_identity_for_test(&fixture.roots[0])
                        .unwrap()
                        .as_deref(),
                    Some(retained_identity.as_str())
                );
                assert!(!displaced.exists());
                let result = fixture.actor.publish_prepared_apply(prepared).unwrap();
                assert!(!result.effects().events().is_empty());
                assert!(!fixture.roots[0].join("foreign.txt").exists());
            }
        }
        fixture.cleanup();

        let fixture = actor_fixture("effect-race-cache-root", &["main"]);
        write_actor_event_fixture(&fixture.roots[0]);
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let prepared =
            prepare_property_effect_batch(&fixture, &binding, false, &CancellationToken::new());
        assert!(!prepared.effects.events.is_empty());
        std::fs::write(fixture.root.join(".build"), b"foreign-cache-parent").unwrap();
        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::ContainmentIdentity
        );
        assert_eq!(
            std::fs::read(fixture.root.join(".build")).unwrap(),
            b"foreign-cache-parent"
        );
        fixture.cleanup();

        for gate in ["cancelled", "deadline"] {
            let fixture = actor_fixture(&format!("effect-race-{gate}"), &["main"]);
            write_actor_event_fixture(&fixture.roots[0]);
            let binding = fixture
                .actor
                .bind_provider_root("main", &fixture.roots[0])
                .unwrap();
            let cancellation = CancellationToken::new();
            let mut prepared =
                prepare_property_effect_batch(&fixture, &binding, false, &cancellation);
            assert!(!prepared.effects.events.is_empty());
            if gate == "cancelled" {
                cancellation.cancel();
            } else {
                prepared.deadline = ProviderDeadline::from_budget(Duration::ZERO);
            }
            let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
            assert_eq!(
                error.kind(),
                if gate == "cancelled" {
                    super::ApplyPublicationErrorKind::Cancelled
                } else {
                    super::ApplyPublicationErrorKind::Deadline
                }
            );
            fixture.cleanup();
        }

        let fixture = actor_fixture("effect-race-dry-final", &["main"]);
        write_actor_event_fixture(&fixture.roots[0]);
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let cancellation = CancellationToken::new();
        let prepared = prepare_property_effect_batch(&fixture, &binding, true, &cancellation);
        assert!(!prepared.effects.events.is_empty());
        let cancel = cancellation.clone();
        set_apply_dry_run_after_confirmation_hook(move || cancel.cancel());
        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Cancelled);
        fixture.cleanup();

        let fixture = actor_fixture("effect-race-trust-epoch", &["main"]);
        write_actor_event_fixture(&fixture.roots[0]);
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();
        let prepared =
            prepare_property_effect_batch(&fixture, &binding, false, &CancellationToken::new());
        assert!(!prepared.effects.events.is_empty());
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        let race_service = Arc::clone(&service);
        crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_post_validation_hook(
            move || race_service.mark_dirty(),
        );
        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::ConcurrentRevision
        );
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        fixture.cleanup();
    }

    #[test]
    fn real_effect_foreign_actor_replay_preserves_both_actor_states() {
        let first = actor_fixture("effect-foreign-actor-first", &["main"]);
        let second = actor_fixture("effect-foreign-actor-second", &["main"]);
        write_actor_event_fixture(&first.roots[0]);
        write_actor_event_fixture(&second.roots[0]);
        let first_binding = first
            .actor
            .bind_provider_root("main", &first.roots[0])
            .unwrap();
        let second_binding = second
            .actor
            .bind_provider_root("main", &second.roots[0])
            .unwrap();
        let first_service = first.actor.source_revision_service(&first_binding).unwrap();
        let second_service = second
            .actor
            .source_revision_service(&second_binding)
            .unwrap();
        let prepared =
            prepare_property_effect_batch(&first, &first_binding, false, &CancellationToken::new());
        assert_prepared_form_module_effect_subject(&prepared);
        let first_source_before = snapshot_tree(&first.roots[0]);
        let first_cache_before = snapshot_tree(&first.root.join(".build/unica"));
        let first_machine_before = first_service.machine_state_for_test();
        let second_source_before = snapshot_tree(&second.roots[0]);
        let second_cache_before = snapshot_tree(&second.root.join(".build/unica"));
        let second_machine_before = second_service.machine_state_for_test();

        let error = second.actor.publish_prepared_apply(prepared).unwrap_err();

        assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Invariant);
        assert!(error.contains("another workspace actor"), "{error}");
        assert_eq!(snapshot_tree(&first.roots[0]), first_source_before);
        assert_eq!(
            snapshot_tree(&first.root.join(".build/unica")),
            first_cache_before
        );
        assert_eq!(first_service.machine_state_for_test(), first_machine_before);
        assert_eq!(snapshot_tree(&second.roots[0]), second_source_before);
        assert_eq!(
            snapshot_tree(&second.root.join(".build/unica")),
            second_cache_before
        );
        assert_eq!(
            second_service.machine_state_for_test(),
            second_machine_before
        );
        first.cleanup();
        second.cleanup();
    }

    #[test]
    fn real_effect_mutation_lane_cancellation_preserves_exact_state() {
        let fixture = actor_fixture("effect-late-lane-cancel", &["main"]);
        write_actor_event_fixture(&fixture.roots[0]);
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();
        let cancellation = CancellationToken::new();
        let prepared = prepare_property_effect_batch(&fixture, &binding, false, &cancellation);
        assert_prepared_form_module_effect_subject(&prepared);
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        let machine_before = service.machine_state_for_test();
        let _ = crate::infrastructure::native_operations::compile_transaction::take_retained_apply_observed_events();
        let owner = fixture.actor.mutation_lane.hold_for_test();
        let actor = Arc::clone(&fixture.actor);
        let (started_tx, started_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let publisher = thread::spawn(move || {
            started_tx.send(()).unwrap();
            result_tx
                .send(actor.publish_prepared_apply(prepared))
                .unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(
            result_rx.recv_timeout(Duration::from_millis(50)).is_err(),
            "real-effect publication crossed the held mutation lane"
        );
        cancellation.cancel();

        let error = result_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("late cancellation did not stop real-effect lane wait")
            .unwrap_err();
        assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Cancelled);
        assert!(
            crate::infrastructure::native_operations::compile_transaction::take_retained_apply_observed_events()
                .is_empty(),
            "lane cancellation reached retained publication"
        );
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        assert_eq!(service.machine_state_for_test(), machine_before);
        drop(owner);
        publisher.join().unwrap();
        fixture.cleanup();
    }

    #[test]
    fn real_effect_mutation_lane_deadline_preserves_exact_state() {
        let fixture = actor_fixture("effect-late-lane-deadline", &["main"]);
        write_actor_event_fixture(&fixture.roots[0]);
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();
        let cancellation = CancellationToken::new();
        let mut prepared = prepare_property_effect_batch(&fixture, &binding, false, &cancellation);
        assert_prepared_form_module_effect_subject(&prepared);
        prepared.deadline = ProviderDeadline::from_budget(Duration::from_millis(40));
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        let machine_before = service.machine_state_for_test();
        let _ = crate::infrastructure::native_operations::compile_transaction::take_retained_apply_observed_events();
        let owner = fixture.actor.mutation_lane.hold_for_test();
        let cancel_after_deadline = cancellation.clone();
        crate::infrastructure::deadline_lock::set_after_deadline_error_hook_for_test(move || {
            std::thread::spawn(move || cancel_after_deadline.cancel())
                .join()
                .unwrap();
        });

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

        assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Deadline);
        assert!(cancellation.is_cancelled());
        assert!(
            crate::infrastructure::native_operations::compile_transaction::take_retained_apply_observed_events()
                .is_empty(),
            "lane deadline reached retained publication"
        );
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        assert_eq!(service.machine_state_for_test(), machine_before);
        drop(owner);
        fixture.cleanup();
    }

    #[test]
    fn real_effect_mid_scan_cancellation_preserves_exact_state() {
        use crate::infrastructure::source_revision::{
            set_repeating_retained_scan_test_mutation, RetainedScanTestMutationPoint,
        };

        let fixture = actor_fixture("effect-mid-scan-cancel", &["main"]);
        write_actor_event_fixture(&fixture.roots[0]);
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();
        let cancellation = CancellationToken::new();
        let prepared = prepare_property_effect_batch(&fixture, &binding, false, &cancellation);
        assert_prepared_form_module_effect_subject(&prepared);
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        let machine_before = service.machine_state_for_test();
        let _ = crate::infrastructure::native_operations::compile_transaction::take_retained_apply_observed_events();
        let scan_ran = Arc::new(AtomicBool::new(false));
        let scan_observed = Arc::clone(&scan_ran);
        let scan_count = Arc::new(AtomicUsize::new(0));
        let scan_counter = Arc::clone(&scan_count);
        let cancel_during_scan = cancellation.clone();
        let _mutation = set_repeating_retained_scan_test_mutation(
            RetainedScanTestMutationPoint::AfterDirectoryEnumeration,
            move || {
                if scan_counter.fetch_add(1, Ordering::AcqRel) == 2 {
                    scan_observed.store(true, Ordering::Release);
                    cancel_during_scan.cancel();
                }
            },
        );

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

        assert!(scan_ran.load(Ordering::Acquire));
        assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Cancelled);
        assert!(
            crate::infrastructure::native_operations::compile_transaction::take_retained_apply_observed_events()
                .is_empty(),
            "mid-scan cancellation reached retained publication"
        );
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        assert_eq!(service.machine_state_for_test(), machine_before);
        fixture.cleanup();
    }

    #[test]
    fn real_effect_mid_scan_deadline_preserves_exact_state() {
        use crate::infrastructure::source_revision::{
            set_repeating_retained_scan_test_mutation, RetainedScanTestMutationPoint,
        };

        let fixture = actor_fixture("effect-mid-scan-deadline", &["main"]);
        write_actor_event_fixture(&fixture.roots[0]);
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();
        let mut prepared =
            prepare_property_effect_batch(&fixture, &binding, false, &CancellationToken::new());
        assert_prepared_form_module_effect_subject(&prepared);
        prepared.deadline = ProviderDeadline::from_budget(Duration::from_millis(200));
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        let machine_before = service.machine_state_for_test();
        let _ = crate::infrastructure::native_operations::compile_transaction::take_retained_apply_observed_events();
        let scan_ran = Arc::new(AtomicBool::new(false));
        let scan_observed = Arc::clone(&scan_ran);
        let scan_count = Arc::new(AtomicUsize::new(0));
        let scan_counter = Arc::clone(&scan_count);
        let _mutation = set_repeating_retained_scan_test_mutation(
            RetainedScanTestMutationPoint::AfterDirectoryEnumeration,
            move || {
                if scan_counter.fetch_add(1, Ordering::AcqRel) == 2 {
                    scan_observed.store(true, Ordering::Release);
                    std::thread::sleep(Duration::from_millis(250));
                }
            },
        );

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

        assert!(scan_ran.load(Ordering::Acquire));
        assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Deadline);
        assert!(
            crate::infrastructure::native_operations::compile_transaction::take_retained_apply_observed_events()
                .is_empty(),
            "mid-scan deadline reached retained publication"
        );
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        assert_eq!(service.machine_state_for_test(), machine_before);
        fixture.cleanup();
    }

    #[test]
    fn real_effect_after_all_postimages_cancellation_rolls_back_exact_state() {
        use crate::infrastructure::native_operations::compile_transaction::RetainedApplyObservedEvent;

        let fixture = actor_fixture("effect-after-postimages-cancel", &["main"]);
        write_actor_event_fixture(&fixture.roots[0]);
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();
        let cancellation = CancellationToken::new();
        let prepared = prepare_property_effect_batch(&fixture, &binding, false, &cancellation);
        assert_prepared_form_module_effect_subject(&prepared);
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        let machine_before = service.machine_state_for_test();
        let _ = crate::infrastructure::native_operations::compile_transaction::take_retained_apply_observed_events();
        let cancel_after_postimages = cancellation.clone();
        crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_post_validation_hook(
            move || cancel_after_postimages.cancel(),
        );

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

        assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Cancelled);
        let observed = crate::infrastructure::native_operations::compile_transaction::take_retained_apply_observed_events();
        assert!(observed
            .iter()
            .any(|event| matches!(event, RetainedApplyObservedEvent::StateMarker(_))));
        assert!(
            observed
                .iter()
                .any(|event| matches!(event, RetainedApplyObservedEvent::Rollback(_))),
            "after-postimages cancellation did not roll back: {observed:?}"
        );
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        assert_eq!(service.machine_state_for_test(), machine_before);
        fixture.cleanup();
    }

    #[test]
    fn real_effect_after_all_postimages_deadline_rolls_back_exact_state() {
        use crate::infrastructure::native_operations::compile_transaction::RetainedApplyObservedEvent;

        let fixture = actor_fixture("effect-after-postimages-deadline", &["main"]);
        write_actor_event_fixture(&fixture.roots[0]);
        let binding = fixture
            .actor
            .bind_provider_root("main", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();
        let mut prepared =
            prepare_property_effect_batch(&fixture, &binding, false, &CancellationToken::new());
        assert_prepared_form_module_effect_subject(&prepared);
        prepared.deadline = ProviderDeadline::from_budget(Duration::from_millis(500));
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        let machine_before = service.machine_state_for_test();
        let _ = crate::infrastructure::native_operations::compile_transaction::take_retained_apply_observed_events();
        crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_post_validation_hook(
            || std::thread::sleep(Duration::from_millis(550)),
        );

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

        assert_eq!(error.kind(), super::ApplyPublicationErrorKind::Deadline);
        let observed = crate::infrastructure::native_operations::compile_transaction::take_retained_apply_observed_events();
        assert!(observed
            .iter()
            .any(|event| matches!(event, RetainedApplyObservedEvent::StateMarker(_))));
        assert!(
            observed
                .iter()
                .any(|event| matches!(event, RetainedApplyObservedEvent::Rollback(_))),
            "after-postimages deadline did not roll back: {observed:?}"
        );
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        assert_eq!(service.machine_state_for_test(), machine_before);
        fixture.cleanup();
    }

    #[test]
    pub(crate) fn retained_apply_effect_result_contract_is_complete() {
        let _total_effects_accessor: for<'a> fn(
            &'a super::ApplyPublicationResult,
        ) -> &'a ApplyEffectReceipt = super::ApplyPublicationResult::effects;
        prepared_apply_effects_are_retained_from_planner_to_result();
        prepared_apply_dry_run_returns_projected_effect_receipt_without_any_write();
        prepared_apply_success_returns_committed_effect_receipt_after_one_commit();
        event_implement_planner_integrates_with_actor_effect_publication_matrix();
        event_implement_op_failure_returns_no_effect_receipt_and_preserves_all_state();
        retained_apply_effect_failure_matrix_rolls_back_and_returns_no_receipt();
        retained_apply_effect_races_never_publish_or_return_effects();
        real_effect_foreign_actor_replay_preserves_both_actor_states();
        real_effect_mutation_lane_cancellation_preserves_exact_state();
        real_effect_mutation_lane_deadline_preserves_exact_state();
        real_effect_mid_scan_cancellation_preserves_exact_state();
        real_effect_mid_scan_deadline_preserves_exact_state();
        real_effect_after_all_postimages_cancellation_rolls_back_exact_state();
        real_effect_after_all_postimages_deadline_rolls_back_exact_state();
    }

    fn prepare_property_effect_batch(
        fixture: &ActorFixture,
        binding: &super::ProviderRootBinding,
        dry_run: bool,
        cancellation: &CancellationToken,
    ) -> PreparedApplyBatch {
        let admitted = fixture
            .actor
            .admit_apply(
                binding,
                None,
                dry_run,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                cancellation,
            )
            .unwrap();
        let (state, effects) = plan_actor_events(
            &admitted,
            &[event_operation(
                "main:Catalog.Products.Form.Main.Event.OnOpen",
            )],
        )
        .unwrap();
        admitted.prepare_with_effects(state, effects).unwrap()
    }

    fn assert_prepared_form_module_effect_subject(prepared: &PreparedApplyBatch) {
        assert_eq!(
            prepared.effects.events,
            [
                crate::domain::events::DomainEvent::new(
                    DomainEventKind::FormChanged,
                    "main:Catalog.Products.Form.Main",
                ),
                crate::domain::events::DomainEvent::new(
                    DomainEventKind::ModuleChanged,
                    "main:Catalog.Products.Form.Main.Module.Form",
                ),
            ]
        );
        assert_eq!(prepared.effects.cache.mode, "applied");
        assert_eq!(
            prepared.effects.cache.events,
            ["FormChanged", "ModuleChanged"]
        );
    }

    #[test]
    fn apply_policy_preserves_workspace_ancestor_precedence_over_source_local_policy() {
        let container = temp_root("apply-policy-precedence");
        let workspace = container.join("worktrees/one");
        let source = workspace.join("src");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            "<MetaDataObject><Configuration/></MetaDataObject>",
        )
        .unwrap();
        std::fs::write(
            container.join(".v8-project.json"),
            br#"{"editingAllowedCheck":"warn"}"#,
        )
        .unwrap();
        std::fs::write(
            source.join(".v8-project.json"),
            br#"{"editingAllowedCheck":"off"}"#,
        )
        .unwrap();
        let context = context(&workspace);
        let identity = WorkspaceIdentity::new(
            &context,
            [source_input("main", source.as_path())],
            "test-provider",
        )
        .unwrap();
        let actor = super::WorkspaceActor::new(identity, context).unwrap();
        let binding = actor.bind_provider_root("main", &source).unwrap();

        let admission = actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();

        assert_eq!(admission.support_policy_mode(), SupportPolicyMode::Warn);
        std::fs::remove_dir_all(container).unwrap();
    }

    #[test]
    fn apply_policy_absent_chain_rejects_nearer_policy_insertion_before_publication() {
        let fixture = actor_fixture("apply-policy-absent-insertion", &["src"]);
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"original").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let cancellation = CancellationToken::new();
        let admission = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let mut state = admission.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"published".to_vec())
            .unwrap();
        let prepared = admission.prepare(state).unwrap();
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        std::fs::write(
            fixture.root.join(".v8-project.json"),
            br#"{"editingAllowedCheck":"off"}"#,
        )
        .unwrap();

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::ContainmentIdentity
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"original");
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        fixture.cleanup();
    }

    #[test]
    fn apply_policy_exact_file_rejects_byte_change_and_rename_replacement() {
        let mut rejected = Vec::new();
        for replacement in [false, true] {
            let fixture = actor_fixture(
                if replacement {
                    "apply-policy-identity-replacement"
                } else {
                    "apply-policy-byte-change"
                },
                &["src"],
            );
            let target = fixture.roots[0].join("Module.bsl");
            let policy = fixture.root.join(".v8-project.json");
            std::fs::write(&target, b"original").unwrap();
            std::fs::write(&policy, br#"{"editingAllowedCheck":"off"}"#).unwrap();
            let binding = fixture
                .actor
                .bind_provider_root("src", &fixture.roots[0])
                .unwrap();
            let admission = fixture
                .actor
                .admit_apply(
                    &binding,
                    None,
                    false,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap();
            assert_eq!(admission.support_policy_mode(), SupportPolicyMode::Off);
            let mut state = admission.staged_state().unwrap();
            state
                .replace("Module.bsl", b"original", b"published".to_vec())
                .unwrap();
            let prepared = admission.prepare(state).unwrap();
            if replacement {
                std::fs::rename(&policy, fixture.root.join("policy-displaced.json")).unwrap();
                std::fs::write(&policy, br#"{"editingAllowedCheck":"off"}"#).unwrap();
            } else {
                std::fs::write(&policy, br#"{"editingAllowedCheck":"warn"}"#).unwrap();
            }
            rejected.push(fixture.actor.publish_prepared_apply(prepared).is_err());
            fixture.cleanup();
        }
        assert_eq!(rejected, [true, true]);
    }

    #[test]
    fn apply_policy_stable_deny_evidence_allows_unrelated_dry_run_and_real_publication() {
        for category in ["wrong-kind", "oversized", "unreadable"] {
            for dry_run in [true, false] {
                let fixture = actor_fixture(
                    &format!(
                        "apply-policy-stable-{category}-{}",
                        if dry_run { "dry" } else { "real" }
                    ),
                    &["src"],
                );
                let policy = fixture.root.join(".v8-project.json");
                let unreadable = match category {
                    "wrong-kind" => {
                        std::fs::create_dir(&policy).unwrap();
                        false
                    }
                    "oversized" => {
                        std::fs::write(&policy, vec![b' '; 32 * 1024 * 1024 + 1]).unwrap();
                        false
                    }
                    "unreadable" => {
                        std::fs::write(&policy, br#"{"editingAllowedCheck":"off"}"#).unwrap();
                        if !set_unix_mode_for_test(&policy, 0o000).unwrap() {
                            eprintln!(
                                "[SKIPPED FIXTURE] unreadable support-policy publication requires Unix permission bits"
                            );
                            fixture.cleanup();
                            continue;
                        }
                        true
                    }
                    _ => unreachable!(),
                };
                let target = fixture.roots[0].join("Module.bsl");
                std::fs::write(&target, b"original").unwrap();
                let binding = fixture
                    .actor
                    .bind_provider_root("src", &fixture.roots[0])
                    .unwrap();
                let admission = fixture
                    .actor
                    .admit_apply(
                        &binding,
                        None,
                        dry_run,
                        ProviderDeadline::from_budget(Duration::from_secs(10)),
                        &CancellationToken::new(),
                    )
                    .unwrap();
                assert_eq!(admission.support_policy_mode(), SupportPolicyMode::Deny);
                let mut state = admission.staged_state().unwrap();
                if !dry_run {
                    state
                        .replace("Module.bsl", b"original", b"published".to_vec())
                        .unwrap();
                }
                let result = fixture
                    .actor
                    .publish_prepared_apply(admission.prepare(state).unwrap())
                    .unwrap();

                assert_eq!(
                    result.effects().disposition(),
                    if dry_run {
                        ApplyEffectDisposition::Projected
                    } else {
                        ApplyEffectDisposition::Committed
                    },
                    "category={category}, dry_run={dry_run}"
                );
                assert_eq!(
                    result.commit_count_for_test(),
                    usize::from(!dry_run),
                    "category={category}, dry_run={dry_run}"
                );
                assert_eq!(
                    std::fs::read(&target).unwrap(),
                    if dry_run {
                        b"original".as_slice()
                    } else {
                        b"published".as_slice()
                    },
                    "category={category}, dry_run={dry_run}"
                );
                if unreadable {
                    assert!(set_unix_mode_for_test(&policy, 0o600).unwrap());
                }
                fixture.cleanup();
            }
        }
    }

    #[test]
    fn apply_policy_category_and_identity_transitions_are_rejected() {
        let wrong_kind = actor_fixture("apply-policy-wrong-kind", &["src"]);
        let policy = wrong_kind.root.join(".v8-project.json");
        std::fs::create_dir(&policy).unwrap();
        let binding = wrong_kind
            .actor
            .bind_provider_root("src", &wrong_kind.roots[0])
            .unwrap();
        let admission = wrong_kind
            .actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(admission.support_policy_mode(), SupportPolicyMode::Deny);
        let state = admission.staged_state().unwrap();
        let prepared = admission.prepare(state).unwrap();
        std::fs::remove_dir(&policy).unwrap();
        std::fs::write(&policy, br#"{"editingAllowedCheck":"off"}"#).unwrap();
        assert_eq!(
            wrong_kind
                .actor
                .publish_prepared_apply(prepared)
                .unwrap_err()
                .kind(),
            super::ApplyPublicationErrorKind::ContainmentIdentity
        );
        wrong_kind.cleanup();

        let symlink = actor_fixture("apply-policy-symlink", &["src"]);
        let policy = symlink.root.join(".v8-project.json");
        let target = symlink.root.join("linked-policy.json");
        std::fs::write(&target, br#"{"editingAllowedCheck":"off"}"#).unwrap();
        let link_outcome = create_file_link_fixture_for_test(&target, &policy).unwrap();
        match link_outcome {
            FileLinkFixtureOutcome::Created => {
                let binding = symlink
                    .actor
                    .bind_provider_root("src", &symlink.roots[0])
                    .unwrap();
                let admission = symlink
                    .actor
                    .admit_apply(
                        &binding,
                        None,
                        true,
                        ProviderDeadline::from_budget(Duration::from_secs(5)),
                        &CancellationToken::new(),
                    )
                    .unwrap();
                assert_eq!(admission.support_policy_mode(), SupportPolicyMode::Deny);
                let state = admission.staged_state().unwrap();
                let prepared = admission.prepare(state).unwrap();
                std::fs::remove_file(&policy).unwrap();
                std::fs::write(&policy, br#"{"editingAllowedCheck":"off"}"#).unwrap();
                assert_eq!(
                    symlink
                        .actor
                        .publish_prepared_apply(prepared)
                        .unwrap_err()
                        .kind(),
                    super::ApplyPublicationErrorKind::ContainmentIdentity
                );
            }
            FileLinkFixtureOutcome::Unsupported => eprintln!(
                "[SKIPPED FIXTURE] support-policy symlink transition is unsupported on this platform"
            ),
            FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => eprintln!(
                "[SKIPPED FIXTURE] support-policy symlink transition needs an unavailable Windows privilege"
            ),
        }
        symlink.cleanup();

        if can_swap_named_child_behind_retained_handle_for_test() {
            let oversized = actor_fixture("apply-policy-oversized-identity", &["src"]);
            let policy = oversized.root.join(".v8-project.json");
            std::fs::write(&policy, vec![b' '; 32 * 1024 * 1024 + 1]).unwrap();
            let binding = oversized
                .actor
                .bind_provider_root("src", &oversized.roots[0])
                .unwrap();
            let admission = oversized
                .actor
                .admit_apply(
                    &binding,
                    None,
                    true,
                    ProviderDeadline::from_budget(Duration::from_secs(10)),
                    &CancellationToken::new(),
                )
                .unwrap();
            assert_eq!(admission.support_policy_mode(), SupportPolicyMode::Deny);
            let state = admission.staged_state().unwrap();
            let prepared = admission.prepare(state).unwrap();
            std::fs::rename(&policy, oversized.root.join("oversized-displaced.json")).unwrap();
            std::fs::write(&policy, vec![b' '; 32 * 1024 * 1024 + 1]).unwrap();
            assert_eq!(
                oversized
                    .actor
                    .publish_prepared_apply(prepared)
                    .unwrap_err()
                    .kind(),
                super::ApplyPublicationErrorKind::ContainmentIdentity
            );
            oversized.cleanup();
        } else {
            eprintln!(
                "[SKIPPED FIXTURE] oversized support-policy identity replacement is unsupported while a retained handle is open"
            );
        }

        let unreadable = actor_fixture("apply-policy-unreadable-transition", &["src"]);
        let policy = unreadable.root.join(".v8-project.json");
        std::fs::write(&policy, br#"{"editingAllowedCheck":"off"}"#).unwrap();
        if set_unix_mode_for_test(&policy, 0o000).unwrap() {
            let binding = unreadable
                .actor
                .bind_provider_root("src", &unreadable.roots[0])
                .unwrap();
            let admission = unreadable
                .actor
                .admit_apply(
                    &binding,
                    None,
                    true,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap();
            assert_eq!(admission.support_policy_mode(), SupportPolicyMode::Deny);
            let state = admission.staged_state().unwrap();
            let prepared = admission.prepare(state).unwrap();
            assert!(set_unix_mode_for_test(&policy, 0o600).unwrap());
            assert_eq!(
                unreadable
                    .actor
                    .publish_prepared_apply(prepared)
                    .unwrap_err()
                    .kind(),
                super::ApplyPublicationErrorKind::ContainmentIdentity
            );
        } else {
            eprintln!(
                "[SKIPPED FIXTURE] unreadable support-policy transition requires Unix permission bits"
            );
        }
        unreadable.cleanup();
    }

    #[test]
    fn apply_policy_dry_run_churn_is_write_free_and_returns_no_receipt() {
        let fixture = actor_fixture("apply-policy-dry-run-churn", &["src"]);
        let target = fixture.roots[0].join("Module.bsl");
        let policy = fixture.root.join(".v8-project.json");
        std::fs::write(&target, b"original").unwrap();
        std::fs::write(&policy, br#"{"editingAllowedCheck":"off"}"#).unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admission = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admission.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"projected".to_vec())
            .unwrap();
        let prepared = admission.prepare(state).unwrap();
        let raced_policy = policy.clone();
        set_apply_dry_run_after_confirmation_hook(move || {
            std::fs::write(raced_policy, br#"{"editingAllowedCheck":"warn"}"#).unwrap();
        });

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::ContainmentIdentity
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"original");
        assert!(!fixture.root.join(".build/unica").exists());
        fixture.cleanup();
    }

    #[test]
    fn apply_policy_churn_before_source_publication_is_write_free() {
        let fixture = actor_fixture("apply-policy-prepublication-churn", &["src"]);
        let target = fixture.roots[0].join("Module.bsl");
        let policy = fixture.root.join(".v8-project.json");
        std::fs::write(&target, b"original").unwrap();
        std::fs::write(&policy, br#"{"editingAllowedCheck":"off"}"#).unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admission = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admission.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"published".to_vec())
            .unwrap();
        let prepared = admission.prepare(state).unwrap();
        std::fs::write(&policy, br#"{"editingAllowedCheck":"warn"}"#).unwrap();

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::ContainmentIdentity
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"original");
        assert!(!fixture.root.join(".build/unica").exists());
        fixture.cleanup();
    }

    #[test]
    fn apply_policy_churn_after_source_publication_rolls_back_all_retained_state() {
        let fixture = actor_fixture("apply-policy-postpublication-churn", &["src"]);
        let target = fixture.roots[0].join("Module.bsl");
        let policy = fixture.root.join(".v8-project.json");
        std::fs::write(&target, b"original").unwrap();
        std::fs::write(&policy, br#"{"editingAllowedCheck":"off"}"#).unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();
        let admission = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admission.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"published".to_vec())
            .unwrap();
        let prepared = admission.prepare(state).unwrap();
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        let machine_before = service.machine_state_for_test();
        let raced_policy = policy.clone();
        crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_post_validation_hook(
            move || {
                std::fs::write(raced_policy, br#"{"editingAllowedCheck":"warn"}"#).unwrap();
            },
        );

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::ContainmentIdentity
        );
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        assert_eq!(service.machine_state_for_test(), machine_before);
        fixture.cleanup();
    }

    #[test]
    fn apply_policy_same_inode_churn_during_late_final_gate_rolls_back_all_retained_state() {
        let _test_state = SupportPolicyTestStateGuard::new(reset_support_policy_actor_test_now);
        let fixture = actor_fixture("apply-policy-late-final-same-inode-churn", &["src"]);
        let target = fixture.roots[0].join("Module.bsl");
        let policy = fixture.root.join(".v8-project.json");
        std::fs::write(&target, b"original").unwrap();
        let admitted = br#"{"editingAllowedCheck":"off"}"#;
        let changed = br#"{"editingAllowedCheck":"bad"}"#;
        assert_eq!(admitted.len(), changed.len());
        std::fs::write(&policy, admitted).unwrap();
        let policy_identity = crate::infrastructure::platform::filesystem::file_identity(
            &std::fs::File::open(&policy).unwrap(),
        )
        .unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let service = fixture.actor.source_revision_service(&binding).unwrap();
        let admission = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(10)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admission.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"published".to_vec())
            .unwrap();
        let prepared = admission.prepare(state).unwrap();
        let source_before = snapshot_tree(&fixture.roots[0]);
        let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
        let machine_before = service.machine_state_for_test();
        let hook_policy = policy.clone();
        crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_post_validation_hook(
            move || {
                crate::infrastructure::support_policy_evidence::set_support_policy_after_retained_read_before_acceptance_hook(
                    move || std::fs::write(hook_policy, changed).unwrap(),
                );
            },
        );

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        let current_policy_identity = crate::infrastructure::platform::filesystem::file_identity(
            &std::fs::File::open(&policy).unwrap(),
        )
        .unwrap();

        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::ContainmentIdentity
        );
        assert_eq!(current_policy_identity, policy_identity);
        assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
        assert_eq!(
            snapshot_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        assert_eq!(service.machine_state_for_test(), machine_before);
        fixture.cleanup();
    }

    #[test]
    fn apply_policy_foreign_actor_and_sibling_worktree_replay_are_rejected() {
        real_effect_foreign_actor_replay_preserves_both_actor_states();

        let fixture = actor_fixture("apply-policy-sibling-source", &["one", "two"]);
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"original").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("one", &fixture.roots[0])
            .unwrap();
        let admission = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admission.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"projected".to_vec())
            .unwrap();
        let mut prepared = admission.prepare(state).unwrap();
        prepared.source_set = fixture
            .actor
            .identity
            .source_sets
            .iter()
            .find(|source| source.name == "two")
            .unwrap()
            .clone();

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

        assert_eq!(
            error.kind(),
            super::ApplyPublicationErrorKind::ContainmentIdentity
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"original");
        fixture.cleanup();
    }

    #[test]
    fn apply_policy_same_ancestor_can_govern_two_worktrees_without_authority_aliasing() {
        let container = temp_root("apply-policy-shared-ancestor");
        std::fs::create_dir_all(&container).unwrap();
        std::fs::write(
            container.join(".v8-project.json"),
            br#"{"editingAllowedCheck":"off"}"#,
        )
        .unwrap();
        let mut actors = Vec::new();
        for name in ["one", "two"] {
            let workspace = container.join("worktrees").join(name);
            let source = workspace.join("src");
            std::fs::create_dir_all(&source).unwrap();
            std::fs::write(
                workspace.join("v8project.yaml"),
                "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
            )
            .unwrap();
            std::fs::write(
                source.join("Configuration.xml"),
                "<MetaDataObject><Configuration/></MetaDataObject>",
            )
            .unwrap();
            let context = context(&workspace);
            let identity = WorkspaceIdentity::new(
                &context,
                [source_input("main", source.as_path())],
                "test-provider",
            )
            .unwrap();
            actors.push((
                Arc::new(super::WorkspaceActor::new(identity, context).unwrap()),
                source,
            ));
        }
        let first_binding = actors[0]
            .0
            .bind_provider_root("main", &actors[0].1)
            .unwrap();
        let first_admission = actors[0]
            .0
            .admit_apply(
                &first_binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(
            first_admission.support_policy_mode(),
            SupportPolicyMode::Off
        );
        let first_state = first_admission.staged_state().unwrap();
        let first_prepared = first_admission.prepare(first_state).unwrap();
        let replay = actors[1].0.publish_prepared_apply(first_prepared);
        assert_eq!(
            replay.unwrap_err().kind(),
            super::ApplyPublicationErrorKind::Invariant
        );

        for (actor, source) in &actors {
            let binding = actor.bind_provider_root("main", source).unwrap();
            let admission = actor
                .admit_apply(
                    &binding,
                    None,
                    true,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap();
            assert_eq!(admission.support_policy_mode(), SupportPolicyMode::Off);
            let state = admission.staged_state().unwrap();
            let result = actor
                .publish_prepared_apply(admission.prepare(state).unwrap())
                .unwrap();
            assert_eq!(result.commit_count_for_test(), 0);
        }
        std::fs::remove_dir_all(container).unwrap();
    }

    #[test]
    fn apply_policy_deadline_and_cancellation_during_capture_are_write_free() {
        let _test_state = SupportPolicyTestStateGuard::new(reset_support_policy_actor_test_now);
        let mut rejected = Vec::new();
        for gate in ["cancelled", "deadline"] {
            let fixture = actor_fixture(&format!("apply-policy-capture-{gate}"), &["src"]);
            std::fs::write(
                fixture.root.join(".v8-project.json"),
                br#"{"editingAllowedCheck":"off"}"#,
            )
            .unwrap();
            let binding = fixture
                .actor
                .bind_provider_root("src", &fixture.roots[0])
                .unwrap();
            let cancellation = CancellationToken::new();
            if gate == "cancelled" {
                let cancel = cancellation.clone();
                set_support_policy_capture_hook(move || cancel.cancel());
            } else {
                set_support_policy_capture_hook(|| {
                    std::thread::sleep(Duration::from_millis(80));
                });
            }
            let result = fixture.actor.admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(if gate == "deadline" {
                    Duration::from_millis(50)
                } else {
                    Duration::from_secs(5)
                }),
                &cancellation,
            );
            rejected.push(result.is_err());
            assert!(!fixture.root.join(".build/unica").exists());
            fixture.cleanup();
        }
        assert_eq!(rejected, [true, true]);
    }

    #[test]
    fn apply_policy_capture_stops_after_first_retained_read_chunk_write_free() {
        let _test_state = SupportPolicyTestStateGuard::new(reset_support_policy_actor_test_now);
        let mut observed = Vec::new();
        for gate in ["cancelled", "deadline"] {
            let fixture = actor_fixture(&format!("apply-policy-chunked-capture-{gate}"), &["src"]);
            let policy = fixture.root.join(".v8-project.json");
            let prefix = br#"{"editingAllowedCheck":"off"}"#;
            let mut policy_bytes = vec![b' '; 3 * 64 * 1024];
            policy_bytes[..prefix.len()].copy_from_slice(prefix);
            std::fs::write(&policy, policy_bytes).unwrap();
            let target = fixture.roots[0].join("Module.bsl");
            std::fs::write(&target, b"original").unwrap();
            let binding = fixture
                .actor
                .bind_provider_root("src", &fixture.roots[0])
                .unwrap();
            let service = fixture.actor.source_revision_service(&binding).unwrap();
            let source_before = snapshot_tree(&fixture.roots[0]);
            let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
            let machine_before = service.machine_state_for_test();
            let cancellation = CancellationToken::new();
            let started = Instant::now();
            set_support_policy_actor_test_now(started);
            let deadline = if gate == "deadline" {
                ProviderDeadline::with_clock(
                    started + Duration::from_secs(1),
                    support_policy_actor_test_now,
                )
            } else {
                ProviderDeadline::from_budget(Duration::from_secs(10))
            };
            let chunks = Arc::new(AtomicUsize::new(0));
            let observed_chunks = Arc::clone(&chunks);
            if gate == "cancelled" {
                let cancel = cancellation.clone();
                set_support_policy_read_chunk_hook_once(move |_| {
                    observed_chunks.fetch_add(1, Ordering::SeqCst);
                    cancel.cancel();
                });
            } else {
                set_support_policy_read_chunk_hook_once(move |_| {
                    observed_chunks.fetch_add(1, Ordering::SeqCst);
                    set_support_policy_actor_test_now(started + Duration::from_secs(2));
                });
            }

            let result = fixture
                .actor
                .admit_apply(&binding, None, false, deadline, &cancellation);
            observed.push((
                result.err().map(|error| error.to_string()),
                chunks.load(Ordering::SeqCst),
            ));

            assert_eq!(std::fs::read(&target).unwrap(), b"original", "{gate}");
            assert_eq!(snapshot_tree(&fixture.roots[0]), source_before, "{gate}");
            assert_eq!(
                snapshot_tree(&fixture.root.join(".build/unica")),
                cache_before,
                "{gate}"
            );
            assert_eq!(service.machine_state_for_test(), machine_before, "{gate}");
            fixture.cleanup();
        }

        assert_eq!(
            observed,
            [
                (Some("support-policy capture cancelled".to_string()), 1),
                (
                    Some("support-policy capture deadline exceeded".to_string()),
                    1,
                ),
            ]
        );
    }

    #[test]
    fn apply_policy_all_absent_capture_rejects_terminal_cancellation_and_deadline_write_free() {
        let _test_state = SupportPolicyTestStateGuard::new(reset_support_policy_actor_test_now);
        let mut observed = Vec::new();
        for gate in ["cancelled", "deadline"] {
            let container = temp_root(&format!("apply-policy-all-absent-capture-{gate}"));
            let root = (0..24).fold(container.clone(), |path, index| {
                path.join(format!("level-{index}"))
            });
            let source = root.join("src");
            std::fs::create_dir_all(&source).unwrap();
            std::fs::write(
                root.join("v8project.yaml"),
                "format: DESIGNER\nsource-set:\n  - name: src\n    type: CONFIGURATION\n    path: src\n",
            )
            .unwrap();
            std::fs::write(
                source.join("Configuration.xml"),
                "<MetaDataObject><Configuration/></MetaDataObject>",
            )
            .unwrap();
            let context = context(&root);
            let identity = WorkspaceIdentity::new(
                &context,
                [source_input("src", source.as_path())],
                "test-provider",
            )
            .unwrap();
            let actor = super::WorkspaceActor::new(identity, context).unwrap();
            let binding = actor.bind_provider_root("src", &source).unwrap();
            let before = snapshot_tree(&root);
            let cancellation = CancellationToken::new();
            let started = Instant::now();
            set_support_policy_actor_test_now(started);
            let deadline = if gate == "deadline" {
                ProviderDeadline::with_clock(
                    started + Duration::from_secs(1),
                    support_policy_actor_test_now,
                )
            } else {
                ProviderDeadline::from_budget(Duration::from_secs(5))
            };
            if gate == "cancelled" {
                let cancel = cancellation.clone();
                set_support_policy_capture_hook(move || cancel.cancel());
            } else {
                set_support_policy_capture_hook(move || {
                    set_support_policy_actor_test_now(started + Duration::from_secs(2));
                });
            }

            let result = actor.admit_apply(&binding, None, false, deadline, &cancellation);

            observed.push(result.err().map(|error| error.to_string()));
            assert_eq!(snapshot_tree(&root), before, "{gate}");
            assert!(!root.join(".build/unica").exists(), "{gate}");
            std::fs::remove_dir_all(container).unwrap();
        }
        assert_eq!(
            observed,
            [
                Some("support-policy capture cancelled".to_string()),
                Some("support-policy capture deadline exceeded".to_string()),
            ]
        );
    }

    #[test]
    fn apply_policy_deadline_and_cancellation_during_final_validation_roll_back() {
        let _test_state = SupportPolicyTestStateGuard::new(reset_support_policy_actor_test_now);
        for gate in ["cancelled", "deadline"] {
            let fixture = actor_fixture(&format!("apply-policy-final-{gate}"), &["src"]);
            let target = fixture.roots[0].join("Module.bsl");
            std::fs::write(&target, b"original").unwrap();
            std::fs::write(
                fixture.root.join(".v8-project.json"),
                br#"{"editingAllowedCheck":"off"}"#,
            )
            .unwrap();
            let binding = fixture
                .actor
                .bind_provider_root("src", &fixture.roots[0])
                .unwrap();
            let service = fixture.actor.source_revision_service(&binding).unwrap();
            let cancellation = CancellationToken::new();
            let admission = fixture
                .actor
                .admit_apply(
                    &binding,
                    None,
                    false,
                    ProviderDeadline::from_budget(if gate == "deadline" {
                        Duration::from_millis(500)
                    } else {
                        Duration::from_secs(5)
                    }),
                    &cancellation,
                )
                .unwrap();
            let mut state = admission.staged_state().unwrap();
            state
                .replace("Module.bsl", b"original", b"published".to_vec())
                .unwrap();
            let prepared = admission.prepare(state).unwrap();
            let source_before = snapshot_tree(&fixture.roots[0]);
            let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
            let machine_before = service.machine_state_for_test();
            if gate == "cancelled" {
                let cancel = cancellation.clone();
                crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_post_validation_hook(
                    move || {
                        set_support_policy_validation_hook(move || cancel.cancel());
                    },
                );
            } else {
                crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_post_validation_hook(
                    || {
                        set_support_policy_validation_hook(|| {
                            std::thread::sleep(Duration::from_millis(550));
                        });
                    },
                );
            }

            let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();

            assert_eq!(
                error.kind(),
                if gate == "cancelled" {
                    super::ApplyPublicationErrorKind::Cancelled
                } else {
                    super::ApplyPublicationErrorKind::Deadline
                }
            );
            assert_eq!(snapshot_tree(&fixture.roots[0]), source_before);
            assert_eq!(
                snapshot_tree(&fixture.root.join(".build/unica")),
                cache_before
            );
            assert_eq!(service.machine_state_for_test(), machine_before);
            fixture.cleanup();
        }
    }

    #[test]
    fn apply_policy_final_gate_stops_after_first_retained_read_chunk_and_rolls_back() {
        let _test_state = SupportPolicyTestStateGuard::new(reset_support_policy_actor_test_now);
        let mut observed = Vec::new();
        for gate in ["cancelled", "deadline"] {
            let fixture = actor_fixture(&format!("apply-policy-chunked-final-{gate}"), &["src"]);
            let target = fixture.roots[0].join("Module.bsl");
            std::fs::write(&target, b"original").unwrap();
            let policy = fixture.root.join(".v8-project.json");
            let prefix = br#"{"editingAllowedCheck":"off"}"#;
            let mut policy_bytes = vec![b' '; 3 * 64 * 1024];
            policy_bytes[..prefix.len()].copy_from_slice(prefix);
            std::fs::write(&policy, policy_bytes).unwrap();
            let binding = fixture
                .actor
                .bind_provider_root("src", &fixture.roots[0])
                .unwrap();
            let service = fixture.actor.source_revision_service(&binding).unwrap();
            let cancellation = CancellationToken::new();
            let started = Instant::now();
            set_support_policy_actor_test_now(started);
            let deadline = if gate == "deadline" {
                ProviderDeadline::with_clock(
                    started + Duration::from_secs(1),
                    support_policy_actor_test_now,
                )
            } else {
                ProviderDeadline::from_budget(Duration::from_secs(10))
            };
            let admission = fixture
                .actor
                .admit_apply(&binding, None, false, deadline, &cancellation)
                .unwrap();
            let mut state = admission.staged_state().unwrap();
            state
                .replace("Module.bsl", b"original", b"published".to_vec())
                .unwrap();
            let prepared = admission.prepare(state).unwrap();
            let source_before = snapshot_tree(&fixture.roots[0]);
            let cache_before = snapshot_tree(&fixture.root.join(".build/unica"));
            let machine_before = service.machine_state_for_test();
            let chunks = Arc::new(AtomicUsize::new(0));
            let observed_chunks = Arc::clone(&chunks);
            if gate == "cancelled" {
                let cancel = cancellation.clone();
                crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_post_validation_hook(
                    move || {
                        set_support_policy_read_chunk_hook_once(move |_| {
                            observed_chunks.fetch_add(1, Ordering::SeqCst);
                            cancel.cancel();
                        });
                    },
                );
            } else {
                crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_post_validation_hook(
                    move || {
                        set_support_policy_read_chunk_hook_once(move |_| {
                            observed_chunks.fetch_add(1, Ordering::SeqCst);
                            set_support_policy_actor_test_now(
                                started + Duration::from_secs(2),
                            );
                        });
                    },
                );
            }

            let result = fixture.actor.publish_prepared_apply(prepared);
            observed.push((
                result.as_ref().err().map(|error| error.kind()),
                chunks.load(Ordering::SeqCst),
            ));
            assert!(result.is_err(), "{gate} returned a terminal receipt");
            assert_eq!(snapshot_tree(&fixture.roots[0]), source_before, "{gate}");
            assert_eq!(
                snapshot_tree(&fixture.root.join(".build/unica")),
                cache_before,
                "{gate}"
            );
            assert_eq!(service.machine_state_for_test(), machine_before, "{gate}");
            fixture.cleanup();
        }

        assert_eq!(
            observed,
            [
                (Some(super::ApplyPublicationErrorKind::Cancelled), 1),
                (Some(super::ApplyPublicationErrorKind::Deadline), 1),
            ]
        );
    }

    #[test]
    fn apply_policy_warn_off_deny_database_and_malformed_match_v12() {
        let cases: &[(&str, &[u8], SupportPolicyMode)] = &[
            (
                "warn",
                br#"{"editingAllowedCheck":"warn"}"#,
                SupportPolicyMode::Warn,
            ),
            (
                "off",
                br#"{"editingAllowedCheck":"off"}"#,
                SupportPolicyMode::Off,
            ),
            (
                "deny",
                br#"{"editingAllowedCheck":"deny"}"#,
                SupportPolicyMode::Deny,
            ),
            ("missing-value", br#"{}"#, SupportPolicyMode::Deny),
            ("malformed", b"not json", SupportPolicyMode::Deny),
            (
                "unknown",
                br#"{"editingAllowedCheck":"WARN"}"#,
                SupportPolicyMode::Deny,
            ),
            (
                "bom",
                b"\xef\xbb\xbf{\"editingAllowedCheck\":\"off\"}",
                SupportPolicyMode::Off,
            ),
            (
                "database",
                br#"{"editingAllowedCheck":"off","databases":[{"configSrc":"src","editingAllowedCheck":"warn"}]}"#,
                SupportPolicyMode::Warn,
            ),
        ];
        for (name, bytes, expected) in cases {
            let fixture = actor_fixture(&format!("apply-policy-mode-{name}"), &["src"]);
            std::fs::write(fixture.root.join(".v8-project.json"), bytes).unwrap();
            let binding = fixture
                .actor
                .bind_provider_root("src", &fixture.roots[0])
                .unwrap();
            let admission = fixture
                .actor
                .admit_apply(
                    &binding,
                    None,
                    true,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap();
            assert_eq!(admission.support_policy_mode(), *expected, "{name}");
            fixture.cleanup();
        }

        let missing = actor_fixture("apply-policy-mode-missing", &["src"]);
        let binding = missing
            .actor
            .bind_provider_root("src", &missing.roots[0])
            .unwrap();
        let admission = missing
            .actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(admission.support_policy_mode(), SupportPolicyMode::Deny);
        missing.cleanup();

        let oversized = actor_fixture("apply-policy-mode-oversized", &["src"]);
        std::fs::write(
            oversized.root.join(".v8-project.json"),
            vec![b' '; 32 * 1024 * 1024 + 1],
        )
        .unwrap();
        let binding = oversized
            .actor
            .bind_provider_root("src", &oversized.roots[0])
            .unwrap();
        let admission = oversized
            .actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(admission.support_policy_mode(), SupportPolicyMode::Deny);
        oversized.cleanup();

        let unreadable = actor_fixture("apply-policy-mode-unreadable", &["src"]);
        let policy = unreadable.root.join(".v8-project.json");
        std::fs::write(&policy, br#"{"editingAllowedCheck":"off"}"#).unwrap();
        if set_unix_mode_for_test(&policy, 0o000).unwrap() {
            let binding = unreadable
                .actor
                .bind_provider_root("src", &unreadable.roots[0])
                .unwrap();
            let admission = unreadable
                .actor
                .admit_apply(
                    &binding,
                    None,
                    true,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap();
            assert_eq!(admission.support_policy_mode(), SupportPolicyMode::Deny);
            drop(admission);
            assert!(set_unix_mode_for_test(&policy, 0o600).unwrap());
        } else {
            eprintln!(
                "[SKIPPED FIXTURE] unreadable support-policy mode requires Unix permission bits"
            );
        }
        unreadable.cleanup();
    }

    #[test]
    fn retained_apply_support_policy_evidence_does_not_add_a_third_writer_participant() {
        let fixture = actor_fixture("apply-policy-two-writers", &["src"]);
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admission = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let state = admission.staged_state().unwrap();
        let prepared = admission.prepare(state).unwrap();
        assert_eq!(
            prepared.transaction.retained_role_root_counts_for_test(),
            (1, 1)
        );
        retained_apply_closed_participant_contract_is_complete();
        fixture.cleanup();
    }

    #[test]
    pub(crate) fn retained_apply_support_policy_evidence_contract_is_complete() {
        apply_policy_preserves_workspace_ancestor_precedence_over_source_local_policy();
        apply_policy_absent_chain_rejects_nearer_policy_insertion_before_publication();
        apply_policy_exact_file_rejects_byte_change_and_rename_replacement();
        apply_policy_stable_deny_evidence_allows_unrelated_dry_run_and_real_publication();
        apply_policy_category_and_identity_transitions_are_rejected();
        crate::infrastructure::support_policy_evidence::tests::retained_support_policy_candidate_parent_replacement_is_rejected();
        crate::infrastructure::support_policy_evidence::tests::retained_support_policy_exact_and_oversized_reject_name_replacement_after_pre_read_identity();
        crate::infrastructure::support_policy_evidence::tests::retained_support_policy_exact_rejects_name_replacement_after_retained_read_before_acceptance();
        crate::infrastructure::support_policy_evidence::tests::retained_support_policy_exact_rejects_same_inode_change_between_stability_passes();
        apply_policy_dry_run_churn_is_write_free_and_returns_no_receipt();
        apply_policy_churn_before_source_publication_is_write_free();
        apply_policy_churn_after_source_publication_rolls_back_all_retained_state();
        apply_policy_same_inode_churn_during_late_final_gate_rolls_back_all_retained_state();
        apply_policy_foreign_actor_and_sibling_worktree_replay_are_rejected();
        apply_policy_same_ancestor_can_govern_two_worktrees_without_authority_aliasing();
        apply_policy_deadline_and_cancellation_during_capture_are_write_free();
        apply_policy_all_absent_capture_rejects_terminal_cancellation_and_deadline_write_free();
        apply_policy_deadline_and_cancellation_during_final_validation_roll_back();
        apply_policy_capture_stops_after_first_retained_read_chunk_write_free();
        apply_policy_final_gate_stops_after_first_retained_read_chunk_and_rolls_back();
        apply_policy_warn_off_deny_database_and_malformed_match_v12();
        crate::infrastructure::support_policy_evidence::tests::retained_support_policy_read_stops_before_post_read_when_pre_read_becomes_terminal();
        crate::infrastructure::support_policy_evidence::tests::retained_support_policy_read_stops_after_first_chunk_when_terminal();
        crate::infrastructure::support_policy_evidence::tests::retained_support_policy_second_pass_reuses_terminal_state_between_chunks();
        crate::infrastructure::support_policy_evidence::tests::retained_support_policy_reader_preserves_limit_plus_one_in_64_kib_chunks();
        crate::infrastructure::support_policy_evidence::tests::retained_support_policy_reader_retries_interrupted_after_partial_read();
        crate::infrastructure::support_policy_evidence::tests::retained_support_policy_reader_stops_repeated_interrupts_at_terminal_state();
        crate::infrastructure::support_policy_evidence::tests::retained_support_policy_reader_preserves_limit_plus_one_after_interrupt();
        crate::infrastructure::support_policy_evidence::tests::terminal_pre_read_does_not_leave_after_read_hook_for_following_validation();
        crate::infrastructure::support_policy_evidence::tests::support_policy_database_paths_distinguish_nested_sources_from_prefix_siblings();
        crate::infrastructure::support_policy_evidence::tests::support_policy_candidate_search_stops_at_exact_twentieth_candidate();
        crate::infrastructure::support_policy_evidence::tests::support_policy_overlapping_chains_keep_first_occurrence_order_without_duplicates();
        retained_apply_support_policy_evidence_does_not_add_a_third_writer_participant();
    }

    fn plan_actor_events(
        admitted: &ApplyAdmission,
        operations: &[EventImplementArgs],
    ) -> Result<
        (
            crate::infrastructure::native_operations::apply::ApplyStagedState,
            PlannedApplyEffects,
        ),
        EventPlanError,
    > {
        plan_event_implement_batch(
            admitted.staged_state().map_err(|error| {
                panic!("actor admission did not produce a plannable staged state: {error}")
            })?,
            "main",
            SourceSetKind::Configuration,
            PlatformProfile::v8_3_27(),
            operations,
        )
    }

    fn event_operation(at: &str) -> EventImplementArgs {
        EventImplementArgs {
            at: QualifiedAddress::parse(at).unwrap(),
            call_type: None,
        }
    }

    fn assert_form_module_effect_subject(
        receipt: &ApplyEffectReceipt,
        disposition: ApplyEffectDisposition,
    ) {
        assert_eq!(receipt.disposition(), disposition);
        assert_eq!(
            receipt.events(),
            &[
                crate::domain::events::DomainEvent::new(
                    DomainEventKind::FormChanged,
                    "main:Catalog.Products.Form.Main",
                ),
                crate::domain::events::DomainEvent::new(
                    DomainEventKind::ModuleChanged,
                    "main:Catalog.Products.Form.Main.Module.Form",
                ),
            ]
        );
        let cache = receipt.cache();
        assert_eq!(cache.mode, "applied");
        assert_eq!(cache.events, ["FormChanged", "ModuleChanged"]);
        assert_eq!(
            cache.invalidated,
            [
                "bsl_diagnostics",
                "bsl_index",
                "form_graph",
                "metadata_graph"
            ]
        );
        assert_eq!(cache.refreshed, ["metadata_graph"]);
    }

    fn write_actor_event_fixture(root: &Path) {
        const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
        std::fs::create_dir_all(root.join("Catalogs/Products/Forms/Main/Ext")).unwrap();
        std::fs::write(
            root.join("Configuration.xml"),
            format!(
                "<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><Configuration><Properties><Name>Fixture</Name></Properties><ChildObjects><Catalog>Products</Catalog></ChildObjects></Configuration></MetaDataObject>"
            ),
        )
        .unwrap();
        std::fs::write(
            root.join("Catalogs/Products.xml"),
            format!(
                "<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><Catalog><Properties><Name>Products</Name></Properties><ChildObjects><Form>Main</Form></ChildObjects></Catalog></MetaDataObject>"
            ),
        )
        .unwrap();
        std::fs::write(
            root.join("Catalogs/Products/Forms/Main.xml"),
            format!(
                "<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><Form><Properties><Name>Main</Name></Properties></Form></MetaDataObject>"
            ),
        )
        .unwrap();
        std::fs::write(
            root.join("Catalogs/Products/Forms/Main/Ext/Form.xml"),
            concat!(
                "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
                "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\r\n",
                "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\r\n",
                "\t<ChildItems/>\r\n",
                "\t<Commands/>\r\n",
                "</Form>\r\n"
            ),
        )
        .unwrap();
    }

    fn write_missing_on_open_binding(root: &Path) {
        std::fs::write(
            root.join("Catalogs/Products/Forms/Main/Ext/Form.xml"),
            concat!(
                "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
                "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\r\n",
                "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\r\n",
                "\t<Events><Event name=\"OnOpen\">ПриОткрытии</Event></Events>\r\n",
                "\t<ChildItems/>\r\n",
                "\t<Commands/>\r\n",
                "</Form>\r\n"
            ),
        )
        .unwrap();
    }

    struct ActorFixture {
        root: PathBuf,
        roots: Vec<PathBuf>,
        actor: Arc<super::WorkspaceActor>,
    }

    impl ActorFixture {
        fn cleanup(self) {
            let _ = std::fs::remove_dir_all(self.root);
        }
    }

    fn actor_fixture(name: &str, relative_roots: &[&str]) -> ActorFixture {
        let root = temp_root(name);
        let roots = relative_roots
            .iter()
            .map(|relative| root.join(relative))
            .collect::<Vec<_>>();
        for source_root in &roots {
            std::fs::create_dir_all(source_root).unwrap();
        }
        write_actor_source_map(&root, relative_roots);
        let context = context(&root);
        let source_sets = relative_roots
            .iter()
            .zip(roots.iter())
            .map(|(name, root)| source_input(name, root));
        let identity = WorkspaceIdentity::new(&context, source_sets, "test-provider").unwrap();
        let actor = Arc::new(super::WorkspaceActor::new(identity, context).unwrap());
        ActorFixture { root, roots, actor }
    }

    fn actor_fixture_without_source_map(name: &str, relative_root: &str) -> ActorFixture {
        let root = temp_root(name);
        let source_root = root.join(relative_root);
        std::fs::create_dir_all(&source_root).unwrap();
        let context = context(&root);
        let identity = WorkspaceIdentity::new(
            &context,
            [source_input("main", &source_root)],
            "test-provider",
        )
        .unwrap();
        let actor = Arc::new(super::WorkspaceActor::new(identity, context).unwrap());
        ActorFixture {
            root,
            roots: vec![source_root],
            actor,
        }
    }

    fn write_actor_source_map(root: &Path, relative_roots: &[&str]) {
        let mut yaml = String::from("format: DESIGNER\nsource-set:\n");
        for relative in relative_roots {
            yaml.push_str("  - name: ");
            yaml.push_str(&serde_json::to_string(relative).unwrap());
            yaml.push_str("\n    type: CONFIGURATION\n    path: ");
            yaml.push_str(&serde_json::to_string(relative).unwrap());
            yaml.push('\n');
        }
        std::fs::write(root.join("v8project.yaml"), yaml).unwrap();
    }

    fn context(root: &Path) -> WorkspaceContext {
        WorkspaceContext {
            cwd: root.to_path_buf(),
            workspace_root: root.to_path_buf(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        }
    }

    fn snapshot_tree(root: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
        if !root.exists() {
            return Vec::new();
        }
        let mut pending = vec![root.to_path_buf()];
        let mut observed = Vec::new();
        while let Some(path) = pending.pop() {
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            let metadata = std::fs::symlink_metadata(&path).unwrap();
            if metadata.is_dir() {
                observed.push((relative, None));
                let mut children = std::fs::read_dir(&path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .collect::<Vec<_>>();
                children.sort();
                pending.extend(children.into_iter().rev());
            } else {
                observed.push((relative, Some(std::fs::read(path).unwrap())));
            }
        }
        observed
    }

    fn temp_root(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "unica-workspace-actor-{name}-{}-{nonce}",
            std::process::id()
        ))
    }
}
