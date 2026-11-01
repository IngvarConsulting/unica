//! Closed read facts for root commands and command groups, independent of writers.
use crate::info_projection::{
    direct_meta_info_property, direct_text_content,
    observed_type_container_is_strict_but_unmodelled, parse_observed_metadata_type_container_node,
    strict_localized_meta_info_property_value,
};
use roxmltree::{Document, Node};
use serde_json::{json, Map, Value};
use unica_address::source_target::MetadataAddress;
use unica_metadata::{
    MetaDiagnostic, MetaDiagnosticCode, MetaDiagnosticSeverity, ObservedMetadataTypeVariant,
};
const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
const READABLE: &str = "http://v8.1c.ru/8.3/xcf/readable";

/// Facts and diagnostics share a single parse of the proven descriptor image.
pub struct CommandRootFacts {
    properties: Vec<Value>,
    relations: Map<String, Value>,
    pub diagnostics: Vec<MetaDiagnostic>,
    owner: MetadataAddress,
}
impl CommandRootFacts {
    fn damaged(&mut self, field: &str, message: impl Into<String>) {
        self.diagnostics.push(
            MetaDiagnostic::error(MetaDiagnosticCode::ValidationFailed, message)
                .with_field(format!("properties.{field}"))
                .with_metadata_path(self.owner.clone()),
        );
    }
    fn property(&mut self, name: &str, value: Value) {
        self.properties.push(json!({"key":name,"value":value}));
    }
    /// Checks the exact expected identity again in the bytes being parsed.
    pub fn read(owner: &MetadataAddress, bytes: &[u8]) -> Result<Self, String> {
        let parts = owner.as_str().split('.').collect::<Vec<_>>();
        let [kind @ ("CommonCommand" | "CommandGroup"), name] = parts.as_slice() else {
            return Err("expected one root command or command group".into());
        };
        let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
        let doc =
            Document::parse(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
        let root = doc.root_element();
        if !root.has_tag_name((MD, "MetaDataObject")) {
            return Err("wrong metadata root".into());
        }
        let objects = root.children().filter(Node::is_element).collect::<Vec<_>>();
        let [object] = objects.as_slice() else {
            return Err("expected one metadata owner".into());
        };
        if !object.has_tag_name((MD, *kind)) {
            return Err("descriptor kind differs from its logical owner".into());
        }
        let properties = direct_meta_info_property(*object, kind, "Properties")
            .map_err(|e| e.message)?
            .ok_or("Properties is absent")?;
        let identity = direct_meta_info_property(properties, kind, "Name")
            .map_err(|e| e.message)?
            .ok_or("Name is absent")?;
        if scalar(identity)? != *name {
            return Err("descriptor name differs from its logical owner".into());
        }
        let mut facts = Self {
            properties: Vec::new(),
            relations: Map::new(),
            diagnostics: Vec::new(),
            owner: owner.clone(),
        };
        let fields: &[&str] = if *kind == "CommonCommand" {
            &[
                "Representation",
                "ToolTip",
                "Picture",
                "CommandParameterType",
                "ParameterUseMode",
                "ModifiesData",
            ]
        } else {
            &["Comment", "Representation", "ToolTip", "Picture"]
        };
        for field in fields {
            let node = match direct_meta_info_property(properties, kind, field) {
                Ok(Some(node)) => node,
                Ok(None) => {
                    facts.missing_relation(field);
                    continue;
                }
                Err(error) => {
                    facts.damaged(field, error.message);
                    facts.missing_relation(field);
                    continue;
                }
            };
            match *field {
                "Picture" => facts.picture(node),
                "CommandParameterType" => facts.parameter_type(node),
                "ToolTip" => match strict_localized_meta_info_property_value(node) {
                    Some(value) => facts.property(field, json!(value)),
                    None => facts.damaged(field, "malformed localized ToolTip"),
                },
                _ => match scalar(node).and_then(|value| match *field {
                    "Representation"
                        if ["Auto", "Text", "Picture", "PictureAndText"]
                            .contains(&value.as_str()) =>
                    {
                        Ok(json!(value))
                    }
                    "ParameterUseMode" if ["Single", "Multiple"].contains(&value.as_str()) => {
                        Ok(json!(value))
                    }
                    "ModifiesData" => flag(&value).map(|value| json!(value)),
                    "Comment" => Ok(json!(value)),
                    _ => Err(format!("unsupported {field} value")),
                }) {
                    Ok(value) => facts.property(field, value),
                    Err(error) => facts.damaged(field, error),
                },
            }
        }
        Ok(facts)
    }
    fn missing_relation(&mut self, field: &str) {
        if let Some(key) = match field {
            "Picture" => Some("commandPicture"),
            "CommandParameterType" => Some("commandParameterTypes"),
            _ => None,
        } {
            self.relations.insert(key.into(), Value::Null);
        }
    }
    fn picture(&mut self, node: Node<'_, '_>) {
        let mut value = Map::new();
        let mut target = None;
        let mut ref_failed = false;
        if node.attributes().len() != 0
            || node
                .children()
                .any(|n| n.is_text() && !n.text().unwrap_or_default().trim().is_empty())
            || node.children().any(|n| {
                n.is_element() && !["Ref", "LoadTransparent"].contains(&n.tag_name().name())
            })
        {
            self.damaged("Picture", "unsupported Picture content");
            ref_failed = true;
        }
        for name in ["Ref", "LoadTransparent"] {
            let field = format!("Picture.{name}");
            let nodes = node
                .children()
                .filter(|n| n.is_element() && n.tag_name().name() == name)
                .collect::<Vec<_>>();
            let child = match nodes.as_slice() {
                [] => continue,
                [child] if child.has_tag_name((READABLE, name)) => *child,
                _ => {
                    self.damaged(&field, "duplicate or foreign Picture field");
                    if name == "Ref" {
                        ref_failed = true;
                    }
                    continue;
                }
            };
            match scalar(child).and_then(|text| {
                if name == "LoadTransparent" {
                    flag(&text).map(|v| json!(v))
                } else {
                    let (kind, name) = text
                        .split_once('.')
                        .ok_or("Picture Ref must name a picture")?;
                    if !["CommonPicture", "StdPicture"].contains(&kind)
                        || !unica_metadata::identifier::is_1c_identifier(name)
                    {
                        return Err("invalid Picture Ref".into());
                    }
                    if kind == "CommonPicture" {
                        target = Some(text.clone());
                    }
                    Ok(json!(text))
                }
            }) {
                Ok(observed) => {
                    value.insert(name.into(), observed);
                }
                Err(error) => {
                    self.damaged(&field, error);
                    if name == "Ref" {
                        ref_failed = true;
                    }
                }
            }
        }
        if node.children().any(|child| child.is_element())
            && !node
                .children()
                .any(|child| child.is_element() && child.tag_name().name() == "Ref")
        {
            self.damaged("Picture.Ref", "nonempty Picture has no source reference");
            ref_failed = true;
        }
        if value.is_empty() {
            self.property("Picture", json!({"kind":"empty"}));
        } else {
            self.property("Picture", json!(value));
        }
        self.relations.insert(
            "commandPicture".into(),
            if ref_failed {
                Value::Null
            } else {
                json!(target
                    .into_iter()
                    .map(|target| json!({"value":target}))
                    .collect::<Vec<_>>())
            },
        );
    }
    fn parameter_type(&mut self, node: Node<'_, '_>) {
        if node.attributes().len() == 0
            && node.children().all(|n| {
                !n.is_element() && (!n.is_text() || n.text().unwrap_or_default().trim().is_empty())
            })
        {
            self.property("CommandParameterType", json!({"kind":"empty"}));
            self.relations
                .insert("commandParameterTypes".into(), json!([]));
            return;
        }
        match parse_observed_metadata_type_container_node(node) {
            Ok(value) => {
                let links = value
                    .variants
                    .iter()
                    .filter_map(|variant| match variant {
                        ObservedMetadataTypeVariant::Reference { metadata_path }
                        | ObservedMetadataTypeVariant::DefinedType { metadata_path }
                        | ObservedMetadataTypeVariant::CharacteristicTypes { metadata_path } => {
                            Some(json!({"value":metadata_path.as_str()}))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                self.property("CommandParameterType", json!(value));
                self.relations
                    .insert("commandParameterTypes".into(), json!(links));
            }
            Err(error) => {
                self.missing_relation("CommandParameterType");
                if observed_type_container_is_strict_but_unmodelled(node) {
                    self.diagnostics.push(
                        MetaDiagnostic::warning(
                            MetaDiagnosticCode::ValidationWarning,
                            error.message,
                        )
                        .with_field("properties.CommandParameterType")
                        .with_metadata_path(self.owner.clone()),
                    );
                } else {
                    self.damaged("CommandParameterType", error.message);
                }
            }
        }
    }
    /// Only the closed observations above enter the identity payload.
    pub fn publish(&self, payload: &mut Value) {
        let mut properties = payload["properties"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        properties.extend(self.properties.iter().cloned());
        payload["properties"] = json!(properties);
        let mut relations = payload["relations"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        relations.extend(self.relations.clone());
        payload["relations"] = json!(relations);
        let unreadable=self.diagnostics.iter().map(|d|json!({"field":d.field,"message":if d.severity==MetaDiagnosticSeverity::Warning {format!("validation_warning: {}",d.message)}else{d.message.clone()}})).collect::<Vec<_>>();
        if !unreadable.is_empty() {
            payload["unreadable"] = json!(unreadable);
        }
    }
}
fn scalar(node: Node<'_, '_>) -> Result<String, String> {
    if node.attributes().len() != 0 || node.children().any(|n| n.is_element()) {
        return Err("expected a scalar without attributes or child elements".into());
    }
    Ok(direct_text_content(node))
}
fn flag(text: &str) -> Result<bool, String> {
    match text {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err("expected explicit true or false".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn read(kind: &str, properties: &str) -> CommandRootFacts {
        let target = MetadataAddress::parse(
            unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            &format!("{kind}.Probe"),
        )
        .unwrap();
        let xml = format!(
            r#"<MetaDataObject xmlns="{MD}" xmlns:v="http://v8.1c.ru/8.1/data/core" xmlns:x="http://www.w3.org/2001/XMLSchema" xmlns:c="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:r="{READABLE}" version="2.20"><{kind}><Properties><Name>Probe</Name>{properties}</Properties></{kind}></MetaDataObject>"#
        );
        CommandRootFacts::read(&target, xml.as_bytes()).unwrap()
    }
    #[test]
    fn root_type_observations_preserve_empty_absent_read_only_and_qname_context() {
        let absent = read("CommonCommand", "");
        assert!(!absent
            .properties
            .iter()
            .any(|p| p["key"] == "CommandParameterType"));
        assert!(absent.relations["commandParameterTypes"].is_null());
        let empty = read("CommonCommand", "<Picture/><CommandParameterType/>");
        assert!(empty.diagnostics.is_empty());
        assert!(empty
            .properties
            .iter()
            .any(|p| p["key"] == "CommandParameterType" && p["value"] == json!({"kind":"empty"})));
        assert_eq!(empty.relations["commandParameterTypes"], json!([]));
        for raw in [
            "<v:TypeSet>c:Characteristic.Options</v:TypeSet>",
            "<v:TypeId>16160000-0000-4000-8000-000000000001</v:TypeId>",
        ] {
            let facts = read(
                "CommonCommand",
                &format!("<CommandParameterType>{raw}</CommandParameterType>"),
            );
            assert!(facts.diagnostics.is_empty(), "{:?}", facts.diagnostics);
            assert_eq!(
                facts.properties[0]["value"]["mutationCapability"],
                "readOnly"
            );
        }
        let alias=read("CommonCommand","<CommandParameterType xmlns:alias=\"http://v8.1c.ru/8.1/data/enterprise/current-config\"><v:TypeSet>alias:DefinedType.Option</v:TypeSet></CommandParameterType>");
        assert!(alias.diagnostics.is_empty());
        assert_eq!(
            alias.relations["commandParameterTypes"][0]["value"],
            "DefinedType.Option"
        );
        let foreign=read("CommonCommand","<CommandParameterType xmlns:c=\"urn:foreign\"><v:TypeSet>c:DefinedType.Option</v:TypeSet></CommandParameterType>");
        assert_eq!(
            foreign.diagnostics[0].severity,
            MetaDiagnosticSeverity::Error
        );
    }
    #[test]
    fn root_type_unknown_warning_is_distinct_from_broken_qualifier_error() {
        for (raw,warning) in [("<v:Type>x:futureType</v:Type>",true),("<v:TypeSet>c:AnyIBRef</v:TypeSet>",true),("<v:Type>missing:boolean</v:Type>",false),("<v:Type>x:string</v:Type><v:StringQualifiers><v:Length>broken</v:Length><v:AllowedLength>Variable</v:AllowedLength></v:StringQualifiers>",false)] {
            let facts=read("CommonCommand",&format!("<CommandParameterType>{raw}</CommandParameterType><ModifiesData>false</ModifiesData>"));assert_eq!(facts.diagnostics.len(),1);assert_eq!(facts.diagnostics[0].code,if warning {MetaDiagnosticCode::ValidationWarning}else{MetaDiagnosticCode::ValidationFailed});assert!(facts.properties.iter().any(|p|p["key"]=="ModifiesData" && p["value"]==false));
        }
    }
    #[test]
    fn root_field_observation_is_closed_and_preserves_damage_independently() {
        for (kind,raw,key) in [("CommonCommand","<Representation>TextPicture</Representation>","Representation"),("CommonCommand","<ModifiesData>false</ModifiesData><ModifiesData>true</ModifiesData>","ModifiesData"),("CommandGroup","<Representation xmlns=\"urn:foreign\">Auto</Representation>","Representation"),("CommandGroup","<ToolTip><v:item><v:lang>ru</v:lang><v:content>One</v:content></v:item><v:item><v:lang>ru</v:lang><v:content>Two</v:content></v:item></ToolTip>","ToolTip")] {
            let facts=read(kind,raw);assert_eq!(facts.diagnostics.len(),1);assert_eq!(facts.diagnostics[0].field.as_deref(),Some(format!("properties.{key}").as_str()));
        }
        for kind in ["CommonCommand", "CommandGroup"] {
            for value in ["Auto", "Text", "Picture", "PictureAndText"] {
                assert!(
                    read(kind, &format!("<Representation>{value}</Representation>"))
                        .diagnostics
                        .is_empty()
                );
            }
        }
    }
}
