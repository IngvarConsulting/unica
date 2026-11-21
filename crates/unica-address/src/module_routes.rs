//! Structural module routes in the logical address grammar.
//! A route identifies an owner and role; profiles decide semantic admission,
//! and source adapters map admitted routes to physical artifacts.

use crate::address::{AddressSegment, NodeKind, QualifiedAddress};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleRole {
    Object,
    Manager,
    RecordSet,
    ValueManager,
    Common,
    Form,
    Command,
    ManagedApplication,
    OrdinaryApplication,
    Session,
    ExternalConnection,
    HttpService,
    WebService,
    IntegrationService,
    Bot,
    WebSocketClient,
}

impl ModuleRole {
    pub const ALL: [Self; 16] = [
        Self::Object,
        Self::Manager,
        Self::RecordSet,
        Self::ValueManager,
        Self::Common,
        Self::Form,
        Self::Command,
        Self::ManagedApplication,
        Self::OrdinaryApplication,
        Self::Session,
        Self::ExternalConnection,
        Self::HttpService,
        Self::WebService,
        Self::IntegrationService,
        Self::Bot,
        Self::WebSocketClient,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Object => "Object",
            Self::Manager => "Manager",
            Self::RecordSet => "RecordSet",
            Self::ValueManager => "ValueManager",
            Self::Common => "Common",
            Self::Form => "Form",
            Self::Command => "Command",
            Self::ManagedApplication => "ManagedApplication",
            Self::OrdinaryApplication => "OrdinaryApplication",
            Self::Session => "Session",
            Self::ExternalConnection => "ExternalConnection",
            Self::HttpService => "HTTPService",
            Self::WebService => "WebService",
            Self::IntegrationService => "IntegrationService",
            Self::Bot => "Bot",
            Self::WebSocketClient => "WebSocketClient",
        }
    }

    pub fn from_v12_terminal(value: &str) -> Option<Self> {
        match value {
            "ObjectModule" => Some(Self::Object),
            "ManagerModule" => Some(Self::Manager),
            "RecordSetModule" => Some(Self::RecordSet),
            "ValueManagerModule" => Some(Self::ValueManager),
            _ => None,
        }
    }

    fn from_semantic_name(value: &str) -> Option<Self> {
        match value {
            "Object" => Some(Self::Object),
            "Manager" => Some(Self::Manager),
            "RecordSet" => Some(Self::RecordSet),
            "ValueManager" => Some(Self::ValueManager),
            "Form" => Some(Self::Form),
            "Command" => Some(Self::Command),
            "ManagedApplication" => Some(Self::ManagedApplication),
            "OrdinaryApplication" => Some(Self::OrdinaryApplication),
            "Session" => Some(Self::Session),
            "ExternalConnection" => Some(Self::ExternalConnection),
            "HTTPService" => Some(Self::HttpService),
            "WebService" => Some(Self::WebService),
            "IntegrationService" => Some(Self::IntegrationService),
            "Bot" => Some(Self::Bot),
            "WebSocketClient" => Some(Self::WebSocketClient),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleRouteKind {
    Root,
    Direct,
    Common,
    CommonForm,
    CommonCommand,
    NestedForm,
    NestedCommand,
    /// A module of a nested object that declares its place in its owner.
    NestedObject,
    Service,
    Bot,
    WebSocketClient,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModuleRoute {
    owner_kind: NodeKind,
    role: ModuleRole,
    route_kind: ModuleRouteKind,
}

impl ModuleRoute {
    pub const fn owner_kind(self) -> NodeKind {
        self.owner_kind
    }
    pub const fn role(self) -> ModuleRole {
        self.role
    }
    pub const fn kind(self) -> ModuleRouteKind {
        self.route_kind
    }
}

pub fn module_route(address: &QualifiedAddress) -> Option<ModuleRoute> {
    module_route_for_segments(address.segments())
}

pub fn module_prefix_route(address: &QualifiedAddress) -> Option<ModuleRoute> {
    let segments = address.segments();
    (1..=segments.len()).rev().find_map(|length| {
        let route = module_route_for_segments(&segments[..length])?;
        module_projection_suffix_is_valid(&segments[length..], route.role()).then_some(route)
    })
}

pub fn has_direct_module_route(owner_kind: NodeKind, role: ModuleRole) -> bool {
    match role {
        ModuleRole::Object => matches!(
            owner_kind,
            NodeKind::Catalog
                | NodeKind::Document
                | NodeKind::ExchangePlan
                | NodeKind::ChartOfAccounts
                | NodeKind::ChartOfCharacteristicTypes
                | NodeKind::ChartOfCalculationTypes
                | NodeKind::BusinessProcess
                | NodeKind::Task
                | NodeKind::Report
                | NodeKind::DataProcessor
                | NodeKind::ExternalDataProcessor
                | NodeKind::ExternalReport
        ),
        ModuleRole::Manager => matches!(
            owner_kind,
            NodeKind::Catalog
                | NodeKind::Document
                | NodeKind::InformationRegister
                | NodeKind::AccumulationRegister
                | NodeKind::AccountingRegister
                | NodeKind::CalculationRegister
                | NodeKind::ChartOfAccounts
                | NodeKind::ChartOfCharacteristicTypes
                | NodeKind::ChartOfCalculationTypes
                | NodeKind::BusinessProcess
                | NodeKind::Task
                | NodeKind::ExchangePlan
                | NodeKind::Enum
                | NodeKind::Report
                | NodeKind::DataProcessor
                | NodeKind::Constant
                | NodeKind::DocumentJournal
                | NodeKind::FilterCriterion
                | NodeKind::SettingsStorage
                // Очередь держит только модуль менеджера: выгрузка 8.5.4
                // кладёт его в `Queues/<Имя>/Ext/ManagerModule.bsl`,
                // модуля объекта у неё нет — её сообщение запись, не ссылка.
                | NodeKind::Queue
        ),
        ModuleRole::RecordSet => matches!(
            owner_kind,
            NodeKind::InformationRegister
                | NodeKind::AccumulationRegister
                | NodeKind::AccountingRegister
                | NodeKind::CalculationRegister
                | NodeKind::Sequence
        ),
        ModuleRole::ValueManager => owner_kind == NodeKind::Constant,
        _ => false,
    }
}

pub fn has_nested_module_route(owner_kind: NodeKind) -> bool {
    has_nested_module_role(owner_kind, ModuleRole::Form)
        || has_nested_module_role(owner_kind, ModuleRole::Command)
}

/// Nested roles share their owner matrix with physical source layouts.
/// Settings storage has forms, but does not declare nested commands.
pub fn has_nested_module_role(owner_kind: NodeKind, role: ModuleRole) -> bool {
    if owner_kind == NodeKind::SettingsStorage {
        return role == ModuleRole::Form;
    }
    if !matches!(role, ModuleRole::Form | ModuleRole::Command) {
        return false;
    }
    matches!(
        owner_kind,
        NodeKind::Document
            | NodeKind::Catalog
            | NodeKind::DataProcessor
            | NodeKind::Report
            | NodeKind::InformationRegister
            | NodeKind::AccumulationRegister
            | NodeKind::AccountingRegister
            | NodeKind::CalculationRegister
            | NodeKind::ChartOfAccounts
            | NodeKind::ChartOfCharacteristicTypes
            | NodeKind::ChartOfCalculationTypes
            | NodeKind::ExchangePlan
            | NodeKind::BusinessProcess
            | NodeKind::Task
            | NodeKind::DocumentJournal
            | NodeKind::Enum
            | NodeKind::Constant
            | NodeKind::Sequence
            | NodeKind::DocumentNumerator
            | NodeKind::ExternalDataProcessor
            | NodeKind::ExternalReport
    )
}
fn module_projection_suffix_is_valid(suffix: &[AddressSegment], role: ModuleRole) -> bool {
    match suffix {
        [] => true,
        [branch] if matches!(branch.kind(), NodeKind::Body) && branch.name().is_none() => true,
        [branch]
            if matches!(
                branch.kind(),
                NodeKind::Method | NodeKind::Interface | NodeKind::Event | NodeKind::Variable
            ) =>
        {
            true
        }
        [method, projection]
            if method.kind() == NodeKind::Method
                && method.name().is_some()
                && (projection.kind() == NodeKind::Body
                    || (projection.kind() == NodeKind::Binding
                        && matches!(role, ModuleRole::Form | ModuleRole::Common)))
                && projection.name().is_none() =>
        {
            true
        }
        regions
            if regions.iter().enumerate().all(|(index, region)| {
                region.kind() == NodeKind::Region
                    && (region.name().is_some() || index + 1 == regions.len())
            }) =>
        {
            true
        }
        _ => false,
    }
}

pub(crate) fn module_route_for_segments(segments: &[AddressSegment]) -> Option<ModuleRoute> {
    if let Some(route) = nested_object_module(segments) {
        return Some(route);
    }
    match segments {
        [owner] if owner.name().is_some() && owner.kind() == NodeKind::CommonModule => Some(route(
            NodeKind::CommonModule,
            ModuleRole::Common,
            ModuleRouteKind::Common,
        )),
        [module] if module.kind() == NodeKind::Module => {
            let role = ModuleRole::from_semantic_name(module.name()?)?;
            matches!(
                role,
                ModuleRole::ManagedApplication
                    | ModuleRole::OrdinaryApplication
                    | ModuleRole::Session
                    | ModuleRole::ExternalConnection
            )
            .then(|| route(NodeKind::Configuration, role, ModuleRouteKind::Root))
        }
        [owner, module]
            if owner.name().is_some()
                && module.kind() == NodeKind::Module
                && module.name().is_some() =>
        {
            direct_or_named_module(owner.kind(), module.name()?)
        }
        [owner, child, module]
            if owner.name().is_some()
                && child.name().is_some()
                && module.kind() == NodeKind::Module
                && module.name().is_some() =>
        {
            nested_module(owner.kind(), child.kind(), module.name()?)
        }
        _ => None,
    }
}

fn direct_or_named_module(owner: NodeKind, raw_role: &str) -> Option<ModuleRoute> {
    let role = ModuleRole::from_semantic_name(raw_role)?;
    if has_direct_module_route(owner, role) {
        return Some(route(owner, role, ModuleRouteKind::Direct));
    }
    match (owner, role) {
        (NodeKind::CommonForm, ModuleRole::Form) => {
            Some(route(owner, role, ModuleRouteKind::CommonForm))
        }
        (NodeKind::CommonCommand, ModuleRole::Command) => {
            Some(route(owner, role, ModuleRouteKind::CommonCommand))
        }
        (NodeKind::HttpService, ModuleRole::HttpService)
        | (NodeKind::WebService, ModuleRole::WebService)
        | (NodeKind::IntegrationService, ModuleRole::IntegrationService) => {
            Some(route(owner, role, ModuleRouteKind::Service))
        }
        (NodeKind::Bot, ModuleRole::Bot) => Some(route(owner, role, ModuleRouteKind::Bot)),
        (NodeKind::WebSocketClient, ModuleRole::WebSocketClient) => {
            Some(route(owner, role, ModuleRouteKind::WebSocketClient))
        }
        _ => None,
    }
}

/// `<Владелец>.<Имя>{.<Вид>.<Имя>}.Module.<Роль>` where every nested link is
/// a kind that declares its place in the previous one and the role is one of
/// the modules the last kind declares.
fn nested_object_module(segments: &[AddressSegment]) -> Option<ModuleRoute> {
    let [chain @ .., module] = segments else {
        return None;
    };
    if chain.len() < 2 || module.kind() != NodeKind::Module {
        return None;
    }
    if chain.iter().any(|segment| segment.name().is_none()) {
        return None;
    }
    let mut place = None;
    for pair in chain.windows(2) {
        place = Some(crate::kinds::nested_place(
            pair[0].kind().as_str(),
            pair[1].kind().as_str(),
        )?);
    }
    let role = ModuleRole::from_semantic_name(module.name()?)?;
    let terminal = format!("{}Module", role.as_str());
    place?.modules.contains(&terminal.as_str()).then(|| {
        route(
            chain[chain.len() - 1].kind(),
            role,
            ModuleRouteKind::NestedObject,
        )
    })
}

fn nested_module(owner: NodeKind, child: NodeKind, raw_role: &str) -> Option<ModuleRoute> {
    let role = ModuleRole::from_semantic_name(raw_role)?;
    if !has_nested_module_role(owner, role) {
        return None;
    }
    match (child, role) {
        (NodeKind::Form, ModuleRole::Form) => {
            Some(route(owner, ModuleRole::Form, ModuleRouteKind::NestedForm))
        }
        (NodeKind::Command, ModuleRole::Command) => Some(route(
            owner,
            ModuleRole::Command,
            ModuleRouteKind::NestedCommand,
        )),
        _ => None,
    }
}

const fn route(owner_kind: NodeKind, role: ModuleRole, route_kind: ModuleRouteKind) -> ModuleRoute {
    ModuleRoute {
        owner_kind,
        role,
        route_kind,
    }
}

#[cfg(test)]
mod tests {
    use super::{module_prefix_route, module_route, ModuleRole};
    use crate::address::QualifiedAddress;
    use serde::Deserialize;

    /// rule: CTR.SOURCE.QUALIFIED-LOGICAL-ADDRESS
    #[test]
    fn binding_is_an_unnamed_branch_of_a_form_method() {
        for at in [
            "main:CommonForm.Main.Module.Form.Method.Handler.Binding",
            "ext:Report.Example.Form.Main.Module.Form.Method.Handler.Binding",
        ] {
            let address = QualifiedAddress::parse(at).unwrap();
            assert_eq!(
                module_prefix_route(&address).unwrap().role(),
                ModuleRole::Form
            );
        }
        for at in [
            "main:CommonForm.Main.Module.Form.Binding",
            "main:CommonForm.Main.Module.Form.Method.Handler.Binding.Name",
            "main:CommonForm.Main.Module.Form.Method.Handler.Binding.Event.OnOpen",
            "main:Catalog.Main.Module.Object.Method.Handler.Binding",
        ] {
            let module = QualifiedAddress::parse(at)
                .ok()
                .as_ref()
                .and_then(module_prefix_route);
            assert_eq!(module, None, "{at}");
        }
    }

    /// rule: CTR.SOURCE.QUALIFIED-LOGICAL-ADDRESS
    #[test]
    fn binding_is_addressable_on_a_common_module_method() {
        let address =
            QualifiedAddress::parse("main:CommonModule.Main.Method.Handler.Binding").unwrap();
        assert_eq!(
            module_prefix_route(&address).unwrap().role(),
            ModuleRole::Common
        );
    }

    /// Очередь держит один прямой модуль — менеджера. Модуля объекта у неё
    /// нет: её сообщение запись, а не ссылка, и выгрузка 8.5.4 файла объекта
    /// не кладёт. Грамматика адреса линию не знает: существует ли сам объект,
    /// отвечает источник.
    /// rule: INV.SOURCE.PLATFORM-CAPABILITY-EXISTENCE
    #[test]
    fn the_queue_owns_a_manager_module_and_no_object_module() {
        let manager = QualifiedAddress::parse("main:Queue.Проба.Module.Manager").unwrap();
        let route = module_route(&manager).expect("the queue owns a manager module");
        assert_eq!(route.role().as_str(), "Manager");
        assert_eq!(route.owner_kind().as_str(), "Queue");
        assert_eq!(route.kind(), super::ModuleRouteKind::Direct);

        for absent in [
            "main:Queue.Проба.Module.Object",
            "main:Queue.Проба.Module.RecordSet",
            "main:Queue.Проба.Module.ValueManager",
        ] {
            let at = QualifiedAddress::parse(absent).unwrap();
            assert_eq!(module_route(&at), None, "{absent}");
        }
    }

    /// Модули вложенных объектов внешнего источника адресуются на полной
    /// глубине; владелец маршрута — сам вложенный объект.
    #[test]
    fn external_data_source_nested_objects_route_their_declared_modules() {
        use crate::address::NodeKind;
        for (raw, owner, role) in [
            (
                "main:ExternalDataSource.S.Table.T.Module.RecordSet",
                NodeKind::Table,
                ModuleRole::RecordSet,
            ),
            (
                "main:ВнешнийИсточникДанных.S.Куб.C.Module.Manager",
                NodeKind::Cube,
                ModuleRole::Manager,
            ),
            (
                "main:ExternalDataSource.S.Cube.C.DimensionTable.D.Module.Object",
                NodeKind::DimensionTable,
                ModuleRole::Object,
            ),
        ] {
            let route = module_route(&QualifiedAddress::parse(raw).unwrap()).unwrap();
            assert_eq!((route.owner_kind(), route.role()), (owner, role), "{raw}");
        }
        for absent in [
            "main:ExternalDataSource.S.Cube.C.Module.Object",
            "main:ExternalDataSource.S.Function.F.Module.Object",
            "main:ExternalDataSource.S.Cube.C.DimensionTable.D.Module.RecordSet",
            "main:Catalog.S.Table.T.Module.Object",
        ] {
            let at = QualifiedAddress::parse(absent);
            assert!(
                at.map_or(true, |at| module_route(&at).is_none()),
                "{absent}"
            );
        }
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Oracle {
        module_capabilities: Vec<Case>,
    }

    #[derive(Deserialize)]
    struct Case {
        case: String,
        at: String,
        exists: bool,
        role: Option<String>,
    }

    #[test]
    fn settings_storage_routes_only_the_form_role_of_nested_forms() {
        for (raw, expected) in [
            ("main:SettingsStorage.User.Form.Save.Module.Form", true),
            ("main:SettingsStorage.User.Module.Manager", true),
            (
                "main:SettingsStorage.User.Command.Open.Module.Command",
                false,
            ),
            ("main:SettingsStorage.User.Form.Save.Module.Command", false),
            ("main:SettingsStorage.User.Form.Save.Module.Object", false),
            (
                "main:SettingsStorage.User.Template.Layout.Module.Form",
                false,
            ),
        ] {
            let at = QualifiedAddress::parse(raw).unwrap();
            assert_eq!(module_route(&at).is_some(), expected, "{raw}");
        }
    }

    #[test]
    fn structural_module_routes_preserve_the_existing_address_oracle() {
        let oracle: Oracle = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/v013/address-profile-8.3.27.json"
        )))
        .unwrap();
        for case in oracle.module_capabilities {
            let at = QualifiedAddress::parse(&case.at).unwrap();
            let route = module_route(&at);
            assert_eq!(route.is_some(), case.exists, "{}", case.case);
            assert_eq!(
                route.map(|route| route.role().as_str()),
                case.role.as_deref(),
                "{}",
                case.case
            );
        }
    }
}
