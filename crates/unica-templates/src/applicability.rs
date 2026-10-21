use unica_address::address::NodeKind;
use unica_metadata::site::{metadata_collection_matches, MetadataSiteExt};
use unica_metadata::MetaCollection;
use unica_source_contract::context::OperationSite;
use unica_source_contract::route::RouteClass;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationApplicability {
    Mxl,
    MxlArea,
}
impl OperationApplicability {
    pub const fn shape(self) -> &'static str {
        match self {
            Self::Mxl => "`Kind.Name.Template.T` of a spreadsheet",
            Self::MxlArea => "`Kind.Name.Template.T.Area.A` of a spreadsheet",
        }
    }
    pub fn matches(self, site: &OperationSite<'_>) -> bool {
        let segments = site.segments();
        match self {
            Self::Mxl => site.owned_template(),
            Self::MxlArea => {
                site.route() == RouteClass::Mxl
                    && matches!(segments, [owner, template, area]
                        if owner.name().is_some()
                            && template.kind() == NodeKind::Template
                            && template.name().is_some()
                            && area.kind() == NodeKind::Area
                            && area.name().is_some()
                            && metadata_collection_matches(owner.kind(), MetaCollection::Templates))
            }
        }
    }
}
