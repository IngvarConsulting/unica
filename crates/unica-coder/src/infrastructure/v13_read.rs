use crate::application::ports::{MetadataChildProfile, MetadataTemplateType};
use crate::application::v13::view::{ViewError, ViewFilter, ViewReadAuthority, ViewSourceSnapshot};
use crate::application::v13::LOGICAL_READ_OPERATION_BUDGET;
use crate::domain::address::{AddressSegment, NodeKind, QualifiedAddress};
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::metadata::MetadataKind;
use crate::domain::module_projection::{
    CommonModuleProperties, EventProjection, MethodProjection, ModuleProjectionSet,
    RegionProjection,
};
use crate::domain::node_view::{BranchRef, CollectionView, NodeView, NodeViewData};
use crate::domain::platform_profile::{
    ModuleCapability, ModuleRole, ModuleSourceLayout, PlatformProfile,
};
use crate::domain::project_sources::SourceSetKind;
#[cfg(test)]
use crate::domain::source_target::SourceTarget;
use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use crate::infrastructure::bsl_module_projection::{
    project_form_owner_events, project_module, FormBindingOwner, FormEventBindingInput,
    FormEventOwnerInput, FormMethodFact, ModuleProjectionRequest, PlatformEventWriteCapability,
};
use crate::infrastructure::logical_tree::{route_logical_address, LogicalReader, LogicalTreeRoute};
use crate::infrastructure::native_operations::form::{
    FormInfoData, FormInfoElement, FormInfoEvent,
};
use crate::infrastructure::native_operations::form_event_registry::FormElementKind;
use crate::infrastructure::platform::filesystem::RetainedDirectoryCapability;
use crate::infrastructure::platform_xml_owner::PlatformXmlSourceSetOwnerEvidence;
#[cfg(test)]
use crate::infrastructure::platform_xml_source_targets::{
    resolve_platform_xml_target, TargetKindPolicy,
};
use crate::infrastructure::source_revision::SourceRevisionService;
use crate::infrastructure::v13_read_port::ProviderReadAuthority;
use serde_json::{json, Map, Value};
#[cfg(test)]
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

#[cfg(test)]
thread_local! {
    static REVIEW_BEFORE_OWNER_PROOF: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
    static REVIEW_AFTER_CANONICAL_ROLE_READ: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
}

#[cfg(test)]
pub(crate) fn review_set_before_owner_proof(hook: impl FnOnce() + 'static) {
    REVIEW_BEFORE_OWNER_PROOF.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(test)]
pub(crate) fn review_set_after_canonical_role_read(hook: impl FnOnce() + 'static) {
    REVIEW_AFTER_CANONICAL_ROLE_READ.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(test)]
fn review_run_before_owner_proof() {
    REVIEW_BEFORE_OWNER_PROOF.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(test)]
fn review_run_after_canonical_role_read() {
    REVIEW_AFTER_CANONICAL_ROLE_READ.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

const MAX_MODULE_BYTES: usize = 8 * 1024 * 1024;
const MODULE_CONTEXTS: &[&str] = &[
    "client",
    "server",
    "externalConnection",
    "thinClient",
    "webClient",
    "thickClientManaged",
    "thickClientOrdinary",
    "mobileClient",
    "mobileAppClient",
    "mobileAppServer",
    "mobileStandaloneServer",
];

#[derive(Default)]
struct ModuleViewFilter {
    context: Option<String>,
    public: Option<bool>,
}

/// Hidden v0.13 read adapter. Its revision service is supplied by the
/// workspace actor that owns the source capability; the adapter never creates
/// an ambient per-call revision authority.
pub(crate) struct LogicalViewReadAuthority<'a> {
    cancellation: &'a CancellationToken,
    read: ProviderReadAuthority,
    profile: PlatformProfile,
    deadline: ProviderDeadline,
    module_projections: Mutex<BTreeMap<ModuleProjectionCacheKey, Arc<ModuleProjectionSet>>>,
    configuration_payloads: Mutex<BTreeMap<RevisionCacheKey, Arc<Value>>>,
    verified_owners: Mutex<BTreeSet<OwnerProofCacheKey>>,
    owner_evidence: Mutex<BTreeMap<OwnerProofCacheKey, Arc<PlatformXmlSourceSetOwnerEvidence>>>,
    verified_owner_edges: Mutex<BTreeSet<OwnerEdgeCacheKey>>,
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

impl<'a> LogicalViewReadAuthority<'a> {
    pub(crate) fn new(
        cancellation: &'a CancellationToken,
        source_set: impl Into<String>,
        source_set_identity: impl Into<String>,
        source_set_kind: SourceSetKind,
        revision_service: Arc<SourceRevisionService>,
        source_root: Arc<RetainedDirectoryCapability>,
        profile: PlatformProfile,
    ) -> Self {
        Self::with_read_authority(
            cancellation,
            ProviderReadAuthority::new(
                source_set,
                source_set_identity,
                source_set_kind,
                source_root,
                revision_service,
            ),
            profile,
            ProviderDeadline::from_budget(LOGICAL_READ_OPERATION_BUDGET),
        )
    }

    pub(crate) fn with_read_authority(
        cancellation: &'a CancellationToken,
        read: ProviderReadAuthority,
        profile: PlatformProfile,
        deadline: ProviderDeadline,
    ) -> Self {
        Self {
            cancellation,
            read,
            profile,
            deadline,
            module_projections: Mutex::new(BTreeMap::new()),
            configuration_payloads: Mutex::new(BTreeMap::new()),
            verified_owners: Mutex::new(BTreeSet::new()),
            owner_evidence: Mutex::new(BTreeMap::new()),
            verified_owner_edges: Mutex::new(BTreeSet::new()),
        }
    }

    fn exact_revision(&self) -> Result<String, ViewError> {
        self.read_checkpoint()?;
        self.read.exact_revision(self.deadline, self.cancellation)
    }

    fn read_checkpoint(&self) -> Result<(), ViewError> {
        if self.cancellation.is_cancelled() {
            return Err(ViewError::new("cancelled", "logical read was cancelled"));
        }
        if self.deadline.remaining().is_zero() {
            return Err(ViewError::new(
                "provider_deadline",
                "logical read operation deadline elapsed",
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn module_source_read_count(&self, target: &str) -> usize {
        self.read.module_source_read_count(target)
    }

    #[cfg(test)]
    pub(crate) fn configuration_payload_read_count(&self) -> usize {
        self.read.configuration_payload_read_count()
    }

    fn typed_payload(&self, route: &LogicalTreeRoute) -> Result<Value, ViewError> {
        let admitted = ViewSourceSnapshot {
            source_set_identity: self.read.source_set_identity().to_string(),
            revision: self.exact_revision()?,
        };
        self.typed_payload_for(route, &admitted)
    }

    fn typed_payload_for(
        &self,
        route: &LogicalTreeRoute,
        admitted: &ViewSourceSnapshot,
    ) -> Result<Value, ViewError> {
        if !matches!(
            route.reader(),
            LogicalReader::Configuration | LogicalReader::Module
        ) {
            if let Some(target) = route.reader_metadata_path() {
                self.verify_registered_owner(target, admitted)?;
            }
        }
        if route.reader() == LogicalReader::Configuration {
            let payload = self.configuration_payload(admitted)?;
            self.verify_top_level_inventory(payload.as_ref(), admitted)?;
            return Ok(payload.as_ref().clone());
        }
        if route.reader() == LogicalReader::Metadata {
            return self.metadata_payload(route, admitted);
        }
        if route.reader() == LogicalReader::Form {
            return self
                .read
                .form_payload(route.reader_metadata_path().ok_or_else(|| {
                    ViewError::new("provider_unavailable", "form route has no typed target")
                })?);
        }
        if route.reader() == LogicalReader::Dcs {
            return self
                .read
                .dcs_payload(route.reader_metadata_path().ok_or_else(|| {
                    ViewError::new("provider_unavailable", "DCS route has no typed target")
                })?);
        }
        if route.reader() == LogicalReader::Role {
            return self
                .read
                .role_payload(route.reader_metadata_path().ok_or_else(|| {
                    ViewError::new("provider_unavailable", "role route has no typed target")
                })?);
        }
        if matches!(
            route.reader(),
            LogicalReader::Subsystem | LogicalReader::Interface
        ) {
            return self
                .read
                .subsystem_payload(route.reader_metadata_path().ok_or_else(|| {
                    ViewError::new(
                        "provider_unavailable",
                        "subsystem route has no typed target",
                    )
                })?);
        }
        if route.reader() == LogicalReader::Mxl {
            return self
                .read
                .mxl_payload(route.reader_metadata_path().ok_or_else(|| {
                    ViewError::new("provider_unavailable", "MXL route has no typed target")
                })?);
        }
        if route.reader() == LogicalReader::Xdto {
            return self.read.xdto_payload(
                route.reader_metadata_path().ok_or_else(|| {
                    ViewError::new("provider_unavailable", "XDTO route has no typed target")
                })?,
                named_segment(route.at(), NodeKind::Type),
            );
        }
        Err(ViewError::new(
            "provider_unavailable",
            "logical reader requires its dedicated retained adapter",
        ))
    }

    fn configuration_payload(
        &self,
        admitted: &ViewSourceSnapshot,
    ) -> Result<Arc<Value>, ViewError> {
        if admitted.source_set_identity != self.read.source_set_identity() {
            return Err(ViewError::new(
                "stale_cursor",
                "configuration payload belongs to another source identity",
            ));
        }
        let key = RevisionCacheKey {
            source_set_identity: admitted.source_set_identity.clone(),
            revision: admitted.revision.clone(),
        };
        let mut cache = self.configuration_payloads.lock().map_err(|_| {
            ViewError::new("provider_unavailable", "configuration cache is poisoned")
        })?;
        if let Some(payload) = cache.get(&key) {
            return Ok(Arc::clone(payload));
        }
        let mut checkpoint = || self.read_checkpoint();
        let payload = Arc::new(
            self.read
                .configuration_payload_with_checkpoint(&mut checkpoint)?,
        );
        cache.insert(key, Arc::clone(&payload));
        Ok(payload)
    }

    fn metadata_payload(
        &self,
        route: &LogicalTreeRoute,
        admitted: &ViewSourceSnapshot,
    ) -> Result<Value, ViewError> {
        if matches!(
            route.at().segments(),
            [owner] if owner.kind() == NodeKind::WebSocketClient && owner.name().is_some()
        ) {
            let owner = &route.at().segments()[0];
            let name = owner.name().expect("matched named WebSocketClient owner");
            self.ensure_owner_registered_parts(NodeKind::WebSocketClient.as_str(), name, admitted)?;
            return Ok(identity_only_metadata_payload(
                NodeKind::WebSocketClient.as_str(),
                name,
            ));
        }
        let target = route.reader_metadata_path().ok_or_else(|| {
            ViewError::new("not_found", "metadata address has no typed reader target")
        })?;
        self.ensure_owner_registered(target, admitted)?;
        if let Some(payload) = self.read.external_metadata_payload(target)? {
            self.verify_payload_physical_children(target, &payload, admitted)?;
            return Ok(payload);
        }
        let kind = target.as_str().split('.').next().unwrap_or_default();
        if kind == NodeKind::WebSocketClient.as_str() {
            let name = target.as_str().split('.').nth(1).unwrap_or_default();
            return Ok(identity_only_metadata_payload(kind, name));
        }
        if MetadataKind::parse(kind).is_err() {
            return self.read.identity_metadata_payload(target);
        }
        let local = self.read.metadata_local(target)?;
        for (kind, children) in [
            (NodeKind::Form, &local.collections.forms),
            (NodeKind::Template, &local.collections.templates),
            (NodeKind::Command, &local.collections.commands),
        ] {
            for child in children {
                let child = MetadataAddress::parse(
                    PLATFORM_XML_8_3_27_FORMAT_2_20,
                    &format!("{}.{}.{}", target.as_str(), kind.as_str(), child.name),
                )
                .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?;
                self.verify_registered_owner(&child, admitted)?;
            }
        }
        let template_branches = local
            .collections
            .templates
            .iter()
            .map(|template| {
                let child = MetadataAddress::parse(
                    PLATFORM_XML_8_3_27_FORMAT_2_20,
                    &format!("{}.Template.{}", target.as_str(), template.name),
                )
                .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?;
                let branches = match self.read.metadata_child_profile(&child) {
                    Ok(MetadataChildProfile::Template(
                        MetadataTemplateType::DataCompositionSchema,
                    )) => {
                        let payload = self.read.dcs_payload(&child)?;
                        vec![(
                            NodeKind::DataSet,
                            payload
                                .get("dataSets")
                                .and_then(Value::as_array)
                                .map_or(0, Vec::len),
                        )]
                    }
                    Ok(MetadataChildProfile::Template(
                        MetadataTemplateType::SpreadsheetDocument,
                    )) => {
                        let payload = self.read.mxl_payload(&child)?;
                        vec![(
                            NodeKind::Area,
                            payload
                                .get("areas")
                                .and_then(Value::as_array)
                                .map_or(0, Vec::len),
                        )]
                    }
                    Ok(MetadataChildProfile::Template(_)) => Vec::new(),
                    Ok(MetadataChildProfile::Form | MetadataChildProfile::Command) => {
                        return Err(ViewError::new(
                            "provider_unavailable",
                            "template registry points to a non-template descriptor",
                        ));
                    }
                    Err(error) if error.code() == "not_found" => Vec::new(),
                    Err(error) => return Err(error),
                };
                Ok((template.name.clone(), branches))
            })
            .collect::<Result<std::collections::BTreeMap<_, _>, ViewError>>()?;
        let mut payload = Map::new();
        payload.insert("name".to_string(), json!(local.name));
        payload.insert("synonym".to_string(), json!(local.synonym));
        insert_serialized(&mut payload, "kind", &local.kind)?;
        insert_serialized(&mut payload, "details", &local.details)?;
        insert_serialized(&mut payload, "support", &local.support)?;
        insert_serialized(&mut payload, "properties", &local.properties)?;
        insert_serialized(&mut payload, "declarations", &local.declarations)?;
        insert_serialized(&mut payload, "relations", &local.relations)?;
        let mut collections = serde_json::to_value(&local.collections)
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?;
        if let Some(templates) = collections
            .get_mut("templates")
            .and_then(Value::as_array_mut)
        {
            for template in templates {
                let Some(name) = template.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let Some(branches) = template_branches.get(name) else {
                    continue;
                };
                template["logicalBranches"] = json!(branches
                    .iter()
                    .filter(|(_, count)| *count > 0)
                    .map(|(kind, count)| json!({"kind": kind.as_str(), "count": count}))
                    .collect::<Vec<_>>());
            }
        }
        payload.insert("collections".to_string(), collections);
        Ok(Value::Object(payload))
    }

    fn verify_payload_physical_children(
        &self,
        target: &MetadataAddress,
        payload: &Value,
        admitted: &ViewSourceSnapshot,
    ) -> Result<(), ViewError> {
        for (kind, field) in [
            (NodeKind::Form, "forms"),
            (NodeKind::Template, "templates"),
            (NodeKind::Command, "commands"),
        ] {
            for child in payload
                .get("collections")
                .and_then(|collections| collections.get(field))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(name) = child.get("name").and_then(Value::as_str) else {
                    return Err(ViewError::new(
                        "provider_unavailable",
                        format!("registered {field} entry has no name"),
                    ));
                };
                let child = MetadataAddress::parse(
                    PLATFORM_XML_8_3_27_FORMAT_2_20,
                    &format!("{}.{}.{name}", target.as_str(), kind.as_str()),
                )
                .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?;
                self.verify_registered_owner(&child, admitted)?;
            }
        }
        Ok(())
    }

    fn ensure_owner_registered(
        &self,
        target: &MetadataAddress,
        admitted: &ViewSourceSnapshot,
    ) -> Result<(), ViewError> {
        let parts = target.as_str().split('.').collect::<Vec<_>>();
        let [owner_kind, owner_name, ..] = parts.as_slice() else {
            return Err(ViewError::new(
                "not_found",
                "metadata owner has no canonical kind and name",
            ));
        };
        self.ensure_owner_registered_parts(owner_kind, owner_name, admitted)
    }

    fn verify_top_level_inventory(
        &self,
        payload: &Value,
        admitted: &ViewSourceSnapshot,
    ) -> Result<(), ViewError> {
        for item in payload
            .get("registeredObjects")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let kind = item.get("kind").and_then(Value::as_str).ok_or_else(|| {
                ViewError::new("provider_unavailable", "registered owner has no kind")
            })?;
            let name = item.get("name").and_then(Value::as_str).ok_or_else(|| {
                ViewError::new("provider_unavailable", "registered owner has no name")
            })?;
            if kind == NodeKind::WebSocketClient.as_str() {
                continue;
            }
            let owner =
                MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &format!("{kind}.{name}"))
                    .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?;
            self.verify_registered_owner(&owner, admitted)?;
        }
        Ok(())
    }

    fn ensure_owner_registered_parts(
        &self,
        owner_kind: &str,
        owner_name: &str,
        admitted: &ViewSourceSnapshot,
    ) -> Result<(), ViewError> {
        let payload = self.configuration_payload(admitted)?;
        let registered = payload
            .get("registeredObjects")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|item| {
                item.get("kind").and_then(Value::as_str) == Some(owner_kind)
                    && item.get("name").and_then(Value::as_str) == Some(owner_name)
            });
        registered.then_some(()).ok_or_else(|| {
            ViewError::new(
                "not_found",
                format!("metadata owner `{owner_kind}.{owner_name}` is not registered"),
            )
        })
    }

    fn module_view(
        &self,
        route: &LogicalTreeRoute,
        admitted: &ViewSourceSnapshot,
        filter: &ModuleViewFilter,
    ) -> Result<NodeViewData, ViewError> {
        let Some(capability) = route.module() else {
            return self.module_branch_view(route.at(), admitted);
        };
        let (module_at, prefix_len) = module_prefix(route.at(), self.profile, capability)?;
        self.verify_module_owner(&module_at, capability, admitted)?;
        if capability.role() == ModuleRole::WebSocketClient {
            return Err(ViewError::new(
                "provider_unavailable",
                "WebSocketClient source layout is not specified for platform profile 8.3.27",
            ));
        }
        let projections = self.module_projection(&module_at, capability, admitted)?;
        module_projection_view(route.at(), prefix_len, projections.as_ref(), filter)
    }

    fn module_projection(
        &self,
        module_at: &QualifiedAddress,
        capability: ModuleCapability,
        admitted: &ViewSourceSnapshot,
    ) -> Result<Arc<ModuleProjectionSet>, ViewError> {
        let key = ModuleProjectionCacheKey {
            source_set_identity: admitted.source_set_identity.clone(),
            revision: admitted.revision.clone(),
            module_at: module_at.to_string(),
        };
        let mut cache = self
            .module_projections
            .lock()
            .map_err(|_| ViewError::new("provider_unavailable", "module cache is poisoned"))?;
        if let Some(projection) = cache.get(&key) {
            return Ok(Arc::clone(projection));
        }
        let metadata_path = module_source_address(module_at, capability)?;
        let source = self.read.module_source(&metadata_path)?;
        let common_module = if capability.role() == ModuleRole::Common {
            let owner = metadata_path
                .as_str()
                .rsplit_once('.')
                .map(|(owner, _)| owner)
                .ok_or_else(|| {
                    ViewError::new("provider_unavailable", "common module owner is invalid")
                })?;
            let owner = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, owner)
                .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?;
            let descriptor = self.read.metadata_descriptor(&owner)?;
            Some(common_module_properties(&descriptor)?)
        } else {
            None
        };
        let handles = if capability.role() == ModuleRole::Form {
            self.form_bindings(module_at)?
        } else {
            Vec::new()
        };
        let projection = Arc::new(
            project_module(ModuleProjectionRequest {
                at: module_at,
                capability,
                title: module_title(module_at, capability),
                rev: &admitted.revision,
                source: source.as_deref(),
                common_module,
                handles: &handles,
                declarative_bindings: &[],
                extension_targets: &[],
                platform_event_write: match self.read.source_set_kind() {
                    SourceSetKind::Extension => PlatformEventWriteCapability::Unproved,
                    _ => PlatformEventWriteCapability::Proven,
                },
            })
            .map_err(|error| ViewError::new("provider_unavailable", error))?,
        );
        cache.insert(key, Arc::clone(&projection));
        Ok(projection)
    }

    fn module_branch_view(
        &self,
        branch: &QualifiedAddress,
        admitted: &ViewSourceSnapshot,
    ) -> Result<NodeViewData, ViewError> {
        if matches!(
            branch.segments(),
            [root] if root.kind() == NodeKind::Module
        ) && matches!(
            self.read.source_set_kind(),
            SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
        ) {
            return Err(ViewError::new(
                "not_found",
                "external source sets have no configuration runtime module branch",
            ));
        }
        if let Some(owner) = branch
            .segments()
            .first()
            .filter(|owner| owner.kind() == NodeKind::WebSocketClient)
        {
            self.ensure_owner_registered_parts(
                NodeKind::WebSocketClient.as_str(),
                owner.name().ok_or_else(|| {
                    ViewError::new("not_found", "WebSocketClient owner name is absent")
                })?,
                admitted,
            )?;
        } else if let Some(owner) = module_branch_owner(branch)? {
            self.verify_registered_module_owner(&owner, admitted)?;
        }
        project_module_branch(branch, self.profile)
    }

    fn verify_module_owner(
        &self,
        module_at: &QualifiedAddress,
        capability: ModuleCapability,
        admitted: &ViewSourceSnapshot,
    ) -> Result<(), ViewError> {
        if capability.source_layout() == ModuleSourceLayout::Root {
            return match self.read.source_set_kind() {
                SourceSetKind::Configuration => Ok(()),
                SourceSetKind::Extension => Err(ViewError::new(
                    "provider_unavailable",
                    "extension root runtime module ownership is not proved",
                )),
                SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport => {
                    Err(ViewError::new(
                        "not_found",
                        "external source sets have no configuration runtime modules",
                    ))
                }
            };
        }
        if capability.role() == ModuleRole::WebSocketClient {
            let owner = module_at
                .segments()
                .first()
                .ok_or_else(|| ViewError::new("not_found", "WebSocketClient owner is absent"))?;
            return self.ensure_owner_registered_parts(
                NodeKind::WebSocketClient.as_str(),
                owner.name().ok_or_else(|| {
                    ViewError::new("not_found", "WebSocketClient owner name is absent")
                })?,
                admitted,
            );
        }
        let owner = module_owner_address(module_at, capability)?;
        self.verify_registered_module_owner(&owner, admitted)
    }

    fn verify_registered_module_owner(
        &self,
        target: &MetadataAddress,
        admitted: &ViewSourceSnapshot,
    ) -> Result<(), ViewError> {
        #[cfg(test)]
        review_run_before_owner_proof();
        self.verify_registered_owner(target, admitted)
    }

    fn verify_registered_owner(
        &self,
        target: &MetadataAddress,
        admitted: &ViewSourceSnapshot,
    ) -> Result<(), ViewError> {
        let key = OwnerProofCacheKey {
            revision: RevisionCacheKey {
                source_set_identity: admitted.source_set_identity.clone(),
                revision: admitted.revision.clone(),
            },
            owner: target.as_str().to_string(),
        };
        if self
            .verified_owners
            .lock()
            .map_err(|_| ViewError::new("provider_unavailable", "owner cache is poisoned"))?
            .contains(&key)
        {
            return Ok(());
        }
        self.ensure_owner_registered(target, admitted)?;
        let parts = target.as_str().split('.').collect::<Vec<_>>();
        if parts.len() < 2 || parts.len() % 2 != 0 {
            return Err(ViewError::new(
                "provider_unavailable",
                "physical metadata owner must contain complete kind/name pairs",
            ));
        }
        let revision = key.revision.clone();
        let mut parent: Option<(MetadataAddress, Arc<PlatformXmlSourceSetOwnerEvidence>)> = None;
        for pair_count in 1..=parts.len() / 2 {
            let end = pair_count * 2;
            let current_text = parts[..end].join(".");
            let current = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &current_text)
                .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?;
            if let Some((parent_address, parent_evidence)) = &parent {
                let child_kind = parts[end - 2];
                let child_name = parts[end - 1];
                let edge_key = OwnerEdgeCacheKey {
                    revision: revision.clone(),
                    parent: parent_address.as_str().to_string(),
                    child: current_text.clone(),
                };
                let mut edges = self.verified_owner_edges.lock().map_err(|_| {
                    ViewError::new("provider_unavailable", "owner edge cache is poisoned")
                })?;
                if !edges.contains(&edge_key) {
                    let registered = parent_evidence
                        .registrations()
                        .any(|(kind, name)| kind == child_kind && name == child_name);
                    if !registered {
                        return Err(ViewError::new(
                            "not_found",
                            format!(
                                "metadata owner `{}` does not register `{child_kind}.{child_name}`",
                                parent_address.as_str()
                            ),
                        ));
                    }
                    edges.insert(edge_key);
                }
            }
            if pair_count == 1 && parts[0] == NodeKind::WebSocketClient.as_str() {
                if parts.len() != 2 {
                    return Err(ViewError::new(
                        "not_found",
                        "WebSocketClient has no specified nested export owners",
                    ));
                }
                continue;
            }
            let evidence = self.owner_evidence(&current, admitted).map_err(|error| {
                if error.code() == "not_found" {
                    ViewError::new(
                        "provider_unavailable",
                        format!(
                            "registered metadata owner `{}` has no descriptor",
                            current.as_str()
                        ),
                    )
                } else {
                    error
                }
            })?;
            if evidence.artifact_kind() != parts[end - 2]
                || evidence.artifact_name() != Some(parts[end - 1])
            {
                return Err(ViewError::new(
                    "provider_unavailable",
                    format!(
                        "metadata descriptor identity does not match `{}`",
                        current.as_str()
                    ),
                ));
            }
            parent = Some((current, evidence));
        }
        self.verified_owners
            .lock()
            .map_err(|_| ViewError::new("provider_unavailable", "owner cache is poisoned"))?
            .insert(key);
        Ok(())
    }

    fn owner_evidence(
        &self,
        target: &MetadataAddress,
        admitted: &ViewSourceSnapshot,
    ) -> Result<Arc<PlatformXmlSourceSetOwnerEvidence>, ViewError> {
        let key = OwnerProofCacheKey {
            revision: RevisionCacheKey {
                source_set_identity: admitted.source_set_identity.clone(),
                revision: admitted.revision.clone(),
            },
            owner: target.as_str().to_string(),
        };
        let mut cache = self.owner_evidence.lock().map_err(|_| {
            ViewError::new("provider_unavailable", "owner evidence cache is poisoned")
        })?;
        if let Some(evidence) = cache.get(&key) {
            return Ok(Arc::clone(evidence));
        }
        let evidence = Arc::new(self.read.metadata_owner_evidence(target)?);
        cache.insert(key, Arc::clone(&evidence));
        Ok(evidence)
    }

    fn form_bindings(
        &self,
        module_at: &QualifiedAddress,
    ) -> Result<Vec<FormEventBindingInput>, ViewError> {
        let module_at_text = module_at.to_string();
        let form_at = module_at_text
            .rsplit_once(".Module.Form")
            .map(|(form, _)| form)
            .ok_or_else(|| {
                ViewError::new("provider_unavailable", "form module address is invalid")
            })?;
        let form = QualifiedAddress::parse(form_at)
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?;
        let metadata_path =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &form.logical_path())
                .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?;
        let data = self.read.form_data(&metadata_path)?;
        Ok(form_semantic_inputs(form_at, &data).bindings)
    }

    fn form_view(
        &self,
        route: &LogicalTreeRoute,
        admitted: &ViewSourceSnapshot,
    ) -> Result<NodeViewData, ViewError> {
        let target = route.reader_metadata_path().ok_or_else(|| {
            ViewError::new("provider_unavailable", "form route has no typed target")
        })?;
        self.verify_registered_owner(target, admitted)?;
        let mut data = self.read.form_data(target)?;
        data.event_context.metadata_owner = route.at().segments().first().map(AddressSegment::kind);
        let form_at =
            QualifiedAddress::parse(&format!("{}:{}", route.at().source_set(), target.as_str()))
                .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?;
        let module_at = QualifiedAddress::parse(&format!("{form_at}.Module.Form"))
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?;
        let capability = self.profile.module_capability(&module_at).ok_or_else(|| {
            ViewError::new(
                "provider_unavailable",
                "form module capability is absent from the platform profile",
            )
        })?;
        self.verify_module_owner(&module_at, capability, admitted)?;
        let module = self.module_projection(&module_at, capability, admitted)?;
        let methods = module
            .methods()
            .iter()
            .map(|method| {
                FormMethodFact::new(
                    &method.name,
                    method.method_kind,
                    &method.signature,
                    method.compile.contexts.iter().map(String::as_str).collect(),
                )
                .with_directive(method.compile.directive.as_deref())
            })
            .collect::<Vec<_>>();
        let inputs = form_semantic_inputs(&form_at.to_string(), &data);
        let events = project_form_owner_events(
            &data.event_context,
            &inputs.owners,
            &inputs.bindings,
            &methods,
        );
        let payload = serde_json::to_value(data).map_err(|error| {
            ViewError::new(
                "provider_unavailable",
                format!("form payload serialization failed: {error}"),
            )
        })?;
        crate::infrastructure::v13_read_projection::project_typed_payload_with_form_events(
            route, payload, &events,
        )
    }
}

impl ViewReadAuthority for LogicalViewReadAuthority<'_> {
    fn snapshot(&self, at: &QualifiedAddress) -> Result<ViewSourceSnapshot, ViewError> {
        if at.source_set() != self.read.source_set() {
            return Err(ViewError::new(
                "not_found",
                "logical address belongs to another actor-owned source set",
            ));
        }
        route_logical_address(at, self.profile)
            .map_err(|error| ViewError::new("not_found", error.to_string()))?;
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
                "stale_cursor",
                "source revision changed before canonical address resolution",
            ));
        }
        let route = route_logical_address(at, self.profile)
            .map_err(|error| ViewError::new("not_found", error.to_string()))?;
        if route.reader() != LogicalReader::Role {
            return Ok(at.clone());
        }
        let payload = self.typed_payload_for(&route, admitted)?;
        let projected = project_typed_payload(&route, payload)?;
        #[cfg(test)]
        review_run_after_canonical_role_read();
        if admitted.revision != self.exact_revision()? {
            return Err(ViewError::new(
                "stale_cursor",
                "source revision changed during canonical address resolution",
            ));
        }
        QualifiedAddress::parse(projected.at())
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))
    }

    fn identity_export_path(&self, at: &QualifiedAddress) -> Result<Option<String>, ViewError> {
        if at.source_set() != self.read.source_set() {
            return Err(ViewError::new(
                "not_found",
                "logical address belongs to another actor-owned source set",
            ));
        }
        let route = route_logical_address(at, self.profile)
            .map_err(|error| ViewError::new("not_found", error.to_string()))?;
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
                ViewError::new("provider_unavailable", "module capability is missing")
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
        let target_depth = target.as_str().split('.').count().div_ceil(2);
        let is_detail = at.segments().len() > target_depth;
        let path = match route.reader() {
            LogicalReader::Form if is_detail => self
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
            | LogicalReader::Form
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
        filter: &ViewFilter,
        admitted: &ViewSourceSnapshot,
    ) -> Result<NodeViewData, ViewError> {
        self.read_checkpoint()?;
        if admitted.source_set_identity != self.read.source_set_identity()
            || admitted.revision != self.exact_revision()?
        {
            return Err(ViewError::new(
                "stale_cursor",
                "source revision changed before the typed read",
            ));
        }
        let route = route_logical_address(at, self.profile)
            .map_err(|error| ViewError::new("not_found", error.to_string()))?;
        let module_filter = validate_view_filter(&route, filter)?;
        let projected = if route.reader() == LogicalReader::Module {
            self.module_view(&route, admitted, &module_filter)?
        } else if route.reader() == LogicalReader::Metadata
            && route.reader_metadata_path().is_none()
            && route
                .at()
                .segments()
                .last()
                .is_some_and(|segment| segment.name().is_none())
        {
            let payload = self.configuration_payload(admitted)?;
            let branch_kind = route
                .at()
                .segments()
                .last()
                .map(AddressSegment::kind)
                .ok_or_else(|| ViewError::new("not_found", "metadata branch kind is absent"))?;
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
                        ViewError::new(
                            "provider_unavailable",
                            "registered metadata owner has no name",
                        )
                    })?;
                    let owner = MetadataAddress::parse(
                        PLATFORM_XML_8_3_27_FORMAT_2_20,
                        &format!("{}.{name}", branch_kind.as_str()),
                    )
                    .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?;
                    self.verify_registered_owner(&owner, admitted)?;
                }
            }
            crate::infrastructure::v13_read_projection::project_registered_metadata_branch(
                route.at(),
                payload.as_ref(),
            )?
        } else if route.reader() == LogicalReader::Form {
            self.form_view(&route, admitted)?
        } else {
            let payload = self.typed_payload_for(&route, admitted)?;
            project_typed_payload(&route, payload)?
        };
        let projected = self.with_module_branch(projected, at, admitted)?;
        self.read_checkpoint()?;
        if admitted.revision != self.exact_revision()? {
            return Err(ViewError::new(
                "stale_cursor",
                "source revision changed during the typed read",
            ));
        }
        Ok(projected)
    }
}

impl LogicalViewReadAuthority<'_> {
    fn with_module_branch(
        &self,
        projected: NodeViewData,
        parent: &QualifiedAddress,
        admitted: &ViewSourceSnapshot,
    ) -> Result<NodeViewData, ViewError> {
        if matches!(
            parent.segments(),
            [root] if root.kind() == NodeKind::Configuration
        ) && matches!(
            self.read.source_set_kind(),
            SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
        ) {
            return Ok(projected);
        }
        let Some(branch) = module_branch_for_parent(parent, self.profile) else {
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
                    ViewError::new("not_found", "WebSocketClient owner name is absent")
                })?,
                admitted,
            )?;
        } else if let Some(owner) = module_branch_owner_address(parent)? {
            self.verify_registered_module_owner(&owner, admitted)?;
        }
        Ok(projected.with_branch(branch))
    }
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
            "provider_unavailable",
            "module branch has no canonical terminal",
        ));
    }
    let logical = render_segments(&segments[..segments.len() - 1]);
    MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &logical)
        .map(Some)
        .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))
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
        .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))
}

fn module_owner_address(
    module_at: &QualifiedAddress,
    capability: ModuleCapability,
) -> Result<MetadataAddress, ViewError> {
    let segments = module_at.segments();
    if capability.role() == ModuleRole::Common {
        return MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &module_at.logical_path())
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()));
    }
    let terminal = segments
        .last()
        .ok_or_else(|| ViewError::new("provider_unavailable", "module address has no terminal"))?;
    if terminal.kind() != NodeKind::Module || terminal.name().is_none() {
        return Err(ViewError::new(
            "provider_unavailable",
            "module address has no named module terminal",
        ));
    }
    let logical = render_segments(&segments[..segments.len() - 1]);
    MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &logical)
        .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))
}

pub(crate) fn module_branch_for_parent(
    parent: &QualifiedAddress,
    profile: PlatformProfile,
) -> Option<BranchRef> {
    let branch = if matches!(parent.segments(), [root] if root.kind() == NodeKind::Configuration) {
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
            "not_found",
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
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))
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

fn validate_view_filter(
    route: &LogicalTreeRoute,
    filter: &ViewFilter,
) -> Result<ModuleViewFilter, ViewError> {
    if route.reader() != LogicalReader::Module {
        return if filter.is_empty() {
            Ok(ModuleViewFilter::default())
        } else {
            Err(ViewError::new(
                "bad_value",
                "this logical projection does not support a filter",
            ))
        };
    }
    if route.module().is_none() {
        return if filter.is_empty() {
            Ok(ModuleViewFilter::default())
        } else {
            Err(ViewError::new(
                "bad_value",
                "module capability collections do not support a filter",
            ))
        };
    }
    let mut result = ModuleViewFilter::default();
    for (key, value) in filter.iter() {
        match key.as_str() {
            "context" => {
                let context = value.as_str().ok_or_else(|| {
                    ViewError::new("bad_value", "module filter.context must be a string")
                })?;
                if !MODULE_CONTEXTS.contains(&context) {
                    return Err(ViewError::new(
                        "bad_value",
                        format!("unsupported module execution context `{context}`"),
                    ));
                }
                result.context = Some(context.to_string());
            }
            "public" => {
                result.public = Some(value.as_bool().ok_or_else(|| {
                    ViewError::new("bad_value", "module filter.public must be boolean")
                })?);
            }
            _ => {
                return Err(ViewError::new(
                    "bad_value",
                    format!("unsupported module filter `{key}`"),
                ));
            }
        }
    }
    let (_, prefix_len) = route
        .module()
        .and_then(|capability| {
            module_prefix(route.at(), PlatformProfile::v8_3_27(), capability).ok()
        })
        .ok_or_else(|| ViewError::new("provider_unavailable", "module prefix is unavailable"))?;
    let suffix = &route.at().segments()[prefix_len..];
    if result.context.is_some()
        && !matches!(
            suffix.first().map(AddressSegment::kind),
            Some(NodeKind::Body | NodeKind::Method)
        )
    {
        return Err(ViewError::new(
            "bad_value",
            "module filter.context is supported only for Body and Method projections",
        ));
    }
    if result.public.is_some()
        && !matches!(
            suffix.first().map(AddressSegment::kind),
            Some(NodeKind::Method)
        )
    {
        return Err(ViewError::new(
            "bad_value",
            "module filter.public is supported only for Method projections",
        ));
    }
    Ok(result)
}

fn module_prefix(
    address: &QualifiedAddress,
    profile: PlatformProfile,
    capability: ModuleCapability,
) -> Result<(QualifiedAddress, usize), ViewError> {
    for length in 1..=address.segments().len() {
        let logical = render_segments(&address.segments()[..length]);
        let prefix = QualifiedAddress::parse(&format!("{}:{logical}", address.source_set()))
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?;
        if profile.module_capability(&prefix) == Some(capability) {
            return Ok((prefix, length));
        }
    }
    Err(ViewError::new(
        "provider_unavailable",
        "module prefix could not be reconstructed from the platform profile",
    ))
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
    let segments = module_at.segments();
    let logical = match capability.source_layout() {
        ModuleSourceLayout::Root => match capability.role() {
            ModuleRole::ManagedApplication => "ManagedApplicationModule".to_string(),
            ModuleRole::OrdinaryApplication => "OrdinaryApplicationModule".to_string(),
            ModuleRole::Session => "SessionModule".to_string(),
            ModuleRole::ExternalConnection => "ExternalConnectionModule".to_string(),
            _ => return Err(unsupported_module_layout(capability)),
        },
        ModuleSourceLayout::Common => format!(
            "CommonModule.{}.Module",
            required_segment_name(segments.first(), capability)?
        ),
        ModuleSourceLayout::Direct => {
            let owner = segments
                .first()
                .ok_or_else(|| unsupported_module_layout(capability))?;
            let role = match capability.role() {
                ModuleRole::Object => "ObjectModule",
                ModuleRole::Manager => "ManagerModule",
                ModuleRole::RecordSet => "RecordSetModule",
                ModuleRole::ValueManager => "ValueManagerModule",
                _ => return Err(unsupported_module_layout(capability)),
            };
            format!(
                "{}.{}.{role}",
                owner.kind().as_str(),
                required_segment_name(Some(owner), capability)?
            )
        }
        ModuleSourceLayout::CommonForm => format!(
            "CommonForm.{}.FormModule",
            required_segment_name(segments.first(), capability)?
        ),
        ModuleSourceLayout::CommonCommand => format!(
            "CommonCommand.{}.CommandModule",
            required_segment_name(segments.first(), capability)?
        ),
        ModuleSourceLayout::NestedForm | ModuleSourceLayout::NestedCommand => {
            let owner = segments
                .first()
                .ok_or_else(|| unsupported_module_layout(capability))?;
            let child = segments
                .get(1)
                .ok_or_else(|| unsupported_module_layout(capability))?;
            let terminal = if capability.source_layout() == ModuleSourceLayout::NestedForm {
                "FormModule"
            } else {
                "CommandModule"
            };
            format!(
                "{}.{}.{}.{}.{terminal}",
                owner.kind().as_str(),
                required_segment_name(Some(owner), capability)?,
                child.kind().as_str(),
                required_segment_name(Some(child), capability)?,
            )
        }
        ModuleSourceLayout::Service | ModuleSourceLayout::Bot => {
            let owner = segments
                .first()
                .ok_or_else(|| unsupported_module_layout(capability))?;
            format!(
                "{}.{}.Module",
                owner.kind().as_str(),
                required_segment_name(Some(owner), capability)?
            )
        }
        ModuleSourceLayout::WebSocketClient => {
            return Err(ViewError::new(
                "provider_unavailable",
                "WebSocketClient source layout is not specified for platform profile 8.3.27",
            ))
        }
    };
    MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &logical)
        .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))
}

fn required_segment_name(
    segment: Option<&AddressSegment>,
    capability: ModuleCapability,
) -> Result<&str, ViewError> {
    segment
        .and_then(AddressSegment::name)
        .ok_or_else(|| unsupported_module_layout(capability))
}

fn unsupported_module_layout(capability: ModuleCapability) -> ViewError {
    ViewError::new(
        "provider_unavailable",
        format!(
            "module source layout is unavailable for {}.{}",
            capability.owner_kind().as_str(),
            capability.role().as_str()
        ),
    )
}

fn common_module_properties(bytes: &[u8]) -> Result<CommonModuleProperties, ViewError> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        ViewError::new(
            "provider_unavailable",
            "common module descriptor is not UTF-8",
        )
    })?;
    let document = roxmltree::Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?;
    let root = document.root_element();
    let boolean = |name| -> Result<bool, ViewError> {
        let raw = xml_descendant_text(root, name).ok_or_else(|| {
            ViewError::new(
                "provider_unavailable",
                format!("common module descriptor has no {name} property"),
            )
        })?;
        match raw {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(ViewError::new(
                "provider_unavailable",
                format!("common module {name} property is not boolean"),
            )),
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
        return_values_reuse: xml_descendant_text(root, "ReturnValuesReuse")
            .ok_or_else(|| {
                ViewError::new(
                    "provider_unavailable",
                    "common module descriptor has no ReturnValuesReuse property",
                )
            })?
            .to_string(),
    })
}

fn xml_descendant_text<'a>(root: roxmltree::Node<'a, 'a>, name: &str) -> Option<&'a str> {
    root.descendants()
        .find(|node| node.is_element() && node.tag_name().name() == name)
        .and_then(|node| node.text())
        .map(str::trim)
}

fn form_event_binding(
    owner: FormBindingOwner,
    at: &str,
    event: &FormInfoEvent,
) -> FormEventBindingInput {
    FormEventBindingInput::property(
        owner,
        at,
        &event.name,
        &event.handler,
        event.call_type.as_deref(),
    )
}

struct FormSemanticInputs {
    owners: Vec<FormEventOwnerInput>,
    bindings: Vec<FormEventBindingInput>,
}

fn form_semantic_inputs(form_at: &str, data: &FormInfoData) -> FormSemanticInputs {
    let mut inputs = FormSemanticInputs {
        owners: vec![FormEventOwnerInput::new(FormBindingOwner::Form, form_at)],
        bindings: data
            .events
            .iter()
            .map(|event| form_event_binding(FormBindingOwner::Form, form_at, event))
            .collect(),
    };
    collect_element_semantics(form_at, &data.elements, false, &mut inputs);
    for command in &data.commands {
        let at = format!("{form_at}.Command.{}", command.name);
        inputs
            .owners
            .push(FormEventOwnerInput::new(FormBindingOwner::Command, &at));
        inputs.bindings.extend(
            command
                .actions
                .iter()
                .map(|event| form_event_binding(FormBindingOwner::Command, &at, event)),
        );
    }
    inputs
}

fn collect_element_semantics(
    parent_at: &str,
    elements: &[FormInfoElement],
    parent_is_table: bool,
    inputs: &mut FormSemanticInputs,
) {
    for element in elements {
        let at = format!("{parent_at}.Item.{}", element.name);
        let is_table = element.event_kind == Some(FormElementKind::Table);
        if let Some(kind) = element.event_kind {
            let owner = if parent_is_table {
                FormBindingOwner::Column(kind)
            } else if is_table {
                FormBindingOwner::Table
            } else {
                FormBindingOwner::Element(kind)
            };
            let mut semantic_owner = FormEventOwnerInput::new(owner, &at);
            if is_table {
                if let Some(data_path) = element
                    .binding
                    .as_ref()
                    .filter(|binding| binding.kind == "dataPath")
                    .map(|binding| binding.target.as_str())
                {
                    semantic_owner = semantic_owner.with_data_path(data_path);
                }
            }
            inputs.owners.push(semantic_owner);
            inputs.bindings.extend(
                element
                    .events
                    .iter()
                    .map(|event| form_event_binding(owner, &at, event)),
            );
        }
        collect_element_semantics(&at, &element.children, is_table, inputs);
    }
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

fn module_projection_view(
    requested: &QualifiedAddress,
    prefix_len: usize,
    projections: &ModuleProjectionSet,
    filter: &ModuleViewFilter,
) -> Result<NodeViewData, ViewError> {
    let suffix = &requested.segments()[prefix_len..];
    if suffix.is_empty() {
        let summary = projections.summary();
        let props = serde_json::to_value(&summary.props)
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?
            .as_object()
            .cloned()
            .ok_or_else(|| ViewError::new("provider_unavailable", "module props are invalid"))?;
        let branches = summary
            .branches
            .iter()
            .map(|branch| BranchRef::new(&branch.at, branch.count))
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
            "not_found",
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
                .filter(|method| module_method_matches_filter(method, filter))
                .map(method_node_value)
                .collect(),
        ),
        (NodeKind::Method, Some(name), None) => projections
            .methods()
            .iter()
            .find(|method| method.name == name && module_method_matches_filter(method, filter))
            .map(method_node)
            .map(NodeViewData::Node)
            .ok_or_else(|| ViewError::new("not_found", format!("method `{name}` was not found"))),
        (NodeKind::Method, Some(name), Some(detail))
            if detail.name().is_none() && detail.kind() == NodeKind::Body =>
        {
            let method = find_method(projections, name)?;
            if !module_method_matches_filter(method, filter) {
                return Err(ViewError::new(
                    "not_found",
                    format!("method `{name}` does not match the requested filter"),
                ));
            }
            let items = projections
                .body()
                .iter()
                .filter(|line| {
                    line.line >= method.body_from_line && line.line <= method.body_to_line
                })
                .filter(|line| module_body_line_matches_filter(line.line, projections, filter))
                .map(|line| json!({"line": line.line, "text": line.text}))
                .collect();
            module_collection(requested, NodeKind::Body, items)
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
                .or_else(|_| {
                    suffix
                        .last()
                        .and_then(AddressSegment::name)
                        .ok_or_else(|| {
                            crate::domain::module_projection::ProjectionError::not_found(
                                "region name is missing",
                            )
                        })
                        .and_then(|name| projections.region(name))
                })
                .map_err(|error| ViewError::new("not_found", error.to_string()))?;
            Ok(NodeViewData::Node(region_node(requested, region)))
        }
        (NodeKind::Interface, None, None) => module_collection(
            requested,
            NodeKind::Interface,
            projections
                .interfaces()
                .iter()
                .map(|interface| {
                    node_value(
                        &interface.at,
                        NodeKind::Interface,
                        interface.interface.as_str(),
                        Map::from_iter([
                            ("interface".to_string(), json!(interface.interface)),
                            ("methods".to_string(), json!(interface.methods)),
                        ]),
                        Vec::new(),
                    )
                })
                .collect(),
        ),
        (NodeKind::Interface, Some(name), None) => projections
            .interfaces()
            .iter()
            .find(|interface| interface.interface.as_str() == name)
            .map(|interface| {
                NodeViewData::Node(NodeView::new(
                    &interface.at,
                    NodeKind::Interface.as_str(),
                    name,
                    Map::from_iter([
                        ("interface".to_string(), json!(interface.interface)),
                        ("methods".to_string(), json!(interface.methods)),
                    ]),
                ))
            })
            .ok_or_else(|| {
                ViewError::new("not_found", format!("interface `{name}` was not found"))
            }),
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
            .ok_or_else(|| ViewError::new("not_found", format!("event `{name}` was not found"))),
        (NodeKind::Compilation, None, None) => module_collection(
            requested,
            NodeKind::Compilation,
            projections
                .compilation()
                .iter()
                .map(|range| serde_json::to_value(range).unwrap_or(Value::Null))
                .collect(),
        ),
        (NodeKind::Body, None, None) => module_collection(
            requested,
            NodeKind::Body,
            projections
                .body()
                .iter()
                .filter(|line| module_body_line_matches_filter(line.line, projections, filter))
                .map(|line| json!({"line": line.line, "text": line.text}))
                .collect(),
        ),
        _ => Err(ViewError::new(
            "not_found",
            "module projection suffix is not available",
        )),
    }
}

fn module_method_matches_filter(method: &MethodProjection, filter: &ModuleViewFilter) -> bool {
    filter.public.is_none_or(|public| method.export == public)
        && filter.context.as_deref().is_none_or(|context| {
            method
                .compile
                .contexts
                .iter()
                .any(|candidate| module_context_matches(context, candidate))
        })
}

fn module_context_matches(requested: &str, actual: &str) -> bool {
    match requested {
        "client" => actual.to_ascii_lowercase().contains("client"),
        "server" => actual == "server" || actual.ends_with("Server"),
        exact => actual == exact,
    }
}

fn module_body_line_matches_filter(
    line: usize,
    projections: &ModuleProjectionSet,
    filter: &ModuleViewFilter,
) -> bool {
    let Some(context) = filter.context.as_deref() else {
        return true;
    };
    projections.methods().iter().any(|method| {
        let directive_lines = usize::from(method.compile.directive.is_some());
        let from = method.body_from_line.saturating_sub(1 + directive_lines);
        let to = method
            .body_to_line
            .saturating_add(1)
            .max(method.body_from_line);
        line >= from
            && line <= to
            && method
                .compile
                .contexts
                .iter()
                .any(|candidate| module_context_matches(context, candidate))
            && projections
                .compilation()
                .iter()
                .filter(|range| line >= range.from_line && line <= range.to_line)
                .all(|range| {
                    range
                        .contexts
                        .iter()
                        .any(|candidate| module_context_matches(context, candidate))
                })
    })
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

fn find_method<'a>(
    projections: &'a ModuleProjectionSet,
    name: &str,
) -> Result<&'a MethodProjection, ViewError> {
    projections
        .methods()
        .iter()
        .find(|method| method.name == name)
        .ok_or_else(|| ViewError::new("not_found", format!("method `{name}` was not found")))
}

fn method_node(method: &MethodProjection) -> NodeView {
    let mut props = Map::from_iter([
        ("signature".to_string(), json!(method.signature)),
        ("methodKind".to_string(), json!(method.method_kind)),
        ("export".to_string(), json!(method.export)),
        ("compile".to_string(), json!(method.compile)),
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
    NodeView::new(&method.at, NodeKind::Method.as_str(), &method.name, props).with_branches(vec![
        BranchRef::new(
            format!("{}.Compilation", method.at),
            method.compilation_count,
        ),
        BranchRef::new(
            format!("{}.Body", method.at),
            if method.body_to_line < method.body_from_line {
                0
            } else {
                method.body_to_line - method.body_from_line + 1
            },
        ),
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
    let props = Map::from_iter([
        ("eventId".to_string(), json!(event.event_id)),
        ("state".to_string(), json!(event.state)),
        ("signature".to_string(), json!(event.signature)),
        ("contexts".to_string(), json!(event.contexts)),
        ("binding".to_string(), json!(event.binding)),
        ("handler".to_string(), json!(event.handler)),
        ("handlerEn".to_string(), json!(event.handler_en)),
        (
            "implementationAt".to_string(),
            json!(event.implementation_at),
        ),
        ("callType".to_string(), json!(event.call_type)),
    ]);
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
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?,
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

#[cfg(test)]
use crate::infrastructure::v13_read_projection::project_known_suffix;
use crate::infrastructure::v13_read_projection::project_typed_payload;

#[cfg(test)]
pub(crate) mod tests;
