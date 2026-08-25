use crate::application::metadata::MetaInfoRequest;
use crate::application::v13::view::{ViewError, ViewFilter, ViewReadAuthority, ViewSourceSnapshot};
use crate::domain::address::{AddressSegment, NodeKind, QualifiedAddress};
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::module_projection::{
    CommonModuleProperties, EventProjection, MethodProjection, ModuleProjectionSet,
    RegionProjection,
};
use crate::domain::node_view::{BranchRef, CollectionView, NodeView, NodeViewData};
use crate::domain::platform_profile::{
    ModuleCapability, ModuleRole, ModuleSourceLayout, PlatformProfile,
};
use crate::domain::source_target::{
    MetadataAddress, SourceTarget, PLATFORM_XML_8_3_27_FORMAT_2_20,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::bsl_module_projection::{
    project_module, FormBindingOwner, FormEventBindingInput, ModuleProjectionRequest,
};
use crate::infrastructure::logical_tree::{route_logical_address, LogicalReader, LogicalTreeRoute};
use crate::infrastructure::metadata_operations::MetadataOperations;
use crate::infrastructure::native_operations::common::read_utf8_sig;
use crate::infrastructure::native_operations::form::{
    analyze_form_info_with_data, FormInfoElement, FormInfoEvent,
};
use crate::infrastructure::native_operations::form_event_registry::FormElementKind;
use crate::infrastructure::native_operations::typed_result::NativeInvocationContext;
use crate::infrastructure::native_operations::NativeOperationAdapter;
use crate::infrastructure::platform::filesystem::RetainedDirectoryCapability;
use crate::infrastructure::platform_xml_source_targets::{
    platform_xml_resource_evidence, resolve_platform_xml_target, TargetKindPolicy,
};
use crate::infrastructure::source_revision::SourceRevisionService;
use crate::infrastructure::support_state::WorkspaceSupportStateReader;
use serde_json::{json, Map, Value};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

const READ_BUDGET: Duration = Duration::from_secs(7);
const META_READ_LIMIT: usize = 1_000;
const MAX_MODULE_BYTES: usize = 8 * 1024 * 1024;

/// Hidden v0.13 read adapter. Its revision service is supplied by the
/// workspace actor that owns the source capability; the adapter never creates
/// an ambient per-call revision authority.
pub(crate) struct LogicalViewReadAuthority<'a> {
    context: &'a WorkspaceContext,
    cancellation: &'a CancellationToken,
    source_set: String,
    source_set_identity: String,
    revision_service: Arc<SourceRevisionService>,
    source_root: Arc<RetainedDirectoryCapability>,
    profile: PlatformProfile,
}

impl<'a> LogicalViewReadAuthority<'a> {
    pub(crate) fn new(
        context: &'a WorkspaceContext,
        cancellation: &'a CancellationToken,
        source_set: impl Into<String>,
        source_set_identity: impl Into<String>,
        revision_service: Arc<SourceRevisionService>,
        source_root: Arc<RetainedDirectoryCapability>,
        profile: PlatformProfile,
    ) -> Self {
        Self {
            context,
            cancellation,
            source_set: source_set.into(),
            source_set_identity: source_set_identity.into(),
            revision_service,
            source_root,
            profile,
        }
    }

    fn exact_revision(&self) -> Result<String, ViewError> {
        self.source_root
            .validate_named_identity()
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?;
        self.revision_service
            .snapshot(
                ProviderDeadline::from_budget(READ_BUDGET),
                self.cancellation,
            )
            .map(|revision| {
                format!(
                    "{}:{}:{}",
                    revision.algorithm, revision.generation, revision.digest
                )
            })
            .map_err(|error| ViewError::new("provider_unavailable", error))
    }

    fn read_module_source(&self, path: &Path) -> Result<Option<String>, ViewError> {
        let relative = path.strip_prefix(self.source_root.path()).map_err(|_| {
            ViewError::new(
                "provider_unavailable",
                "module target escaped the actor-owned source root",
            )
        })?;
        match self
            .source_root
            .read_relative_regular_bounded(relative, MAX_MODULE_BYTES)
        {
            Ok(bytes) => String::from_utf8(bytes)
                .map(|text| Some(text.trim_start_matches('\u{feff}').to_string()))
                .map_err(|_| ViewError::new("provider_unavailable", "BSL module is not UTF-8")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(ViewError::new("provider_unavailable", error.to_string())),
        }
    }

    fn typed_payload(&self, route: &LogicalTreeRoute) -> Result<Value, ViewError> {
        if route.reader() == LogicalReader::Metadata {
            return self.metadata_payload(route);
        }
        let (operation, tool_name) = match route.reader() {
            LogicalReader::Configuration => ("cf-info", "unica.cf.info"),
            LogicalReader::Form => ("form-info", "unica.form.info"),
            LogicalReader::Role => ("role-info", "unica.role.info"),
            LogicalReader::Subsystem | LogicalReader::Interface => {
                ("subsystem-info", "unica.subsystem.info")
            }
            LogicalReader::Dcs => ("dcs-info", "unica.dcs.info"),
            LogicalReader::Mxl => ("mxl-info", "unica.mxl.info"),
            LogicalReader::Xdto => ("xdto-info", "unica.xdto.info"),
            LogicalReader::Metadata | LogicalReader::Module => {
                return Err(ViewError::new(
                    "provider_unavailable",
                    "logical reader requires its dedicated adapter",
                ));
            }
        };
        let mut args = Map::from_iter([("sourceSet".to_string(), json!(self.source_set))]);
        if let Some(target) = route.reader_metadata_path() {
            args.insert("metadataPath".to_string(), json!(target.as_str()));
        }
        if route.reader() == LogicalReader::Xdto {
            if let Some(type_name) = named_segment(route.at(), NodeKind::Type) {
                args.insert("typeName".to_string(), json!(type_name));
            }
        }
        let support = WorkspaceSupportStateReader::new(self.context);
        let invocation = NativeInvocationContext::new(
            &support,
            self.cancellation,
            ProviderDeadline::from_budget(READ_BUDGET),
        );
        let result = NativeOperationAdapter::invoke_with_data(
            operation,
            tool_name,
            &args,
            self.context,
            false,
            false,
            invocation,
        )
        .map_err(|error| ViewError::new("provider_unavailable", error))?;
        if !result.adapter.ok {
            return Err(ViewError::new(
                classify_reader_failure(&result.adapter.errors),
                result.adapter.summary,
            ));
        }
        result.data.ok_or_else(|| {
            ViewError::new(
                "provider_unavailable",
                format!("{operation} returned no typed payload"),
            )
        })
    }

    fn metadata_payload(&self, route: &LogicalTreeRoute) -> Result<Value, ViewError> {
        let target = route.reader_metadata_path().ok_or_else(|| {
            ViewError::new("not_found", "metadata address has no typed reader target")
        })?;
        let request = MetaInfoRequest {
            source_set: self.source_set.clone(),
            metadata_path: target.clone(),
            sections: Vec::new(),
            limit: META_READ_LIMIT,
        };
        let support = WorkspaceSupportStateReader::new(self.context);
        let read =
            MetadataOperations::read_local(&request, self.context, self.cancellation, &support)
                .map_err(|failure| {
                    let message = serde_json::to_string(&failure.diagnostics)
                        .unwrap_or_else(|_| "metadata reader failed".to_string());
                    ViewError::new("provider_unavailable", message)
                })?;
        let local = read.local;
        let mut payload = Map::new();
        payload.insert("name".to_string(), json!(local.name));
        payload.insert("synonym".to_string(), json!(local.synonym));
        insert_serialized(&mut payload, "kind", &local.kind)?;
        insert_serialized(&mut payload, "details", &local.details)?;
        insert_serialized(&mut payload, "support", &local.support)?;
        insert_serialized(&mut payload, "properties", &local.properties)?;
        insert_serialized(&mut payload, "declarations", &local.declarations)?;
        insert_serialized(&mut payload, "relations", &local.relations)?;
        insert_serialized(&mut payload, "collections", &local.collections)?;
        Ok(Value::Object(payload))
    }

    fn module_view(
        &self,
        route: &LogicalTreeRoute,
        admitted: &ViewSourceSnapshot,
    ) -> Result<NodeViewData, ViewError> {
        let capability = route.module().ok_or_else(|| {
            ViewError::new(
                "provider_unavailable",
                "module route has no platform capability",
            )
        })?;
        if capability.role() == ModuleRole::WebSocketClient {
            return Err(ViewError::new(
                "provider_unavailable",
                "WebSocketClient source layout is not specified for platform profile 8.3.27",
            ));
        }
        let (module_at, prefix_len) = module_prefix(route.at(), self.profile, capability)?;
        let metadata_path = module_source_address(&module_at, capability)?;
        let resolution = resolve_platform_xml_target(
            self.context,
            &SourceTarget {
                source_set: self.source_set.clone(),
                metadata_path: Some(metadata_path),
            },
            TargetKindPolicy::ModuleOnlyAllowingAbsent,
        )
        .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?;
        let evidence = platform_xml_resource_evidence(self.context, &resolution.handle)
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?;
        let source = self.read_module_source(&evidence.target_path)?;
        let common_module = if capability.role() == ModuleRole::Common {
            Some(common_module_properties(
                evidence.descriptor_paths.first().ok_or_else(|| {
                    ViewError::new(
                        "provider_unavailable",
                        "common module has no proven descriptor",
                    )
                })?,
            )?)
        } else {
            None
        };
        let handles = if capability.role() == ModuleRole::Form {
            self.form_bindings(&module_at)?
        } else {
            Vec::new()
        };
        let projections = project_module(ModuleProjectionRequest {
            at: &module_at,
            capability,
            title: module_title(&module_at, capability),
            rev: &admitted.revision,
            source: source.as_deref(),
            common_module,
            handles: &handles,
            declarative_bindings: &[],
            extension_targets: &[],
        })
        .map_err(|error| ViewError::new("provider_unavailable", error))?;
        module_projection_view(route.at(), prefix_len, &projections)
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
        let args = Map::from_iter([
            ("sourceSet".to_string(), json!(self.source_set)),
            ("metadataPath".to_string(), json!(metadata_path.as_str())),
        ]);
        let support = WorkspaceSupportStateReader::new(self.context);
        let execution = analyze_form_info_with_data(&args, self.context, &support);
        if !execution.outcome.ok {
            return Err(ViewError::new(
                classify_reader_failure(&execution.outcome.errors),
                execution.outcome.summary,
            ));
        }
        let data = execution.data.ok_or_else(|| {
            ViewError::new(
                "provider_unavailable",
                "form reader returned no typed payload",
            )
        })?;
        let mut bindings = data
            .events
            .iter()
            .map(|event| form_event_binding(FormBindingOwner::Form, form_at, event))
            .collect::<Vec<_>>();
        collect_element_bindings(form_at, &data.elements, false, &mut bindings);
        for command in &data.commands {
            let at = format!("{form_at}.Command.{}", command.name);
            bindings.extend(
                command
                    .actions
                    .iter()
                    .map(|event| form_event_binding(FormBindingOwner::Command, &at, event)),
            );
        }
        Ok(bindings)
    }
}

impl ViewReadAuthority for LogicalViewReadAuthority<'_> {
    fn snapshot(&self, at: &QualifiedAddress) -> Result<ViewSourceSnapshot, ViewError> {
        if at.source_set() != self.source_set {
            return Err(ViewError::new(
                "not_found",
                "logical address belongs to another actor-owned source set",
            ));
        }
        route_logical_address(at, self.profile)
            .map_err(|error| ViewError::new("not_found", error.to_string()))?;
        Ok(ViewSourceSnapshot {
            source_set_identity: self.source_set_identity.clone(),
            revision: self.exact_revision()?,
        })
    }

    fn read_exact(
        &self,
        at: &QualifiedAddress,
        _filter: &ViewFilter,
        admitted: &ViewSourceSnapshot,
    ) -> Result<NodeViewData, ViewError> {
        if admitted.source_set_identity != self.source_set_identity
            || admitted.revision != self.exact_revision()?
        {
            return Err(ViewError::new(
                "stale_cursor",
                "source revision changed before the typed read",
            ));
        }
        let route = route_logical_address(at, self.profile)
            .map_err(|error| ViewError::new("not_found", error.to_string()))?;
        let projected = if route.reader() == LogicalReader::Module {
            self.module_view(&route, admitted)?
        } else {
            let payload = self.typed_payload(&route)?;
            project_typed_payload(&route, payload)?
        };
        if admitted.revision != self.exact_revision()? {
            return Err(ViewError::new(
                "stale_cursor",
                "source revision changed during the typed read",
            ));
        }
        Ok(projected)
    }
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

fn common_module_properties(path: &Path) -> Result<CommonModuleProperties, ViewError> {
    let text =
        read_utf8_sig(path).map_err(|error| ViewError::new("provider_unavailable", error))?;
    let document = roxmltree::Document::parse(&text)
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

fn collect_element_bindings(
    form_at: &str,
    elements: &[FormInfoElement],
    parent_is_table: bool,
    bindings: &mut Vec<FormEventBindingInput>,
) {
    for element in elements {
        let Some(kind) = FormElementKind::from_xml_tag(&element.tag) else {
            continue;
        };
        let is_table = kind == FormElementKind::Table;
        let owner = if is_table {
            FormBindingOwner::Table
        } else if parent_is_table {
            FormBindingOwner::Column(kind)
        } else {
            FormBindingOwner::Element(kind)
        };
        let at = format!("{form_at}.Item.{}", element.name);
        bindings.extend(
            element
                .events
                .iter()
                .map(|event| form_event_binding(owner, &at, event)),
        );
        collect_element_bindings(form_at, &element.children, is_table, bindings);
    }
}

fn module_title(at: &QualifiedAddress, capability: ModuleCapability) -> String {
    let owner = at
        .segments()
        .first()
        .and_then(AddressSegment::name)
        .unwrap_or("Configuration");
    format!("{} module {owner}", capability.role().as_str())
}

fn module_projection_view(
    requested: &QualifiedAddress,
    prefix_len: usize,
    projections: &ModuleProjectionSet,
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
            .ok_or_else(|| ViewError::new("not_found", format!("method `{name}` was not found"))),
        (NodeKind::Method, Some(name), Some(detail))
            if detail.name().is_none() && detail.kind() == NodeKind::Body =>
        {
            let method = find_method(projections, name)?;
            let items = projections
                .body()
                .iter()
                .filter(|line| {
                    line.line >= method.body_from_line && line.line <= method.body_to_line
                })
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
                .map(|line| json!({"line": line.line, "text": line.text}))
                .collect(),
        ),
        _ => Err(ViewError::new(
            "not_found",
            "module projection suffix is not available",
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

fn event_node(event: &EventProjection) -> NodeView {
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
    let can = event
        .can
        .iter()
        .map(|operation| {
            crate::domain::node_view::OperationRef::new(
                &operation.op,
                Map::from_iter([("at".to_string(), json!(operation.at))]),
            )
        })
        .collect();
    NodeView::new(&event.at, NodeKind::Event.as_str(), &event.event_id, props).with_can(can)
}

fn event_node_value(event: &EventProjection) -> Value {
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

fn classify_reader_failure(errors: &[String]) -> &'static str {
    if errors.iter().any(|error| {
        error.contains("not found")
            || error.contains("File not found")
            || error.contains("resource_missing")
    }) {
        "not_found"
    } else {
        "provider_unavailable"
    }
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
mod tests;
