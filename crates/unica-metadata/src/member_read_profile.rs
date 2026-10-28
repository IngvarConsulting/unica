use super::{MetaObservedPropertyValue, MetadataKind};
use serde::{Deserialize, Serialize};
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_source_contract::SourceSetKind;

/// A read capability, independent of the metadata mutation grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetaMemberReadProfile {
    CatalogTabularSection220,
    DocumentFilling220,
    ExchangePlanFilling220,
    ChartOfAccountsFilling220,
    ChartOfCharacteristicTypesFilling220,
    ChartOfCalculationTypesFilling220,
    BusinessProcessFilling220,
    TaskFilling220,
    ReportFilling220,
    DataProcessorFilling220,
    InformationRegisterFilling220,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaMemberReadContext {
    TabularSection,
    HeaderAttribute,
    InformationDimension,
    InformationResource,
    TabularAttribute,
}

#[derive(Debug, Clone, Copy)]
pub enum MetaMemberValueKind {
    UnsignedInteger,
    Boolean,
    Text(&'static [&'static str]),
    LocalizedString,
}

#[derive(Debug, Clone, Copy)]
pub struct MetaMemberPropertySpec {
    pub name: &'static str,
    context: MetaMemberReadContext,
    kind: MetaMemberValueKind,
}

const CATALOG_MEMBER_PROPERTIES: &[MetaMemberPropertySpec] = &[
    MetaMemberPropertySpec {
        name: "LineNumberLength",
        context: MetaMemberReadContext::TabularSection,
        kind: MetaMemberValueKind::UnsignedInteger,
    },
    MetaMemberPropertySpec {
        name: "Use",
        context: MetaMemberReadContext::TabularSection,
        kind: MetaMemberValueKind::Text(&["ForItem", "ForFolder", "ForFolderAndItem"]),
    },
    MetaMemberPropertySpec {
        name: "Indexing",
        context: MetaMemberReadContext::TabularAttribute,
        kind: MetaMemberValueKind::Text(&["DontIndex", "Index", "IndexWithAdditionalOrder"]),
    },
    MetaMemberPropertySpec {
        name: "FullTextSearch",
        context: MetaMemberReadContext::TabularAttribute,
        kind: MetaMemberValueKind::Text(&["Use", "DontUse"]),
    },
    MetaMemberPropertySpec {
        name: "DataHistory",
        context: MetaMemberReadContext::TabularAttribute,
        kind: MetaMemberValueKind::Text(&["Use", "DontUse"]),
    },
    MetaMemberPropertySpec {
        name: "ToolTip",
        context: MetaMemberReadContext::TabularAttribute,
        kind: MetaMemberValueKind::LocalizedString,
    },
];

const FILLING_PROPERTIES: &[MetaMemberPropertySpec] = &[
    MetaMemberPropertySpec {
        name: "FillFromFillingValue",
        context: MetaMemberReadContext::HeaderAttribute,
        kind: MetaMemberValueKind::Boolean,
    },
    MetaMemberPropertySpec {
        name: "FillFromFillingValue",
        context: MetaMemberReadContext::TabularAttribute,
        kind: MetaMemberValueKind::Boolean,
    },
    MetaMemberPropertySpec {
        name: "FillFromFillingValue",
        context: MetaMemberReadContext::InformationDimension,
        kind: MetaMemberValueKind::Boolean,
    },
    MetaMemberPropertySpec {
        name: "FillFromFillingValue",
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
            MetadataKind::Document => Some(Self::DocumentFilling220),
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
            MetadataKind::InformationRegister => Some(Self::InformationRegisterFilling220),
            _ => None,
        }
    }
    pub fn owner_kind(self) -> MetadataKind {
        match self {
            Self::CatalogTabularSection220 => MetadataKind::Catalog,
            Self::DocumentFilling220 => MetadataKind::Document,
            Self::ExchangePlanFilling220 => MetadataKind::ExchangePlan,
            Self::ChartOfAccountsFilling220 => MetadataKind::ChartOfAccounts,
            Self::ChartOfCharacteristicTypesFilling220 => MetadataKind::ChartOfCharacteristicTypes,
            Self::ChartOfCalculationTypesFilling220 => MetadataKind::ChartOfCalculationTypes,
            Self::BusinessProcessFilling220 => MetadataKind::BusinessProcess,
            Self::TaskFilling220 => MetadataKind::Task,
            Self::ReportFilling220 => MetadataKind::Report,
            Self::DataProcessorFilling220 => MetadataKind::DataProcessor,
            Self::InformationRegisterFilling220 => MetadataKind::InformationRegister,
        }
    }
    pub fn properties(
        self,
        context: MetaMemberReadContext,
    ) -> impl Iterator<Item = &'static MetaMemberPropertySpec> {
        let properties = match (self, context) {
            (
                Self::CatalogTabularSection220,
                MetaMemberReadContext::TabularSection | MetaMemberReadContext::TabularAttribute,
            ) => CATALOG_MEMBER_PROPERTIES,
            (
                Self::CatalogTabularSection220
                | Self::DocumentFilling220
                | Self::ExchangePlanFilling220
                | Self::ChartOfAccountsFilling220
                | Self::ChartOfCharacteristicTypesFilling220
                | Self::ChartOfCalculationTypesFilling220
                | Self::BusinessProcessFilling220
                | Self::TaskFilling220
                | Self::InformationRegisterFilling220,
                MetaMemberReadContext::HeaderAttribute,
            ) => FILLING_PROPERTIES,
            (
                Self::ReportFilling220 | Self::DataProcessorFilling220,
                MetaMemberReadContext::TabularAttribute,
            ) => FILLING_PROPERTIES,
            (
                Self::InformationRegisterFilling220,
                MetaMemberReadContext::InformationDimension
                | MetaMemberReadContext::InformationResource,
            ) => FILLING_PROPERTIES,
            _ => &[],
        };
        properties
            .iter()
            .filter(move |spec| spec.context == context)
    }
    pub fn context_for_field(self, field: &str) -> Option<MetaMemberReadContext> {
        let direct = [
            (
                "collections.attributes[",
                MetaMemberReadContext::HeaderAttribute,
            ),
            (
                "collections.dimensions[",
                MetaMemberReadContext::InformationDimension,
            ),
            (
                "collections.resources[",
                MetaMemberReadContext::InformationResource,
            ),
        ];
        for (prefix, context) in direct {
            if indexed_field_suffix(field, prefix).is_some_and(str::is_empty) {
                return self.properties(context).next().map(|_| context);
            }
        }
        let rest = indexed_field_suffix(field, "collections.tabularSections[")?;
        let context = if rest.is_empty() {
            MetaMemberReadContext::TabularSection
        } else if indexed_field_suffix(rest, ".attributes[")?.is_empty() {
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
                NodeKind::Dimension => MetaMemberReadContext::InformationDimension,
                NodeKind::Resource => MetaMemberReadContext::InformationResource,
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
            Self::HeaderAttribute => "Attribute",
            Self::InformationDimension => "Dimension",
            Self::InformationResource => "Resource",
            Self::TabularSection => "TabularSection",
            Self::TabularAttribute => "Attribute",
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

#[cfg(test)]
mod tests;
