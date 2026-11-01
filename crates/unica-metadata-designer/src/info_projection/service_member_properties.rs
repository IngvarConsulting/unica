use super::*;
use unica_metadata::{MetaServiceMemberContext, MetaServiceMemberProperties};

/// Structural fields stay strict; only the closed descriptive profile is local.
pub(super) fn ensure_structure(
    node: roxmltree::Node<'_, '_>,
    allowed: &[&str],
    field: &str,
    context: MetaServiceMemberContext,
) -> Result<(), ProjectionError> {
    if node.attributes().len() != 0 || !node_has_only_whitespace_text(node) {
        return Err(ProjectionError::unsupported(
            field,
            "nested metadata container has unexpected attributes or mixed text",
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for child in node.children().filter(roxmltree::Node::is_element) {
        let name = child.tag_name().name();
        if context.property_names().contains(&name) {
            continue;
        }
        if child.tag_name().namespace() != Some(MD_CLASSES_NAMESPACE) || !allowed.contains(&name) {
            return Err(ProjectionError::unsupported(
                field,
                format!("unexpected nested metadata child `{name}`"),
            ));
        }
        if !seen.insert(name) {
            return Err(ProjectionError::unsupported(
                field,
                format!("duplicate nested metadata child `{name}`"),
            ));
        }
    }
    Ok(())
}

pub(super) fn read(
    properties: roxmltree::Node<'_, '_>,
    base: &str,
    context: MetaServiceMemberContext,
) -> MetaServiceMemberProperties {
    let mut result = MetaServiceMemberProperties::default();
    for &name in context.property_names() {
        let key = match name {
            "Synonym" => "synonym",
            "Comment" => "comment",
            _ => "dataLockControlMode",
        };
        let field = format!("{base}.{key}");
        let nodes = properties
            .children()
            .filter(|node| node.is_element() && node.tag_name().name() == name)
            .collect::<Vec<_>>();
        if nodes.is_empty() {
            continue;
        }
        let value = match nodes.as_slice() {
            [node] if node.tag_name().namespace() == Some(MD_CLASSES_NAMESPACE) => {
                parse(*node, name, &field)
            }
            _ => Err(ProjectionError::unsupported(
                &field,
                format!("{name} is duplicated or uses a foreign namespace"),
            )),
        };
        match value {
            Ok(value) => result.properties.push(MetaObservedProperty {
                name: name.to_string(),
                value,
            }),
            Err(error) => result
                .property_diagnostics
                .push(MetaDiagnostic::error(error.code, error.message).with_field(error.field)),
        }
    }
    result
}

fn parse(
    node: roxmltree::Node<'_, '_>,
    name: &str,
    field: &str,
) -> Result<MetaObservedPropertyValue, ProjectionError> {
    match name {
        "Synonym" => strict_localized_meta_info_property_value(node).ok_or_else(|| {
            ProjectionError::malformed(field, "nested metadata synonym is malformed")
        }),
        "Comment" if strict_text_leaf_is_valid(node) => Ok(MetaObservedPropertyValue::Text {
            value: direct_text_content(node),
        }),
        "Comment" => Err(ProjectionError::malformed(
            field,
            "nested metadata comment contains nested markup",
        )),
        _ => {
            if !strict_text_leaf_is_valid(node) {
                return Err(ProjectionError::malformed(
                    field,
                    "DataLockControlMode must contain text only",
                ));
            }
            let value = direct_text_content(node);
            if !["Automatic", "Managed"].contains(&value.as_str()) {
                return Err(ProjectionError::malformed(
                    field,
                    "Web service operation data lock control mode is outside the platform enum",
                ));
            }
            Ok(MetaObservedPropertyValue::Text { value })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_observations_keep_absent_empty_and_two_local_errors_distinct() {
        for (body,count,errors) in [
            ("",0,0),
            ("<Synonym/><Comment/>",2,0),
            ("<Comment><Hidden/></Comment><DataLockControlMode>AutomaticAndManaged</DataLockControlMode>",0,2),
            ("<Comment/><Comment/>",0,1),
            ("<Comment xmlns=\"urn:foreign\"/>",0,1),
        ] {
            let xml=format!("<Properties xmlns=\"{MD_CLASSES_NAMESPACE}\">{body}</Properties>");
            let doc=roxmltree::Document::parse(&xml).unwrap();
            ensure_structure(doc.root_element(),&["Name"],"details.operations[0]",MetaServiceMemberContext::Operation).unwrap();
            let facts=read(doc.root_element(),"details.operations[0]",MetaServiceMemberContext::Operation);
            assert_eq!(facts.properties.len(),count,"{body}");assert_eq!(facts.property_diagnostics.len(),errors,"{body}");
            if count==2 {
                assert_eq!(facts.properties[0].value,MetaObservedPropertyValue::LocalizedString{values:vec![]});
                assert_eq!(facts.properties[1].value,MetaObservedPropertyValue::Text{value:String::new()});
            }
            if errors==2 {assert!(facts.property_diagnostics.iter().all(|d|d.code==MetaDiagnosticCode::ValidationFailed));}
        }
    }

    #[test]
    fn service_descriptive_profile_does_not_relax_structural_or_other_context_fields() {
        for body in [
            "<Name>A</Name><Name>B</Name>",
            "<Handler xmlns=\"urn:foreign\">A</Handler>",
            "<Unknown/>",
            "<DataLockControlMode>Managed</DataLockControlMode>",
        ] {
            let xml = format!("<Properties xmlns=\"{MD_CLASSES_NAMESPACE}\">{body}</Properties>");
            let doc = roxmltree::Document::parse(&xml).unwrap();
            assert!(
                ensure_structure(
                    doc.root_element(),
                    &["Name", "Handler"],
                    "details.urlTemplates[0].methods[0]",
                    MetaServiceMemberContext::HTTPMethod
                )
                .is_err(),
                "{body}"
            );
        }
    }
}
