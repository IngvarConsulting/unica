use super::{MetaObservedPropertyValue, MetadataKind};
use serde::{Deserialize, Serialize};
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_source_contract::SourceSetKind;

/// A read capability, independent of the metadata mutation grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetaMemberReadProfile {
    CatalogTabularSection220,
    InformationRegisterFields220,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaMemberReadContext {
    TabularSection,
    TabularAttribute,
    Attribute,
    Dimension,
    Resource,
}

#[derive(Debug, Clone, Copy)]
pub enum MetaMemberValueKind {
    UnsignedInteger,
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
        name: "FullTextSearch",
        context: MetaMemberReadContext::Attribute,
        kind: MetaMemberValueKind::Text(&["Use", "DontUse"]),
    },
    MetaMemberPropertySpec {
        name: "DataHistory",
        context: MetaMemberReadContext::Attribute,
        kind: MetaMemberValueKind::Text(&["Use", "DontUse"]),
    },
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

const REGISTER_MEMBER_PROPERTIES: &[MetaMemberPropertySpec] = &[
    MetaMemberPropertySpec {
        name: "FullTextSearch",
        context: MetaMemberReadContext::Dimension,
        kind: MetaMemberValueKind::Text(&["Use", "DontUse"]),
    },
    MetaMemberPropertySpec {
        name: "DataHistory",
        context: MetaMemberReadContext::Dimension,
        kind: MetaMemberValueKind::Text(&["Use", "DontUse"]),
    },
    MetaMemberPropertySpec {
        name: "FullTextSearch",
        context: MetaMemberReadContext::Resource,
        kind: MetaMemberValueKind::Text(&["Use", "DontUse"]),
    },
    MetaMemberPropertySpec {
        name: "DataHistory",
        context: MetaMemberReadContext::Resource,
        kind: MetaMemberValueKind::Text(&["Use", "DontUse"]),
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
            MetadataKind::InformationRegister => Some(Self::InformationRegisterFields220),
            _ => None,
        }
    }

    pub fn properties(
        self,
        context: MetaMemberReadContext,
    ) -> impl Iterator<Item = &'static MetaMemberPropertySpec> {
        let properties = match self {
            Self::CatalogTabularSection220 => CATALOG_MEMBER_PROPERTIES,
            Self::InformationRegisterFields220 => REGISTER_MEMBER_PROPERTIES,
        };
        properties
            .iter()
            .filter(move |spec| spec.context == context)
    }

    pub fn context_for_field(self, field: &str) -> Option<MetaMemberReadContext> {
        if self == Self::InformationRegisterFields220 {
            for (prefix, context) in [
                ("collections.dimensions[", MetaMemberReadContext::Dimension),
                ("collections.resources[", MetaMemberReadContext::Resource),
            ] {
                if indexed_field_suffix(field, prefix).is_some_and(str::is_empty) {
                    return Some(context);
                }
            }
            return None;
        }
        if indexed_field_suffix(field, "collections.attributes[").is_some_and(str::is_empty) {
            return Some(MetaMemberReadContext::Attribute);
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
        let owner_kind = match self {
            Self::CatalogTabularSection220 => NodeKind::Catalog,
            Self::InformationRegisterFields220 => NodeKind::InformationRegister,
        };
        if address
            .segments()
            .first()
            .is_none_or(|owner| owner.kind() != owner_kind)
        {
            return None;
        }
        match address.segments() {
            [owner, member]
                if owner.name().is_some() && member.name().is_some() && member.kind() == kind =>
            {
                match (self, kind) {
                    (Self::CatalogTabularSection220, NodeKind::Attribute) => {
                        Some(MetaMemberReadContext::Attribute)
                    }
                    (Self::CatalogTabularSection220, NodeKind::TabularSection) => {
                        Some(MetaMemberReadContext::TabularSection)
                    }
                    (Self::InformationRegisterFields220, NodeKind::Dimension) => {
                        Some(MetaMemberReadContext::Dimension)
                    }
                    (Self::InformationRegisterFields220, NodeKind::Resource) => {
                        Some(MetaMemberReadContext::Resource)
                    }
                    _ => None,
                }
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
            Self::TabularAttribute | Self::Attribute => "Attribute",
            Self::Dimension => "Dimension",
            Self::Resource => "Resource",
        }
    }
}

impl MetaMemberPropertySpec {
    pub fn value_kind(self) -> MetaMemberValueKind {
        self.kind
    }

    pub fn parse_text(self, value: &str) -> Option<MetaObservedPropertyValue> {
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
            MetaMemberValueKind::LocalizedString => None,
        }
    }

    pub fn accepts(self, value: &MetaObservedPropertyValue) -> bool {
        match (self.kind, value) {
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
