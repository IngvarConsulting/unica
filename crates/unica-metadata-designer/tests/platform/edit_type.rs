use super::*;
#[test]
fn edit_type_reads_native_in_list_and_both_ways_for_both_profile_owners() {
    for (source,kind,expected) in [
        (include_str!(concat!(env!("CARGO_MANIFEST_DIR"),"/../../tests/fixtures/acceptance/workspace-metadata-edit-type/native-controls/Catalog-InList.xml")),MetadataKind::Catalog,"InList"),
        (include_str!(concat!(env!("CARGO_MANIFEST_DIR"),"/../../tests/fixtures/acceptance/workspace-metadata-edit-type/native-controls/Catalog-BothWays.xml")),MetadataKind::Catalog,"BothWays"),
        (include_str!(concat!(env!("CARGO_MANIFEST_DIR"),"/../../tests/fixtures/acceptance/workspace-metadata-edit-type/native-controls/InformationRegister-InList.xml")),MetadataKind::InformationRegister,"InList"),
        (include_str!(concat!(env!("CARGO_MANIFEST_DIR"),"/../../tests/fixtures/acceptance/workspace-metadata-edit-type/native-controls/InformationRegister-BothWays.xml")),MetadataKind::InformationRegister,"BothWays"),
    ]{let doc=Document::parse(source.trim_start_matches('\u{feff}')).unwrap();let (_,actual)=existing(doc.root_element().first_element_child().unwrap(),kind).unwrap();assert_eq!(actual,expected);}
}
