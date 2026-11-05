use super::{MetaObservedPropertyValue, MetadataKind};
use serde::{Deserialize, Serialize};
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_source_contract::SourceSetKind;

/// A read capability, independent of the metadata mutation grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetaMemberReadProfile {
    CatalogTabularSection220,
    ExchangePlanFilling220,
    ChartOfAccountsFilling220,
    ChartOfCharacteristicTypesFilling220,
    ChartOfCalculationTypesFilling220,
    BusinessProcessFilling220,
    TaskFilling220,
    ReportFilling220,
    DataProcessorFilling220,
    DocumentCommand220,
    InformationRegisterDimension220,
    AccumulationRegisterDimension220,
    DocumentJournalColumn220,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaMemberReadContext {
    TabularSection,
    TabularAttribute,
    HeaderAttribute,
    InformationResource,
    DocumentCommand,
    InformationRegisterDimension,
    AccumulationRegisterDimension,
    DocumentJournalColumn,
}

#[derive(Debug, Clone, Copy)]
pub enum MetaMemberValueKind {
    UnsignedInteger,
    Boolean,
    CommandGroup,
    Text(&'static [&'static str]),
    LocalizedString,
}

#[derive(Debug, Clone, Copy)]
pub struct MetaMemberPropertySpec {
    pub name: &'static str,
    pub public_name: &'static str,
    context: MetaMemberReadContext,
    kind: MetaMemberValueKind,
}

const CATALOG_MEMBER_PROPERTIES: &[MetaMemberPropertySpec] = &[
    MetaMemberPropertySpec {
        name: "LineNumberLength",
        public_name: "LineNumberLength",
        context: MetaMemberReadContext::TabularSection,
        kind: MetaMemberValueKind::UnsignedInteger,
    },
    MetaMemberPropertySpec {
        name: "Use",
        public_name: "Use",
        context: MetaMemberReadContext::TabularSection,
        kind: MetaMemberValueKind::Text(&["ForItem", "ForFolder", "ForFolderAndItem"]),
    },
    MetaMemberPropertySpec {
        name: "Indexing",
        public_name: "Indexing",
        context: MetaMemberReadContext::TabularAttribute,
        kind: MetaMemberValueKind::Text(&["DontIndex", "Index", "IndexWithAdditionalOrder"]),
    },
    MetaMemberPropertySpec {
        name: "FullTextSearch",
        public_name: "FullTextSearch",
        context: MetaMemberReadContext::TabularAttribute,
        kind: MetaMemberValueKind::Text(&["Use", "DontUse"]),
    },
    MetaMemberPropertySpec {
        name: "DataHistory",
        public_name: "DataHistory",
        context: MetaMemberReadContext::TabularAttribute,
        kind: MetaMemberValueKind::Text(&["Use", "DontUse"]),
    },
    MetaMemberPropertySpec {
        name: "ToolTip",
        public_name: "ToolTip",
        context: MetaMemberReadContext::TabularAttribute,
        kind: MetaMemberValueKind::LocalizedString,
    },
];

const DOCUMENT_COMMAND_PROPERTIES: &[MetaMemberPropertySpec] = &[
    MetaMemberPropertySpec {
        name: "Group",
        public_name: "Group",
        context: MetaMemberReadContext::DocumentCommand,
        kind: MetaMemberValueKind::CommandGroup,
    },
    MetaMemberPropertySpec {
        name: "ParameterUseMode",
        public_name: "ParameterUseMode",
        context: MetaMemberReadContext::DocumentCommand,
        kind: MetaMemberValueKind::Text(&["Single", "Multiple"]),
    },
    MetaMemberPropertySpec {
        name: "ModifiesData",
        public_name: "ModifiesData",
        context: MetaMemberReadContext::DocumentCommand,
        kind: MetaMemberValueKind::Boolean,
    },
];

const INFORMATION_DIMENSION_PROPERTIES: &[MetaMemberPropertySpec] = &[
    MetaMemberPropertySpec {
        name: "Master",
        public_name: "master",
        context: MetaMemberReadContext::InformationRegisterDimension,
        kind: MetaMemberValueKind::Boolean,
    },
    MetaMemberPropertySpec {
        name: "MainFilter",
        public_name: "mainFilter",
        context: MetaMemberReadContext::InformationRegisterDimension,
        kind: MetaMemberValueKind::Boolean,
    },
    MetaMemberPropertySpec {
        name: "DenyIncompleteValues",
        public_name: "denyIncompleteValues",
        context: MetaMemberReadContext::InformationRegisterDimension,
        kind: MetaMemberValueKind::Boolean,
    },
    MetaMemberPropertySpec {
        name: "TypeReductionMode",
        public_name: "TypeReductionMode",
        context: MetaMemberReadContext::InformationRegisterDimension,
        kind: MetaMemberValueKind::Text(&["TransformValues", "Deny", "DeleteData"]),
    },
];

const ACCUMULATION_DIMENSION_PROPERTIES: &[MetaMemberPropertySpec] = &[MetaMemberPropertySpec {
    name: "UseInTotals",
    public_name: "UseInTotals",
    context: MetaMemberReadContext::AccumulationRegisterDimension,
    kind: MetaMemberValueKind::Boolean,
}];

const JOURNAL_COLUMN_PROPERTIES: &[MetaMemberPropertySpec] = &[MetaMemberPropertySpec {
    name: "Indexing",
    public_name: "Indexing",
    context: MetaMemberReadContext::DocumentJournalColumn,
    kind: MetaMemberValueKind::Text(&["DontIndex", "Index", "IndexWithAdditionalOrder"]),
}];

const FILLING_PROPERTIES: &[MetaMemberPropertySpec] = &[
    MetaMemberPropertySpec {
        name: "FillFromFillingValue",
        public_name: "FillFromFillingValue",
        context: MetaMemberReadContext::HeaderAttribute,
        kind: MetaMemberValueKind::Boolean,
    },
    MetaMemberPropertySpec {
        name: "FillFromFillingValue",
        public_name: "FillFromFillingValue",
        context: MetaMemberReadContext::TabularAttribute,
        kind: MetaMemberValueKind::Boolean,
    },
    MetaMemberPropertySpec {
        name: "FillFromFillingValue",
        public_name: "FillFromFillingValue",
        context: MetaMemberReadContext::InformationRegisterDimension,
        kind: MetaMemberValueKind::Boolean,
    },
    MetaMemberPropertySpec {
        name: "FillFromFillingValue",
        public_name: "FillFromFillingValue",
        context: MetaMemberReadContext::InformationResource,
        kind: MetaMemberValueKind::Boolean,
    },
];

impl MetaMemberReadProfile {
    pub fn for_source(
        source_kind: SourceSetKind,
        owner: MetadataKind,
        format: Option<&str>,
    ) -> Option<Self> {
        if source_kind != SourceSetKind::Configuration || format != Some("2.20") {
            return None;
        }
        match owner {
            MetadataKind::Catalog => Some(Self::CatalogTabularSection220),
            MetadataKind::Document => Some(Self::DocumentCommand220),
            MetadataKind::InformationRegister => Some(Self::InformationRegisterDimension220),
            MetadataKind::AccumulationRegister => Some(Self::AccumulationRegisterDimension220),
            MetadataKind::DocumentJournal => Some(Self::DocumentJournalColumn220),
            MetadataKind::ExchangePlan => Some(Self::ExchangePlanFilling220),
            MetadataKind::ChartOfAccounts => Some(Self::ChartOfAccountsFilling220),
            MetadataKind::ChartOfCharacteristicTypes => {
                Some(Self::ChartOfCharacteristicTypesFilling220)
            }
            MetadataKind::ChartOfCalculationTypes => Some(Self::ChartOfCalculationTypesFilling220),
            MetadataKind::BusinessProcess => Some(Self::BusinessProcessFilling220),
            MetadataKind::Task => Some(Self::TaskFilling220),
            MetadataKind::Report => Some(Self::ReportFilling220),
            MetadataKind::DataProcessor => Some(Self::DataProcessorFilling220),
            _ => None,
        }
    }

    pub fn owner_kind(self) -> MetadataKind {
        match self {
            Self::CatalogTabularSection220 => MetadataKind::Catalog,
            Self::ExchangePlanFilling220 => MetadataKind::ExchangePlan,
            Self::ChartOfAccountsFilling220 => MetadataKind::ChartOfAccounts,
            Self::ChartOfCharacteristicTypesFilling220 => MetadataKind::ChartOfCharacteristicTypes,
            Self::ChartOfCalculationTypesFilling220 => MetadataKind::ChartOfCalculationTypes,
            Self::BusinessProcessFilling220 => MetadataKind::BusinessProcess,
            Self::TaskFilling220 => MetadataKind::Task,
            Self::ReportFilling220 => MetadataKind::Report,
            Self::DataProcessorFilling220 => MetadataKind::DataProcessor,
            Self::DocumentCommand220 => MetadataKind::Document,
            Self::InformationRegisterDimension220 => MetadataKind::InformationRegister,
            Self::AccumulationRegisterDimension220 => MetadataKind::AccumulationRegister,
            Self::DocumentJournalColumn220 => MetadataKind::DocumentJournal,
        }
    }

    pub fn properties(
        self,
        context: MetaMemberReadContext,
    ) -> impl Iterator<Item = &'static MetaMemberPropertySpec> {
        let owner = self.owner_kind();
        let primary = match owner {
            MetadataKind::Catalog => CATALOG_MEMBER_PROPERTIES,
            MetadataKind::Document => DOCUMENT_COMMAND_PROPERTIES,
            MetadataKind::InformationRegister => INFORMATION_DIMENSION_PROPERTIES,
            MetadataKind::AccumulationRegister => ACCUMULATION_DIMENSION_PROPERTIES,
            MetadataKind::DocumentJournal => JOURNAL_COLUMN_PROPERTIES,
            _ => &[],
        };
        let collection = match context {
            MetaMemberReadContext::HeaderAttribute => {
                Some((crate::MetaCollection::Attributes, false))
            }
            MetaMemberReadContext::TabularAttribute => {
                Some((crate::MetaCollection::Attributes, true))
            }
            MetaMemberReadContext::InformationRegisterDimension => {
                Some((crate::MetaCollection::Dimensions, false))
            }
            MetaMemberReadContext::InformationResource => {
                Some((crate::MetaCollection::Resources, false))
            }
            _ => None,
        };
        let filling = collection.is_some_and(|(collection, section)| {
            crate::metadata_filling_data_is_allowed(owner, collection, section)
        });
        primary
            .iter()
            .chain(FILLING_PROPERTIES.iter().filter(move |_| filling))
            .filter(move |property| property.context == context)
    }

    pub fn context_for_field(self, field: &str) -> Option<MetaMemberReadContext> {
        let direct = [
            (
                "collections.attributes[",
                MetaMemberReadContext::HeaderAttribute,
            ),
            (
                "collections.commands[",
                MetaMemberReadContext::DocumentCommand,
            ),
            (
                "collections.resources[",
                MetaMemberReadContext::InformationResource,
            ),
            (
                "collections.columns[",
                MetaMemberReadContext::DocumentJournalColumn,
            ),
        ];
        for (prefix, context) in direct {
            if indexed_field_suffix(field, prefix).is_some_and(str::is_empty) {
                return self.properties(context).next().map(|_| context);
            }
        }
        if indexed_field_suffix(field, "collections.dimensions[").is_some_and(str::is_empty) {
            let context = match self.owner_kind() {
                MetadataKind::InformationRegister => {
                    MetaMemberReadContext::InformationRegisterDimension
                }
                MetadataKind::AccumulationRegister => {
                    MetaMemberReadContext::AccumulationRegisterDimension
                }
                _ => return None,
            };
            return self.properties(context).next().map(|_| context);
        }
        let rest = indexed_field_suffix(field, "collections.tabularSections[")?;
        let context = if rest.is_empty() {
            MetaMemberReadContext::TabularSection
        } else if indexed_field_suffix(rest, ".attributes[").is_some_and(str::is_empty) {
            MetaMemberReadContext::TabularAttribute
        } else {
            return None;
        };
        self.properties(context).next().map(|_| context)
    }

    pub fn context_for_address(
        self,
        address: &QualifiedAddress,
        kind: NodeKind,
    ) -> Option<MetaMemberReadContext> {
        let owner = address.segments().first()?;
        if owner.kind().as_str() != self.owner_kind().as_str() || owner.name().is_none() {
            return None;
        }
        let context = match address.segments() {
            [_, member] if member.name().is_some() && member.kind() == kind => match kind {
                NodeKind::Attribute => MetaMemberReadContext::HeaderAttribute,
                NodeKind::Resource => MetaMemberReadContext::InformationResource,
                NodeKind::Command => MetaMemberReadContext::DocumentCommand,
                NodeKind::Column => MetaMemberReadContext::DocumentJournalColumn,
                NodeKind::Dimension => match self.owner_kind() {
                    MetadataKind::InformationRegister => {
                        MetaMemberReadContext::InformationRegisterDimension
                    }
                    MetadataKind::AccumulationRegister => {
                        MetaMemberReadContext::AccumulationRegisterDimension
                    }
                    _ => return None,
                },
                NodeKind::TabularSection => MetaMemberReadContext::TabularSection,
                _ => return None,
            },
            [_, section, member]
                if section.kind() == NodeKind::TabularSection
                    && section.name().is_some()
                    && member.kind() == NodeKind::Attribute
                    && member.name().is_some()
                    && kind == NodeKind::Attribute =>
            {
                MetaMemberReadContext::TabularAttribute
            }
            _ => return None,
        };
        self.properties(context).next().map(|_| context)
    }

    pub fn owns_diagnostic(self, field: &str) -> bool {
        let Some((member, name)) = field.rsplit_once(".properties.") else {
            return false;
        };
        self.context_for_field(member)
            .is_some_and(|context| self.properties(context).any(|spec| spec.name == name))
    }
}

fn indexed_field_suffix<'a>(field: &'a str, prefix: &str) -> Option<&'a str> {
    let (index, rest) = field.strip_prefix(prefix)?.split_once(']')?;
    (!index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit())).then_some(rest)
}

impl MetaMemberReadContext {
    pub fn xml_tag(self) -> &'static str {
        match self {
            Self::TabularSection => "TabularSection",
            Self::TabularAttribute => "Attribute",
            Self::HeaderAttribute => "Attribute",
            Self::InformationResource => "Resource",
            Self::DocumentCommand => "Command",
            Self::InformationRegisterDimension | Self::AccumulationRegisterDimension => "Dimension",
            Self::DocumentJournalColumn => "Column",
        }
    }
}

impl MetaMemberPropertySpec {
    pub fn value_kind(self) -> MetaMemberValueKind {
        self.kind
    }

    pub fn parse_text(self, value: &str) -> Option<MetaObservedPropertyValue> {
        match self.kind {
            MetaMemberValueKind::Boolean => match value {
                "true" => Some(MetaObservedPropertyValue::Boolean { value: true }),
                "false" => Some(MetaObservedPropertyValue::Boolean { value: false }),
                _ => None,
            },
            MetaMemberValueKind::CommandGroup => {
                command_group(value).then(|| MetaObservedPropertyValue::Text {
                    value: value.into(),
                })
            }
            MetaMemberValueKind::UnsignedInteger => value
                .parse::<u32>()
                .ok()
                .map(|value| MetaObservedPropertyValue::UnsignedInteger { value }),
            MetaMemberValueKind::Text(values) => {
                values
                    .contains(&value)
                    .then(|| MetaObservedPropertyValue::Text {
                        value: value.into(),
                    })
            }
            MetaMemberValueKind::LocalizedString => None,
        }
    }

    pub fn accepts(self, value: &MetaObservedPropertyValue) -> bool {
        match (self.kind, value) {
            (MetaMemberValueKind::Boolean, MetaObservedPropertyValue::Boolean { .. }) => true,
            (MetaMemberValueKind::CommandGroup, MetaObservedPropertyValue::Text { value }) => {
                command_group(value)
            }
            (
                MetaMemberValueKind::UnsignedInteger,
                MetaObservedPropertyValue::UnsignedInteger { .. },
            ) => true,
            (MetaMemberValueKind::Text(values), MetaObservedPropertyValue::Text { value }) => {
                values.contains(&value.as_str())
            }
            (
                MetaMemberValueKind::LocalizedString,
                MetaObservedPropertyValue::LocalizedString { values },
            ) => {
                let mut languages = std::collections::HashSet::new();
                values.iter().all(|value| languages.insert(&value.language))
            }
            _ => false,
        }
    }
}

// The eleven StandardCommandsGroup values of the platform (since 8.2) and
// named CommandGroup references. This read vocabulary does not depend on
// mutation contracts.
fn command_group(value: &str) -> bool {
    if [
        "NavigationPanelImportant",
        "NavigationPanelOrdinary",
        "NavigationPanelSeeAlso",
        "FormNavigationPanelGoTo",
        "FormNavigationPanelImportant",
        "FormNavigationPanelSeeAlso",
        "FormCommandBarImportant",
        "FormCommandBarCreateBasedOn",
        "ActionsPanelCreate",
        "ActionsPanelReports",
        "ActionsPanelTools",
    ]
    .contains(&value)
    {
        return true;
    }
    let Ok(address) = unica_address::source_target::MetadataAddress::parse(
        unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        value,
    ) else {
        return false;
    };
    let parts = address.segments().collect::<Vec<_>>();
    matches!(parts.as_slice(), ["CommandGroup", _]) && address.as_str() == value
}

#[cfg(test)]
mod tests;
