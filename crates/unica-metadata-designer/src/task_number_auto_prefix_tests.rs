use super::apply_typed_properties;
use unica_metadata::{MetaPropertyChanges, MetaPropertyInput, MetaPropertyValue, MetadataKind};

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
