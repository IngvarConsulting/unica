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
        module_projection_suffix_is_valid(&segments[length..]).then_some(route)
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
        ),
        ModuleRole::RecordSet => matches!(
            owner_kind,
            NodeKind::InformationRegister
                | NodeKind::AccumulationRegister
                | NodeKind::AccountingRegister
                | NodeKind::CalculationRegister
        ),
        ModuleRole::ValueManager => owner_kind == NodeKind::Constant,
        _ => false,
    }
}

pub fn has_nested_module_route(owner_kind: NodeKind) -> bool {
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
fn module_projection_suffix_is_valid(suffix: &[AddressSegment]) -> bool {
    match suffix {
        [] => true,
        [branch]
            if matches!(branch.kind(), NodeKind::Compilation | NodeKind::Body)
                && branch.name().is_none() =>
        {
            true
        }
        [branch]
            if matches!(
                branch.kind(),
                NodeKind::Method | NodeKind::Interface | NodeKind::Event
            ) =>
        {
            true
        }
        [method, projection]
            if method.kind() == NodeKind::Method
                && method.name().is_some()
                && matches!(projection.kind(), NodeKind::Compilation | NodeKind::Body)
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

fn module_route_for_segments(segments: &[AddressSegment]) -> Option<ModuleRoute> {
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

fn nested_module(owner: NodeKind, child: NodeKind, raw_role: &str) -> Option<ModuleRoute> {
    if !has_nested_module_route(owner) {
        return None;
    }
    match (child, ModuleRole::from_semantic_name(raw_role)?) {
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
    use super::module_route;
    use crate::address::QualifiedAddress;
    use serde::Deserialize;

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
