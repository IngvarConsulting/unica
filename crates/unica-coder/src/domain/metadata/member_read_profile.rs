use super::{MetaObservedPropertyValue, MetadataKind};
use crate::domain::project_sources::SourceSetKind;
use serde::{Deserialize, Serialize};

/// A read capability, independent of the metadata mutation grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MetaMemberReadProfile {
    CatalogTabularSection220,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum MetaMemberValueKind {
    UnsignedInteger,
    Text(&'static [&'static str]),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct MetaMemberPropertySpec {
    pub(crate) name: &'static str,
    kind: MetaMemberValueKind,
}

const CATALOG_TABULAR_PROPERTIES: &[MetaMemberPropertySpec] = &[
    MetaMemberPropertySpec {
        name: "LineNumberLength",
        kind: MetaMemberValueKind::UnsignedInteger,
    },
    MetaMemberPropertySpec {
        name: "Use",
        kind: MetaMemberValueKind::Text(&["ForItem", "ForFolder", "ForFolderAndItem"]),
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

    pub(crate) fn properties(self) -> &'static [MetaMemberPropertySpec] {
        match self {
            Self::CatalogTabularSection220 => CATALOG_TABULAR_PROPERTIES,
        }
    }

    pub(crate) fn owns_diagnostic(self, field: &str) -> bool {
        field.starts_with("collections.tabularSections[")
            && self
                .properties()
                .iter()
                .any(|spec| field.ends_with(&format!(".properties.{}", spec.name)))
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
