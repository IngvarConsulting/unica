use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticAction {
    Analyze,
    Findings,
    Status,
    Catalog,
}

impl DiagnosticAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Analyze => "analyze",
            Self::Findings => "findings",
            Self::Status => "status",
            Self::Catalog => "catalog",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "analyze" => Some(Self::Analyze),
            "findings" => Some(Self::Findings),
            "status" => Some(Self::Status),
            "catalog" => Some(Self::Catalog),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct DiagnosticProviderId(&'static str);

impl DiagnosticProviderId {
    pub const fn new_const(value: &'static str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

pub const BSL_ANALYZER_PROVIDER: DiagnosticProviderId =
    DiagnosticProviderId::new_const("bsl-analyzer");

pub const LIVE_DIAGNOSTIC_PROVIDERS: &[DiagnosticProviderId] = &[BSL_ANALYZER_PROVIDER];
