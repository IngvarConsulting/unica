//! Факты схемы компоновки, которые чтение отдаёт независимо от представления.
use unica_metadata::support_state::ObjectSupportData;

/// Typed answer of `unica.dcs.info` (ADR-0023). Eleven `Mode` values each
/// rendered its own report; the data carries every section at once and a caller
/// projects what it needs, so `Limit` and `Offset` go away with the line layout.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DcsFacts {
    pub support: ObjectSupportData,
    pub data_sources: Vec<DcsInfoDataSource>,
    pub data_sets: Vec<DcsInfoDataSet>,
    pub links: Vec<DcsInfoLink>,
    pub calculated_fields: Vec<DcsInfoCalculatedField>,
    pub total_fields: Vec<DcsInfoTotalField>,
    pub parameters: Vec<DcsInfoParameter>,
    pub variants: Vec<DcsInfoVariant>,
    pub templates: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DcsInfoDataSource {
    pub name: String,
    /// `dataSourceType`, for example `Local`; `null` when the source omits it.
    pub kind: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DcsInfoDataSet {
    pub name: String,
    /// `Query`, `Object`, `Union` or the raw `xsi:type` when it is none of them.
    pub kind: String,
    /// The dataset query; `null` for kinds that carry none.
    pub query: Option<String>,
    /// The platform object an `Object` dataset reads; `null` for other kinds.
    pub object_name: Option<String>,
    /// The `dataSource` this dataset reads through; `null` when it names none.
    pub data_source: Option<String>,
    pub fields: Vec<DcsInfoField>,
    /// Nested datasets of a union; empty for every other kind.
    pub items: Vec<DcsInfoDataSet>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DcsInfoField {
    pub data_path: String,
    /// The query column behind the field; `null` when the field declares none.
    pub field: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DcsInfoLink {
    pub source: String,
    pub destination: String,
    pub source_expression: Option<String>,
    pub destination_expression: Option<String>,
    pub parameter: Option<String>,
    pub condition: Option<String>,
    pub start_expression: Option<String>,
    pub parameter_list_allowed: Option<bool>,
    pub required: Option<bool>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DcsInfoCalculatedField {
    pub data_path: String,
    pub expression: Option<String>,
    pub title: Option<String>,
    /// Which uses `useRestriction` bars, in schema order: `field`, `condition`,
    /// `group`, `order`. On a field the element is structured, not a flag, so a
    /// boolean cannot carry the answer the retired `calculated` report printed.
    /// `null` when the field declares no restriction at all.
    pub restrictions: Option<Vec<String>>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DcsInfoTotalField {
    pub data_path: String,
    pub expression: Option<String>,
    /// The grouping this total belongs to; `null` is the overall total, and
    /// without it a per-group total is indistinguishable from an overall one.
    pub group: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DcsInfoParameter {
    pub name: String,
    #[serde(rename = "type")]
    pub type_name: Option<String>,
    pub value: Option<String>,
    pub expression: Option<String>,
    /// True when `useRestriction` hides the parameter from the user.
    pub restricted: bool,
    /// `availableAsField`; `null` when the parameter declares none. `false`
    /// means the parameter cannot be used as a field, which the retired
    /// `params` report printed as `[noField]`.
    pub available_as_field: Option<bool>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
/// The platform writes a variant's own elements in the settings namespace, not
/// the schema one. Reading them with the schema namespace answered every
/// variant as an empty shell, so this section is bound to `dcsset` throughout.
pub struct DcsInfoVariant {
    pub name: String,
    pub presentation: Option<String>,
    pub selection: Vec<String>,
    pub order: Vec<DcsInfoOrderItem>,
    /// How many filter items the variant declares.
    pub filters: usize,
    pub structure: Vec<DcsInfoStructureItem>,
    pub structure_items: Vec<DcsInfoStructureRow>,
    pub setting_items: Vec<serde_json::Value>,
}

/// XML facts in document order. Indices identify rows within this variant,
/// not logical addresses: anonymous and duplicate names remain observable.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DcsInfoStructureRow {
    pub index: usize,
    pub parent_index: Option<usize>,
    pub axis: Option<String>,
    pub kind: String,
    pub name: Option<String>,
    pub group_by: Vec<String>,
    #[serde(flatten)]
    pub properties: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DcsInfoOrderItem {
    pub field: String,
    /// `Asc` or `Desc`; `null` when the item declares no direction.
    pub direction: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DcsInfoStructureItem {
    /// `Group`, `Table`, `Chart` or `Unknown`.
    pub kind: String,
    /// Fields the item groups by; empty for a detail item.
    pub group_by: Vec<String>,
}
