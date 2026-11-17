use super::{replace_language_value, replace_language_value_keeping_empty, DATA_CORE_NS};

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";

fn update(source: &str, lang: &str, text: &str) -> String {
    let doc = roxmltree::Document::parse(source).unwrap();
    replace_language_value(source, doc.root_element(), "", lang, text)
}

fn assert_translation(xml: &str, lang: &str, value: &str) {
    let doc = roxmltree::Document::parse(xml).expect("replacement remains namespace-well-formed");
    let item = doc
        .root_element()
        .children()
        .find(|item| {
            item.has_tag_name((DATA_CORE_NS, "item"))
                && item.children().any(|child| {
                    child.has_tag_name((DATA_CORE_NS, "lang")) && child.text() == Some(lang)
                })
        })
        .expect("language item keeps its expanded name");
    let content = item
        .children()
        .find(|child| child.has_tag_name((DATA_CORE_NS, "content")))
        .expect("content keeps its expanded name");
    assert_eq!(crate::read::direct_text(content), value);
}

#[test]
fn explicit_empty_translation_keeps_its_item_and_other_namespace_bound_bytes() {
    let english = format!(
        "<c:item xmlns:c='{DATA_CORE_NS}'><c:lang>en</c:lang><c:content>Keep</c:content></c:item>"
    );
    for body in [
        String::new(),
        "\r\n".to_string(),
        english.clone(),
        "<d:item><d:lang>ru</d:lang></d:item>".to_string(),
        "<d:item><d:lang>ru</d:lang><d:content/></d:item>".to_string(),
        "<d:item><d:lang>ru</d:lang><c:content xmlns:c='http://v8.1c.ru/8.1/data/core' a='>'>Old</c:content></d:item>".to_string(),
    ] {
        let source = format!("<v8:Title xmlns:v8='{MD}' xmlns:d='{DATA_CORE_NS}' a='>'>{body}</v8:Title>");
        let doc = roxmltree::Document::parse(&source).unwrap();
        let result = replace_language_value_keeping_empty(&source, doc.root_element(), "", "ru", "");
        assert_translation(&result, "ru", "");
        let doc = roxmltree::Document::parse(&result).unwrap();
        assert!(doc.root_element().has_tag_name((MD, "Title")));
        assert_eq!(doc.root_element().attribute("a"), Some(">"));
        assert_eq!(replace_language_value_keeping_empty(&result, doc.root_element(), "", "ru", ""), result);
        if body == english {
            assert!(result.contains(&english));
            assert_translation(&result, "en", "Keep");
        }
        if body.contains("<d:content/>") {
            assert_eq!(result, source);
        }
        if body.contains("a='>'") {
            assert!(result.contains(&format!("<c:content xmlns:c='{DATA_CORE_NS}' a='>'>")));
        }
        if body.contains("\r\n") {
            assert!(!result.replace("\r\n", "").contains('\n'));
        }
    }
    let source = format!("<v8:Title xmlns:v8='{MD}'/>");
    let doc = roxmltree::Document::parse(&source).unwrap();
    let result = replace_language_value_keeping_empty(&source, doc.root_element(), "", "ru", "");
    assert_translation(&result, "ru", "");
    assert!(roxmltree::Document::parse(&result)
        .unwrap()
        .root_element()
        .has_tag_name((MD, "Title")));
}

#[test]
fn replacing_content_preserves_local_namespace_and_attributes() {
    for header in [
        format!("c:content xmlns:c='{DATA_CORE_NS}' a='>'"),
        format!("content xmlns='{DATA_CORE_NS}' a='>'"),
    ] {
        let name = header.split_whitespace().next().unwrap();
        let source = format!("<Format xmlns='{MD}' xmlns:d='{DATA_CORE_NS}'><d:item><d:lang>ru</d:lang><{header}>Старый</{name}></d:item></Format>");
        let result = update(&source, "ru", "Новый & текст");
        assert_translation(&result, "ru", "Новый & текст");
        assert!(result.contains(&format!("<{header}>")));
    }
}

#[test]
fn appended_item_binds_a_prefix_declared_only_on_its_sibling() {
    for item in [format!("<d:item xmlns:d='{DATA_CORE_NS}'><d:lang>ru</d:lang><d:content>Первый</d:content></d:item>"), format!("<item xmlns='{DATA_CORE_NS}'><lang>ru</lang><content>Первый</content></item>")] {
        let source = format!("<Synonym xmlns='{MD}'>{item}</Synonym>");
        let result = update(&source, "en", "Second");
        assert_translation(&result, "ru", "Первый");
        assert_translation(&result, "en", "Second");
        assert!(result.contains(&item), "other translations remain byte-identical");
    }
}

#[test]
fn new_item_does_not_rebind_the_property_prefix() {
    let source = format!("<v8:Format xmlns:v8='{MD}' a='>'/>");
    let result = update(&source, "ru", "Текст");
    assert_translation(&result, "ru", "Текст");
    let doc = roxmltree::Document::parse(&result).unwrap();
    assert!(doc.root_element().has_tag_name((MD, "Format")));
    assert_eq!(doc.root_element().attribute("a"), Some(">"));
}

#[test]
fn missing_content_binds_the_language_child_local_prefix() {
    let source = format!("<Synonym xmlns='{MD}' xmlns:d='{DATA_CORE_NS}'><d:item><c:lang xmlns:c='{DATA_CORE_NS}'>ru</c:lang></d:item></Synonym>");
    assert_translation(&update(&source, "ru", "Текст"), "ru", "Текст");
}

#[test]
fn deleting_last_translation_preserves_property_header_and_unrelated_content() {
    let source = format!("<Synonym xmlns='{MD}' xmlns:d='{DATA_CORE_NS}' a='>'><d:item><d:lang>ru</d:lang><d:content>Текст</d:content></d:item><!--keep--><?keep x?><foreign xmlns='urn:x'/></Synonym>");
    let result = update(&source, "ru", "");
    assert_eq!(
        result,
        source.replace(
            "<d:item><d:lang>ru</d:lang><d:content>Текст</d:content></d:item>",
            ""
        )
    );
}

#[test]
fn empty_and_semantically_equal_values_are_exact_noops() {
    for source in [
        format!("<Synonym xmlns='{MD}' a='>'/>"),
        format!("<Synonym xmlns='{MD}'><!--keep--></Synonym>"),
    ] {
        assert_eq!(update(&source, "ru", ""), source);
    }
    let source = format!("<Synonym xmlns='{MD}' xmlns:d='{DATA_CORE_NS}'><d:item><d:lang>ru</d:lang><d:content><![CDATA[Текст & текст]]><!--keep--></d:content></d:item></Synonym>");
    assert_eq!(update(&source, "ru", "Текст & текст"), source);
}

#[test]
fn adding_first_translation_preserves_existing_siblings() {
    let source =
        format!("<Format xmlns='{MD}'><!--keep--><?keep x?><foreign xmlns='urn:x'/></Format>");
    let result = update(&source, "ru", "Текст");
    assert_translation(&result, "ru", "Текст");
    assert!(result.contains("<!--keep--><?keep x?><foreign xmlns='urn:x'/>"));
}

#[test]
fn first_translation_of_nested_property_preserves_uniform_document_eol() {
    for eol in ["\n", "\r\n", "\r"] {
        let source = format!("<Root xmlns='{MD}'>{eol}\t<Synonym/>{eol}</Root>");
        let doc = roxmltree::Document::parse(&source).unwrap();
        let property = doc.root_element().first_element_child().unwrap();
        let result = replace_language_value(&source, property, "\t", "ru", "Текст");
        assert!(result.contains(eol));
        let without_expected = result.replace(eol, "");
        assert!(
            !without_expected.contains(['\r', '\n']),
            "unexpected line ending: {result:?}"
        );
    }
}

#[test]
fn expanding_self_closed_content_keeps_local_binding_and_attributes() {
    let source = format!("<Synonym xmlns='{MD}' xmlns:d='{DATA_CORE_NS}'><d:item><d:lang>ru</d:lang><c:content xmlns:c='{DATA_CORE_NS}' a='>' /></d:item></Synonym>");
    let result = update(&source, "ru", "Текст");
    assert_translation(&result, "ru", "Текст");
    assert!(result.contains(&format!("<c:content xmlns:c='{DATA_CORE_NS}' a='>' >")));
}

mod whole_value {
    use super::super::replace_language_values;

    fn replaced(source: &str, values: &[(&str, &str)]) -> String {
        let document = roxmltree::Document::parse(source).unwrap();
        let property = document
            .descendants()
            .find(|node| node.has_tag_name("Synonym"))
            .unwrap();
        let values = values
            .iter()
            .map(|(language, text)| (language.to_string(), text.to_string()))
            .collect::<Vec<_>>();
        let replacement = replace_language_values(source, property, "\t\t\t", &values);
        let mut updated = source.to_string();
        updated.replace_range(property.range(), &replacement);
        updated
    }

    const SOURCE: &str = "<P xmlns:v8=\"http://v8.1c.ru/8.1/data/core\">\r\n\t\t\t<Synonym>\r\n\t\t\t\t<v8:item>\r\n\t\t\t\t\t<v8:lang>ru</v8:lang>\r\n\t\t\t\t\t<v8:content>Товары</v8:content>\r\n\t\t\t\t</v8:item>\r\n\t\t\t\t<v8:item>\r\n\t\t\t\t\t<v8:lang>en</v8:lang>\r\n\t\t\t\t\t<v8:content>Goods</v8:content>\r\n\t\t\t\t</v8:item>\r\n\t\t\t</Synonym>\r\n</P>";

    #[test]
    fn the_same_values_keep_every_byte() {
        assert_eq!(
            replaced(SOURCE, &[("ru", "Товары"), ("en", "Goods")]),
            SOURCE
        );
    }

    #[test]
    fn listed_languages_are_set_unlisted_removed_and_new_appended() {
        assert_eq!(
            replaced(SOURCE, &[("en", "Items"), ("kk", "Тауарлар")]),
            "<P xmlns:v8=\"http://v8.1c.ru/8.1/data/core\">\r\n\t\t\t<Synonym>\r\n\t\t\t\t<v8:item>\r\n\t\t\t\t\t<v8:lang>en</v8:lang>\r\n\t\t\t\t\t<v8:content>Items</v8:content>\r\n\t\t\t\t</v8:item>\r\n\t\t\t\t<v8:item>\r\n\t\t\t\t\t<v8:lang>kk</v8:lang>\r\n\t\t\t\t\t<v8:content>Тауарлар</v8:content>\r\n\t\t\t\t</v8:item>\r\n\t\t\t</Synonym>\r\n</P>"
        );
        assert_eq!(
            replaced(SOURCE, &[]),
            "<P xmlns:v8=\"http://v8.1c.ru/8.1/data/core\">\r\n\t\t\t<Synonym/>\r\n</P>"
        );
    }

    #[test]
    fn listed_order_is_the_written_order() {
        assert_eq!(
            replaced(SOURCE, &[("en", "Goods"), ("ru", "Товары")]),
            "<P xmlns:v8=\"http://v8.1c.ru/8.1/data/core\">\r\n\t\t\t<Synonym>\r\n\t\t\t\t<v8:item>\r\n\t\t\t\t\t<v8:lang>en</v8:lang>\r\n\t\t\t\t\t<v8:content>Goods</v8:content>\r\n\t\t\t\t</v8:item>\r\n\t\t\t\t<v8:item>\r\n\t\t\t\t\t<v8:lang>ru</v8:lang>\r\n\t\t\t\t\t<v8:content>Товары</v8:content>\r\n\t\t\t\t</v8:item>\r\n\t\t\t</Synonym>\r\n</P>"
        );
        // A new language listed before a kept one moves the kept one after it.
        let reordered = replaced(SOURCE, &[("kk", "Тауарлар"), ("ru", "Товары")]);
        let kk = reordered.find("<v8:lang>kk</v8:lang>").unwrap();
        let ru = reordered.find("<v8:lang>ru</v8:lang>").unwrap();
        assert!(kk < ru, "{reordered}");
        assert!(!reordered.contains("<v8:lang>en</v8:lang>"), "{reordered}");
    }

    #[test]
    fn an_empty_text_keeps_its_language() {
        let updated = replaced(SOURCE, &[("ru", ""), ("en", "Goods")]);
        assert!(
            updated.contains("<v8:lang>ru</v8:lang>\r\n\t\t\t\t\t<v8:content></v8:content>"),
            "{updated}"
        );
        assert!(updated.contains("<v8:content>Goods</v8:content>"));
        // Reading `ru = ''` back and writing it again changes nothing.
        assert_eq!(replaced(&updated, &[("ru", ""), ("en", "Goods")]), updated);
        let appended = replaced(SOURCE, &[("ru", "Товары"), ("en", "Goods"), ("kk", "")]);
        assert!(
            appended.contains("<v8:lang>kk</v8:lang>\r\n\t\t\t\t\t<v8:content></v8:content>"),
            "{appended}"
        );
    }

    #[test]
    fn untouched_items_keep_platform_quotes_while_a_neighbour_changes() {
        let source = SOURCE.replace("Goods", "\"Goods\" & more");
        let xml = source.replace("& more", "&amp; more");
        assert_eq!(
            replaced(&xml, &[("ru", "Товары"), ("en", "\"Goods\" & more")]),
            xml
        );
        let updated = replaced(&xml, &[("ru", "Склад"), ("en", "\"Goods\" & more")]);
        assert_eq!(updated, xml.replace("Товары", "Склад"));
    }

    #[test]
    fn an_empty_property_takes_all_listed_languages() {
        let source = "<P xmlns:v8=\"http://v8.1c.ru/8.1/data/core\">\n\t\t\t<Synonym/>\n</P>";
        assert_eq!(
            replaced(source, &[("ru", "А"), ("en", "B")]),
            "<P xmlns:v8=\"http://v8.1c.ru/8.1/data/core\">\n\t\t\t<Synonym>\n\t\t\t\t<v8:item>\n\t\t\t\t\t<v8:lang>ru</v8:lang>\n\t\t\t\t\t<v8:content>А</v8:content>\n\t\t\t\t</v8:item>\n\t\t\t\t<v8:item>\n\t\t\t\t\t<v8:lang>en</v8:lang>\n\t\t\t\t\t<v8:content>B</v8:content>\n\t\t\t\t</v8:item>\n\t\t\t</Synonym>\n</P>"
        );
    }
}
