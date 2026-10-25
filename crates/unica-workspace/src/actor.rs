use crate::cache::CacheAccess;
use unica_source_contract::events::DomainEvent;
use unica_source_contract::portable_path::PortablePathExt;
use unica_source_contract::{SourceFormat, SourceProfile, SourceSetKind};
use unica_work_control::shared_work::{
    SharedWork, SharedWorkLease, SharedWorkLifetime, SharedWorkProducer,
};

use crate::apply::ApplyStagedState;
use crate::compile_transaction::CompileTransaction;
use crate::deadline_lock::{DeadlineLock, DeadlineLockError, DeadlineLockErrorKind, FailClosed};
use crate::paths::{normalize_path_identity, GENERATED_DIR_NAME};
use crate::platform::filesystem::{
    path_starts_with_host_root, stable_path_identity_bytes, strip_windows_extended_length_prefix,
    RetainedChildCapability, RetainedDirectoryCapability,
};
use crate::policy_evidence::{
    RetainedSupportPolicyEvidence, SupportPolicyEvidenceError, SupportPolicyEvidenceErrorKind,
};
use crate::selection_evidence::{
    discover_project_source_admission, ResolvedProjectSourceAdmission,
    RetainedSourceSelectionEvidence, SourceSelectionEvidenceError,
    SourceSelectionEvidenceErrorKind,
};
use crate::state_scope::WorkspaceStateScope;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::{OsStr, OsString};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::MutexGuard;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};
use unica_execution_contract::invocation::{DomainResult, SafeIdentityHash};
use unica_source_contract::proposal::PlannedApplyEffects;
use unica_source_contract::proposal::{
    ApplyPlanError, ApplyPlanErrorKind, ApplyStagingError, ApplyStagingErrorKind, SupportPolicyMode,
};
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::{CancellationToken, FactualOutcomeAuthority};
use unica_work_control::provider_deadline::ProviderDeadline;

#[cfg(not(any(test, feature = "test-support")))]
fn actor_retention_now() -> Instant {
    Instant::now()
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static ACTOR_RETENTION_TEST_NOW: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}

#[cfg(any(test, feature = "test-support"))]
fn actor_retention_now() -> Instant {
    ACTOR_RETENTION_TEST_NOW.with(|clock| clock.get().unwrap_or_else(Instant::now))
}

#[cfg(any(test, feature = "test-support"))]
pub fn with_actor_retention_time_for_test<T>(now: Instant, operation: impl FnOnce() -> T) -> T {
    struct RestoreTime(Option<Instant>);
    impl Drop for RestoreTime {
        fn drop(&mut self) {
            ACTOR_RETENTION_TEST_NOW.with(|clock| clock.set(self.0));
        }
    }
    let _restore = RestoreTime(ACTOR_RETENTION_TEST_NOW.with(|clock| clock.replace(Some(now))));
    operation()
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static APPLY_BEFORE_INPUT_CONFIRMATION_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = std::cell::RefCell::new(None);
    static APPLY_DRY_RUN_AFTER_CONFIRMATION_HOOK: std::cell::RefCell<
        Option<Box<dyn FnOnce()>>,
    > = std::cell::RefCell::new(None);
}

#[cfg(feature = "test-support")]
fn set_apply_before_input_confirmation_hook(hook: impl FnOnce() + 'static) {
    APPLY_BEFORE_INPUT_CONFIRMATION_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(any(test, feature = "test-support"))]
fn run_apply_before_input_confirmation_hook() {
    APPLY_BEFORE_INPUT_CONFIRMATION_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(feature = "test-support")]
fn set_apply_dry_run_after_confirmation_hook(hook: impl FnOnce() + 'static) {
    APPLY_DRY_RUN_AFTER_CONFIRMATION_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

#[cfg(any(test, feature = "test-support"))]
fn run_apply_dry_run_after_confirmation_hook() {
    APPLY_DRY_RUN_AFTER_CONFIRMATION_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

/// Idle interval after which an unowned actor without promised state releases its
/// retained root descriptors.
pub const WARM_WORKSPACE_ACTOR_TTL: Duration = Duration::from_secs(600);
static APPLY_WRITER_AUTHORITY_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct IndexWorkIdentity([u8; 32]);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IndexWorkFailure {
    Invalidated,
    Unavailable,
}

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
assert_not_impl_production!(PlannedApplyEffects: Clone);
assert_not_impl_production!(PlannedApplyEffects: serde::Serialize);
assert_not_impl_production!(PlannedApplyEffects: serde::de::DeserializeOwned);

/// Exact daemon-local ownership key for mutable workspace state.
///
/// A Git repository is intentionally absent. Linked worktrees have separate
/// canonical roots, revisions and publication lanes even when `.git` points at
/// the same repository metadata.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorkspaceIdentity {
    workspace_root: PathBuf,
    source_sets: Vec<WorkspaceSourceSetIdentity>,
    provider_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WorkspaceSourceSetIdentity {
    name: String,
    root: PathBuf,
    kind: SourceSetKind,
    source_format: SourceFormat,
    source_profile: SourceProfile,
}

/// Complete typed source-set input. The registry canonicalizes and retains its
/// root before it becomes actor authority.
#[derive(Debug, Clone)]
pub struct WorkspaceSourceSetInput {
    name: String,
    root: PathBuf,
    kind: SourceSetKind,
    source_format: SourceFormat,
    source_profile: SourceProfile,
}

impl WorkspaceSourceSetInput {
    pub fn new(
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
    pub fn new<I>(
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

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn source_sets(&self) -> &[WorkspaceSourceSetIdentity] {
        &self.source_sets
    }

    pub fn provider_profile(&self) -> &str {
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

/// Actor-issued read observation for an admitted source root.
/// The retained filesystem capability never leaves this owner through the lease.
#[derive(Debug, Clone)]
pub struct ProviderSourceLease {
    source_root: Arc<RetainedDirectoryCapability>,
    state_scope: WorkspaceStateScope,
}

impl ProviderSourceLease {
    pub fn source_root(&self) -> &Path {
        self.source_root.path()
    }

    pub fn validate_named_identity(&self) -> std::io::Result<()> {
        self.source_root.validate_named_identity()
    }

    pub fn state_scope(&self) -> &WorkspaceStateScope {
        &self.state_scope
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn for_test(source_root: Arc<RetainedDirectoryCapability>) -> Self {
        Self {
            source_root,
            state_scope: WorkspaceStateScope::LegacyPhysical,
        }
    }
}

#[derive(Clone)]
pub struct ProviderRootBinding {
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
    pub fn source_set_name(&self) -> &str {
        &self.source_set.name
    }

    pub const fn source_kind(&self) -> SourceSetKind {
        self.source_set.kind
    }

    pub const fn source_format(&self) -> SourceFormat {
        self.source_set.source_format
    }

    pub const fn source_profile(&self) -> SourceProfile {
        self.source_set.source_profile
    }

    pub fn source_root(&self) -> &Path {
        self.source_root.path()
    }

    pub fn retained_root(&self) -> Arc<unica_source_contract::read::SourceDirectory> {
        Arc::new(crate::read::retained_source_directory(
            (*self.source_root).clone(),
        ))
    }
}

/// Unforgeable admission token for the retained apply publisher. Low-level
/// transaction methods require the same token that created the staged state;
/// infrastructure callers can name the type but cannot construct a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ApplyWriterAuthority(u64);

impl ApplyWriterAuthority {
    pub(crate) fn scope_identity(&self) -> u64 {
        self.0
    }

    fn issue() -> Self {
        Self(APPLY_WRITER_AUTHORITY_SEQUENCE.fetch_add(1, Ordering::Relaxed))
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn apply_writer_authority_for_test() -> ApplyWriterAuthority {
    ApplyWriterAuthority::issue()
}

/// Admission failures describe root, policy and layout authority. The plan
/// token is compared only after the concrete plan has been prepared.
#[derive(Debug)]
pub enum ApplyAdmissionError {
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
            Self::Other(message) => message.fmt(formatter),
        }
    }
}

impl std::error::Error for ApplyAdmissionError {}

/// Actor-issued authority for planning exactly one hidden-v0.13 apply batch.
/// All identity, root, plan, deadline and cancellation fields are closed;
/// callers can stage bytes but cannot substitute publication authority.
pub struct ApplyAdmission {
    actor_identity: WorkspaceIdentity,
    actor_instance: ActorInstanceId,
    source_set: WorkspaceSourceSetIdentity,
    source_root: Arc<RetainedDirectoryCapability>,
    expected_plan: Option<String>,
    request_identity: Vec<u8>,
    read_dependencies: Vec<ApplyReadDependency>,
    dry_run: bool,
    deadline: ProviderDeadline,
    cancellation: CancellationToken,
    writer_authority: ApplyWriterAuthority,
    workspace_cache: WorkspaceCachePublicationAuthority,
    support_policy: RetainedSupportPolicyEvidence,
    source_selection: RetainedSourceSelectionEvidence,
    artifact_classifier: Arc<dyn unica_source_contract::read::SourceArtifactClassifier>,
    context: WorkspaceContext,
}

/// Read-only authority retained by the actor that also owns the destination.
struct ApplyReadDependency {
    binding: ProviderRootBinding,
    reads: Mutex<std::collections::BTreeMap<PathBuf, Vec<u8>>>,
}

/// Admission-sealed authority for dormant Code planning. It borrows the exact
/// actor binding and writer token admitted together with retained policy and
/// source-selection evidence; callers cannot construct or substitute fields.
pub struct CodeApplyAuthority<'a> {
    context: &'a WorkspaceContext,
    binding: &'a ProviderRootBinding,
    writer_authority: &'a ApplyWriterAuthority,
    profile: SourceProfile,
    expected_format: &'static str,
    support_policy: SupportPolicyMode,
}

/// Admission-sealed authority for dormant XDTO planning. It binds the same
/// retained source, writer token, exact Platform XML profile and support
/// evidence as the staged state it is allowed to plan.
pub struct XdtoApplyAuthority<'a> {
    context: &'a WorkspaceContext,
    binding: &'a ProviderRootBinding,
    writer_authority: &'a ApplyWriterAuthority,
    expected_format: &'static str,
    support_policy: SupportPolicyMode,
}

struct PlatformXmlApplyAuthority<'a> {
    binding: &'a ProviderRootBinding,
    writer_authority: &'a ApplyWriterAuthority,
    profile: SourceProfile,
    expected_format: &'static str,
    support_policy: SupportPolicyMode,
    context: &'a WorkspaceContext,
}

/// Admission-sealed authority for dormant metadata/property planning.
pub struct MetadataApplyAuthority<'a>(PlatformXmlApplyAuthority<'a>);

/// Admission-sealed authority for dormant form/resource planning.
pub struct FormResourceApplyAuthority<'a>(PlatformXmlApplyAuthority<'a>);

/// Admission-sealed authority for dormant DCS/MXL planning.
pub struct DcsMxlApplyAuthority<'a>(PlatformXmlApplyAuthority<'a>);

impl PlatformXmlApplyAuthority<'_> {
    fn owns_staged_state(&self, staged: &ApplyStagedState) -> bool {
        staged.retained_root_identity() == self.binding.source_root.identity()
            && staged.has_writer_authority(self.writer_authority)
    }
}

macro_rules! impl_platform_xml_apply_authority {
    ($authority:ident) => {
        impl $authority<'_> {
            pub fn source_set_name(&self) -> &str {
                self.0.binding.source_set_name()
            }

            pub const fn source_kind(&self) -> SourceSetKind {
                self.0.binding.source_kind()
            }

            pub const fn source_profile(&self) -> SourceProfile {
                self.0.profile
            }

            pub const fn expected_format(&self) -> &str {
                self.0.expected_format
            }

            pub const fn support_policy_mode(&self) -> SupportPolicyMode {
                self.0.support_policy
            }

            pub fn owns_staged_state(&self, staged: &ApplyStagedState) -> bool {
                self.0.owns_staged_state(staged)
            }

            /// The admitted workspace, for planners that consult the source
            /// root beyond the staged files (templates, reference scans).
            pub fn workspace_context(&self) -> &WorkspaceContext {
                self.0.context
            }

            /// The physical root of the admitted source set.
            pub fn source_root(&self) -> &Path {
                self.0.binding.source_root()
            }
        }
    };
}

impl_platform_xml_apply_authority!(MetadataApplyAuthority);
impl_platform_xml_apply_authority!(FormResourceApplyAuthority);
impl_platform_xml_apply_authority!(DcsMxlApplyAuthority);

impl XdtoApplyAuthority<'_> {
    pub fn source_set_name(&self) -> &str {
        self.binding.source_set_name()
    }

    pub const fn source_kind(&self) -> SourceSetKind {
        self.binding.source_kind()
    }

    pub const fn expected_format(&self) -> &str {
        self.expected_format
    }

    pub const fn support_policy_mode(&self) -> SupportPolicyMode {
        self.support_policy
    }

    pub fn owns_staged_state(&self, staged: &ApplyStagedState) -> bool {
        staged.retained_root_identity() == self.binding.source_root.identity()
            && staged.has_writer_authority(self.writer_authority)
    }
}

impl CodeApplyAuthority<'_> {
    pub fn source_set_name(&self) -> &str {
        self.binding.source_set_name()
    }

    pub const fn source_kind(&self) -> SourceSetKind {
        self.binding.source_kind()
    }

    pub const fn source_profile(&self) -> SourceProfile {
        self.profile
    }

    pub const fn expected_format(&self) -> &str {
        self.expected_format
    }

    pub const fn support_policy_mode(&self) -> SupportPolicyMode {
        self.support_policy
    }

    pub fn owns_staged_state(&self, staged: &ApplyStagedState) -> bool {
        staged.retained_root_identity() == self.binding.source_root.identity()
            && staged.has_writer_authority(self.writer_authority)
    }
}

macro_rules! impl_readonly_xml_authority {
    ($name:ident) => {
        impl unica_source_contract::proposal::ApplySourceAuthority for $name<'_> {
            fn source_set_name(&self) -> &str {
                self.0.binding.source_set_name()
            }
            fn source_kind(&self) -> SourceSetKind {
                self.0.binding.source_kind()
            }
            fn source_profile(&self) -> SourceProfile {
                self.0.profile
            }
            fn expected_format(&self) -> &str {
                self.0.expected_format
            }
            fn support_policy_mode(&self) -> SupportPolicyMode {
                self.0.support_policy
            }
            fn workspace_context(&self) -> &WorkspaceContext {
                self.0.context
            }
            fn source_root(&self) -> &Path {
                self.0.binding.source_root()
            }
            fn source_reader(&self) -> Arc<unica_source_contract::read::SourceDirectory> {
                self.0.binding.retained_root()
            }
            fn binding(&self) -> unica_source_contract::proposal::ProposalBinding {
                unica_source_contract::proposal::ProposalBinding::from_observation(
                    self.0.binding.source_root.identity(),
                    self.0.writer_authority.scope_identity(),
                )
            }
        }
    };
}
impl_readonly_xml_authority!(MetadataApplyAuthority);
impl_readonly_xml_authority!(FormResourceApplyAuthority);
impl_readonly_xml_authority!(DcsMxlApplyAuthority);

macro_rules! impl_readonly_direct_authority {
    ($name:ident) => {
        impl unica_source_contract::proposal::ApplySourceAuthority for $name<'_> {
            fn source_set_name(&self) -> &str {
                self.binding.source_set_name()
            }
            fn source_kind(&self) -> SourceSetKind {
                self.binding.source_kind()
            }
            fn source_profile(&self) -> SourceProfile {
                self.binding.source_profile()
            }
            fn expected_format(&self) -> &str {
                self.expected_format
            }
            fn support_policy_mode(&self) -> SupportPolicyMode {
                self.support_policy
            }
            fn workspace_context(&self) -> &WorkspaceContext {
                self.context
            }
            fn source_root(&self) -> &Path {
                self.binding.source_root()
            }
            fn source_reader(&self) -> Arc<unica_source_contract::read::SourceDirectory> {
                self.binding.retained_root()
            }
            fn binding(&self) -> unica_source_contract::proposal::ProposalBinding {
                unica_source_contract::proposal::ProposalBinding::from_observation(
                    self.binding.source_root.identity(),
                    self.writer_authority.scope_identity(),
                )
            }
        }
    };
}
impl_readonly_direct_authority!(CodeApplyAuthority);
impl_readonly_direct_authority!(XdtoApplyAuthority);

fn validate_source_artifact_paths(
    policy: &dyn unica_source_contract::read::BoundSourceArtifactPolicy,
    paths: &[PathBuf],
) -> Result<(), ApplyPublicationError> {
    for relative in paths {
        if policy.classify(relative)
            == unica_source_contract::read::RevisionArtifactDisposition::Ignored
        {
            return Err(ApplyPublicationError::new(
                ApplyPublicationErrorKind::Invariant,
                format!(
                    "staged artifact is not part of the supported source profile: {}",
                    relative.portable()
                ),
            ));
        }
    }
    Ok(())
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
pub(crate) struct WorkspaceCacheParticipantAuthority {
    writer_authority: ApplyWriterAuthority,
    logical_root: PathBuf,
    anchor_identity: crate::platform::filesystem::FileIdentity,
}

impl WorkspaceCacheParticipantAuthority {
    pub(crate) fn writer_authority(&self) -> &ApplyWriterAuthority {
        &self.writer_authority
    }

    pub(crate) fn logical_root(&self) -> &Path {
        &self.logical_root
    }

    pub(crate) fn anchor_identity(&self) -> crate::platform::filesystem::FileIdentity {
        self.anchor_identity
    }
}

#[cfg(test)]
pub(crate) fn workspace_cache_participant_authority_for_test(
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
    pub fn bind_request_identity<'a>(
        &mut self,
        at: &str,
        operations: impl IntoIterator<
            Item = (
                &'a str,
                String,
                &'a serde_json::Map<String, serde_json::Value>,
            ),
        >,
    ) {
        let operations = operations
            .into_iter()
            .map(|(name, at, args)| serde_json::json!({"op": name, "at": at, "args": args}))
            .collect::<Vec<_>>();
        let mut value = serde_json::json!({"at": at, "ops": operations});
        value.sort_all_objects();
        self.request_identity = serde_json::to_vec(&value).expect("parsed apply request is JSON");
    }

    fn plan_identity(
        &self,
        state: &ApplyStagedState,
        events: &[DomainEvent],
    ) -> Result<String, ApplyStagingError> {
        fn field(hash: &mut Sha256, bytes: &[u8]) {
            hash.update((bytes.len() as u64).to_be_bytes());
            hash.update(bytes);
        }
        let checkpoint = || {
            apply_staging_checkpoint(self.deadline, &self.cancellation, "apply plan fingerprint")
        };
        checkpoint()?;
        let mut hash = Sha256::new();
        hash.update(b"unica-apply-plan-v2\0");
        field(
            &mut hash,
            &stable_path_identity_bytes(&self.actor_identity.workspace_root).map_err(|error| {
                ApplyStagingError::new(ApplyStagingErrorKind::UnsupportedProvider, error)
            })?,
        );
        field(&mut hash, self.actor_identity.provider_profile.as_bytes());
        field(&mut hash, format!("{:?}", self.source_set).as_bytes());
        field(
            &mut hash,
            format!("{:?}", self.support_policy.mode()).as_bytes(),
        );
        field(&mut hash, &self.request_identity);
        state.fingerprint_inputs(&mut hash)?;
        hash.update((self.read_dependencies.len() as u64).to_be_bytes());
        for dependency in &self.read_dependencies {
            checkpoint()?;
            field(&mut hash, dependency.binding.source_set_name().as_bytes());
            hash.update(dependency.binding.source_root.identity().stable_bytes());
            let reads = dependency.reads.lock().map_err(|_| {
                ApplyStagingError::new(
                    ApplyStagingErrorKind::Invariant,
                    "apply dependency inputs are poisoned",
                )
            })?;
            hash.update((reads.len() as u64).to_be_bytes());
            for (path, bytes) in reads.iter() {
                let path = stable_path_identity_bytes(path).map_err(|error| {
                    ApplyStagingError::new(ApplyStagingErrorKind::UnsupportedProvider, error)
                })?;
                field(&mut hash, &path);
                hash.update((bytes.len() as u64).to_be_bytes());
                for chunk in bytes.chunks(64 * 1024) {
                    checkpoint()?;
                    hash.update(chunk);
                }
            }
        }
        hash.update((events.len() as u64).to_be_bytes());
        for event in events {
            checkpoint()?;
            field(&mut hash, event.name().as_bytes());
            field(&mut hash, event.artifact.as_bytes());
        }
        checkpoint()?;
        Ok(format!("unica-apply-plan-v2:{:x}", hash.finalize()))
    }

    pub fn read_apply_dependency(
        &self,
        source_name: &str,
        relative: &Path,
    ) -> Result<Vec<u8>, ApplyPlanError> {
        apply_staging_checkpoint(self.deadline, &self.cancellation, "apply dependency read")
            .map_err(|error| ApplyPlanError::staging(error, source_name))?;
        let dependency = self
            .read_dependencies
            .iter()
            .find(|dependency| dependency.binding.source_set_name() == source_name)
            .ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidState,
                    "parent source was not admitted for this apply",
                )
            })?;
        dependency
            .binding
            .source_root
            .validate_named_identity()
            .map_err(|error| {
                ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, error.to_string())
            })?;
        let mut bytes = Vec::new();
        let mut stopped = None;
        let read = dependency
            .binding
            .source_root
            .visit_relative_regular_chunks(
                relative,
                || {
                    apply_staging_checkpoint(
                        self.deadline,
                        &self.cancellation,
                        "apply dependency read",
                    )
                    .map_err(|error| {
                        stopped = Some(error);
                        std::io::Error::other("apply dependency read stopped")
                    })
                },
                |chunk| {
                    if chunk.len() > (16 * 1024 * 1024_usize).saturating_sub(bytes.len()) {
                        return Err(std::io::Error::other(
                            "apply dependency exceeds the input limit",
                        ));
                    }
                    bytes.extend_from_slice(chunk);
                    Ok(())
                },
            );
        if let Some(error) = stopped {
            return Err(ApplyPlanError::staging(error, source_name));
        }
        read.map_err(|error| {
            ApplyPlanError::new(
                if error.kind() == std::io::ErrorKind::NotFound {
                    ApplyPlanErrorKind::NotFound
                } else {
                    ApplyPlanErrorKind::InvalidSource
                },
                error.to_string(),
            )
        })?;
        apply_staging_checkpoint(
            self.deadline,
            &self.cancellation,
            "apply dependency read result",
        )
        .map_err(|error| ApplyPlanError::staging(error, source_name))?;
        let mut reads = dependency.reads.lock().map_err(|_| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidState,
                "apply dependency inputs are poisoned",
            )
        })?;
        if reads
            .get(relative)
            .is_some_and(|previous| previous != &bytes)
        {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidState,
                "apply dependency changed while planning",
            ));
        }
        reads.insert(relative.to_path_buf(), bytes.clone());
        Ok(bytes)
    }

    pub fn staged_state(&self) -> Result<ApplyStagedState, ApplyStagingError> {
        apply_staging_checkpoint(self.deadline, &self.cancellation, "apply planning")?;
        Ok(ApplyStagedState::from_retained_root(
            Arc::clone(&self.source_root),
            self.deadline,
            self.cancellation.clone(),
            self.writer_authority.clone(),
        )
        .forbid_generated_subtree())
    }

    pub const fn support_policy_mode(&self) -> SupportPolicyMode {
        self.support_policy.mode()
    }

    pub fn code_planning_authority<'a>(
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
        let profile = (binding.source_profile()
            == SourceProfile::platform_xml_8_3_27_format_2_20())
        .then_some(binding.source_profile())
        .ok_or_else(|| {
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
            context: &self.context,
            binding,
            writer_authority: &self.writer_authority,
            profile,
            expected_format,
            support_policy: self.support_policy.mode(),
        })
    }

    pub fn xdto_planning_authority<'a>(
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
            context: &self.context,
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
        let profile = (binding.source_profile()
            == SourceProfile::platform_xml_8_3_27_format_2_20())
        .then_some(binding.source_profile())
        .ok_or_else(|| {
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

    pub fn metadata_planning_authority<'a>(
        &'a self,
        binding: &'a ProviderRootBinding,
    ) -> Result<MetadataApplyAuthority<'a>, ApplyPlanError> {
        self.platform_xml_family_authority(binding, "metadata")
            .map(MetadataApplyAuthority)
    }

    pub fn form_resource_planning_authority<'a>(
        &'a self,
        binding: &'a ProviderRootBinding,
    ) -> Result<FormResourceApplyAuthority<'a>, ApplyPlanError> {
        self.platform_xml_family_authority(binding, "form/resource")
            .map(FormResourceApplyAuthority)
    }

    pub fn dcs_mxl_planning_authority<'a>(
        &'a self,
        binding: &'a ProviderRootBinding,
    ) -> Result<DcsMxlApplyAuthority<'a>, ApplyPlanError> {
        self.platform_xml_family_authority(binding, "DCS/MXL")
            .map(DcsMxlApplyAuthority)
    }

    pub fn prepare(self, state: ApplyStagedState) -> Result<PreparedApplyBatch, ApplyStagingError> {
        self.prepare_with_effects(state, PlannedApplyEffects::default())
    }

    pub fn prepare_with_effects(
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
        let artifact_policy = self
            .artifact_classifier
            .bind(
                self.source_set.kind,
                self.source_set.source_format,
                self.source_set.source_profile,
            )
            .map_err(|error| {
                ApplyStagingError::new(ApplyStagingErrorKind::UnsupportedProvider, error)
            })?;
        let source_artifact_paths = state
            .planned_changes()
            .into_iter()
            .map(|change| change.relative_path)
            .collect::<Vec<_>>();
        for relative in &source_artifact_paths {
            if artifact_policy.classify(relative)
                == unica_source_contract::read::RevisionArtifactDisposition::Ignored
            {
                return Err(ApplyStagingError::new(
                    ApplyStagingErrorKind::Invariant,
                    format!(
                        "staged artifact is not part of the supported source profile: {}",
                        relative.portable()
                    ),
                ));
            }
        }
        let plan_identity = self.plan_identity(&state, &events)?;
        if self
            .expected_plan
            .as_ref()
            .is_some_and(|expected| expected != &plan_identity)
        {
            return Err(ApplyStagingError::new(ApplyStagingErrorKind::ConcurrentRevision, format!("apply ifRev is stale: expected {}, planned {plan_identity}; prepare a new dryRun preview", self.expected_plan.as_deref().unwrap_or_default())));
        }
        let no_op = state.planned_changes().is_empty() && events.is_empty();
        let source_transaction = state.finalize()?;
        let mut cache_state = ApplyStagedState::from_retained_root(
            Arc::clone(&self.workspace_cache.anchor),
            self.deadline,
            self.cancellation.clone(),
            self.writer_authority.clone(),
        );
        let projected_cache_report = crate::state::WorkspaceStateRepository::new(&self.context)
            .stage_report_in_retained_apply(
                &mut cache_state,
                &self.workspace_cache.relative_root,
                &self.context,
                &events,
                false,
                CacheAccess::default(),
            )?;
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
            plan_identity,
            read_dependencies: self.read_dependencies,
            dry_run: self.dry_run,
            no_op,
            deadline: self.deadline,
            cancellation: self.cancellation,
            transaction,
            writer_authority: self.writer_authority,
            workspace_cache: self.workspace_cache,
            support_policy: self.support_policy,
            source_selection: self.source_selection,
            artifact_policy,
            source_artifact_paths,
            effects: PreparedApplyEffectReceipt {
                events,
                cache: projected_cache_report,
            },
        })
    }
}

#[cfg(any(test, feature = "test-support"))]
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

pub struct PreparedApplyBatch {
    actor_identity: WorkspaceIdentity,
    actor_instance: ActorInstanceId,
    source_set: WorkspaceSourceSetIdentity,
    source_root: Arc<RetainedDirectoryCapability>,
    plan_identity: String,
    read_dependencies: Vec<ApplyReadDependency>,
    dry_run: bool,
    no_op: bool,
    deadline: ProviderDeadline,
    cancellation: CancellationToken,
    transaction: CompileTransaction,
    writer_authority: ApplyWriterAuthority,
    workspace_cache: WorkspaceCachePublicationAuthority,
    support_policy: RetainedSupportPolicyEvidence,
    source_selection: RetainedSourceSelectionEvidence,
    artifact_policy: Arc<dyn unica_source_contract::read::BoundSourceArtifactPolicy>,
    source_artifact_paths: Vec<PathBuf>,
    effects: PreparedApplyEffectReceipt,
}

#[derive(Debug, Clone)]
struct PreparedApplyEffectReceipt {
    events: Vec<DomainEvent>,
    cache: crate::cache::CacheReport,
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

pub(crate) struct RetainedApplyFinalGate<'a, R, C> {
    actor: &'a WorkspaceActor<R, C>,
    binding: ProviderRootBinding,
    deadline: ProviderDeadline,
    cancellation: CancellationToken,
    support_policy: RetainedSupportPolicyEvidence,
    source_selection: RetainedSourceSelectionEvidence,
    read_dependencies: Vec<ApplyReadDependency>,
    artifact_policy: Arc<dyn unica_source_contract::read::BoundSourceArtifactPolicy>,
    source_artifact_paths: Vec<PathBuf>,
}

impl<R, C> RetainedApplyFinalGate<'_, R, C> {
    pub fn checkpoint(&self, phase: &str) -> Result<(), ApplyPublicationError> {
        apply_publication_checkpoint(self.deadline, &self.cancellation, phase)
    }

    /// Final gate after the transaction published its source files. Each
    /// published replacement travels as `(absolute path, post-image)`; the
    /// source-map evidence accepts a new identity for exactly those files
    /// when their bytes match and the selection semantics stay unchanged.
    pub fn validate_after_publication(
        &self,
        published: &[(PathBuf, Vec<u8>)],
    ) -> Result<(), ApplyPublicationError> {
        self.actor
            .validate_binding(&self.binding)
            .map_err(|error| {
                ApplyPublicationError::new(ApplyPublicationErrorKind::ContainmentIdentity, error)
            })?;
        self.checkpoint("prepared apply final gate")?;
        validate_source_artifact_paths(self.artifact_policy.as_ref(), &self.source_artifact_paths)?;
        self.actor.confirm_apply_dependencies(
            &self.read_dependencies,
            self.deadline,
            &self.cancellation,
        )?;
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
pub enum ApplyCleanupDiagnosticKind {
    RetainedRecoveryCleanupIncomplete,
}

/// Bounded actor-facing cleanup context. The target stays relative to the
/// selected source set and the artifact is only the fixed-format internal leaf;
/// neither field exposes the retained provider root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyCleanupDiagnostic {
    kind: ApplyCleanupDiagnosticKind,
    logical_target: PathBuf,
    last_known_artifact_name: OsString,
}

impl ApplyCleanupDiagnostic {
    pub const fn kind(&self) -> ApplyCleanupDiagnosticKind {
        self.kind
    }

    pub fn logical_target(&self) -> &Path {
        &self.logical_target
    }

    /// The transaction-generated recovery leaf before cleanup began. A
    /// concurrent namespace mutation may mean it no longer names the retained
    /// artifact, so callers must not treat this diagnostic as delete authority.
    pub fn last_known_artifact_name(&self) -> &OsStr {
        &self.last_known_artifact_name
    }
}

#[derive(Debug)]
pub struct ApplyPublicationResult {
    rev: String,
    effects: ApplyEffectReceipt,
    commit_count: usize,
    cleanup_diagnostics: Vec<ApplyCleanupDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyEffectDisposition {
    Projected,
    Committed,
}

#[derive(Debug)]
pub struct ApplyEffectReceipt {
    disposition: ApplyEffectDisposition,
    events: Vec<DomainEvent>,
    cache: crate::cache::CacheReport,
}

impl ApplyEffectReceipt {
    pub const fn disposition(&self) -> ApplyEffectDisposition {
        self.disposition
    }

    pub fn events(&self) -> &[DomainEvent] {
        &self.events
    }

    pub const fn cache(&self) -> &crate::cache::CacheReport {
        &self.cache
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyPublicationErrorKind {
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
pub struct ApplyPublicationError {
    kind: ApplyPublicationErrorKind,
    message: String,
}

impl ApplyPublicationError {
    pub fn new(kind: ApplyPublicationErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> ApplyPublicationErrorKind {
        self.kind
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn contains(&self, pattern: &str) -> bool {
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
    pub fn rev(&self) -> &str {
        &self.rev
    }

    pub fn cleanup_diagnostics(&self) -> &[ApplyCleanupDiagnostic] {
        &self.cleanup_diagnostics
    }

    pub const fn effects(&self) -> &ApplyEffectReceipt {
        &self.effects
    }

    #[cfg(any(test, feature = "test-support"))]
    pub const fn commit_count_for_test(&self) -> usize {
        self.commit_count
    }
}

/// Actor-owned runtime payloads expose only a payload-defined projection.
///
/// This prevents the generic actor from returning `&R` in production while
/// still allowing a compatibility adapter to define a module-private view of
/// the state it must bridge during migration.
pub trait WorkspaceActorRuntimeProjection {
    type Projection<'a>
    where
        Self: 'a;

    fn project_for_actor(&self) -> Self::Projection<'_>;
}

#[cfg(any(test, feature = "test-support"))]
pub trait WorkspaceActorRuntimeTestProjection {
    type ProjectionMut<'a>
    where
        Self: 'a;

    fn project_mut_for_actor_test(&mut self) -> Self::ProjectionMut<'_>;
}

/// Failures retaining an already prepared plan describe internal service state.
/// The public request cannot provide or replace a prepared batch's actor identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SavedApplyPlanError {
    ActorMismatch,
    RegistryUnavailable,
}

impl std::fmt::Display for SavedApplyPlanError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ActorMismatch => {
                formatter.write_str("saved apply plan belongs to another workspace actor")
            }
            Self::RegistryUnavailable => formatter.write_str("saved apply plans are unavailable"),
        }
    }
}

#[derive(Debug)]
pub enum SavedApplyExecutionError {
    Busy,
    Unavailable(String),
    RegistryUnavailable,
    StateUnavailable(&'static str),
    Publication(ApplyPublicationError),
}

#[derive(Default)]
struct SavedApplyPlans {
    entries: HashMap<String, Arc<SavedApplyPlan>>,
}
struct SavedApplyPlan {
    diff_layout: std::sync::OnceLock<Box<dyn crate::preview::SavedSourceDiffRenderer>>,
    execution_lane: DeadlineLock<FailClosed>,
    state: Mutex<SavedApplyState>,
}
#[derive(Clone)]
struct SavedApplyOutcome {
    result: DomainResult,
    authority: FactualOutcomeAuthority,
}

impl SavedApplyOutcome {
    fn record_authority(&self, cancellation: &CancellationToken) {
        if self.authority == FactualOutcomeAuthority::CommittedWorkspaceApply {
            cancellation.record_committed_apply();
        }
    }
}

enum SavedApplyState {
    Pending {
        batch: Box<PreparedApplyBatch>,
        preview: DomainResult,
    },
    Completed(SavedApplyOutcome),
    Running,
}

/// One daemon-owned coordination boundary for a canonical worktree.
///
/// Reads retain their selected roots and do not take the mutation lane.
/// Concrete prepared writes publish in that lane after checking their inputs.
pub struct WorkspaceActor<R = (), C = ()> {
    identity: WorkspaceIdentity,
    instance_id: ActorInstanceId,
    context: WorkspaceContext,
    source_roots: HashMap<WorkspaceSourceSetIdentity, Arc<RetainedDirectoryCapability>>,
    state_scope: WorkspaceStateScope,
    mutation_lane: DeadlineLock<FailClosed>,
    saved_apply_plans: Mutex<SavedApplyPlans>,
    /// Parsed root artifacts of each admitted source set, retained across
    /// calls and revalidated by file stamp on every use.
    source_artifacts: Mutex<HashMap<WorkspaceSourceSetIdentity, Arc<C>>>,
    policies: Option<crate::policies::WorkspacePolicies>,
    index_work: SharedWork<IndexWorkIdentity, (), IndexWorkFailure>,
    runtime: R,
}

assert_not_impl_production!(WorkspaceActor<()>: std::ops::Deref);
assert_not_impl_production!(WorkspaceActor<()>: std::ops::DerefMut);

impl<R, C> std::fmt::Debug for WorkspaceActor<R, C> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkspaceActor")
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

impl<C> WorkspaceActor<(), C> {
    pub fn new(identity: WorkspaceIdentity, context: WorkspaceContext) -> Result<Self, String> {
        Self::with_runtime(identity, context, ())
    }
}

fn aggregate_named_root_matches(
    matches: impl IntoIterator<Item = std::io::Result<bool>>,
) -> std::io::Result<bool> {
    let mut unavailable = None;
    for matches in matches {
        match matches {
            Ok(true) => {}
            Ok(false) => return Ok(false),
            Err(error) if unavailable.is_none() => unavailable = Some(error),
            Err(_) => {}
        }
    }
    unavailable.map_or(Ok(true), Err)
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

impl<R, C> WorkspaceActor<R, C> {
    pub fn with_runtime(
        identity: WorkspaceIdentity,
        context: WorkspaceContext,
        runtime: R,
    ) -> Result<Self, String> {
        let state_scope = WorkspaceStateScope::scoped_sha256(identity.state_scope_digest()?)?;
        Self::with_runtime_scope(identity, context, runtime, state_scope)
    }

    pub fn with_legacy_runtime(
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
        context.cache_root = project_workspace_cache_route(&context, &context_root);
        context.workspace_root = context_root.clone();
        context.cwd = context_root;
        Ok(Self {
            identity,
            instance_id: ActorInstanceId::new(),
            context,
            source_roots,
            state_scope,
            mutation_lane: DeadlineLock::fail_closed("workspace actor mutation lane is poisoned"),
            saved_apply_plans: Mutex::new(SavedApplyPlans::default()),
            source_artifacts: Mutex::new(HashMap::new()),
            policies: None,
            index_work: SharedWork::new(SharedWorkLifetime::ProducerBound),
            runtime,
        })
    }

    pub fn with_policies(mut self, policies: crate::policies::WorkspacePolicies) -> Self {
        self.policies = Some(policies);
        self
    }

    pub fn identity(&self) -> &WorkspaceIdentity {
        &self.identity
    }

    fn retains_promised_state(&self) -> bool {
        let saved = self
            .saved_apply_plans
            .lock()
            .map(|plans| !plans.entries.is_empty())
            .unwrap_or(true);
        saved || self.index_work.has_live_entries()
    }

    /// Whether every retained source-set root is still the directory its
    /// name resolves to. A warm actor whose root was replaced while idle
    /// must not be handed to the next admission.
    pub fn retains_named_roots(&self) -> std::io::Result<bool> {
        aggregate_named_root_matches(
            self.source_roots
                .values()
                .map(|root| root.named_identity_matches()),
        )
    }

    pub fn workspace_identity(&self) -> &WorkspaceIdentity {
        &self.identity
    }

    /// Closed daemon-safe identity for task persistence. It is derived from
    /// the actor's complete structural identity; no caller-provided path or
    /// repository label can forge it.
    pub fn safe_identity_hash(&self) -> Result<SafeIdentityHash, String> {
        self.identity
            .state_scope_hash()
            .map(SafeIdentityHash::from_sha256)
    }

    pub fn runtime_projection(&self) -> R::Projection<'_>
    where
        R: WorkspaceActorRuntimeProjection,
    {
        self.runtime.project_for_actor()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn runtime_projection_mut_for_test(&mut self) -> R::ProjectionMut<'_>
    where
        R: WorkspaceActorRuntimeTestProjection,
    {
        self.runtime.project_mut_for_actor_test()
    }

    pub fn context(&self) -> &WorkspaceContext {
        &self.context
    }

    pub fn bind_provider_root(
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

    pub fn read<T>(
        &self,
        binding: &ProviderRootBinding,
        read: impl FnOnce(&Path) -> Result<T, String>,
    ) -> Result<T, String> {
        self.validate_binding(binding)?;
        let result = read(binding.source_root.path());
        self.validate_binding(binding)?;
        result
    }

    pub fn read_relative_file(
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

    pub fn admit_apply(
        &self,
        binding: &ProviderRootBinding,
        if_rev: Option<&str>,
        dry_run: bool,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<ApplyAdmission, ApplyAdmissionError> {
        self.admit_apply_with_dependencies(binding, &[], if_rev, dry_run, deadline, cancellation)
    }

    pub fn admit_apply_with_dependencies(
        &self,
        binding: &ProviderRootBinding,
        dependencies: &[ProviderRootBinding],
        if_rev: Option<&str>,
        dry_run: bool,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<ApplyAdmission, ApplyAdmissionError> {
        apply_checkpoint(deadline, cancellation, "apply admission")?;
        self.validate_binding(binding)?;
        let mut dependency_bindings = dependencies.to_vec();
        dependency_bindings
            .sort_by(|left, right| left.source_set_name().cmp(right.source_set_name()));
        dependency_bindings.dedup_by(|left, right| left.source_set == right.source_set);
        let mut read_dependencies = Vec::new();
        for parent in dependency_bindings {
            self.validate_binding(&parent)?;
            if parent.source_set == binding.source_set
                || parent.source_root.identity() == binding.source_root.identity()
                || parent
                    .source_root
                    .path()
                    .starts_with(binding.source_root.path())
                || binding
                    .source_root
                    .path()
                    .starts_with(parent.source_root.path())
            {
                return Err(ApplyAdmissionError::Other(
                    "borrow parent must be a separate non-overlapping source root".to_string(),
                ));
            }
            if parent.source_kind() != SourceSetKind::Configuration
                || parent.source_format() != SourceFormat::PlatformXml
                || parent.source_profile() != SourceProfile::platform_xml_8_3_27_format_2_20()
            {
                return Err(ApplyAdmissionError::Other(
                    "borrow parent must provide the supported Platform XML configuration profile"
                        .to_string(),
                ));
            }
            read_dependencies.push(ApplyReadDependency {
                binding: parent,
                reads: Mutex::new(std::collections::BTreeMap::new()),
            });
        }
        let source_selection = {
            let mut checkpoint =
                || apply_checkpoint(deadline, cancellation, "apply source-map admission");
            let resolved = discover_project_source_admission(
                &self.context.workspace_root,
                self.policies
                    .as_ref()
                    .ok_or_else(|| "workspace source discovery is not bound".to_string())?
                    .discovery
                    .clone(),
                &mut checkpoint,
            )?;
            self.validate_source_selection_admission(binding, &resolved)?;
            for parent in &read_dependencies {
                self.validate_source_selection_admission(&parent.binding, &resolved)?;
            }
            resolved.into_evidence()
        };
        let writer_authority = ApplyWriterAuthority::issue();
        let workspace_cache = retain_workspace_cache_publication_authority(
            &self.context,
            &binding.source_root,
            &writer_authority,
        )?;
        let mut context = self.context.clone();
        context.cache_root = workspace_cache.logical_root.clone();
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
            expected_plan: if_rev.map(str::to_string),
            request_identity: Vec::new(),
            read_dependencies,
            dry_run,
            deadline,
            cancellation: cancellation.clone(),
            writer_authority,
            workspace_cache,
            support_policy,
            source_selection,
            artifact_classifier: self
                .policies
                .as_ref()
                .ok_or_else(|| "workspace artifact classifier is not bound".to_string())?
                .artifacts
                .clone(),
            context,
        })
    }

    fn confirm_apply_dependencies(
        &self,
        dependencies: &[ApplyReadDependency],
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), ApplyPublicationError> {
        for dependency in dependencies {
            self.validate_binding(&dependency.binding)
                .map_err(|error| {
                    ApplyPublicationError::new(
                        ApplyPublicationErrorKind::ContainmentIdentity,
                        error,
                    )
                })?;
            let reads = dependency.reads.lock().map_err(|_| {
                ApplyPublicationError::new(
                    ApplyPublicationErrorKind::Invariant,
                    "apply dependency inputs are poisoned",
                )
            })?;
            for (path, expected) in reads.iter() {
                apply_publication_checkpoint(
                    deadline,
                    cancellation,
                    "apply dependency confirmation",
                )?;
                let mut offset = 0;
                let mut stopped = None;
                let read = dependency
                    .binding
                    .source_root
                    .visit_relative_regular_chunks(
                        path,
                        || {
                            apply_publication_checkpoint(
                                deadline,
                                cancellation,
                                "apply dependency confirmation",
                            )
                            .map_err(|error| {
                                stopped = Some(error);
                                std::io::Error::other("apply dependency confirmation stopped")
                            })
                        },
                        |chunk| {
                            if expected.get(offset..offset + chunk.len()) != Some(chunk) {
                                return Err(std::io::Error::other(
                                    "apply dependency bytes changed",
                                ));
                            }
                            offset += chunk.len();
                            Ok(())
                        },
                    );
                if let Some(error) = stopped {
                    return Err(error);
                }
                read.map_err(|error| {
                    ApplyPublicationError::new(
                        ApplyPublicationErrorKind::ConcurrentRevision,
                        format!(
                            "read-only input `{}:{}` changed; the plan was prepared using this source dependency: {error}",
                            dependency.binding.source_set_name(),
                            path.portable(),
                        ),
                    )
                })?;
                if offset != expected.len() {
                    return Err(ApplyPublicationError::new(
                        ApplyPublicationErrorKind::ConcurrentRevision,
                        format!(
                            "read-only input `{}:{}` changed; the plan was prepared using this source dependency",
                            dependency.binding.source_set_name(),
                            path.portable(),
                        ),
                    ));
                }
            }
            self.validate_binding(&dependency.binding)
                .map_err(|error| {
                    ApplyPublicationError::new(
                        ApplyPublicationErrorKind::ContainmentIdentity,
                        error,
                    )
                })?;
        }
        Ok(())
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

    pub fn save_prepared_apply(
        &self,
        prepared: PreparedApplyBatch,
        preview: DomainResult,
    ) -> Result<String, SavedApplyPlanError> {
        if prepared.actor_identity != self.identity || prepared.actor_instance != self.instance_id {
            return Err(SavedApplyPlanError::ActorMismatch);
        }
        let mut plans = self
            .saved_apply_plans
            .lock()
            .map_err(|_| SavedApplyPlanError::RegistryUnavailable)?;
        let token = uuid::Uuid::new_v4().to_string();
        plans.entries.insert(
            token.clone(),
            Arc::new(SavedApplyPlan {
                diff_layout: std::sync::OnceLock::new(),
                execution_lane: DeadlineLock::fail_closed("saved apply execution lane is poisoned"),
                state: Mutex::new(SavedApplyState::Pending {
                    batch: Box::new(prepared),
                    preview,
                }),
            }),
        );
        Ok(token)
    }

    pub fn read_saved_apply_diff(
        &self,
        token: &str,
        limit: usize,
        cursor: Option<&str>,
        cancellation: &CancellationToken,
        renderer: crate::preview::SavedSourceDiffFactory,
    ) -> Result<serde_json::Value, SavedApplyExecutionError> {
        let entry = self
            .saved_apply_plans
            .lock()
            .map_err(|_| SavedApplyExecutionError::RegistryUnavailable)?
            .entries
            .get(token)
            .cloned()
            .ok_or_else(|| {
                SavedApplyExecutionError::Unavailable(
                    "executionToken is unavailable; request a fresh apply plan".into(),
                )
            })?;
        let mut state = entry.state.try_lock().map_err(|error| match error {
            std::sync::TryLockError::WouldBlock => SavedApplyExecutionError::Busy,
            std::sync::TryLockError::Poisoned(_) => {
                SavedApplyExecutionError::StateUnavailable("saved plan state is unavailable")
            }
        })?;
        let SavedApplyState::Pending { batch, .. } = &mut *state else {
            return Err(SavedApplyExecutionError::Unavailable(
                "source preview is available only before saved plan execution".into(),
            ));
        };
        let deadline = ProviderDeadline::no_deadline();
        batch.deadline = deadline;
        batch.cancellation = cancellation.clone();
        batch
            .transaction
            .rebind_retained_apply_execution_context(deadline, cancellation);
        let binding = self
            .validate_prepared_apply(batch)
            .map_err(SavedApplyExecutionError::Publication)?;
        self.preview_prepared_apply_result(batch, &binding, ApplyEffectDisposition::Projected)
            .map_err(SavedApplyExecutionError::Publication)?;
        let checkpoint = || {
            if cancellation.is_cancelled() {
                Err("saved source read was cancelled".to_string())
            } else {
                Ok(())
            }
        };
        let images = batch.transaction.saved_source_images();
        let page_error = |message| {
            if cancellation.is_cancelled() {
                SavedApplyExecutionError::Publication(ApplyPublicationError::new(
                    ApplyPublicationErrorKind::Cancelled,
                    message,
                ))
            } else {
                SavedApplyExecutionError::Unavailable(message)
            }
        };
        if entry.diff_layout.get().is_none() {
            let layout = renderer(&images, &checkpoint).map_err(page_error)?;
            let _ = entry.diff_layout.set(layout);
        }
        let key = serde_json::json!(["saved-source-diff-v1", token, batch.plan_identity, limit])
            .to_string();
        let mut page = entry
            .diff_layout
            .get()
            .expect("initialized layout")
            .page(&images, &key, limit, cursor, &checkpoint)
            .map_err(page_error)?;
        page["planIdentity"] = serde_json::json!(batch.plan_identity);
        page["sourceSet"] = serde_json::json!(batch.source_set.name);
        checkpoint().map_err(page_error)?;
        Ok(page)
    }

    pub fn execute_saved_apply(
        &self,
        token: &str,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
        finish: impl FnOnce(
            DomainResult,
            Result<ApplyPublicationResult, ApplyPublicationError>,
        ) -> DomainResult,
    ) -> Result<DomainResult, SavedApplyExecutionError> {
        let entry = self.saved_apply_plans.lock().map_err(|_| SavedApplyExecutionError::RegistryUnavailable)?
            .entries.get(token).cloned()
            .ok_or_else(|| SavedApplyExecutionError::Unavailable("executionToken is unavailable in this workspace actor; request a fresh plan with apply(at, ops)".into()))?;
        // One token retains its terminal result across distinct RPC invocations.
        // Both lane waits finish before consuming a pending plan; completed
        // replay needs only the per-token lane.
        let _execution = entry
            .execution_lane
            .acquire_before(deadline, cancellation, "saved apply execution wait")
            .map_err(|error| {
                SavedApplyExecutionError::Publication(deadline_lock_publication_error(error))
            })?;
        let mut state = entry.state.lock().map_err(|_| {
            SavedApplyExecutionError::StateUnavailable("saved apply execution state is unavailable")
        })?;
        if let SavedApplyState::Completed(outcome) = &*state {
            outcome.record_authority(cancellation);
            return Ok(outcome.result.clone());
        }
        // Waiting for either lane must leave the exact pending plan usable.
        let publication_lane = self
            .mutation_lane
            .acquire_before(
                deadline,
                cancellation,
                "workspace actor prepared apply wait",
            )
            .map_err(|error| {
                SavedApplyExecutionError::Publication(deadline_lock_publication_error(error))
            })?;
        apply_publication_checkpoint(deadline, cancellation, "saved apply execution start")
            .map_err(SavedApplyExecutionError::Publication)?;
        let SavedApplyState::Pending { mut batch, preview } =
            std::mem::replace(&mut *state, SavedApplyState::Running)
        else {
            return Err(SavedApplyExecutionError::StateUnavailable(
                "saved apply execution state is unavailable",
            ));
        };
        batch.deadline = deadline;
        batch.cancellation = cancellation.clone();
        batch.dry_run = false;
        batch
            .transaction
            .rebind_retained_apply_execution_context(deadline, cancellation);
        let publication = self.publish_prepared_apply_under_lane(*batch, &publication_lane);
        drop(publication_lane);
        let authority = match &publication {
            Ok(publication)
                if publication.effects().disposition() == ApplyEffectDisposition::Committed
                    && publication.commit_count > 0 =>
            {
                FactualOutcomeAuthority::CommittedWorkspaceApply
            }
            _ => FactualOutcomeAuthority::Cancellable,
        };
        if authority == FactualOutcomeAuthority::CommittedWorkspaceApply {
            cancellation.record_committed_apply();
        }
        let result = finish(preview, publication);
        *state = SavedApplyState::Completed(SavedApplyOutcome {
            result: result.clone(),
            authority,
        });
        Ok(result)
    }

    fn validate_prepared_apply(
        &self,
        prepared: &PreparedApplyBatch,
    ) -> Result<ProviderRootBinding, ApplyPublicationError> {
        validate_source_artifact_paths(
            prepared.artifact_policy.as_ref(),
            &prepared.source_artifact_paths,
        )?;

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
            .support_policy
            .validate(prepared.deadline, &prepared.cancellation)
            .map_err(support_policy_publication_error)?;
        prepared
            .source_selection
            .validate(prepared.deadline, &prepared.cancellation)
            .map_err(source_selection_publication_error)?;
        #[cfg(any(test, feature = "test-support"))]
        run_apply_before_input_confirmation_hook();
        self.confirm_apply_dependencies(
            &prepared.read_dependencies,
            prepared.deadline,
            &prepared.cancellation,
        )?;
        Ok(binding)
    }

    fn preview_prepared_apply_result(
        &self,
        prepared: &PreparedApplyBatch,
        binding: &ProviderRootBinding,
        disposition: ApplyEffectDisposition,
    ) -> Result<ApplyPublicationResult, ApplyPublicationError> {
        prepared
            .transaction
            .validate_retained_for_apply_typed()
            .map_err(apply_validation_publication_error)?;
        #[cfg(any(test, feature = "test-support"))]
        run_apply_dry_run_after_confirmation_hook();
        prepared
            .support_policy
            .validate(prepared.deadline, &prepared.cancellation)
            .map_err(support_policy_publication_error)?;
        prepared
            .source_selection
            .validate_dry_result(prepared.deadline, &prepared.cancellation)
            .map_err(source_selection_publication_error)?;
        self.validate_binding(binding).map_err(|error| {
            ApplyPublicationError::new(ApplyPublicationErrorKind::ContainmentIdentity, error)
        })?;
        apply_publication_checkpoint(
            prepared.deadline,
            &prepared.cancellation,
            "prepared apply result",
        )?;
        self.confirm_apply_dependencies(
            &prepared.read_dependencies,
            prepared.deadline,
            &prepared.cancellation,
        )?;
        Ok(ApplyPublicationResult {
            rev: prepared.plan_identity.clone(),
            effects: prepared.effects.clone().into_terminal(disposition),
            commit_count: 0,
            cleanup_diagnostics: Vec::new(),
        })
    }

    pub fn preview_prepared_apply(
        &self,
        prepared: &PreparedApplyBatch,
    ) -> Result<ApplyPublicationResult, ApplyPublicationError> {
        let _lane = self
            .mutation_lane
            .acquire_before(
                prepared.deadline,
                &prepared.cancellation,
                "workspace actor prepared apply wait",
            )
            .map_err(deadline_lock_publication_error)?;
        let binding = self.validate_prepared_apply(prepared)?;
        self.preview_prepared_apply_result(prepared, &binding, ApplyEffectDisposition::Projected)
    }

    pub fn publish_prepared_apply(
        &self,
        prepared: PreparedApplyBatch,
    ) -> Result<ApplyPublicationResult, ApplyPublicationError> {
        let _lane = self
            .mutation_lane
            .acquire_before(
                prepared.deadline,
                &prepared.cancellation,
                "workspace actor prepared apply wait",
            )
            .map_err(deadline_lock_publication_error)?;
        self.publish_prepared_apply_under_lane(prepared, &_lane)
    }

    fn publish_prepared_apply_under_lane(
        &self,
        prepared: PreparedApplyBatch,
        _lane: &MutexGuard<'_, ()>,
    ) -> Result<ApplyPublicationResult, ApplyPublicationError> {
        let binding = self.validate_prepared_apply(&prepared)?;
        if prepared.dry_run || prepared.no_op {
            return self.preview_prepared_apply_result(
                &prepared,
                &binding,
                if prepared.dry_run {
                    ApplyEffectDisposition::Projected
                } else {
                    ApplyEffectDisposition::Committed
                },
            );
        }

        let plan_identity = prepared.plan_identity;
        let final_gate = RetainedApplyFinalGate {
            actor: self,
            binding,
            deadline: prepared.deadline,
            cancellation: prepared.cancellation.clone(),
            support_policy: prepared.support_policy,
            source_selection: prepared.source_selection,
            read_dependencies: prepared.read_dependencies,
            artifact_policy: prepared.artifact_policy,
            source_artifact_paths: prepared.source_artifact_paths,
        };
        let report = prepared
            .transaction
            .commit_retained_apply(prepared.writer_authority, final_gate)?;
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
            rev: plan_identity,
            effects: prepared
                .effects
                .into_terminal(ApplyEffectDisposition::Committed),
            commit_count: 1,
            cleanup_diagnostics,
        })
    }

    /// Shared index work is keyed by the concrete build generation and selected
    /// root. Source freshness is reported separately; admission never scans it.
    pub fn join_index_work<W>(
        &self,
        binding: &ProviderRootBinding,
        generation: &str,
        provider: &str,
        profile: &str,
        work: W,
    ) -> Result<
        (
            IndexWorkIdentity,
            SharedWorkLease<IndexWorkIdentity, (), IndexWorkFailure>,
        ),
        String,
    >
    where
        W: FnOnce(SharedWorkProducer) -> Result<(), IndexWorkFailure> + Send + 'static,
    {
        self.validate_binding(binding)?;
        if !closed_index_component(provider)
            || !closed_index_component(profile)
            || generation.is_empty()
            || generation.len() > 256
            || generation.contains(char::is_control)
        {
            return Err("actor-owned index build identity is invalid".into());
        }
        let mut digest = Sha256::new();
        digest.update(b"unica-index-work-v3\0");
        digest.update(self.identity.state_scope_digest()?);
        digest_index_component(&mut digest, binding.source_set.name.as_bytes())?;
        digest.update(binding.source_root.identity().stable_bytes());
        digest_index_component(&mut digest, generation.as_bytes())?;
        digest_index_component(&mut digest, provider.as_bytes())?;
        digest_index_component(&mut digest, profile.as_bytes())?;
        let identity = IndexWorkIdentity(digest.finalize().into());
        let retained_root = Arc::clone(&binding.source_root);
        let lease = self
            .index_work
            .join_or_start(identity.clone(), move |producer| {
                retained_root
                    .validate_named_identity()
                    .map_err(|_| IndexWorkFailure::Invalidated)?;
                work(producer)
            });
        Ok((identity, lease))
    }

    /// Holds the exclusive mutation lane, as a concurrent apply publication does.
    #[cfg(any(test, feature = "test-support"))]
    pub fn hold_mutation_lane_for_test(&self) -> MutexGuard<'_, ()> {
        self.mutation_lane.hold_for_test()
    }

    /// The root-artifact cache this actor retains for one bound source set.
    pub fn source_artifacts(&self, binding: &ProviderRootBinding) -> Result<Arc<C>, String>
    where
        C: Default,
    {
        self.validate_binding(binding)?;
        let mut artifacts = self
            .source_artifacts
            .lock()
            .map_err(|_| "workspace actor source-artifact cache is poisoned".to_string())?;
        Ok(Arc::clone(
            artifacts.entry(binding.source_set.clone()).or_default(),
        ))
    }

    pub fn provider_source_lease(
        &self,
        binding: &ProviderRootBinding,
    ) -> Result<ProviderSourceLease, String> {
        self.validate_binding(binding)?;
        Ok(ProviderSourceLease {
            source_root: Arc::clone(&binding.source_root),
            state_scope: self.state_scope.clone(),
        })
    }

    pub fn validate_binding(&self, binding: &ProviderRootBinding) -> Result<(), String> {
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
    map: &unica_source_contract::ProjectSourceMap,
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

/// A workspace alias is already accepted by actor identity admission. Reuse only
/// the common part of that route, and leave the cache-only suffix untouched so
/// a link there cannot disappear through canonicalization.
fn project_workspace_cache_route(context: &WorkspaceContext, workspace_root: &Path) -> PathBuf {
    let Ok(requested) = absolute_cache_route(&context.cache_root) else {
        // An unavailable relative base is an apply refusal, not an actor/read
        // construction failure.
        return context.cache_root.clone();
    };
    let original_workspace = strip_windows_extended_length_prefix(&context.workspace_root);
    let Some(common) = original_workspace
        .ancestors()
        .find(|ancestor| requested.starts_with(ancestor))
    else {
        return requested;
    };
    let Ok(canonical_common) = normalize_path_identity(common) else {
        return requested;
    };
    let workspace_suffix = original_workspace.strip_prefix(common).expect("ancestor");
    let Ok((projected_workspace, missing)) =
        retain_cache_route(&canonical_common.join(workspace_suffix))
    else {
        return requested;
    };
    let Ok(expected_workspace) = RetainedDirectoryCapability::open(workspace_root) else {
        return requested;
    };
    // Do not canonicalize the workspace suffix: a second alias could otherwise
    // disguise a changed common ancestor as the original workspace identity.
    if !missing.as_os_str().is_empty()
        || projected_workspace.identity() != expected_workspace.identity()
    {
        return requested;
    }
    canonical_common.join(requested.strip_prefix(common).expect("common prefix"))
}

/// Resolve only the base of a relative override. In particular Windows
/// `D:cache` uses that drive's current directory, but `link/../cache` must not
/// pass through `absolute` (which would erase the link before admission).
fn absolute_cache_route(path: &Path) -> Result<PathBuf, String> {
    use std::path::{Component, Prefix};

    let path = strip_windows_extended_length_prefix(path);
    if path.is_absolute() {
        return Ok(path);
    }
    let mut components = path.components();
    if let Some(Component::Prefix(prefix)) = components.next() {
        if matches!(prefix.kind(), Prefix::Disk(_)) {
            let mut drive_base = PathBuf::from(prefix.as_os_str());
            drive_base.push(".");
            let drive_base = std::path::absolute(drive_base)
                .map_err(|error| format!("workspace cache drive base is unavailable: {error}"))?;
            return Ok(strip_windows_extended_length_prefix(&drive_base).join(components.as_path()));
        }
    }
    let base = std::env::current_dir()
        .map_err(|error| format!("workspace cache current directory is unavailable: {error}"))?;
    Ok(strip_windows_extended_length_prefix(&base).join(path))
}

/// Retain existing route components without following links or creating the
/// missing tail. Parent components are resolved only after inspecting the
/// component they cancel, so `link/../cache` cannot erase a link check.
fn retain_cache_route(path: &Path) -> Result<(RetainedDirectoryCapability, PathBuf), String> {
    use std::path::Component;

    let absolute = absolute_cache_route(path)?;
    let mut components = absolute.components().peekable();
    let mut namespace_root = PathBuf::new();
    while let Some(Component::Prefix(_) | Component::RootDir) = components.peek() {
        namespace_root.push(components.next().expect("peeked component").as_os_str());
    }
    if !namespace_root.is_absolute() {
        return Err("workspace cache route must have an absolute namespace root".to_string());
    }
    let root = RetainedDirectoryCapability::open(&namespace_root)
        .map_err(|error| format!("workspace cache ancestor cannot be retained: {error}"))?;
    let mut retained = vec![root];
    let mut missing = PathBuf::new();
    for component in components {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !missing.pop() && retained.len() > 1 {
                    retained.pop();
                }
            }
            Component::Normal(name) if !missing.as_os_str().is_empty() => missing.push(name),
            Component::Normal(name) => {
                let anchor = retained.last().expect("namespace root remains retained");
                match anchor.retain_immediate_child_nofollow(name) {
                    Ok(RetainedChildCapability::Directory(directory)) => retained.push(directory),
                    Err(error) if error.kind() == ErrorKind::NotFound => missing.push(name),
                    Ok(RetainedChildCapability::ReparsePoint) => {
                        return Err("workspace cache authority encountered a link/reparse point"
                            .to_string())
                    }
                    Ok(_) => {
                        return Err(
                            "workspace cache authority encountered a non-directory component"
                                .to_string(),
                        )
                    }
                    Err(error) => {
                        return Err(format!(
                            "workspace cache authority cannot retain its path: {error}"
                        ))
                    }
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err("workspace cache route contains a misplaced namespace root".to_string());
            }
        }
    }
    let anchor = retained.pop().expect("namespace root remains retained");
    anchor
        .validate_named_identity()
        .map_err(|error| format!("workspace cache ancestor changed during admission: {error}"))?;
    Ok((anchor, missing))
}

fn retain_workspace_cache_publication_authority(
    context: &WorkspaceContext,
    source_root: &RetainedDirectoryCapability,
    writer_authority: &ApplyWriterAuthority,
) -> Result<WorkspaceCachePublicationAuthority, String> {
    let (anchor, relative_root) = retain_cache_route(&context.cache_root)?;
    let cache_root = anchor.path().join(&relative_root);
    // Physical spelling is additional overlap evidence, never the route used
    // to obtain authority. The requested route has already passed no-follow.
    let cache_identity = normalize_path_identity(&cache_root)?;
    let source_root_path = normalize_path_identity(source_root.path())?;
    if source_root_path.starts_with(&cache_identity) {
        return Err(
            "workspace cache/source overlap: roots must not contain one another".to_string(),
        );
    }
    if let Ok(relative_cache) = cache_identity.strip_prefix(&source_root_path) {
        let expected = Path::new(GENERATED_DIR_NAME).join("unica");
        if relative_cache != expected {
            return Err(
                "workspace cache/source overlap is only allowed for exact .build/unica".to_string(),
            );
        }
    }
    if relative_root.as_os_str().is_empty() && anchor.identity() == source_root.identity() {
        return Err(
            "workspace cache and source roots resolve to one physical directory".to_string(),
        );
    }
    anchor
        .validate_named_identity()
        .map_err(|error| format!("workspace cache ancestor changed during admission: {error}"))?;
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
    } else if deadline.is_elapsed() {
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
    } else if deadline.is_elapsed() {
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

fn apply_validation_publication_error(
    error: crate::compile_transaction::RetainedApplyValidationError,
) -> ApplyPublicationError {
    use crate::compile_transaction::RetainedApplyValidationErrorKind;

    let kind = match error.kind() {
        RetainedApplyValidationErrorKind::Cancelled => ApplyPublicationErrorKind::Cancelled,
        RetainedApplyValidationErrorKind::Deadline => ApplyPublicationErrorKind::Deadline,
        RetainedApplyValidationErrorKind::ConcurrentRevision => {
            ApplyPublicationErrorKind::ConcurrentRevision
        }
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
    } else if deadline.is_elapsed() {
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

fn digest_index_component(digest: &mut Sha256, bytes: &[u8]) -> Result<(), String> {
    let length = u64::try_from(bytes.len())
        .map_err(|_| "actor-owned index work component is too large".to_string())?;
    digest.update(length.to_le_bytes());
    digest.update(bytes);
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceActorRegistryError {
    InvalidIdentity(String),
    NamedRootUnavailable,
    Poisoned,
}

impl std::fmt::Display for WorkspaceActorRegistryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidIdentity(message) => formatter.write_str(message),
            Self::NamedRootUnavailable => {
                formatter.write_str("workspace actor named root is temporarily unavailable")
            }
            Self::Poisoned => formatter.write_str("workspace actor registry is poisoned"),
        }
    }
}

impl std::error::Error for WorkspaceActorRegistryError {}

pub struct WorkspaceActorRegistry<C = ()> {
    actors: Mutex<HashMap<WorkspaceIdentity, Weak<WorkspaceActor<(), C>>>>,
    /// Strong ownership between invocations. Promised plans, terminal replay
    /// and shared index work keep their actor alive for the daemon lifetime.
    /// Only unowned actors without such state may be released after idle.
    warm: Mutex<VecDeque<WarmWorkspaceActor<C>>>,
    warm_ttl: Duration,
    policies: Option<crate::policies::WorkspacePolicies>,
    #[cfg(any(test, feature = "test-support"))]
    named_root_validation_error: Mutex<Option<ErrorKind>>,
}

struct WarmWorkspaceActor<C> {
    actor: Arc<WorkspaceActor<(), C>>,
    last_used: Instant,
}

impl<C> WarmWorkspaceActor<C> {
    fn retains_owner(&self) -> bool {
        // Registry locks prevent new Weak upgrades. Check live owners before
        // state: an admitted invocation may still publish a plan or index work.
        Arc::strong_count(&self.actor) > 1 || self.actor.retains_promised_state()
    }
}

impl<C> Default for WorkspaceActorRegistry<C> {
    fn default() -> Self {
        Self {
            actors: Mutex::new(HashMap::new()),
            warm: Mutex::new(VecDeque::new()),
            warm_ttl: WARM_WORKSPACE_ACTOR_TTL,
            policies: None,
            #[cfg(any(test, feature = "test-support"))]
            named_root_validation_error: Mutex::new(None),
        }
    }
}

impl<C> WorkspaceActorRegistry<C> {
    pub fn with_policies(policies: crate::policies::WorkspacePolicies) -> Self {
        Self {
            policies: Some(policies),
            ..Self::default()
        }
    }

    pub fn get_or_create<I>(
        &self,
        context: &WorkspaceContext,
        source_sets: I,
        provider_profile: &str,
    ) -> Result<Arc<WorkspaceActor<(), C>>, WorkspaceActorRegistryError>
    where
        I: IntoIterator<Item = WorkspaceSourceSetInput>,
    {
        let identity = WorkspaceIdentity::new(context, source_sets, provider_profile)
            .map_err(WorkspaceActorRegistryError::InvalidIdentity)?;
        let now = actor_retention_now();
        let mut actors = self
            .actors
            .lock()
            .map_err(|_| WorkspaceActorRegistryError::Poisoned)?;
        // Lock order is always `actors` then `warm`.
        self.evict_expired_warm(now)?;
        actors.retain(|_, actor| actor.strong_count() > 0);
        if let Some(actor) = actors.get(&identity).and_then(Weak::upgrade) {
            match self.retains_named_roots(&actor) {
                Ok(true) => {
                    self.touch_warm(&actor, now)?;
                    return Ok(actor);
                }
                Err(_) => return Err(WorkspaceActorRegistryError::NamedRootUnavailable),
                Ok(false) => {}
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
        let actor = Arc::new(
            WorkspaceActor::<(), C>::new(identity.clone(), context.clone())
                .map(|actor| {
                    if let Some(policies) = &self.policies {
                        actor.with_policies(policies.clone())
                    } else {
                        actor
                    }
                })
                .map_err(WorkspaceActorRegistryError::InvalidIdentity)?,
        );
        actors.insert(identity, Arc::downgrade(&actor));
        self.touch_warm(&actor, now)?;
        Ok(actor)
    }

    /// An idle listener may stop only when no invocation or promise needs an
    /// actor. Unknown ownership retains the listener instead of losing work.
    pub fn has_retained_work(&self) -> bool {
        let Ok(_actors) = self.actors.lock() else {
            return true;
        };
        let Ok(warm) = self.warm.lock() else {
            return true;
        };
        warm.iter().any(WarmWorkspaceActor::retains_owner)
    }

    /// Releases actors only after live invocations and promised state are gone.
    /// A live invocation may still add a plan or shared index work.
    pub fn release_warm_actors(&self) -> Result<(), WorkspaceActorRegistryError> {
        let _actors = self
            .actors
            .lock()
            .map_err(|_| WorkspaceActorRegistryError::Poisoned)?;
        self.clear_warm()
    }

    /// Releases idle actors only after live invocations and promises are gone.
    pub fn evict_idle_warm_actors(&self) -> Result<(), WorkspaceActorRegistryError> {
        let _actors = self
            .actors
            .lock()
            .map_err(|_| WorkspaceActorRegistryError::Poisoned)?;
        self.evict_expired_warm(actor_retention_now())
    }

    fn warm_lock(
        &self,
    ) -> Result<
        std::sync::MutexGuard<'_, VecDeque<WarmWorkspaceActor<C>>>,
        WorkspaceActorRegistryError,
    > {
        self.warm
            .lock()
            .map_err(|_| WorkspaceActorRegistryError::Poisoned)
    }

    fn retains_named_roots(&self, actor: &WorkspaceActor<(), C>) -> std::io::Result<bool> {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(kind) = self
            .named_root_validation_error
            .lock()
            .expect("named-root validation test seam must not be poisoned")
            .take()
        {
            return Err(std::io::Error::from(kind));
        }
        actor.retains_named_roots()
    }

    fn touch_warm(
        &self,
        actor: &Arc<WorkspaceActor<(), C>>,
        now: Instant,
    ) -> Result<(), WorkspaceActorRegistryError> {
        let mut warm = self.warm_lock()?;
        warm.retain(|entry| !Arc::ptr_eq(&entry.actor, actor));
        warm.push_back(WarmWorkspaceActor {
            actor: Arc::clone(actor),
            last_used: now,
        });
        Ok(())
    }

    fn evict_expired_warm(&self, now: Instant) -> Result<(), WorkspaceActorRegistryError> {
        let mut warm = self.warm_lock()?;
        warm.retain(|entry| {
            now.saturating_duration_since(entry.last_used) < self.warm_ttl || entry.retains_owner()
        });
        Ok(())
    }

    fn forget_warm(&self, identity: &WorkspaceIdentity) -> Result<(), WorkspaceActorRegistryError> {
        let mut warm = self.warm_lock()?;
        warm.retain(|entry| entry.actor.identity() != identity);
        Ok(())
    }

    fn clear_warm(&self) -> Result<(), WorkspaceActorRegistryError> {
        self.warm_lock()?.retain(WarmWorkspaceActor::retains_owner);
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_idle_policy_for_test(warm_ttl: Duration) -> Self {
        Self {
            warm_ttl,
            ..Self::default()
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn warm_len_for_test(&self) -> Result<usize, String> {
        self.warm
            .lock()
            .map(|warm| warm.len())
            .map_err(|_| "workspace actor registry is poisoned".to_string())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn live_len_for_test(&self) -> Result<usize, String> {
        self.len()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn entry_len_for_test(&self) -> Result<usize, String> {
        self.actors
            .lock()
            .map(|actors| actors.len())
            .map_err(|_| "workspace actor registry is poisoned".to_string())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn poison_for_test(&self) {
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = self.actors.lock().unwrap();
            panic!("poison workspace actor registry for deterministic admission test");
        }));
        assert!(poisoned.is_err());
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn fail_next_named_root_validation_for_test(&self, kind: ErrorKind) {
        let replaced = self
            .named_root_validation_error
            .lock()
            .expect("named-root validation test seam must not be poisoned")
            .replace(kind);
        assert!(
            replaced.is_none(),
            "named-root validation failure already set"
        );
    }

    #[cfg(test)]
    fn prune_dead_for_test(&self) -> Result<(), String> {
        self.actors
            .lock()
            .map(|mut actors| actors.retain(|_, actor| actor.strong_count() > 0))
            .map_err(|_| "workspace actor registry is poisoned".to_string())
    }
}

impl<C> std::fmt::Debug for WorkspaceActorRegistry<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceActorRegistry")
            .field("actors", &self.actors)
            .field("warm", &self.warm)
            .field("warm_ttl", &self.warm_ttl)
            .finish_non_exhaustive()
    }
}
impl<C> std::fmt::Debug for WarmWorkspaceActor<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WarmWorkspaceActor")
            .field("actor", &self.actor)
            .field("last_used", &self.last_used)
            .finish()
    }
}

#[cfg(feature = "test-support")]
#[path = "workspace_actor_cache_tests.rs"]
pub mod cache_authority_contract_tests;
#[cfg(feature = "test-support")]
#[path = "actor_event_contract_tests.rs"]
pub mod event_contract_tests;
#[cfg(feature = "test-support")]
#[path = "actor_index_contract_tests.rs"]
pub mod index_contract_tests;
#[cfg(feature = "test-support")]
#[path = "actor_policy_contract_tests.rs"]
pub mod policy_contract_tests;
#[cfg(any(test, feature = "test-support"))]
#[path = "actor_retention_contract_tests.rs"]
pub mod retention_contract_tests;
#[cfg(any(test, feature = "test-support"))]
#[path = "../tests/support/actor.rs"]
pub mod test_support;
#[cfg(test)]
#[path = "actor_local_tests.rs"]
mod tests;
