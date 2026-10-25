//! Byte parity with the 8.3.27 export for the first form of an owner (#1404).
use super::form_add_owner_image;
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};

fn fixture(name: &str) -> String {
    let path = format!(
        "{}/tests/platform/dump_parity/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    String::from_utf8(std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn the_first_form_of_an_empty_child_objects_takes_its_own_line() {
    let before = fixture("Catalog.before.xml");
    let owner = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Товары").unwrap();
    let after = form_add_owner_image(
        before.trim_start_matches('\u{feff}'),
        &owner,
        ("Catalog", "Товары"),
        "ФормаЭлемента",
        "DefaultObjectForm",
        "ops[0].args.at",
        "ops[0].args.items[0].name",
    )
    .unwrap();
    assert_eq!(format!("\u{feff}{after}"), fixture("Catalog.after.xml"));
}
