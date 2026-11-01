use super::apply_typed_properties;
use unica_metadata::{MetaPropertyChanges, MetaPropertyInput, MetaPropertyValue, MetadataKind};

fn changes(value: &str) -> MetaPropertyChanges {
    MetaPropertyChanges::convert(
        MetadataKind::Catalog,
        vec![MetaPropertyInput::new(
            "CodeSeries",
            MetaPropertyValue::String(value.into()),
        )],
    )
    .unwrap()
}

fn catalog() -> String {
    "\u{feff}<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">\r\n<Catalog uuid=\"e2292000-0000-4000-8000-000000000001\"><Properties><Name>Items</Name><Owners><Item>Catalog.Owners</Item></Owners><Hierarchical>true</Hierarchical><CodeSeries>WholeCatalog</CodeSeries><CheckUnique>true</CheckUnique></Properties><ChildObjects/></Catalog>\r\n</MetaDataObject>".into()
}

#[test]
fn catalog_code_series_preserves_the_complete_descriptor_and_repeated_value() {
    let before = catalog();
    for value in [
        "WholeCatalog",
        "WithinOwnerSubordination",
        "WithinSubordination",
    ] {
        let mut image = before.clone();
        apply_typed_properties(&mut image, &changes(value), "ru").unwrap();
        assert_eq!(
            image,
            before.replace(
                "<CodeSeries>WholeCatalog</CodeSeries>",
                &format!("<CodeSeries>{value}</CodeSeries>"),
            )
        );
        let after = image.clone();
        apply_typed_properties(&mut image, &changes(value), "ru").unwrap();
        assert_eq!(image, after);
        apply_typed_properties(&mut image, &changes("WholeCatalog"), "ru").unwrap();
        assert_eq!(image, before);
    }
}

#[test]
fn catalog_code_series_does_not_repair_missing_or_damaged_input() {
    for replacement in [
        "",
        "<CodeSeries/>",
        "<CodeSeries>WithinParent</CodeSeries>",
        "<CodeSeries>WholeCatalog</CodeSeries><CodeSeries>WholeCatalog</CodeSeries>",
        "<CodeSeries><Value>WholeCatalog</Value></CodeSeries>",
    ] {
        let before = catalog().replace("<CodeSeries>WholeCatalog</CodeSeries>", replacement);
        let mut image = before.clone();
        assert!(
            apply_typed_properties(&mut image, &changes("WithinOwnerSubordination"), "ru").is_err(),
            "{replacement}"
        );
        assert_eq!(image, before);
    }
}

#[test]
fn catalog_code_series_equivalent_value_preserves_xml_spelling() {
    for value in [
        "<CodeSeries>Whole&#67;atalog</CodeSeries>",
        "<CodeSeries>Whole<!-- keep -->Catalog</CodeSeries>",
        "<md:CodeSeries xmlns:md=\"http://v8.1c.ru/8.3/MDClasses\">WholeCatalog</md:CodeSeries>",
    ] {
        let before = catalog().replace("<CodeSeries>WholeCatalog</CodeSeries>", value);
        let mut image = before.clone();
        apply_typed_properties(&mut image, &changes("WholeCatalog"), "ru").unwrap();
        assert_eq!(image, before, "{value}");
    }
}
