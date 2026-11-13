use super::{MetaObservedPropertyValue, MetadataKind};
use serde::{Deserialize, Serialize};
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_source_contract::SourceSetKind;

/// A read capability, independent of the metadata mutation grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetaMemberReadProfile {
    CatalogTabularSection220,
    ChartTabularSection,
    DocumentCommand220,
    InformationRegisterDimension220,
    AccumulationRegisterDimension220,
    DocumentJournalColumn220,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaMemberReadContext {
    TabularSection,
    TabularAttribute,
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

const TABULAR_SECTION_USE: MetaMemberPropertySpec = MetaMemberPropertySpec {
    public_name: "Use",
    name: "Use",
    context: MetaMemberReadContext::TabularSection,
    kind: MetaMemberValueKind::Text(crate::TABULAR_SECTION_USES),
};

const CATALOG_MEMBER_PROPERTIES: &[MetaMemberPropertySpec] = &[
    MetaMemberPropertySpec {
        name: "LineNumberLength",
        public_name: "LineNumberLength",
        context: MetaMemberReadContext::TabularSection,
        kind: MetaMemberValueKind::UnsignedInteger,
    },
    TABULAR_SECTION_USE,
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

impl MetaMemberReadProfile {
    /// The member profile of an `owner` read from a source set of
    /// `source_kind`; `writable_format` is whether the codec found the
    /// descriptor in the export format it writes (its dialect decides).
    pub fn for_source(
        source_kind: SourceSetKind,
        owner: MetadataKind,
        writable_format: bool,
    ) -> Option<Self> {
        if source_kind != SourceSetKind::Configuration || !writable_format {
            return None;
        }
        match owner {
            MetadataKind::Catalog => Some(Self::CatalogTabularSection220),
            MetadataKind::Document => Some(Self::DocumentCommand220),
            MetadataKind::InformationRegister => Some(Self::InformationRegisterDimension220),
            MetadataKind::AccumulationRegister => Some(Self::AccumulationRegisterDimension220),
            MetadataKind::DocumentJournal => Some(Self::DocumentJournalColumn220),
            _ => None,
        }
    }

    pub fn owner_kind(self) -> MetadataKind {
        match self {
            Self::CatalogTabularSection220 => MetadataKind::Catalog,
            Self::ChartTabularSection => MetadataKind::ChartOfCharacteristicTypes,
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
        let properties = match self {
            Self::CatalogTabularSection220 => CATALOG_MEMBER_PROPERTIES,
            Self::ChartTabularSection => &[TABULAR_SECTION_USE],
            Self::DocumentCommand220 => DOCUMENT_COMMAND_PROPERTIES,
            Self::InformationRegisterDimension220 => INFORMATION_DIMENSION_PROPERTIES,
            Self::AccumulationRegisterDimension220 => ACCUMULATION_DIMENSION_PROPERTIES,
            Self::DocumentJournalColumn220 => JOURNAL_COLUMN_PROPERTIES,
        };
        properties
            .iter()
            .filter(move |spec| spec.context == context)
    }

    pub fn context_for_field(self, field: &str) -> Option<MetaMemberReadContext> {
        let direct = match self {
            Self::CatalogTabularSection220 | Self::ChartTabularSection => None,
            Self::DocumentCommand220 => Some((
                "collections.commands[",
                MetaMemberReadContext::DocumentCommand,
            )),
            Self::InformationRegisterDimension220 => Some((
                "collections.dimensions[",
                MetaMemberReadContext::InformationRegisterDimension,
            )),
            Self::AccumulationRegisterDimension220 => Some((
                "collections.dimensions[",
                MetaMemberReadContext::AccumulationRegisterDimension,
            )),
            Self::DocumentJournalColumn220 => Some((
                "collections.columns[",
                MetaMemberReadContext::DocumentJournalColumn,
            )),
        };
        if let Some((prefix, context)) = direct {
            return indexed_field_suffix(field, prefix)?
                .is_empty()
                .then_some(context);
        }
        let rest = indexed_field_suffix(field, "collections.tabularSections[")?;
        if rest.is_empty() {
            return Some(MetaMemberReadContext::TabularSection);
        }
        indexed_field_suffix(rest, ".attributes[")?
            .is_empty()
            .then_some(MetaMemberReadContext::TabularAttribute)
    }

    pub fn context_for_address(
        self,
        address: &QualifiedAddress,
        kind: NodeKind,
    ) -> Option<MetaMemberReadContext> {
        if self != Self::CatalogTabularSection220 {
            let [owner, member] = address.segments() else {
                return None;
            };
            if owner.kind().as_str() != self.owner_kind().as_str()
                || owner.name().is_none()
                || member.name().is_none()
                || member.kind() != kind
            {
                return None;
            }
            let context = match self {
                Self::DocumentCommand220 => MetaMemberReadContext::DocumentCommand,
                Self::InformationRegisterDimension220 => {
                    MetaMemberReadContext::InformationRegisterDimension
                }
                Self::AccumulationRegisterDimension220 => {
                    MetaMemberReadContext::AccumulationRegisterDimension
                }
                Self::DocumentJournalColumn220 => MetaMemberReadContext::DocumentJournalColumn,
                Self::ChartTabularSection => MetaMemberReadContext::TabularSection,
                Self::CatalogTabularSection220 => unreachable!(),
            };
            return (context.xml_tag() == kind.as_str()).then_some(context);
        }
        match address.segments() {
            [owner, section]
                if owner.kind() == NodeKind::Catalog
                    && owner.name().is_some()
                    && section.kind() == NodeKind::TabularSection
                    && section.name().is_some()
                    && kind == NodeKind::TabularSection =>
            {
                Some(MetaMemberReadContext::TabularSection)
            }
            [owner, section, attribute]
                if owner.kind() == NodeKind::Catalog
                    && owner.name().is_some()
                    && section.kind() == NodeKind::TabularSection
                    && section.name().is_some()
                    && attribute.kind() == NodeKind::Attribute
                    && attribute.name().is_some()
                    && kind == NodeKind::Attribute =>
            {
                Some(MetaMemberReadContext::TabularAttribute)
            }
            _ => None,
        }
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
    let Ok(address) = unica_address::source_target::MetadataAddress::parse_logical(value) else {
        return false;
    };
    let parts = address.segments().collect::<Vec<_>>();
    matches!(parts.as_slice(), ["CommandGroup", _]) && address.as_str() == value
}

#[cfg(test)]
mod tests;
