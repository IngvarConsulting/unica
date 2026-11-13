use super::*;
use std::collections::HashSet;
#[test]
fn cf_consumers_share_the_canonical_metadata_kind_order() {
    let validate_kinds = cf_validate_child_object_types();

    assert_eq!(validate_kinds.len(), 47);
    assert_eq!(validate_kinds, cf_type_order());
    let bot_index = validate_kinds
        .iter()
        .position(|kind| *kind == "Bot")
        .expect("Bot must be a modeled metadata kind");
    assert_eq!(validate_kinds[bot_index - 1], "CommonModule");
    assert_eq!(validate_kinds[bot_index + 1], "CommonAttribute");
    assert_eq!(cf_validate_child_type_dir("Bot"), Some("Bots"));
    assert_eq!(cf_type_ru_name("Bot"), "Боты");
    assert_eq!(cf_validate_child_type_dir("SyntheticMetadata"), None);

    let directories = validate_kinds
        .iter()
        .map(|kind| cf_validate_child_type_dir(kind).expect("known kind has a directory"))
        .collect::<HashSet<_>>();
    assert_eq!(directories.len(), validate_kinds.len());
    for kind in validate_kinds {
        let directory = cf_validate_child_type_dir(kind).unwrap();
        assert_eq!(cf_edit_dir_to_type(directory), Some(*kind));
    }
}

/// rule: INV.SOURCE.CHILD-OBJECTS-PLATFORM-ORDER
#[test]
fn child_object_insertion_orders_bot_and_rejects_unknown_kinds() {
    let mut xml = concat!(
        "<MetaDataObject><Configuration><ChildObjects>\n",
        "\t<CommonModule>Core</CommonModule>\n",
        "\t<CommonAttribute>Shared</CommonAttribute>\n",
        "</ChildObjects></Configuration></MetaDataObject>"
    )
    .to_string();

    assert!(
        crate::registration::cf_edit_add_child_object_text(&mut xml, "Bot", "Assistant").unwrap()
    );
    assert!(
        xml.find("<CommonModule>Core</CommonModule>").unwrap()
            < xml.find("<Bot>Assistant</Bot>").unwrap()
    );
    assert!(
        xml.find("<Bot>Assistant</Bot>").unwrap()
            < xml
                .find("<CommonAttribute>Shared</CommonAttribute>")
                .unwrap()
    );
    assert!(
        !crate::registration::cf_edit_add_child_object_text(&mut xml, "Bot", "Assistant").unwrap()
    );

    let before_unknown = xml.clone();
    let error = crate::registration::cf_edit_add_child_object_text(
        &mut xml,
        "SyntheticMetadata",
        "Unknown",
    )
    .expect_err("unknown metadata kinds must be rejected");
    assert!(
        error.contains("Unknown type 'SyntheticMetadata'"),
        "{error}"
    );
    assert_eq!(xml, before_unknown);
}

#[test]
fn child_object_insertion_preserves_crlf_for_existing_and_self_closing_lists() {
    let mut populated = concat!(
        "<MetaDataObject>\r\n",
        "\t<Configuration>\r\n",
        "\t\t<ChildObjects>\r\n",
        "\t\t\t<Catalog>Items</Catalog>\r\n",
        "\t\t</ChildObjects>\r\n",
        "\t</Configuration>\r\n",
        "</MetaDataObject>"
    )
    .to_string();

    assert!(
        crate::registration::cf_edit_add_child_object_text(&mut populated, "Report", "Sales")
            .unwrap()
    );
    assert!(populated.contains(concat!(
        "\t\t\t<Catalog>Items</Catalog>\r\n",
        "\t\t\t<Report>Sales</Report>\r\n",
        "\t\t</ChildObjects>"
    )));
    assert!(!populated.replace("\r\n", "").contains('\n'));

    let mut empty = concat!(
        "<MetaDataObject>\r\n",
        "\t<Configuration>\r\n",
        "\t\t<ChildObjects/>\r\n",
        "\t</Configuration>\r\n",
        "</MetaDataObject>"
    )
    .to_string();
    assert!(
        crate::registration::cf_edit_add_child_object_text(&mut empty, "Role", "User").unwrap()
    );
    assert!(empty.contains(concat!(
        "\t\t<ChildObjects>\r\n",
        "\t\t\t<Role>User</Role>\r\n",
        "\t\t</ChildObjects>"
    )));
    assert!(!empty.replace("\r\n", "").contains('\n'));
}

#[test]
fn child_object_insertion_preserves_cr_only_line_boundaries() {
    let mut xml = concat!(
        "<MetaDataObject>\r",
        "\t<Configuration>\r",
        "\t\t<ChildObjects>\r",
        "\t\t\t<Catalog>Items</Catalog>\r",
        "\t\t</ChildObjects>\r",
        "\t</Configuration>\r",
        "</MetaDataObject>"
    )
    .to_string();

    assert!(
        crate::registration::cf_edit_add_child_object_text(&mut xml, "Role", "Reader").unwrap()
    );

    assert!(
        xml.contains(concat!(
            "\t\t\t<Role>Reader</Role>\r",
            "\t\t\t<Catalog>Items</Catalog>\r",
            "\t\t</ChildObjects>"
        )),
        "{xml:?}"
    );
    assert!(!xml.contains('\n'));
}

#[test]
fn child_object_insertion_targets_only_the_root_metadata_child_objects() {
    let mut xml = concat!(
        "\u{feff}<MetaDataObject>\r\n",
        "\t<Configuration>\r\n",
        "\t\t<Properties>\r\n",
        "\t\t\t<!-- <ChildObjects><Role>CommentTrap</Role></ChildObjects> -->\r\n",
        "\t\t\t<Nested><ChildObjects><Role>NestedTrap</Role></ChildObjects></Nested>\r\n",
        "\t\t</Properties>\r\n",
        "\t\t<ChildObjects>\r\n",
        "\t\t\t<Catalog>Items</Catalog>\r\n",
        "\t\t</ChildObjects>\r\n",
        "\t</Configuration>\r\n",
        "</MetaDataObject>"
    )
    .to_string();

    assert!(
        crate::registration::cf_edit_add_child_object_text(&mut xml, "Role", "Reader").unwrap()
    );

    assert!(xml.contains("<Nested><ChildObjects><Role>NestedTrap</Role></ChildObjects></Nested>"));
    assert!(xml.contains(concat!(
        "\t\t<ChildObjects>\r\n",
        "\t\t\t<Role>Reader</Role>\r\n",
        "\t\t\t<Catalog>Items</Catalog>\r\n",
        "\t\t</ChildObjects>"
    )));
    assert_eq!(xml.matches("<Role>Reader</Role>").count(), 1);
    assert!(!xml.replace("\r\n", "").contains('\n'));
}

/// A palette colour, a kind of 8.5.1 and later, is registered where the
/// platform export of each line puts it: adding it to the export without it
/// gives the export byte for byte.
/// rule: INV.SOURCE.CHILD-OBJECTS-PLATFORM-ORDER
#[test]
fn a_palette_color_is_registered_where_the_platform_export_puts_it() {
    for export in [
        include_str!(
            "../../../../tests/fixtures/formats/platform_xml/2.21/all_kinds/src/Configuration.xml"
        ),
        include_str!(
            "../../../../tests/fixtures/formats/platform_xml/2.22/all_kinds/src/Configuration.xml"
        ),
    ] {
        let line = "\t\t\t<PaletteColor>Probe</PaletteColor>\r\n";
        let line = if export.contains(line) {
            line
        } else {
            &line[..line.len() - 2]
        };
        let mut without = export.replacen(line, "", 1);
        assert_ne!(without, export, "the export registers its palette colour");
        assert!(crate::registration::cf_edit_add_child_object_text(
            &mut without,
            "PaletteColor",
            "Probe"
        )
        .unwrap());
        assert_eq!(without, export);
    }
}

/// A queue, a kind of 8.5.4 and later, is registered where the platform
/// export puts it: adding it to the export without it gives the export byte
/// for byte.
/// rule: INV.SOURCE.CHILD-OBJECTS-PLATFORM-ORDER
#[test]
fn a_queue_is_registered_where_the_platform_export_puts_it() {
    let export = include_str!(
        "../../../../tests/fixtures/formats/platform_xml/2.22/all_kinds/src/Configuration.xml"
    );
    let line = "\t\t\t<Queue>Probe</Queue>\r\n";
    let line = if export.contains(line) {
        line
    } else {
        &line[..line.len() - 2]
    };
    let mut without = export.replacen(line, "", 1);
    assert_ne!(without, export, "the export registers its queue");
    assert!(
        crate::registration::cf_edit_add_child_object_text(&mut without, "Queue", "Probe").unwrap()
    );
    assert_eq!(without, export);
}
