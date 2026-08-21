use crate::application::source_resources::{
    ContinueResourceSnapshotRequest, OpenResourceSnapshotRequest, SourceReadRequest,
    SourceResourcesRequest,
};
use crate::domain::cancellation::CancellationToken;
use crate::domain::source_resources::{
    EolProfile, ResourceAccess, ResourceCompleteness, ResourceLimits, ResourceManifestPage,
    ResourceRole, ResourceScope, SourceReadResult, SourceResource, SourceResourceError,
    SourceResourceErrorCode, TextEncoding, TextProfile, SOURCE_MANIFEST_RESOURCE_MAX,
    SOURCE_READ_LIMIT_MAX, SOURCE_RESOURCE_PAGE_LIMIT_MAX, SOURCE_SNAPSHOT_TTL_SECONDS,
};
use crate::domain::source_target::{
    ResolvedTarget, SourceTargetError, SourceTargetErrorCode, TargetKind,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point;
use crate::infrastructure::platform::secure_read::read_root_relative_regular_file;
use crate::infrastructure::platform_xml_source_targets::{
    platform_xml_resource_evidence, resolve_platform_xml_target, TargetKindPolicy,
};
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
    bytes: Arc<[u8]>,
}

struct StoredSnapshot {
    snapshot_id: String,
    provider_id: &'static str,
    workspace_root: PathBuf,
    workspace_epoch: u64,
    source_set: String,
    target: ResolvedTarget,
    scope: ResourceScope,
    completeness: ResourceCompleteness,
    manifest_revision: String,
    expires_at: Duration,
    page_size: usize,
    resources: Vec<StoredResource>,
    byte_size: usize,
}

#[derive(Default)]
struct SnapshotStore {
    snapshots: HashMap<String, StoredSnapshot>,
    reserved_bytes: usize,
    reserved_count: usize,
}

struct SnapshotCapacityReservation<'a> {
    store: &'a Mutex<SnapshotStore>,
    byte_size: usize,
    active: bool,
}

impl SnapshotCapacityReservation<'_> {
    fn publish(mut self, snapshot: StoredSnapshot) -> Result<(), SourceResourceError> {
        if snapshot.byte_size > self.byte_size {
            return Err(capacity_error());
        }
        let mut store = self.store.lock().map_err(|_| {
            public_error(
                SourceResourceErrorCode::SourceUnavailable,
                "resource snapshot store is unavailable",
            )
        })?;
        store.reserved_bytes -= self.byte_size;
        store.reserved_count -= 1;
        store
            .snapshots
            .insert(snapshot.snapshot_id.clone(), snapshot);
        self.active = false;
        Ok(())
    }
}

impl Drop for SnapshotCapacityReservation<'_> {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        if let Ok(mut store) = self.store.lock() {
            store.reserved_bytes -= self.byte_size;
            store.reserved_count -= 1;
        }
    }
}

pub(crate) struct PlatformXmlResourceProvider {
    instance_secret: String,
    clock: Arc<dyn SourceResourceClock>,
    snapshots: Mutex<SnapshotStore>,
    #[cfg(test)]
    phase_hook: Mutex<Option<Box<dyn FnOnce() + Send>>>,
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
            snapshots: Mutex::new(SnapshotStore::default()),
            #[cfg(test)]
            phase_hook: Mutex::new(None),
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
                .snapshots
                .get(&request.snapshot_id)
                .is_some_and(|snapshot| self.clock.now() >= snapshot.expires_at)
            {
                snapshots.snapshots.remove(&request.snapshot_id);
                return Err(public_error(
                    SourceResourceErrorCode::SnapshotExpired,
                    "resource snapshot has expired",
                ));
            }
            let snapshot = snapshots
                .snapshots
                .get(&request.snapshot_id)
                .ok_or_else(|| {
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
        let requested_end = text_chunk_end(
            &bytes,
            request.offset,
            request
                .offset
                .saturating_add(request.limit)
                .min(bytes.len()),
            public.text_profile.is_some(),
        );
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
        let resolution =
            resolve_platform_xml_target(context, &request.target, TargetKindPolicy::Any)
                .map_err(public_target_error)?;
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
            (TargetKind::MetadataObject, ResourceScope::SelfOnly) => (
                vec![(evidence.target_path, ResourceRole::MetadataDescriptor)],
                ResourceCompleteness::Complete,
            ),
            (TargetKind::MetadataObject, ResourceScope::Aggregate) => {
                let mut candidates = vec![(evidence.target_path, ResourceRole::MetadataDescriptor)];
                candidates.extend(
                    evidence
                        .module_paths
                        .into_iter()
                        .map(|path| (path, ResourceRole::BslModule)),
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
        };
        self.check_cancelled(cancellation)?;

        let reservation = self.reserve_snapshot_capacity()?;
        let budget = reservation.byte_size;
        let source_root = evidence.source_root.clone();
        let mut stored_resources = Vec::new();
        let mut byte_size = 0_usize;
        for (path, role) in candidates.into_iter().take(SOURCE_MANIFEST_RESOURCE_MAX) {
            self.check_cancelled(cancellation)?;
            let remaining = budget - byte_size;
            let resource = snapshot_resource(&source_root, &path, role, remaining)?;
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
        reservation.publish(snapshot)?;
        Ok(page)
    }

    /// Atomically reserves live count and byte capacity before construction.
    /// Publishing replaces the reservation with the actual snapshot size;
    /// dropping the guard releases it on every other exit path.
    fn reserve_snapshot_capacity(
        &self,
    ) -> Result<SnapshotCapacityReservation<'_>, SourceResourceError> {
        let mut store = self.snapshots.lock().map_err(|_| {
            public_error(
                SourceResourceErrorCode::SourceUnavailable,
                "resource snapshot store is unavailable",
            )
        })?;
        let now = self.clock.now();
        store.snapshots.retain(|_, stored| now < stored.expires_at);
        let live_bytes = store
            .snapshots
            .values()
            .map(|stored| stored.byte_size)
            .sum::<usize>();
        let occupied_bytes = live_bytes.saturating_add(store.reserved_bytes);
        let occupied_count = store.snapshots.len().saturating_add(store.reserved_count);
        if occupied_count >= MAX_LIVE_SNAPSHOTS || occupied_bytes >= MAX_LIVE_SNAPSHOT_BYTES {
            return Err(capacity_error());
        }
        let byte_size = MAX_SNAPSHOT_BYTES.min(MAX_LIVE_SNAPSHOT_BYTES - occupied_bytes);
        store.reserved_bytes += byte_size;
        store.reserved_count += 1;
        Ok(SnapshotCapacityReservation {
            store: &self.snapshots,
            byte_size,
            active: true,
        })
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
            .snapshots
            .get(&request.snapshot_id)
            .is_some_and(|snapshot| self.clock.now() >= snapshot.expires_at)
        {
            snapshots.snapshots.remove(&request.snapshot_id);
            return Err(public_error(
                SourceResourceErrorCode::SnapshotExpired,
                "resource snapshot has expired",
            ));
        }
        let snapshot = snapshots
            .snapshots
            .get(&request.snapshot_id)
            .ok_or_else(|| {
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

/// Shrinks a chunk of a declared-text resource to the nearest UTF-8 boundary.
/// Without it a byte limit landing inside a multi-byte character flips the
/// whole chunk to base64, so reading one Cyrillic BSL module in `maxReadBytes`
/// steps would alternate encodings for reasons the caller cannot predict.
/// The chunk is left as requested when shrinking would return no bytes at all,
/// so a limit smaller than one character still makes progress through the
/// base64 branch instead of looping forever on an empty read.
fn text_chunk_end(bytes: &[u8], offset: usize, end: usize, is_text: bool) -> usize {
    if !is_text || end >= bytes.len() {
        return end;
    }
    let mut boundary = end;
    while boundary > offset && !is_utf8_char_boundary(bytes[boundary]) {
        boundary -= 1;
    }
    if boundary == offset {
        end
    } else {
        boundary
    }
}

fn is_utf8_char_boundary(byte: u8) -> bool {
    (byte & 0xc0) != 0x80
}

fn snapshot_resource(
    root: &Path,
    path: &Path,
    role: ResourceRole,
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
            // The surface is read-only: BSL mutation belongs to
            // `unica.code.patch`, which edits a selected span in place.
            access: vec![ResourceAccess::Read],
            limits: ResourceLimits {
                max_read_bytes: SOURCE_READ_LIMIT_MAX,
            },
        },
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

/// Carries the resolver's own verdict outward. Collapsing every refusal into
/// `source_unavailable` reads as a transient outage when the real answer is
/// "this address is unknown" or "this scope is not supported for that target".
/// The resolver's message is logical by construction and names no path.
fn public_target_error(error: SourceTargetError) -> SourceResourceError {
    let code = match error.code {
        SourceTargetErrorCode::SourceSetRequired
        | SourceTargetErrorCode::MetadataAddressInvalid
        | SourceTargetErrorCode::AddressProfileUnsupported
        // This surface accepts every address kind the profile renders, so a
        // kind mismatch cannot reach it; the arm stays for totality.
        | SourceTargetErrorCode::TargetKindMismatch => SourceResourceErrorCode::InvalidRequest,
        SourceTargetErrorCode::SourceSetNotFound
        | SourceTargetErrorCode::SourceRootNotAddressable
        | SourceTargetErrorCode::MetadataAddressNotFound => SourceResourceErrorCode::TargetNotFound,
        SourceTargetErrorCode::ContainmentDenied => SourceResourceErrorCode::ContainmentDenied,
    };
    public_error(code, error.message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::source_resources::{
        ContinueResourceSnapshotRequest, OpenResourceSnapshotRequest, SourceReadRequest,
        SourceResourcesRequest,
    };
    use crate::domain::cancellation::CancellationToken;

    use crate::domain::source_resources::{
        EolProfile, ResourceCompleteness, ResourceRole, ResourceScope, SourceResourceErrorCode,
        TextEncoding, SOURCE_READ_LIMIT_MAX,
    };
    use crate::domain::source_target::{
        MetadataAddress, SourceTarget, PLATFORM_XML_8_3_27_FORMAT_2_20,
    };
    use crate::domain::workspace::WorkspaceContext;

    use std::fs;
    use std::sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Barrier,
    };
    use std::thread;
    use std::time::{Duration, Instant};
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
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Main</Name></Properties><ChildObjects><CommonModule>Shared</CommonModule><Catalog>Items</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
            )
            .unwrap();
            fs::write(
                source.join("CommonModules/Shared.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule><Properties><Name>Shared</Name></Properties></CommonModule></MetaDataObject>"#,
            )
            .unwrap();
            fs::write(source.join("CommonModules/Shared/Ext/Module.bsl"), module).unwrap();
            fs::create_dir_all(source.join("Catalogs/Items/Ext")).unwrap();
            fs::write(
                source.join("Catalogs/Items.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Items</Name></Properties></Catalog></MetaDataObject>"#,
            )
            .unwrap();
            fs::write(
                source.join("Catalogs/Items/Ext/ObjectModule.bsl"),
                b"Procedure BeforeWrite()\nEndProcedure\n",
            )
            .unwrap();
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

        fn object_request(
            &self,
            address: &str,
            scope: ResourceScope,
            limit: usize,
        ) -> SourceResourcesRequest {
            SourceResourcesRequest::Open(OpenResourceSnapshotRequest {
                target: SourceTarget {
                    source_set: "main".to_string(),
                    metadata_path: Some(
                        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, address).unwrap(),
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
        assert_eq!(page.resources[0].access, [ResourceAccess::Read]);
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
    fn source_resources_object_self_snapshot_returns_the_descriptor() {
        let fixture = Fixture::new(b"Procedure Run()\nEndProcedure\n");
        let (provider, _) = provider();

        let page = provider
            .resources(
                fixture.object_request("Catalog.Items", ResourceScope::SelfOnly, 50),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();

        assert_eq!(page.completeness, ResourceCompleteness::Complete);
        assert_eq!(page.resources.len(), 1);
        assert_eq!(page.resources[0].role, ResourceRole::MetadataDescriptor);
        assert_eq!(page.resources[0].access, [ResourceAccess::Read]);
        assert_eq!(page.target.target_kind, TargetKind::MetadataObject);
        assert_eq!(
            page.target.metadata_path.as_ref().map(|path| path.as_str()),
            Some("Catalog.Items")
        );
    }

    #[test]
    fn source_resources_object_aggregate_adds_only_proven_modules() {
        let fixture = Fixture::new(b"Procedure Run()\nEndProcedure\n");
        let (provider, _) = provider();

        let page = provider
            .resources(
                fixture.object_request("Catalog.Items", ResourceScope::Aggregate, 50),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();

        assert_eq!(page.completeness, ResourceCompleteness::Partial);
        assert_eq!(
            page.resources
                .iter()
                .map(|resource| resource.role)
                .collect::<Vec<_>>(),
            vec![ResourceRole::MetadataDescriptor, ResourceRole::BslModule]
        );
        assert!(page
            .resources
            .iter()
            .all(|resource| resource.access == [ResourceAccess::Read]));
    }

    #[test]
    fn source_resources_object_registrations_name_the_configuration() {
        let fixture = Fixture::new(b"Procedure Run()\nEndProcedure\n");
        let (provider, _) = provider();

        let page = provider
            .resources(
                fixture.object_request("Catalog.Items", ResourceScope::Registrations, 50),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();

        assert_eq!(page.resources.len(), 1);
        assert_eq!(page.resources[0].role, ResourceRole::Registration);
    }

    /// An address nobody can prove is not an outage. Collapsing it into
    /// `source_unavailable` told callers to retry something that will never
    /// succeed.
    #[test]
    fn source_resources_report_an_unknown_address_as_a_missing_target() {
        let fixture = Fixture::new(b"Procedure Run()\nEndProcedure\n");
        let (provider, _) = provider();

        let error = provider
            .resources(
                fixture.object_request("Catalog.Missing", ResourceScope::SelfOnly, 50),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap_err();

        assert_eq!(error.code, SourceResourceErrorCode::TargetNotFound);
        assert!(!error.message.contains('/'), "{}", error.message);
        assert!(!error.message.contains('\\'), "{}", error.message);
    }

    #[test]
    fn source_resources_report_an_unknown_source_set_as_a_missing_target() {
        let fixture = Fixture::new(b"Procedure Run()\nEndProcedure\n");
        let (provider, _) = provider();

        let error = provider
            .resources(
                SourceResourcesRequest::Open(OpenResourceSnapshotRequest {
                    target: SourceTarget {
                        source_set: "absent".to_string(),
                        metadata_path: None,
                    },
                    scope: ResourceScope::SelfOnly,
                    limit: 50,
                }),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap_err();

        assert_eq!(error.code, SourceResourceErrorCode::TargetNotFound);
    }

    #[test]
    fn source_resources_keep_unsupported_source_format_as_a_missing_target() {
        let fixture = Fixture::new(b"Procedure Run()\nEndProcedure\n");
        fs::create_dir_all(fixture.root.join("external")).unwrap();
        fs::write(
            fixture.root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: external\n",
        )
        .unwrap();
        let (provider, _) = provider();

        let error = provider
            .resources(
                SourceResourcesRequest::Open(OpenResourceSnapshotRequest {
                    target: SourceTarget {
                        source_set: "external".to_string(),
                        metadata_path: None,
                    },
                    scope: ResourceScope::SelfOnly,
                    limit: 50,
                }),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap_err();

        assert_eq!(error.code, SourceResourceErrorCode::TargetNotFound);
    }

    #[test]
    fn source_resources_report_a_missing_source_set_name_as_an_invalid_request() {
        let fixture = Fixture::new(b"Procedure Run()\nEndProcedure\n");
        let (provider, _) = provider();

        let error = provider
            .resources(
                SourceResourcesRequest::Open(OpenResourceSnapshotRequest {
                    target: SourceTarget {
                        source_set: String::new(),
                        metadata_path: None,
                    },
                    scope: ResourceScope::SelfOnly,
                    limit: 50,
                }),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap_err();

        assert_eq!(error.code, SourceResourceErrorCode::InvalidRequest);
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
    fn source_read_keeps_text_chunks_on_utf8_boundaries_and_still_advances() {
        let module = "Процедура Выполнить()\nКонецПроцедуры\n".as_bytes();
        let fixture = Fixture::new(module);
        let (provider, _) = provider();
        let page = provider
            .resources(
                fixture.module_request(ResourceScope::SelfOnly, 50),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        let resource_id = page.resources[0].resource_id.clone();
        let read_chunk = |offset: usize, limit: usize| {
            provider
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
                .unwrap()
        };

        // `П` is two bytes, so a three-byte limit lands inside `р`.
        let clipped = read_chunk(0, 3);
        assert_eq!(clipped.content_encoding, "utf-8");
        assert_eq!(clipped.content, "П");
        assert_eq!(clipped.length, 2);
        assert!(!clipped.eof);

        // A limit narrower than one character still returns exact bytes rather
        // than an empty chunk the caller could never move past.
        let unsplittable = read_chunk(0, 1);
        assert_eq!(unsplittable.content_encoding, "base64");
        assert_eq!(unsplittable.length, 1);

        // Chunked reading reassembles the module byte for byte.
        let mut assembled = Vec::new();
        let mut offset = 0;
        while offset < module.len() {
            let chunk = read_chunk(offset, 3);
            assert!(chunk.length > 0, "read must always advance");
            assert_eq!(chunk.content_encoding, "utf-8");
            assembled.extend_from_slice(chunk.content.as_bytes());
            offset += chunk.length;
        }
        assert_eq!(assembled, module);
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
    fn source_resources_reservations_are_atomic_across_threads_and_released_on_drop() {
        const WORKERS: usize = 8;
        const MAX_CONCURRENT_RESERVATIONS: usize = 4;

        let (provider, _) = provider();
        let provider = Arc::new(provider);
        let start = Arc::new(Barrier::new(WORKERS + 1));
        let release = Arc::new(AtomicBool::new(false));
        let (outcomes, received) = mpsc::channel();
        let workers = (0..WORKERS)
            .map(|_| {
                let provider = Arc::clone(&provider);
                let start = Arc::clone(&start);
                let release = Arc::clone(&release);
                let outcomes = outcomes.clone();
                thread::spawn(move || {
                    start.wait();
                    match provider.reserve_snapshot_capacity() {
                        Ok(reservation) => {
                            outcomes.send(true).unwrap();
                            let deadline = Instant::now() + Duration::from_secs(5);
                            while !release.load(Ordering::Acquire) {
                                assert!(
                                    Instant::now() < deadline,
                                    "timed out waiting to release snapshot reservation"
                                );
                                thread::yield_now();
                            }
                            drop(reservation);
                        }
                        Err(error) => {
                            assert_eq!(
                                error.code,
                                SourceResourceErrorCode::SnapshotCapacityExceeded
                            );
                            outcomes.send(false).unwrap();
                        }
                    }
                })
            })
            .collect::<Vec<_>>();
        drop(outcomes);

        start.wait();
        let successful = (0..WORKERS)
            .map(|_| received.recv_timeout(Duration::from_secs(5)).unwrap())
            .filter(|reserved| *reserved)
            .count();
        assert_eq!(successful, MAX_CONCURRENT_RESERVATIONS);

        release.store(true, Ordering::Release);
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(
            provider.reserve_snapshot_capacity().unwrap().byte_size,
            MAX_SNAPSHOT_BYTES
        );
    }

    #[test]
    fn source_resources_publish_rejects_snapshot_larger_than_its_reservation() {
        let fixture = Fixture::new(b"x");
        let (provider, _) = provider();
        let page = provider
            .resources(
                fixture.module_request(ResourceScope::SelfOnly, 50),
                &fixture.context,
                &CancellationToken::new(),
            )
            .unwrap();
        let mut snapshot = provider
            .snapshots
            .lock()
            .unwrap()
            .snapshots
            .remove(&page.snapshot_id)
            .unwrap();
        let reservation = provider.reserve_snapshot_capacity().unwrap();
        snapshot.byte_size = reservation.byte_size + 1;

        let error = reservation.publish(snapshot).unwrap_err();

        assert_eq!(
            error.code,
            SourceResourceErrorCode::SnapshotCapacityExceeded
        );
        let store = provider.snapshots.lock().unwrap();
        assert!(store.snapshots.is_empty());
        assert_eq!(store.reserved_bytes, 0);
        assert_eq!(store.reserved_count, 0);
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

    #[test]
    fn source_resource_limits_and_cancellation_matrix_is_exact() {
        use crate::domain::source_resources::{
            SOURCE_MANIFEST_RESOURCE_MAX, SOURCE_REPLACEMENT_MAX_BYTES,
            SOURCE_RESOURCE_PAGE_LIMIT_MAX, SOURCE_SNAPSHOT_TTL_SECONDS,
        };

        assert_eq!(SOURCE_MANIFEST_RESOURCE_MAX, 100);
        assert_eq!(SOURCE_RESOURCE_PAGE_LIMIT_MAX, 50);
        assert_eq!(SOURCE_READ_LIMIT_MAX, 64 * 1024);
        assert_eq!(SOURCE_REPLACEMENT_MAX_BYTES, 1024 * 1024);
        assert_eq!(SOURCE_SNAPSHOT_TTL_SECONDS, 5 * 60);
        assert_eq!(MAX_SNAPSHOT_BYTES, 32 * 1024 * 1024);
        assert_eq!(MAX_LIVE_SNAPSHOTS, 64);
        assert_eq!(MAX_LIVE_SNAPSHOT_BYTES, 128 * 1024 * 1024);

        let fixture = Fixture::new(b"Procedure Run()\nEndProcedure\n");
        for limit in [0, SOURCE_RESOURCE_PAGE_LIMIT_MAX + 1] {
            let (provider, _) = provider();
            let error = provider
                .resources(
                    fixture.module_request(ResourceScope::SelfOnly, limit),
                    &fixture.context,
                    &CancellationToken::new(),
                )
                .unwrap_err();
            assert_eq!(
                error.code,
                SourceResourceErrorCode::LimitExceeded,
                "limit={limit}"
            );
        }
        for limit in [0, SOURCE_READ_LIMIT_MAX + 1] {
            let (provider, _) = provider();
            let error = provider
                .read(
                    SourceReadRequest {
                        snapshot_id: Uuid::new_v4().to_string(),
                        resource_id: Uuid::new_v4().to_string(),
                        offset: 0,
                        limit,
                    },
                    &fixture.context,
                    &CancellationToken::new(),
                )
                .unwrap_err();
            assert_eq!(
                error.code,
                SourceResourceErrorCode::LimitExceeded,
                "read limit={limit}"
            );
        }

        let cancelled_before_resolution = CancellationToken::new();
        cancelled_before_resolution.cancel();
        let (cancelled_provider, _) = provider();
        let error = cancelled_provider
            .resources(
                fixture.module_request(ResourceScope::SelfOnly, 1),
                &fixture.context,
                &cancelled_before_resolution,
            )
            .unwrap_err();
        assert_eq!(error.code, SourceResourceErrorCode::Cancelled);

        let (publication_provider, _) = provider();
        let cancelled_before_publication = CancellationToken::new();
        let observer_token = cancelled_before_publication.clone();
        publication_provider.set_construction_observer_for_test(move |_| observer_token.cancel());
        let error = publication_provider
            .resources(
                fixture.module_request(ResourceScope::SelfOnly, 1),
                &fixture.context,
                &cancelled_before_publication,
            )
            .unwrap_err();
        assert_eq!(error.code, SourceResourceErrorCode::Cancelled);
        assert!(publication_provider
            .snapshots
            .lock()
            .unwrap()
            .snapshots
            .is_empty());

        source_resources_cancellation_between_phases_returns_private_error();
        ttl_boundary_expires_pages_and_reads_at_exact_deadline();
        live_snapshot_capacity_is_bounded_without_evicting_unexpired_snapshots();
        single_snapshot_byte_limit_has_stable_capacity_error();
        aggregate_construction_never_buffers_beyond_snapshot_budget();
    }
}
