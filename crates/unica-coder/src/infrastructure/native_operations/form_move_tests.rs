use super::{move_item, FormItemMove, FormMoveDestination, NS};
use roxmltree::{Document, Node};

fn form(body: &str) -> String {
    format!(r#"<Form xmlns="{NS}" version="2.20">{body}</Form>"#)
}

fn moved(xml: &str, name: &str, into: Option<&str>, after: Option<&str>) -> String {
    move_item(
        xml,
        &FormItemMove {
            name: name.into(),
            source_path: None,
            into: into.map_or(FormMoveDestination::Root, |name| {
                FormMoveDestination::Named(name.to_owned())
            }),
            after: after.map(str::to_owned),
        },
    )
    .unwrap_or_else(|e| panic!("move {name} into {into:?} after {after:?}: {e:?}"))
}

fn named<'a>(doc: &'a Document<'a>, name: &str) -> Node<'a, 'a> {
    doc.descendants()
        .find(|n| {
            n.attribute("name") == Some(name)
                && !n.ancestors().any(|a| a.has_tag_name((NS, "BaseForm")))
        })
        .unwrap()
}

fn children(node: Node<'_, '_>) -> Vec<String> {
    node.children()
        .find(|n| n.has_tag_name((NS, "ChildItems")))
        .into_iter()
        .flat_map(|items| items.children())
        .filter_map(|n| n.attribute("name").map(str::to_owned))
        .collect()
}

fn rejection(xml: &str, name: &str, into: Option<&str>, after: Option<&str>) -> super::MoveError {
    move_item(
        xml,
        &FormItemMove {
            name: name.into(),
            source_path: None,
            into: into.map_or(FormMoveDestination::Root, |name| {
                FormMoveDestination::Named(name.to_owned())
            }),
            after: after.map(str::to_owned),
        },
    )
    .expect_err("invalid move must be rejected")
}

fn rejected(xml: &str, name: &str, into: Option<&str>, after: Option<&str>, field: &str) {
    let error = rejection(xml, name, into, after);
    assert_eq!(error.field, field, "{error:?}");
    assert!(!error.message.is_empty());
}

fn item_xml(spec: &str, name: &str, data_path: Option<&str>, nested: bool) -> String {
    let (kind, button_type) = spec
        .split_once(':')
        .map_or((spec, None), |(kind, value)| (kind, Some(value)));
    let mut body = String::new();
    if let Some(button_type) = button_type {
        body.push_str(&format!("<Type>{button_type}</Type>"));
    }
    if let Some(data_path) = data_path {
        body.push_str(&format!("<DataPath>{data_path}</DataPath>"));
    }
    if nested {
        body.push_str("<ChildItems><ColumnGroup name=\"Nested\"/></ChildItems>");
    }
    if body.is_empty() {
        format!("<{kind} name=\"{name}\"/>")
    } else {
        format!("<{kind} name=\"{name}\">{body}</{kind}>")
    }
}

fn matrix_xml(source_kind: &str, destination_kind: &str, context: &str) -> String {
    let is_field = super::is_field(source_kind.split(':').next().unwrap());
    let unbound = context == "unboundTableColumn";
    let source = item_xml(
        source_kind,
        "MovedChild",
        (is_field && context != "ordinary" && !unbound).then_some("Rows.Value"),
        context == "differentTableNested",
    );
    let target = item_xml(destination_kind, "TargetParent", None, false);
    match context {
        "ordinary" => form(&format!("<ChildItems>{source}{target}</ChildItems>")),
        "sameTableColumn" => {
            let (table_name, target) = if destination_kind == "Table" {
                ("TargetParent", String::new())
            } else {
                ("SourceTable", target)
            };
            form(&format!(
                "<ChildItems><Table name=\"{table_name}\"><DataPath>Rows</DataPath><ChildItems>{source}{target}</ChildItems></Table></ChildItems>"
            ))
        }
        "tableColumn" => form(&format!(
            "<ChildItems><Table name=\"SourceTable\"><DataPath>Rows</DataPath><ChildItems>{source}</ChildItems></Table>{target}</ChildItems>"
        )),
        "matchingTable" | "differentTable" | "differentTableNested"
        | "unboundTableColumn" => {
            let target_path = if context == "matchingTable" { "Rows" } else { "OtherRows" };
            let (table_name, target_child) = if destination_kind == "ColumnGroup" {
                ("TargetTable", target)
            } else {
                ("TargetParent", String::new())
            };
            form(&format!(
                "<ChildItems><Table name=\"SourceTable\"><DataPath>Rows</DataPath><ChildItems>{source}</ChildItems></Table><Table name=\"{table_name}\"><DataPath>{target_path}</DataPath><ChildItems>{target_child}</ChildItems></Table></ChildItems>"
            ))
        }
        other => panic!("unknown matrix context {other}"),
    }
}

#[test]
fn compact_subtree_moves_without_losing_adjacent_siblings_or_references() {
    let subtree = r#"<UsualGroup name="Move" id="2"><Title>Title &amp; text</Title><ChildItems><InputField name="Field" id="3"><DataPath>Object.Code</DataPath><Events><Event name="OnChange">Handle</Event></Events><ContextMenu name="FieldMenu" id="4"/><ExtendedTooltip name="Tip" id="5"/></InputField></ChildItems></UsualGroup>"#;
    let xml = form(&format!(
        r#"<ChildItems><UsualGroup name="Before" id="1"/>{subtree}<UsualGroup name="Target" id="6"><ChildItems><UsualGroup name="Existing" id="7"/></ChildItems></UsualGroup><UsualGroup name="After" id="8"/></ChildItems><Commands><Command name="Ref"><Action>Items.Field</Action></Command></Commands>"#
    ));
    let result = moved(&xml, "Move", Some("Target"), None);
    let doc = Document::parse(&result).unwrap();
    assert_eq!(children(doc.root_element()), ["Before", "Target", "After"]);
    assert_eq!(children(named(&doc, "Target")), ["Existing", "Move"]);
    assert_eq!(&result[named(&doc, "Move").range()], subtree);
    assert_eq!(
        named(&doc, "FieldMenu").parent_element(),
        Some(named(&doc, "Field"))
    );
    assert_eq!(
        named(&doc, "Ref")
            .children()
            .find(Node::is_element)
            .unwrap()
            .text(),
        Some("Items.Field")
    );
}

#[test]
fn crlf_comments_and_meaningful_text_survive_platform_reindentation() {
    let subtree = "<UsualGroup name=\"Move\" id=\"1\">\r\n<Title>first\r\n  second &amp; third</Title><!--inside--></UsualGroup>";
    let xml = form(&format!(
        "<ChildItems>\r\n<!--before-->{subtree}<!--after--><UsualGroup name=\"Target\" id=\"2\"><ChildItems/></UsualGroup>\r\n</ChildItems>"
    ));
    let result = moved(&xml, "Move", Some("Target"), None);
    let doc = Document::parse(&result).unwrap();
    assert_eq!(
        &result[named(&doc, "Move").range()],
        "<UsualGroup name=\"Move\" id=\"1\">\r\n\t<Title>first\r\n  second &amp; third</Title><!--inside--></UsualGroup>"
    );
    assert!(!result.replace("\r\n", "").contains('\n'));
    let root_items = doc
        .root_element()
        .children()
        .find(|n| n.has_tag_name((NS, "ChildItems")))
        .unwrap();
    assert_eq!(
        root_items
            .children()
            .filter(Node::is_comment)
            .map(|n| n.text().unwrap())
            .collect::<Vec<_>>(),
        ["before", "after"]
    );
    assert!(named(&doc, "Move")
        .children()
        .any(|n| n.is_comment() && n.text() == Some("inside")));
}

#[test]
fn inherited_qname_namespaces_survive_conflicting_destination_bindings() {
    let xml = form(
        r#"<ChildItems><UsualGroup name="Source" xmlns:t="urn:source" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><ChildItems><UsualGroup name="Move" id="1"><Unknown xsi:type="t:Kind">t:Value</Unknown><Nested xmlns:t="urn:local"><Value>t:Local</Value></Nested></UsualGroup></ChildItems></UsualGroup><UsualGroup name="Target" xmlns:t="urn:target"><ChildItems/></UsualGroup></ChildItems>"#,
    );
    let result = moved(&xml, "Move", Some("Target"), None);
    let doc = Document::parse(&result).unwrap();
    let node = named(&doc, "Move");
    let value = node
        .children()
        .find(|n| n.has_tag_name((NS, "Unknown")))
        .unwrap();
    assert_eq!(value.lookup_namespace_uri(Some("t")), Some("urn:source"));
    assert_eq!(
        value.attribute(("http://www.w3.org/2001/XMLSchema-instance", "type")),
        Some("t:Kind")
    );
    assert_eq!(value.text(), Some("t:Value"));
    let local = node
        .descendants()
        .find(|n| n.has_tag_name((NS, "Value")))
        .unwrap();
    assert_eq!(local.lookup_namespace_uri(Some("t")), Some("urn:local"));
    assert_eq!(local.text(), Some("t:Local"));
    assert_eq!(
        named(&doc, "Target").lookup_namespace_uri(Some("t")),
        Some("urn:target")
    );
}

#[test]
fn prefixed_form_preserves_inherited_default_namespace_for_qname_values() {
    let xml = format!(
        r#"<f:Form xmlns:f="{NS}" xmlns="urn:outer"><f:ChildItems><f:UsualGroup name="Source" xmlns="urn:source"><f:ChildItems><f:UsualGroup name="Move" id="1"><f:Value>Kind</f:Value><Extension/></f:UsualGroup></f:ChildItems></f:UsualGroup><f:UsualGroup name="Target" xmlns="urn:target"/></f:ChildItems></f:Form>"#
    );
    let result = moved(&xml, "Move", Some("Target"), None);
    let doc = Document::parse(&result).unwrap();
    let node = named(&doc, "Move");
    assert_eq!(node.lookup_namespace_uri(None), Some("urn:source"));
    assert!(node
        .children()
        .any(|n| n.has_tag_name(("urn:source", "Extension"))));
    assert_eq!(children(named(&doc, "Target")), ["Move"]);
    assert_eq!(node.tag_name().namespace(), Some(NS));
}

#[test]
fn local_namespace_declarations_are_not_duplicated_even_with_attribute_decoys() {
    let xml = form(
        r#"<ChildItems><UsualGroup name="Move" xmlns:t="urn:local" description="xmlns:q='decoy' >" xmlns:q="urn:q"><Title>t:Kind</Title></UsualGroup><UsualGroup name="Target" xmlns:t="urn:other" xmlns:q="urn:other-q"/></ChildItems>"#,
    );
    let result = moved(&xml, "Move", Some("Target"), None);
    let doc = Document::parse(&result).unwrap();
    let node = named(&doc, "Move");
    assert_eq!(node.lookup_namespace_uri(Some("t")), Some("urn:local"));
    assert_eq!(node.lookup_namespace_uri(Some("q")), Some("urn:q"));
    assert_eq!(node.attribute("description"), Some("xmlns:q='decoy' >"));
}

#[test]
fn positioning_honors_into_and_after_and_defaults_to_root() {
    let xml = form(
        r#"<ChildItems><UsualGroup name="Move"/><UsualGroup name="Target"><ChildItems><UsualGroup name="A"/><UsualGroup name="B"/></ChildItems></UsualGroup><UsualGroup name="Tail"/></ChildItems>"#,
    );
    let placed = moved(&xml, "Move", Some("Target"), Some("A"));
    let doc = Document::parse(&placed).unwrap();
    assert_eq!(children(named(&doc, "Target")), ["A", "Move", "B"]);
    let root = moved(&placed, "Move", None, Some("Target"));
    let doc = Document::parse(&root).unwrap();
    assert_eq!(children(doc.root_element()), ["Target", "Move", "Tail"]);
    let end = moved(&root, "Move", None, None);
    assert_eq!(
        children(Document::parse(&end).unwrap().root_element()),
        ["Target", "Tail", "Move"]
    );
}

#[test]
fn exact_placement_is_a_byte_preserving_noop() {
    let xml = form(
        "<ChildItems>\r\n<UsualGroup name=\"A\"/>\r\n<!--keep--><UsualGroup name=\"Move\" />\r\n</ChildItems>",
    );
    assert_eq!(moved(&xml, "Move", None, None), xml);
    assert_eq!(moved(&xml, "Move", None, Some("A")), xml);
}

#[test]
fn cyclic_and_wrong_neighbor_moves_are_rejected() {
    let xml = form(
        r#"<ChildItems><UsualGroup name="Parent"><ChildItems><UsualGroup name="Child"/></ChildItems></UsualGroup><UsualGroup name="Other"/></ChildItems>"#,
    );
    for into in ["Parent", "Child"] {
        let error = rejection(&xml, "Parent", Some(into), None);
        assert_eq!(error.field, "into", "{error:?}");
        assert_eq!(
            error.message,
            "an element cannot be moved into itself or its descendant"
        );
    }
    rejected(&xml, "Parent", None, Some("Parent"), "after");
    rejected(&xml, "Child", Some("Other"), Some("Parent"), "after");
    rejected(&xml, "Other", None, Some("Child"), "after");
}

#[test]
fn moving_the_last_child_removes_empty_child_items_and_its_whitespace() {
    let xml = form(
        "\n\t<ChildItems>\n\t\t<UsualGroup name=\"Parent\">\n\t\t\t<ChildItems>\n\t\t\t\t<InputField name=\"Child\"/>\n\t\t\t</ChildItems>\n\t\t</UsualGroup>\n\t\t<UsualGroup name=\"Target\"/>\n\t</ChildItems>\n",
    );
    let result = moved(&xml, "Child", Some("Target"), None);
    let doc = Document::parse(&result).unwrap();
    let parent = named(&doc, "Parent");
    assert!(
        parent
            .children()
            .all(|node| !node.has_tag_name((NS, "ChildItems"))),
        "{result}"
    );
    assert_eq!(
        &result[parent.range()],
        r#"<UsualGroup name="Parent"></UsualGroup>"#
    );
}

#[test]
fn parent_moved_after_its_last_child_gets_destination_indentation() {
    let xml = form(
        "\n\t<ChildItems>\n\t\t<UsualGroup name=\"Parent\">\n\t\t\t<ChildItems>\n\t\t\t\t<InputField name=\"Child\"/>\n\t\t\t</ChildItems>\n\t\t</UsualGroup>\n\t\t<UsualGroup name=\"ChildTarget\"/>\n\t\t<UsualGroup name=\"ParentTarget\">\n\t\t\t<ChildItems/>\n\t\t</UsualGroup>\n\t</ChildItems>\n",
    );
    let without_child = moved(&xml, "Child", Some("ChildTarget"), None);
    let result = moved(&without_child, "Parent", Some("ParentTarget"), None);
    assert!(
        result.contains("\n\t\t\t\t<UsualGroup name=\"Parent\"></UsualGroup>\n\t\t\t</ChildItems>"),
        "{result}"
    );
}

#[test]
fn base_form_and_companions_cannot_be_independently_moved() {
    let baseline = r#"<BaseForm><ChildItems><UsualGroup name="Baseline"/><UsualGroup name="Move" id="999"/></ChildItems></BaseForm>"#;
    let xml = form(&format!(
        r#"<AutoCommandBar name="RootBar"/><ChildItems><UsualGroup name="Move" id="1"><ExtendedTooltip name="Tip"/><ContextMenu name="Menu"/></UsualGroup><UsualGroup name="Target"/></ChildItems>{baseline}"#
    ));
    for name in ["Baseline", "RootBar", "Tip", "Menu"] {
        rejected(&xml, name, Some("Target"), None, "name");
    }
    let result = moved(&xml, "Move", Some("Target"), None);
    let doc = Document::parse(&result).unwrap();
    assert_eq!(named(&doc, "Move").attribute("id"), Some("1"));
    let preserved = doc
        .descendants()
        .find(|n| n.has_tag_name((NS, "BaseForm")))
        .unwrap();
    assert_eq!(&result[preserved.range()], baseline);
}

#[test]
fn names_are_exact_and_ambiguity_is_not_resolved_by_first_match() {
    let xml = form(
        r#"<ChildItems><UsualGroup name="Move" id="1"/><UsualGroup name=" Move " id="2"/><UsualGroup name="Target"/></ChildItems>"#,
    );
    let result = moved(&xml, " Move ", Some("Target"), None);
    let doc = Document::parse(&result).unwrap();
    assert_eq!(children(doc.root_element()), ["Move", "Target"]);
    assert_eq!(children(named(&doc, "Target")), [" Move "]);
    rejected(&xml, "Move ", Some("Target"), None, "name");
    let ambiguous = form(
        r#"<ChildItems><UsualGroup name="Move"/><UsualGroup name="Move"/><UsualGroup name="Target"/></ChildItems>"#,
    );
    rejected(&ambiguous, "Move", Some("Target"), None, "name");
    rejected(&ambiguous, "Target", Some("Move"), None, "into");
    rejected(&ambiguous, "Target", None, Some("Move"), "after");
}

#[test]
fn successive_parent_child_and_repeated_moves_use_current_tree() {
    let xml = form(
        r#"<ChildItems><UsualGroup name="Group"><ChildItems><UsualGroup name="Child" id="2"/></ChildItems></UsualGroup><Pages name="Tabs"><ChildItems><Page name="First"/><Page name="Second"/></ChildItems></Pages></ChildItems>"#,
    );
    let one = moved(&xml, "Group", Some("First"), None);
    let two = moved(&one, "Child", Some("Second"), None);
    let three = moved(&two, "Child", Some("Group"), None);
    let doc = Document::parse(&three).unwrap();
    assert_eq!(children(named(&doc, "First")), ["Group"]);
    assert_eq!(children(named(&doc, "Group")), ["Child"]);
    assert!(children(named(&doc, "Second")).is_empty());
    assert_eq!(named(&doc, "Child").attribute("id"), Some("2"));
    assert_eq!(moved(&three, "Child", Some("Group"), None), three);
}

#[test]
fn moving_from_no_default_namespace_does_not_rebind_unqualified_extensions() {
    let xml = format!(
        r#"<f:Form xmlns:f="{NS}"><f:ChildItems><f:UsualGroup name="Move"><Extension value="Kind"/></f:UsualGroup><f:UsualGroup name="Target" xmlns="urn:target"/></f:ChildItems></f:Form>"#
    );
    let result = moved(&xml, "Move", Some("Target"), None);
    let doc = Document::parse(&result).unwrap();
    let extension = named(&doc, "Move")
        .children()
        .find(Node::is_element)
        .unwrap();
    assert_eq!(extension.tag_name().namespace().unwrap_or(""), "");
    assert_eq!(extension.lookup_namespace_uri(None).unwrap_or(""), "");
    assert_eq!(extension.attribute("value"), Some("Kind"));
}

#[test]
fn placement_matrix_matches_observed_platform_kinds_bindings_and_parents() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/platform_8_3_27/form_move/cases.json"
    ))
    .unwrap();
    assert_eq!(fixture["schemaVersion"], 1);
    let field_kinds = fixture["fieldKinds"].as_array().unwrap();
    let mut checked = 0;
    for case in fixture["cases"].as_array().unwrap() {
        let source_kinds: Vec<&str> = if case["sourceKinds"] == "$fieldKinds" {
            field_kinds
                .iter()
                .map(|kind| kind.as_str().unwrap())
                .collect()
        } else {
            case["sourceKinds"]
                .as_array()
                .unwrap()
                .iter()
                .map(|kind| kind.as_str().unwrap())
                .collect()
        };
        for source_kind in source_kinds {
            for destination_kind in case["destinationKinds"].as_array().unwrap() {
                let destination_kind = destination_kind.as_str().unwrap();
                let xml = matrix_xml(
                    source_kind,
                    destination_kind,
                    case["sourceContext"].as_str().unwrap(),
                );
                let doc = Document::parse(&xml).unwrap();
                let source = named(&doc, "MovedChild");
                let destination = if destination_kind == "Form" {
                    doc.root_element()
                } else {
                    named(&doc, "TargetParent")
                };
                let result = super::validate_placement(source, destination);
                let expected = case["expected"].as_str().unwrap();
                assert_eq!(
                    result.is_ok(),
                    expected == "allowed",
                    "{}: {source_kind} to {destination_kind}: {result:?}",
                    case["id"]
                );
                let request = FormItemMove {
                    name: "MovedChild".into(),
                    source_path: None,
                    into: if destination_kind == "Form" {
                        FormMoveDestination::Root
                    } else {
                        FormMoveDestination::Named("TargetParent".into())
                    },
                    after: None,
                };
                let moved_result = move_item(&xml, &request);
                assert_eq!(
                    moved_result.is_ok(),
                    expected == "allowed",
                    "{} through move_item: {source_kind} to {destination_kind}: {moved_result:?}",
                    case["id"]
                );
                if expected == "unsupported" {
                    assert!(
                        result
                            .unwrap_err()
                            .message
                            .contains("temporarily unsupported"),
                        "{}: unsupported case lost its typed reason",
                        case["id"]
                    );
                    assert!(
                        moved_result
                            .as_ref()
                            .unwrap_err()
                            .message
                            .contains("temporarily unsupported"),
                        "{}: move_item lost the typed unsupported reason",
                        case["id"]
                    );
                }
                if expected == "allowed" {
                    let before = xml[source.range()].to_string();
                    let result = moved_result.unwrap();
                    let moved_doc = Document::parse(&result).unwrap();
                    let actual = named(&moved_doc, "MovedChild");
                    assert_eq!(&result[actual.range()], before, "{}", case["id"]);
                    let actual_parent = actual.parent_element().unwrap().parent_element().unwrap();
                    assert_eq!(actual_parent.tag_name().name(), destination_kind);
                }
                checked += 1;
            }
        }
    }
    assert!(
        checked >= 100,
        "matrix unexpectedly lost coverage: {checked}"
    );
}

#[test]
fn incompatible_non_table_destination_has_no_table_binding_claim() {
    let xml = form(
        r#"<ChildItems><Pages name="Tabs"><ChildItems><Page name="Page"/></ChildItems></Pages><UsualGroup name="Target"/></ChildItems>"#,
    );
    let error = rejection(&xml, "Page", Some("Target"), None);
    assert_eq!(error.field, "into");
    assert_eq!(
        error.message,
        "Page cannot be placed in UsualGroup; this destination does not accept that element kind"
    );
}

#[test]
fn exact_noop_still_checks_the_destination_matrix() {
    let xml = form(
        r#"<ChildItems><Button name="Existing"><Type>CommandBarButton</Type></Button></ChildItems>"#,
    );
    rejected(&xml, "Existing", None, None, "into");
}

#[test]
fn compatibility_reads_complete_property_text_across_comments() {
    let xml = form(
        r#"<ChildItems><Table name="Source"><DataPath>Rows<!--comment-->A</DataPath><ChildItems><InputField name="Field"><DataPath>RowsA.Value</DataPath></InputField></ChildItems></Table><Table name="Target"><DataPath>Rows<!--comment-->B</DataPath></Table></ChildItems>"#,
    );
    rejected(&xml, "Field", Some("Target"), None, "into");
    let same = xml.replace("-->B", "-->A");
    let result = moved(&same, "Field", Some("Target"), None);
    let doc = Document::parse(&result).unwrap();
    assert_eq!(children(named(&doc, "Target")), ["Field"]);
    let xml = form(
        r#"<AutoCommandBar name="Target"/><ChildItems><Button name="Button"><Type>CommandBar<!--comment-->Button</Type></Button></ChildItems>"#,
    );
    let result = moved(&xml, "Button", Some("Target"), None);
    assert!(result.contains("<Type>CommandBar<!--comment-->Button</Type>"));
}

#[test]
fn platform_indentation_follows_destination_without_changing_leaf_text() {
    let xml = format!(
        "<Form xmlns=\"{NS}\">\r\n\t<ChildItems>\r\n\t\t<InputField name=\"Move\">\r\n\t\t\t<DataPath>Object.Value</DataPath>\r\n\t\t\t<Title><item><content>  Значение  </content></item></Title>\r\n\t\t</InputField>\r\n\t\t<UsualGroup name=\"Target\">\r\n\t\t\t<ChildItems/>\r\n\t\t</UsualGroup>\r\n\t</ChildItems>\r\n</Form>"
    );
    let nested = moved(&xml, "Move", Some("Target"), None);
    assert!(nested.contains(
        "\r\n\t\t\t\t<InputField name=\"Move\">\r\n\t\t\t\t\t<DataPath>Object.Value</DataPath>"
    ));
    assert!(nested.contains("<content>  Значение  </content>"));

    let root = moved(&nested, "Move", None, None);
    assert!(root
        .contains("\r\n\t\t<InputField name=\"Move\">\r\n\t\t\t<DataPath>Object.Value</DataPath>"));
    assert!(root.contains("<content>  Значение  </content>"));
    assert!(!root.replace("\r\n", "").contains('\n'));
}

#[test]
fn addressed_source_keeps_column_groups_in_its_complete_current_path() {
    let xml = form(
        r#"<ChildItems><Table name="Table"><DataPath>41/1:abc</DataPath><ChildItems><ColumnGroup name="Before"><ChildItems><InputField name="Field"><DataPath>42/1:def</DataPath></InputField></ChildItems></ColumnGroup><ColumnGroup name="After"/></ChildItems></Table></ChildItems>"#,
    );
    let request = |path: &[&str]| FormItemMove {
        name: "Field".into(),
        source_path: Some(path.iter().map(|part| part.to_string()).collect()),
        into: FormMoveDestination::Named("After".into()),
        after: None,
    };
    let full = request(&["Table", "Before", "Field"]);
    let moved = move_item(&xml, &full).unwrap();
    let doc = Document::parse(&moved).unwrap();
    assert_eq!(children(named(&doc, "After")), ["Field"]);
    assert!(moved.contains("<DataPath>42/1:def</DataPath>"));
    assert!(
        move_item(&moved, &full).is_err(),
        "old parent path survived the move"
    );
    assert!(
        move_item(&xml, &request(&["Table", "Field"])).is_err(),
        "a column group was silently skipped"
    );
    assert_eq!(
        move_item(&moved, &request(&["Table", "After", "Field"])).unwrap(),
        moved
    );
}

#[test]
fn uninstantiated_columns_have_an_explicit_temporary_cross_table_refusal() {
    let xml = matrix_xml("InputField", "ColumnGroup", "unboundTableColumn");
    let request = FormItemMove {
        name: "MovedChild".into(),
        source_path: None,
        into: FormMoveDestination::Named("TargetParent".into()),
        after: None,
    };
    let error = move_item(&xml, &request).unwrap_err();
    assert_eq!(error.field, "into");
    assert!(
        error.message.contains("temporarily unsupported"),
        "{error:?}"
    );
    assert!(error.message.contains("DataPath"), "{error:?}");
    let xml = form(
        r#"<ChildItems><Table name="Table"><ChildItems><ColumnGroup name="Before"><ChildItems><InputField name="Unbound"/></ChildItems></ColumnGroup><ColumnGroup name="After"/></ChildItems></Table></ChildItems>"#,
    );
    let result = moved(&xml, "Unbound", Some("After"), None);
    let doc = Document::parse(&result).unwrap();
    assert_eq!(children(named(&doc, "After")), ["Unbound"]);
}

#[test]
fn absent_and_undeclared_default_namespaces_need_no_extra_declaration() {
    let xml = format!(
        r#"<f:Form xmlns:f="{NS}"><f:ChildItems><f:UsualGroup name="Source" xmlns=""><f:ChildItems><f:UsualGroup name="Move"><Extension/></f:UsualGroup></f:ChildItems></f:UsualGroup><f:UsualGroup name="Target"/></f:ChildItems></f:Form>"#
    );
    let result = moved(&xml, "Move", Some("Target"), None);
    assert!(result.contains(r#"<f:UsualGroup name="Move"><Extension/></f:UsualGroup>"#));
    let doc = Document::parse(&result).unwrap();
    let extension = named(&doc, "Move").first_element_child().unwrap();
    assert_eq!(extension.tag_name().namespace().unwrap_or(""), "");
}
