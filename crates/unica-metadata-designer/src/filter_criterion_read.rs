//! Explicit FilterCriterion declarations, without opening the referenced graph.
use crate::info_projection::{
    direct_meta_info_property, direct_text_content, parse_bool, qname_resolves_to,
    strict_text_leaf_is_valid,
};
use roxmltree::{Document, Node};
use serde_json::{json, Value};
use unica_address::source_target::MetadataAddress;
const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
const READABLE: &str = "http://v8.1c.ru/8.3/xcf/readable";
const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";

/// A full declared attribute identity; this is not a physical descriptor path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentReference(String);
impl ContentReference {
    fn parse(raw: &str) -> Result<Self, String> {
        let parts = raw.split('.').collect::<Vec<_>>();
        let names=match parts.as_slice() {
            ["Catalog"|"Document",owner,"Attribute",attribute]=>vec![*owner,*attribute],
            ["Catalog"|"Document",owner,"TabularSection",section,"Attribute",attribute]=>vec![*owner,*section,*attribute],
            _=>return Err("Content reference must name a direct or tabular-section attribute of a document or catalog".into()),
        };
        if !names
            .into_iter()
            .all(unica_metadata::identifier::is_1c_identifier)
        {
            return Err("Content reference has an invalid identifier".into());
        }
        Ok(Self(raw.to_owned()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug)]
pub struct FilterCriterionFacts {
    pub content: Option<Vec<ContentReference>>,
    pub use_standard_commands: Option<bool>,
    unreadable: Vec<Value>,
}
impl FilterCriterionFacts {
    fn damaged(&mut self, field: impl Into<String>, message: impl Into<String>) {
        self.unreadable
            .push(json!({"field":field.into(),"message":message.into()}));
    }
    /// Reads the same descriptor image that proved the expected root identity.
    pub fn read(target: &MetadataAddress, bytes: &[u8]) -> Result<Self, String> {
        let parts = target.as_str().split('.').collect::<Vec<_>>();
        let ["FilterCriterion", name] = parts.as_slice() else {
            return Err("expected one FilterCriterion owner".into());
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
        if !owner.has_tag_name((MD, "FilterCriterion")) {
            return Err("wrong metadata owner kind".into());
        }
        let properties = direct_meta_info_property(*owner, "FilterCriterion", "Properties")
            .map_err(|e| e.message)?
            .ok_or("Properties is absent")?;
        if properties.attributes().len() != 0
            || properties
                .children()
                .any(|node| node.is_text() && !node.text().unwrap_or_default().trim().is_empty())
        {
            return Err("Properties has unsupported attributes or mixed text".into());
        }
        let identity = direct_meta_info_property(properties, "FilterCriterion", "Name")
            .map_err(|e| e.message)?
            .ok_or("Name is absent")?;
        if !strict_text_leaf_is_valid(identity) || direct_text_content(identity) != *name {
            return Err("descriptor identity differs from the expected FilterCriterion".into());
        }
        let mut facts = Self {
            content: None,
            use_standard_commands: None,
            unreadable: Vec::new(),
        };
        match direct_meta_info_property(properties, "FilterCriterion", "UseStandardCommands") {
            Ok(Some(node)) => match strict_text_leaf_is_valid(node)
                .then(|| direct_text_content(node))
                .ok_or_else(|| "UseStandardCommands must be a scalar without attributes".to_owned())
                .and_then(|text| {
                    parse_bool(&text, "properties.UseStandardCommands").map_err(|e| e.message)
                }) {
                Ok(flag) => facts.use_standard_commands = Some(flag),
                Err(error) => facts.damaged("properties.UseStandardCommands", error),
            },
            Ok(None) => {}
            Err(error) => facts.damaged("properties.UseStandardCommands", error.message),
        }
        match direct_meta_info_property(properties, "FilterCriterion", "Content") {
            Ok(Some(node)) => facts.read_content(node),
            Ok(None) => {}
            Err(error) => facts.damaged("relations.filterCriterionContent", error.message),
        }
        Ok(facts)
    }
    fn read_content(&mut self, node: Node<'_, '_>) {
        if node.attributes().len() != 0
            || node
                .children()
                .any(|n| n.is_text() && !n.text().unwrap_or_default().trim().is_empty())
        {
            self.damaged(
                "relations.filterCriterionContent",
                "Content has unexpected attributes or mixed text",
            );
            return;
        }
        let mut rows = Vec::new();
        for (index, item) in node.children().filter(Node::is_element).enumerate() {
            match read_item(item) {
                Ok(reference) => rows.push(reference),
                Err(message) => self.damaged(
                    format!("relations.filterCriterionContent[{index}]"),
                    message,
                ),
            }
        }
        self.content = Some(rows);
    }
    /// Publishes only this profile, keeping other independently observed facts.
    pub fn publish(&self, payload: &mut Value) {
        let mut properties = payload["properties"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if let Some(value) = self.use_standard_commands {
            properties.push(json!({"key":"UseStandardCommands","value":value}));
        }
        payload["properties"] = json!(properties);
        if !payload["relations"].is_object() {
            payload["relations"] = json!({});
        }
        payload["relations"]["filterCriterionContent"] = self
            .content
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
fn read_item(item: Node<'_, '_>) -> Result<ContentReference, String> {
    if !item.has_tag_name((READABLE, "Item")) || item.children().any(|n| n.is_element()) {
        return Err("Content entry must be a direct readable Item with text only".into());
    }
    let Some(type_) = item.attribute((XSI, "type")) else {
        return Err("Content Item has no xsi:type".into());
    };
    let type_matches = qname_resolves_to(item, type_, READABLE, "MDObjectRef")
        || (type_ == "MDObjectRef" && item.lookup_namespace_uri(None) == Some(READABLE));
    if item.attributes().len() != 1 || !type_matches {
        return Err(
            "Content Item must have only xsi:type resolving to readable MDObjectRef".into(),
        );
    }
    ContentReference::parse(&direct_text_content(item))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn read(properties: &str) -> FilterCriterionFacts {
        let target = MetadataAddress::parse(
            unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            "FilterCriterion.Probe",
        )
        .unwrap();
        let xml = format!(
            r#"<MetaDataObject xmlns="{MD}" xmlns:r="{READABLE}" xmlns:xsi="{XSI}" version="2.20"><FilterCriterion><Properties><Name>Probe</Name>{properties}</Properties></FilterCriterion></MetaDataObject>"#
        );
        FilterCriterionFacts::read(&target, xml.as_bytes()).unwrap()
    }
    #[test]
    fn content_reference_keeps_full_identity_and_rejects_unsupported_shapes() {
        for path in [
            "Catalog.Items.Attribute.Article",
            "Document.Order.TabularSection.Rows.Attribute.Product",
        ] {
            assert_eq!(ContentReference::parse(path).unwrap().as_str(), path);
        }
        for path in [
            "Catalog.Items",
            "Catalog.Items.StandardAttribute.Code",
            "Document.Order.TabularSection.Rows",
            "Catalog.Items.Attribute.A.Extra.More",
            "other:Catalog.Items.Attribute.A",
            "Catalog../File.Attribute.A",
            "InformationRegister.R.Dimension.D",
        ] {
            assert!(ContentReference::parse(path).is_err(), "{path}");
        }
    }
    #[test]
    fn content_qnames_use_namespaces_instead_of_prefix_spelling() {
        for item in [format!("<alias:Item xmlns:alias=\"{READABLE}\" xsi:type=\"alias:MDObjectRef\">Catalog.Items.Attribute.Article</alias:Item>"),format!("<Item xmlns=\"{READABLE}\" xsi:type=\"MDObjectRef\">Catalog.Items.Attribute.Article</Item>")] {
            let facts=read(&format!("<Content>{item}</Content>"));assert_eq!(facts.content.unwrap()[0].as_str(),"Catalog.Items.Attribute.Article");assert!(facts.unreadable.is_empty());
        }
        for item in ["<r:Item xsi:type=\"r:MDObjectRef\" xmlns:r=\"urn:foreign\">Catalog.Items.Attribute.Article</r:Item>","<r:Item xsi:type=\"unknown:MDObjectRef\">Catalog.Items.Attribute.Article</r:Item>","<r:Item xsi:type=\"r:Other\">Catalog.Items.Attribute.Article</r:Item>","<r:Item>Catalog.Items.Attribute.Article</r:Item>","<r:Item xsi:type=\"r:MDObjectRef\" vendor=\"hidden\">Catalog.Items.Attribute.Article</r:Item>","<r:Item xsi:type=\"r:MDObjectRef\"><Nested/></r:Item>"] {
            let facts=read(&format!("<UseStandardCommands>false</UseStandardCommands><Content>{item}</Content>"));assert!(facts.content.unwrap().is_empty());assert_eq!(facts.use_standard_commands,Some(false));assert_eq!(facts.unreadable.len(),1);
        }
    }
    #[test]
    fn content_container_damage_is_local_and_does_not_become_empty_success() {
        for content in [
            "<Content xmlns=\"urn:foreign\"/>",
            "<Content/><Content/>",
            "<Content vendor=\"hidden\"/>",
            "<Content>mixed text</Content>",
        ] {
            let facts = read(&format!(
                "<UseStandardCommands>true</UseStandardCommands>{content}"
            ));
            assert!(facts.content.is_none());
            assert_eq!(facts.use_standard_commands, Some(true));
            assert_eq!(facts.unreadable.len(), 1);
        }
        assert!(read("").content.is_none());
        assert_eq!(read("<Content/>").content.unwrap().len(), 0);
    }
}
