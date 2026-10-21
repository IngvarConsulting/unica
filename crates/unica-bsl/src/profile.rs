use unica_address::address::{NodeKind, QualifiedAddress};
use unica_address::module_routes::{
    has_direct_module_route, has_nested_module_route, module_prefix_route, module_route,
    ModuleRole, ModuleRoute, ModuleRouteKind,
};

pub const PLATFORM_PROFILE_8_3_27: &str = "8.3.27";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformProfile {
    id: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModuleCapability {
    route: ModuleRoute,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleAddressCapability {
    at: QualifiedAddress,
    capability: ModuleCapability,
}

impl ModuleAddressCapability {
    pub const fn at(&self) -> &QualifiedAddress {
        &self.at
    }

    pub const fn capability(&self) -> ModuleCapability {
        self.capability
    }
}

impl ModuleCapability {
    pub const fn role(self) -> ModuleRole {
        self.route.role()
    }

    pub const fn owner_kind(self) -> NodeKind {
        self.route.owner_kind()
    }

    pub const fn route_kind(self) -> ModuleRouteKind {
        self.route.kind()
    }
}

impl PlatformProfile {
    pub const fn v8_3_27() -> Self {
        Self {
            id: PLATFORM_PROFILE_8_3_27,
        }
    }

    pub const fn id(self) -> &'static str {
        self.id
    }

    pub fn module_capability(self, address: &QualifiedAddress) -> Option<ModuleCapability> {
        module_route(address).map(|route| ModuleCapability { route })
    }

    pub fn module_prefix_capability(self, address: &QualifiedAddress) -> Option<ModuleCapability> {
        module_prefix_route(address).map(|route| ModuleCapability { route })
    }

    /// Enumerates the canonical module nodes owned by one addressable `Module`
    /// branch. This is the sole profile-owned role list used by logical view
    /// and find; projections do not restate module-role matrices.
    pub fn module_children(self, branch: &QualifiedAddress) -> Vec<ModuleAddressCapability> {
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

    pub fn supports_owner_kind(self, raw: &str) -> bool {
        NodeKind::parse(raw)
            .ok()
            .is_some_and(|kind| kind.is_metadata_kind())
    }

    pub fn supports_direct_module_role(self, owner_kind: NodeKind, role: ModuleRole) -> bool {
        has_direct_module_route(owner_kind, role)
    }

    pub fn supports_nested_form_or_command(self, owner_kind: NodeKind) -> bool {
        has_nested_module_route(owner_kind)
    }
}

#[cfg(test)]
mod tests {
    use super::{ModuleRole, PlatformProfile};
    use serde::Deserialize;
    use std::collections::HashSet;
    use unica_address::address::{NodeKind, QualifiedAddress};

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
            "../../../tests/fixtures/v013/address-profile-8.3.27.json"
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
