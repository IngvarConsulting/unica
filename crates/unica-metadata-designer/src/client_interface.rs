//! Panel layout (`Ext/ClientApplicationInterface.xml`) and home page
//! (`Ext/HomePageWorkArea.xml`) of a configuration root.
//!
//! Both documents are replaced as a whole from a closed model. The writer
//! spells them as the 8.3.27 platform re-exports them; instance identifiers
//! of panels and groups already in the document are kept, new ones are the
//! lowest stable identifiers the document does not use yet, so a repeated
//! edit writes the same bytes.

use crate::configuration::{
    parse_cf_home_page_xml_strict, CfHomePageItem, CF_CAI_NS, CF_HP_NS, CF_XR_NS,
};
use std::collections::{BTreeMap, BTreeSet};

pub const CLIENT_INTERFACE_RELATIVE: &str = "Ext/ClientApplicationInterface.xml";
pub const HOME_PAGE_RELATIVE: &str = "Ext/HomePageWorkArea.xml";

/// A standard panel of the client application. `Tools` is accepted by the
/// platform only in the top area: elsewhere 8.3.27 drops it on load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Panel {
    Sections,
    Favorites,
    History,
    Open,
    Functions,
    Tools,
}

impl Panel {
    /// Declaration order of a new document, as the platform writes it.
    pub const ALL: [Panel; 6] = [
        Self::Sections,
        Self::Favorites,
        Self::History,
        Self::Open,
        Self::Functions,
        Self::Tools,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Sections => "sections",
            Self::Favorites => "favorites",
            Self::History => "history",
            Self::Open => "open",
            Self::Functions => "functions",
            Self::Tools => "tools",
        }
    }

    pub const fn uuid(self) -> &'static str {
        match self {
            Self::Sections => "b553047f-c9aa-4157-978d-448ecad24248",
            Self::Favorites => "13322b22-3960-4d68-93a6-fe2dd7f28ca3",
            Self::History => "c933ac92-92cd-459d-81cc-e0c8a83ced99",
            Self::Open => "cbab57f2-a0f3-4f0a-89ea-4cb19570ab75",
            Self::Functions => "b2735bd3-d822-4430-ba59-c9e869693b24",
            Self::Tools => "8e10648b-f52d-4ec2-b4dd-87de33778d95",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|panel| panel.name() == name)
    }

    pub fn from_uuid(uuid: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|panel| panel.uuid() == uuid)
    }
}

/// The four areas around the working area, in document order.
pub const SIDES: [&str; 4] = ["top", "left", "right", "bottom"];

/// One entry of an area: a panel or a group of panels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelEntry {
    Panel(Panel),
    Group(Vec<Panel>),
}

impl PanelEntry {
    fn panels(&self) -> &[Panel] {
        match self {
            Self::Panel(panel) => std::slice::from_ref(panel),
            Self::Group(panels) => panels,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PanelLayout {
    /// Entries of `top`, `left`, `right`, `bottom`.
    pub sides: [Vec<PanelEntry>; 4],
}

impl PanelLayout {
    /// The read projection: areas with entries, a group as an array of names.
    pub fn to_json(&self) -> serde_json::Value {
        let entry = |entry: &PanelEntry| match entry {
            PanelEntry::Panel(panel) => serde_json::json!(panel.name()),
            PanelEntry::Group(panels) => {
                serde_json::json!(panels.iter().map(|panel| panel.name()).collect::<Vec<_>>())
            }
        };
        let mut object = serde_json::Map::new();
        for (side, entries) in SIDES.iter().zip(&self.sides) {
            if !entries.is_empty() {
                object.insert(
                    (*side).to_string(),
                    serde_json::Value::Array(entries.iter().map(entry).collect()),
                );
            }
        }
        serde_json::Value::Object(object)
    }

    /// A panel appears once in the layout; `tools` only at the top. The
    /// error names the entry, `side[index]`, and the reason.
    pub fn validate(&self) -> Result<(), (String, String)> {
        let mut seen = BTreeSet::new();
        for (side, entries) in SIDES.iter().zip(&self.sides) {
            for (index, entry) in entries.iter().enumerate() {
                for panel in entry.panels() {
                    if *panel == Panel::Tools && *side != "top" {
                        return Err((
                            format!("{side}[{index}]"),
                            "the `tools` panel is placed only in the `top` area".to_string(),
                        ));
                    }
                    if !seen.insert(*panel) {
                        return Err((
                            format!("{side}[{index}]"),
                            format!("panel `{}` is placed twice", panel.name()),
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

/// The stored document: its layout and what the writer keeps from it.
#[derive(Debug, Default)]
pub struct StoredPanels {
    pub layout: PanelLayout,
    /// Instance identifier of each placed panel.
    panel_ids: BTreeMap<Panel, String>,
    /// Identifier of each group, keyed by its first panel.
    group_ids: BTreeMap<Panel, String>,
    /// `panelDef` identifiers in document order.
    declared: Vec<String>,
    /// Every identifier the document uses.
    used: BTreeSet<String>,
    /// Platform identifiers of placed panels outside the six standard ones.
    pub unknown: Vec<String>,
}

impl StoredPanels {
    /// The read projection; a panel this model does not know is listed by
    /// its platform identifier instead of failing the read.
    pub fn to_json(&self) -> serde_json::Value {
        let mut json = self.layout.to_json();
        if !self.unknown.is_empty() {
            json["unknown"] = serde_json::json!(self.unknown);
        }
        json
    }
}

fn named(node: roxmltree::Node<'_, '_>, name: &str) -> bool {
    unica_xml_format::names::element_has_name(node, name, Some(CF_CAI_NS))
}

fn unexpected(node: roxmltree::Node<'_, '_>, place: &str) -> String {
    format!(
        "ClientApplicationInterface.xml has an unexpected `{}` in {place}",
        node.tag_name().name()
    )
}

impl StoredPanels {
    /// The standard panel of a `panel` element; an unknown one is recorded.
    fn panel(&mut self, node: roxmltree::Node<'_, '_>) -> Option<Panel> {
        let uuid = node
            .children()
            .find(|child| named(*child, "uuid"))
            .and_then(|child| child.text())
            .map(str::trim)
            .unwrap_or_default();
        let panel = Panel::from_uuid(uuid);
        match (panel, node.attribute("id")) {
            (Some(panel), Some(id)) => {
                self.panel_ids.insert(panel, id.to_string());
            }
            (None, _) => self.unknown.push(uuid.to_string()),
            _ => {}
        }
        panel
    }
}

pub fn parse_panels(text: &str) -> Result<StoredPanels, String> {
    let document = roxmltree::Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("ClientApplicationInterface.xml is malformed: {error}"))?;
    let root = document.root_element();
    if !named(root, "ClientApplicationInterface") {
        return Err("ClientApplicationInterface.xml has a wrong root element".to_string());
    }
    let mut stored = StoredPanels::default();
    for node in root.descendants().filter(roxmltree::Node::is_element) {
        if let Some(id) = node.attribute("id") {
            stored.used.insert(id.to_string());
        }
    }
    for node in root.children().filter(roxmltree::Node::is_element) {
        if named(node, "panelDef") {
            stored
                .declared
                .push(node.attribute("id").unwrap_or_default().to_string());
            continue;
        }
        let Some(side) = SIDES.iter().position(|side| named(node, side)) else {
            continue;
        };
        for item in node.children().filter(roxmltree::Node::is_element) {
            if named(item, "panel") {
                if let Some(panel) = stored.panel(item) {
                    stored.layout.sides[side].push(PanelEntry::Panel(panel));
                }
                continue;
            }
            if !named(item, "group") {
                return Err(unexpected(item, "an area"));
            }
            // A group holds one cell per panel, as the platform writes it.
            let mut panels = Vec::new();
            for cell in item.children().filter(roxmltree::Node::is_element) {
                if !named(cell, "group") {
                    return Err(unexpected(cell, "a group"));
                }
                for panel_node in cell.children().filter(roxmltree::Node::is_element) {
                    if !named(panel_node, "panel") {
                        return Err(unexpected(panel_node, "a group cell"));
                    }
                    panels.extend(stored.panel(panel_node));
                }
            }
            if let (Some(first), Some(id)) = (panels.first(), item.attribute("id")) {
                stored.group_ids.insert(*first, id.to_string());
            }
            if !panels.is_empty() {
                stored.layout.sides[side].push(PanelEntry::Group(panels));
            }
        }
    }
    Ok(stored)
}

/// The document for `layout`; `stored` is the current document, when any.
pub fn render_panels(layout: &PanelLayout, stored: Option<&StoredPanels>) -> String {
    let empty = StoredPanels::default();
    let stored = stored.unwrap_or(&empty);
    let mut used = stored.used.clone();
    let mut next = 1usize;
    let mut fresh = move || loop {
        let id = unica_xml_format::names::stable_uuid(next);
        next += 1;
        if used.insert(id.clone()) {
            return id;
        }
    };
    let panel_xml = |id: &str, panel: Panel, indent: &str| {
        format!(
            "{indent}<panel id=\"{id}\">\r\n{indent}\t<uuid>{}</uuid>\r\n{indent}</panel>\r\n",
            panel.uuid()
        )
    };
    let mut body = String::new();
    for (side, entries) in SIDES.iter().zip(&layout.sides) {
        for entry in entries {
            body.push_str(&format!("\t<{side}>\r\n"));
            match entry {
                PanelEntry::Panel(panel) => {
                    let id = stored
                        .panel_ids
                        .get(panel)
                        .cloned()
                        .unwrap_or_else(&mut fresh);
                    body.push_str(&panel_xml(&id, *panel, "\t\t"));
                }
                PanelEntry::Group(panels) => {
                    let id = panels
                        .first()
                        .and_then(|first| stored.group_ids.get(first).cloned())
                        .unwrap_or_else(&mut fresh);
                    body.push_str(&format!("\t\t<group id=\"{id}\">\r\n"));
                    for panel in panels {
                        let id = stored
                            .panel_ids
                            .get(panel)
                            .cloned()
                            .unwrap_or_else(&mut fresh);
                        body.push_str("\t\t\t<group>\r\n");
                        body.push_str(&panel_xml(&id, *panel, "\t\t\t\t"));
                        body.push_str("\t\t\t</group>\r\n");
                    }
                    body.push_str("\t\t</group>\r\n");
                }
            }
            body.push_str(&format!("\t</{side}>\r\n"));
        }
    }
    // The platform drops a placed panel it has no declaration for, so a
    // newly placed one is declared after the stored declarations.
    let mut declared = if stored.declared.is_empty() {
        Panel::ALL[..5]
            .iter()
            .map(|panel| panel.uuid().to_string())
            .collect::<Vec<_>>()
    } else {
        stored.declared.clone()
    };
    for panel in layout.sides.iter().flatten().flat_map(PanelEntry::panels) {
        if !declared.iter().any(|id| id == panel.uuid()) {
            declared.push(panel.uuid().to_string());
        }
    }
    for id in declared {
        body.push_str(&format!("\t<panelDef id=\"{id}\"/>\r\n"));
    }
    format!(
        "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
         <ClientApplicationInterface xmlns=\"{CF_CAI_NS}\" \
         xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" \
         xsi:type=\"InterfaceLayouter\">\r\n{body}</ClientApplicationInterface>"
    )
}

/// Templates of the home page working area.
pub const HOME_PAGE_TEMPLATES: [&str; 3] = [
    "OneColumn",
    "TwoColumnsEqualWidth",
    "TwoColumnsVariableWidth",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomePageItem {
    /// `CommonForm.F` or `Kind.Name.Form.F`.
    pub form: String,
    pub height: i64,
    /// Visibility for every role not listed in `roles`.
    pub common: bool,
    /// Per-role visibility: `Role.R` and the value.
    pub roles: Vec<(String, bool)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomePage {
    pub template: String,
    pub left: Vec<HomePageItem>,
    /// Empty for `OneColumn`.
    pub right: Vec<HomePageItem>,
}

pub fn parse_home_page(text: &str) -> Result<HomePage, String> {
    let layout = parse_cf_home_page_xml_strict(text.trim_start_matches('\u{feff}'))?;
    let items = |items: Vec<CfHomePageItem>| {
        items
            .into_iter()
            .map(|item| HomePageItem {
                form: item.form,
                height: item.height,
                common: item.common,
                roles: item.roles,
            })
            .collect()
    };
    Ok(HomePage {
        template: layout.template,
        left: items(layout.left),
        right: items(layout.right),
    })
}

pub fn render_home_page(page: &HomePage, format_version: &str) -> String {
    let column = |tag: &str, items: &[HomePageItem]| {
        if items.is_empty() {
            return format!("\t<{tag}/>\r\n");
        }
        let mut text = format!("\t<{tag}>\r\n");
        for item in items {
            text.push_str(&format!(
                "\t\t<Item>\r\n\t\t\t<Form>{}</Form>\r\n\t\t\t<Height>{}</Height>\r\n\
                 \t\t\t<Visibility>\r\n\t\t\t\t<xr:Common>{}</xr:Common>\r\n",
                unica_xml_format::text::escape_xml(&item.form),
                item.height,
                item.common
            ));
            for (role, visible) in &item.roles {
                text.push_str(&format!(
                    "\t\t\t\t<xr:Value name=\"{}\">{visible}</xr:Value>\r\n",
                    unica_xml_format::text::escape_xml(role)
                ));
            }
            text.push_str("\t\t\t</Visibility>\r\n\t\t</Item>\r\n");
        }
        text.push_str(&format!("\t</{tag}>\r\n"));
        text
    };
    // One column is spelled `Column`; 8.3.27 refuses `LeftColumn` there.
    let columns = if page.template == "OneColumn" {
        column("Column", &page.left)
    } else {
        column("LeftColumn", &page.left) + &column("RightColumn", &page.right)
    };
    format!(
        "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
         <HomePageWorkArea xmlns=\"{CF_HP_NS}\" xmlns:xr=\"{CF_XR_NS}\" \
         xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"{format_version}\">\r\n\
         \t<WorkingAreaTemplate>{}</WorkingAreaTemplate>\r\n{columns}</HomePageWorkArea>",
        page.template
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const NATIVE_EMPTY: &str = include_str!(
        "../../../tests/fixtures/platform_8_3_27/empty_configuration/src/Ext/ClientApplicationInterface.xml"
    );

    #[test]
    fn unchanged_layout_renders_the_native_document_byte_for_byte() {
        let stored = parse_panels(NATIVE_EMPTY).unwrap();
        assert_eq!(
            stored.layout.to_json(),
            serde_json::json!({"top":["open"],"left":["sections"]})
        );
        assert_eq!(render_panels(&stored.layout, Some(&stored)), NATIVE_EMPTY);
    }

    #[test]
    fn new_identifiers_are_the_lowest_unused_and_tools_gets_its_declaration() {
        let stored = parse_panels(NATIVE_EMPTY).unwrap();
        let layout = PanelLayout {
            sides: [
                vec![PanelEntry::Group(vec![Panel::Tools, Panel::Open])],
                vec![PanelEntry::Panel(Panel::Sections)],
                vec![],
                vec![PanelEntry::Panel(Panel::Favorites)],
            ],
        };
        let text = render_panels(&layout, Some(&stored));
        assert_eq!(render_panels(&layout, Some(&stored)), text);
        // The open and sections panels keep their ids; the group, the tools
        // and the favorites panels take 1, 2 and 3.
        assert!(text.contains("<group id=\"00000000-0000-0000-0000-000000000001\">"));
        assert!(text.contains(
            "<panel id=\"00000000-0000-0000-0000-000000000002\">\r\n\t\t\t\t\t<uuid>8e10648b"
        ));
        assert!(text.contains("<panel id=\"00000000-0000-0000-0000-000000000009\">"));
        assert!(text.contains(
            "<panel id=\"00000000-0000-0000-0000-000000000003\">\r\n\t\t\t<uuid>13322b22"
        ));
        assert!(text.ends_with(
            "\t<panelDef id=\"8e10648b-f52d-4ec2-b4dd-87de33778d95\"/>\r\n</ClientApplicationInterface>"
        ));
        assert_eq!(parse_panels(&text).unwrap().layout, layout);
    }

    #[test]
    fn an_unexpected_nesting_is_an_error_rather_than_a_silent_loss() {
        let nested = NATIVE_EMPTY.replace(
            "\t\t<panel id=\"00000000-0000-0000-0000-00000000000a\">",
            "\t\t<group id=\"00000000-0000-0000-0000-00000000000b\"><panel/></group>\r\n\t\t<panel id=\"00000000-0000-0000-0000-00000000000a\">",
        );
        assert!(parse_panels(&nested)
            .unwrap_err()
            .contains("unexpected `panel` in a group"));
    }

    #[test]
    fn a_panel_is_placed_once_and_tools_only_at_the_top() {
        let twice = PanelLayout {
            sides: [
                vec![PanelEntry::Panel(Panel::Open)],
                vec![PanelEntry::Group(vec![Panel::Sections, Panel::Open])],
                vec![],
                vec![],
            ],
        };
        assert_eq!(twice.validate().unwrap_err().0, "left[0]");
        let tools = PanelLayout {
            sides: [
                vec![],
                vec![],
                vec![PanelEntry::Panel(Panel::Tools)],
                vec![],
            ],
        };
        assert!(tools.validate().unwrap_err().1.contains("`top`"));
    }

    #[test]
    fn home_page_round_trips_one_and_two_columns() {
        for page in [
            HomePage {
                template: "OneColumn".to_string(),
                left: vec![HomePageItem {
                    form: "CommonForm.Card".to_string(),
                    height: 10,
                    common: true,
                    roles: vec![],
                }],
                right: vec![],
            },
            HomePage {
                template: "TwoColumnsEqualWidth".to_string(),
                left: vec![HomePageItem {
                    form: "Catalog.Items.Form.List".to_string(),
                    height: 15,
                    common: false,
                    roles: vec![("Role.Reader".to_string(), true)],
                }],
                right: vec![],
            },
        ] {
            let text = render_home_page(&page, "2.20");
            assert_eq!(parse_home_page(&text).unwrap(), page);
        }
    }
}
