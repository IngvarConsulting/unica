//! Closed XML observations for a CommonPicture and its fixed companion.
//! Abs is a declaration, never a filesystem authority or a request to read bytes.
use roxmltree::{Document, Node};
use serde_json::{json, Map, Value};

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
const EXT: &str = "http://v8.1c.ru/8.3/xcf/extrnprops";
const READABLE: &str = "http://v8.1c.ru/8.3/xcf/readable";

fn parse(bytes: &[u8]) -> Result<Document<'_>, String> {
    let text = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
    Document::parse(text.trim_start_matches('\u{feff}')).map_err(|error| error.to_string())
}
fn child<'a, 'b>(node: Node<'a, 'b>, ns: &str, name: &str) -> Result<Option<Node<'a, 'b>>, String> {
    let nodes = node
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == name)
        .collect::<Vec<_>>();
    match nodes.as_slice() {
        [] => Ok(None),
        [only] if only.has_tag_name((ns, name)) => Ok(Some(*only)),
        _ => Err(format!("{name} is duplicated or has a foreign namespace")),
    }
}
fn text(node: Node<'_, '_>) -> Result<String, String> {
    if node.attributes().len() != 0 || node.children().any(|n| n.is_element()) {
        return Err("expected an explicit scalar without attributes or nested elements".into());
    }
    Ok(node
        .children()
        .filter(|n| n.is_text())
        .filter_map(|n| n.text())
        .collect())
}
fn boolean(node: Node<'_, '_>) -> Result<bool, String> {
    match text(node)?.trim() {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err("expected an explicit XML boolean".into()),
    }
}
fn appearance(bytes: &[u8]) -> Result<Option<bool>, String> {
    let doc = parse(bytes)?;
    let root = doc.root_element();
    if !root.has_tag_name((MD, "MetaDataObject")) {
        return Err("wrong metadata root".into());
    }
    let owner = child(root, MD, "CommonPicture")?.ok_or("CommonPicture is absent")?;
    let props = child(owner, MD, "Properties")?.ok_or("Properties is absent")?;
    child(props, MD, "AvailabilityForAppearance")?
        .map(boolean)
        .transpose()
}
#[derive(Default)]
struct Picture {
    abs: Option<String>,
    transparent: Option<bool>,
    unreadable: Vec<Value>,
}
impl Picture {
    fn damaged(&mut self, field: &str, message: impl Into<String>) {
        self.unreadable
            .push(json!({"field":format!("properties.Picture{field}"),"message":message.into()}));
    }
    fn read(bytes: &[u8]) -> Result<Self, String> {
        let doc = parse(bytes)?;
        let root = doc.root_element();
        if !root.has_tag_name((EXT, "ExtPicture")) {
            return Err("expected ExtPicture root in its declared namespace".into());
        }
        if root
            .attributes()
            .any(|a| a.namespace().is_some() || a.name() != "version")
        {
            return Err("unsupported ExtPicture attribute".into());
        }
        let container = child(root, EXT, "Picture")?.ok_or("Picture is absent")?;
        let mut result = Self::default();
        if root
            .children()
            .any(|n| n.is_element() && !n.has_tag_name((EXT, "Picture")))
            || root
                .children()
                .any(|n| n.is_text() && !n.text().unwrap_or_default().trim().is_empty())
        {
            result.damaged("", "unsupported ExtPicture content");
        }
        if container.attributes().len() != 0
            || container.children().any(|n| {
                n.is_element() && !["Abs", "LoadTransparent"].contains(&n.tag_name().name())
            })
            || container
                .children()
                .any(|n| n.is_text() && !n.text().unwrap_or_default().trim().is_empty())
        {
            result.damaged("", "unsupported Picture content");
        }
        match child(container, READABLE, "Abs")
            .and_then(|n| n.ok_or_else(|| "Abs is absent".into()))
            .and_then(text)
        {
            Ok(value) if !value.trim().is_empty() => result.abs = Some(value),
            Ok(_) => result.damaged(".Abs", "Abs is empty"),
            Err(error) => result.damaged(".Abs", error),
        }
        match child(container, READABLE, "LoadTransparent").and_then(|n| n.map(boolean).transpose())
        {
            Ok(value) => result.transparent = value,
            Err(error) => result.damaged(".LoadTransparent", error),
        }
        Ok(result)
    }
}
/// Enriches an already proven owner. Companion failures retain independent owner facts.
/// The caller supplies only the fixed XML companion, never the referenced payload.
pub fn enrich(owner: &[u8], companion: Result<&[u8], String>, payload: &mut Value) {
    let mut properties = Vec::new();
    let mut unreadable = Vec::new();
    match appearance(owner) {
        Ok(Some(value)) => {
            properties.push(json!({"key":"AvailabilityForAppearance","value":value}))
        }
        Ok(None) => {}
        Err(message) => unreadable
            .push(json!({"field":"properties.AvailabilityForAppearance","message":message})),
    }
    match companion.and_then(Picture::read) {
        Ok(picture) => {
            let mut value = Map::new();
            if let Some(abs) = picture.abs {
                value.insert("Abs".into(), json!(abs));
            }
            if let Some(flag) = picture.transparent {
                value.insert("LoadTransparent".into(), json!(flag));
            }
            if !value.is_empty() {
                properties.push(json!({"key":"Picture","value":value}));
            }
            unreadable.extend(picture.unreadable);
        }
        Err(message) => unreadable.push(json!({"field":"properties.Picture","message":message})),
    }
    payload["properties"] = json!(properties);
    if !unreadable.is_empty() {
        payload["unreadable"] = json!(unreadable);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn companion_observation_uses_expanded_names_and_retains_independent_fields() {
        let xml = format!(
            r#"<e:ExtPicture xmlns:e="{EXT}" xmlns:alias="{READABLE}" version="2.20"><e:Picture><alias:Abs>A&amp;B<![CDATA[.png]]></alias:Abs><alias:LoadTransparent>0</alias:LoadTransparent></e:Picture></e:ExtPicture>"#
        );
        let facts = Picture::read(xml.as_bytes()).unwrap();
        assert_eq!(facts.abs.as_deref(), Some("A&B.png"));
        assert_eq!(facts.transparent, Some(false));
        assert!(facts.unreadable.is_empty());
        for bad in [
            xml.replace(EXT, "urn:foreign"),
            xml.replace("</e:ExtPicture>", "<e:Picture/></e:ExtPicture>"),
            xml.replace("version=\"2.20\"", "vendor=\"hidden\""),
        ] {
            assert!(Picture::read(bad.as_bytes()).is_err());
        }
        let foreign = xml.replace(READABLE, "urn:foreign");
        let facts = Picture::read(foreign.as_bytes()).unwrap();
        assert!(facts.abs.is_none() && facts.transparent.is_none());
        assert_eq!(facts.unreadable.len(), 2);
    }
}
