use unica_address::address::NodeKind;
use unica_source_contract::context::OperationSite;
use unica_source_contract::route::RouteClass;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationApplicability {
    Configuration,
    Role,
    RoleRight,
}
impl OperationApplicability {
    pub const fn shape(self) -> &'static str {
        match self {
            Self::Configuration => "`Configuration`",
            Self::Role => "`Role.Name`",
            Self::RoleRight => "`Role.Name.Right.<Kind>_<Name>`",
        }
    }
    pub fn matches(self, site: &OperationSite<'_>) -> bool {
        let segments = site.segments();
        match self {
            Self::Configuration => site.route() == RouteClass::Configuration,
            Self::Role => {
                site.route() == RouteClass::Role
                    && matches!(segments, [role] if role.kind() == NodeKind::Role && role.name().is_some())
            }
            Self::RoleRight => {
                site.route() == RouteClass::Role
                    && matches!(segments, [role, right]
                        if role.kind() == NodeKind::Role
                            && role.name().is_some()
                            && right.kind() == NodeKind::Right
                            && right.name().is_some())
            }
        }
    }
}
