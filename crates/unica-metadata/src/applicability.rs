use crate::descriptor_operations::PropertiesSite;
use crate::site::{metadata_collection_matches, MetadataSiteExt};
use crate::MetaCollection;
use unica_address::address::NodeKind;
use unica_source_contract::context::OperationSite;
use unica_source_contract::route::RouteClass;
use unica_source_contract::SourceSetKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationApplicability {
    ExchangePlan,
    Configuration,
    ConfigurationRoot,
    Event,
    ExtensionRoot,
    Interface,
    InterfaceGroup,
    Metadata,
    MetadataOrCommonTemplate,
    MetadataAttribute,
    MetadataAttributeContainer,
    /// An attribute of a catalog tabular section in a configuration source.
    CatalogTabularAttribute,
    MetadataColumn,
    MetadataColumns,
    MetadataCommand,
    MetadataCommands,
    MetadataDimension,
    MetadataDimensions,
    MetadataEnumValue,
    MetadataEnumValues,
    MetadataOrRoleOrSubsystem,
    MetadataPredefinedItem,
    MetadataPredefinedItems,
    MetadataResource,
    MetadataResources,
    MetadataTabularSection,
    MetadataTabularSections,
    MetadataTemplate,
    MetadataTemplates,
    NestedSubsystem,
    Subsystem,
    SupportRule,
}
impl OperationApplicability {
    pub const fn shape(self) -> &'static str {
        match self {
            Self::ExchangePlan => "`ExchangePlan.Name`",
            Self::Configuration => "`Configuration`",
            Self::ConfigurationRoot => "`Configuration` of a configuration, not of an extension",
            Self::ExtensionRoot => "`Configuration` of an extension",
            Self::Metadata => "`Kind.Name`",
            Self::MetadataOrCommonTemplate => "`Kind.Name` or `CommonTemplate.Name`",
            Self::SupportRule => "`Kind.Name` or `Kind.Name.Form.F`",
            Self::MetadataOrRoleOrSubsystem => {
                "`Configuration`, `Kind.Name`, `Role.Name`, `Subsystem.Name` or `Catalog.Name.StandardAttribute.Owner`"
            }
            Self::MetadataAttributeContainer => {
                "`Kind.Name` of a kind with attributes or `Kind.Name.TabularSection.T`"
            }
            Self::MetadataTabularSections => "`Kind.Name` of a kind with tabular sections",
            Self::MetadataDimensions => "`Kind.Name` of a register with dimensions",
            Self::MetadataResources => "`Kind.Name` of a register with resources",
            Self::MetadataEnumValues => "`Enum.Name`",
            Self::MetadataColumns => "`DocumentJournal.Name`",
            Self::MetadataTemplates => "`Kind.Name` of a kind with templates",
            Self::MetadataCommands => "`Kind.Name` of a kind with commands",
            Self::MetadataPredefinedItems => "`Kind.Name` of a kind with predefined items",
            Self::MetadataAttribute => "`Kind.Name[.TabularSection.T].Attribute.A`",
            Self::CatalogTabularAttribute => {
                "`Catalog.Name.TabularSection.T.Attribute.A` of a configuration"
            }
            Self::MetadataTabularSection => "`Kind.Name.TabularSection.T`",
            Self::MetadataDimension => "`Kind.Name.Dimension.D` of a register",
            Self::MetadataResource => "`Kind.Name.Resource.R` of a register",
            Self::MetadataEnumValue => "`Enum.Name.EnumValue.V`",
            Self::MetadataColumn => "`DocumentJournal.Name.Column.C`",
            Self::MetadataCommand => "`Kind.Name.Command.C`",
            Self::MetadataTemplate => "`Kind.Name.Template.T`",
            Self::MetadataPredefinedItem => "`Kind.Name.PredefinedItem.P`",
            Self::Subsystem => "`Subsystem.Name[.Subsystem.Child]`",
            Self::NestedSubsystem => "`Subsystem.Parent.Subsystem.Child`",
            Self::Interface => "`Subsystem.Name[.Subsystem.Child].Interface`",
            Self::InterfaceGroup => "`Subsystem.Name[.Subsystem.Child].Interface.Group.G`",
            Self::Event => "an `Event` of a module or a form",
        }
    }
    pub fn matches(self, site: &OperationSite<'_>) -> bool {
        let segments = site.segments();
        let terminal = site.terminal();
        match self {
            Self::ExchangePlan => {
                matches!(segments, [only] if only.kind() == NodeKind::ExchangePlan && only.name().is_some())
            }
            Self::Configuration => site.route() == RouteClass::Configuration,
            Self::ConfigurationRoot => {
                site.route() == RouteClass::Configuration
                    && site.source().kind() == SourceSetKind::Configuration
            }
            Self::ExtensionRoot => {
                site.route() == RouteClass::Configuration
                    && site.source().kind() == SourceSetKind::Extension
            }
            Self::Metadata => site.metadata_object().is_some(),
            // A common template is a top-level object the configurator
            // removes like any other, though it has no typed metadata kind.
            Self::MetadataOrCommonTemplate => {
                site.metadata_object().is_some()
                    || matches!(segments, [only] if only.kind() == NodeKind::CommonTemplate && only.name().is_some())
            }
            Self::MetadataOrRoleOrSubsystem => PropertiesSite::from_site(site).is_some(),
            Self::MetadataAttributeContainer => {
                site.metadata_owner_with(MetaCollection::Attributes)
                    || site
                        .metadata_member(NodeKind::TabularSection, MetaCollection::TabularSections)
            }
            Self::MetadataTabularSections => {
                site.metadata_owner_with(MetaCollection::TabularSections)
            }
            Self::MetadataDimensions => site.metadata_owner_with(MetaCollection::Dimensions),
            Self::MetadataResources => site.metadata_owner_with(MetaCollection::Resources),
            Self::MetadataEnumValues => site.metadata_owner_with(MetaCollection::EnumValues),
            Self::MetadataColumns => site.metadata_owner_with(MetaCollection::Columns),
            Self::MetadataTemplates => site.metadata_owner_with(MetaCollection::Templates),
            Self::MetadataCommands => site.metadata_owner_with(MetaCollection::Commands),
            Self::MetadataPredefinedItems => {
                site.metadata_owner_with(MetaCollection::PredefinedItems)
            }
            Self::MetadataAttribute => {
                site.route() == RouteClass::Metadata
                    && terminal.kind() == NodeKind::Attribute
                    && terminal.name().is_some()
                    && match segments {
                        [owner, _] => {
                            owner.name().is_some()
                                && metadata_collection_matches(
                                    owner.kind(),
                                    MetaCollection::Attributes,
                                )
                        }
                        [owner, section, _] => {
                            owner.name().is_some()
                                && section.kind() == NodeKind::TabularSection
                                && section.name().is_some()
                                && metadata_collection_matches(
                                    owner.kind(),
                                    MetaCollection::TabularSections,
                                )
                        }
                        _ => false,
                    }
            }
            Self::CatalogTabularAttribute => {
                site.route() == RouteClass::Metadata
                    && site.source().kind() == SourceSetKind::Configuration
                    && matches!(segments, [owner, section, attribute]
                        if owner.kind() == NodeKind::Catalog
                            && owner.name().is_some()
                            && section.kind() == NodeKind::TabularSection
                            && section.name().is_some()
                            && attribute.kind() == NodeKind::Attribute
                            && attribute.name().is_some())
            }
            Self::MetadataTabularSection => {
                site.metadata_member(NodeKind::TabularSection, MetaCollection::TabularSections)
            }
            Self::MetadataDimension => {
                site.metadata_member(NodeKind::Dimension, MetaCollection::Dimensions)
            }
            Self::MetadataResource => {
                site.metadata_member(NodeKind::Resource, MetaCollection::Resources)
            }
            Self::MetadataEnumValue => {
                site.metadata_member(NodeKind::EnumValue, MetaCollection::EnumValues)
            }
            Self::MetadataColumn => site.metadata_member(NodeKind::Column, MetaCollection::Columns),
            Self::MetadataCommand => {
                site.metadata_member(NodeKind::Command, MetaCollection::Commands)
            }
            Self::MetadataTemplate => site.owned_template(),
            Self::MetadataPredefinedItem => {
                site.metadata_member(NodeKind::PredefinedItem, MetaCollection::PredefinedItems)
            }
            Self::Subsystem => {
                site.route() == RouteClass::Subsystem
                    && segments.iter().all(|segment| {
                        segment.kind() == NodeKind::Subsystem && segment.name().is_some()
                    })
            }
            Self::NestedSubsystem => Self::Subsystem.matches(site) && site.segments().len() > 1,
            Self::Interface => {
                site.route() == RouteClass::Interface
                    && terminal.kind() == NodeKind::Interface
                    && segments[..segments.len() - 1].iter().all(|segment| {
                        segment.kind() == NodeKind::Subsystem && segment.name().is_some()
                    })
            }
            Self::InterfaceGroup => {
                site.route() == RouteClass::Interface
                    && matches!(segments, [owners @ .., interface, group]
                    if interface.kind() == NodeKind::Interface
                        && group.kind() == NodeKind::Group
                        && group.name().is_some()
                        && !owners.is_empty()
                        && owners.iter().all(|segment| {
                            segment.kind() == NodeKind::Subsystem && segment.name().is_some()
                        }))
            }
            Self::SupportRule => match segments {
                [owner] | [owner, _] => {
                    owner.name().is_some()
                        && owner.kind().is_metadata_kind()
                        && !matches!(
                            owner.kind(),
                            NodeKind::Form | NodeKind::Template | NodeKind::Command
                        )
                        && (segments.len() == 1
                            || (terminal.kind() == NodeKind::Form && terminal.name().is_some()))
                }
                _ => false,
            },
            Self::Event => {
                terminal.kind() == NodeKind::Event
                    && terminal.name().is_some()
                    && match site.route() {
                        RouteClass::Form => true,
                        // Событие объекта пишется в модуль платформы, которого
                        // у расширения нет.
                        RouteClass::Module | RouteClass::Metadata => {
                            site.source().kind() == SourceSetKind::Configuration
                        }
                        _ => false,
                    }
            }
        }
    }
}
