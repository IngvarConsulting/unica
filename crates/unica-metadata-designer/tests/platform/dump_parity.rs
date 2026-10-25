//! Byte parity with the 8.3.27 export for the writers fixed in #1404.
use super::apply_typed_operations_to_image_with_seed;
use crate::decode::{parse_legacy_edit, MetadataPlanKind};
use crate::template_catalog::{add_document_register_record, place_register_record};
use serde_json::json;
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_metadata::MetadataKind;

fn fixture(name: &str) -> String {
    let path = format!(
        "{}/tests/platform/dump_parity/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    String::from_utf8(std::fs::read(path).unwrap()).unwrap()
}

/// The planner's own line-ending treatment of a uniform CRLF descriptor.
fn as_crlf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

fn uuid_of_attribute(xml: &str, name: &str) -> String {
    let at = xml.find(&format!("<Name>{name}</Name>")).unwrap();
    let start = xml[..at].rfind("<Attribute uuid=\"").unwrap() + "<Attribute uuid=\"".len();
    xml[start..start + 36].to_string()
}

#[test]
fn chart_header_attributes_match_the_platform_export() {
    for (file, kind, name) in [
        (
            "ChartsOfAccounts",
            MetadataKind::ChartOfAccounts,
            "ChartOfAccounts.Хозрасчетный",
        ),
        (
            "ChartsOfCharacteristicTypes",
            MetadataKind::ChartOfCharacteristicTypes,
            "ChartOfCharacteristicTypes.ВидыСубконто",
        ),
        (
            "ChartsOfCalculationTypes",
            MetadataKind::ChartOfCalculationTypes,
            "ChartOfCalculationTypes.ВидыРасчета",
        ),
    ] {
        let before = fixture(&format!("{file}.before.xml"));
        let after = fixture(&format!("{file}.after.xml"));
        let target = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, name).unwrap();
        let MetadataPlanKind::Edit { operation, .. } = parse_legacy_edit(
            "main",
            target,
            kind,
            json!({"op": "add", "collection": "attributes", "elements": [{
                "name": "Реквизит",
                "type": {"variants": [{"kind": "string", "length": 10, "allowedLength": "variable"}]}
            }]}),
            |field| field.to_string(),
            0,
        )
        .unwrap() else {
            panic!("attribute.add plans an edit");
        };
        let mut text = before.trim_start_matches('\u{feff}').to_string();
        apply_typed_operations_to_image_with_seed(&mut text, &[operation], "ru", b"parity")
            .unwrap();
        let written = format!("\u{feff}{}", as_crlf(&text));
        let expected = after.replace(
            &uuid_of_attribute(&after, "Реквизит"),
            &uuid_of_attribute(&written, "Реквизит"),
        );
        assert_eq!(written, expected, "{file}");
    }
}

#[test]
fn register_records_take_their_own_lines_in_register_uuid_order() {
    // Creation order and the UUIDs the registers received in the export.
    let registers = [
        (
            "AccumulationRegister.Яблоки",
            "879eccf8-164f-4611-9361-4d5b493d7d83",
        ),
        (
            "CalculationRegister.Начисления",
            "a04a49cd-544f-4fd5-b1a3-af0065c07a66",
        ),
        (
            "AccumulationRegister.Апельсины",
            "9c601539-e364-44cf-aa84-2d96c8c3e5a0",
        ),
        (
            "AccountingRegister.Бух",
            "31587bae-a98e-42e3-9914-544306e48474",
        ),
    ];
    let mut document = fixture("Document.before.xml").into_bytes();
    for (register, _) in registers {
        document = add_document_register_record(&document, register).unwrap();
        let mut uuid_of = |reference: &str| {
            registers
                .iter()
                .find(|(name, _)| *name == reference)
                .map(|(_, uuid)| uuid.to_string())
        };
        if let Some(ordered) = place_register_record(&document, register, &mut uuid_of) {
            document = ordered;
        }
    }
    assert_eq!(
        String::from_utf8(document).unwrap(),
        fixture("Document.after.xml")
    );
}
