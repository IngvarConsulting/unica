use crate::application::source_resources::{
    ContinueResourceSnapshotRequest, OpenResourceSnapshotRequest, SourceApplyExecution,
    SourceApplyRequest, SourceReadRequest, SourceResourcesRequest,
};
use crate::application::SupportGuardRequirement;
use crate::domain::cancellation::CancellationToken;
use crate::domain::events::{DomainEvent, SourceResourcesReplaced};
use crate::domain::source_resources::{
    EolProfile, ResourceAccess, ResourceCompleteness, ResourceLimits, ResourceManifestPage,
    ResourceRole, ResourceScope, SourceApplyResult, SourceChangedRange, SourceReadResult,
    SourceResource, SourceResourceError, SourceResourceErrorCode, SourceValidationEvidence,
    TextEncoding, TextProfile, SOURCE_MANIFEST_RESOURCE_MAX, SOURCE_READ_LIMIT_MAX,
    SOURCE_REPLACEMENT_MAX_BYTES, SOURCE_RESOURCE_PAGE_LIMIT_MAX, SOURCE_SNAPSHOT_TTL_SECONDS,
};
use crate::domain::source_target::{ResolvedTarget, SourceTargetErrorCode, TargetKind};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::common::{
    detect_format_version, guard_code_patch_resolved_target,
};
use crate::infrastructure::native_operations::compile_transaction::CompileTransaction;
use crate::infrastructure::native_operations::text_snapshot::{
    LineEnding, LineEndingProfile, SourceTextSnapshot,
};
use crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point;
use crate::infrastructure::platform::secure_read::read_root_relative_regular_file;
use crate::infrastructure::platform_xml_source_targets::{
    platform_xml_resource_evidence, resolve_platform_xml_target, ClosedPlatformXmlTarget,
};
use crate::infrastructure::support_guard::{
    evaluate_resolved_support_guard, ResolvedSupportGuardCheck,
};
use diffy::{apply as apply_diff, DiffOptions, Patch};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

const PROVIDER_ID: &str = "platform-xml";
const MAX_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024;
const MAX_LIVE_SNAPSHOT_BYTES: usize = 128 * 1024 * 1024;
const MAX_LIVE_SNAPSHOTS: usize = 64;

#[cfg(test)]
type ConstructionObserver = Box<dyn FnMut(usize) + Send>;

#[cfg(test)]
type PostValidationHook = Box<dyn FnOnce() -> Result<(), String> + Send>;

pub(crate) trait SourceResourceClock: Send + Sync {
    fn now(&self) -> Duration;
}

struct MonotonicClock {
    origin: Instant,
}

impl SourceResourceClock for MonotonicClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

struct StoredResource {
    public: SourceResource,
    path: PathBuf,
    bytes: Arc<[u8]>,
}

struct StoredSnapshot {
    snapshot_id: String,
    provider_id: &'static str,
    workspace_root: PathBuf,
    workspace_epoch: u64,
    source_set: String,
    target: ResolvedTarget,
    handle: ClosedPlatformXmlTarget,
    scope: ResourceScope,
    completeness: ResourceCompleteness,
    manifest_revision: String,
    expires_at: Duration,
    page_size: usize,
    resources: Vec<StoredResource>,
    byte_size: usize,
}

struct ApplySnapshot {
    snapshot_id: String,
    source_set: String,
    target: ResolvedTarget,
    handle: ClosedPlatformXmlTarget,
    resource: SourceResource,
    path: PathBuf,
    preimage: Arc<[u8]>,
}

pub(crate) struct PlatformXmlResourceProvider {
    instance_secret: String,
    clock: Arc<dyn SourceResourceClock>,
    snapshots: Mutex<HashMap<String, StoredSnapshot>>,
    #[cfg(test)]
    phase_hook: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    #[cfg(test)]
    post_validation_hook: Mutex<Option<PostValidationHook>>,
    #[cfg(test)]
    construction_observer: Mutex<Option<ConstructionObserver>>,
}

impl PlatformXmlResourceProvider {
    pub(crate) fn new() -> Self {
        Self::with_clock(Arc::new(MonotonicClock {
            origin: Instant::now(),
        }))
    }

    pub(crate) fn with_clock(clock: Arc<dyn SourceResourceClock>) -> Self {
        Self {
            instance_secret: Uuid::new_v4().to_string(),
            clock,
            snapshots: Mutex::new(HashMap::new()),
            #[cfg(test)]
            phase_hook: Mutex::new(None),
            #[cfg(test)]
            post_validation_hook: Mutex::new(None),
            #[cfg(test)]
            construction_observer: Mutex::new(None),
        }
    }

    pub(crate) fn resources(
        &self,
        request: SourceResourcesRequest,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<ResourceManifestPage, SourceResourceError> {
        self.check_cancelled(cancellation)?;
        match request {
            SourceResourcesRequest::Open(request) => {
                self.open_snapshot(request, context, cancellation)
            }
            SourceResourcesRequest::Continue(request) => {
                self.continue_snapshot(request, context, cancellation)
            }
        }
    }

    pub(crate) fn read(
        &self,
        request: SourceReadRequest,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<SourceReadResult, SourceResourceError> {
        self.check_cancelled(cancellation)?;
        if !(1..=SOURCE_READ_LIMIT_MAX).contains(&request.limit) {
            return Err(public_error(
                SourceResourceErrorCode::LimitExceeded,
                "read limit is outside the public bound",
            ));
        }
        let (resource, snapshot_id) = {
            let mut snapshots = self.snapshots.lock().map_err(|_| {
                public_error(
                    SourceResourceErrorCode::SourceUnavailable,
                    "resource snapshot store is unavailable",
                )
            })?;
            if snapshots
                .get(&request.snapshot_id)
                .is_some_and(|snapshot| self.clock.now() >= snapshot.expires_at)
            {
                snapshots.remove(&request.snapshot_id);
                return Err(public_error(
                    SourceResourceErrorCode::SnapshotExpired,
                    "resource snapshot has expired",
                ));
            }
            let snapshot = snapshots.get(&request.snapshot_id).ok_or_else(|| {
                public_error(
                    SourceResourceErrorCode::SnapshotNotFound,
                    "resource snapshot was not issued by this application instance",
                )
            })?;
            self.validate_snapshot(snapshot, context)?;
            let resource = snapshot
                .resources
                .iter()
                .find(|resource| resource.public.resource_id == request.resource_id)
                .ok_or_else(|| {
                    public_error(
                        SourceResourceErrorCode::ResourceNotFound,
                        "resource does not belong to the supplied snapshot",
                    )
                })?;
            if !resource.public.access.contains(&ResourceAccess::Read) {
                return Err(public_error(
                    SourceResourceErrorCode::ResourceNotReadable,
                    "resource is not readable",
                ));
            }
            (
                (resource.public.clone(), Arc::clone(&resource.bytes)),
                snapshot.snapshot_id.clone(),
            )
        };
        self.run_phase_hook();
        self.check_cancelled(cancellation)?;
        let (public, bytes) = resource;
        if request.offset > bytes.len() {
            return Err(public_error(
                SourceResourceErrorCode::OffsetOutOfRange,
                "byte offset is beyond the resource snapshot",
            ));
        }
        let requested_end = request
            .offset
            .saturating_add(request.limit)
            .min(bytes.len());
        let (content, content_encoding) = if public.text_profile.is_some()
            && std::str::from_utf8(&bytes[request.offset..requested_end]).is_ok()
        {
            (
                std::str::from_utf8(&bytes[request.offset..requested_end])
                    .expect("validated UTF-8 slice")
                    .to_string(),
                "utf-8".to_string(),
            )
        } else {
            (
                base64_encode(&bytes[request.offset..requested_end]),
                "base64".to_string(),
            )
        };
        self.check_cancelled(cancellation)?;
        Ok(SourceReadResult {
            snapshot_id,
            resource_id: public.resource_id,
            offset: request.offset,
            length: requested_end - request.offset,
            size: bytes.len(),
            hash: public.hash,
            content,
            content_encoding,
            eof: requested_end == bytes.len(),
            applied_limit: request.limit,
            text_profile: public.text_profile,
        })
    }

    pub(crate) fn apply(
        &self,
        request: SourceApplyRequest,
        context: &WorkspaceContext,
        dry_run: bool,
        cancellation: &CancellationToken,
    ) -> Result<SourceApplyExecution, SourceResourceError> {
        self.check_cancelled(cancellation)?;
        if request.content.len() > SOURCE_REPLACEMENT_MAX_BYTES {
            return Err(public_error(
                SourceResourceErrorCode::ContentTooLarge,
                "replacement content exceeds the one MiB decoded byte limit",
            ));
        }
        let snapshot = self.apply_snapshot(&request, context)?;
        self.check_cancelled(cancellation)?;
        let evidence = platform_xml_resource_evidence(context, &snapshot.handle)
            .map_err(map_reauthorization_error)?;
        if evidence.target_path != snapshot.path {
            return Err(public_error(
                SourceResourceErrorCode::StaleRevision,
                "logical source resource binding changed after the snapshot",
            ));
        }
        detect_format_version(&evidence.target_path, context).map_err(|_| {
            public_error(
                SourceResourceErrorCode::FormatDenied,
                "the current Platform XML format is not writable",
            )
        })?;
        authorize_support(&evidence.target_path, context)?;
        let current = read_root_relative_regular_file(
            &evidence.source_root,
            &evidence.target_path,
            snapshot.preimage.len(),
            |_| {},
        )
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::FileTooLarge {
                public_error(
                    SourceResourceErrorCode::StaleRevision,
                    "source resource size changed after the snapshot",
                )
            } else {
                public_error(
                    SourceResourceErrorCode::ContainmentDenied,
                    "source resource cannot be safely reauthorized",
                )
            }
        })?;
        if current.bytes.as_slice() != snapshot.preimage.as_ref() {
            return Err(public_error(
                SourceResourceErrorCode::StaleRevision,
                "source resource bytes changed after the snapshot",
            ));
        }
        self.check_cancelled(cancellation)?;

        let postimage = normalized_replacement(
            snapshot.preimage.as_ref(),
            snapshot.resource.text_profile.as_ref(),
            &request.content,
        )?;
        validate_bsl(&postimage)?;
        let no_op = postimage == snapshot.preimage.as_ref();
        let metadata_path = snapshot
            .target
            .metadata_path
            .as_ref()
            .ok_or_else(|| {
                public_error(
                    SourceResourceErrorCode::AtomicityUnproven,
                    "source.apply requires one exact module target",
                )
            })?
            .as_str()
            .to_string();
        let post_hash = sha256_hash(&postimage);
        let result = SourceApplyResult {
            snapshot_id: snapshot.snapshot_id.clone(),
            resource_id: snapshot.resource.resource_id.clone(),
            source_set: snapshot.source_set.clone(),
            target: snapshot.target.clone(),
            role: snapshot.resource.role,
            pre_hash: snapshot.resource.hash.clone(),
            post_hash: post_hash.clone(),
            no_op,
            changed_ranges: changed_ranges(snapshot.preimage.as_ref(), &postimage)?,
            diff: replacement_diff(&metadata_path, snapshot.preimage.as_ref(), &postimage)?,
            validation: SourceValidationEvidence {
                kind: "bsl-analyzer-parser".to_string(),
                status: "passed".to_string(),
            },
        };
        if no_op {
            return Ok(SourceApplyExecution {
                result,
                event: None,
                projected_event: None,
            });
        }
        let event = DomainEvent::source_resources_replaced(SourceResourcesReplaced {
            source_set: snapshot.source_set,
            owner: logical_owner(&metadata_path),
            roles: vec![snapshot.resource.role],
            preimage_hashes: vec![snapshot.resource.hash],
            postimage_hashes: vec![post_hash],
            affected_targets: vec![metadata_path],
        });
        if dry_run {
            return Ok(SourceApplyExecution {
                result,
                event: None,
                projected_event: Some(event),
            });
        }

        let mut transaction = CompileTransaction::new();
        transaction
            .replace_bytes(
                &evidence.target_path,
                snapshot.preimage.as_ref(),
                postimage.clone(),
            )
            .map_err(map_transaction_error)?;
        let revalidated =
            guard_code_patch_resolved_target(&mut transaction, &snapshot.handle, context)
                .map_err(map_transaction_error)?;
        if revalidated != evidence.target_path {
            return Err(public_error(
                SourceResourceErrorCode::StaleRevision,
                "logical source resource binding changed before publication",
            ));
        }
        authorize_support(&revalidated, context)?;
        self.run_phase_hook();
        self.check_cancelled(cancellation)?;
        let report = transaction
            .commit_with_post_validation(|| {
                self.run_post_validation_hook_for_test()?;
                let published = read_root_relative_regular_file(
                    &evidence.source_root,
                    &evidence.target_path,
                    postimage.len(),
                    |_| {},
                )
                .map_err(|_| {
                    "published source resource could not be securely verified".to_string()
                })?;
                if published.bytes != postimage {
                    return Err(
                        "published source resource bytes do not match the mutation plan"
                            .to_string(),
                    );
                }
                Ok(())
            })
            .map_err(map_transaction_error)?;
        if report.updated != [evidence.target_path.clone()] || !report.created.is_empty() {
            return Err(public_error(
                SourceResourceErrorCode::IntegrityFailed,
                "source resource transaction reported an unexpected publication set",
            ));
        }
        Ok(SourceApplyExecution {
            result,
            event: Some(event),
            projected_event: None,
        })
    }

    fn apply_snapshot(
        &self,
        request: &SourceApplyRequest,
        context: &WorkspaceContext,
    ) -> Result<ApplySnapshot, SourceResourceError> {
        let mut snapshots = self.snapshots.lock().map_err(|_| {
            public_error(
                SourceResourceErrorCode::SourceUnavailable,
                "resource snapshot store is unavailable",
            )
        })?;
        if snapshots
            .get(&request.snapshot_id)
            .is_some_and(|snapshot| self.clock.now() >= snapshot.expires_at)
        {
            snapshots.remove(&request.snapshot_id);
            return Err(public_error(
                SourceResourceErrorCode::SnapshotExpired,
                "resource snapshot has expired",
            ));
        }
        let snapshot = snapshots.get(&request.snapshot_id).ok_or_else(|| {
            public_error(
                SourceResourceErrorCode::SnapshotNotFound,
                "resource snapshot was not issued by this application instance",
            )
        })?;
        self.validate_snapshot(snapshot, context)?;
        if snapshot.completeness != ResourceCompleteness::Complete {
            return Err(public_error(
                SourceResourceErrorCode::SnapshotIncomplete,
                "source.apply requires a complete resource snapshot",
            ));
        }
        if snapshot.resources.len() != 1 {
            return Err(public_error(
                SourceResourceErrorCode::AtomicityUnproven,
                "source.apply first contract requires exactly one snapshotted resource",
            ));
        }
        let resource = snapshot
            .resources
            .iter()
            .find(|resource| resource.public.resource_id == request.resource_id)
            .ok_or_else(|| {
                public_error(
                    SourceResourceErrorCode::ResourceNotFound,
                    "resource does not belong to the supplied snapshot",
                )
            })?;
        if resource.public.role != ResourceRole::BslModule
            || !resource.public.access.contains(&ResourceAccess::Replace)
            || resource.public.text_profile.is_none()
        {
            return Err(public_error(
                SourceResourceErrorCode::ResourceNotReplaceable,
                "the snapshotted resource role is not replaceable",
            ));
        }
        if request.expected_hash != resource.public.hash {
            return Err(public_error(
                SourceResourceErrorCode::HashMismatch,
                "expectedHash does not match the immutable resource snapshot",
            ));
        }
        Ok(ApplySnapshot {
            snapshot_id: snapshot.snapshot_id.clone(),
            source_set: snapshot.source_set.clone(),
            target: snapshot.target.clone(),
            handle: snapshot.handle.clone(),
            resource: resource.public.clone(),
            path: resource.path.clone(),
            preimage: Arc::clone(&resource.bytes),
        })
    }

    fn open_snapshot(
        &self,
        request: OpenResourceSnapshotRequest,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<ResourceManifestPage, SourceResourceError> {
        if !(1..=SOURCE_RESOURCE_PAGE_LIMIT_MAX).contains(&request.limit) {
            return Err(public_error(
                SourceResourceErrorCode::LimitExceeded,
                "manifest page limit is outside the public bound",
            ));
        }
        let resolution = resolve_platform_xml_target(context, &request.target).map_err(|_| {
            public_error(
                SourceResourceErrorCode::SourceUnavailable,
                "logical source target is unavailable",
            )
        })?;
        self.run_phase_hook();
        self.check_cancelled(cancellation)?;
        let evidence =
            platform_xml_resource_evidence(context, &resolution.handle).map_err(|_| {
                public_error(
                    SourceResourceErrorCode::SourceUnavailable,
                    "logical source resource evidence is unavailable",
                )
            })?;
        self.check_cancelled(cancellation)?;

        let (candidates, completeness) = match (resolution.resolved.target_kind, request.scope) {
            (TargetKind::Module, ResourceScope::SelfOnly) => (
                vec![(evidence.target_path, ResourceRole::BslModule)],
                ResourceCompleteness::Complete,
            ),
            (TargetKind::Module, ResourceScope::Aggregate) => {
                let mut candidates = vec![(evidence.target_path, ResourceRole::BslModule)];
                candidates.extend(
                    evidence
                        .descriptor_paths
                        .into_iter()
                        .map(|path| (path, ResourceRole::MetadataDescriptor)),
                );
                (candidates, ResourceCompleteness::Partial)
            }
            (_, ResourceScope::Registrations) => (
                vec![(evidence.registration_path, ResourceRole::Registration)],
                ResourceCompleteness::Partial,
            ),
            (TargetKind::SourceRoot, ResourceScope::SelfOnly) => (
                vec![(
                    evidence.registration_path,
                    ResourceRole::ConfigurationDescriptor,
                )],
                ResourceCompleteness::Complete,
            ),
            (TargetKind::SourceRoot, ResourceScope::Aggregate) => (
                scan_root_resources(&evidence.source_root, cancellation)?,
                ResourceCompleteness::Partial,
            ),
            _ => (Vec::new(), ResourceCompleteness::Unavailable),
        };
        self.check_cancelled(cancellation)?;

        let source_root = evidence.source_root.clone();
        let mut stored_resources = Vec::new();
        let mut byte_size = 0_usize;
        for (path, role) in candidates.into_iter().take(SOURCE_MANIFEST_RESOURCE_MAX) {
            self.check_cancelled(cancellation)?;
            let remaining = MAX_SNAPSHOT_BYTES - byte_size;
            let replace_allowed = completeness == ResourceCompleteness::Complete
                && request.scope == ResourceScope::SelfOnly
                && role == ResourceRole::BslModule;
            let resource =
                snapshot_resource(&source_root, &path, role, replace_allowed, remaining)?;
            byte_size += resource.bytes.len();
            stored_resources.push(resource);
            self.observe_construction_for_test(byte_size);
        }
        let revision = manifest_revision(&resolution.resolved, request.scope, &stored_resources);
        let snapshot_id = Uuid::new_v4().to_string();
        let workspace_root = fs::canonicalize(&context.workspace_root).map_err(|_| {
            public_error(
                SourceResourceErrorCode::SourceUnavailable,
                "workspace identity is unavailable",
            )
        })?;
        let snapshot = StoredSnapshot {
            snapshot_id: snapshot_id.clone(),
            provider_id: PROVIDER_ID,
            workspace_root,
            workspace_epoch: context.workspace_epoch,
            source_set: resolution.resolved.source_set.clone(),
            target: resolution.resolved,
            handle: resolution.handle,
            scope: request.scope,
            completeness,
            manifest_revision: revision,
            expires_at: self.clock.now() + Duration::from_secs(SOURCE_SNAPSHOT_TTL_SECONDS),
            page_size: request.limit,
            resources: stored_resources,
            byte_size,
        };
        self.check_cancelled(cancellation)?;
        let page = self.page(&snapshot, 0);
        let mut snapshots = self.snapshots.lock().map_err(|_| {
            public_error(
                SourceResourceErrorCode::SourceUnavailable,
                "resource snapshot store is unavailable",
            )
        })?;
        let now = self.clock.now();
        snapshots.retain(|_, stored| now < stored.expires_at);
        let live_bytes = snapshots
            .values()
            .map(|stored| stored.byte_size)
            .sum::<usize>();
        if !within_live_capacity(snapshots.len(), live_bytes, snapshot.byte_size) {
            return Err(capacity_error());
        }
        snapshots.insert(snapshot_id, snapshot);
        Ok(page)
    }

    fn continue_snapshot(
        &self,
        request: ContinueResourceSnapshotRequest,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<ResourceManifestPage, SourceResourceError> {
        self.check_cancelled(cancellation)?;
        let mut snapshots = self.snapshots.lock().map_err(|_| {
            public_error(
                SourceResourceErrorCode::SourceUnavailable,
                "resource snapshot store is unavailable",
            )
        })?;
        if snapshots
            .get(&request.snapshot_id)
            .is_some_and(|snapshot| self.clock.now() >= snapshot.expires_at)
        {
            snapshots.remove(&request.snapshot_id);
            return Err(public_error(
                SourceResourceErrorCode::SnapshotExpired,
                "resource snapshot has expired",
            ));
        }
        let snapshot = snapshots.get(&request.snapshot_id).ok_or_else(|| {
            public_error(
                SourceResourceErrorCode::SnapshotNotFound,
                "resource snapshot was not issued by this application instance",
            )
        })?;
        self.validate_snapshot(snapshot, context)?;
        if request
            .limit
            .is_some_and(|limit| limit != snapshot.page_size)
        {
            return Err(public_error(
                SourceResourceErrorCode::InvalidCursor,
                "cursor is bound to the original manifest page size",
            ));
        }
        let start = (snapshot.page_size..snapshot.resources.len())
            .step_by(snapshot.page_size)
            .find(|offset| self.cursor(snapshot, *offset) == request.cursor)
            .ok_or_else(|| {
                public_error(
                    SourceResourceErrorCode::InvalidCursor,
                    "cursor does not belong to the supplied snapshot request",
                )
            })?;
        self.check_cancelled(cancellation)?;
        Ok(self.page(snapshot, start))
    }

    fn page(&self, snapshot: &StoredSnapshot, start: usize) -> ResourceManifestPage {
        let end = start
            .saturating_add(snapshot.page_size)
            .min(snapshot.resources.len());
        ResourceManifestPage {
            snapshot_id: snapshot.snapshot_id.clone(),
            source_set: snapshot.source_set.clone(),
            target: snapshot.target.clone(),
            scope: snapshot.scope,
            completeness: snapshot.completeness,
            resources: snapshot.resources[start..end]
                .iter()
                .map(|resource| resource.public.clone())
                .collect(),
            next_cursor: (end < snapshot.resources.len()).then(|| self.cursor(snapshot, end)),
        }
    }

    fn cursor(&self, snapshot: &StoredSnapshot, offset: usize) -> String {
        let mut hasher = Sha256::new();
        for component in [
            self.instance_secret.as_str(),
            snapshot.snapshot_id.as_str(),
            snapshot.provider_id,
            snapshot.source_set.as_str(),
            snapshot.manifest_revision.as_str(),
            &format!("{:?}", snapshot.target),
            &format!("{:?}", snapshot.scope),
            &snapshot.page_size.to_string(),
            &offset.to_string(),
        ] {
            hasher.update(component.as_bytes());
            hasher.update([0]);
        }
        hex_digest(hasher.finalize().as_slice())
    }

    fn validate_snapshot(
        &self,
        snapshot: &StoredSnapshot,
        context: &WorkspaceContext,
    ) -> Result<(), SourceResourceError> {
        if self.clock.now() >= snapshot.expires_at {
            return Err(public_error(
                SourceResourceErrorCode::SnapshotExpired,
                "resource snapshot has expired",
            ));
        }
        let workspace_root = fs::canonicalize(&context.workspace_root).map_err(|_| {
            public_error(
                SourceResourceErrorCode::SnapshotScopeMismatch,
                "resource snapshot does not belong to this workspace",
            )
        })?;
        if workspace_root != snapshot.workspace_root
            || context.workspace_epoch != snapshot.workspace_epoch
        {
            return Err(public_error(
                SourceResourceErrorCode::SnapshotScopeMismatch,
                "resource snapshot does not belong to this workspace",
            ));
        }
        Ok(())
    }

    fn check_cancelled(&self, cancellation: &CancellationToken) -> Result<(), SourceResourceError> {
        if cancellation.is_cancelled() {
            Err(public_error(
                SourceResourceErrorCode::Cancelled,
                "source resource operation was cancelled",
            ))
        } else {
            Ok(())
        }
    }

    #[cfg(test)]
    fn set_phase_hook_for_test(&self, hook: impl FnOnce() + Send + 'static) {
        let previous = self.phase_hook.lock().unwrap().replace(Box::new(hook));
        assert!(previous.is_none(), "source resource phase hook leaked");
    }

    #[cfg(test)]
    fn run_phase_hook(&self) {
        if let Some(hook) = self.phase_hook.lock().unwrap().take() {
            hook();
        }
    }

    #[cfg(not(test))]
    fn run_phase_hook(&self) {}

    #[cfg(test)]
    fn set_post_validation_hook_for_test(
        &self,
        hook: impl FnOnce() -> Result<(), String> + Send + 'static,
    ) {
        let previous = self
            .post_validation_hook
            .lock()
            .unwrap()
            .replace(Box::new(hook));
        assert!(
            previous.is_none(),
            "source resource post-validation hook leaked"
        );
    }

    #[cfg(test)]
    fn run_post_validation_hook_for_test(&self) -> Result<(), String> {
        match self.post_validation_hook.lock().unwrap().take() {
            Some(hook) => hook(),
            None => Ok(()),
        }
    }

    #[cfg(not(test))]
    fn run_post_validation_hook_for_test(&self) -> Result<(), String> {
        Ok(())
    }

    #[cfg(test)]
    fn set_construction_observer_for_test(&self, observer: impl FnMut(usize) + Send + 'static) {
        let previous = self
            .construction_observer
            .lock()
            .unwrap()
            .replace(Box::new(observer));
        assert!(previous.is_none(), "construction observer leaked");
    }

    #[cfg(test)]
    fn observe_construction_for_test(&self, bytes: usize) {
        if let Some(observer) = self.construction_observer.lock().unwrap().as_mut() {
            observer(bytes);
        }
    }

    #[cfg(not(test))]
    fn observe_construction_for_test(&self, _bytes: usize) {}
}

fn normalized_replacement(
    preimage: &[u8],
    observed: Option<&TextProfile>,
    content: &str,
) -> Result<Vec<u8>, SourceResourceError> {
    let observed = observed.ok_or_else(|| {
        public_error(
            SourceResourceErrorCode::ResourceNotReplaceable,
            "the snapshotted BSL resource is not UTF-8 text",
        )
    })?;
    let snapshot = SourceTextSnapshot::from_bytes(preimage).map_err(|_| {
        public_error(
            SourceResourceErrorCode::StaleRevision,
            "the snapshotted BSL preimage is not valid UTF-8",
        )
    })?;
    if matches!(snapshot.line_endings(), LineEndingProfile::Mixed { .. })
        || observed.eol == EolProfile::Mixed
    {
        return Err(public_error(
            SourceResourceErrorCode::ValidationFailed,
            "mixed EOL in the snapshotted BSL resource cannot be preserved safely",
        ));
    }
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let replacement_profile = text_profile(content.as_bytes()).ok_or_else(|| {
        public_error(
            SourceResourceErrorCode::ValidationFailed,
            "replacement content is not valid UTF-8",
        )
    })?;
    if replacement_profile.eol == EolProfile::Mixed {
        return Err(public_error(
            SourceResourceErrorCode::ValidationFailed,
            "replacement BSL content must use one uniform EOL style",
        ));
    }
    let target_eol = match observed.eol {
        EolProfile::Lf => Some(LineEnding::Lf),
        EolProfile::Crlf => Some(LineEnding::CrLf),
        EolProfile::Cr => Some(LineEnding::Cr),
        EolProfile::None => None,
        EolProfile::Mixed => unreachable!("mixed observed EOL was rejected"),
    };
    let payload = match target_eol {
        Some(eol) => canonicalize_eol(content).replace('\n', eol.as_str()),
        None => content.to_string(),
    };
    let mut bytes = Vec::with_capacity(observed.bom_prefix_bytes + payload.len());
    bytes.extend_from_slice(preimage.get(..observed.bom_prefix_bytes).ok_or_else(|| {
        public_error(
            SourceResourceErrorCode::StaleRevision,
            "snapshotted BOM profile no longer matches the preimage",
        )
    })?);
    bytes.extend_from_slice(payload.as_bytes());
    Ok(bytes)
}

fn canonicalize_eol(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn validate_bsl(bytes: &[u8]) -> Result<(), SourceResourceError> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        public_error(
            SourceResourceErrorCode::ValidationFailed,
            "replacement BSL module must remain UTF-8",
        )
    })?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let parsed = bsl_parser::parse(text);
    if parsed.errors().is_empty() {
        Ok(())
    } else {
        Err(public_error(
            SourceResourceErrorCode::ValidationFailed,
            "replacement BSL module did not pass the parser",
        ))
    }
}

fn changed_ranges(
    before: &[u8],
    after: &[u8],
) -> Result<Vec<SourceChangedRange>, SourceResourceError> {
    if before == after {
        return Ok(Vec::new());
    }
    let before_text = std::str::from_utf8(before).map_err(|_| {
        public_error(
            SourceResourceErrorCode::ValidationFailed,
            "snapshotted BSL module must be UTF-8",
        )
    })?;
    let after_text = std::str::from_utf8(after).map_err(|_| {
        public_error(
            SourceResourceErrorCode::ValidationFailed,
            "replacement BSL module must be UTF-8",
        )
    })?;
    let mut start = before
        .iter()
        .zip(after.iter())
        .take_while(|(left, right)| left == right)
        .count();
    while start > 0 && (!before_text.is_char_boundary(start) || !after_text.is_char_boundary(start))
    {
        start -= 1;
    }
    let mut suffix = before[start..]
        .iter()
        .rev()
        .zip(after[start..].iter().rev())
        .take_while(|(left, right)| left == right)
        .count();
    while suffix > 0
        && (!before_text.is_char_boundary(before.len() - suffix)
            || !after_text.is_char_boundary(after.len() - suffix))
    {
        suffix -= 1;
    }
    let end = after.len() - suffix;
    let (start_line, start_column) = line_column(after_text, start)?;
    let (end_line, end_column) = line_column(after_text, end)?;
    Ok(vec![SourceChangedRange {
        start_byte: start,
        end_byte: end,
        start_line,
        start_column,
        end_line,
        end_column,
    }])
}

fn line_column(text: &str, offset: usize) -> Result<(usize, usize), SourceResourceError> {
    let prefix = text.get(..offset).ok_or_else(|| {
        public_error(
            SourceResourceErrorCode::ValidationFailed,
            "replacement changed range is not on a UTF-8 boundary",
        )
    })?;
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix, |(_, current)| current)
        .chars()
        .count()
        + 1;
    Ok((line, column))
}

fn replacement_diff(
    metadata_path: &str,
    before: &[u8],
    after: &[u8],
) -> Result<String, SourceResourceError> {
    if before == after {
        return Ok(String::new());
    }
    let before = std::str::from_utf8(before).map_err(|_| {
        public_error(
            SourceResourceErrorCode::ValidationFailed,
            "snapshotted BSL module must be UTF-8",
        )
    })?;
    let after = std::str::from_utf8(after).map_err(|_| {
        public_error(
            SourceResourceErrorCode::ValidationFailed,
            "replacement BSL module must be UTF-8",
        )
    })?;
    let mut options = DiffOptions::new();
    options
        .set_original_filename(format!("a/{metadata_path}"))
        .set_modified_filename(format!("b/{metadata_path}"));
    let rendered = options.create_patch(before, after).to_string();
    let patch = Patch::from_str(&rendered).map_err(|_| {
        public_error(
            SourceResourceErrorCode::IntegrityFailed,
            "generated BSL replacement diff cannot be parsed",
        )
    })?;
    let rebuilt = apply_diff(before, &patch).map_err(|_| {
        public_error(
            SourceResourceErrorCode::IntegrityFailed,
            "generated BSL replacement diff cannot be applied",
        )
    })?;
    if rebuilt.as_bytes() != after.as_bytes() {
        return Err(public_error(
            SourceResourceErrorCode::IntegrityFailed,
            "generated BSL replacement diff does not reproduce the postimage",
        ));
    }
    Ok(rendered)
}

fn authorize_support(target: &Path, context: &WorkspaceContext) -> Result<(), SourceResourceError> {
    match evaluate_resolved_support_guard(target, SupportGuardRequirement::Editable, context) {
        ResolvedSupportGuardCheck::Allow | ResolvedSupportGuardCheck::Warn(_) => Ok(()),
        ResolvedSupportGuardCheck::Block(_) => Err(public_error(
            SourceResourceErrorCode::SupportDenied,
            "the current support state does not allow BSL replacement",
        )),
    }
}

fn map_reauthorization_error(
    error: crate::domain::source_target::SourceTargetError,
) -> SourceResourceError {
    let code = if error.code == SourceTargetErrorCode::ContainmentDenied {
        SourceResourceErrorCode::ContainmentDenied
    } else {
        SourceResourceErrorCode::StaleRevision
    };
    public_error(code, "logical source resource could not be reauthorized")
}

fn map_transaction_error(error: String) -> SourceResourceError {
    let lowered = error.to_ascii_lowercase();
    let code = if lowered.contains("symbolic link")
        || lowered.contains("reparse point")
        || lowered.contains("containment")
        || lowered.contains("outside")
    {
        SourceResourceErrorCode::ContainmentDenied
    } else if lowered.contains("format") || lowered.contains("version") {
        SourceResourceErrorCode::FormatDenied
    } else if lowered.contains("changed")
        || lowered.contains("stale")
        || lowered.contains("preimage")
        || lowered.contains("read guard")
        || lowered.contains("absence guard")
    {
        SourceResourceErrorCode::StaleRevision
    } else {
        SourceResourceErrorCode::IntegrityFailed
    };
    public_error(code, "source resource transaction could not be published")
}

fn logical_owner(metadata_path: &str) -> String {
    let mut segments = metadata_path.split('.').collect::<Vec<_>>();
    segments.pop();
    if segments.is_empty() {
        "Configuration".to_string()
    } else {
        segments.join(".")
    }
}

fn snapshot_resource(
    root: &Path,
    path: &Path,
    role: ResourceRole,
    replace_allowed: bool,
    maximum_bytes: usize,
) -> Result<StoredResource, SourceResourceError> {
    let bytes = read_root_relative_regular_file(root, path, maximum_bytes, |_| {})
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::FileTooLarge {
                capacity_error()
            } else {
                public_error(
                    SourceResourceErrorCode::SourceUnavailable,
                    "source resource cannot be safely snapshotted",
                )
            }
        })?
        .bytes;
    let text_profile = is_text_role(role).then(|| text_profile(&bytes)).flatten();
    let media_type = match role {
        ResourceRole::BslModule => "text/x-bsl",
        ResourceRole::ConfigurationDescriptor
        | ResourceRole::MetadataDescriptor
        | ResourceRole::Registration
        | ResourceRole::Form
        | ResourceRole::Dcs
        | ResourceRole::Rights => "application/xml",
        ResourceRole::Mxl | ResourceRole::BinaryTemplate | ResourceRole::Unknown => {
            "application/octet-stream"
        }
    };
    Ok(StoredResource {
        public: SourceResource {
            resource_id: Uuid::new_v4().to_string(),
            role,
            media_type: media_type.to_string(),
            size: bytes.len(),
            hash: sha256_hash(&bytes),
            text_profile,
            access: if replace_allowed {
                vec![ResourceAccess::Read, ResourceAccess::Replace]
            } else {
                vec![ResourceAccess::Read]
            },
            limits: ResourceLimits {
                max_read_bytes: SOURCE_READ_LIMIT_MAX,
            },
        },
        path: path.to_path_buf(),
        bytes: Arc::from(bytes),
    })
}

fn is_text_role(role: ResourceRole) -> bool {
    matches!(
        role,
        ResourceRole::BslModule
            | ResourceRole::ConfigurationDescriptor
            | ResourceRole::MetadataDescriptor
            | ResourceRole::Registration
            | ResourceRole::Form
            | ResourceRole::Dcs
            | ResourceRole::Rights
    )
}

fn capacity_error() -> SourceResourceError {
    public_error(
        SourceResourceErrorCode::SnapshotCapacityExceeded,
        "resource snapshot capacity is exhausted",
    )
}

fn within_live_capacity(count: usize, bytes: usize, new_bytes: usize) -> bool {
    count < MAX_LIVE_SNAPSHOTS && bytes.saturating_add(new_bytes) <= MAX_LIVE_SNAPSHOT_BYTES
}

fn scan_root_resources(
    root: &Path,
    cancellation: &CancellationToken,
) -> Result<Vec<(PathBuf, ResourceRole)>, SourceResourceError> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        if cancellation.is_cancelled() {
            return Err(public_error(
                SourceResourceErrorCode::Cancelled,
                "source resource operation was cancelled",
            ));
        }
        let mut entries = fs::read_dir(&directory)
            .map_err(|_| {
                public_error(
                    SourceResourceErrorCode::SourceUnavailable,
                    "source aggregate is unavailable",
                )
            })?
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries.into_iter().rev() {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|_| {
                public_error(
                    SourceResourceErrorCode::SourceUnavailable,
                    "source aggregate entry is unavailable",
                )
            })?;
            if metadata_is_link_or_reparse_point(&metadata) {
                continue;
            }
            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file() {
                files.push((path.clone(), classify_resource(root, &path)));
                if files.len() >= SOURCE_MANIFEST_RESOURCE_MAX {
                    break;
                }
            }
        }
        if files.len() >= SOURCE_MANIFEST_RESOURCE_MAX {
            break;
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

fn classify_resource(root: &Path, path: &Path) -> ResourceRole {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if relative == Path::new("Configuration.xml") {
        ResourceRole::ConfigurationDescriptor
    } else if extension.eq_ignore_ascii_case("bsl") {
        ResourceRole::BslModule
    } else if name.eq_ignore_ascii_case("Rights.xml") {
        ResourceRole::Rights
    } else if name.eq_ignore_ascii_case("Form.xml") {
        ResourceRole::Form
    } else if extension.eq_ignore_ascii_case("mxl") {
        ResourceRole::Mxl
    } else if extension.eq_ignore_ascii_case("bin") {
        ResourceRole::BinaryTemplate
    } else if extension.eq_ignore_ascii_case("xml") {
        ResourceRole::MetadataDescriptor
    } else {
        ResourceRole::Unknown
    }
}

fn text_profile(bytes: &[u8]) -> Option<TextProfile> {
    std::str::from_utf8(bytes).ok()?;
    let bom_prefix_bytes = usize::from(bytes.starts_with(&[0xef, 0xbb, 0xbf])) * 3;
    let content = &bytes[bom_prefix_bytes..];
    let mut crlf = 0;
    let mut lf = 0;
    let mut cr = 0;
    let mut index = 0;
    while index < content.len() {
        match content[index] {
            b'\r' if content.get(index + 1) == Some(&b'\n') => {
                crlf += 1;
                index += 2;
            }
            b'\r' => {
                cr += 1;
                index += 1;
            }
            b'\n' => {
                lf += 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
    let kinds = usize::from(crlf > 0) + usize::from(lf > 0) + usize::from(cr > 0);
    let eol = match (kinds, crlf > 0, lf > 0, cr > 0) {
        (0, _, _, _) => EolProfile::None,
        (1, true, _, _) => EolProfile::Crlf,
        (1, _, true, _) => EolProfile::Lf,
        (1, _, _, true) => EolProfile::Cr,
        _ => EolProfile::Mixed,
    };
    Some(TextProfile {
        encoding: TextEncoding::Utf8,
        bom_prefix_bytes,
        eol,
    })
}

fn manifest_revision(
    target: &ResolvedTarget,
    scope: ResourceScope,
    resources: &[StoredResource],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{target:?}:{scope:?}").as_bytes());
    for resource in resources {
        hasher.update(format!(
            "{:?}:{}:{}",
            resource.public.role, resource.public.size, resource.public.hash
        ));
    }
    hex_digest(hasher.finalize().as_slice())
}

fn sha256_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{}", hex_digest(digest.as_slice()))
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 0xf) as usize] as char);
    }
    result
}

fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        encoded.push(ALPHABET[(first >> 2) as usize] as char);
        encoded.push(ALPHABET[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            ALPHABET[(((second & 0x0f) << 2) | (third >> 6)) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            ALPHABET[(third & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    encoded
}

fn public_error(code: SourceResourceErrorCode, message: impl Into<String>) -> SourceResourceError {
    SourceResourceError::new(code, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::source_resources::{
        ContinueResourceSnapshotRequest, OpenResourceSnapshotRequest, SourceApplyRequest,
        SourceReadRequest, SourceResourcesRequest,
    };
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::events::DomainEventKind;
    use crate::domain::source_resources::{
        EolProfile, ResourceCompleteness, ResourceRole, ResourceScope, SourceResourceErrorCode,
        TextEncoding, SOURCE_READ_LIMIT_MAX,
    };
    use crate::domain::source_target::{
        MetadataAddress, SourceTarget, PLATFORM_XML_8_3_27_FORMAT_2_20,
    };
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::native_operations::single_file_publisher::with_before_commit_hook;
    use crate::infrastructure::platform::testing::{
        create_file_link_fixture_for_test, FileLinkFixtureOutcome,
    };
    use std::fs;
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    };
    use std::time::Duration;
    use uuid::{Uuid, Version};

    #[derive(Default)]
    struct ManualClock(AtomicU64);

    impl SourceResourceClock for ManualClock {
        fn now(&self) -> Duration {
            Duration::from_secs(self.0.load(Ordering::SeqCst))
        }
    }

    impl ManualClock {
        fn advance(&self, seconds: u64) {
            self.0.fetch_add(seconds, Ordering::SeqCst);
        }
    }

    struct Fixture {
        root: std::path::PathBuf,
        context: WorkspaceContext,
    }

    impl Fixture {
        fn new(module: &[u8]) -> Self {
            let root = std::env::temp_dir().join(format!(
                "unica-source-resources-{}-{}",
                std::process::id(),
                Uuid::new_v4()
            ));
            let source = root.join("src");
            fs::create_dir_all(source.join("CommonModules/Shared/Ext")).unwrap();
            fs::write(
                root.join("v8project.yaml"),
                "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
            )
            .unwrap();
            fs::write(
                root.join(".v8-project.json"),
                r#"{"editingAllowedCheck":"off"}"#,
            )
            .unwrap();
            fs::write(
                source.join("Configuration.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Main</Name></Properties><ChildObjects><CommonModule>Shared</CommonModule></ChildObjects></Configuration></MetaDataObject>"#,
            )
            .unwrap();
            fs::write(
                source.join("CommonModules/Shared.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule><Properties><Name>Shared</Name></Properties></CommonModule></MetaDataObject>"#,
            )
            .unwrap();
            fs::write(source.join("CommonModules/Shared/Ext/Module.bsl"), module).unwrap();
            fs::create_dir_all(source.join("Templates/Blob/Ext")).unwrap();
            fs::write(
                source.join("Templates/Blob/Ext/Template.bin"),
                [0xff, 0, 1, 2],
            )
            .unwrap();
            let context = WorkspaceContext {
                cwd: root.clone(),
                workspace_root: root.clone(),
                cache_root: root.join(".cache"),
                workspace_epoch: 7,
            };
            Self { root, context }
        }

        fn module_request(&self, scope: ResourceScope, limit: usize) -> SourceResourcesRequest {
            SourceResourcesRequest::Open(OpenResourceSnapshotRequest {
                target: SourceTarget {
                    source_set: "main".to_string(),
                    metadata_path: Some(
                        MetadataAddress::parse(
                            PLATFORM_XML_8_3_27_FORMAT_2_20,
                            "CommonModule.Shared.Module",
                        )
                        .unwrap(),
                    ),
                },
                scope,
                limit,
            })
        }

        fn root_request(&self, scope: ResourceScope, limit: usize) -> SourceResourcesRequest {
            SourceResourcesRequest::Open(OpenResourceSnapshotRequest {
                target: SourceTarget {
                    source_set: "main".to_string(),
                    metadata_path: None,
                },
                scope,
                limit,
            })
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn provider() -> (PlatformXmlResourceProvider, Arc<ManualClock>) {
        let clock = Arc::new(ManualClock::default());
        (
            PlatformXmlResourceProvider::with_clock(clock.clone()),
            clock,
        )
    }

    #[test]
    fn source_resources_module_self_snapshot_has_one_bsl_resource_and_opaque_random_ids() {
        let fixture = Fixture::new(b"Procedure Run()\nEndProcedure\n");
        let (provider, _) = provider();
        let page = provider
            .resources(
                fixture.module_request(ResourceScope::SelfOnly, 50),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();

        assert_eq!(page.completeness, ResourceCompleteness::Complete);
        assert_eq!(page.resources.len(), 1);
        assert_eq!(page.resources[0].role, ResourceRole::BslModule);
        assert_eq!(
            page.resources[0].access,
            [ResourceAccess::Read, ResourceAccess::Replace]
        );
        let snapshot = Uuid::parse_str(&page.snapshot_id).unwrap();
        let resource = Uuid::parse_str(&page.resources[0].resource_id).unwrap();
        assert_eq!(snapshot.get_version(), Some(Version::Random));
        assert_eq!(resource.get_version(), Some(Version::Random));
        for id in [&page.snapshot_id, &page.resources[0].resource_id] {
            assert!(!id.contains("main"));
            assert!(!id.contains("Shared"));
            assert!(!id.contains("platform"));
            assert!(!id.contains(fixture.root.to_string_lossy().as_ref()));
        }
    }

    #[test]
    fn source_resources_ids_are_valid_only_inside_the_snapshot_that_issued_them() {
        let fixture = Fixture::new(b"Procedure Run()\nEndProcedure\n");
        let (provider, _) = provider();
        let first = provider
            .resources(
                fixture.module_request(ResourceScope::SelfOnly, 50),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        let second = provider
            .resources(
                fixture.module_request(ResourceScope::SelfOnly, 50),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();

        let error = provider
            .read(
                SourceReadRequest {
                    snapshot_id: second.snapshot_id,
                    resource_id: first.resources[0].resource_id.clone(),
                    offset: 0,
                    limit: 10,
                },
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap_err();
        assert_eq!(error.code, SourceResourceErrorCode::ResourceNotFound);

        let error = provider
            .read(
                SourceReadRequest {
                    snapshot_id: first.snapshot_id,
                    resource_id: Uuid::new_v4().to_string(),
                    offset: 0,
                    limit: 10,
                },
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap_err();
        assert_eq!(error.code, SourceResourceErrorCode::ResourceNotFound);
    }

    #[test]
    fn source_resources_snapshots_expire_after_five_minutes_and_are_workspace_bound() {
        let fixture = Fixture::new(b"Procedure Run()\nEndProcedure\n");
        let (provider, clock) = provider();
        let page = provider
            .resources(
                fixture.module_request(ResourceScope::SelfOnly, 50),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        let read = SourceReadRequest {
            snapshot_id: page.snapshot_id.clone(),
            resource_id: page.resources[0].resource_id.clone(),
            offset: 0,
            limit: 10,
        };
        let mut other_workspace = fixture.context.clone();
        other_workspace.workspace_root = fixture.root.join("other");
        let mismatch = provider
            .read(read.clone(), &other_workspace, &CancellationToken::new())
            .unwrap_err();
        assert_eq!(
            mismatch.code,
            SourceResourceErrorCode::SnapshotScopeMismatch
        );
        assert!(!mismatch
            .message
            .contains(fixture.root.to_string_lossy().as_ref()));

        clock.advance(301);
        let expired = provider
            .read(read.clone(), &fixture.context, &CancellationToken::new())
            .unwrap_err();
        assert_eq!(expired.code, SourceResourceErrorCode::SnapshotExpired);
        let evicted = provider
            .read(read, &fixture.context, &CancellationToken::new())
            .unwrap_err();
        assert_eq!(evicted.code, SourceResourceErrorCode::SnapshotNotFound);
    }

    #[test]
    fn source_resources_pagination_uses_deterministic_request_bound_cursor() {
        let fixture = Fixture::new(b"Procedure Run()\nEndProcedure\n");
        let (provider, _) = provider();
        let first = provider
            .resources(
                fixture.root_request(ResourceScope::Aggregate, 2),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(first.resources.len(), 2);
        let cursor = first.next_cursor.clone().expect("more than two resources");
        let second = provider
            .resources(
                SourceResourcesRequest::Continue(ContinueResourceSnapshotRequest {
                    snapshot_id: first.snapshot_id.clone(),
                    cursor: cursor.clone(),
                    limit: None,
                }),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        let repeated = provider
            .resources(
                SourceResourcesRequest::Continue(ContinueResourceSnapshotRequest {
                    snapshot_id: first.snapshot_id.clone(),
                    cursor,
                    limit: None,
                }),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(second.resources, repeated.resources);
        assert_eq!(second.next_cursor, repeated.next_cursor);
        assert!(first
            .resources
            .iter()
            .all(|resource| !second.resources.contains(resource)));

        let wrong_limit = provider
            .resources(
                SourceResourcesRequest::Continue(ContinueResourceSnapshotRequest {
                    snapshot_id: first.snapshot_id,
                    cursor: first.next_cursor.unwrap(),
                    limit: Some(1),
                }),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap_err();
        assert_eq!(wrong_limit.code, SourceResourceErrorCode::InvalidCursor);
    }

    #[test]
    fn source_resources_aggregate_and_registration_manifests_are_read_only_and_partial() {
        let fixture = Fixture::new(b"Procedure Run()\nEndProcedure\n");
        let (provider, _) = provider();
        let aggregate = provider
            .resources(
                fixture.module_request(ResourceScope::Aggregate, 50),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(aggregate.completeness, ResourceCompleteness::Partial);
        assert!(aggregate
            .resources
            .iter()
            .any(|resource| resource.role == ResourceRole::MetadataDescriptor));
        assert!(aggregate
            .resources
            .iter()
            .all(|resource| resource.access
                == [crate::domain::source_resources::ResourceAccess::Read]));

        let registrations = provider
            .resources(
                fixture.module_request(ResourceScope::Registrations, 50),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(registrations.completeness, ResourceCompleteness::Partial);
        assert_eq!(registrations.resources[0].role, ResourceRole::Registration);
    }

    #[test]
    fn source_resources_text_reads_are_bounded_and_report_bom_and_crlf() {
        let fixture = Fixture::new(b"\xef\xbb\xbfProcedure Run()\r\nEndProcedure\r\n");
        let (provider, _) = provider();
        let page = provider
            .resources(
                fixture.module_request(ResourceScope::SelfOnly, 50),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        let resource = &page.resources[0];
        assert_eq!(
            resource.text_profile.as_ref().unwrap().encoding,
            TextEncoding::Utf8
        );
        assert_eq!(resource.text_profile.as_ref().unwrap().bom_prefix_bytes, 3);
        assert_eq!(
            resource.text_profile.as_ref().unwrap().eol,
            EolProfile::Crlf
        );

        let read = provider
            .read(
                SourceReadRequest {
                    snapshot_id: page.snapshot_id,
                    resource_id: resource.resource_id.clone(),
                    offset: 0,
                    limit: 8,
                },
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(read.offset, 0);
        assert!(read.length <= 8);
        assert_eq!(read.applied_limit, 8);
        assert_eq!(read.content_encoding, "utf-8");
        assert!(!read.eof);
        assert_eq!(read.hash, resource.hash);
        assert_eq!(read.text_profile.unwrap().bom_prefix_bytes, 3);
    }

    #[test]
    fn source_resources_binary_reads_are_bounded_base64_and_read_only() {
        let fixture = Fixture::new(b"Procedure Run()\nEndProcedure\n");
        fs::write(
            fixture.root.join("src/Templates/Blob/Ext/Template.bin"),
            b"plain ASCII bytes",
        )
        .unwrap();
        let (provider, _) = provider();
        let page = provider
            .resources(
                fixture.root_request(ResourceScope::Aggregate, 50),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        let resource = page
            .resources
            .iter()
            .find(|resource| resource.role == ResourceRole::BinaryTemplate)
            .unwrap();
        assert!(resource.text_profile.is_none());
        assert_eq!(
            resource.access,
            [crate::domain::source_resources::ResourceAccess::Read]
        );
        let read = provider
            .read(
                SourceReadRequest {
                    snapshot_id: page.snapshot_id,
                    resource_id: resource.resource_id.clone(),
                    offset: 0,
                    limit: SOURCE_READ_LIMIT_MAX,
                },
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(read.content_encoding, "base64");
        assert_eq!(read.content, "cGxhaW4gQVNDSUkgYnl0ZXM=");
        assert!(read.eof);
        assert_eq!(read.length, 17);
    }

    #[test]
    fn text_reads_preserve_exact_byte_progress_at_utf8_boundaries() {
        let fixture = Fixture::new("\u{feff}Ж".as_bytes());
        let (provider, _) = provider();
        let page = provider
            .resources(
                fixture.module_request(ResourceScope::SelfOnly, 50),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        let resource_id = page.resources[0].resource_id.clone();
        for (offset, limit, encoding, length, eof) in [
            (0, 1, "base64", 1, false),
            (0, 2, "base64", 2, false),
            (3, 1, "base64", 1, false),
            (4, 1, "base64", 1, true),
        ] {
            let read = provider
                .read(
                    SourceReadRequest {
                        snapshot_id: page.snapshot_id.clone(),
                        resource_id: resource_id.clone(),
                        offset,
                        limit,
                    },
                    &fixture.context,
                    &CancellationToken::new(),
                )
                .unwrap();
            assert_eq!(read.offset, offset);
            assert_eq!(read.length, length);
            assert_eq!(read.content_encoding, encoding);
            assert_eq!(read.eof, eof);
            assert!(read.length > 0 || read.eof);
        }
    }

    #[test]
    fn ttl_boundary_expires_pages_and_reads_at_exact_deadline() {
        let fixture = Fixture::new(b"Procedure Run()\nEndProcedure\n");
        let (provider, clock) = provider();
        let first = provider
            .resources(
                fixture.root_request(ResourceScope::Aggregate, 1),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        let continuation = ContinueResourceSnapshotRequest {
            snapshot_id: first.snapshot_id.clone(),
            cursor: first.next_cursor.clone().unwrap(),
            limit: None,
        };
        clock.advance(299);
        provider
            .resources(
                SourceResourcesRequest::Continue(continuation.clone()),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        clock.advance(1);
        assert_eq!(
            provider
                .resources(
                    SourceResourcesRequest::Continue(continuation),
                    &fixture.context,
                    &CancellationToken::new(),
                )
                .unwrap_err()
                .code,
            SourceResourceErrorCode::SnapshotExpired
        );

        let second = provider
            .resources(
                fixture.module_request(ResourceScope::SelfOnly, 50),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        let read = SourceReadRequest {
            snapshot_id: second.snapshot_id,
            resource_id: second.resources[0].resource_id.clone(),
            offset: 0,
            limit: 1,
        };
        clock.advance(299);
        provider
            .read(read.clone(), &fixture.context, &CancellationToken::new())
            .unwrap();
        clock.advance(1);
        assert_eq!(
            provider
                .read(read.clone(), &fixture.context, &CancellationToken::new())
                .unwrap_err()
                .code,
            SourceResourceErrorCode::SnapshotExpired
        );
        clock.advance(1);
        assert_eq!(
            provider
                .read(read, &fixture.context, &CancellationToken::new())
                .unwrap_err()
                .code,
            SourceResourceErrorCode::SnapshotNotFound
        );
    }

    #[test]
    fn live_snapshot_capacity_is_bounded_without_evicting_unexpired_snapshots() {
        assert!(within_live_capacity(63, MAX_LIVE_SNAPSHOT_BYTES - 1, 1));
        assert!(!within_live_capacity(64, 0, 0));
        assert!(!within_live_capacity(0, MAX_LIVE_SNAPSHOT_BYTES, 1));

        let fixture = Fixture::new(b"x");
        let (provider, _) = provider();
        let first = provider
            .resources(
                fixture.module_request(ResourceScope::SelfOnly, 50),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        for _ in 1..MAX_LIVE_SNAPSHOTS {
            provider
                .resources(
                    fixture.module_request(ResourceScope::SelfOnly, 50),
                    &fixture.context,
                    &CancellationToken::new(),
                )
                .unwrap();
        }
        assert_eq!(
            provider
                .resources(
                    fixture.module_request(ResourceScope::SelfOnly, 50),
                    &fixture.context,
                    &CancellationToken::new(),
                )
                .unwrap_err()
                .code,
            SourceResourceErrorCode::SnapshotCapacityExceeded
        );
        provider
            .read(
                SourceReadRequest {
                    snapshot_id: first.snapshot_id,
                    resource_id: first.resources[0].resource_id.clone(),
                    offset: 0,
                    limit: 1,
                },
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
    }

    #[test]
    fn single_snapshot_byte_limit_has_stable_capacity_error() {
        let fixture = Fixture::new(b"x");
        fs::write(
            fixture.root.join("src/CommonModules/Shared/Ext/Module.bsl"),
            vec![b'x'; MAX_SNAPSHOT_BYTES + 1],
        )
        .unwrap();
        let (provider, _) = provider();
        let error = provider
            .resources(
                fixture.module_request(ResourceScope::SelfOnly, 50),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap_err();
        assert_eq!(
            error.code,
            SourceResourceErrorCode::SnapshotCapacityExceeded
        );
        assert!(!error
            .message
            .contains(fixture.root.to_string_lossy().as_ref()));
    }

    #[test]
    fn aggregate_construction_never_buffers_beyond_snapshot_budget() {
        let fixture = Fixture::new(b"x");
        let source = fixture.root.join("src");
        fs::create_dir_all(source.join("Bulk")).unwrap();
        fs::write(source.join("Bulk/A.bin"), vec![b'a'; 20 * 1024 * 1024]).unwrap();
        fs::write(source.join("Bulk/B.bin"), vec![b'b'; 20 * 1024 * 1024]).unwrap();
        let (provider, _) = provider();
        let maximum_observed = Arc::new(AtomicU64::new(0));
        let observed = Arc::clone(&maximum_observed);
        provider.set_construction_observer_for_test(move |bytes| {
            observed.fetch_max(bytes as u64, Ordering::SeqCst);
        });

        let error = provider
            .resources(
                fixture.root_request(ResourceScope::Aggregate, 50),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap_err();

        assert_eq!(
            error.code,
            SourceResourceErrorCode::SnapshotCapacityExceeded
        );
        assert!(
            maximum_observed.load(Ordering::SeqCst) <= MAX_SNAPSHOT_BYTES as u64,
            "construction buffered more than the advertised snapshot budget"
        );
    }

    #[test]
    fn source_resources_cancellation_between_phases_returns_private_error() {
        let fixture = Fixture::new(b"Procedure Run()\nEndProcedure\n");
        let (provider, _) = provider();
        let cancellation = CancellationToken::new();
        let cancel_between_phases = cancellation.clone();
        provider.set_phase_hook_for_test(move || cancel_between_phases.cancel());

        let error = provider
            .resources(
                fixture.module_request(ResourceScope::SelfOnly, 50),
                &fixture.context,
                &cancellation,
            )
            .unwrap_err();
        assert_eq!(error.code, SourceResourceErrorCode::Cancelled);
        assert!(!error
            .message
            .contains(fixture.root.to_string_lossy().as_ref()));
    }

    fn apply_request(page: &ResourceManifestPage, content: &str) -> SourceApplyRequest {
        SourceApplyRequest {
            snapshot_id: page.snapshot_id.clone(),
            resource_id: page.resources[0].resource_id.clone(),
            expected_hash: page.resources[0].hash.clone(),
            content: content.to_string(),
        }
    }

    #[test]
    fn source_apply_preview_and_apply_publish_identical_bom_crlf_bytes_and_one_event() {
        let fixture = Fixture::new(b"\xef\xbb\xbfProcedure Run()\r\nEndProcedure\r\n");
        let module = fixture.root.join("src/CommonModules/Shared/Ext/Module.bsl");
        let descriptor = fixture.root.join("src/CommonModules/Shared.xml");
        let registration = fixture.root.join("src/Configuration.xml");
        let descriptor_before = fs::read(&descriptor).unwrap();
        let registration_before = fs::read(&registration).unwrap();
        let (provider, _) = provider();
        let page = provider
            .resources(
                fixture.module_request(ResourceScope::SelfOnly, 50),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(
            page.resources[0].access,
            [ResourceAccess::Read, ResourceAccess::Replace]
        );
        let request = apply_request(
            &page,
            "Procedure Changed()\n    Message(\"ok\");\nEndProcedure\n",
        );

        let preview = provider
            .apply(
                request.clone(),
                &fixture.context,
                true,
                &CancellationToken::new(),
            )
            .unwrap();
        assert!(preview.event.is_none());
        assert!(!preview.result.no_op);
        assert!(!preview.result.changed_ranges.is_empty());
        assert!(preview.result.diff.contains("CommonModule.Shared.Module"));
        assert_eq!(
            fs::read(&module).unwrap(),
            b"\xef\xbb\xbfProcedure Run()\r\nEndProcedure\r\n"
        );

        let applied = provider
            .apply(request, &fixture.context, false, &CancellationToken::new())
            .unwrap();
        assert_eq!(applied.result.post_hash, preview.result.post_hash);
        assert_eq!(applied.result.diff, preview.result.diff);
        assert_eq!(
            fs::read(&module).unwrap(),
            b"\xef\xbb\xbfProcedure Changed()\r\n    Message(\"ok\");\r\nEndProcedure\r\n"
        );
        let event = applied.event.expect("successful apply emits one event");
        assert_eq!(event.kind, DomainEventKind::SourceResourcesReplaced);
        assert_eq!(event.details.as_ref().unwrap().source_set, "main");
        assert_eq!(
            event.details.as_ref().unwrap().affected_targets,
            ["CommonModule.Shared.Module"]
        );
        assert_eq!(fs::read(&descriptor).unwrap(), descriptor_before);
        assert_eq!(fs::read(&registration).unwrap(), registration_before);
    }

    #[test]
    fn source_apply_noop_emits_no_event_and_writes_nothing() {
        let before = b"\xef\xbb\xbfProcedure Run()\r\nEndProcedure\r\n";
        let fixture = Fixture::new(before);
        let module = fixture.root.join("src/CommonModules/Shared/Ext/Module.bsl");
        let (provider, _) = provider();
        let page = provider
            .resources(
                fixture.module_request(ResourceScope::SelfOnly, 50),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();

        let outcome = provider
            .apply(
                apply_request(&page, "Procedure Run()\nEndProcedure\n"),
                &fixture.context,
                false,
                &CancellationToken::new(),
            )
            .unwrap();

        assert!(outcome.result.no_op);
        assert!(outcome.result.changed_ranges.is_empty());
        assert!(outcome.result.diff.is_empty());
        assert!(outcome.event.is_none());
        assert_eq!(fs::read(module).unwrap(), before);
    }

    #[test]
    fn source_apply_rejects_hash_mismatch_and_stale_snapshot_preimage() {
        let fixture = Fixture::new(b"Procedure Run()\nEndProcedure\n");
        let module = fixture.root.join("src/CommonModules/Shared/Ext/Module.bsl");
        let (provider, _) = provider();
        let page = provider
            .resources(
                fixture.module_request(ResourceScope::SelfOnly, 50),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        let mut wrong_hash = apply_request(&page, "Procedure Changed()\nEndProcedure\n");
        wrong_hash.expected_hash = "sha256:wrong".to_string();
        assert_eq!(
            provider
                .apply(
                    wrong_hash,
                    &fixture.context,
                    false,
                    &CancellationToken::new(),
                )
                .unwrap_err()
                .code,
            SourceResourceErrorCode::HashMismatch
        );

        fs::write(&module, b"Procedure Concurrent()\nEndProcedure\n").unwrap();
        assert_eq!(
            provider
                .apply(
                    apply_request(&page, "Procedure Changed()\nEndProcedure\n"),
                    &fixture.context,
                    false,
                    &CancellationToken::new(),
                )
                .unwrap_err()
                .code,
            SourceResourceErrorCode::StaleRevision
        );
        assert_eq!(
            fs::read(&module).unwrap(),
            b"Procedure Concurrent()\nEndProcedure\n"
        );
    }

    #[test]
    fn source_apply_rejects_partial_snapshot_and_non_bsl_role() {
        let fixture = Fixture::new(b"Procedure Run()\nEndProcedure\n");
        let (provider, _) = provider();
        let partial = provider
            .resources(
                fixture.module_request(ResourceScope::Aggregate, 50),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(
            provider
                .apply(
                    apply_request(&partial, "Procedure Changed()\nEndProcedure\n"),
                    &fixture.context,
                    true,
                    &CancellationToken::new(),
                )
                .unwrap_err()
                .code,
            SourceResourceErrorCode::SnapshotIncomplete
        );

        let descriptor = provider
            .resources(
                fixture.root_request(ResourceScope::SelfOnly, 50),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(
            provider
                .apply(
                    apply_request(&descriptor, "<Configuration/>"),
                    &fixture.context,
                    true,
                    &CancellationToken::new(),
                )
                .unwrap_err()
                .code,
            SourceResourceErrorCode::ResourceNotReplaceable
        );
    }

    #[test]
    fn source_apply_rejects_mixed_eol_and_bsl_parse_failure() {
        let fixture = Fixture::new(b"Procedure Run()\r\nEndProcedure\r\n");
        let (provider, _) = provider();
        let page = provider
            .resources(
                fixture.module_request(ResourceScope::SelfOnly, 50),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        for content in [
            "Procedure Changed()\r\n    Message(\"x\");\nEndProcedure\r\n",
            "Procedure Broken(\n",
        ] {
            let error = provider
                .apply(
                    apply_request(&page, content),
                    &fixture.context,
                    true,
                    &CancellationToken::new(),
                )
                .unwrap_err();
            assert_eq!(error.code, SourceResourceErrorCode::ValidationFailed);
        }
    }

    #[test]
    fn source_apply_reauthorizes_support_format_owner_source_map_and_containment() {
        let replacement = "Procedure Changed()\nEndProcedure\n";

        let support_fixture = Fixture::new(b"Procedure Run()\nEndProcedure\n");
        let (support_provider, _) = provider();
        let support_page = support_provider
            .resources(
                support_fixture.module_request(ResourceScope::SelfOnly, 50),
                &support_fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        fs::create_dir_all(support_fixture.root.join("src/Ext")).unwrap();
        fs::write(
            support_fixture
                .root
                .join("src/Ext/ParentConfigurations.bin"),
            concat!(
                "\u{feff}{6,1,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,",
                "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",",
                "\"VendorConf\",0}"
            ),
        )
        .unwrap();
        fs::write(
            support_fixture.root.join(".v8-project.json"),
            r#"{"editingAllowedCheck":"deny"}"#,
        )
        .unwrap();
        assert_eq!(
            support_provider
                .apply(
                    apply_request(&support_page, replacement),
                    &support_fixture.context,
                    true,
                    &CancellationToken::new(),
                )
                .unwrap_err()
                .code,
            SourceResourceErrorCode::SupportDenied
        );

        let format_fixture = Fixture::new(b"Procedure Run()\nEndProcedure\n");
        let (format_provider, _) = provider();
        let format_page = format_provider
            .resources(
                format_fixture.module_request(ResourceScope::SelfOnly, 50),
                &format_fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        let configuration = format_fixture.root.join("src/Configuration.xml");
        let configuration_text = fs::read_to_string(&configuration).unwrap();
        fs::write(&configuration, configuration_text.replace("2.20", "2.21")).unwrap();
        assert_eq!(
            format_provider
                .apply(
                    apply_request(&format_page, replacement),
                    &format_fixture.context,
                    true,
                    &CancellationToken::new(),
                )
                .unwrap_err()
                .code,
            SourceResourceErrorCode::FormatDenied
        );

        let source_map_fixture = Fixture::new(b"Procedure Run()\nEndProcedure\n");
        let (source_map_provider, _) = provider();
        let source_map_page = source_map_provider
            .resources(
                source_map_fixture.module_request(ResourceScope::SelfOnly, 50),
                &source_map_fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        fs::write(
            source_map_fixture.root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: EXTENSION\n    path: src\n",
        )
        .unwrap();
        assert_eq!(
            source_map_provider
                .apply(
                    apply_request(&source_map_page, replacement),
                    &source_map_fixture.context,
                    true,
                    &CancellationToken::new(),
                )
                .unwrap_err()
                .code,
            SourceResourceErrorCode::ContainmentDenied
        );

        let link_fixture = Fixture::new(b"Procedure Run()\nEndProcedure\n");
        let (link_provider, _) = provider();
        let link_page = link_provider
            .resources(
                link_fixture.module_request(ResourceScope::SelfOnly, 50),
                &link_fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        let module = link_fixture
            .root
            .join("src/CommonModules/Shared/Ext/Module.bsl");
        let external = link_fixture.root.join("replacement.bsl");
        fs::write(&external, b"Procedure External()\nEndProcedure\n").unwrap();
        fs::remove_file(&module).unwrap();
        if create_file_link_fixture_for_test(&external, &module).unwrap()
            == FileLinkFixtureOutcome::Created
        {
            assert_eq!(
                link_provider
                    .apply(
                        apply_request(&link_page, replacement),
                        &link_fixture.context,
                        true,
                        &CancellationToken::new(),
                    )
                    .unwrap_err()
                    .code,
                SourceResourceErrorCode::ContainmentDenied
            );
        }
    }

    #[test]
    fn source_apply_rolls_back_owner_race_and_cancel_before_publication() {
        let fixture = Fixture::new(b"Procedure Run()\nEndProcedure\n");
        let module = fixture.root.join("src/CommonModules/Shared/Ext/Module.bsl");
        let descriptor = fixture.root.join("src/CommonModules/Shared.xml");
        let before = fs::read(&module).unwrap();
        let (race_provider, _) = provider();
        let page = race_provider
            .resources(
                fixture.module_request(ResourceScope::SelfOnly, 50),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        let descriptor_for_hook = descriptor.clone();
        let error = with_before_commit_hook(
            move |_| {
                fs::write(
                    &descriptor_for_hook,
                    "<MetaDataObject concurrent=\"true\"/>",
                )
                .unwrap()
            },
            || {
                race_provider.apply(
                    apply_request(&page, "Procedure Changed()\nEndProcedure\n"),
                    &fixture.context,
                    false,
                    &CancellationToken::new(),
                )
            },
        )
        .unwrap_err();
        assert_eq!(error.code, SourceResourceErrorCode::StaleRevision);
        assert_eq!(fs::read(&module).unwrap(), before);

        let cancelled_fixture = Fixture::new(b"Procedure Run()\nEndProcedure\n");
        let (cancelled_provider, _) = provider();
        let cancelled_page = cancelled_provider
            .resources(
                cancelled_fixture.module_request(ResourceScope::SelfOnly, 50),
                &cancelled_fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        let cancellation = CancellationToken::new();
        let cancel_at_commit = cancellation.clone();
        cancelled_provider.set_phase_hook_for_test(move || cancel_at_commit.cancel());
        let error = cancelled_provider
            .apply(
                apply_request(&cancelled_page, "Procedure Changed()\nEndProcedure\n"),
                &cancelled_fixture.context,
                false,
                &cancellation,
            )
            .unwrap_err();
        assert_eq!(error.code, SourceResourceErrorCode::Cancelled);
        assert_eq!(
            fs::read(
                cancelled_fixture
                    .root
                    .join("src/CommonModules/Shared/Ext/Module.bsl")
            )
            .unwrap(),
            b"Procedure Run()\nEndProcedure\n"
        );
    }

    #[test]
    fn source_apply_post_publication_validation_failure_rolls_back_before_returning_error() {
        let fixture = Fixture::new(b"Procedure Run()\nEndProcedure\n");
        let module = fixture.root.join("src/CommonModules/Shared/Ext/Module.bsl");
        let before = fs::read(&module).unwrap();
        let replacement = b"Procedure Changed()\nEndProcedure\n".to_vec();
        let (provider, _) = provider();
        let page = provider
            .resources(
                fixture.module_request(ResourceScope::SelfOnly, 50),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        let module_during_validation = module.clone();
        let replacement_during_validation = replacement.clone();
        provider.set_post_validation_hook_for_test(move || {
            assert_eq!(
                fs::read(&module_during_validation).unwrap(),
                replacement_during_validation,
                "post-validation must run after publication"
            );
            Err("injected source.apply post-publication validation failure".to_string())
        });

        let error = provider
            .apply(
                apply_request(&page, std::str::from_utf8(&replacement).unwrap()),
                &fixture.context,
                false,
                &CancellationToken::new(),
            )
            .unwrap_err();

        assert_eq!(error.code, SourceResourceErrorCode::IntegrityFailed);
        assert_eq!(
            fs::read(&module).unwrap(),
            before,
            "a reported post-validation failure must restore the snapshot preimage"
        );
    }
}
