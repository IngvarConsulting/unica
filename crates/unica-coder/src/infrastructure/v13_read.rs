use crate::application::v13::resolve::{ModuleLine, ResolvedLines};
use crate::application::v13::source_text::{admit_text, ModuleText};
use unica_bsl::projection::{
    CommonModuleProperties, EventProjection, MethodProjection, ModuleProjectionSet,
    RegionProjection,
};
use unica_metadata::validation_subject::{MetadataChildProfile, MetadataTemplateType};
use unica_metadata::{MetaDiagnosticCode, MetaDiagnosticSeverity, MetadataKind};
use {
    crate::application::v13::view::ViewReadAuthority,
    crate::application::v13::view::ViewSourceSnapshot,
    unica_execution_contract::view_error::ViewError,
};

use crate::domain::node_view::{
    BranchRef, CollectionView, NodeView, NodeViewData, SourceText, TextView,
};

use crate::infrastructure::event_sources::module_prefix as shared_module_prefix;
use crate::infrastructure::event_sources::{resolve_event_source, LogicalEventSource};
use unica_bindings::events::{
    form_semantic_inputs, project_platform_event_from_projection,
    project_property_events_from_projection,
};
use unica_bindings::projection::{
    project_module, FormEventBindingInput, ModuleProjectionRequest, PlatformEventWriteCapability,
};
use unica_source_designer::module_layout::module_source_address as shared_module_source_address;

use crate::infrastructure::logical_tree::{route_logical_address, LogicalReader, LogicalTreeRoute};
#[cfg(any(test, feature = "test-support"))]
use crate::infrastructure::platform_xml_source_targets::{
    resolve_platform_xml_target, TargetKindPolicy,
};
use crate::infrastructure::v13_large_configuration::{RegistrationCache, RegistrationIndex};
use crate::infrastructure::v13_read_port::ProviderReadAuthority;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use unica_address::address::{AddressSegment, NodeKind, QualifiedAddress};
use unica_address::module_routes::ModuleRole;
use unica_address::module_routes::ModuleRouteKind;
#[cfg(any(test, feature = "test-support"))]
use unica_address::source_target::SourceTarget;
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_bsl::profile::{ModuleCapability, PlatformProfile};
use unica_forms::facts::{FormEventEvidence, FormInfoData};
use unica_refusal::{RefusalCode, RefusalDetail};
#[cfg(any(test, feature = "test-support"))]
use unica_source_contract::read::SourceDirectory;
use unica_source_contract::SourceSetKind;
use unica_source_designer::owner::PlatformXmlSourceSetOwnerEvidence;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;

#[cfg(any(test, feature = "test-support"))]
std::thread_local! {
    static COMPLETED_VIEW_PROJECTION_PROBE_FOR_TEST: std::cell::RefCell<Option<(String, Vec<String>)>> = const { std::cell::RefCell::new(None) };
}

/// Counts successful complete typed projections of one address on this thread.
#[cfg(any(test, feature = "test-support"))]
pub struct CompletedViewProjectionProbeForTest {
    previous: Option<(String, Vec<String>)>,
}

#[cfg(any(test, feature = "test-support"))]
impl CompletedViewProjectionProbeForTest {
    pub fn start(at: &str) -> Self {
        Self {
            previous: COMPLETED_VIEW_PROJECTION_PROBE_FOR_TEST
                .with(|slot| slot.replace(Some((at.to_owned(), Vec::new())))),
        }
    }

    pub fn revisions(&self) -> Vec<String> {
        COMPLETED_VIEW_PROJECTION_PROBE_FOR_TEST.with(|slot| {
            slot.borrow()
                .as_ref()
                .expect("projection observer is active")
                .1
                .clone()
        })
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for CompletedViewProjectionProbeForTest {
    fn drop(&mut self) {
        COMPLETED_VIEW_PROJECTION_PROBE_FOR_TEST.with(|slot| {
            slot.replace(self.previous.take());
        });
    }
}

#[cfg(any(test, feature = "test-support"))]
fn observe_completed_view_projection_for_test(at: &QualifiedAddress, revision: &str) {
    COMPLETED_VIEW_PROJECTION_PROBE_FOR_TEST.with(|slot| {
        if let Some((target, revisions)) = slot.borrow_mut().as_mut() {
            if target == &at.to_string() {
                revisions.push(revision.to_owned());
            }
        }
    });
}

/// Logical read adapter over one actor-issued source capability. Its read
/// identity is fixed at admission; the adapter never snapshots the tree.
pub struct LogicalViewReadAuthority<'a> {
    cancellation: &'a CancellationToken,
    read: ProviderReadAuthority,
    profile: PlatformProfile,
    deadline: ProviderDeadline,
    module_projections: Mutex<BTreeMap<ModuleProjectionCacheKey, Arc<ModuleProjectionSet>>>,
    configuration_payloads: Mutex<BTreeMap<RevisionCacheKey, Arc<Value>>>,
    verified_owners: Mutex<BTreeSet<OwnerProofCacheKey>>,
    owner_evidence: Mutex<BTreeMap<OwnerProofCacheKey, Arc<PlatformXmlSourceSetOwnerEvidence>>>,
    verified_owner_edges: Mutex<BTreeSet<OwnerEdgeCacheKey>>,
    typed_payloads: Mutex<BTreeMap<TypedPayloadCacheKey, Arc<Value>>>,
    form_data: Mutex<BTreeMap<OwnerProofCacheKey, Arc<FormInfoData>>>,
}

/// One typed reader payload is a function of the admitted revision, the
/// reader and its physical target (plus the XDTO type for that reader), so
/// every logical address projected from the same descriptor shares one parse.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct TypedPayloadCacheKey {
    revision: RevisionCacheKey,
    reader: LogicalReader,
    target: String,
    detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ModuleProjectionCacheKey {
    source_set_identity: String,
    revision: String,
    module_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct RevisionCacheKey {
    source_set_identity: String,
    revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct OwnerProofCacheKey {
    revision: RevisionCacheKey,
    owner: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct OwnerEdgeCacheKey {
    revision: RevisionCacheKey,
    parent: String,
    child: String,
}

mod diagnostic_scope;
mod form_payload;
mod module_body_read;
mod typed_payload_read;
mod validation_input;

impl<'a> LogicalViewReadAuthority<'a> {
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn new(
        cancellation: &'a CancellationToken,
        source_set: impl Into<String>,
        source_set_identity: impl Into<String>,
        source_set_kind: SourceSetKind,
        source_root: Arc<SourceDirectory>,
        profile: PlatformProfile,
    ) -> Self {
        Self::with_read_authority(
            cancellation,
            ProviderReadAuthority::new(
                source_set,
                source_set_identity,
                source_set_kind,
                source_root,
            ),
            profile,
            ProviderDeadline::no_deadline(),
        )
    }

    /// The exact platform profile the reader serves; a logical reader exists
    /// only for such a profile.
    pub const fn platform_profile(&self) -> PlatformProfile {
        self.profile
    }

    pub(crate) fn with_read_authority(
        cancellation: &'a CancellationToken,
        read: ProviderReadAuthority,
        profile: PlatformProfile,
        deadline: ProviderDeadline,
    ) -> Self {
        Self::with_read_authority_and_registration_cache(
            cancellation,
            read,
            profile,
            deadline,
            Arc::new(RegistrationCache::default()),
        )
    }

    pub fn with_read_authority_and_registration_cache(
        cancellation: &'a CancellationToken,
        read: ProviderReadAuthority,
        profile: PlatformProfile,
        deadline: ProviderDeadline,
        registration_cache: Arc<RegistrationCache>,
    ) -> Self {
        Self {
            cancellation,
            read: read.with_artifacts(registration_cache),
            profile,
            deadline,
            module_projections: Mutex::new(BTreeMap::new()),
            configuration_payloads: Mutex::new(BTreeMap::new()),
            verified_owners: Mutex::new(BTreeSet::new()),
            owner_evidence: Mutex::new(BTreeMap::new()),
            verified_owner_edges: Mutex::new(BTreeSet::new()),
            typed_payloads: Mutex::new(BTreeMap::new()),
            form_data: Mutex::new(BTreeMap::new()),
        }
    }

    /// Only the path resolver supplies this placement; subsequent owner and
    /// child-registration checks still consume the target's actual XML.
    pub fn with_external_point_binding(
        mut self,
        binding: unica_source_designer::external_artifact::ExternalArtifactBinding,
    ) -> Self {
        self.read = self.read.with_external_point_binding(binding);
        self
    }

    fn exact_revision(&self) -> Result<String, ViewError> {
        self.read_checkpoint()?;
        self.read.exact_revision(self.deadline, self.cancellation)
    }

    fn read_checkpoint(&self) -> Result<(), ViewError> {
        if self.cancellation.is_cancelled() {
            return Err(ViewError::new(
                RefusalCode::Cancelled,
                "logical read was cancelled",
            ));
        }
        if self.deadline.is_elapsed() {
            return Err(ViewError::new(
                RefusalCode::DeadlineExceeded,
                "logical read operation deadline elapsed",
            ));
        }
        Ok(())
    }

    /// Resolve can reuse the typed identity locator only after reading the
    /// exact node. View treats an absent BSL file as empty; a source location
    /// must distinguish that case from an existing empty file.
    pub fn existing_export_path(
        &self,
        at: &QualifiedAddress,
        requested_path: Option<&str>,
    ) -> Result<String, ViewError> {
        self.read_checkpoint()?;
        let route = route_logical_address(at, self.profile)
            .map_err(|error| ViewError::new(RefusalCode::NotFound, error.to_string()))?;
        if route.reader() == LogicalReader::Module {
            let capability = route.module().ok_or_else(|| {
                ViewError::new(RefusalCode::NotFound, "the logical node has no BSL module")
            })?;
            let (module_at, _) = module_prefix(at, self.profile, capability)?;
            let target = module_source_address(&module_at, capability)?;
            if !self.read.module_source_exists(&target)? {
                return Err(ViewError::new(
                    RefusalCode::NotFound,
                    "the BSL module source file does not exist",
                ));
            }
        }
        let path = self.identity_export_path(at)?.ok_or_else(|| {
            ViewError::new(RefusalCode::NotFound, "the logical node has no source file")
        })?;
        if matches!(at.segments(), [owner, definition]
            if owner.kind() == NodeKind::WsReference && definition.kind() == NodeKind::Definition)
        {
            let target = route
                .reader_metadata_path()
                .ok_or_else(|| ViewError::new(RefusalCode::NotFound, "WSDL route has no owner"))?;
            if !self
                .read
                .attached_resource_exists(target, "WSDefinition.xml")?
            {
                return Err(ViewError::new(
                    RefusalCode::NotFound,
                    "the local WSDefinition.xml source file does not exist",
                ));
            }
        }
        if route.reader() == LogicalReader::Form {
            let target = route
                .reader_metadata_path()
                .ok_or_else(|| ViewError::new(RefusalCode::NotFound, "form route has no target"))?;
            if !self.read.form_source_exists(target)? {
                return Err(ViewError::new(
                    RefusalCode::NotFound,
                    "the local Form.xml source file does not exist",
                ));
            }
        }
        // An absent schedule is a successful `view` answer (`state: absent`),
        // but the path its layout names is not a source file to resolve to.
        if route.reader() == LogicalReader::Metadata
            && matches!(at.segments(), [owner, schedule]
                if owner.kind() == NodeKind::ScheduledJob && schedule.kind() == NodeKind::Schedule)
        {
            let target = route.reader_metadata_path().ok_or_else(|| {
                ViewError::new(RefusalCode::NotFound, "schedule route has no target")
            })?;
            if !self.read.attached_resource_exists(target, "Schedule.xml")? {
                return Err(ViewError::new(
                    RefusalCode::NotFound,
                    "the local Schedule.xml source file does not exist",
                ));
            }
        }
        if requested_path.is_some_and(|requested| requested != path) {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                "the requested file is not the logical node's source",
            ));
        }
        self.read_checkpoint()?;
        Ok(path)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn module_source_read_count(&self, target: &str) -> usize {
        self.read.module_source_read_count(target)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn configuration_payload_read_count(&self) -> usize {
        self.read.configuration_payload_read_count()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn metadata_descriptor_read_count(&self, target: &str) -> usize {
        self.read.metadata_descriptor_read_count(target)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn support_state_read_count(&self) -> usize {
        self.read.support_state_read_count()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub const fn source_set_kind_for_test(&self) -> SourceSetKind {
        self.read.source_set_kind()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub const fn deadline_for_test(&self) -> ProviderDeadline {
        self.deadline
    }
}

impl ViewReadAuthority for LogicalViewReadAuthority<'_> {
    fn read_checkpoint(&self) -> Result<(), ViewError> {
        LogicalViewReadAuthority::read_checkpoint(self)
    }

    fn read_module_text(
        &self,
        at: &QualifiedAddress,
        admitted: &ViewSourceSnapshot,
    ) -> Result<Option<ModuleText>, ViewError> {
        let route = route_logical_address(at, self.profile)
            .map_err(|error| ViewError::new(RefusalCode::NotFound, error.to_string()))?;
        if route.reader() != LogicalReader::Module {
            return Ok(None);
        }
        let Some(capability) = route.module() else {
            return Ok(None);
        };
        let (module_at, prefix_len) = module_prefix(at, self.profile, capability)?;
        let suffix = &at.segments()[prefix_len..];
        if !matches!(suffix, [body] if body.kind() == NodeKind::Body && body.name().is_none()) {
            return Ok(None);
        }
        self.read_checkpoint()?;
        if admitted.source_set_identity != self.read.source_set_identity()
            || admitted.revision != self.exact_revision()?
        {
            return Err(ViewError::new(
                RefusalCode::StaleCursor,
                "source revision changed before the Body read",
            ));
        }
        self.verify_module_owner(&module_at, capability, admitted)?;
        if capability.role() == ModuleRole::WebSocketClient {
            return Err(ViewError::detailed(
                RefusalDetail::ProviderAbsent,
                "WebSocketClient source layout is not specified for platform profile 8.3.27",
            ));
        }
        let target = module_source_address(&module_at, capability)?;
        let text = match at.line_range() {
            Some(range) => self
                .read
                .module_text_range(&target, range, || self.read_checkpoint())?,
            None => self
                .read
                .module_text_within_limit(&target, || self.read_checkpoint())?,
        };
        Ok(Some(text))
    }

    fn snapshot(&self, at: &QualifiedAddress) -> Result<ViewSourceSnapshot, ViewError> {
        if at.source_set() != self.read.source_set() {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                "logical address belongs to another actor-owned source set",
            ));
        }
        route_logical_address(at, self.profile)
            .map_err(|error| ViewError::new(RefusalCode::NotFound, error.to_string()))?;
        Ok(ViewSourceSnapshot {
            source_set_identity: self.read.source_set_identity().to_string(),
            revision: self.exact_revision()?,
        })
    }

    fn canonical_address(
        &self,
        at: &QualifiedAddress,
        admitted: &ViewSourceSnapshot,
    ) -> Result<QualifiedAddress, ViewError> {
        if admitted.source_set_identity != self.read.source_set_identity()
            || admitted.revision != self.exact_revision()?
        {
            return Err(ViewError::new(
                RefusalCode::StaleCursor,
                "source revision changed before canonical address resolution",
            ));
        }
        let route = route_logical_address(at, self.profile)
            .map_err(|error| ViewError::new(RefusalCode::NotFound, error.to_string()))?;
        if route.reader() != LogicalReader::Role {
            return Ok(at.clone());
        }
        if let Some((borrowing, resource)) = self.inherited_route_part(&route, admitted)? {
            // Без своих прав каноническим остаётся сам адрес роли; адрес
            // права внутри неё отказывает так же, как в `view`.
            project_inherited_part(&route, &borrowing, resource)?;
            return Ok(at.clone());
        }
        let payload = self.typed_payload_for(&route, admitted)?;
        let projected = project_typed_payload(&route, payload)?;
        if admitted.revision != self.exact_revision()? {
            return Err(ViewError::new(
                RefusalCode::StaleCursor,
                "source revision changed during canonical address resolution",
            ));
        }
        QualifiedAddress::parse(projected.at()).map_err(|error| {
            ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
        })
    }

    fn identity_export_path(&self, at: &QualifiedAddress) -> Result<Option<String>, ViewError> {
        if at.source_set() != self.read.source_set() {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                "logical address belongs to another actor-owned source set",
            ));
        }
        let route = route_logical_address(at, self.profile)
            .map_err(|error| ViewError::new(RefusalCode::NotFound, error.to_string()))?;
        if route.reader() == LogicalReader::Configuration {
            return Ok(matches!(
                self.read.source_set_kind(),
                SourceSetKind::Configuration | SourceSetKind::Extension
            )
            .then(|| "Configuration.xml".to_string()));
        }
        if route.reader() == LogicalReader::Module {
            if route.module().is_none() {
                return Ok(None);
            }
            let capability = route.module().ok_or_else(|| {
                ViewError::new(
                    RefusalCode::ProviderUnavailable,
                    "module capability is missing",
                )
            })?;
            let (module_at, _) = module_prefix(at, self.profile, capability)?;
            let target = module_source_address(&module_at, capability)?;
            return self.read.module_export_path(&target).map(Some);
        }
        let Some(target) = route.reader_metadata_path() else {
            return Ok(None);
        };
        if target.as_str().split('.').next() == Some(NodeKind::WebSocketClient.as_str()) {
            return Ok(None);
        }
        if !route.nested_subsystems().is_empty() {
            let resource =
                (route.reader() == LogicalReader::Interface).then_some("CommandInterface.xml");
            return self
                .read
                .nested_subsystem_export_path(target, route.nested_subsystems(), resource)
                .map(Some);
        }
        let target_depth = target.as_str().split('.').count().div_ceil(2);
        let is_detail = at.segments().len() > target_depth;
        let path = match route.reader() {
            LogicalReader::Metadata
                if matches!(at.segments(), [owner, definition]
                if owner.kind() == NodeKind::WsReference && definition.kind() == NodeKind::Definition) =>
            {
                self.read
                    .attached_resource_export_path(target, "WSDefinition.xml")?
            }
            LogicalReader::Metadata
                if matches!(at.segments(), [owner, schedule]
                if owner.kind() == NodeKind::ScheduledJob && schedule.kind() == NodeKind::Schedule) =>
            {
                self.read
                    .attached_resource_export_path(target, "Schedule.xml")?
            }
            LogicalReader::Form => self
                .read
                .attached_resource_export_path(target, "Form.xml")?,
            LogicalReader::Role if is_detail => self
                .read
                .attached_resource_export_path(target, "Rights.xml")?,
            LogicalReader::Interface => self
                .read
                .attached_resource_export_path(target, "CommandInterface.xml")?,
            LogicalReader::Dcs | LogicalReader::Mxl => self
                .read
                .attached_resource_export_path(target, "Template.xml")?,
            LogicalReader::Xdto if is_detail => self
                .read
                .attached_resource_export_path(target, "Package.bin")?,
            LogicalReader::Metadata
            | LogicalReader::Role
            | LogicalReader::Subsystem
            | LogicalReader::Xdto => self.read.metadata_descriptor_export_path(target)?,
            LogicalReader::Configuration | LogicalReader::Module => unreachable!(),
        };
        Ok(Some(path))
    }

    fn permits_identity_fallback(&self, at: &QualifiedAddress) -> bool {
        self.profile
            .module_capability(at)
            .is_some_and(|capability| capability.role() == ModuleRole::WebSocketClient)
    }

    fn read_exact(
        &self,
        at: &QualifiedAddress,
        admitted: &ViewSourceSnapshot,
    ) -> Result<NodeViewData, ViewError> {
        self.read_checkpoint()?;
        if admitted.source_set_identity != self.read.source_set_identity()
            || admitted.revision != self.exact_revision()?
        {
            return Err(ViewError::new(
                RefusalCode::StaleCursor,
                "source revision changed before the typed read",
            ));
        }
        let route = route_logical_address(at, self.profile)
            .map_err(|error| ViewError::new(RefusalCode::NotFound, error.to_string()))?;
        let inherited = self.inherited_route_part(&route, admitted)?;
        let projected = if let Some((borrowing, resource)) = &inherited {
            project_inherited_part(&route, borrowing, resource)?
        } else if route.reader() == LogicalReader::Module {
            self.module_view(&route, admitted)?
        } else if route.reader() == LogicalReader::Configuration
            && matches!(
                self.read.source_set_kind(),
                SourceSetKind::Configuration | SourceSetKind::Extension
            )
            && self.read.configuration_xml_requires_streaming()
        {
            let streamed = self
                .read
                .streamed_configuration_root(&|| self.read_checkpoint())?;
            crate::infrastructure::v13_read_projection::project_configuration_with_counts(
                route.at(),
                &streamed.payload,
                &[],
                streamed.counts,
            )?
        } else if route.reader() == LogicalReader::Metadata
            && route.reader_metadata_path().is_none()
            && route
                .at()
                .segments()
                .last()
                .is_some_and(|segment| segment.name().is_none())
        {
            let branch_kind = route
                .at()
                .segments()
                .last()
                .map(AddressSegment::kind)
                .ok_or_else(|| {
                    ViewError::new(RefusalCode::NotFound, "metadata branch kind is absent")
                })?;
            if matches!(
                self.read.source_set_kind(),
                SourceSetKind::Configuration | SourceSetKind::Extension
            ) && self.read.configuration_xml_requires_streaming()
            {
                let index = self.streamed_configuration_registration_index(admitted)?;
                let names =
                    index.names_for_kind(branch_kind.as_str(), &|| self.read_checkpoint())?;
                if branch_kind != NodeKind::WebSocketClient {
                    for name in &names {
                        let owner = MetadataAddress::parse(
                            PLATFORM_XML_8_3_27_FORMAT_2_20,
                            &format!("{}.{name}", branch_kind.as_str()),
                        )
                        .map_err(|error| {
                            ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                        })?;
                        self.verify_registered_owner(&owner, admitted)?;
                    }
                }
                let support = self.member_support(
                    route.at().source_set(),
                    branch_kind,
                    names.iter().map(String::as_str),
                    admitted,
                )?;
                crate::infrastructure::v13_read_projection::project_registered_metadata_branch_names(
                    route.at(),
                    &names,
                    support,
                )?
            } else {
                let payload = self.configuration_payload(admitted)?;
                if branch_kind != NodeKind::WebSocketClient {
                    for item in payload
                        .get("registeredObjects")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter(|item| {
                            item.get("kind").and_then(Value::as_str) == Some(branch_kind.as_str())
                        })
                    {
                        let name = item.get("name").and_then(Value::as_str).ok_or_else(|| {
                            ViewError::detailed(
                                RefusalDetail::SourceUnreadable,
                                "registered metadata owner has no name",
                            )
                        })?;
                        let owner = MetadataAddress::parse(
                            PLATFORM_XML_8_3_27_FORMAT_2_20,
                            &format!("{}.{name}", branch_kind.as_str()),
                        )
                        .map_err(|error| {
                            ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                        })?;
                        self.verify_registered_owner(&owner, admitted)?;
                    }
                }
                let names = payload
                    .get("registeredObjects")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|item| {
                        item.get("kind").and_then(Value::as_str) == Some(branch_kind.as_str())
                    })
                    .filter_map(|item| item.get("name").and_then(Value::as_str));
                let support =
                    self.member_support(route.at().source_set(), branch_kind, names, admitted)?;
                crate::infrastructure::v13_read_projection::project_registered_metadata_branch(
                    route.at(),
                    payload.as_ref(),
                    support,
                )?
            }
        } else if route.reader() == LogicalReader::Form {
            self.form_view(&route, admitted)?
        } else {
            let payload = self.typed_payload_for(&route, admitted)?;
            let projected =
                crate::infrastructure::v13_read_projection::project_typed_payload_for_source(
                    &route,
                    payload,
                    self.read.source_set_kind(),
                )?;
            self.with_template_body_branches(projected, &route)?
        };
        let projected = match projected {
            NodeViewData::Collection(collection)
                if matches!(at.segments(), [root]
                    if root.kind() == NodeKind::CommonModule && root.name().is_none()) =>
            {
                NodeViewData::Collection(collection.try_map_items(|mut row| {
                    let owner = row["at"].as_str().ok_or_else(|| {
                        ViewError::detailed(
                            RefusalDetail::SourceUnreadable,
                            "common module row has no address",
                        )
                    })?;
                    let owner = QualifiedAddress::parse(owner).map_err(|error| {
                        ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                    })?;
                    let (synonym, limits) = match self.common_module_synonym(&owner) {
                        Ok(fact) => fact,
                        Err(error) if error.detail() == Some(RefusalDetail::SourceUnreadable) => (
                            None,
                            vec![format!("common module Synonym is unreadable: {error}")],
                        ),
                        Err(error) => return Err(error),
                    };
                    if let Some(synonym) = synonym {
                        row["props"] = json!({"synonym": synonym});
                    }
                    if !limits.is_empty() {
                        row["limits"] = json!(limits);
                    }
                    Ok::<_, ViewError>(row)
                })?)
            }
            other => other,
        };
        // Модуль формы без `Form.xml` платформа не выгружает: ветви модуля у
        // унаследованной формы нет.
        let projected = if inherited.is_some() {
            projected
        } else {
            self.with_module_branch(projected, at, admitted)?
        };
        let projected = if route.reader() == LogicalReader::Form
            && route.reader_metadata_path().is_some_and(|target| {
                at.segments().len() == target.as_str().split('.').count().div_ceil(2)
            }) {
            // #2159: the form's declaration (synonym, comment, form type,
            // standard commands, explanation) is shown next to its structure,
            // as `declaration`, and changed in the `Metadata` node. A
            // declaration that cannot be read is named, never the whole form.
            let metadata_at =
                QualifiedAddress::parse(&format!("{at}.Metadata")).map_err(|error| {
                    ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                })?;
            let declaration = match route.reader_metadata_path() {
                Some(target) => self
                    .read
                    .form_metadata_payload(target, &mut || self.read_checkpoint()),
                None => Err(ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    "form route has no typed target",
                )),
            };
            let projected = match declaration {
                Ok(declaration) => {
                    let properties = declaration["properties"]
                        .as_object()
                        .cloned()
                        .unwrap_or_default();
                    let (declaration_prop, mut limits) =
                        crate::infrastructure::v13_read_projection::form_declaration_prop(
                            at,
                            &metadata_at,
                            &properties,
                        );
                    limits.extend(
                        declaration["unreadable"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|item| item["message"].as_str())
                            .map(|message| format!("declaration: {message}")),
                    );
                    let projected = match declaration_prop {
                        Some(value) => projected.with_prop("declaration", value),
                        None => projected,
                    };
                    projected.with_additional_limits(limits)
                }
                Err(error)
                    if matches!(
                        error.code(),
                        RefusalCode::Cancelled | RefusalCode::DeadlineExceeded
                    ) =>
                {
                    return Err(error)
                }
                Err(error) => projected.with_additional_limits(vec![format!(
                    "declaration is unreadable ({error}); read it at {metadata_at}"
                )]),
            };
            projected.with_branch(crate::domain::node_view::BranchRef::new(
                format!("{at}.Metadata"),
                1,
            ))
        } else {
            projected
        };
        self.read_checkpoint()?;
        if admitted.revision != self.exact_revision()? {
            return Err(ViewError::new(
                RefusalCode::StaleCursor,
                "source revision changed during the typed read",
            ));
        }
        #[cfg(any(test, feature = "test-support"))]
        observe_completed_view_projection_for_test(at, &admitted.revision);
        Ok(projected)
    }
}

impl LogicalViewReadAuthority<'_> {
    /// Search needs the named owner's registration and descriptor, but not a
    /// projected collection of its children. An unnamed kind branch is a
    /// valid logical scope even when it currently contains no objects.
    pub fn validate_search_scope_owner(
        &self,
        scope: &QualifiedAddress,
        admitted: &ViewSourceSnapshot,
    ) -> Result<(), ViewError> {
        let [owner] = scope.segments() else {
            return Err(ViewError::new(
                RefusalCode::UnsupportedScope,
                "search scope must name the root or one metadata branch or owner",
            ));
        };
        if owner.kind() == NodeKind::Configuration {
            return Ok(());
        }
        let Some(name) = owner.name() else {
            return Ok(());
        };
        let target = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            &format!("{}.{name}", owner.kind().as_str()),
        )
        .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string()))?;
        self.verify_registered_owner(&target, admitted)
    }

    /// Сопутствующий файл, которого заимствованный объект в расширении не
    /// несёт (см. `ProviderReadAuthority::inherited_resource`): права роли,
    /// описание формы, пакет XDTO или тело макета.
    fn inherited_route_part(
        &self,
        route: &LogicalTreeRoute,
        admitted: &ViewSourceSnapshot,
    ) -> Result<Option<(unica_metadata::borrowing::MetaBorrowing, &'static str)>, ViewError> {
        // Модуль формы живёт в её содержимом: без `Form.xml` платформа его
        // не хранит, и узлы модуля такой формы в расширении не существуют.
        let resource = match route.reader() {
            LogicalReader::Role => "Rights.xml",
            LogicalReader::Form | LogicalReader::Module => "Form.xml",
            LogicalReader::Xdto => "Package.bin",
            LogicalReader::Dcs | LogicalReader::Mxl => "Template.xml",
            _ => return Ok(None),
        };
        if self.read.source_set_kind() != SourceSetKind::Extension {
            return Ok(None);
        }
        let Some(target) =
            crate::infrastructure::v13_read_port::companion_owner(route.at(), resource)
        else {
            return Ok(None);
        };
        self.verify_registered_owner(&target, admitted)?;
        Ok(self
            .read
            .inherited_resource(&target, resource)?
            .map(|borrowing| (borrowing, resource)))
    }

    /// A template's own body (data sets of a DCS, areas of a spreadsheet) is
    /// read only when the template node itself is addressed. Projecting the
    /// owner or the template collection never opens template payloads, so a
    /// large or unreadable body fails exactly one node instead of its owner.
    fn with_template_body_branches(
        &self,
        projected: NodeViewData,
        route: &LogicalTreeRoute,
    ) -> Result<NodeViewData, ViewError> {
        if route.reader() != LogicalReader::Metadata {
            return Ok(projected);
        }
        let segments = route.at().segments();
        let child_text = match segments {
            [template]
                if template.kind() == NodeKind::CommonTemplate && template.name().is_some() =>
            {
                format!("CommonTemplate.{}", template.name().unwrap_or_default())
            }
            [owner, template]
                if template.kind() == NodeKind::Template
                    && template.name().is_some()
                    && owner.name().is_some() =>
            {
                format!(
                    "{}.{}.Template.{}",
                    owner.kind().as_str(),
                    owner.name().unwrap_or_default(),
                    template.name().unwrap_or_default()
                )
            }
            _ => return Ok(projected),
        };
        let child = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &child_text).map_err(
            |error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string()),
        )?;
        let mut projected = projected;
        let profile = self.read.metadata_child_profile(&child);
        let format = match &profile {
            Ok(unica_metadata::validation_subject::MetadataChildProfile::Template(
                unica_metadata::validation_subject::MetadataTemplateType::DataCompositionSchema,
            )) => Some("DataCompositionSchema"),
            Ok(unica_metadata::validation_subject::MetadataChildProfile::Template(
                unica_metadata::validation_subject::MetadataTemplateType::SpreadsheetDocument,
            )) => Some("SpreadsheetDocument"),
            _ => None,
        };
        let common_opaque_xml = matches!(segments, [template] if template.kind() == NodeKind::CommonTemplate)
            && matches!(
                &profile,
                Ok(MetadataChildProfile::Template(
                    MetadataTemplateType::HtmlDocument
                        | MetadataTemplateType::DataCompositionAppearanceTemplate
                ))
            );
        if format.is_some() || common_opaque_xml {
            if let Some(borrowing) = self.read.inherited_resource(&child, "Template.xml")? {
                // Тела у заимствованного макета в расширении нет: ветвей не
                // выдумываем, узел называет содержимое унаследованным. Вид
                // макета в описании расширения платформа пишет умолчанием,
                // когда его не задали, поэтому он не говорит о виде тела
                // родителя и остаётся неизвестным.
                projected = projected
                    .with_prop("format", Value::Null)
                    .with_prop("content", json!(INHERITED_CONTENT));
                for (key, value) in
                    crate::infrastructure::v13_read_projection::borrowing_props(&borrowing)
                {
                    projected = projected.with_prop(&key, value);
                }
                if let Some(reason) = child_parent_reason(route.at()) {
                    projected = projected.with_prop("parentReason", json!(reason));
                }
                return Ok(projected);
            }
        }
        if let Some(format) = format {
            projected = projected.with_prop("format", Value::String(format.into()));
        } else if matches!(segments, [template] if template.kind() == NodeKind::CommonTemplate) {
            if let MetadataChildProfile::Template(kind) = profile? {
                projected = projected.with_prop("format", json!(kind.descriptor_value()));
            }
        }
        for (kind, count) in self.template_body_branches(&child)? {
            if count > 0 {
                projected = projected.with_branch(BranchRef::new(
                    format!("{}.{}", route.at(), kind.as_str()),
                    count,
                ));
            }
        }
        Ok(projected)
    }

    fn template_body_branches(
        &self,
        child: &MetadataAddress,
    ) -> Result<Vec<(NodeKind, usize)>, ViewError> {
        Ok(match self.read.metadata_child_profile(child) {
            Ok(unica_metadata::validation_subject::MetadataChildProfile::Template(
                unica_metadata::validation_subject::MetadataTemplateType::DataCompositionSchema,
            )) => {
                let payload = self
                    .read
                    .dcs_payload(child, &mut || self.read_checkpoint())?;
                crate::infrastructure::v13_read_projection::dcs_template_branches(&payload)
            }
            Ok(unica_metadata::validation_subject::MetadataChildProfile::Template(
                unica_metadata::validation_subject::MetadataTemplateType::SpreadsheetDocument,
            )) => {
                let payload = self
                    .read
                    .mxl_payload(child, false, &mut || self.read_checkpoint())?;
                vec![(
                    NodeKind::Area,
                    payload
                        .get("areas")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len),
                )]
            }
            // Text, HTML, binary, add-in and appearance templates have no
            // addressable interior: addressing stops at the template.
            Ok(unica_metadata::validation_subject::MetadataChildProfile::Template(_)) => Vec::new(),
            Ok(
                unica_metadata::validation_subject::MetadataChildProfile::Form
                | unica_metadata::validation_subject::MetadataChildProfile::OrdinaryForm
                | unica_metadata::validation_subject::MetadataChildProfile::Command,
            ) => {
                return Err(ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    "template registry points to a non-template descriptor",
                ));
            }
            Err(error) if error.code() == RefusalCode::NotFound => Vec::new(),
            Err(error) => return Err(error),
        })
    }

    fn with_module_branch(
        &self,
        projected: NodeViewData,
        parent: &QualifiedAddress,
        admitted: &ViewSourceSnapshot,
    ) -> Result<NodeViewData, ViewError> {
        let Some(branch) =
            module_branch_for_parent(parent, self.profile, self.read.source_set_kind())
        else {
            return Ok(projected);
        };
        if let Some(owner) = parent
            .segments()
            .first()
            .filter(|owner| owner.kind() == NodeKind::WebSocketClient)
        {
            self.ensure_owner_registered_parts(
                NodeKind::WebSocketClient.as_str(),
                owner.name().ok_or_else(|| {
                    ViewError::new(
                        RefusalCode::NotFound,
                        "WebSocketClient owner name is absent",
                    )
                })?,
                admitted,
            )?;
        } else if let Some(owner) = module_branch_owner_address(parent)? {
            self.verify_registered_module_owner(&owner, admitted)?;
        }
        Ok(projected.with_branch(branch))
    }
}

/// Объект верхнего уровня, чей `props.extends` называет родителя: у формы и
/// макета владельца адрес родителя разрешается по владельцу.
pub(crate) fn extension_object(at: &QualifiedAddress) -> String {
    match at.segments().first() {
        Some(owner) => format!(
            "{}:{}",
            at.source_set(),
            render_segments(std::slice::from_ref(owner))
        ),
        None => at.to_string(),
    }
}

/// Родителя заимствованной формы или макета владельца читатель не ищет
/// отдельно: это тот же ребёнок родителя владельца. `parentStatus` у такого
/// узла остаётся `unavailable`, а причина называет, где адрес разрешается.
fn child_parent_reason(at: &QualifiedAddress) -> Option<String> {
    (at.segments().len() > 1).then(|| {
        format!(
            "the parent of a borrowed child is the same child of the owner's parent: read props.extends of `{}`",
            extension_object(at)
        )
    })
}

/// Значение `props.content` узла, чьё содержимое расширение не несёт.
pub const INHERITED_CONTENT: &str = "inherited";

/// Узел заимствованного объекта без своего сопутствующего файла.
///
/// Сам объект читается по дескриптору: заимствование и адрес родителя, а
/// `content: "inherited"` говорит, что прав, элементов формы, типов пакета
/// или тела макета в расширении нет и действует содержимое родителя. Счёт и
/// ветви не выдумываются. Адрес внутри такого содержимого в этом наборе не
/// существует: отказ `not_found` направляет к родителю.
fn project_inherited_part(
    route: &LogicalTreeRoute,
    borrowing: &unica_metadata::borrowing::MetaBorrowing,
    resource: &str,
) -> Result<NodeViewData, ViewError> {
    let at = route.at();
    let depth = crate::infrastructure::v13_read_port::companion_owner(at, resource)
        .map_or(0, |target| target.as_str().split('.').count().div_ceil(2));
    if at.segments().len() != depth {
        return Err(ViewError::new(
            RefusalCode::NotFound,
            format!(
                "`{at}` lies inside content this borrowed object does not carry: the extension has no `{resource}`, so its content is inherited from the parent; read the parent named by props.extends of `{}`",
                extension_object(at)
            ),
        ));
    }
    let kind = if route.reader() == LogicalReader::Form {
        NodeKind::Form
    } else {
        at.segments()
            .last()
            .map(AddressSegment::kind)
            .unwrap_or(NodeKind::Configuration)
    };
    let title = at
        .segments()
        .last()
        .and_then(AddressSegment::name)
        .unwrap_or(kind.as_str())
        .to_string();
    let mut props = crate::infrastructure::v13_read_projection::borrowing_props(borrowing);
    props.insert("content".into(), json!(INHERITED_CONTENT));
    if let Some(reason) = child_parent_reason(at) {
        props.insert("parentReason".into(), json!(reason));
    }
    Ok(NodeViewData::Node(NodeView::new(
        at.to_string(),
        kind.as_str(),
        title,
        props,
    )))
}

fn module_branch_owner(branch: &QualifiedAddress) -> Result<Option<MetadataAddress>, ViewError> {
    let segments = branch.segments();
    if matches!(segments, [root] if root.kind() == NodeKind::Module) {
        return Ok(None);
    }
    let Some(last) = segments.last() else {
        return Ok(None);
    };
    if last.kind() != NodeKind::Module || last.name().is_some() {
        return Err(ViewError::new(
            RefusalCode::ProviderUnavailable,
            "module branch has no canonical terminal",
        ));
    }
    let logical = render_segments(&segments[..segments.len() - 1]);
    MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &logical)
        .map(Some)
        .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string()))
}

fn identity_only_metadata_payload(kind: &str, name: &str) -> Value {
    json!({
        "name": name,
        "kind": kind,
        "support": {"state": "not_supported"},
        "properties": {},
        "declarations": {},
        "relations": {},
        "collections": {
            "attributes": [],
            "tabularSections": [],
            "dimensions": [],
            "resources": [],
            "enumValues": [],
            "columns": [],
            "forms": [],
            "templates": [],
            "commands": []
        }
    })
}

fn module_branch_owner_address(
    parent: &QualifiedAddress,
) -> Result<Option<MetadataAddress>, ViewError> {
    if matches!(
        parent.segments(),
        [root] if root.kind() == NodeKind::Configuration
    ) {
        return Ok(None);
    }
    MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &parent.logical_path())
        .map(Some)
        .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string()))
}

fn module_owner_address(
    module_at: &QualifiedAddress,
    capability: ModuleCapability,
) -> Result<MetadataAddress, ViewError> {
    let segments = module_at.segments();
    if capability.role() == ModuleRole::Common {
        return MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &module_at.logical_path())
            .map_err(|error| {
                ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
            });
    }
    let terminal = segments.last().ok_or_else(|| {
        ViewError::new(
            RefusalCode::ProviderUnavailable,
            "module address has no terminal",
        )
    })?;
    if terminal.kind() != NodeKind::Module || terminal.name().is_none() {
        return Err(ViewError::new(
            RefusalCode::ProviderUnavailable,
            "module address has no named module terminal",
        ));
    }
    let logical = render_segments(&segments[..segments.len() - 1]);
    MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &logical)
        .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string()))
}

/// Ветка `Module` узла, которому профиль платформы даёт модули. Корень
/// конфигурации и расширения ведёт к модулям приложения; у корня внешней
/// обработки и отчёта их нет.
pub fn module_branch_for_parent(
    parent: &QualifiedAddress,
    profile: PlatformProfile,
    source_kind: SourceSetKind,
) -> Option<BranchRef> {
    let branch = if matches!(parent.segments(), [root] if root.kind() == NodeKind::Configuration) {
        if matches!(
            source_kind,
            SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
        ) {
            return None;
        }
        QualifiedAddress::parse(&format!("{}:Module", parent.source_set())).ok()
    } else {
        QualifiedAddress::parse(&format!("{parent}.Module")).ok()
    };
    let branch = branch?;
    let count = profile.module_children(&branch).len();
    if count == 0 {
        return None;
    }
    Some(BranchRef::new(branch.to_string(), count))
}

pub(crate) fn project_module_branch(
    branch: &QualifiedAddress,
    profile: PlatformProfile,
) -> Result<NodeViewData, ViewError> {
    let children = profile.module_children(branch);
    if children.is_empty() {
        return Err(ViewError::new(
            RefusalCode::NotFound,
            "module branch has no platform capabilities",
        ));
    }
    let items = children
        .iter()
        .map(|child| {
            let capability = child.capability();
            serde_json::to_value(NodeView::new(
                child.at().to_string(),
                NodeKind::Module.as_str(),
                module_title(child.at(), capability),
                Map::from_iter([
                    (
                        "ownerKind".to_string(),
                        json!(capability.owner_kind().as_str()),
                    ),
                    ("role".to_string(), json!(capability.role().as_str())),
                ]),
            ))
            .map_err(|error| ViewError::new(RefusalCode::ProviderUnavailable, error.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(NodeViewData::Collection(CollectionView::new(
        NodeView::new(
            branch.to_string(),
            NodeKind::Module.as_str(),
            NodeKind::Module.as_str(),
            Map::new(),
        ),
        items,
    )))
}

/// Текст ячеек области — отдельный адрес `…Area.<Имя>.Body`, а не признак в
/// фильтре: `props` держат только скаляры, значит содержимое обязано быть
/// коллекцией, а коллекция обязана быть адресуемой. Структурное чтение области
/// за текст не платит.
fn mxl_content_requested(route: &LogicalTreeRoute) -> bool {
    if route.reader() != LogicalReader::Mxl {
        return false;
    }
    let [.., area, body] = route.at().segments() else {
        return false;
    };
    area.kind() == NodeKind::Area
        && area.name().is_some()
        && body.kind() == NodeKind::Body
        && body.name().is_none()
}

fn module_prefix(
    address: &QualifiedAddress,
    profile: PlatformProfile,
    capability: ModuleCapability,
) -> Result<(QualifiedAddress, usize), ViewError> {
    shared_module_prefix(address, profile, capability)
        .map_err(|error| ViewError::new(RefusalCode::ProviderUnavailable, error))
}

fn render_segments(segments: &[AddressSegment]) -> String {
    let mut values = Vec::with_capacity(segments.len() * 2);
    for segment in segments {
        values.push(segment.kind().as_str());
        if let Some(name) = segment.name() {
            values.push(name);
        }
    }
    values.join(".")
}

fn module_source_address(
    module_at: &QualifiedAddress,
    capability: ModuleCapability,
) -> Result<MetadataAddress, ViewError> {
    shared_module_source_address(
        module_at,
        capability.owner_kind(),
        capability.role(),
        capability.route_kind(),
    )
    .map_err(|error| ViewError::new(RefusalCode::ProviderUnavailable, error))
}

fn common_module_properties(
    bytes: &[u8],
    source_kind: SourceSetKind,
) -> Result<CommonModuleProperties, ViewError> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        ViewError::detailed(
            RefusalDetail::SourceUnreadable,
            "common module descriptor is not UTF-8",
        )
    })?;
    use unica_metadata::{
        MetaInfoPropertyAbsence, MetaInfoPropertyValue, META_INFO_PROPERTY_PROFILE,
    };
    let properties = crate::infrastructure::family_codecs::metadata_read_codec()
        .common_module_properties(text)
        .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?;
    let borrowing_property = |name| {
        properties
            .borrowing(name)
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.message))
    };
    borrowing_property("ObjectBelonging")?;
    let borrowed = source_kind == SourceSetKind::Extension
        && borrowing_property("ExtendedConfigurationObject")?.is_some();
    // Платформа 8.3.27 принимает и выгружает заимствованный общий модуль без
    // любого из этих свойств и не переносит в него значения родителя.
    // Отсутствие у заимствованного модуля читается как неизвестное (`None`),
    // у обычного остаётся ошибкой источника. Записанное значение проверяется
    // одинаково.
    let observe = |name: &str| -> Result<Option<MetaInfoPropertyValue>, ViewError> {
        let value = properties
            .info(name)
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.message))?;
        if value.is_none()
            && META_INFO_PROPERTY_PROFILE.absence(MetadataKind::CommonModule, name, borrowed)
                == MetaInfoPropertyAbsence::Required
        {
            return Err(ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                format!("common module descriptor has no {name} property"),
            ));
        }
        Ok(value)
    };
    let boolean = |name: &str| -> Result<Option<bool>, ViewError> {
        match observe(name)? {
            Some(MetaInfoPropertyValue::Boolean(value)) => Ok(Some(value)),
            None => Ok(None),
            Some(_) => Err(ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                format!("common module {name} property is not boolean"),
            )),
        }
    };
    let return_values_reuse = match observe("ReturnValuesReuse")? {
        Some(MetaInfoPropertyValue::String(value)) if !value.trim().is_empty() => {
            Some(value.trim().to_string())
        }
        None => None,
        Some(_) => {
            return Err(ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "common module ReturnValuesReuse property is empty or malformed",
            ))
        }
    };
    Ok(CommonModuleProperties {
        global: boolean("Global")?,
        client_managed_application: boolean("ClientManagedApplication")?,
        server: boolean("Server")?,
        external_connection: boolean("ExternalConnection")?,
        client_ordinary_application: boolean("ClientOrdinaryApplication")?,
        server_call: boolean("ServerCall")?,
        privileged: boolean("Privileged")?,
        return_values_reuse,
    })
}

fn module_title(at: &QualifiedAddress, capability: ModuleCapability) -> String {
    let segments = at.segments();
    let owner_segments = if segments
        .last()
        .is_some_and(|segment| segment.kind() == NodeKind::Module)
    {
        &segments[..segments.len().saturating_sub(1)]
    } else {
        segments
    };
    let owner = owner_segments
        .iter()
        .rev()
        .find_map(AddressSegment::name)
        .unwrap_or("Configuration");
    format!("{} module {owner}", capability.role().as_str())
}

/// Предел ответа о модуле, чьи флаги среды неизвестны.
// resource-limit-excluded: текст предела в ответе view о неизвестных контекстах, не ресурсный предел
pub(crate) const UNKNOWN_CONTEXTS_LIMIT: &str =
    "compilation contexts are unknown because the borrowed common module does not state its execution environment";

fn module_projection_view(
    requested: &QualifiedAddress,
    prefix_len: usize,
    projections: &ModuleProjectionSet,
) -> Result<NodeViewData, ViewError> {
    let unknown = projections
        .summary()
        .props
        .common_module
        .as_ref()
        .is_some_and(|common| !common.execution_environment_known());
    let view = module_projection_view_data(requested, prefix_len, projections)?;
    // `null` в контекстах — ответ «неизвестно»; предел называет причину на
    // уровне узла, чтобы её видел и тот, кто контексты не читает (#1364).
    Ok(if unknown {
        view.with_limit(UNKNOWN_CONTEXTS_LIMIT)
    } else {
        view
    })
}

fn module_projection_view_data(
    requested: &QualifiedAddress,
    prefix_len: usize,
    projections: &ModuleProjectionSet,
) -> Result<NodeViewData, ViewError> {
    let suffix = &requested.segments()[prefix_len..];
    if suffix.is_empty() {
        let summary = projections.summary();
        let props = serde_json::to_value(&summary.props)
            .map_err(|error| ViewError::new(RefusalCode::ProviderUnavailable, error.to_string()))?
            .as_object()
            .cloned()
            .ok_or_else(|| {
                ViewError::new(RefusalCode::ProviderUnavailable, "module props are invalid")
            })?;
        let branches = summary
            .branches
            .iter()
            .map(|branch| BranchRef::new(&branch.at, branch.count).with_bytes(branch.bytes))
            .collect();
        return Ok(NodeViewData::Node(
            NodeView::new(&summary.at, summary.kind, &summary.title, props).with_branches(branches),
        ));
    }
    let branch = suffix[0].kind();
    if (branch == NodeKind::Method && suffix.len() > 2)
        || (matches!(
            branch,
            NodeKind::Interface | NodeKind::Event | NodeKind::Compilation | NodeKind::Body
        ) && suffix.len() > 1)
    {
        return Err(ViewError::new(
            RefusalCode::NotFound,
            "module projection did not consume the complete suffix",
        ));
    }
    match (branch, suffix[0].name(), suffix.get(1)) {
        (NodeKind::Method, None, None) => module_collection(
            requested,
            NodeKind::Method,
            projections
                .methods()
                .iter()
                .map(method_node_value)
                .collect(),
        ),
        (NodeKind::Method, Some(name), None) => projections
            .methods()
            .iter()
            .find(|method| method.name == name)
            .map(method_node)
            .map(NodeViewData::Node)
            .ok_or_else(|| {
                ViewError::new(
                    RefusalCode::NotFound,
                    format!("method `{name}` was not found"),
                )
            }),
        (NodeKind::Method, Some(name), Some(detail))
            if detail.name().is_none() && detail.kind() == NodeKind::Body =>
        {
            let method = find_method(projections, name)?;
            module_text(
                requested,
                projections.method_text(method),
                method.text_from_line,
            )
        }
        (NodeKind::Method, Some(name), Some(detail))
            if detail.name().is_none() && detail.kind() == NodeKind::Compilation =>
        {
            let method = find_method(projections, name)?;
            let from = method.body_from_line.saturating_sub(1);
            let to = method.body_to_line.saturating_add(1);
            let items = projections
                .compilation()
                .iter()
                .filter(|range| range.from_line <= to && range.to_line >= from)
                .map(|range| serde_json::to_value(range).unwrap_or(Value::Null))
                .collect();
            module_collection(requested, NodeKind::Compilation, items)
        }
        (NodeKind::Region, None, None) => module_collection(
            requested,
            NodeKind::Region,
            projections
                .regions()
                .iter()
                .map(region_item_value)
                .collect(),
        ),
        (NodeKind::Region, _, _) => {
            let region = projections
                .region(&requested.to_string())
                .map_err(|error| ViewError::new(RefusalCode::NotFound, error.to_string()))?;
            Ok(NodeViewData::Node(region_node(requested, region)))
        }
        // Срез `Public`/`Library`/`Override` — элемент без адреса: у
        // `Interface` нет имени, и `…Interface.Public` грамматика отвергает.
        // Адресуются методы среза (#1467).
        (NodeKind::Interface, None, None) => module_collection(
            requested,
            NodeKind::Interface,
            projections
                .interfaces()
                .iter()
                .map(|interface| {
                    json!({
                        "kind": NodeKind::Interface.as_str(),
                        "name": interface.interface.as_str(),
                        "addressable": false,
                        "interface": interface.interface,
                        "methods": interface.methods,
                    })
                })
                .collect(),
        ),
        (NodeKind::Event, None, None) => module_collection(
            requested,
            NodeKind::Event,
            projections.events().iter().map(event_node_value).collect(),
        ),
        (NodeKind::Event, Some(name), None) => projections
            .events()
            .iter()
            .find(|event| event.event_id == name)
            .map(event_node)
            .map(NodeViewData::Node)
            .ok_or_else(|| {
                ViewError::new(
                    RefusalCode::NotFound,
                    format!("event `{name}` was not found"),
                )
            }),
        (NodeKind::Compilation, None, None) => module_collection(
            requested,
            NodeKind::Compilation,
            projections
                .compilation()
                .iter()
                .map(|range| serde_json::to_value(range).unwrap_or(Value::Null))
                .collect(),
        ),
        (NodeKind::Body, None, None) => module_text(requested, projections.source_text(), 1),
        _ => Err(ViewError::new(
            RefusalCode::NotFound,
            "module projection suffix is not available",
        )),
    }
}

/// Узел `Body` с текстом; предел проверяет тот, кто отдаёт сам текст.
fn module_text(
    requested: &QualifiedAddress,
    text: &str,
    first_line: usize,
) -> Result<NodeViewData, ViewError> {
    let count = text.lines().count();
    let (from, to) = requested.line_range().map_or(
        (first_line, first_line + count.saturating_sub(1)),
        |range| (range.from, range.to),
    );
    let text = if let Some(range) = requested.line_range() {
        selected_lines(
            requested,
            line_range(first_line, first_line + count.saturating_sub(1)),
        )?;
        let start = range.from - first_line;
        let end = range.to - first_line;
        let mut offset = 0;
        let mut first = 0;
        let mut last = 0;
        for (line, fragment) in text.split_inclusive('\n').enumerate() {
            if line == start {
                first = offset;
            }
            offset += fragment.len();
            if line == end {
                last = offset;
                break;
            }
        }
        &text[first..last]
    } else {
        text
    };
    let text = if admit_text(text).is_ok() {
        SourceText::Whole(text.to_string())
    } else {
        SourceText::Oversized {
            bytes: text.len() as u64,
            prefix: unica_execution_contract::source_text_budget::complete_prefix(
                text,
                unica_bootstrap::MAX_RESPONSE_BUDGET,
            )
            .to_string(),
        }
    };
    Ok(NodeViewData::Text(TextView::new(
        NodeView::new(
            requested.to_string(),
            NodeKind::Body.as_str(),
            NodeKind::Body.as_str(),
            Map::new(),
        ),
        SourceText::Window {
            text: Box::new(text),
            from,
            to,
        },
    )))
}

fn selected_lines(at: &QualifiedAddress, full: ResolvedLines) -> Result<ResolvedLines, ViewError> {
    let Some(range) = at.line_range() else {
        return Ok(full);
    };
    match full {
        ResolvedLines::Range { from, to } if range.from >= from && range.to <= to => {
            Ok(ResolvedLines::Range {
                from: range.from,
                to: range.to,
            })
        }
        _ => Err(ViewError::new(
            RefusalCode::BadValue,
            "Body line selector is outside this Body; use absolute file lines within its bounds",
        )),
    }
}

fn module_collection(
    requested: &QualifiedAddress,
    kind: NodeKind,
    items: Vec<Value>,
) -> Result<NodeViewData, ViewError> {
    Ok(NodeViewData::Collection(CollectionView::new(
        NodeView::new(
            requested.to_string(),
            kind.as_str(),
            kind.as_str(),
            Map::new(),
        ),
        items,
    )))
}

/// An empty interval has no lines to name: such a node is not line-based.
fn line_range(from: usize, to: usize) -> ResolvedLines {
    if from == 0 || to < from {
        return ResolvedLines::NotLineBased;
    }
    ResolvedLines::Range { from, to }
}

fn find_method<'a>(
    projections: &'a ModuleProjectionSet,
    name: &str,
) -> Result<&'a MethodProjection, ViewError> {
    projections
        .methods()
        .iter()
        .find(|method| method.name == name)
        .ok_or_else(|| {
            ViewError::new(
                RefusalCode::NotFound,
                format!("method `{name}` was not found"),
            )
        })
}

fn method_node(method: &MethodProjection) -> NodeView {
    let mut props = Map::from_iter([
        ("signature".to_string(), json!(method.signature)),
        ("methodKind".to_string(), json!(method.method_kind)),
        ("export".to_string(), json!(method.export)),
        ("compile".to_string(), json!(method.compile)),
        // Область называет свои строки; метод называет строки своего текста
        // `Body` — от документирующего комментария до строки окончания.
        // Это координата внутри модуля, а не физическая раскладка.
        ("line".to_string(), json!(method.text_from_line)),
        ("endLine".to_string(), json!(method.to_line)),
    ]);
    if let Some(doc) = &method.doc {
        props.insert("doc".to_string(), json!(doc));
    }
    if !method.handles.is_empty() {
        props.insert("handles".to_string(), json!(method.handles));
    }
    if let Some(extension) = &method.extension {
        props.insert("extension".to_string(), json!(extension));
    }
    // Нормативная область метода (INV.SOURCE.MODULE-REGION-INTERFACE).
    if let Some(area) = method.area {
        props.insert("area".to_string(), json!(area));
    }
    NodeView::new(&method.at, NodeKind::Method.as_str(), &method.name, props).with_branches(vec![
        BranchRef::new(
            format!("{}.Compilation", method.at),
            method.compilation_count,
        ),
        BranchRef::new(format!("{}.Body", method.at), method.text_line_count()),
    ])
}

fn method_node_value(method: &MethodProjection) -> Value {
    serde_json::to_value(method_node(method)).unwrap_or(Value::Null)
}

fn region_node(requested: &QualifiedAddress, region: &RegionProjection) -> NodeView {
    NodeView::new(
        requested.to_string(),
        NodeKind::Region.as_str(),
        region.name.as_deref().unwrap_or("Region"),
        Map::from_iter([
            ("line".to_string(), json!(region.line)),
            ("endLine".to_string(), json!(region.end_line)),
            ("methods".to_string(), json!(region.methods)),
            ("children".to_string(), json!(region.children)),
        ]),
    )
}

fn region_item_value(region: &RegionProjection) -> Value {
    let Some(at) = region.at.as_deref() else {
        return json!({
            "name": region.name,
            "addressable": false,
            "line": region.line,
            "endLine": region.end_line
        });
    };
    let Ok(at) = QualifiedAddress::parse(at) else {
        return json!({"name": region.name, "addressable": false});
    };
    serde_json::to_value(region_node(&at, region)).unwrap_or(Value::Null)
}

pub(super) fn event_node(event: &EventProjection) -> NodeView {
    let mut props = Map::from_iter([
        ("eventId".to_string(), json!(event.event_id)),
        ("state".to_string(), json!(event.state)),
        ("signature".to_string(), json!(event.signature)),
        ("contexts".to_string(), json!(event.contexts)),
        ("binding".to_string(), json!(event.binding)),
        // Several conflicting bindings name no single handler.
        (
            "handler".to_string(),
            if event.handlers.is_empty() {
                json!(event.handler)
            } else {
                Value::Null
            },
        ),
        ("handlerEn".to_string(), json!(event.handler_en)),
        (
            "implementationAt".to_string(),
            json!(event.implementation_at),
        ),
        ("callType".to_string(), json!(event.call_type)),
    ]);
    if !event.handlers.is_empty() {
        props.insert("handlers".to_string(), json!(event.handlers));
    }
    NodeView::new(&event.at, NodeKind::Event.as_str(), &event.event_id, props)
        .with_can(event.can.clone())
}

pub(super) fn event_node_value(event: &EventProjection) -> Value {
    serde_json::to_value(event_node(event)).unwrap_or(Value::Null)
}

fn node_value(
    at: &str,
    kind: NodeKind,
    title: &str,
    props: Map<String, Value>,
    branches: Vec<BranchRef>,
) -> Value {
    serde_json::to_value(NodeView::new(at, kind.as_str(), title, props).with_branches(branches))
        .unwrap_or(Value::Null)
}

fn insert_serialized<T: serde::Serialize>(
    payload: &mut Map<String, Value>,
    key: &str,
    value: &T,
) -> Result<(), ViewError> {
    payload.insert(
        key.to_string(),
        serde_json::to_value(value)
            .map_err(|error| ViewError::new(RefusalCode::ProviderUnavailable, error.to_string()))?,
    );
    Ok(())
}

fn named_segment(address: &QualifiedAddress, kind: NodeKind) -> Option<&str> {
    address
        .segments()
        .iter()
        .find(|segment| segment.kind() == kind)
        .and_then(AddressSegment::name)
}

#[cfg(any(test, feature = "test-support"))]
use crate::infrastructure::v13_read_projection::project_known_suffix;
use crate::infrastructure::v13_read_projection::project_typed_payload;

#[cfg(test)]
pub(crate) mod tests;
