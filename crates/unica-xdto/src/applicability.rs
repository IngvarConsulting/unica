use crate::operations::XdtoSite;
use unica_source_contract::context::OperationSite;
use unica_source_contract::route::RouteClass;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationApplicability {
    XdtoPackage,
    XdtoProperty,
    XdtoPropertyContainer,
    XdtoType,
}
impl OperationApplicability {
    pub const fn shape(self) -> &'static str {
        match self {
            Self::XdtoPackage => "`XDTOPackage.Name`",
            Self::XdtoType => "`XDTOPackage.Name.Type.T`",
            Self::XdtoProperty => "`XDTOPackage.Name.Type.T.Property.P[.Property.Q…]`",
            Self::XdtoPropertyContainer => "`XDTOPackage.Name.Type.T[.Property.P…]`",
        }
    }
    pub fn matches(self, site: &OperationSite<'_>) -> bool {
        match self {
            Self::XdtoPackage => {
                site.route() == RouteClass::Xdto && XdtoSite::XdtoPackage.matches(site.at())
            }
            Self::XdtoType => {
                site.route() == RouteClass::Xdto && XdtoSite::XdtoType.matches(site.at())
            }
            Self::XdtoProperty => {
                site.route() == RouteClass::Xdto && XdtoSite::XdtoProperty.matches(site.at())
            }
            Self::XdtoPropertyContainer => {
                site.route() == RouteClass::Xdto
                    && XdtoSite::XdtoPropertyContainer.matches(site.at())
            }
        }
    }
}
