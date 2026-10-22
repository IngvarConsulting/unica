use unica_address::address::NodeKind;
use unica_metadata::site::{metadata_collection_matches, MetadataSiteExt};
use unica_metadata::MetaCollection;
use unica_source_contract::context::OperationSite;
use unica_source_contract::route::RouteClass;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationApplicability {
    DcsCalculatedField,
    DcsData,
    DcsDataContainer,
    DcsDataSource,
    DcsField,
    DcsParameter,
    DcsQuery,
    DcsSchema,
    DcsSettings,
    DcsSettingsContainer,
    DcsStructureItem,
    DcsTotal,
}
impl OperationApplicability {
    pub const fn shape(self) -> &'static str {
        match self {
            Self::DcsSchema => "`Kind.Name.Template.T` of a data composition schema",
            Self::DcsDataContainer => "`Kind.Name.Template.T[.DataSet.D]` of a schema",
            Self::DcsData => "`Kind.Name.Template.T.DataSet.D`",
            Self::DcsQuery => "`Kind.Name.Template.T.DataSet.D.Query`",
            Self::DcsSettings => "`Kind.Name.Template.T.Setting.S`",
            Self::DcsParameter => "`Kind.Name.Template.T.Parameter.P`",
            Self::DcsField => "`Kind.Name.Template.T.DataSet.D.Field.F`",
            Self::DcsDataSource => "`Kind.Name.Template.T.DataSource.S`",
            Self::DcsCalculatedField => "`Kind.Name.Template.T.CalculatedField.F`",
            Self::DcsTotal => "`Kind.Name.Template.T.Total.F`",
            Self::DcsSettingsContainer => "`Kind.Name.Template.T.Setting.S[.Item.I…]`",
            Self::DcsStructureItem => "`Kind.Name.Template.T.Setting.S.Item.I[.Item.J…]`",
        }
    }
    pub fn matches(self, site: &OperationSite<'_>) -> bool {
        let segments = site.segments();
        match self {
            Self::DcsSchema => site.owned_template(),
            Self::DcsDataContainer => {
                site.owned_template() || site.dcs_terminal(&[NodeKind::DataSet])
            }
            Self::DcsData => site.dcs_terminal(&[NodeKind::DataSet]),
            Self::DcsQuery => {
                site.route() == RouteClass::Dcs
                    && matches!(segments, [.., data_set, query]
                        if query.kind() == NodeKind::Query
                            && query.name().is_none()
                            && data_set.kind() == NodeKind::DataSet
                            && data_set.name().is_some())
            }
            Self::DcsSettings => site.dcs_terminal(&[NodeKind::Setting]),
            Self::DcsParameter => site.dcs_terminal(&[NodeKind::Parameter]),
            Self::DcsField => site.dcs_terminal(&[NodeKind::Field]),
            Self::DcsDataSource => site.dcs_schema_member(NodeKind::DataSource),
            Self::DcsCalculatedField => site.dcs_schema_member(NodeKind::CalculatedField),
            Self::DcsTotal => site.dcs_schema_member(NodeKind::Total),
            Self::DcsSettingsContainer => site.dcs_structure_depth().is_some(),
            Self::DcsStructureItem => site.dcs_structure_depth().is_some_and(|depth| depth > 0),
        }
    }
}
trait SiteExt {
    fn dcs_schema_member(&self, kind: NodeKind) -> bool;
    fn dcs_structure_depth(&self) -> Option<usize>;
    fn dcs_terminal(&self, kinds: &[NodeKind]) -> bool;
}
impl SiteExt for OperationSite<'_> {
    /// Узел схемы прямо под макетом владельца: `Owner.X.Template.T.<kind>.N`.
    fn dcs_schema_member(&self, kind: NodeKind) -> bool {
        self.route() == RouteClass::Dcs
            && matches!(self.segments(), [owner, template, member]
                if owner.name().is_some()
                    && template.kind() == NodeKind::Template
                    && template.name().is_some()
                    && member.kind() == kind
                    && member.name().is_some()
                    && metadata_collection_matches(owner.kind(), MetaCollection::Templates))
    }

    /// Вариант настроек макета владельца и цепочка именованных элементов его
    /// структуры: число сегментов `Item` после `Setting`, `None` — адрес не
    /// такой формы.
    fn dcs_structure_depth(&self) -> Option<usize> {
        if self.route() != RouteClass::Dcs {
            return None;
        }
        let [owner, template, setting, items @ ..] = self.segments() else {
            return None;
        };
        (owner.name().is_some()
            && template.kind() == NodeKind::Template
            && template.name().is_some()
            && setting.kind() == NodeKind::Setting
            && setting.name().is_some()
            && metadata_collection_matches(owner.kind(), MetaCollection::Templates)
            && items
                .iter()
                .all(|item| item.kind() == NodeKind::Item && item.name().is_some()))
        .then_some(items.len())
    }

    fn dcs_terminal(&self, kinds: &[NodeKind]) -> bool {
        self.route() == RouteClass::Dcs
            && kinds.contains(&self.terminal().kind())
            && self.terminal().name().is_some()
    }
}
