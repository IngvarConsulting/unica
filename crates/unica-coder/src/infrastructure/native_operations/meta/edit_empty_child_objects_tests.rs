use super::MetaCollection;

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";

fn owner(body: &str) -> String {
    format!(
        "<MetaDataObject xmlns=\"{MD}\" xmlns:md=\"{MD}\" xmlns:other=\"urn:other\"><Report><Properties><Name>Owner</Name><Comment>Сохранить &amp; текст</Comment></Properties>{body}</Report></MetaDataObject>"
    )
}

fn typed_template() -> &'static str {
    "<Template uuid=\"11111111-1111-4111-8111-111111111111\"><Properties><Name>Last</Name></Properties></Template>"
}

#[test]
fn last_registration_removal_uses_native_empty_child_objects_and_preserves_outer_bytes() {
    // Both retained native cases export this exact self-closing container.
    for tag in ["Form", "Template"] {
        for prefix in ["", "md:"] {
            for (opening_eol, child_eol, trailing_eol) in [
                ("\n", "\n", "\n"),
                ("\r\n", "\r\n", "\r\n"),
                ("\r\n", "\n", "\r\n"),
            ] {
                for bom in ["", "\u{feff}"] {
                    let before = format!(
                        "{bom}<?xml version=\"1.0\"?>{opening_eol}<root xmlns=\"{MD}\" xmlns:md=\"{MD}\"><Note>Привет &amp; мир</Note><untouched><ChildObjects> \r\n </ChildObjects></untouched>\r\n\t"
                    );
                    let after = format!("{trailing_eol}<!-- keep -->\n</root>");
                    let input = format!(
                        "{before}<{prefix}ChildObjects>{opening_eol}\t\t<{prefix}{tag}>Last</{prefix}{tag}>{child_eol}\t</{prefix}ChildObjects>{after}"
                    );
                    let expected = format!("{before}<{prefix}ChildObjects/>{after}");
                    let (actual, removed) =
                        unica_source_designer::registration::remove_metadata_child_text_with_flag(
                            &input, tag, "Last",
                        );
                    assert!(removed);
                    assert_eq!(actual, expected, "{tag}, prefix={prefix}, BOM={bom:?}");
                    assert_eq!(
                        unica_source_designer::registration::remove_metadata_child_text_with_flag(
                            &actual, tag, "Last"
                        ),
                        (actual.clone(), false),
                        "a repeated removal must not sweep existing empty containers",
                    );
                }
            }
        }
    }
}

#[test]
fn last_typed_child_removal_compacts_only_its_direct_parent() {
    for (inner, outer, ns) in [("", "", ""), ("\n\t", "\r\n", ""), ("\r\n\t", "\n", "md:")] {
        let prefix = "<untouched><ChildObjects> \n </ChildObjects></untouched>";
        let suffix = "<other:ChildObjects>\r\n</other:ChildObjects>";
        let mut actual = owner(&format!(
            "{prefix}{outer}<{ns}ChildObjects>{inner}{}{inner}</{ns}ChildObjects>{outer}{suffix}",
            typed_template()
        ));
        let expected = owner(&format!(
            "{prefix}{outer}<{ns}ChildObjects/>{outer}{suffix}"
        ));
        unica_metadata_designer::edit::remove_typed_element(
            &mut actual,
            MetaCollection::Templates,
            None,
            "Last",
        )
        .unwrap();
        assert_eq!(actual, expected);
    }
}

#[test]
fn scoped_removal_preserves_outer_collection_and_parent_opening_spelling() {
    let before = "<ChildObjects>\r\n<TabularSection uuid=\"11111111-1111-4111-8111-111111111111\"><Properties><Name>Rows</Name></Properties>";
    let after = "</TabularSection>\r\n<Form>Keep</Form></ChildObjects>";
    let mut actual = owner(&format!(
        "{before}<md:ChildObjects \t>\n<Attribute uuid=\"22222222-2222-4222-8222-222222222222\"><Properties><Name>Last</Name></Properties></Attribute>\r\n</md:ChildObjects>{after}"
    ));
    let expected = owner(&format!("{before}<md:ChildObjects \t/>{after}"));
    unica_metadata_designer::edit::remove_typed_element(
        &mut actual,
        MetaCollection::Attributes,
        Some("Rows"),
        "Last",
    )
    .unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn inline_registration_removal_preserves_neighbors_and_parent_prefix() {
    for prefix in ["", "md:"] {
        let input = owner(&format!(
            "<untouched><ChildObjects> </ChildObjects></untouched><{prefix}ChildObjects> <{prefix}Form>Last</{prefix}Form>\t</{prefix}ChildObjects><!-- after -->"
        ));
        let expected = owner(&format!(
            "<untouched><ChildObjects> </ChildObjects></untouched><{prefix}ChildObjects/><!-- after -->"
        ));
        let (actual, removed) =
            unica_source_designer::registration::remove_metadata_child_text_with_flag(
                &input, "Form", "Last",
            );
        assert!(removed);
        assert_eq!(actual, expected);
    }
}

#[test]
fn registration_removal_preserves_attributed_or_nonempty_container_bytes() {
    for (opening, retained) in [
        ("<ChildObjects>", "<!-- keep -->"),
        ("<ChildObjects>", "<?keep value?>"),
        ("<ChildObjects>", "<![CDATA[ ]]>"),
        ("<ChildObjects>", "text"),
        ("<ChildObjects>", "&#x20;"),
        ("<ChildObjects>", "\u{a0}"),
        ("<ChildObjects>", "<Form>Other</Form>"),
        ("<ChildObjects>", "<other:ChildObjects/>"),
        ("<ChildObjects custom=\"keep\">", ""),
        ("<ChildObjects xmlns:local=\"urn:keep\">", ""),
    ] {
        let before = format!("{opening}\r\n\t<Form>Last</Form>\n\t{retained}\r\n</ChildObjects>");
        let expected = format!("{opening}\r\n\t{retained}\r\n</ChildObjects>");
        let (actual, removed) =
            unica_source_designer::registration::remove_metadata_child_text_with_flag(
                &owner(&before),
                "Form",
                "Last",
            );
        assert!(removed);
        assert_eq!(actual, owner(&expected), "{opening}, {retained:?}");
    }
}

#[test]
fn typed_removal_preserves_attributed_or_nonempty_container_bytes() {
    for (opening, retained) in [
        ("<ChildObjects>", "<!-- keep -->"),
        ("<ChildObjects>", "<?keep value?>"),
        ("<ChildObjects>", "<![CDATA[ ]]>"),
        ("<ChildObjects>", "text"),
        ("<ChildObjects>", "&#x20;"),
        ("<ChildObjects>", "\u{a0}"),
        ("<ChildObjects>", "<Form>Other</Form>"),
        ("<ChildObjects custom=\"keep\">", ""),
        ("<ChildObjects xmlns:local=\"urn:keep\">", ""),
    ] {
        let mut actual = owner(&format!(
            "{opening}\n\t{}\r\n\t{retained}\n</ChildObjects>",
            typed_template()
        ));
        let expected = owner(&format!("{opening}\n\t{retained}\n</ChildObjects>"));
        unica_metadata_designer::edit::remove_typed_element(
            &mut actual,
            MetaCollection::Templates,
            None,
            "Last",
        )
        .unwrap();
        assert_eq!(actual, expected, "{opening}, {retained:?}");
    }
}

#[test]
fn removing_a_registration_does_not_normalize_an_unrelated_qname_or_content_parent() {
    for (opening, closing) in [
        ("<other:ChildObjects>", "</other:ChildObjects>"),
        ("<ChildObjects xmlns=\"urn:other\">", "</ChildObjects>"),
        ("<ChildObjects xmlns=\"\">", "</ChildObjects>"),
        ("<Content>", "</Content>"),
    ] {
        let untouched = format!("{opening}<Form>Last</Form>{closing}\r\n");
        let input = owner(&format!(
            "{untouched}<ChildObjects>\n\t<Form>Last</Form>\n</ChildObjects>"
        ));
        let expected = owner(&format!("{untouched}<ChildObjects/>"));
        let (actual, removed) =
            unica_source_designer::registration::remove_metadata_child_text_with_flag(
                &input, "Form", "Last",
            );
        assert!(removed);
        assert_eq!(actual, expected);
    }
}

#[test]
fn registration_compaction_does_not_remove_matching_comment_or_foreign_sibling() {
    let before = "<!-- <Form>Last</Form> -->\r\n<other:ChildObjects><other:Form>Last</other:Form></other:ChildObjects>\n";
    let input = owner(&format!(
        "{before}<ChildObjects>\r\n\t<Form>Last</Form>\n</ChildObjects>"
    ));
    let (actual, removed) =
        unica_source_designer::registration::remove_metadata_child_text_with_flag(
            &input, "Form", "Last",
        );
    assert!(removed);
    assert_eq!(actual, owner(&format!("{before}<ChildObjects/>")));
}

#[test]
fn absent_registration_leaves_all_existing_empty_child_objects_unchanged() {
    let input = owner("<ChildObjects>\r\n\t</ChildObjects><other:ChildObjects/> ");
    assert_eq!(
        unica_source_designer::registration::remove_metadata_child_text_with_flag(
            &input, "Form", "Missing"
        ),
        (input.clone(), false),
    );
}

#[test]
fn temporary_range_cut_for_a_move_keeps_the_expanded_container() {
    let child = typed_template();
    let mut actual = owner(&format!("<ChildObjects>{child}</ChildObjects>"));
    let start = actual.find(child).unwrap();
    unica_metadata_designer::edit::meta_edit_remove_xml_node_range(
        &mut actual,
        start..start + child.len(),
    );
    assert_eq!(actual, owner("<ChildObjects></ChildObjects>"));
}

#[test]
fn compaction_requires_the_exact_removed_element_preimage_range() {
    let child = "<Form>Last</Form>";
    let input = format!(
        "\u{feff}{}",
        owner(&format!("<ChildObjects> {child} </ChildObjects>"))
    );
    let start = input.find(child).unwrap();
    let end = start + child.len();
    for range in [
        start + 1..end,
        start..end - 1,
        start - 1..end + 1,
        start..start,
        0..input.len(),
        input.len()..input.len() + 1,
    ] {
        assert_eq!(
            unica_source_designer::registration::empty_child_objects_after_removal(&input, range),
            None
        );
    }
    assert_eq!(
        unica_source_designer::registration::empty_child_objects_after_removal(&input, start..end),
        Some(format!("\u{feff}{}", owner("<ChildObjects/>"))),
    );
}

#[test]
fn compaction_requires_a_direct_md_child_and_md_collection() {
    for (opening, child, closing) in [
        (
            "<other:ChildObjects>",
            "<Form>Last</Form>",
            "</other:ChildObjects>",
        ),
        (
            "<ChildObjects xmlns=\"urn:other\">",
            "<Form>Last</Form>",
            "</ChildObjects>",
        ),
        (
            "<ChildObjects>",
            "<other:Form>Last</other:Form>",
            "</ChildObjects>",
        ),
        (
            "<ChildObjects><Wrapper>",
            "<Form>Last</Form>",
            "</Wrapper></ChildObjects>",
        ),
        ("<Content>", "<Form>Last</Form>", "</Content>"),
    ] {
        let input = owner(&format!("{opening}{child}{closing}"));
        let start = input.find(child).unwrap();
        assert_eq!(
            unica_source_designer::registration::empty_child_objects_after_removal(
                &input,
                start..start + child.len()
            ),
            None,
            "{opening}",
        );
    }
}
