use super::apply_typed_properties;
use unica_metadata::{MetaPropertyChanges, MetaPropertyInput, MetaPropertyValue, MetadataKind};

fn prefixed_task(property: &str) -> String {
    format!(
        "<md:MetaDataObject xmlns:md=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">\
         <md:Task><md:Properties><md:Name>Approval</md:Name>\
         <md:Comment>keep</md:Comment>{property}</md:Properties></md:Task></md:MetaDataObject>"
    )
}

fn changes(value: &str) -> MetaPropertyChanges {
    MetaPropertyChanges::convert(
        MetadataKind::Task,
        vec![MetaPropertyInput::new(
            "TaskNumberAutoPrefix",
            MetaPropertyValue::String(value.to_string()),
        )],
    )
    .unwrap()
}

#[test]
fn task_number_auto_prefix_changes_only_the_selected_property_and_repeats_byte_identically() {
    for eol in ["\n", "\r\n"] {
        for (number_type, length, autonumbering) in [
            ("String", 14, true),
            ("Number", 9, false),
            ("String", 0, false),
        ] {
            let original = format!(
                "\u{feff}<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">\n\
                 <Task uuid=\"b1db2293-361b-4314-97cc-99bdc62bf20e\">\n\
                 <Properties><Name>Approval</Name><Comment>preserve</Comment>\n\
                 <NumberType>{number_type}</NumberType><NumberLength>{length}</NumberLength>\n\
                 <Autonumbering>{autonumbering}</Autonumbering><CheckUnique>false</CheckUnique>\n\
                 <TaskNumberAutoPrefix>BusinessProcessNumber</TaskNumberAutoPrefix>\n\
                 <Addressing>InformationRegister.Performers</Addressing>\n\
                 <MainAddressingAttribute>Task.Approval.AddressingAttribute.Performer</MainAddressingAttribute>\n\
                 <CurrentPerformer>SessionParameter.Performer</CurrentPerformer></Properties>\n\
                 <ChildObjects><AddressingAttribute uuid=\"b1db2293-361b-4314-97cc-99bdc62bf20f\">\n\
                 <Properties><Name>Performer</Name></Properties></AddressingAttribute></ChildObjects>\n\
                 </Task></MetaDataObject>"
            )
            .replace('\n', eol);
            let mut xml = original.clone();
            apply_typed_properties(&mut xml, &changes("DontUse"), "ru").unwrap();
            assert_eq!(
                xml,
                original.replace(
                    "<TaskNumberAutoPrefix>BusinessProcessNumber</TaskNumberAutoPrefix>",
                    "<TaskNumberAutoPrefix>DontUse</TaskNumberAutoPrefix>"
                )
            );
            let first = xml.clone();
            apply_typed_properties(&mut xml, &changes("DontUse"), "ru").unwrap();
            assert_eq!(xml.as_bytes(), first.as_bytes());
            apply_typed_properties(&mut xml, &changes("BusinessProcessNumber"), "ru").unwrap();
            assert_eq!(xml.as_bytes(), original.as_bytes());
        }
    }
}

#[test]
fn task_number_auto_prefix_missing_property_refuses_without_other_changes() {
    let mut xml = "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><Task><Properties><Name>Approval</Name><Comment>keep</Comment></Properties></Task></MetaDataObject>".to_string();
    let before = xml.clone();
    let error = apply_typed_properties(&mut xml, &changes("DontUse"), "ru").unwrap_err();
    assert!(error.message.contains("not present"));
    assert_eq!(xml, before);
}

#[test]
fn task_number_auto_prefix_equivalent_xml_keeps_its_lexical_bytes() {
    for value in ["Dont&#85;se", "<![CDATA[DontUse]]>", "Dont<!--keep-->Use"] {
        let before = prefixed_task(&format!(
            "<md:TaskNumberAutoPrefix>{value}</md:TaskNumberAutoPrefix>"
        ));
        let mut xml = before.clone();
        apply_typed_properties(&mut xml, &changes("DontUse"), "ru").unwrap();
        assert_eq!(xml.as_bytes(), before.as_bytes(), "{value}");
    }
}

#[test]
fn task_number_auto_prefix_change_preserves_qualified_element_and_namespace() {
    let before = prefixed_task(
        "<md:TaskNumberAutoPrefix xmlns:other=\"urn:keep\">BusinessProcessNumber</md:TaskNumberAutoPrefix>",
    );
    let mut xml = before.clone();
    apply_typed_properties(&mut xml, &changes("DontUse"), "ru").unwrap();
    assert_eq!(xml, before.replace("BusinessProcessNumber", "DontUse"));
    let doc = roxmltree::Document::parse(&xml).unwrap();
    let property = doc
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "TaskNumberAutoPrefix")
        .unwrap();
    assert_eq!(
        property.tag_name().namespace(),
        Some("http://v8.1c.ru/8.3/MDClasses")
    );
    drop(doc);
    apply_typed_properties(&mut xml, &changes("BusinessProcessNumber"), "ru").unwrap();
    assert_eq!(xml, before);
}

#[test]
fn task_number_auto_prefix_damaged_source_refuses_the_entire_property_batch() {
    let changes = MetaPropertyChanges::convert(
        MetadataKind::Task,
        vec![
            MetaPropertyInput::new("Comment", MetaPropertyValue::String("must not leak".into())),
            MetaPropertyInput::new(
                "TaskNumberAutoPrefix",
                MetaPropertyValue::String("DontUse".into()),
            ),
        ],
    )
    .unwrap();
    for property in [
        "<md:TaskNumberAutoPrefix>Unknown</md:TaskNumberAutoPrefix>",
        "<md:TaskNumberAutoPrefix/>",
        "<md:TaskNumberAutoPrefix><md:Value>DontUse</md:Value></md:TaskNumberAutoPrefix>",
        "<md:TaskNumberAutoPrefix extra=\"invalid\">DontUse</md:TaskNumberAutoPrefix>",
        "<TaskNumberAutoPrefix xmlns=\"urn:foreign\">DontUse</TaskNumberAutoPrefix>",
        "<md:TaskNumberAutoPrefix>DontUse</md:TaskNumberAutoPrefix><md:TaskNumberAutoPrefix>BusinessProcessNumber</md:TaskNumberAutoPrefix>",
    ] {
        let before = prefixed_task(property);
        let mut xml = before.clone();
        let error = apply_typed_properties(&mut xml, &changes, "ru").unwrap_err();
        assert_eq!(error.code, unica_metadata::MetaDiagnosticCode::InvalidArguments);
        assert_eq!(error.field.as_deref(), Some("values.TaskNumberAutoPrefix"));
        assert_eq!(xml, before, "{property}");
    }
}
