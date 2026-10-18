use super::{MetaObservedPropertyValue, MetadataKind};
use crate::domain::address::{NodeKind, QualifiedAddress};
use crate::domain::project_sources::SourceSetKind;
use serde::{Deserialize, Serialize};

/// A read capability, independent of the metadata mutation grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MetaMemberReadProfile {
    CatalogTabularSection220,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetaMemberReadContext {
    TabularSection,
    TabularAttribute,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum MetaMemberValueKind {
    UnsignedInteger,
    Text(&'static [&'static str]),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct MetaMemberPropertySpec {
    pub(crate) name: &'static str,
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
];

impl MetaMemberReadProfile {
    pub(crate) fn for_source(
        source_kind: SourceSetKind,
        owner: MetadataKind,
        format: Option<&str>,
    ) -> Option<Self> {
        (source_kind == SourceSetKind::Configuration
            && owner == MetadataKind::Catalog
            && format == Some("2.20"))
        .then_some(Self::CatalogTabularSection220)
    }

    pub(crate) fn properties(
        self,
        context: MetaMemberReadContext,
    ) -> impl Iterator<Item = &'static MetaMemberPropertySpec> {
        let properties = match self {
            Self::CatalogTabularSection220 => CATALOG_MEMBER_PROPERTIES,
        };
        properties
            .iter()
            .filter(move |spec| spec.context == context)
    }

    pub(crate) fn context_for_field(self, field: &str) -> Option<MetaMemberReadContext> {
        let rest = indexed_field_suffix(field, "collections.tabularSections[")?;
        if rest.is_empty() {
            return Some(MetaMemberReadContext::TabularSection);
        }
        indexed_field_suffix(rest, ".attributes[")?
            .is_empty()
            .then_some(MetaMemberReadContext::TabularAttribute)
    }

    pub(crate) fn context_for_address(
        self,
        address: &QualifiedAddress,
        kind: NodeKind,
    ) -> Option<MetaMemberReadContext> {
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

    pub(crate) fn owns_diagnostic(self, field: &str) -> bool {
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
    pub(crate) fn xml_tag(self) -> &'static str {
        match self {
            Self::TabularSection => "TabularSection",
            Self::TabularAttribute => "Attribute",
        }
    }
}

impl MetaMemberPropertySpec {
    pub(crate) fn parse_text(self, value: &str) -> Option<MetaObservedPropertyValue> {
        match self.kind {
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
        }
    }

    pub(crate) fn accepts(self, value: &MetaObservedPropertyValue) -> bool {
        match (self.kind, value) {
            (
                MetaMemberValueKind::UnsignedInteger,
                MetaObservedPropertyValue::UnsignedInteger { .. },
            ) => true,
            (MetaMemberValueKind::Text(values), MetaObservedPropertyValue::Text { value }) => {
                values.contains(&value.as_str())
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests;
