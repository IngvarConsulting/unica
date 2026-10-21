use serde::ser::{SerializeStruct, Serializer};
use serde::Serialize;
use std::fmt;
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_address::module_routes::ModuleRole;
use unica_metadata::event::BindingFact;
use unica_operation_contract::OperationRef;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum BranchKind {
    Method,
    Region,
    Interface,
    Event,
    Compilation,
    Body,
}

impl BranchKind {
    pub const ALL: [Self; 6] = [
        Self::Method,
        Self::Region,
        Self::Interface,
        Self::Event,
        Self::Compilation,
        Self::Body,
    ];

    pub const fn as_str(self) -> &'static str {
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
/// Свойства общего модуля из его дескриптора. `None` — свойство не
/// записано: так платформа выгружает заимствованный модуль расширения,
/// не перенося в него значения родителя. У обычного модуля читатель
/// требует все свойства.
pub struct CommonModuleProperties {
    pub global: Option<bool>,
    pub client_managed_application: Option<bool>,
    pub server: Option<bool>,
    pub external_connection: Option<bool>,
    pub client_ordinary_application: Option<bool>,
    pub server_call: Option<bool>,
    pub privileged: Option<bool>,
    pub return_values_reuse: Option<String>,
}

impl CommonModuleProperties {
    /// Известны ли все четыре флага среды, из которых складываются
    /// контексты компиляции. Без любого из них контексты неизвестны.
    pub fn execution_environment_known(&self) -> bool {
        self.client_managed_application.is_some()
            && self.client_ordinary_application.is_some()
            && self.server.is_some()
            && self.external_connection.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleProperties {
    pub owner_kind: String,
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub common_module: Option<CommonModuleProperties>,
}

impl ModuleProperties {
    pub fn new(owner_kind: NodeKind, role: ModuleRole) -> Self {
        Self {
            owner_kind: owner_kind.as_str().to_string(),
            role: role.as_str().to_string(),
            common_module: None,
        }
    }

    pub fn common(common_module: CommonModuleProperties) -> Self {
        Self {
            owner_kind: NodeKind::CommonModule.as_str().to_string(),
            role: ModuleRole::Common.as_str().to_string(),
            common_module: Some(common_module),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleIdentity {
    pub at: QualifiedAddress,
    pub title: String,
    pub props: ModuleProperties,
    pub rev: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BranchSummary {
    #[serde(skip)]
    pub kind: BranchKind,
    pub at: String,
    pub count: usize,
    /// Размер текста ветви в байтах UTF-8; есть только у `Body` модуля
    /// с исходным файлом.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModuleSummary {
    pub at: String,
    pub kind: &'static str,
    pub title: String,
    pub props: ModuleProperties,
    pub branches: Vec<BranchSummary>,
    pub rev: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MethodKind {
    Procedure,
    Function,
}

impl MethodKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Procedure => "procedure",
            Self::Function => "function",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompilationFacts {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directive: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guard: Option<String>,
    /// `None` — контексты неизвестны: заимствованный общий модуль не
    /// записал флаги среды. Сериализуется явным `null`.
    pub contexts: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form_context: Option<bool>,
    pub conditional_body: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandleProjection {
    pub owner: String,
    pub event: String,
    pub at: String,
    pub binding: BindingFact,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_type: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ExtensionKind {
    Before,
    After,
    Instead,
    ChangeAndValidate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionProjection {
    pub kind: ExtensionKind,
    pub directive: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_at: Option<String>,
}

/// One declared method parameter: its name without `Знач` and whether it has
/// a default value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodParameter {
    pub name: String,
    pub optional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodProjection {
    pub at: String,
    pub name: String,
    pub signature: String,
    /// Declared parameters from the parser, in order. Not serialized: the
    /// signature already shows them.
    pub parameters: Vec<MethodParameter>,
    pub doc: Option<String>,
    pub method_kind: MethodKind,
    pub export: bool,
    pub compile: CompilationFacts,
    pub handles: Vec<HandleProjection>,
    pub extension: Option<ExtensionProjection>,
    pub compilation_count: usize,
    /// Physical lines of the whole method: from its first annotation or
    /// compilation directive to the closing keyword. Body lines lie inside.
    pub from_line: usize,
    pub to_line: usize,
    /// Первая строка текста `Method.<имя>.Body`: начало документирующего
    /// комментария непосредственно над методом, иначе `from_line`.
    pub text_from_line: usize,
    pub body_from_line: usize,
    pub body_to_line: usize,
}

impl MethodProjection {
    /// Число строк текста `Method.<имя>.Body`: комментарий, директива,
    /// объявление, тело и строка окончания.
    pub fn text_line_count(&self) -> usize {
        (self.to_line + 1).saturating_sub(self.text_from_line)
    }
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
                    count: self.compilation_count,
                    bytes: None,
                },
                BranchSummary {
                    kind: BranchKind::Body,
                    at: format!("{}.Body", self.at),
                    count: self.text_line_count(),
                    bytes: None,
                },
            ],
        )?;
        state.end()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionProjection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    pub name: Option<String>,
    pub addressable: bool,
    pub line: usize,
    pub end_line: Option<usize>,
    pub methods: Vec<String>,
    pub children: Vec<String>,
    #[serde(skip)]
    pub parent: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum InterfaceKind {
    Public,
    Library,
    Override,
}

impl InterfaceKind {
    pub const ALL: [Self; 3] = [Self::Public, Self::Library, Self::Override];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Public => "Public",
            Self::Library => "Library",
            Self::Override => "Override",
        }
    }
}

/// Срез методов модуля по нормативным областям. У среза нет адреса:
/// у `Interface` не бывает имени, и `…Interface.Public` грамматика адреса
/// отвергает (CTR.SOURCE.QUALIFIED-LOGICAL-ADDRESS). Срез — элемент ветви
/// `…Interface`, а его методы адресуются своими `…Method.<имя>` (#1467).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InterfaceProjection {
    pub kind: &'static str,
    pub interface: InterfaceKind,
    pub methods: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EventState {
    Available,
    Implemented,
    Missing,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventProjection {
    pub at: String,
    pub event_id: String,
    pub state: EventState,
    pub signature: String,
    pub contexts: Vec<String>,
    pub binding: BindingFact,
    pub handler: String,
    pub handler_en: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implementation_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_type: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub can: Vec<OperationRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompilationProjection {
    pub from_line: usize,
    pub to_line: usize,
    pub guard: String,
    /// `None` — контексты модуля неизвестны, как у `CompilationFacts`.
    pub contexts: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BodyLine {
    pub line: usize,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionError {
    code: &'static str,
    message: String,
}

impl ProjectionError {
    pub fn ambiguous(message: impl Into<String>) -> Self {
        Self {
            code: "ambiguous_address",
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            code: "not_found",
            message: message.into(),
        }
    }

    pub fn invalid_context(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_context",
            message: message.into(),
        }
    }

    pub fn is_ambiguous(&self) -> bool {
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
pub struct ModuleProjectionSet {
    summary: ModuleSummary,
    methods: Vec<MethodProjection>,
    regions: Vec<RegionProjection>,
    interfaces: Vec<InterfaceProjection>,
    events: Vec<EventProjection>,
    compilation: Vec<CompilationProjection>,
    body: Vec<BodyLine>,
    source: String,
}

impl ModuleProjectionSet {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
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
                bytes: None,
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
            source: String::new(),
        }
    }

    /// Исходный текст модуля, из которого построена проекция.
    pub fn with_source(mut self, source: String) -> Self {
        if let Some(body) = self
            .summary
            .branches
            .iter_mut()
            .find(|branch| branch.kind == BranchKind::Body)
        {
            body.bytes = Some(source.len() as u64);
        }
        self.source = source;
        self
    }

    /// Текст модуля для `Body`: файл без начальных U+FEFF, разделители строк
    /// как в файле.
    pub fn source_text(&self) -> &str {
        &self.source
    }

    /// Текст `Method.<имя>.Body`: строки от документирующего комментария до
    /// строки окончания метода включительно, каждая со своим разделителем.
    pub fn method_text(&self, method: &MethodProjection) -> &str {
        line_span(&self.source, method.text_from_line, method.to_line)
    }

    pub const fn summary(&self) -> &ModuleSummary {
        &self.summary
    }

    pub fn methods(&self) -> &[MethodProjection] {
        &self.methods
    }

    pub fn regions(&self) -> &[RegionProjection] {
        &self.regions
    }

    pub fn interfaces(&self) -> &[InterfaceProjection] {
        &self.interfaces
    }

    pub fn events(&self) -> &[EventProjection] {
        &self.events
    }

    pub fn compilation(&self) -> &[CompilationProjection] {
        &self.compilation
    }

    pub fn body(&self) -> &[BodyLine] {
        &self.body
    }

    pub fn region(&self, name: &str) -> Result<&RegionProjection, ProjectionError> {
        if name.contains(".Region.") {
            return self
                .regions
                .iter()
                .find(|region| region.at.as_deref() == Some(name))
                .ok_or_else(|| {
                    ProjectionError::not_found(format!("region `{name}` was not found"))
                });
        }
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
}

/// Строки `from..=to` (с единицы) вместе с их разделителями.
fn line_span(text: &str, from: usize, to: usize) -> &str {
    if from == 0 || to < from {
        return "";
    }
    let mut line = 1;
    let mut start = (from == 1).then_some(0);
    let mut end = text.len();
    for (index, byte) in text.bytes().enumerate() {
        if byte != b'\n' {
            continue;
        }
        if line == to {
            end = index + 1;
            break;
        }
        line += 1;
        if line == from {
            start = Some(index + 1);
        }
    }
    start.map_or("", |start| &text[start..end])
}

#[cfg(test)]
mod tests {
    use super::{
        BindingFact, BodyLine, BranchKind, CommonModuleProperties, CompilationFacts,
        CompilationProjection, EventProjection, EventState, ExtensionKind, ExtensionProjection,
        HandleProjection, InterfaceKind, InterfaceProjection, MethodKind, MethodParameter,
        MethodProjection, ModuleIdentity, ModuleProjectionSet, ModuleProperties, RegionProjection,
    };
    use serde_json::{json, Value};
    use unica_address::address::{NodeKind, QualifiedAddress};
    use unica_address::module_routes::ModuleRole;

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
            contexts: vec!["server".to_string()],
            binding: BindingFact::Platform,
            handler: "ПередЗаписью".to_string(),
            handler_en: "BeforeWrite".to_string(),
            implementation_at: None,
            call_type: None,
            can: vec![
                unica_metadata::event::EventOperation::implementation_skeleton(
                    "main:Document.Пустой.Module.Object.Event.BeforeWrite",
                    None,
                ),
            ],
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
        assert_eq!(projection.events()[0].can[0].op(), "event.implement");
    }

    #[test]
    fn event_capability_is_the_registry_copyable_apply_skeleton() {
        let capability = serde_json::to_value(&available_event().can[0]).unwrap();
        assert_eq!(
            capability,
            json!({
                "op": "event.implement",
                "args": {
                    "at": "main:Document.Пустой.Module.Object.Event.BeforeWrite"
                }
            })
        );
    }

    #[test]
    fn serialized_module_projection_contract_is_complete() {
        let base = "main:Document.Заказ.Module.Object";
        let method_at = format!("{base}.Method.Проверить");
        let method = MethodProjection {
            at: method_at.clone(),
            name: "Проверить".to_string(),
            signature: "Процедура Проверить(Отказ) Экспорт".to_string(),
            parameters: vec![MethodParameter {
                name: "Отказ".to_string(),
                optional: false,
            }],
            doc: Some("Проверяет заказ.".to_string()),
            method_kind: MethodKind::Procedure,
            export: true,
            compile: CompilationFacts {
                directive: Some("&НаСервере".to_string()),
                guard: Some("Server".to_string()),
                contexts: Some(vec!["server".to_string()]),
                form_context: None,
                conditional_body: true,
            },
            handles: vec![HandleProjection {
                owner: "platform".to_string(),
                event: "BeforeWrite".to_string(),
                at: format!("{base}.Event.BeforeWrite"),
                binding: BindingFact::Platform,
                call_type: None,
            }],
            extension: Some(ExtensionProjection {
                kind: ExtensionKind::Instead,
                directive: "&Вместо(\"Проверить\")".to_string(),
                target_at: Some("main:Document.Заказ.Module.Object.Method.Проверить".to_string()),
            }),
            compilation_count: 1,
            from_line: 5,
            to_line: 9,
            text_from_line: 4,
            body_from_line: 7,
            body_to_line: 8,
        };
        let region = RegionProjection {
            at: Some(format!("{base}.Region.ПрограммныйИнтерфейс")),
            name: Some("ПрограммныйИнтерфейс".to_string()),
            addressable: true,
            line: 1,
            end_line: Some(9),
            methods: vec![method_at.clone()],
            children: Vec::new(),
            parent: None,
        };
        let interface = InterfaceProjection {
            kind: "Interface",
            interface: InterfaceKind::Public,
            methods: vec![method_at.clone()],
        };
        let event = EventProjection {
            at: format!("{base}.Event.BeforeWrite"),
            event_id: "BeforeWrite".to_string(),
            state: EventState::Implemented,
            signature: "Процедура ПередЗаписью(Отказ, РежимЗаписи, РежимПроведения)".to_string(),
            contexts: vec!["server".to_string()],
            binding: BindingFact::Platform,
            handler: "ПередЗаписью".to_string(),
            handler_en: "BeforeWrite".to_string(),
            implementation_at: Some(method_at.clone()),
            call_type: None,
            can: Vec::new(),
        };
        let compilation = CompilationProjection {
            from_line: 7,
            to_line: 7,
            guard: "Server".to_string(),
            contexts: Some(vec!["server".to_string()]),
        };
        let body = [
            BodyLine {
                line: 7,
                text: "    Отказ = Ложь;".to_string(),
            },
            BodyLine {
                line: 8,
                text: "    Возврат;".to_string(),
            },
        ];
        let mut full_identity = identity(ModuleProperties::new(
            NodeKind::Document,
            ModuleRole::Object,
        ));
        full_identity.at = QualifiedAddress::parse(base).unwrap();
        full_identity.title = "Модуль объекта Заказ".to_string();
        let projection = ModuleProjectionSet::new(
            full_identity,
            vec![method],
            vec![region],
            vec![interface],
            vec![event],
            vec![compilation],
            body.to_vec(),
        );

        let summary = serde_json::to_value(projection.summary()).unwrap();
        assert_eq!(
            summary,
            json!({
                "at": base,
                "kind": "Module",
                "title": "Модуль объекта Заказ",
                "props": {"ownerKind": "Document", "role": "Object"},
                "branches": [
                    {"at": format!("{base}.Method"), "count": 1},
                    {"at": format!("{base}.Region"), "count": 1},
                    {"at": format!("{base}.Interface"), "count": 1},
                    {"at": format!("{base}.Event"), "count": 1},
                    {"at": format!("{base}.Compilation"), "count": 1},
                    {"at": format!("{base}.Body"), "count": 2}
                ],
                "rev": "rev-1"
            })
        );
        assert_eq!(
            serde_json::to_value(&projection.methods()[0]).unwrap(),
            json!({
                "at": method_at,
                "kind": "Method",
                "props": {
                    "signature": "Процедура Проверить(Отказ) Экспорт",
                    "doc": "Проверяет заказ.",
                    "methodKind": "procedure",
                    "export": true,
                    "compile": {
                        "directive": "&НаСервере",
                        "guard": "Server",
                        "contexts": ["server"],
                        "conditionalBody": true
                    },
                    "handles": [{
                        "owner": "platform",
                        "event": "BeforeWrite",
                        "at": format!("{base}.Event.BeforeWrite"),
                        "binding": "platform"
                    }],
                    "extension": {
                        "kind": "instead",
                        "directive": "&Вместо(\"Проверить\")",
                        "targetAt": "main:Document.Заказ.Module.Object.Method.Проверить"
                    }
                },
                "branches": [
                    {"at": format!("{base}.Method.Проверить.Compilation"), "count": 1},
                    {"at": format!("{base}.Method.Проверить.Body"), "count": 6}
                ]
            })
        );
        assert_eq!(
            serde_json::to_value(&projection.regions()[0]).unwrap(),
            json!({
                "at": format!("{base}.Region.ПрограммныйИнтерфейс"),
                "name": "ПрограммныйИнтерфейс",
                "addressable": true,
                "line": 1,
                "endLine": 9,
                "methods": [format!("{base}.Method.Проверить")],
                "children": []
            })
        );
        assert_eq!(
            serde_json::to_value(&projection.interfaces()[0]).unwrap(),
            json!({
                "kind": "Interface",
                "interface": "Public",
                "methods": [format!("{base}.Method.Проверить")]
            })
        );
        assert_eq!(
            serde_json::to_value(&projection.events()[0]).unwrap(),
            json!({
                "at": format!("{base}.Event.BeforeWrite"),
                "eventId": "BeforeWrite",
                "state": "implemented",
                "signature": "Процедура ПередЗаписью(Отказ, РежимЗаписи, РежимПроведения)",
                "contexts": ["server"],
                "binding": "platform",
                "handler": "ПередЗаписью",
                "handlerEn": "BeforeWrite",
                "implementationAt": format!("{base}.Method.Проверить")
            })
        );
        assert_eq!(
            serde_json::to_value(&projection.compilation()[0]).unwrap(),
            json!({
                "fromLine": 7,
                "toLine": 7,
                "guard": "Server",
                "contexts": ["server"]
            })
        );
        // Текст метода берётся из файла байт в байт: разделители строк
        // сохраняются, а строки до комментария и после окончания не входят.
        let source = "#Область ПрограммныйИнтерфейс\n\n\r\n// Проверяет заказ.\r\n&НаСервере\nПроцедура Проверить(Отказ) Экспорт\n    Отказ = Ложь;\n    Возврат;\nКонецПроцедуры\n#КонецОбласти\n";
        let projection = projection.with_source(source.to_string());
        assert_eq!(projection.source_text(), source);
        assert_eq!(
            projection.method_text(&projection.methods()[0]),
            "// Проверяет заказ.\r\n&НаСервере\nПроцедура Проверить(Отказ) Экспорт\n    Отказ = Ложь;\n    Возврат;\nКонецПроцедуры\n"
        );
        assert_forbidden_keys_are_absent(&summary);
    }

    #[test]
    fn common_module_flags_serialize_exactly_once_and_never_become_contexts() {
        let props = ModuleProperties::common(CommonModuleProperties {
            global: Some(false),
            client_managed_application: Some(true),
            server: Some(true),
            external_connection: Some(true),
            client_ordinary_application: Some(false),
            server_call: Some(false),
            privileged: Some(false),
            return_values_reuse: Some("DuringRequest".to_string()),
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
    fn method_with_an_empty_body_still_counts_its_text_lines_and_inlines_no_text() {
        let method = MethodProjection {
            at: "main:HTTPService.API.Module.HTTPService.Method.get".to_string(),
            name: "get".to_string(),
            signature: "Процедура get()".to_string(),
            parameters: Vec::new(),
            doc: None,
            method_kind: MethodKind::Procedure,
            export: false,
            compile: CompilationFacts {
                directive: None,
                guard: None,
                contexts: Some(vec!["server".to_string()]),
                form_context: None,
                conditional_body: false,
            },
            handles: Vec::new(),
            extension: None,
            compilation_count: 0,
            from_line: 1,
            to_line: 1,
            text_from_line: 1,
            body_from_line: 1,
            body_to_line: 0,
        };
        let value = serde_json::to_value(method).unwrap();
        // Текст `Body` метода включает объявление и строку окончания, поэтому
        // у метода без тела ветвь не пуста.
        assert_eq!(value["branches"][1]["count"], 1);
        assert!(value.get("body").is_none());
        assert!(value.get("source").is_none());
    }
}
