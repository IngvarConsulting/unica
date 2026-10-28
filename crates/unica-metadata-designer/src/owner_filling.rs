//! A local patch of the catalog's standard Owner, within the ordinary metadata plan.
use std::ops::Range;
use unica_metadata::{MetaDiagnostic, MetaDiagnosticCode, MetaObservedPropertyValue};

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
const XR: &str = "http://v8.1c.ru/8.3/xcf/readable";

fn damaged(message: impl Into<String>) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::InvalidArguments, message)
        .with_field("values.FillFromFillingValue")
}

fn only_child<'a, 'input>(
    parent: roxmltree::Node<'a, 'input>,
    name: &str,
    namespace: &str,
) -> Result<roxmltree::Node<'a, 'input>, MetaDiagnostic> {
    let mut children = parent
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == name);
    let child = children
        .next()
        .ok_or_else(|| damaged(format!("{name} is absent")))?;
    if children.next().is_some() || child.tag_name().namespace() != Some(namespace) {
        return Err(damaged(format!(
            "{name} is duplicated or has an invalid namespace"
        )));
    }
    Ok(child)
}

pub(crate) fn observe(xml: &str) -> Result<(bool, Range<usize>), MetaDiagnostic> {
    let body = xml.trim_start_matches('\u{feff}');
    let offset = xml.len() - body.len();
    let document = roxmltree::Document::parse(body)
        .map_err(|_| damaged("catalog descriptor is not valid XML"))?;
    let root = document.root_element();
    if !root.has_tag_name((MD, "MetaDataObject")) {
        return Err(damaged("catalog descriptor has an invalid root"));
    }
    let catalog = only_child(root, "Catalog", MD)?;
    let properties = only_child(catalog, "Properties", MD)?;
    let standard = only_child(properties, "StandardAttributes", MD)?;
    let mut owners = standard.children().filter(|n| {
        n.is_element()
            && n.tag_name().name() == "StandardAttribute"
            && n.attribute("name") == Some("Owner")
    });
    let owner = owners
        .next()
        .ok_or_else(|| damaged("standard Owner is absent"))?;
    if owners.next().is_some()
        || !owner.has_tag_name((XR, "StandardAttribute"))
        || owner.attributes().len() != 1
    {
        return Err(damaged(
            "standard Owner is duplicated or has an invalid namespace",
        ));
    }
    let property = only_child(owner, "FillFromFillingValue", XR)?;
    let observed = crate::info_projection::parse_standard_attribute_property(property, "values.FillFromFillingValue")
        .map_err(|_| damaged("Owner FillFromFillingValue is damaged; restore a confirmed working state before changing it"))?;
    let MetaObservedPropertyValue::Boolean { value } = observed else {
        return Err(damaged("Owner FillFromFillingValue is not a boolean"));
    };
    let texts: Vec<_> = property.children().filter(|n| n.is_text()).collect();
    let [text] = texts.as_slice() else {
        return Err(damaged(
            "Owner FillFromFillingValue must contain one boolean text",
        ));
    };
    Ok((
        value,
        text.range().start + offset..text.range().end + offset,
    ))
}

pub(crate) fn apply(xml: &mut String, enabled: bool) -> Result<(), MetaDiagnostic> {
    let (old, range) = observe(xml)?;
    if old != enabled {
        xml.replace_range(range, if enabled { "true" } else { "false" });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLAG: &str = "<xr:FillFromFillingValue>false</xr:FillFromFillingValue>";

    fn fixture() -> String {
        format!("\u{feff}<MetaDataObject xmlns=\"{MD}\" xmlns:xr=\"{XR}\" version=\"2.20\">\r\n<Catalog uuid=\"e2295000-0000-4000-8000-000000000001\"><Properties><Name>Items</Name><Owners><xr:Item>Catalog.Owners</xr:Item></Owners><StandardAttributes><xr:StandardAttribute name=\"Code\">{FLAG}</xr:StandardAttribute><xr:StandardAttribute name=\"Owner\"><xr:FillChecking>ShowError</xr:FillChecking>{FLAG}<xr:FillValue/></xr:StandardAttribute></StandardAttributes></Properties><ChildObjects/></Catalog>\r\n</MetaDataObject>")
    }

    #[test]
    fn owner_filling_changes_only_the_owners_boolean_and_repeats_without_writing() {
        let before = fixture();
        let mut xml = before.clone();
        apply(&mut xml, true).unwrap();
        let mut expected = before.clone();
        let start = expected.rfind(FLAG).unwrap();
        expected.replace_range(
            start..start + FLAG.len(),
            "<xr:FillFromFillingValue>true</xr:FillFromFillingValue>",
        );
        assert_eq!(xml, expected);
        assert!(observe(&xml).unwrap().0);
        apply(&mut xml, true).unwrap();
        assert_eq!(xml, expected);
        apply(&mut xml, false).unwrap();
        assert_eq!(xml, before);
    }

    #[test]
    fn owner_filling_rejects_damage_and_ambiguity_without_changing_bytes() {
        let original = fixture();
        let owner_start = original
            .find("<xr:StandardAttribute name=\"Owner\">")
            .unwrap();
        let owner_end = owner_start
            + original[owner_start..]
                .find("</xr:StandardAttribute>")
                .unwrap()
            + "</xr:StandardAttribute>".len();
        let owner = &original[owner_start..owner_end];
        let mut cases = vec![
            original.replace(owner, ""),
            original.replace(owner, &owner.repeat(2)),
            original.replace("xmlns:xr=", "xmlns:other="),
            original
                .replace("<Catalog ", "<Document ")
                .replace("</Catalog>", "</Document>"),
        ];
        for replacement in [
            "",
            "<xr:FillFromFillingValue/>",
            "<xr:FillFromFillingValue>1</xr:FillFromFillingValue>",
            "<xr:FillFromFillingValue>maybe</xr:FillFromFillingValue>",
            "<FillFromFillingValue>false</FillFromFillingValue>",
            "<xr:FillFromFillingValue extra=\"x\">false</xr:FillFromFillingValue>",
            "<xr:FillFromFillingValue><xr:Value>false</xr:Value></xr:FillFromFillingValue>",
        ] {
            cases.push(original.replace(owner, &owner.replace(FLAG, replacement)));
        }
        cases.push(original.replace(owner, &owner.replace(FLAG, &FLAG.repeat(2))));
        for before in cases {
            let mut xml = before.clone();
            assert!(apply(&mut xml, true).is_err(), "{before}");
            assert_eq!(xml, before);
        }
    }
}
