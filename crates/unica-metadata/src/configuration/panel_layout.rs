//! Explicit topology of a stored client application interface.

#[derive(Debug, serde::Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum CfPanelLayoutData {
    Complete {
        areas: Vec<CfPanelArea>,
        definitions: Vec<CfPanelDefinition>,
        limits: Vec<String>,
    },
    Limited {
        areas: Vec<CfPanelArea>,
        definitions: Vec<CfPanelDefinition>,
        limits: Vec<String>,
    },
    Unsupported {
        limits: Vec<String>,
    },
}

#[derive(Debug, serde::Serialize)]
pub struct CfPanelArea {
    pub side: String,
    pub items: Vec<CfPanelItem>,
}

#[derive(Debug, serde::Serialize)]
pub struct CfPanelDefinition {
    pub id: String,
}

/// Items follow XML preorder. A parent is an earlier group in the same
/// area, indexed from zero in this answer; it is not a persistent identity.
#[derive(Debug, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CfPanelItem {
    Group {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        parent: Option<usize>,
    },
    Panel {
        id: String,
        #[serde(rename = "definitionId")]
        definition_id: String,
        resolved: bool,
        parent: Option<usize>,
    },
}
