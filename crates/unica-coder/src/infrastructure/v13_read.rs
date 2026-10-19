use crate::application::ports::{MetadataChildProfile, MetadataTemplateType};
use crate::application::v13::resolve::ResolvedLines;
use crate::application::v13::source_text::{admit_text, ModuleText};
use crate::application::v13::view::{ViewError, ViewReadAuthority, ViewSourceSnapshot};
use crate::domain::address::{AddressSegment, NodeKind, QualifiedAddress};
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::metadata::{MetaDiagnosticSeverity, MetadataKind};
use crate::domain::module_projection::{
    CommonModuleProperties, EventProjection, MethodProjection, ModuleProjectionSet,
    RegionProjection,
};
use crate::domain::node_view::{
    BranchRef, CollectionView, NodeView, NodeViewData, SourceText, TextView,
};
use crate::domain::platform_profile::{
    ModuleCapability, ModuleRole, ModuleSourceLayout, PlatformProfile,
};
use crate::domain::project_sources::SourceSetKind;
use crate::domain::refusal::{RefusalCode, RefusalDetail};
#[cfg(test)]
use crate::domain::source_target::SourceTarget;
use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use crate::infrastructure::bsl_module_projection::{
    project_module, FormEventBindingInput, ModuleProjectionRequest, PlatformEventWriteCapability,
};
use crate::infrastructure::event_projection::{
    form_semantic_inputs, project_platform_event_from_projection,
    project_property_events_from_projection,
};
use crate::infrastructure::logical_event_source::{
    module_prefix as shared_module_prefix, module_source_address as shared_module_source_address,
};
use crate::infrastructure::logical_event_source::{resolve_event_source, LogicalEventSource};
use crate::infrastructure::logical_tree::{route_logical_address, LogicalReader, LogicalTreeRoute};
use crate::infrastructure::native_operations::form::{FormEventEvidence, FormInfoData};
#[cfg(test)]
use crate::infrastructure::platform::filesystem::RetainedDirectoryCapability;
use crate::infrastructure::platform_xml_owner::PlatformXmlSourceSetOwnerEvidence;
#[cfg(test)]
use crate::infrastructure::platform_xml_source_targets::{
    resolve_platform_xml_target, TargetKindPolicy,
};
use crate::infrastructure::v13_large_configuration::{RegistrationCache, RegistrationIndex};
use crate::infrastructure::v13_read_port::ProviderReadAuthority;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

#[cfg(test)]
std::thread_local! {
    static COMPLETED_VIEW_PROJECTION_PROBE_FOR_TEST: std::cell::RefCell<Option<(String, Vec<String>)>> = const { std::cell::RefCell::new(None) };
}

/// Counts successful complete typed projections of one address on this thread.
#[cfg(test)]
pub(crate) struct CompletedViewProjectionProbeForTest {
    previous: Option<(String, Vec<String>)>,
}

#[cfg(test)]
impl CompletedViewProjectionProbeForTest {
    pub(crate) fn start(at: &str) -> Self {
        Self {
            previous: COMPLETED_VIEW_PROJECTION_PROBE_FOR_TEST
                .with(|slot| slot.replace(Some((at.to_owned(), Vec::new())))),
        }
    }

    pub(crate) fn revisions(&self) -> Vec<String> {
        COMPLETED_VIEW_PROJECTION_PROBE_FOR_TEST.with(|slot| {
            slot.borrow()
                .as_ref()
                .expect("projection observer is active")
                .1
                .clone()
        })
    }
}

#[cfg(test)]
impl Drop for CompletedViewProjectionProbeForTest {
    fn drop(&mut self) {
        COMPLETED_VIEW_PROJECTION_PROBE_FOR_TEST.with(|slot| {
            slot.replace(self.previous.take());
        });
    }
}

#[cfg(test)]
fn observe_completed_view_projection_for_test(at: &QualifiedAddress, revision: &str) {
    COMPLETED_VIEW_PROJECTION_PROBE_FOR_TEST.with(|slot| {
        if let Some((target, revisions)) = slot.borrow_mut().as_mut() {
            if target == &at.to_string() {
                revisions.push(revision.to_owned());
            }
        }
    });
}

const MAX_MODULE_BYTES: usize = 8 * 1024 * 1024;
/// Logical read adapter over one actor-issued source capability. Its read
/// identity is fixed at admission; the adapter never snapshots the tree.
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

impl<'a> LogicalViewReadAuthority<'a> {
    #[cfg(test)]
    pub(crate) fn new(
        cancellation: &'a CancellationToken,
        source_set: impl Into<String>,
        source_set_identity: impl Into<String>,
        source_set_kind: SourceSetKind,
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
            ),
            profile,
            ProviderDeadline::no_deadline(),
        )
    }

    /// The exact platform profile the reader serves; a logical reader exists
    /// only for such a profile.
    pub(crate) const fn platform_profile(&self) -> PlatformProfile {
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

    pub(crate) fn with_read_authority_and_registration_cache(
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

    pub(crate) fn object_borrowing(
        &self,
        at: &QualifiedAddress,
    ) -> Result<Option<crate::infrastructure::native_operations::meta::MetaBorrowing>, ViewError>
    {
        if self.read.source_set_kind() != SourceSetKind::Extension
            || at.segments().len() != 1
            || at.segments()[0].name().is_none()
            || !at.segments()[0].kind().is_metadata_kind()
        {
            return Ok(None);
        }
        let target = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            at.to_string().split_once(':').expect("qualified address").1,
        )
        .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string()))?;
        let admitted = ViewSourceSnapshot {
            source_set_identity: self.read.source_set_identity().to_string(),
            revision: self.exact_revision()?,
        };
        self.verify_registered_owner(&target, &admitted)?;
        crate::infrastructure::native_operations::meta::parse_meta_borrowing(
            &self.read.metadata_descriptor(&target)?,
        )
        .map(Some)
        .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))
    }

    /// Resolve only registered, readable objects under this retained source.
    /// A matching filename or name alone is not parent identity evidence.
    pub(crate) fn borrowed_parent_matches(
        &self,
        parent_uuid: &str,
        kind: &str,
    ) -> Result<Vec<String>, ViewError> {
        let admitted = ViewSourceSnapshot {
            source_set_identity: self.read.source_set_identity().to_string(),
            revision: self.exact_revision()?,
        };
        let payload = self
            .read
            .configuration_payload_with_checkpoint(&mut || self.read_checkpoint())?;
        let mut matches = Vec::new();
        for item in payload
            .get("registeredObjects")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if item.get("kind").and_then(Value::as_str) != Some(kind) {
                continue;
            }
            self.read_checkpoint()?;
            let name = item.get("name").and_then(Value::as_str).ok_or_else(|| {
                ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    "registered parent has no name",
                )
            })?;
            let target =
                MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &format!("{kind}.{name}"))
                    .map_err(|error| {
                        ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                    })?;
            self.verify_registered_owner(&target, &admitted)?;
            let bytes = self.read.metadata_descriptor(&target)?;
            let text = std::str::from_utf8(&bytes).map_err(|_| {
                ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    "parent descriptor is not UTF-8",
                )
            })?;
            let doc = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).map_err(
                |error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string()),
            )?;
            let uuid = doc
                .root_element()
                .children()
                .find(|node| node.is_element())
                .and_then(|node| node.attribute("uuid"));
            if uuid.is_some_and(|uuid| uuid.eq_ignore_ascii_case(parent_uuid)) {
                let at = format!("{}:{}", self.read.source_set(), target.as_str());
                let address = QualifiedAddress::parse(&at).map_err(|error| {
                    ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                })?;
                // A valid registered descriptor must also be readable through the public reader.
                self.read_exact(&address, &admitted)?;
                matches.push(at);
            }
        }
        if self.exact_revision()? != admitted.revision {
            return Err(ViewError::new(
                RefusalCode::StaleRevision,
                "parent source changed during identity resolution",
            ));
        }
        Ok(matches)
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
    pub(crate) fn existing_export_path(
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
        if requested_path.is_some_and(|requested| requested != path) {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                "the requested file is not the logical node's source",
            ));
        }
        self.read_checkpoint()?;
        Ok(path)
    }

    /// Resolve names the node's kind and its own source lines. In a BSL
    /// module `Method` spans the whole declaration through its closing line,
    /// and its `Body` the lines `view` returns as its text: the doc comment
    /// above the method through the closing line. The module `Body` counts the
    /// file stream like the view fast path, without projecting the module.
    pub(crate) fn source_lines(
        &self,
        at: &QualifiedAddress,
        admitted: &ViewSourceSnapshot,
    ) -> Result<(String, ResolvedLines), ViewError> {
        let route = route_logical_address(at, self.profile)
            .map_err(|error| ViewError::new(RefusalCode::NotFound, error.to_string()))?;
        let capability = route
            .module()
            .filter(|_| route.reader() == LogicalReader::Module);
        let Some(capability) = capability else {
            let projected = self.read_exact(at, admitted)?;
            return Ok((projected.kind().to_string(), ResolvedLines::NotLineBased));
        };
        let (module_at, prefix_len) = module_prefix(at, self.profile, capability)?;
        let suffix = &at.segments()[prefix_len..];
        let is_named =
            |segment: &AddressSegment, kind| segment.kind() == kind && segment.name().is_some();
        let is_branch =
            |segment: &AddressSegment, kind| segment.kind() == kind && segment.name().is_none();
        let module_body = matches!(suffix, [body] if is_branch(body, NodeKind::Body));
        let method = matches!(suffix, [method] if is_named(method, NodeKind::Method));
        let method_body = matches!(
            suffix,
            [method, body] if is_named(method, NodeKind::Method) && is_branch(body, NodeKind::Body)
        );
        let region = !suffix.is_empty()
            && suffix
                .iter()
                .all(|segment| is_named(segment, NodeKind::Region));
        if !(module_body || method || method_body || region) {
            let projected = self.read_exact(at, admitted)?;
            return Ok((projected.kind().to_string(), ResolvedLines::NotLineBased));
        }
        self.read_checkpoint()?;
        if admitted.source_set_identity != self.read.source_set_identity()
            || admitted.revision != self.exact_revision()?
        {
            return Err(ViewError::new(
                RefusalCode::StaleCursor,
                "source revision changed before the source line read",
            ));
        }
        self.verify_module_owner(&module_at, capability, admitted)?;
        if capability.role() == ModuleRole::WebSocketClient {
            return Err(ViewError::detailed(
                RefusalDetail::ProviderAbsent,
                "WebSocketClient source layout is not specified for platform profile 8.3.27",
            ));
        }
        if module_body {
            let target = module_source_address(&module_at, capability)?;
            let count = self
                .read
                .module_line_count(&target, || self.read_checkpoint())?
                .ok_or_else(|| {
                    ViewError::new(
                        RefusalCode::NotFound,
                        "the BSL module source file does not exist",
                    )
                })?;
            return Ok((
                NodeKind::Body.as_str().to_string(),
                selected_lines(at, line_range(1, count))?,
            ));
        }
        let projections = self.module_projection(&module_at, capability, admitted)?;
        let (kind, lines) = if region {
            let region = projections
                .region(&at.to_string())
                .map_err(|error| ViewError::new(RefusalCode::NotFound, error.to_string()))?;
            (
                NodeKind::Region,
                region.end_line.map_or(ResolvedLines::NotLineBased, |end| {
                    line_range(region.line, end)
                }),
            )
        } else {
            let name = suffix[0].name().unwrap_or_default();
            let method = find_method(&projections, name)?;
            if method_body {
                (
                    NodeKind::Body,
                    line_range(method.text_from_line, method.to_line),
                )
            } else {
                (
                    NodeKind::Method,
                    line_range(method.from_line, method.to_line),
                )
            }
        };
        self.read_checkpoint()?;
        Ok((kind.as_str().to_string(), selected_lines(at, lines)?))
    }

    /// Inspect containment and source bytes in one retained projection, with
    /// one final source identity/cancellation fence for the entire answer.
    fn with_body_line_projection<T>(
        &self,
        module: &QualifiedAddress,
        from: usize,
        to: usize,
        inspect: impl FnOnce(QualifiedAddress, &ModuleProjectionSet) -> T,
    ) -> Result<T, ViewError> {
        self.read_checkpoint()?;
        let admitted = self.snapshot(module)?;
        let route = route_logical_address(module, self.profile)
            .map_err(|error| ViewError::new(RefusalCode::NotFound, error.to_string()))?;
        let capability = route.module().ok_or_else(|| {
            ViewError::new(
                RefusalCode::BadValue,
                "search hit does not name a BSL module",
            )
        })?;
        let (module_at, _) = module_prefix(module, self.profile, capability)?;
        self.verify_module_owner(&module_at, capability, &admitted)?;
        let projections = self.module_projection(&module_at, capability, &admitted)?;
        let body = projections
            .methods()
            .iter()
            .filter(|method| from >= method.text_from_line && to <= method.to_line)
            .min_by_key(|method| method.to_line - method.text_from_line)
            .map_or_else(
                || format!("{module_at}.Body"),
                |method| format!("{}.Body", method.at),
            );
        let body = QualifiedAddress::parse(&body)
            .map_err(|error| ViewError::new(RefusalCode::BadValue, error.to_string()))?;
        let result = inspect(body, &projections);
        self.read_checkpoint()?;
        if self.snapshot(module)? != admitted {
            return Err(ViewError::new(
                RefusalCode::ConcurrentChange,
                "source changed while search hit boundaries were read",
            ));
        }
        Ok(result)
    }

    /// Resolves semantic containment from the same projection as Method
    /// views. Provider symbols are never accepted as boundary evidence.
    pub(crate) fn narrowest_body_for_lines(
        &self,
        module: &QualifiedAddress,
        from: usize,
        to: usize,
    ) -> Result<QualifiedAddress, ViewError> {
        self.with_body_line_projection(module, from, to, |body, _| body)
    }

    pub(crate) fn module_address_for_source_target(
        &self,
        target: &MetadataAddress,
    ) -> Option<QualifiedAddress> {
        let path = crate::infrastructure::logical_event_source::module_relative(
            target,
            self.read.source_set_kind(),
        )
        .ok()?;
        crate::infrastructure::logical_event_source::module_address_for_relative(
            self.read.source_set(),
            self.read.source_set_kind(),
            self.profile,
            &path,
        )
    }

    pub(crate) fn body_for_search_hit(
        &self,
        module: &QualifiedAddress,
        from: usize,
        to: usize,
        snippet: &str,
    ) -> Result<Option<QualifiedAddress>, ViewError> {
        self.with_body_line_projection(module, from, to, |body, projection| {
            let source = projection.source_text();
            if from == 0 || to < from || to > source.lines().count() {
                return None;
            }
            let start = source
                .split_inclusive('\n')
                .take(from - 1)
                .map(str::len)
                .sum::<usize>();
            let end = source
                .split_inclusive('\n')
                .take(to)
                .map(str::len)
                .sum::<usize>();
            source[start..end].contains(snippet).then_some(body)
        })
    }

    #[cfg(test)]
    pub(crate) fn module_source_read_count(&self, target: &str) -> usize {
        self.read.module_source_read_count(target)
    }

    #[cfg(test)]
    pub(crate) fn configuration_payload_read_count(&self) -> usize {
        self.read.configuration_payload_read_count()
    }

    #[cfg(test)]
    pub(crate) fn metadata_descriptor_read_count(&self, target: &str) -> usize {
        self.read.metadata_descriptor_read_count(target)
    }

    #[cfg(test)]
    pub(crate) fn support_state_read_count(&self) -> usize {
        self.read.support_state_read_count()
    }

    #[cfg(test)]
    pub(crate) const fn source_set_kind_for_test(&self) -> SourceSetKind {
        self.read.source_set_kind()
    }

    #[cfg(test)]
    pub(crate) const fn deadline_for_test(&self) -> ProviderDeadline {
        self.deadline
    }

    pub(crate) fn diagnostic_mapping(
        &self,
        at: &QualifiedAddress,
        workspace: &crate::domain::workspace::WorkspaceContext,
    ) -> Result<crate::infrastructure::diagnostics::CanonicalModuleDiagnosticMapping, ViewError>
    {
        use crate::domain::diagnostics::DiagnosticContext;
        use crate::domain::project_sources::{ProjectSourceSet, SourceFormat, SourceSetState};
        use crate::domain::source_roots::ResolvedSourceRoot;
        use crate::domain::source_target::{ResolvedTarget, TargetKind};
        let admitted = self.snapshot(at)?;
        let canonical = self.canonical_address(at, &admitted)?;
        let capability = self
            .profile
            .module_prefix_capability(&canonical)
            .ok_or_else(|| {
                ViewError::new(
                    RefusalCode::BadValue,
                    "BSL diagnostics requires a module or its body",
                )
            })?;
        let (module_at, _) = shared_module_prefix(&canonical, self.profile, capability)
            .map_err(|error| ViewError::new(RefusalCode::BadValue, error))?;
        // Registration, owner identity, format and module containment are proved
        // by the same reader as view, using this invocation's retained root.
        self.read_exact(&module_at, &admitted)?;
        let target = module_source_address(&module_at, capability)?;
        let relative = self.read.module_export_path(&target)?;
        let source_set = self.read.source_set().to_owned();
        let context = DiagnosticContext::new(
            workspace.clone(),
            ProjectSourceSet {
                name: source_set.clone(),
                kind: self.read.source_set_kind(),
                path: self.read.root_path().to_string_lossy().into_owned(),
                source_format: SourceFormat::PlatformXml,
                source_state: SourceSetState::Supported,
                format_evidence: Vec::new(),
                format_probe_error: None,
            },
            ResolvedSourceRoot {
                source_set: Some(source_set.clone()),
                path: self.read.root_path().to_path_buf(),
            },
            ResolvedTarget {
                source_set,
                metadata_path: Some(target),
                target_kind: TargetKind::Module,
            },
        );
        crate::infrastructure::diagnostics::CanonicalModuleDiagnosticMapping::from_proven_module(
            context,
            relative.into(),
        )
        .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))
    }

    pub(crate) fn validate_subsystem(
        &self,
        at: &QualifiedAddress,
    ) -> Result<crate::application::v13::check::NativeCheckOutcome, ViewError> {
        let admitted = self.snapshot(at)?;
        let canonical = self.canonical_address(at, &admitted)?;
        let route = route_logical_address(&canonical, self.profile)
            .map_err(|error| ViewError::new(RefusalCode::BadValue, error.to_string()))?;
        let target = route.reader_metadata_path().ok_or_else(|| {
            ViewError::new(RefusalCode::BadValue, "subsystem check has no descriptor")
        })?;
        self.verify_registered_owner(target, &admitted)?;
        self.read
            .validate_subsystem(&canonical, &mut || self.read_checkpoint())
    }

    /// Proves a DCS template through the same registration and descriptor
    /// authority as view, then reads its body through the retained source root.
    pub(crate) fn dcs_validation_input(
        &self,
        at: &QualifiedAddress,
    ) -> Result<crate::infrastructure::v13_read_port::TemplateValidationInput, ViewError> {
        let admitted = self.snapshot(at)?;
        let canonical = self.canonical_address(at, &admitted)?;
        let target =
            crate::infrastructure::native_operations::dcs::typed_dcs_reader_target(&canonical)
                .ok_or_else(|| {
                    ViewError::new(RefusalCode::BadValue, "DCS validation requires a template")
                })?;
        self.verify_registered_owner(&target, &admitted)?;
        if !matches!(
            self.read.metadata_child_profile(&target)?,
            MetadataChildProfile::Template(MetadataTemplateType::DataCompositionSchema)
        ) {
            return Err(ViewError::new(
                RefusalCode::BadValue,
                "template is not a data composition schema",
            ));
        }
        self.read_checkpoint()?;
        let input = self
            .read
            .dcs_validation_input(&target, &mut || self.read_checkpoint())?;
        self.read_checkpoint()?;
        Ok(input)
    }

    /// Proves a MXL template through the same registration and descriptor
    /// authority as view, then reads its body through the retained source root.
    pub(crate) fn mxl_validation_input(
        &self,
        at: &QualifiedAddress,
    ) -> Result<crate::infrastructure::v13_read_port::TemplateValidationInput, ViewError> {
        let admitted = self.snapshot(at)?;
        let canonical = self.canonical_address(at, &admitted)?;
        let target =
            crate::infrastructure::native_operations::mxl::typed_mxl_reader_target(&canonical)
                .ok_or_else(|| {
                    ViewError::new(RefusalCode::BadValue, "MXL validation requires a template")
                })?;
        self.verify_registered_owner(&target, &admitted)?;
        if !matches!(
            self.read.metadata_child_profile(&target)?,
            MetadataChildProfile::Template(MetadataTemplateType::SpreadsheetDocument)
        ) {
            return Err(ViewError::new(
                RefusalCode::BadValue,
                "template is not a spreadsheet document",
            ));
        }
        self.read_checkpoint()?;
        let input = self
            .read
            .mxl_validation_input(&target, &mut || self.read_checkpoint())?;
        self.read_checkpoint()?;
        Ok(input)
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
                let key = TypedPayloadCacheKey {
                    revision: RevisionCacheKey {
                        source_set_identity: admitted.source_set_identity.clone(),
                        revision: admitted.revision.clone(),
                    },
                    reader: route.reader(),
                    target: target.as_str().to_string(),
                    // Кэш ключуется адресом предмета, а содержимое ячеек
                    // читается тем же предметом другим разбором. Без этого
                    // различия на запрос с текстом вернулась бы нагрузка
                    // без текста, отложенная структурным чтением.
                    detail: match route.reader() {
                        LogicalReader::Xdto => {
                            named_segment(route.at(), NodeKind::Type).map(str::to_string)
                        }
                        LogicalReader::Mxl => mxl_content_requested(route)
                            .then(|| NodeKind::Body.as_str().to_string()),
                        // Вложенная подсистема — другой файл под той же
                        // корневой целью.
                        LogicalReader::Subsystem | LogicalReader::Interface => {
                            (!route.nested_subsystems().is_empty())
                                .then(|| route.nested_subsystems().join("."))
                        }
                        _ => None,
                    },
                };
                let mut cache = self.typed_payloads.lock().map_err(|_| {
                    ViewError::detailed(
                        RefusalDetail::CachePoisoned,
                        "typed payload cache is poisoned",
                    )
                })?;
                if let Some(payload) = cache.get(&key) {
                    return Ok(payload.as_ref().clone());
                }
                let payload = Arc::new(self.uncached_typed_payload_for(route, admitted)?);
                cache.insert(key, Arc::clone(&payload));
                return Ok(payload.as_ref().clone());
            }
        }
        self.uncached_typed_payload_for(route, admitted)
    }

    fn uncached_typed_payload_for(
        &self,
        route: &LogicalTreeRoute,
        admitted: &ViewSourceSnapshot,
    ) -> Result<Value, ViewError> {
        if route.reader() == LogicalReader::Configuration {
            // Корень доказывает только `Configuration.xml`: описание
            // объекта проверяют прямое чтение и ветка его вида.
            let payload = self.configuration_payload(admitted)?;
            return Ok(payload.as_ref().clone());
        }
        if route.reader() == LogicalReader::Metadata {
            return self.metadata_payload(route, admitted);
        }
        if route.reader() == LogicalReader::Form {
            return self.read.form_payload(
                route.reader_metadata_path().ok_or_else(|| {
                    ViewError::new(
                        RefusalCode::ProviderUnavailable,
                        "form route has no typed target",
                    )
                })?,
                &mut || self.read_checkpoint(),
            );
        }
        if route.reader() == LogicalReader::Dcs {
            return self.read.dcs_payload(
                route.reader_metadata_path().ok_or_else(|| {
                    ViewError::new(
                        RefusalCode::ProviderUnavailable,
                        "DCS route has no typed target",
                    )
                })?,
                &mut || self.read_checkpoint(),
            );
        }
        if route.reader() == LogicalReader::Role {
            return self.read.role_payload(
                route.reader_metadata_path().ok_or_else(|| {
                    ViewError::new(
                        RefusalCode::ProviderUnavailable,
                        "role route has no typed target",
                    )
                })?,
                &mut || self.read_checkpoint(),
            );
        }
        if matches!(
            route.reader(),
            LogicalReader::Subsystem | LogicalReader::Interface
        ) {
            return self.read.subsystem_payload(
                route.reader_metadata_path().ok_or_else(|| {
                    ViewError::new(
                        RefusalCode::ProviderUnavailable,
                        "subsystem route has no typed target",
                    )
                })?,
                route.nested_subsystems(),
                &mut || self.read_checkpoint(),
            );
        }
        if route.reader() == LogicalReader::Mxl {
            return self.read.mxl_payload(
                route.reader_metadata_path().ok_or_else(|| {
                    ViewError::new(
                        RefusalCode::ProviderUnavailable,
                        "MXL route has no typed target",
                    )
                })?,
                mxl_content_requested(route),
                &mut || self.read_checkpoint(),
            );
        }
        if route.reader() == LogicalReader::Xdto {
            return self.read.xdto_payload(
                route.reader_metadata_path().ok_or_else(|| {
                    ViewError::new(
                        RefusalCode::ProviderUnavailable,
                        "XDTO route has no typed target",
                    )
                })?,
                named_segment(route.at(), NodeKind::Type),
            );
        }
        Err(ViewError::new(
            RefusalCode::ProviderUnavailable,
            "logical reader requires its dedicated retained adapter",
        ))
    }

    fn configuration_payload(
        &self,
        admitted: &ViewSourceSnapshot,
    ) -> Result<Arc<Value>, ViewError> {
        if admitted.source_set_identity != self.read.source_set_identity() {
            return Err(ViewError::new(
                RefusalCode::StaleCursor,
                "configuration payload belongs to another source identity",
            ));
        }
        let key = RevisionCacheKey {
            source_set_identity: admitted.source_set_identity.clone(),
            revision: admitted.revision.clone(),
        };
        let mut cache = self.configuration_payloads.lock().map_err(|_| {
            ViewError::detailed(
                RefusalDetail::CachePoisoned,
                "configuration cache is poisoned",
            )
        })?;
        if let Some(payload) = cache.get(&key) {
            return Ok(Arc::clone(payload));
        }
        let mut checkpoint = || self.read_checkpoint();
        let payload = self
            .read
            .configuration_payload_with_checkpoint(&mut checkpoint)?;
        cache.insert(key, Arc::clone(&payload));
        Ok(payload)
    }

    fn metadata_payload(
        &self,
        route: &LogicalTreeRoute,
        admitted: &ViewSourceSnapshot,
    ) -> Result<Value, ViewError> {
        if route.at().form_metadata_owner().is_some() {
            let target = route.reader_metadata_path().ok_or_else(|| {
                ViewError::new(RefusalCode::NotFound, "form metadata route has no owner")
            })?;
            self.verify_registered_owner(target, admitted)?;
            return self
                .read
                .form_metadata_payload(target, &mut || self.read_checkpoint());
        }
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
            ViewError::new(
                RefusalCode::NotFound,
                "metadata address has no typed reader target",
            )
        })?;
        self.ensure_owner_registered(target, admitted)?;
        if let Some(payload) = self
            .read
            .external_metadata_payload(target, &mut || self.read_checkpoint())?
        {
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
        let read = self
            .read
            .metadata_local(target, &mut || self.read_checkpoint())?;
        let local = read.info;
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
                .map_err(|error| {
                    ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                })?;
                self.verify_registered_owner(&child, admitted)?;
            }
        }
        let mut payload = Map::new();
        payload.insert("name".to_string(), json!(local.name));
        payload.insert("synonym".to_string(), json!(local.synonym));
        insert_serialized(&mut payload, "kind", &local.kind)?;
        insert_serialized(&mut payload, "details", &local.details)?;
        insert_serialized(&mut payload, "support", &local.support)?;
        insert_serialized(&mut payload, "properties", &local.properties)?;
        if let Some(profile) = local.member_read_profile {
            insert_serialized(&mut payload, "memberReadProfile", &profile)?;
        }
        insert_serialized(&mut payload, "declarations", &local.declarations)?;
        insert_serialized(&mut payload, "relations", &local.relations)?;
        let collections = serde_json::to_value(&local.collections).map_err(|error| {
            ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
        })?;
        payload.insert("collections".to_string(), collections);
        // Непрочитанное поле — повреждённое свойство, связь или пофактовая
        // часть вида — узел называет в `limits`, а ветвь без данных отказывает,
        // а не отвечает пустым списком: иначе `view` выдал бы непрочитанное за
        // отсутствующее. Сведения, не зависящие от поля, остаются.
        let mut unreadable = local
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == MetaDiagnosticSeverity::Error)
            .filter_map(|diagnostic| {
                let field = diagnostic.field.as_deref()?;
                (field.starts_with("details.")
                    || field.starts_with("properties.")
                    || field.starts_with("relations.")
                    || [
                        "standardAttributes",
                        "characteristics",
                        "standardTabularSections",
                    ]
                    .iter()
                    .any(|prefix| {
                        field
                            .strip_prefix(prefix)
                            .is_some_and(|rest| rest.is_empty() || rest.starts_with(['.', '[']))
                    })
                    || local
                        .member_read_profile
                        .is_some_and(|profile| profile.owns_diagnostic(field)))
                .then(|| {
                    json!({
                        "field": field,
                        "message": diagnostic.message,
                    })
                })
            })
            .collect::<Vec<_>>();
        // Без `WriteMode` неизвестно, подчинён ли регистр регистратору: список
        // документов-регистраторов не утверждается ни полным, ни пустым, и
        // узел с ветвью `Relation` называют это пределом. Повреждённое
        // значение уже названо диагностикой `properties.WriteMode`.
        if local.kind == MetadataKind::InformationRegister
            && !local
                .properties
                .iter()
                .any(|property| property.key == "WriteMode")
            && !unreadable
                .iter()
                .any(|entry| entry["field"] == "properties.WriteMode")
        {
            unreadable.push(json!({
                "field": "relations.registrars",
                "message": "registrar applicability is unknown because WriteMode is absent",
            }));
        }
        // Заимствование отвечает только в наборе расширения; там оно есть у
        // всякого объекта, и «своё» — такой же ответ, как «заимствовано».
        if let Some(borrowing) = read.borrowing {
            payload.extend(crate::infrastructure::v13_read_projection::borrowing_props(
                &borrowing,
            ));
        }

        // Предопределённые элементы — содержимое самого объекта, и писатель у
        // них есть. Без читателя агент, добавивший элемент, не может
        // подтвердить результат: ни счёта, ни списка, ни адреса.
        // Без известного `CodeType` владельца коды элементов не проверить:
        // ветвь предопределённых ограничена, а не выдумана и не роняет узел.
        match self.read.predefined_items(
            target,
            local.kind,
            local.predefined_code_type.as_deref(),
            &mut || self.read_checkpoint(),
        ) {
            Ok(Some(predefined)) => {
                insert_serialized(&mut payload, "predefinedItems", &predefined)?;
            }
            Ok(None) => {}
            Err(error)
                if local.predefined_code_type.is_none()
                    && error.detail() == Some(RefusalDetail::SourceUnreadable) =>
            {
                unreadable.push(json!({"field": "predefinedItems", "message": error.to_string()}));
            }
            Err(error) => return Err(error),
        }
        if !unreadable.is_empty() {
            payload.insert("unreadable".to_string(), Value::Array(unreadable));
        }
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
                    return Err(ViewError::detailed(
                        RefusalDetail::SourceUnreadable,
                        format!("registered {field} entry has no name"),
                    ));
                };
                let child = MetadataAddress::parse(
                    PLATFORM_XML_8_3_27_FORMAT_2_20,
                    &format!("{}.{}.{name}", target.as_str(), kind.as_str()),
                )
                .map_err(|error| {
                    ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                })?;
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
                RefusalCode::NotFound,
                "metadata owner has no canonical kind and name",
            ));
        };
        self.ensure_owner_registered_parts(owner_kind, owner_name, admitted)
    }

    fn ensure_owner_registered_parts(
        &self,
        owner_kind: &str,
        owner_name: &str,
        admitted: &ViewSourceSnapshot,
    ) -> Result<(), ViewError> {
        if matches!(
            self.read.source_set_kind(),
            SourceSetKind::Configuration | SourceSetKind::Extension
        ) && self.read.configuration_xml_requires_streaming()
        {
            let index = self.streamed_configuration_registration_index(admitted)?;
            return index
                .contains(owner_kind, owner_name, &|| self.read_checkpoint())?
                .then_some(())
                .ok_or_else(|| {
                    ViewError::new(
                        RefusalCode::NotFound,
                        format!("metadata owner `{owner_kind}.{owner_name}` is not registered"),
                    )
                });
        }
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
                RefusalCode::NotFound,
                format!("metadata owner `{owner_kind}.{owner_name}` is not registered"),
            )
        })
    }

    fn streamed_configuration_registration_index(
        &self,
        _admitted: &ViewSourceSnapshot,
    ) -> Result<Arc<RegistrationIndex>, ViewError> {
        self.read
            .streamed_configuration_registration_index(&|| self.read_checkpoint())
    }

    fn module_view(
        &self,
        route: &LogicalTreeRoute,
        admitted: &ViewSourceSnapshot,
    ) -> Result<NodeViewData, ViewError> {
        let Some(capability) = route.module() else {
            return self.module_branch_view(route.at(), admitted);
        };
        let (module_at, prefix_len) = module_prefix(route.at(), self.profile, capability)?;
        self.verify_module_owner(&module_at, capability, admitted)?;
        if capability.role() == ModuleRole::WebSocketClient {
            return Err(ViewError::detailed(
                RefusalDetail::ProviderAbsent,
                "WebSocketClient source layout is not specified for platform profile 8.3.27",
            ));
        }
        if self.read.source_set_kind() == SourceSetKind::Configuration
            && route.at().segments().last().is_some_and(|segment| {
                segment.kind() == NodeKind::Event && segment.name().is_some()
            })
        {
            let LogicalEventSource::Platform(source) = resolve_event_source(
                self.read.source_set(),
                self.read.source_set_kind(),
                self.profile,
                route.at(),
            )
            .map_err(|error| ViewError::new(error.code(), error.to_string()))?
            else {
                return Err(ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    "module event did not resolve to a Platform source",
                ));
            };
            let projection = self.module_projection(&source.module_at, capability, admitted)?;
            let event = project_platform_event_from_projection(&source, &projection)
                .map_err(|error| ViewError::new(error.code(), error.to_string()))?;
            return Ok(NodeViewData::Node(event_node(&event)));
        }
        let projections = self.module_projection(&module_at, capability, admitted)?;
        module_projection_view(route.at(), prefix_len, projections.as_ref())
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
        let mut cache = self.module_projections.lock().map_err(|_| {
            ViewError::detailed(RefusalDetail::CachePoisoned, "module cache is poisoned")
        })?;
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
                    ViewError::detailed(
                        RefusalDetail::SourceUnreadable,
                        "common module owner is invalid",
                    )
                })?;
            let owner = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, owner).map_err(
                |error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string()),
            )?;
            let descriptor = self.read.metadata_descriptor(&owner)?;
            Some(common_module_properties(
                &descriptor,
                self.read.source_set_kind(),
            )?)
        } else {
            None
        };
        let handles = if capability.role() == ModuleRole::Form {
            self.form_bindings(module_at, admitted)?
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
            .map_err(|error| ViewError::new(RefusalCode::ProviderUnavailable, error))?,
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
                RefusalCode::NotFound,
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
                    ViewError::new(
                        RefusalCode::NotFound,
                        "WebSocketClient owner name is absent",
                    )
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
                // An extension extends the configuration runtime modules from
                // its own root (`Ext/ManagedApplicationModule.bsl` and the
                // like), so its `Configuration.xml` owns them exactly as the
                // main configuration does.
                SourceSetKind::Configuration | SourceSetKind::Extension => Ok(()),
                SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport => {
                    Err(ViewError::new(
                        RefusalCode::NotFound,
                        "external source sets have no configuration runtime modules",
                    ))
                }
            };
        }
        if capability.role() == ModuleRole::WebSocketClient {
            let owner = module_at.segments().first().ok_or_else(|| {
                ViewError::new(RefusalCode::NotFound, "WebSocketClient owner is absent")
            })?;
            return self.ensure_owner_registered_parts(
                NodeKind::WebSocketClient.as_str(),
                owner.name().ok_or_else(|| {
                    ViewError::new(
                        RefusalCode::NotFound,
                        "WebSocketClient owner name is absent",
                    )
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
            .map_err(|_| {
                ViewError::detailed(RefusalDetail::CachePoisoned, "owner cache is poisoned")
            })?
            .contains(&key)
        {
            return Ok(());
        }
        self.ensure_owner_registered(target, admitted)?;
        let parts = target.as_str().split('.').collect::<Vec<_>>();
        if parts.len() < 2 || parts.len() % 2 != 0 {
            return Err(ViewError::new(
                RefusalCode::ProviderUnavailable,
                "physical metadata owner must contain complete kind/name pairs",
            ));
        }
        let revision = key.revision.clone();
        let mut parent: Option<(MetadataAddress, Arc<PlatformXmlSourceSetOwnerEvidence>)> = None;
        for pair_count in 1..=parts.len() / 2 {
            let end = pair_count * 2;
            let current_text = parts[..end].join(".");
            let current = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &current_text)
                .map_err(|error| {
                    ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                })?;
            if let Some((parent_address, parent_evidence)) = &parent {
                let child_kind = parts[end - 2];
                let child_name = parts[end - 1];
                let edge_key = OwnerEdgeCacheKey {
                    revision: revision.clone(),
                    parent: parent_address.as_str().to_string(),
                    child: current_text.clone(),
                };
                let mut edges = self.verified_owner_edges.lock().map_err(|_| {
                    ViewError::detailed(
                        RefusalDetail::CachePoisoned,
                        "owner edge cache is poisoned",
                    )
                })?;
                if !edges.contains(&edge_key) {
                    let registered = parent_evidence
                        .registrations()
                        .any(|(kind, name)| kind == child_kind && name == child_name);
                    if !registered {
                        return Err(ViewError::new(
                            RefusalCode::NotFound,
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
                        RefusalCode::NotFound,
                        "WebSocketClient has no specified nested export owners",
                    ));
                }
                continue;
            }
            if pair_count > 1 && parts[end - 2] == NodeKind::Command.as_str() {
                // A command has no descriptor file: its full definition is the
                // inline `ChildObjects` registration already proven above.
                if end != parts.len() {
                    return Err(ViewError::new(
                        RefusalCode::NotFound,
                        "metadata command has no nested physical owners",
                    ));
                }
                break;
            }
            let evidence = self.owner_evidence(&current, admitted).map_err(|error| {
                if error.code() == RefusalCode::NotFound {
                    ViewError::detailed(
                        RefusalDetail::SourceUnreadable,
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
                return Err(ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
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
            .map_err(|_| {
                ViewError::detailed(RefusalDetail::CachePoisoned, "owner cache is poisoned")
            })?
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
            ViewError::detailed(
                RefusalDetail::CachePoisoned,
                "owner evidence cache is poisoned",
            )
        })?;
        if let Some(evidence) = cache.get(&key) {
            return Ok(Arc::clone(evidence));
        }
        let evidence = Arc::new(
            self.read
                .metadata_owner_evidence(target, &mut || self.read_checkpoint())?,
        );
        cache.insert(key, Arc::clone(&evidence));
        Ok(evidence)
    }

    /// One parsed `Form.xml` per admitted revision and form target: every
    /// item, attribute, command and event address under the form shares it.
    fn form_data(
        &self,
        target: &MetadataAddress,
        admitted: &ViewSourceSnapshot,
    ) -> Result<Arc<FormInfoData>, ViewError> {
        let key = OwnerProofCacheKey {
            revision: RevisionCacheKey {
                source_set_identity: admitted.source_set_identity.clone(),
                revision: admitted.revision.clone(),
            },
            owner: target.as_str().to_string(),
        };
        let mut cache = self.form_data.lock().map_err(|_| {
            ViewError::detailed(RefusalDetail::CachePoisoned, "form cache is poisoned")
        })?;
        if let Some(data) = cache.get(&key) {
            return Ok(Arc::clone(data));
        }
        let data = Arc::new(
            self.read
                .form_data(target, &mut || self.read_checkpoint())?,
        );
        cache.insert(key, Arc::clone(&data));
        Ok(data)
    }

    fn form_bindings(
        &self,
        module_at: &QualifiedAddress,
        admitted: &ViewSourceSnapshot,
    ) -> Result<Vec<FormEventBindingInput>, ViewError> {
        let module_at_text = module_at.to_string();
        let form_at = module_at_text
            .rsplit_once(".Module.Form")
            .map(|(form, _)| form)
            .ok_or_else(|| {
                ViewError::new(
                    RefusalCode::ProviderUnavailable,
                    "form module address is invalid",
                )
            })?;
        let form = QualifiedAddress::parse(form_at).map_err(|error| {
            ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
        })?;
        let metadata_path =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &form.logical_path()).map_err(
                |error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string()),
            )?;
        if self
            .read
            .inherited_resource(&metadata_path, "Form.xml")?
            .is_some()
        {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                format!(
                    "`{module_at}` belongs to a borrowed form whose content the extension does not carry: there is no `Form.xml`, and the platform keeps no form module without it; read the parent named by props.extends of `{}`",
                    extension_object(module_at)
                ),
            ));
        }
        let data = self.form_data(&metadata_path, admitted)?;
        Ok(form_semantic_inputs(form_at, &FormEventEvidence::from_info(&data)).bindings)
    }

    fn form_view(
        &self,
        route: &LogicalTreeRoute,
        admitted: &ViewSourceSnapshot,
    ) -> Result<NodeViewData, ViewError> {
        let target = route.reader_metadata_path().ok_or_else(|| {
            ViewError::new(
                RefusalCode::ProviderUnavailable,
                "form route has no typed target",
            )
        })?;
        self.verify_registered_owner(target, admitted)?;
        let data = self.form_data(target, admitted)?;
        let form_at =
            QualifiedAddress::parse(&format!("{}:{}", route.at().source_set(), target.as_str()))
                .map_err(|error| {
                    ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                })?;
        let module_at =
            QualifiedAddress::parse(&format!("{form_at}.Module.Form")).map_err(|error| {
                ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
            })?;
        let capability = self.profile.module_capability(&module_at).ok_or_else(|| {
            ViewError::detailed(
                RefusalDetail::ProviderAbsent,
                "form module capability is absent from the platform profile",
            )
        })?;
        self.verify_module_owner(&module_at, capability, admitted)?;
        let mut evidence = FormEventEvidence::from_info(&data);
        evidence.context.metadata_owner = route.at().segments().first().map(AddressSegment::kind);
        let inputs = form_semantic_inputs(&form_at.to_string(), &evidence);
        let module = self.module_projection(&module_at, capability, admitted)?;
        let events = project_property_events_from_projection(&evidence, &inputs, &module)
            .map_err(|error| ViewError::new(error.code(), error.to_string()))?;
        let payload = serde_json::to_value(data.as_ref()).map_err(|error| {
            ViewError::new(
                RefusalCode::ProviderUnavailable,
                format!("form payload serialization failed: {error}"),
            )
        })?;
        crate::infrastructure::v13_read_projection::project_typed_payload_with_form_events(
            route, payload, &events,
        )
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
                crate::infrastructure::v13_read_projection::project_registered_metadata_branch_names(
                    route.at(),
                    &names,
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
                crate::infrastructure::v13_read_projection::project_registered_metadata_branch(
                    route.at(),
                    payload.as_ref(),
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
        #[cfg(test)]
        observe_completed_view_projection_for_test(at, &admitted.revision);
        Ok(projected)
    }
}

impl LogicalViewReadAuthority<'_> {
    /// Search needs the named owner's registration and descriptor, but not a
    /// projected collection of its children. An unnamed kind branch is a
    /// valid logical scope even when it currently contains no objects.
    pub(crate) fn validate_search_scope_owner(
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
    ) -> Result<
        Option<(
            crate::infrastructure::native_operations::meta::MetaBorrowing,
            &'static str,
        )>,
        ViewError,
    > {
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
        let [owner, template] = segments else {
            return Ok(projected);
        };
        if template.kind() != NodeKind::Template || template.name().is_none() {
            return Ok(projected);
        }
        let Some(owner_name) = owner.name() else {
            return Ok(projected);
        };
        let child = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            &format!(
                "{}.{owner_name}.Template.{}",
                owner.kind().as_str(),
                template.name().unwrap_or_default()
            ),
        )
        .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string()))?;
        let mut projected = projected;
        let format = match self.read.metadata_child_profile(&child) {
            Ok(MetadataChildProfile::Template(MetadataTemplateType::DataCompositionSchema)) => {
                Some("DataCompositionSchema")
            }
            Ok(MetadataChildProfile::Template(MetadataTemplateType::SpreadsheetDocument)) => {
                Some("SpreadsheetDocument")
            }
            _ => None,
        };
        if format.is_some() {
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
            Ok(MetadataChildProfile::Template(MetadataTemplateType::DataCompositionSchema)) => {
                let payload = self
                    .read
                    .dcs_payload(child, &mut || self.read_checkpoint())?;
                crate::infrastructure::v13_read_projection::dcs_template_branches(&payload)
            }
            Ok(MetadataChildProfile::Template(MetadataTemplateType::SpreadsheetDocument)) => {
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
            Ok(MetadataChildProfile::Template(_)) => Vec::new(),
            Ok(
                MetadataChildProfile::Form
                | MetadataChildProfile::OrdinaryForm
                | MetadataChildProfile::Command,
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
pub(crate) const INHERITED_CONTENT: &str = "inherited";

/// Узел заимствованного объекта без своего сопутствующего файла.
///
/// Сам объект читается по дескриптору: заимствование и адрес родителя, а
/// `content: "inherited"` говорит, что прав, элементов формы, типов пакета
/// или тела макета в расширении нет и действует содержимое родителя. Счёт и
/// ветви не выдумываются. Адрес внутри такого содержимого в этом наборе не
/// существует: отказ `not_found` направляет к родителю.
fn project_inherited_part(
    route: &LogicalTreeRoute,
    borrowing: &crate::infrastructure::native_operations::meta::MetaBorrowing,
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
    shared_module_source_address(module_at, capability)
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
    let document = roxmltree::Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string()))?;
    let root = document.root_element();
    use crate::domain::metadata::{
        MetaInfoPropertyAbsence, MetaInfoPropertyValue, META_INFO_PROPERTY_PROFILE,
    };
    const NS: &str = "http://v8.1c.ru/8.3/MDClasses";
    // Идентичность заимствования проверяется строго до того, как разрешить
    // пропуски: один объект `CommonModule`, один прямой `Properties` из
    // пространства имён метаданных, единственные корректные `ObjectBelonging`
    // и `ExtendedConfigurationObject`. Иначе повреждённый маркер открыл бы
    // послабление для любого описания.
    let objects = root
        .children()
        .filter(roxmltree::Node::is_element)
        .collect::<Vec<_>>();
    let [object] = objects.as_slice() else {
        return Err(ViewError::detailed(
            RefusalDetail::SourceUnreadable,
            "common module descriptor must contain one object",
        ));
    };
    if !object.has_tag_name((NS, "CommonModule")) {
        return Err(ViewError::detailed(
            RefusalDetail::SourceUnreadable,
            "common module descriptor has the wrong object kind",
        ));
    }
    let containers = object
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "Properties")
        .collect::<Vec<_>>();
    let properties = match containers.as_slice() {
        [node] if node.tag_name().namespace() == Some(NS) => *node,
        _ => {
            return Err(ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "common module descriptor must contain one direct Properties container",
            ))
        }
    };
    let borrowing_property = |name| {
        crate::infrastructure::native_operations::meta::observe_meta_borrowing_property(
            properties,
            MetadataKind::CommonModule,
            name,
        )
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
        let value = crate::infrastructure::native_operations::meta::observe_meta_info_property(
            properties,
            MetadataKind::CommonModule,
            name,
        )
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
            prefix: crate::application::v13::source_text::complete_prefix(text).to_string(),
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

#[cfg(test)]
use crate::infrastructure::v13_read_projection::project_known_suffix;
use crate::infrastructure::v13_read_projection::project_typed_payload;

#[cfg(test)]
pub(crate) mod tests;
