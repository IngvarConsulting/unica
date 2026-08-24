use crate::domain::address::{NodeKind, QualifiedAddress};
use crate::domain::platform_profile::ModuleRole;
use serde::ser::{SerializeStruct, Serializer};
use serde::Serialize;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) enum BranchKind {
    Method,
    Region,
    Interface,
    Event,
    Compilation,
    Body,
}

impl BranchKind {
    pub(crate) const ALL: [Self; 6] = [
        Self::Method,
        Self::Region,
        Self::Interface,
        Self::Event,
        Self::Compilation,
        Self::Body,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Method => "Method",
            Self::Region => "Region",
            Self::Interface => "Interface",
            Self::Event => "Event",
            Self::Compilation => "Compilation",
            Self::Body => "Body",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommonModuleProperties {
    pub(crate) global: bool,
    pub(crate) client_managed_application: bool,
    pub(crate) server: bool,
    pub(crate) external_connection: bool,
    pub(crate) client_ordinary_application: bool,
    pub(crate) server_call: bool,
    pub(crate) privileged: bool,
    pub(crate) return_values_reuse: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModuleProperties {
    pub(crate) owner_kind: String,
    pub(crate) role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) common_module: Option<CommonModuleProperties>,
}

impl ModuleProperties {
    pub(crate) fn new(owner_kind: NodeKind, role: ModuleRole) -> Self {
        Self {
            owner_kind: owner_kind.as_str().to_string(),
            role: role.as_str().to_string(),
            common_module: None,
        }
    }

    pub(crate) fn common(common_module: CommonModuleProperties) -> Self {
        Self {
            owner_kind: NodeKind::CommonModule.as_str().to_string(),
            role: ModuleRole::Common.as_str().to_string(),
            common_module: Some(common_module),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModuleIdentity {
    pub(crate) at: QualifiedAddress,
    pub(crate) title: String,
    pub(crate) props: ModuleProperties,
    pub(crate) rev: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct BranchSummary {
    #[serde(skip)]
    pub(crate) kind: BranchKind,
    pub(crate) at: String,
    pub(crate) count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ModuleSummary {
    pub(crate) at: String,
    pub(crate) kind: &'static str,
    pub(crate) title: String,
    pub(crate) props: ModuleProperties,
    pub(crate) branches: Vec<BranchSummary>,
    pub(crate) rev: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MethodKind {
    Procedure,
    Function,
}

impl MethodKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Procedure => "procedure",
            Self::Function => "function",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompilationFacts {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) directive: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) guard: Option<String>,
    pub(crate) contexts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) form_context: Option<bool>,
    pub(crate) conditional_body: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum BindingFact {
    Platform,
    Property,
    Code,
    Subscription,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HandleProjection {
    pub(crate) owner: String,
    pub(crate) event: String,
    pub(crate) at: String,
    pub(crate) binding: BindingFact,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) call_type: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ExtensionKind {
    Before,
    After,
    Instead,
    ChangeAndValidate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExtensionProjection {
    pub(crate) kind: ExtensionKind,
    pub(crate) directive: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) target_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MethodProjection {
    pub(crate) at: String,
    pub(crate) name: String,
    pub(crate) signature: String,
    pub(crate) doc: Option<String>,
    pub(crate) method_kind: MethodKind,
    pub(crate) export: bool,
    pub(crate) compile: CompilationFacts,
    pub(crate) handles: Vec<HandleProjection>,
    pub(crate) extension: Option<ExtensionProjection>,
    pub(crate) body_from_line: usize,
    pub(crate) body_to_line: usize,
}

impl Serialize for MethodProjection {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Props<'a> {
            signature: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            doc: &'a Option<String>,
            method_kind: MethodKind,
            export: bool,
            compile: &'a CompilationFacts,
            #[serde(skip_serializing_if = "Vec::is_empty")]
            handles: &'a Vec<HandleProjection>,
            #[serde(skip_serializing_if = "Option::is_none")]
            extension: &'a Option<ExtensionProjection>,
        }

        let mut state = serializer.serialize_struct("MethodProjection", 4)?;
        state.serialize_field("at", &self.at)?;
        state.serialize_field("kind", "Method")?;
        state.serialize_field(
            "props",
            &Props {
                signature: &self.signature,
                doc: &self.doc,
                method_kind: self.method_kind,
                export: self.export,
                compile: &self.compile,
                handles: &self.handles,
                extension: &self.extension,
            },
        )?;
        state.serialize_field(
            "branches",
            &[
                BranchSummary {
                    kind: BranchKind::Compilation,
                    at: format!("{}.Compilation", self.at),
                    count: usize::from(
                        self.compile.guard.is_some() || self.compile.directive.is_some(),
                    ),
                },
                BranchSummary {
                    kind: BranchKind::Body,
                    at: format!("{}.Body", self.at),
                    count: if self.body_to_line < self.body_from_line {
                        0
                    } else {
                        self.body_to_line - self.body_from_line + 1
                    },
                },
            ],
        )?;
        state.end()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegionProjection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) at: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) addressable: bool,
    pub(crate) line: usize,
    pub(crate) end_line: Option<usize>,
    pub(crate) methods: Vec<String>,
    pub(crate) children: Vec<String>,
    #[serde(skip)]
    pub(crate) parent: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) enum InterfaceKind {
    Public,
    Library,
    Override,
}

impl InterfaceKind {
    pub(crate) const ALL: [Self; 3] = [Self::Public, Self::Library, Self::Override];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Public => "Public",
            Self::Library => "Library",
            Self::Override => "Override",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct InterfaceProjection {
    pub(crate) at: String,
    pub(crate) kind: &'static str,
    pub(crate) interface: InterfaceKind,
    pub(crate) methods: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum EventState {
    Available,
    Implemented,
    Missing,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OperationCapability {
    pub(crate) op: String,
    pub(crate) at: String,
}

impl OperationCapability {
    pub(crate) fn event_implement(at: impl Into<String>) -> Self {
        Self {
            op: "event.implement".to_string(),
            at: at.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EventProjection {
    pub(crate) at: String,
    pub(crate) event_id: String,
    pub(crate) state: EventState,
    pub(crate) signature: String,
    pub(crate) context: String,
    pub(crate) binding: BindingFact,
    pub(crate) handler: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) implementation_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) call_type: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) can: Vec<OperationCapability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompilationProjection {
    pub(crate) from_line: usize,
    pub(crate) to_line: usize,
    pub(crate) guard: String,
    pub(crate) contexts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct BodyLine {
    pub(crate) line: usize,
    pub(crate) text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BodyPage {
    pub(crate) lines: Vec<BodyLine>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectionError {
    code: &'static str,
    message: String,
}

impl ProjectionError {
    pub(crate) fn ambiguous(message: impl Into<String>) -> Self {
        Self {
            code: "ambiguous_address",
            message: message.into(),
        }
    }

    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self {
            code: "not_found",
            message: message.into(),
        }
    }

    pub(crate) fn invalid_context(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_context",
            message: message.into(),
        }
    }

    pub(crate) fn is_ambiguous(&self) -> bool {
        self.code == "ambiguous_address"
    }
}

impl fmt::Display for ProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProjectionError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModuleProjectionSet {
    summary: ModuleSummary,
    methods: Vec<MethodProjection>,
    regions: Vec<RegionProjection>,
    interfaces: Vec<InterfaceProjection>,
    events: Vec<EventProjection>,
    compilation: Vec<CompilationProjection>,
    body: Vec<BodyLine>,
}

impl ModuleProjectionSet {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        identity: ModuleIdentity,
        methods: Vec<MethodProjection>,
        regions: Vec<RegionProjection>,
        interfaces: Vec<InterfaceProjection>,
        events: Vec<EventProjection>,
        compilation: Vec<CompilationProjection>,
        body: Vec<BodyLine>,
    ) -> Self {
        let counts = [
            methods.len(),
            regions.len(),
            interfaces.len(),
            events.len(),
            compilation.len(),
            body.len(),
        ];
        let base = identity.at.to_string();
        let branches = BranchKind::ALL
            .iter()
            .zip(counts)
            .map(|(kind, count)| BranchSummary {
                kind: *kind,
                at: format!("{base}.{}", kind.as_str()),
                count,
            })
            .collect();
        Self {
            summary: ModuleSummary {
                at: base,
                kind: "Module",
                title: identity.title,
                props: identity.props,
                branches,
                rev: identity.rev,
            },
            methods,
            regions,
            interfaces,
            events,
            compilation,
            body,
        }
    }

    pub(crate) const fn summary(&self) -> &ModuleSummary {
        &self.summary
    }

    pub(crate) fn methods(&self) -> &[MethodProjection] {
        &self.methods
    }

    pub(crate) fn regions(&self) -> &[RegionProjection] {
        &self.regions
    }

    pub(crate) fn interfaces(&self) -> &[InterfaceProjection] {
        &self.interfaces
    }

    pub(crate) fn events(&self) -> &[EventProjection] {
        &self.events
    }

    pub(crate) fn compilation(&self) -> &[CompilationProjection] {
        &self.compilation
    }

    pub(crate) fn body(&self) -> &[BodyLine] {
        &self.body
    }

    pub(crate) fn region(&self, name: &str) -> Result<&RegionProjection, ProjectionError> {
        let matches = self
            .regions
            .iter()
            .filter(|region| region.name.as_deref() == Some(name))
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [] => Err(ProjectionError::not_found(format!(
                "region `{name}` was not found"
            ))),
            [region] if region.addressable => Ok(*region),
            _ => Err(ProjectionError::ambiguous(format!(
                "region `{name}` is ambiguous"
            ))),
        }
    }

    pub(crate) fn method_body(
        &self,
        method_at: &str,
        context: Option<&str>,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<BodyPage, ProjectionError> {
        let method = self
            .methods
            .iter()
            .find(|method| method.at == method_at)
            .ok_or_else(|| {
                ProjectionError::not_found(format!("method `{method_at}` was not found"))
            })?;
        if let Some(context) = context {
            const CONTEXTS: &[&str] = &[
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
            if !CONTEXTS.contains(&context) {
                return Err(ProjectionError::invalid_context(format!(
                    "context `{context}` is not part of platform profile 8.3.27"
                )));
            }
            if !method
                .compile
                .contexts
                .iter()
                .any(|candidate| candidate == context)
            {
                return Ok(BodyPage {
                    lines: Vec::new(),
                    next: None,
                });
            }
        }

        let mut lines = self
            .body
            .iter()
            .filter(|line| line.line >= method.body_from_line && line.line <= method.body_to_line)
            .filter(|line| {
                let Some(context) = context else {
                    return true;
                };
                if line.text.trim_start().starts_with('#') {
                    return false;
                }
                let covering = self
                    .compilation
                    .iter()
                    .filter(|range| line.line >= range.from_line && line.line <= range.to_line)
                    .collect::<Vec<_>>();
                covering.is_empty()
                    || covering
                        .iter()
                        .all(|range| range.contexts.iter().any(|candidate| candidate == context))
            })
            .cloned()
            .collect::<Vec<_>>();
        let offset = cursor
            .unwrap_or("0")
            .parse::<usize>()
            .map_err(|_| ProjectionError::not_found("body cursor is invalid"))?;
        if offset > lines.len() {
            return Err(ProjectionError::not_found("body cursor is out of range"));
        }
        let end = offset.saturating_add(limit.max(1)).min(lines.len());
        let next = (end < lines.len()).then(|| end.to_string());
        lines = lines[offset..end].to_vec();
        Ok(BodyPage { lines, next })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BindingFact, BranchKind, CommonModuleProperties, CompilationFacts, EventProjection,
        EventState, MethodKind, MethodProjection, ModuleIdentity, ModuleProjectionSet,
        ModuleProperties, OperationCapability,
    };
    use crate::domain::address::{NodeKind, QualifiedAddress};
    use crate::domain::platform_profile::ModuleRole;
    use serde_json::{json, Value};

    fn identity(props: ModuleProperties) -> ModuleIdentity {
        ModuleIdentity {
            at: QualifiedAddress::parse("main:Document.Пустой.Module.Object").unwrap(),
            title: "Модуль объекта Пустой".to_string(),
            props,
            rev: "rev-1".to_string(),
        }
    }

    fn available_event() -> EventProjection {
        EventProjection {
            at: "main:Document.Пустой.Module.Object.Event.BeforeWrite".to_string(),
            event_id: "BeforeWrite".to_string(),
            state: EventState::Available,
            signature: "Процедура ПередЗаписью(Отказ, РежимЗаписи, РежимПроведения)".to_string(),
            context: "server".to_string(),
            binding: BindingFact::Platform,
            handler: "ПередЗаписью".to_string(),
            implementation_at: None,
            call_type: None,
            can: vec![OperationCapability::event_implement(
                "main:Document.Пустой.Module.Object.Event.BeforeWrite",
            )],
        }
    }

    fn assert_forbidden_keys_are_absent(value: &Value) {
        match value {
            Value::Object(map) => {
                for forbidden in ["sourceState", "fileExists", "set", "methods", "source"] {
                    assert!(
                        !map.contains_key(forbidden),
                        "forbidden key {forbidden}: {value}"
                    );
                }
                for child in map.values() {
                    assert_forbidden_keys_are_absent(child);
                }
            }
            Value::Array(items) => {
                for item in items {
                    assert_forbidden_keys_are_absent(item);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn serialized_module_projection_shape_is_stable() {
        let projection = ModuleProjectionSet::new(
            identity(ModuleProperties::new(
                NodeKind::Document,
                ModuleRole::Object,
            )),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![available_event()],
            Vec::new(),
            Vec::new(),
        );

        let actual = serde_json::to_value(projection.summary()).unwrap();
        assert_eq!(
            actual,
            json!({
                "at": "main:Document.Пустой.Module.Object",
                "kind": "Module",
                "title": "Модуль объекта Пустой",
                "props": {"ownerKind": "Document", "role": "Object"},
                "branches": [
                    {"at": "main:Document.Пустой.Module.Object.Method", "count": 0},
                    {"at": "main:Document.Пустой.Module.Object.Region", "count": 0},
                    {"at": "main:Document.Пустой.Module.Object.Interface", "count": 0},
                    {"at": "main:Document.Пустой.Module.Object.Event", "count": 1},
                    {"at": "main:Document.Пустой.Module.Object.Compilation", "count": 0},
                    {"at": "main:Document.Пустой.Module.Object.Body", "count": 0}
                ],
                "rev": "rev-1"
            })
        );
        assert_eq!(
            projection
                .summary()
                .branches
                .iter()
                .map(|branch| branch.kind)
                .collect::<Vec<_>>(),
            BranchKind::ALL
        );
        assert_forbidden_keys_are_absent(&actual);
        valid_missing_file_keeps_truthful_event_navigation_and_no_source_facts();
        common_module_flags_serialize_exactly_once_and_never_become_contexts();
    }

    #[test]
    fn valid_missing_file_keeps_truthful_event_navigation_and_no_source_facts() {
        let projection = ModuleProjectionSet::new(
            identity(ModuleProperties::new(
                NodeKind::Document,
                ModuleRole::Object,
            )),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![available_event()],
            Vec::new(),
            Vec::new(),
        );

        let counts = projection
            .summary()
            .branches
            .iter()
            .map(|branch| (branch.kind, branch.count))
            .collect::<Vec<_>>();
        assert_eq!(
            counts,
            vec![
                (BranchKind::Method, 0),
                (BranchKind::Region, 0),
                (BranchKind::Interface, 0),
                (BranchKind::Event, 1),
                (BranchKind::Compilation, 0),
                (BranchKind::Body, 0),
            ]
        );
        assert_eq!(projection.events().len(), 1);
        assert_eq!(projection.events()[0].state, EventState::Available);
        assert_eq!(projection.events()[0].can.len(), 1);
        assert_eq!(projection.events()[0].can[0].op, "event.implement");
    }

    #[test]
    fn common_module_flags_serialize_exactly_once_and_never_become_contexts() {
        let props = ModuleProperties::common(CommonModuleProperties {
            global: false,
            client_managed_application: true,
            server: true,
            external_connection: true,
            client_ordinary_application: false,
            server_call: false,
            privileged: false,
            return_values_reuse: "DuringRequest".to_string(),
        });
        let mut common_identity = identity(props);
        common_identity.at = QualifiedAddress::parse("main:CommonModule.ЗаказыСервер").unwrap();
        common_identity.title = "Общий модуль ЗаказыСервер".to_string();
        let projection = ModuleProjectionSet::new(
            common_identity,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );

        let serialized = serde_json::to_string(projection.summary()).unwrap();
        for key in [
            "global",
            "clientManagedApplication",
            "server",
            "externalConnection",
            "clientOrdinaryApplication",
            "serverCall",
            "privileged",
            "returnValuesReuse",
        ] {
            assert_eq!(
                serialized.matches(&format!("\"{key}\"")).count(),
                1,
                "{key}"
            );
        }
        assert!(!serialized.contains("contexts"));
    }

    #[test]
    fn event_state_vocabulary_and_binding_facts_are_closed() {
        let states = [
            EventState::Available,
            EventState::Implemented,
            EventState::Missing,
            EventState::Invalid,
        ];
        assert_eq!(
            serde_json::to_value(states).unwrap(),
            json!(["available", "implemented", "missing", "invalid"])
        );
        assert_eq!(
            serde_json::to_value([
                BindingFact::Platform,
                BindingFact::Property,
                BindingFact::Code,
                BindingFact::Subscription,
            ])
            .unwrap(),
            json!(["platform", "property", "code", "subscription"])
        );
    }

    #[test]
    fn method_without_body_reports_a_zero_body_branch_and_no_text() {
        let method = MethodProjection {
            at: "main:HTTPService.API.Module.HTTPService.Method.get".to_string(),
            name: "get".to_string(),
            signature: "Процедура get()".to_string(),
            doc: None,
            method_kind: MethodKind::Procedure,
            export: false,
            compile: CompilationFacts {
                directive: None,
                guard: None,
                contexts: vec!["server".to_string()],
                form_context: None,
                conditional_body: false,
            },
            handles: Vec::new(),
            extension: None,
            body_from_line: 1,
            body_to_line: 0,
        };
        let value = serde_json::to_value(method).unwrap();
        assert_eq!(value["branches"][1]["count"], 0);
        assert!(value.get("body").is_none());
        assert!(value.get("source").is_none());
    }
}
