//! Bounded outgoing functional-option declarations. No reverse usage or graph scan.
use crate::info_projection::{
    direct_meta_info_property, direct_text_content, qname_resolves_to, strict_text_leaf_is_valid,
};
use roxmltree::{Document, Node};
use serde_json::{json, Value};
use unica_address::source_target::MetadataAddress;
const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
const READABLE: &str = "http://v8.1c.ru/8.3/xcf/readable";
const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclarationKind {
    Content,
    Use,
}
impl DeclarationKind {
    pub fn field(self) -> &'static str {
        match self {
            Self::Content => "Content",
            Self::Use => "Use",
        }
    }
    pub fn payload_field(self) -> &'static str {
        match self {
            Self::Content => "functionalOptionContent",
            Self::Use => "functionalParameterUse",
        }
    }
}
/// A full declared identity in one of this profile's supported target forms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionalReference(String);
impl FunctionalReference {
    fn parse(kind: DeclarationKind, raw: &str) -> Result<Self, String> {
        let parts = raw.split('.').collect::<Vec<_>>();
        let names = match (kind, parts.as_slice()) {
            (DeclarationKind::Content, ["Catalog" | "CommonCommand" | "Report", name]) => {
                vec![*name]
            }
            (DeclarationKind::Content, ["InformationRegister", owner, "Command", member]) => {
                vec![*owner, *member]
            }
            (DeclarationKind::Use, ["Catalog", name]) => vec![*name],
            (DeclarationKind::Use, ["InformationRegister", owner, "Dimension", member]) => {
                vec![*owner, *member]
            }
            _ => {
                return Err(format!(
                    "unsupported {} declaration target form",
                    kind.field()
                ))
            }
        };
        if !names
            .into_iter()
            .all(unica_metadata::identifier::is_1c_identifier)
        {
            return Err("declaration target has an invalid identifier".into());
        }
        Ok(Self(raw.to_owned()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Debug)]
pub struct FunctionalOptionFacts {
    pub kind: DeclarationKind,
    pub references: Option<Vec<FunctionalReference>>,
    unreadable: Vec<Value>,
}
impl FunctionalOptionFacts {
    fn damaged(&mut self, suffix: &str, message: impl Into<String>) {
        self.unreadable.push(json!({"field":format!("relations.{}{suffix}",self.kind.payload_field()),"message":message.into()}));
    }
    /// Observes the retained descriptor bytes and rechecks the expected identity.
    pub fn read(target: &MetadataAddress, bytes: &[u8]) -> Result<Self, String> {
        let parts = target.as_str().split('.').collect::<Vec<_>>();
        let [owner_kind @ ("FunctionalOption" | "FunctionalOptionsParameter"), name] =
            parts.as_slice()
        else {
            return Err("expected one functional-option declaration owner".into());
        };
        let kind = if *owner_kind == "FunctionalOption" {
            DeclarationKind::Content
        } else {
            DeclarationKind::Use
        };
        let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
        let doc =
            Document::parse(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
        let root = doc.root_element();
        if !root.has_tag_name((MD, "MetaDataObject")) {
            return Err("wrong metadata root".into());
        }
        let owners = root.children().filter(Node::is_element).collect::<Vec<_>>();
        let [owner] = owners.as_slice() else {
            return Err("expected one metadata owner".into());
        };
        if !owner.has_tag_name((MD, *owner_kind)) {
            return Err("descriptor kind differs from its logical owner".into());
        }
        let properties = direct_meta_info_property(*owner, owner_kind, "Properties")
            .map_err(|e| e.message)?
            .ok_or("Properties is absent")?;
        if properties.attributes().len() != 0
            || properties
                .children()
                .any(|n| n.is_text() && !n.text().unwrap_or_default().trim().is_empty())
        {
            return Err("Properties has unsupported attributes or mixed text".into());
        }
        let identity = direct_meta_info_property(properties, owner_kind, "Name")
            .map_err(|e| e.message)?
            .ok_or("Name is absent")?;
        if !strict_text_leaf_is_valid(identity) || direct_text_content(identity) != *name {
            return Err("descriptor name differs from its logical owner".into());
        }
        let mut facts = Self {
            kind,
            references: None,
            unreadable: Vec::new(),
        };
        match direct_meta_info_property(properties, owner_kind, kind.field()) {
            Ok(Some(node)) => facts.read_declarations(node),
            Ok(None) => {
                if kind == DeclarationKind::Use {
                    facts.damaged("","Use is absent; the functional-options parameter requires its use to be set");
                }
            }
            Err(error) => facts.damaged("", error.message),
        }
        Ok(facts)
    }
    fn read_declarations(&mut self, node: Node<'_, '_>) {
        if node.attributes().len() != 0
            || node
                .children()
                .any(|n| n.is_text() && !n.text().unwrap_or_default().trim().is_empty())
        {
            self.damaged(
                "",
                format!(
                    "{} has unsupported attributes or mixed text",
                    self.kind.field()
                ),
            );
            return;
        }
        let mut references = Vec::new();
        let mut entries = 0;
        for (index, item) in node.children().filter(Node::is_element).enumerate() {
            entries += 1;
            match read_entry(self.kind, item) {
                Ok(reference) => references.push(reference),
                Err(message) => self.damaged(&format!("[{index}]"), message),
            }
        }
        if self.kind == DeclarationKind::Use && entries == 0 {
            // 8.3.27 imports <Use/>, but its metadata check rejects it.
            self.damaged("","Use is explicitly empty; platform 8.3.27 requires the functional-options parameter use to be set");
        }
        self.references = Some(references);
    }
    /// Keeps unrelated facts and the existing ExchangePlan content representation.
    pub fn publish(&self, payload: &mut Value) {
        if !payload["relations"].is_object() {
            payload["relations"] = json!({});
        }
        payload["relations"][self.kind.payload_field()] = self
            .references
            .as_ref()
            .map(|rows| {
                json!(rows
                    .iter()
                    .map(|r| json!({"value":r.as_str()}))
                    .collect::<Vec<_>>())
            })
            .unwrap_or(Value::Null);
        if !self.unreadable.is_empty() {
            let mut unreadable = payload["unreadable"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            unreadable.extend(self.unreadable.iter().cloned());
            payload["unreadable"] = json!(unreadable);
        }
    }
}
fn read_entry(kind: DeclarationKind, item: Node<'_, '_>) -> Result<FunctionalReference, String> {
    let expected = if kind == DeclarationKind::Content {
        "Object"
    } else {
        "Item"
    };
    if !item.has_tag_name((READABLE, expected)) || item.children().any(|n| n.is_element()) {
        return Err(format!(
            "{} entry must be a direct readable {expected} with text only",
            kind.field()
        ));
    }
    match kind {
        DeclarationKind::Content => {
            if item.attributes().len() != 0 {
                return Err("Content Object must not have attributes or xsi:type".into());
            }
        }
        DeclarationKind::Use => {
            let Some(type_) = item.attribute((XSI, "type")) else {
                return Err("Use Item has no xsi:type".into());
            };
            let matches = qname_resolves_to(item, type_, READABLE, "MDObjectRef")
                || (type_ == "MDObjectRef" && item.lookup_namespace_uri(None) == Some(READABLE));
            if item.attributes().len() != 1 || !matches {
                return Err(
                    "Use Item must have only xsi:type resolving to readable MDObjectRef".into(),
                );
            }
        }
    }
    FunctionalReference::parse(kind, &direct_text_content(item))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn read(kind: &str, properties: &str) -> FunctionalOptionFacts {
        let target = MetadataAddress::parse(
            unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            &format!("{kind}.Probe"),
        )
        .unwrap();
        let xml = format!(
            r#"<MetaDataObject xmlns="{MD}" xmlns:r="{READABLE}" xmlns:xsi="{XSI}" version="2.20"><{kind}><Properties><Name>Probe</Name>{properties}</Properties></{kind}></MetaDataObject>"#
        );
        FunctionalOptionFacts::read(&target, xml.as_bytes()).unwrap()
    }
    #[test]
    fn functional_reference_target_shapes_are_closed_by_declaration_kind() {
        for path in [
            "Catalog.Items",
            "CommonCommand.Run",
            "Report.Main",
            "InformationRegister.R.Command.Open",
        ] {
            assert!(FunctionalReference::parse(DeclarationKind::Content, path).is_ok());
        }
        for path in ["Catalog.Items", "InformationRegister.R.Dimension.Warehouse"] {
            assert!(FunctionalReference::parse(DeclarationKind::Use, path).is_ok());
        }
        for (kind, path) in [
            (DeclarationKind::Use, "CommonCommand.Run"),
            (
                DeclarationKind::Content,
                "InformationRegister.R.Dimension.Warehouse",
            ),
            (DeclarationKind::Use, "InformationRegister.R.Command.Open"),
            (DeclarationKind::Content, "other:Catalog.Items"),
            (DeclarationKind::Content, "Catalog.Items.Attribute.Owner"),
            (DeclarationKind::Use, "Catalog../outside"),
        ] {
            assert!(FunctionalReference::parse(kind, path).is_err());
        }
    }
    #[test]
    fn functional_entry_namespaces_and_qnames_do_not_depend_on_prefix_spelling() {
        let valid = read(
            "FunctionalOption",
            &format!(
                "<Content><a:Object xmlns:a=\"{READABLE}\">Catalog.Items</a:Object></Content>"
            ),
        );
        assert_eq!(valid.references.unwrap()[0].as_str(), "Catalog.Items");
        assert!(valid.unreadable.is_empty());
        for item in [
            format!(
                "<a:Item xmlns:a=\"{READABLE}\" xsi:type=\"a:MDObjectRef\">Catalog.Items</a:Item>"
            ),
            format!("<Item xmlns=\"{READABLE}\" xsi:type=\"MDObjectRef\">Catalog.Items</Item>"),
        ] {
            let facts = read("FunctionalOptionsParameter", &format!("<Use>{item}</Use>"));
            assert_eq!(facts.references.unwrap().len(), 1);
            assert!(facts.unreadable.is_empty());
        }
        for item in [
            "<r:Item xmlns:r=\"urn:foreign\" xsi:type=\"r:MDObjectRef\">Catalog.Items</r:Item>",
            "<r:Item xsi:type=\"missing:MDObjectRef\">Catalog.Items</r:Item>",
            "<r:Item xsi:type=\"r:MDObjectRef\" vendor=\"hidden\">Catalog.Items</r:Item>",
            "<r:Item xsi:type=\"r:Other\">Catalog.Items</r:Item>",
            "<r:Item xsi:type=\"r:MDObjectRef\"><Nested/></r:Item>",
        ] {
            let facts = read("FunctionalOptionsParameter", &format!("<Use>{item}</Use>"));
            assert!(facts.references.unwrap().is_empty());
            assert_eq!(facts.unreadable.len(), 1);
        }
    }
    #[test]
    fn functional_empty_missing_and_malformed_observations_remain_distinct() {
        let empty = read("FunctionalOptionsParameter", "<Use/>");
        assert!(empty.references.unwrap().is_empty());
        assert!(empty.unreadable[0]["message"]
            .as_str()
            .unwrap()
            .contains("explicitly empty"));
        let missing = read("FunctionalOptionsParameter", "");
        assert!(missing.references.is_none());
        assert!(missing.unreadable[0]["message"]
            .as_str()
            .unwrap()
            .contains("absent"));
        for raw in [
            "<Use/><Use/>",
            "<Use vendor=\"hidden\"/>",
            "<Use>mixed</Use>",
            "<Use xmlns=\"urn:foreign\"/>",
        ] {
            let facts = read("FunctionalOptionsParameter", raw);
            assert!(facts.references.is_none());
            assert_eq!(facts.unreadable.len(), 1);
        }
        let empty_content = read("FunctionalOption", "<Content/>");
        assert!(empty_content.references.unwrap().is_empty());
        assert!(empty_content.unreadable.is_empty());
        let missing_content = read("FunctionalOption", "");
        assert!(missing_content.references.is_none());
        assert!(missing_content.unreadable.is_empty());
    }
    #[test]
    fn native_8_3_27_readback_preserves_empty_xml_without_accepting_it_as_valid_use() {
        let target = MetadataAddress::parse(
            unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            "FunctionalOptionsParameter.Склад",
        )
        .unwrap();
        let filled = FunctionalOptionFacts::read(
            &target,
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/platform_8_3_27/functional_option_links/filled-use.xml"
            )),
        )
        .unwrap();
        assert_eq!(filled.references.unwrap().len(), 2);
        assert!(filled.unreadable.is_empty());
        let empty = FunctionalOptionFacts::read(
            &target,
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/platform_8_3_27/functional_option_links/empty-use.xml"
            )),
        )
        .unwrap();
        assert_eq!(empty.references.unwrap().len(), 0);
        assert!(empty.unreadable[0]["message"]
            .as_str()
            .unwrap()
            .contains("requires"));
    }
}
