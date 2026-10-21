//! Typed spreadsheet information facts returned by template reads.
use unica_metadata::support_state::ObjectSupportData as DomainObjectSupportData;
/// Typed answer of `unica.mxl.info` (ADR-0023).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MxlInfoData {
    pub name: String,
    pub support: DomainObjectSupportData,
    pub rows: i64,
    pub columns: i64,
    pub column_sets: Vec<MxlColumnSetData>,
    pub areas: Vec<MxlAreaData>,
    pub outside: MxlOutsideData,
    pub merge_count: usize,
    pub drawing_count: usize,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MxlColumnSetData {
    pub id: String,
    pub size: i64,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MxlAreaData {
    pub name: String,
    /// `Rows`, `Columns`, `Rectangle` or `Drawing`.
    pub kind: String,
    pub begin_row: i64,
    pub end_row: i64,
    pub begin_col: i64,
    pub end_col: i64,
    pub columns_id: Option<String>,
    pub drawing_id: Option<String>,
    pub params: Vec<String>,
    pub details: Vec<String>,
    /// How many non-empty cells the area holds. Always counted, so a branch
    /// can advertise its length without carrying the text.
    pub content_count: usize,
    /// `null` unless `WithText` asked for cell content, which stays a genuine
    /// content selector rather than a print-size lever. Cells keep their
    /// reading order: for a print form the order is the meaning.
    pub content: Option<Vec<MxlContentData>>,
}

/// One non-empty cell. `template` marks content that carries `[parameter]`
/// placeholders, which the area also lists under `params`.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MxlContentData {
    pub template: bool,
    pub text: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MxlOutsideData {
    pub params: Vec<String>,
    pub details: Vec<String>,
    pub content_count: usize,
    pub content: Option<Vec<MxlContentData>>,
}

/// The address kinds a template reader accepts. `Report.X.Template.Y` names a
/// nested template; `CommonTemplate.Y` names a top-level one.
pub const TEMPLATE_KINDS: &[&str] = &["Template", "CommonTemplate"];
