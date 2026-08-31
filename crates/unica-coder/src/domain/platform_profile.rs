use crate::domain::address::{AddressSegment, NodeKind, QualifiedAddress};

pub(crate) const PLATFORM_PROFILE_8_3_27: &str = "8.3.27";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlatformProfile {
    id: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModuleRole {
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
    const ALL: [Self; 16] = [
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

    pub(crate) const fn as_str(self) -> &'static str {
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

    pub(crate) fn from_v12_terminal(value: &str) -> Option<Self> {
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
pub(crate) enum ModuleSourceLayout {
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
pub(crate) struct ModuleCapability {
    owner_kind: NodeKind,
    role: ModuleRole,
    source_layout: ModuleSourceLayout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModuleAddressCapability {
    at: QualifiedAddress,
    capability: ModuleCapability,
}

impl ModuleAddressCapability {
    pub(crate) const fn at(&self) -> &QualifiedAddress {
        &self.at
    }

    pub(crate) const fn capability(&self) -> ModuleCapability {
        self.capability
    }
}

impl ModuleCapability {
    pub(crate) const fn role(self) -> ModuleRole {
        self.role
    }

    pub(crate) const fn owner_kind(self) -> NodeKind {
        self.owner_kind
    }

    pub(crate) const fn source_layout(self) -> ModuleSourceLayout {
        self.source_layout
    }
}

impl PlatformProfile {
    pub(crate) const fn v8_3_27() -> Self {
        Self {
            id: PLATFORM_PROFILE_8_3_27,
        }
    }

    pub(crate) const fn id(self) -> &'static str {
        self.id
    }

    pub(crate) fn module_capability(self, address: &QualifiedAddress) -> Option<ModuleCapability> {
        module_capability_for_segments(address.segments())
    }

    pub(crate) fn module_prefix_capability(
        self,
        address: &QualifiedAddress,
    ) -> Option<ModuleCapability> {
        let segments = address.segments();
        (1..=segments.len()).rev().find_map(|length| {
            let capability = module_capability_for_segments(&segments[..length])?;
            module_projection_suffix_is_valid(&segments[length..]).then_some(capability)
        })
    }

    /// Enumerates the canonical module nodes owned by one addressable `Module`
    /// branch. This is the sole profile-owned role list used by logical view
    /// and find; projections do not restate module-role matrices.
    pub(crate) fn module_children(self, branch: &QualifiedAddress) -> Vec<ModuleAddressCapability> {
        let segments = branch.segments();
        if !matches!(segments.last(), Some(module) if module.kind() == NodeKind::Module && module.name().is_none())
        {
            return Vec::new();
        }
        ModuleRole::ALL
            .into_iter()
            .filter_map(|role| {
                let at = QualifiedAddress::parse(&format!("{branch}.{}", role.as_str())).ok()?;
                let capability = self.module_capability(&at)?;
                Some(ModuleAddressCapability { at, capability })
            })
            .collect()
    }

    pub(crate) fn supports_owner_kind(self, raw: &str) -> bool {
        NodeKind::parse(raw)
            .ok()
            .is_some_and(|kind| kind.is_metadata_kind())
    }

    pub(crate) fn supports_direct_module_role(
        self,
        owner_kind: NodeKind,
        role: ModuleRole,
    ) -> bool {
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

    pub(crate) fn supports_nested_form_or_command(self, owner_kind: NodeKind) -> bool {
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

fn module_capability_for_segments(segments: &[AddressSegment]) -> Option<ModuleCapability> {
    match segments {
        [owner] if owner.name().is_some() && owner.kind() == NodeKind::CommonModule => {
            Some(capability(
                NodeKind::CommonModule,
                ModuleRole::Common,
                ModuleSourceLayout::Common,
            ))
        }
        [module] if module.kind() == NodeKind::Module => {
            let role = ModuleRole::from_semantic_name(module.name()?)?;
            matches!(
                role,
                ModuleRole::ManagedApplication
                    | ModuleRole::OrdinaryApplication
                    | ModuleRole::Session
                    | ModuleRole::ExternalConnection
            )
            .then(|| capability(NodeKind::Configuration, role, ModuleSourceLayout::Root))
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

fn direct_or_named_module(owner: NodeKind, raw_role: &str) -> Option<ModuleCapability> {
    let role = ModuleRole::from_semantic_name(raw_role)?;
    let profile = PlatformProfile::v8_3_27();
    if profile.supports_direct_module_role(owner, role) {
        return Some(capability(owner, role, ModuleSourceLayout::Direct));
    }
    match (owner, role) {
        (NodeKind::CommonForm, ModuleRole::Form) => {
            Some(capability(owner, role, ModuleSourceLayout::CommonForm))
        }
        (NodeKind::CommonCommand, ModuleRole::Command) => {
            Some(capability(owner, role, ModuleSourceLayout::CommonCommand))
        }
        (NodeKind::HttpService, ModuleRole::HttpService)
        | (NodeKind::WebService, ModuleRole::WebService)
        | (NodeKind::IntegrationService, ModuleRole::IntegrationService) => {
            Some(capability(owner, role, ModuleSourceLayout::Service))
        }
        (NodeKind::Bot, ModuleRole::Bot) => Some(capability(owner, role, ModuleSourceLayout::Bot)),
        (NodeKind::WebSocketClient, ModuleRole::WebSocketClient) => {
            Some(capability(owner, role, ModuleSourceLayout::WebSocketClient))
        }
        _ => None,
    }
}

fn nested_module(owner: NodeKind, child: NodeKind, raw_role: &str) -> Option<ModuleCapability> {
    if !PlatformProfile::v8_3_27().supports_nested_form_or_command(owner) {
        return None;
    }
    match (child, ModuleRole::from_semantic_name(raw_role)?) {
        (NodeKind::Form, ModuleRole::Form) => Some(capability(
            owner,
            ModuleRole::Form,
            ModuleSourceLayout::NestedForm,
        )),
        (NodeKind::Command, ModuleRole::Command) => Some(capability(
            owner,
            ModuleRole::Command,
            ModuleSourceLayout::NestedCommand,
        )),
        _ => None,
    }
}

const fn capability(
    owner_kind: NodeKind,
    role: ModuleRole,
    source_layout: ModuleSourceLayout,
) -> ModuleCapability {
    ModuleCapability {
        owner_kind,
        role,
        source_layout,
    }
}

#[cfg(test)]
mod tests {
    use super::{ModuleRole, PlatformProfile};
    use crate::domain::address::{NodeKind, QualifiedAddress};
    use serde::Deserialize;
    use std::collections::HashSet;

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ProfileFixture {
        profile: String,
        module_capabilities: Vec<ModuleCase>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ModuleCase {
        case: String,
        at: String,
        exists: bool,
        role: Option<String>,
    }

    fn fixture() -> ProfileFixture {
        serde_json::from_str(include_str!(
            "../../../../tests/fixtures/v013/address-profile-8.3.27.json"
        ))
        .expect("the checked platform profile fixture must be valid JSON")
    }

    #[test]
    fn every_approved_module_role_is_a_closed_8_3_27_capability() {
        let fixture = fixture();
        assert_eq!(fixture.profile, "8.3.27");
        let profile = PlatformProfile::v8_3_27();

        for case in fixture.module_capabilities {
            let address = QualifiedAddress::parse(&case.at).unwrap();
            let capability = profile.module_capability(&address);
            assert_eq!(capability.is_some(), case.exists, "{}", case.case);
            if let Some(expected_role) = case.role {
                assert_eq!(
                    capability
                        .expect("expected module capability")
                        .role()
                        .as_str(),
                    expected_role,
                    "{}",
                    case.case
                );
            }
        }
    }

    #[test]
    fn service_bot_websocket_and_absent_grpc_capabilities_are_not_conflated() {
        let profile = PlatformProfile::v8_3_27();
        let expected = [
            (
                "main:HTTPService.API.Module.HTTPService",
                ModuleRole::HttpService,
            ),
            (
                "main:WebService.Обмен.Module.WebService",
                ModuleRole::WebService,
            ),
            (
                "main:IntegrationService.Шина.Module.IntegrationService",
                ModuleRole::IntegrationService,
            ),
            ("main:Bot.Помощник.Module.Bot", ModuleRole::Bot),
            (
                "main:WebSocketClient.Телефония.Module.WebSocketClient",
                ModuleRole::WebSocketClient,
            ),
        ];
        for (raw, role) in expected {
            let address = QualifiedAddress::parse(raw).unwrap();
            assert_eq!(profile.module_capability(&address).unwrap().role(), role);
        }

        assert!(!profile.supports_owner_kind("GRPCService"));
        assert!(QualifiedAddress::parse("main:GRPCService.API.Module.GRPCService").is_err());
    }

    #[test]
    fn external_epf_erf_object_form_and_command_roles_are_capabilities() {
        let profile = PlatformProfile::v8_3_27();
        let expected = [
            (
                "epf:ExternalDataProcessor.Импорт.Module.Object",
                ModuleRole::Object,
            ),
            (
                "epf:ExternalDataProcessor.Импорт.Form.Основная.Module.Form",
                ModuleRole::Form,
            ),
            (
                "epf:ExternalDataProcessor.Импорт.Command.Выполнить.Module.Command",
                ModuleRole::Command,
            ),
            (
                "erf:ExternalReport.Продажи.Module.Object",
                ModuleRole::Object,
            ),
            (
                "erf:ExternalReport.Продажи.Form.Основная.Module.Form",
                ModuleRole::Form,
            ),
            (
                "erf:ExternalReport.Продажи.Command.Сформировать.Module.Command",
                ModuleRole::Command,
            ),
        ];

        for (raw, role) in expected {
            let address = QualifiedAddress::parse(raw).unwrap();
            assert_eq!(
                profile
                    .module_capability(&address)
                    .map(|value| value.role()),
                Some(role),
                "{raw}"
            );
        }
    }

    #[test]
    fn platform_profile_8_3_27_has_closed_module_capabilities() {
        every_approved_module_role_is_a_closed_8_3_27_capability();
        service_bot_websocket_and_absent_grpc_capabilities_are_not_conflated();
    }

    #[test]
    fn every_module_capability_is_owned_by_one_canonical_parent_branch() {
        let profile = PlatformProfile::v8_3_27();
        let mut reached = HashSet::new();
        for case in fixture().module_capabilities {
            let address = QualifiedAddress::parse(&case.at).unwrap();
            if !case.exists {
                continue;
            }
            if matches!(address.segments(), [owner] if owner.kind() == NodeKind::CommonModule) {
                assert!(
                    profile.module_capability(&address).is_some(),
                    "{}",
                    case.case
                );
                assert!(reached.insert(address.to_string()), "{}", case.case);
                continue;
            }
            let module_index = address
                .segments()
                .iter()
                .position(|segment| segment.kind() == NodeKind::Module)
                .expect("approved module address has a Module segment");
            let mut branch = case
                .at
                .split('.')
                .take(module_index * 2 + 1)
                .collect::<Vec<_>>()
                .join(".");
            if module_index == 0 {
                branch = format!("{}:Module", address.source_set());
            }
            let branch = QualifiedAddress::parse(&branch).unwrap();
            let children = profile.module_children(&branch);
            assert_eq!(
                children
                    .iter()
                    .filter(|child| child.at() == &address)
                    .count(),
                1,
                "{} must be reachable once from {branch}: {children:?}",
                case.case,
            );
            assert!(reached.insert(address.to_string()), "{}", case.case);
        }
        assert_eq!(reached.len(), 25);
    }
}
