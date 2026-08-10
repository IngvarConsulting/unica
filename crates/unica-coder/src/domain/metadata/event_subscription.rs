use super::{MetaDiagnostic, MetaDiagnosticCode};
use serde::Serialize;

/// A platform class that owns subscription events.
///
/// The enum is a domain identity. XML QNames are mapped to it only by the
/// Platform XML adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum EventSourceClass {
    CatalogObject,
    DocumentObject,
}

impl EventSourceClass {
    pub(crate) const ALL: &'static [Self] = &[Self::CatalogObject, Self::DocumentObject];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::CatalogObject => "catalogObject",
            Self::DocumentObject => "documentObject",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, MetaDiagnostic> {
        Self::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.as_str() == value)
            .ok_or_else(|| {
                MetaDiagnostic::error(
                    MetaDiagnosticCode::InvalidArguments,
                    format!("unsupported event source class `{value}`"),
                )
            })
    }
}
