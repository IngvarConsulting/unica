use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeOutlineResult {
    pub module: String,
    pub identity: CodeOutlineIdentity,
    pub totals: CodeOutlineTotals,
    pub regions: Vec<CodeOutlineRegion>,
    pub methods: Vec<CodeOutlineMethod>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeOutlineIdentity {
    pub category: Option<String>,
    pub object: Option<String>,
    pub module_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeOutlineTotals {
    pub methods: usize,
    pub exports: usize,
    pub regions: usize,
    pub loc: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeOutlineRegion {
    pub name: Option<String>,
    pub line: usize,
    pub end_line: Option<usize>,
    pub regions: Vec<CodeOutlineRegion>,
    pub methods: Vec<CodeOutlineMethod>,
}

pub use unica_bsl_parser::{CodeOutlineMethod, CodeOutlineMethodKind, CodeOutlineParameter};
