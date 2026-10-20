//! Explicit bootstrap and process-owned leases. Missing storage is not fresh.
use super::OpenError;
use crate::application::execution::AuthorityEpoch;
use crate::application::invocation_store::MAX_TASK_RECORD_ENVELOPE_BYTES;
use crate::domain::execution::digest;
use crate::infrastructure::platform::filesystem::execution_store::{
    self as platform, PublishMode, StableLock, Stage,
};
use crate::infrastructure::platform::filesystem::{
    is_file_lock_contended, RetainedDirectoryCapability, RetainedRegularFileCapability,
};
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

static REGISTRY_GATE: Mutex<()> = Mutex::new(());
static KEEPERS: OnceLock<Mutex<HashMap<[u8; 32], Arc<Keeper>>>> = OnceLock::new();
fn keepers() -> &'static Mutex<HashMap<[u8; 32], Arc<Keeper>>> {
    KEEPERS.get_or_init(Mutex::default)
}
fn invalid() -> OpenError {
    OpenError::InvalidInventory
}
fn lock_error(error: io::Error) -> OpenError {
    if is_file_lock_contended(&error) {
        OpenError::Busy
    } else {
        OpenError::Io(error)
    }
}
pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(DIGITS[(byte >> 4) as usize] as char);
        result.push(DIGITS[(byte & 15) as usize] as char);
    }
    result
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NamespaceSpec {
    build: [u8; 32],
    source: [u8; 32],
    schema: u32,
}
impl NamespaceSpec {
    pub(crate) fn new(build: [u8; 32], source: [u8; 32]) -> Self {
        Self {
            build,
            source,
            schema: 1,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    authority: [u8; 32],
    spec: NamespaceSpec,
    nonce: [u8; 16],
    codec: u32,
    lease_identity: [u8; 16],
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NamespaceSeal {
    registration: Manifest,
    directory_identity: [u8; 16],
    records_identity: [u8; 16],
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RootMarker {
    identity: [u8; 32],
    version: u32,
    directory_identity: [u8; 16],
    registrations_identity: [u8; 16],
    namespaces_identity: [u8; 16],
    ownership_identity: [u8; 16],
    registry_lock_identity: [u8; 16],
}

#[derive(Clone)]
pub(crate) struct AuthorityRoot {
    directory: RetainedDirectoryCapability,
    registrations: RetainedDirectoryCapability,
    namespaces: RetainedDirectoryCapability,
    ownership: RetainedDirectoryCapability,
    marker: RetainedRegularFileCapabilityHolder,
    identity: [u8; 32],
    marker_value: RootMarker,
}
// Capabilities themselves are intentionally not Clone; sharing the immutable
// bootstrap witness does not grant a writer or a lease unlock operation.
#[derive(Clone)]
struct RetainedRegularFileCapabilityHolder(Arc<RetainedRegularFileCapability>);

fn read_json<T: serde::de::DeserializeOwned>(
    file: &RetainedRegularFileCapability,
) -> Result<T, OpenError> {
    let reader = file.open_named_identity_for_read()?;
    crate::infrastructure::platform::filesystem::verify_owner_only_acl(&reader)?;
    let mut bytes = Vec::new();
    reader
        .take(MAX_TASK_RECORD_ENVELOPE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_TASK_RECORD_ENVELOPE_BYTES {
        return Err(invalid());
    }
    file.validate_named_identity()?;
    serde_json::from_slice(&bytes).map_err(|_| invalid())
}
fn publish_new(
    parent: &RetainedDirectoryCapability,
    name: &str,
    bytes: &[u8],
) -> Result<RetainedRegularFileCapability, OpenError> {
    let stage_name = format!(".stage.bootstrap.{}", uuid::Uuid::new_v4());
    let mut stage = Stage::create(parent, OsStr::new(&stage_name))?;
    stage.write_all(bytes)?;
    stage
        .seal()?
        .publish(OsStr::new(name), PublishMode::CreateNew)
        .map_err(|e| {
            OpenError::Io(io::Error::new(
                e.io_error().kind(),
                e.io_error().to_string(),
            ))
        })
}
fn optional_file(
    parent: &RetainedDirectoryCapability,
    name: &str,
) -> Result<Option<RetainedRegularFileCapability>, OpenError> {
    match parent.retain_regular_child(OsStr::new(name)) {
        Ok(value) => Ok(Some(value)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
fn optional_directory(
    parent: &RetainedDirectoryCapability,
    name: &str,
) -> Result<Option<RetainedDirectoryCapability>, OpenError> {
    match parent.retain_directory_child(OsStr::new(name)) {
        Ok(value) => Ok(Some(value)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

struct RegistryGate {
    _process: MutexGuard<'static, ()>,
    _os: StableLock,
}
pub(crate) struct FreshNamespaceTicket {
    root: AuthorityRoot,
    spec: NamespaceSpec,
    key: [u8; 32],
    _gate: RegistryGate,
}
pub(crate) struct NamespaceRegistration {
    root: AuthorityRoot,
    manifest: Manifest,
    registration: RetainedRegularFileCapability,
    child: RetainedDirectoryCapability,
    key: [u8; 32],
    _gate: RegistryGate,
}
pub(crate) enum ResolvedNamespace {
    Fresh(FreshNamespaceTicket),
    Existing(NamespaceRegistration),
}

impl AuthorityRoot {
    /// Explicit fixture/installer provisioning, never a NotFound fallback.
    pub(crate) fn provision(path: &Path, identity: [u8; 32]) -> Result<Self, OpenError> {
        platform::check_durability_support()?;
        let parent_path = path.parent().ok_or_else(invalid)?;
        let name = path.file_name().ok_or_else(invalid)?;
        let parent = RetainedDirectoryCapability::open(parent_path)?;
        let directory = platform::create_owner_only_directory(&parent, name)?;
        platform::flush_directory(&parent)?;
        for child in ["registrations", "namespaces", "ownership"] {
            platform::create_owner_only_directory(&directory, OsStr::new(child))?;
            platform::flush_directory(&directory)?;
        }
        let registry_lock = publish_new(&directory, "registry.lock", b"")?;
        let marker = RootMarker {
            identity,
            version: 1,
            directory_identity: directory.identity().stable_bytes(),
            registrations_identity: directory
                .retain_directory_child(OsStr::new("registrations"))?
                .identity()
                .stable_bytes(),
            namespaces_identity: directory
                .retain_directory_child(OsStr::new("namespaces"))?
                .identity()
                .stable_bytes(),
            ownership_identity: directory
                .retain_directory_child(OsStr::new("ownership"))?
                .identity()
                .stable_bytes(),
            registry_lock_identity: registry_lock.identity().stable_bytes(),
        };
        publish_new(
            &directory,
            "authority.v1",
            &serde_json::to_vec(&marker).expect("closed root marker"),
        )?;
        Self::open_existing(path, identity)
    }

    pub(crate) fn open_existing(path: &Path, identity: [u8; 32]) -> Result<Self, OpenError> {
        platform::check_durability_support()?;
        let directory = RetainedDirectoryCapability::open(path)?;
        platform::verify_owner_only_directory(&directory)?;
        let marker = directory.retain_regular_child(OsStr::new("authority.v1"))?;
        let value: RootMarker = read_json(&marker)?;
        if value.identity != identity
            || value.version != 1
            || value.directory_identity != directory.identity().stable_bytes()
        {
            return Err(invalid());
        }
        let registrations = directory.retain_directory_child(OsStr::new("registrations"))?;
        let namespaces = directory.retain_directory_child(OsStr::new("namespaces"))?;
        let ownership = directory.retain_directory_child(OsStr::new("ownership"))?;
        for child in [&registrations, &namespaces, &ownership] {
            platform::verify_owner_only_directory(child)?;
        }
        let registry_lock = directory.retain_regular_child(OsStr::new("registry.lock"))?;
        if value.registrations_identity != registrations.identity().stable_bytes()
            || value.namespaces_identity != namespaces.identity().stable_bytes()
            || value.ownership_identity != ownership.identity().stable_bytes()
            || value.registry_lock_identity != registry_lock.identity().stable_bytes()
        {
            return Err(invalid());
        }
        Ok(Self {
            directory,
            registrations,
            namespaces,
            ownership,
            marker: RetainedRegularFileCapabilityHolder(Arc::new(marker)),
            identity,
            marker_value: value,
        })
    }

    fn validate(&self) -> Result<(), OpenError> {
        self.directory.validate_named_identity()?;
        self.registrations.validate_named_identity()?;
        self.namespaces.validate_named_identity()?;
        self.ownership.validate_named_identity()?;
        let marker: RootMarker = read_json(&self.marker.0)?;
        let registry_lock = self
            .directory
            .retain_regular_child(OsStr::new("registry.lock"))?;
        if marker != self.marker_value
            || marker.registry_lock_identity != registry_lock.identity().stable_bytes()
        {
            return Err(invalid());
        }
        Ok(())
    }

    pub(crate) fn resolve(&self, spec: NamespaceSpec) -> Result<ResolvedNamespace, OpenError> {
        let process = REGISTRY_GATE.lock().map_err(|_| invalid())?;
        self.validate()?;
        let os = StableLock::acquire_existing(&self.directory, OsStr::new("registry.lock"))
            .map_err(lock_error)?;
        let gate = RegistryGate {
            _process: process,
            _os: os,
        };
        let key =
            digest(&serde_json::to_vec(&(self.identity, &spec)).expect("closed namespace key"));
        if keepers().lock().map_err(|_| invalid())?.contains_key(&key) {
            return Err(OpenError::Busy);
        }
        let name = hex(&key);
        let registration = optional_file(&self.registrations, &name)?;
        let child = optional_directory(&self.namespaces, &name)?;
        match (registration, child) {
            (None, None) => {
                if optional_file(&self.ownership, &name)?.is_some() {
                    return Err(OpenError::IncompleteBootstrap);
                }
                Ok(ResolvedNamespace::Fresh(FreshNamespaceTicket {
                    root: self.clone(),
                    spec,
                    key,
                    _gate: gate,
                }))
            }
            (Some(registration), Some(child)) => {
                let manifest: Manifest = read_json(&registration)?;
                if manifest.authority != self.identity
                    || manifest.spec != spec
                    || manifest.codec != 1
                    || spec.schema != 1
                {
                    return Err(invalid());
                }
                Ok(ResolvedNamespace::Existing(NamespaceRegistration {
                    root: self.clone(),
                    manifest,
                    registration,
                    child,
                    key,
                    _gate: gate,
                }))
            }
            _ => Err(OpenError::IncompleteBootstrap),
        }
    }
}

struct KeeperState {
    frozen: bool,
    closed: bool,
    evidence: Option<Box<dyn Any + Send>>,
}
struct Keeper {
    root: AuthorityRoot,
    lock: StableLock,
    epoch: AuthorityEpoch,
    key: [u8; 32],
    state: Mutex<KeeperState>,
}
impl Keeper {
    fn acquire(
        root: AuthorityRoot,
        key: [u8; 32],
        epoch: AuthorityEpoch,
    ) -> Result<Arc<Self>, OpenError> {
        let mut registry = keepers().lock().map_err(|_| invalid())?;
        if registry.contains_key(&key) {
            return Err(OpenError::Busy);
        }
        let lock = StableLock::acquire_existing(&root.ownership, OsStr::new(&hex(&key)))
            .map_err(lock_error)?;
        let keeper = Arc::new(Self {
            root,
            lock,
            epoch,
            key,
            state: Mutex::new(KeeperState {
                frozen: false,
                closed: false,
                evidence: None,
            }),
        });
        // The registry owns the lease before any registration/catalog mutation.
        registry.insert(key, keeper.clone());
        Ok(keeper)
    }
    fn freeze(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.frozen = true;
        }
    }
}

pub(crate) struct NamespaceAuthority {
    keeper: Arc<Keeper>,
    child: RetainedDirectoryCapability,
    records: RetainedDirectoryCapability,
    registration: RetainedRegularFileCapability,
    marker: RetainedRegularFileCapability,
    manifest: Manifest,
    seal: NamespaceSeal,
    closed: bool,
}
impl Drop for NamespaceAuthority {
    fn drop(&mut self) {
        if !self.closed {
            self.keeper.freeze();
        }
    }
}
impl NamespaceAuthority {
    pub(crate) fn create(
        ticket: FreshNamespaceTicket,
        epoch: AuthorityEpoch,
    ) -> Result<Self, OpenError> {
        let name = hex(&ticket.key);
        publish_new(&ticket.root.ownership, &name, b"")?;
        let keeper = Keeper::acquire(ticket.root.clone(), ticket.key, epoch)?;
        let result = (|| {
            let manifest = Manifest {
                authority: ticket.root.identity,
                spec: ticket.spec.clone(),
                nonce: *uuid::Uuid::new_v4().as_bytes(),
                codec: 1,
                lease_identity: keeper.lock.identity().stable_bytes(),
            };
            let bytes = serde_json::to_vec(&manifest).expect("closed namespace manifest");
            let registration = publish_new(&ticket.root.registrations, &name, &bytes)?;
            let child =
                platform::create_owner_only_directory(&ticket.root.namespaces, OsStr::new(&name))?;
            platform::flush_directory(&ticket.root.namespaces)?;
            let records = platform::create_owner_only_directory(&child, OsStr::new("records"))?;
            platform::flush_directory(&child)?;
            let seal = NamespaceSeal {
                registration: manifest.clone(),
                directory_identity: child.identity().stable_bytes(),
                records_identity: records.identity().stable_bytes(),
            };
            let marker = publish_new(
                &child,
                "manifest.v1",
                &serde_json::to_vec(&seal).expect("closed namespace seal"),
            )?;
            Ok(Self {
                keeper: keeper.clone(),
                child,
                records,
                registration,
                marker,
                manifest,
                seal,
                closed: false,
            })
        })();
        if result.is_err() {
            keeper.freeze();
        }
        result
    }
    pub(crate) fn open(
        registration: NamespaceRegistration,
        epoch: AuthorityEpoch,
    ) -> Result<Self, OpenError> {
        let keeper = Keeper::acquire(registration.root.clone(), registration.key, epoch)?;
        let result = (|| {
            if keeper.lock.identity().stable_bytes() != registration.manifest.lease_identity {
                return Err(invalid());
            }
            platform::verify_owner_only_directory(&registration.child)?;
            let marker = registration
                .child
                .retain_regular_child(OsStr::new("manifest.v1"))?;
            let seal: NamespaceSeal = read_json(&marker)?;
            if seal.registration != registration.manifest
                || seal.directory_identity != registration.child.identity().stable_bytes()
            {
                return Err(invalid());
            }
            let records = registration
                .child
                .retain_directory_child(OsStr::new("records"))?;
            platform::verify_owner_only_directory(&records)?;
            if seal.records_identity != records.identity().stable_bytes() {
                return Err(invalid());
            }
            Ok(Self {
                keeper: keeper.clone(),
                child: registration.child.clone(),
                records,
                registration: registration.registration,
                marker,
                manifest: registration.manifest,
                seal,
                closed: false,
            })
        })();
        if result.is_err() {
            keeper.freeze();
        }
        result
    }
    pub(crate) fn records(&self) -> &RetainedDirectoryCapability {
        &self.records
    }
    pub(crate) fn nonce(&self) -> [u8; 16] {
        self.manifest.nonce
    }
    pub(crate) fn epoch(&self) -> AuthorityEpoch {
        self.keeper.epoch
    }
    pub(crate) fn is_frozen(&self) -> bool {
        self.keeper
            .state
            .lock()
            .map_or(true, |s| s.frozen || s.closed)
    }
    pub(crate) fn freeze(&self) {
        self.keeper.freeze();
    }
    pub(crate) fn retain_failure(&self, evidence: impl Any + Send) {
        self.freeze();
        if let Ok(mut state) = self.keeper.state.lock() {
            if state.evidence.is_none() {
                state.evidence = Some(Box::new(evidence));
            }
        }
    }
    pub(crate) fn validate(&self) -> Result<(), OpenError> {
        if self.is_frozen() {
            return Err(OpenError::Busy);
        }
        let result = (|| {
            self.keeper.root.validate()?;
            self.keeper.lock.validate()?;
            self.child.validate_named_identity()?;
            self.records.validate_named_identity()?;
            if read_json::<Manifest>(&self.registration)? != self.manifest
                || read_json::<NamespaceSeal>(&self.marker)? != self.seal
            {
                return Err(invalid());
            }
            Ok(())
        })();
        if result.is_err() {
            self.freeze();
        }
        result
    }
    /// Caller proves that no running operation or held completion can publish.
    /// Drop deliberately cannot substitute for this explicit normal shutdown.
    pub(crate) fn close_quiescent(mut self) -> Result<(), OpenError> {
        self.validate()?;
        let mut registry = keepers().lock().map_err(|_| invalid())?;
        let mut state = self.keeper.state.lock().map_err(|_| invalid())?;
        if state.frozen {
            return Err(OpenError::Busy);
        }
        state.closed = true;
        registry.remove(&self.keeper.key);
        self.closed = true;
        Ok(())
    }
}
