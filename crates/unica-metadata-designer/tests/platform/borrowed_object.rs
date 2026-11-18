use super::test_support::plan_borrow_object;
use super::*;
fn parent(kind: &str) -> String {
    format!(
        r#"<MetaDataObject xmlns="{MD}" xmlns:xr="{XR}" version="2.20"><{kind} uuid="00000000-0000-4000-8000-000000000001"><Properties><Name>Orders</Name><Comment>parent comment</Comment><Synonym>parent label</Synonym><DescriptionLength>10</DescriptionLength><Server>true</Server></Properties><ChildObjects/></{kind}></MetaDataObject>"#
    )
}
fn borrow(kind: &str, overrides: &[String]) -> Vec<u8> {
    plan_borrow_object(
        parent(kind).as_bytes(),
        None,
        Some(overrides),
        fixture_uuid_v4,
    )
    .unwrap()
    .bytes
}
#[test]
fn seeded_borrow_replays_all_generated_ids_and_separates_objects() {
    fn identifiers(bytes: &[u8]) -> BTreeSet<String> {
        let text = std::str::from_utf8(bytes).unwrap();
        let doc = Document::parse(text).unwrap();
        let mut ids = BTreeSet::new();
        for node in doc.descendants().filter(|node| node.is_element()) {
            if let Some(value) = node.attribute("uuid") {
                assert!(ids.insert(value.to_owned()));
            }
            if matches!(node.tag_name().name(), "TypeId" | "ValueId" | "ThisNode") {
                assert!(ids.insert(node.text().unwrap().to_owned()));
            }
        }
        ids
    }
    for kind in ["Catalog", "ExchangePlan", "CommonModule"] {
        let source = parent(kind);
        let identity = CfeBorrowIdentity::for_plan([1; 32]);
        let first = plan_borrow_object_with_identity(
            source.as_bytes(),
            None,
            None,
            &identity,
            fixture_uuid_v4,
        )
        .unwrap();
        let repeated = plan_borrow_object_with_identity(
            source.as_bytes(),
            None,
            None,
            &identity,
            fixture_uuid_v4,
        )
        .unwrap();
        assert_eq!(
            first.bytes, repeated.bytes,
            "preview and publication must agree for {kind}"
        );
        let other = plan_borrow_object_with_identity(
            source.as_bytes(),
            None,
            None,
            &CfeBorrowIdentity::for_plan([2; 32]),
            fixture_uuid_v4,
        )
        .unwrap();
        let first_ids = identifiers(&first.bytes);
        let other_ids = identifiers(&other.bytes);
        assert!(!first_ids.is_empty());
        assert!(
            first_ids.is_disjoint(&other_ids),
            "generated identities overlap between object plans: {kind}"
        );
        if kind == "ExchangePlan" {
            assert!(std::str::from_utf8(&first.bytes)
                .unwrap()
                .contains("ThisNode"));
            assert!(
                first_ids.len() > 3,
                "generated type/value identities must be exercised"
            );
        }
        let refresh = plan_borrow_object_with_identity(
            source.as_bytes(),
            Some(&first.bytes),
            None,
            &CfeBorrowIdentity::for_plan([3; 32]),
            fixture_uuid_v4,
        )
        .unwrap();
        assert_eq!(
            identifiers(&refresh.bytes),
            first_ids,
            "refresh must retain published identity"
        );
    }
}

#[test]
fn unseeded_borrow_keeps_fresh_legacy_identity() {
    let source = parent("Catalog");
    let first = plan_borrow_object(source.as_bytes(), None, None, fixture_uuid_v4).unwrap();
    let second = plan_borrow_object(source.as_bytes(), None, None, fixture_uuid_v4).unwrap();
    let first_text = std::str::from_utf8(&first.bytes).unwrap();
    let second_text = std::str::from_utf8(&second.bytes).unwrap();
    let first_doc = Document::parse(first_text).unwrap();
    let second_doc = Document::parse(second_text).unwrap();
    assert_ne!(
        object(&first_doc).unwrap().attribute("uuid"),
        object(&second_doc).unwrap().attribute("uuid")
    );
}

#[test]
fn comment_is_extension_owned_without_a_property_state() {
    let parent = parent("CommonModule");
    let first = plan_borrow_object(parent.as_bytes(), None, None, fixture_uuid_v4).unwrap();
    let text = String::from_utf8(first.bytes).unwrap();
    let doc = Document::parse(&text).unwrap();
    assert_eq!(
        doc.descendants()
            .find(|n| n.has_tag_name((MD, "Comment")))
            .unwrap()
            .text(),
        None
    );
    let local = text.replace("<Comment/>", "<Comment>local comment</Comment>");
    let changed = parent
        .replace("parent comment", "changed parent comment")
        .replace("<Server>true</Server>", "<Server>false</Server>");
    let next = plan_borrow_object(
        changed.as_bytes(),
        Some(local.as_bytes()),
        None,
        fixture_uuid_v4,
    )
    .unwrap();
    let next = String::from_utf8(next.bytes).unwrap();
    assert!(next.contains("<Comment>local comment</Comment>"));
    assert!(!next.contains("changed parent comment"));
}

/// rule: INV.SOURCE.BORROWED-OBJECT-IDENTITY
#[test]
fn refresh_updates_transferred_properties_preserving_local_xml_and_noop_bytes() {
    let old = borrow("CommonModule", &[]);
    let old = String::from_utf8(old)
        .unwrap()
        .replace("<Comment/>", "<Comment>local comment</Comment>")
        .replace(
            "</Properties>",
            "<Future xmlns=\"urn:local\" a=\"keep\"/></Properties>",
        );
    let changed = parent("CommonModule")
        .replace("<Server>true</Server>", "<Server>false</Server>")
        .replace("parent comment", "changed parent");
    let next = plan_borrow_object(
        changed.as_bytes(),
        Some(old.as_bytes()),
        None,
        fixture_uuid_v4,
    )
    .unwrap();
    let next_text = String::from_utf8(next.bytes.clone()).unwrap();
    assert!(next_text.contains("local comment"));
    assert!(next_text.contains("<Future xmlns=\"urn:local\" a=\"keep\"/>"));
    let old_doc = Document::parse(&old).unwrap();
    let new_doc = Document::parse(&next_text).unwrap();
    assert_eq!(
        old_doc
            .root_element()
            .first_element_child()
            .unwrap()
            .attribute("uuid"),
        new_doc
            .root_element()
            .first_element_child()
            .unwrap()
            .attribute("uuid")
    );
    assert_eq!(
        new_doc
            .descendants()
            .find(|n| n.has_tag_name((MD, "Server")))
            .unwrap()
            .text(),
        Some("false")
    );
    assert_eq!(
        plan_borrow_object(changed.as_bytes(), Some(&next.bytes), None, fixture_uuid_v4)
            .unwrap()
            .bytes,
        next.bytes
    );
}

#[test]
fn catalog_refresh_keeps_platform_identity_children_and_overridden_property_state() {
    let parent = include_str!(
        "../../../../tests/fixtures/formats/platform_xml/2.20/cfe_borrow/parent-catalog.xml"
    );
    let old = include_str!(
        "../../../../tests/fixtures/formats/platform_xml/2.20/cfe_borrow/extension-catalog.xml"
    );
    let changed = parent.replace(
        "<DescriptionLength>10</DescriptionLength>",
        "<DescriptionLength>20</DescriptionLength>",
    );
    assert_ne!(parent, changed);
    let new = plan_borrow_object(
        changed.as_bytes(),
        Some(old.as_bytes()),
        Some(&["DescriptionLength".into()]),
        fixture_uuid_v4,
    )
    .unwrap();
    assert_eq!(new.bytes, old.as_bytes());
    assert_eq!(new.parent_uuid, "1d6b8425-360c-4ab1-9bab-cc9a3b590bb2");
    let first = plan_borrow_object(
        parent.as_bytes(),
        None,
        Some(&["DescriptionLength".into()]),
        fixture_uuid_v4,
    )
    .unwrap();
    let text = std::str::from_utf8(&first.bytes).unwrap();
    let doc = Document::parse(text).unwrap();
    let obj = doc.root_element().first_element_child().unwrap();
    assert_ne!(obj.attribute("uuid"), Some(first.parent_uuid.as_str()));
    assert_eq!(
        doc.descendants()
            .find(|n| n.has_tag_name((XR, "Property")))
            .unwrap()
            .text(),
        Some("DescriptionLength")
    );
    assert_eq!(
        doc.descendants()
            .find(|n| n.has_tag_name((MD, "DescriptionLength")))
            .unwrap()
            .text(),
        Some("10")
    );
}

/// rule: INV.SOURCE.BORROWED-OBJECT-REFRESH
#[test]
fn catalog_refresh_updates_platform_control_properties_and_keeps_override() {
    let parent = include_str!(
        "../../../../tests/fixtures/formats/platform_xml/2.20/cfe_borrow/parent-catalog.xml"
    );
    let old = include_str!(
        "../../../../tests/fixtures/formats/platform_xml/2.20/cfe_borrow/extension-catalog.xml"
    );
    let changed = parent
        .replace("<CodeLength>3</CodeLength>", "<CodeLength>5</CodeLength>")
        .replace(
            "<DescriptionLength>10</DescriptionLength>",
            "<DescriptionLength>20</DescriptionLength>",
        )
        .replace(
            "<Hierarchical>false</Hierarchical>",
            "<Hierarchical>true</Hierarchical>",
        );
    assert_ne!(parent, changed);
    let new = plan_borrow_object(
        changed.as_bytes(),
        Some(old.as_bytes()),
        None,
        fixture_uuid_v4,
    )
    .unwrap();
    let text = std::str::from_utf8(&new.bytes)
        .unwrap()
        .trim_start_matches('\u{feff}');
    let doc = Document::parse(text).unwrap();
    let old_doc = Document::parse(old.trim_start_matches('\u{feff}')).unwrap();
    for (prop, value) in [
        ("CodeLength", "5"),
        ("Hierarchical", "true"),
        ("DescriptionLength", "50"),
    ] {
        assert_eq!(
            doc.descendants()
                .find(|n| n.has_tag_name((MD, prop)))
                .unwrap()
                .text(),
            Some(value)
        );
    }
    // A property taken from the parent reads as the platform writes it, not
    // with every namespace of the parent on it (#2996).
    let properties = doc
        .descendants()
        .find(|n| n.has_tag_name((MD, "Properties")))
        .unwrap();
    assert!(
        !text[properties.range()].contains("xmlns"),
        "{}",
        &text[properties.range()]
    );
    for prop in ["InternalInfo", "ChildObjects", "Synonym", "Comment"] {
        let before = old_doc
            .descendants()
            .find(|n| n.has_tag_name((MD, prop)))
            .unwrap();
        let after = doc
            .descendants()
            .find(|n| n.has_tag_name((MD, prop)))
            .unwrap();
        assert_eq!(
            &old.trim_start_matches('\u{feff}')[before.range()],
            &text[after.range()]
        );
    }
    assert_eq!(
        plan_borrow_object(changed.as_bytes(), Some(&new.bytes), None, fixture_uuid_v4)
            .unwrap()
            .bytes,
        new.bytes
    );
}

/// rule: INV.SOURCE.BORROWED-OBJECT-IDENTITY
#[test]
fn exchange_plan_identity_is_fresh_then_stable_after_untransferred_parent_changes() {
    let parent_uuid = "00000000-0000-4000-8000-000000000001";
    let parent_node = "00000000-0000-4000-8000-000000000002";
    let categories = ["Object", "Ref", "Selection", "List", "Manager"];
    let generated = categories.iter().enumerate().map(|(index, category)| {
            format!("<xr:GeneratedType name=\"ExchangePlan{category}.Orders\" category=\"{category}\"><xr:TypeId>00000000-0000-4000-8000-{:012x}</xr:TypeId><xr:ValueId>00000000-0000-4000-8000-{:012x}</xr:ValueId></xr:GeneratedType>", 100 + index * 2, 101 + index * 2)
        }).collect::<String>();
    let parent = format!("<MetaDataObject xmlns=\"{MD}\" xmlns:xr=\"{XR}\" version=\"2.20\"><ExchangePlan uuid=\"{parent_uuid}\"><InternalInfo><xr:ThisNode>{parent_node}</xr:ThisNode>{generated}</InternalInfo><Properties><Name>Orders</Name><Comment>before</Comment><CodeLength>9</CodeLength></Properties><ChildObjects/></ExchangePlan></MetaDataObject>");
    let identity = |text: &str| {
        let doc = Document::parse(text).unwrap();
        let object = doc.root_element().first_element_child().unwrap();
        let internal = object
            .children()
            .find(|n| n.has_tag_name((MD, "InternalInfo")))
            .unwrap();
        let node = internal
            .children()
            .find(|n| n.has_tag_name((XR, "ThisNode")))
            .unwrap()
            .text()
            .unwrap()
            .to_string();
        let pairs = internal
            .children()
            .filter(|n| n.has_tag_name((XR, "GeneratedType")))
            .map(|generated| {
                let type_id = generated
                    .children()
                    .find(|n| n.has_tag_name((XR, "TypeId")))
                    .unwrap()
                    .text()
                    .unwrap()
                    .to_string();
                let value_id = generated
                    .children()
                    .find(|n| n.has_tag_name((XR, "ValueId")))
                    .unwrap()
                    .text()
                    .unwrap()
                    .to_string();
                (
                    generated.attribute("name").unwrap().to_string(),
                    generated.attribute("category").unwrap().to_string(),
                    type_id,
                    value_id,
                )
            })
            .collect::<Vec<_>>();
        (
            object.attribute("uuid").unwrap().to_string(),
            node,
            pairs,
            text[internal.range()].to_string(),
        )
    };
    let parent_identity = identity(&parent);
    let mut parent_ids = BTreeSet::from([parent_identity.0.clone(), parent_identity.1.clone()]);
    for (_, _, type_id, value_id) in &parent_identity.2 {
        parent_ids.extend([type_id.clone(), value_id.clone()]);
    }
    let first = plan_borrow_object(parent.as_bytes(), None, None, fixture_uuid_v4).unwrap();
    let first_identity = identity(std::str::from_utf8(&first.bytes).unwrap());
    assert_eq!(first_identity.2.len(), categories.len());
    assert_eq!(
        first_identity
            .2
            .iter()
            .map(|(_, category, _, _)| category.as_str())
            .collect::<BTreeSet<_>>(),
        categories.into_iter().collect::<BTreeSet<_>>()
    );
    let mut own_ids = BTreeSet::from([first_identity.0.clone(), first_identity.1.clone()]);
    for (name, category, type_id, value_id) in &first_identity.2 {
        assert!(categories.contains(&category.as_str()));
        assert_eq!(name, &format!("ExchangePlan{category}.Orders"));
        assert!(own_ids.insert(type_id.clone()));
        assert!(own_ids.insert(value_id.clone()));
    }
    assert_eq!(own_ids.len(), 12);
    assert!(parent_ids.is_disjoint(&own_ids));
    for value in &own_ids {
        assert!(!uuid::Uuid::parse_str(value).unwrap().is_nil());
    }
    // This profile does not transfer Comment or CodeLength. Their changes
    // exercise identity stability, while the Catalog test proves refresh of
    // transferred fields together with GeneratedType preservation.
    let changed = parent
        .replace("<Comment>before</Comment>", "<Comment>after</Comment>")
        .replace("<CodeLength>9</CodeLength>", "<CodeLength>12</CodeLength>");
    assert_ne!(changed, parent);
    let repeated = plan_borrow_object(
        changed.as_bytes(),
        Some(&first.bytes),
        None,
        fixture_uuid_v4,
    )
    .unwrap();
    assert_eq!(
        identity(std::str::from_utf8(&repeated.bytes).unwrap()),
        first_identity
    );
    assert_eq!(repeated.bytes, first.bytes);
}

/// rule: INV.SOURCE.BORROWED-OBJECT-IDENTITY
#[test]
fn extension_identity_damage_is_refused_instead_of_repaired() {
    let original = String::from_utf8(borrow("ExchangePlan", &[])).unwrap();
    let doc = Document::parse(&original).unwrap();
    for tag in ["ThisNode", "TypeId", "ValueId"] {
        let node = doc
            .descendants()
            .find(|n| n.has_tag_name((XR, tag)))
            .unwrap();
        let mut damaged = original.clone();
        damaged.replace_range(node.range(), "");
        assert!(
            plan_borrow_object(
                parent("ExchangePlan").as_bytes(),
                Some(damaged.as_bytes()),
                None,
                fixture_uuid_v4
            )
            .is_err(),
            "{tag}"
        );
    }
    let wrapper = doc
        .root_element()
        .first_element_child()
        .unwrap()
        .attribute("uuid")
        .unwrap();
    for invalid in [
        "",
        "not-a-uuid",
        "00000000-0000-0000-0000-000000000000",
        "00000000-0000-4000-8000-000000000001",
    ] {
        let damaged = original.replace(wrapper, invalid);
        assert!(plan_borrow_object(
            parent("ExchangePlan").as_bytes(),
            Some(damaged.as_bytes()),
            None,
            fixture_uuid_v4
        )
        .is_err());
    }
    assert_eq!(
        plan_borrow_object(
            parent("ExchangePlan").as_bytes(),
            Some(original.as_bytes()),
            None,
            fixture_uuid_v4
        )
        .unwrap()
        .bytes,
        original.as_bytes()
    );
}

#[test]
fn known_state_variants_and_module_states_do_not_erase_local_overrides() {
    let parent = include_str!(
        "../../../../tests/fixtures/formats/platform_xml/2.20/cfe_borrow/parent-catalog.xml"
    );
    let original = include_str!(
        "../../../../tests/fixtures/formats/platform_xml/2.20/cfe_borrow/extension-catalog.xml"
    );
    for state in ["Extended", "Notify", "MultiState"] {
        let existing = original.replace("<xr:State>Extended</xr:State>", &format!("<xr:State>{state}</xr:State>")).replace("</InternalInfo>", "<xr:PropertyState><xr:Property>ObjectModule</xr:Property><xr:State>Extended</xr:State></xr:PropertyState></InternalInfo>");
        let next = plan_borrow_object(
            parent.as_bytes(),
            Some(existing.as_bytes()),
            Some(&["DescriptionLength".into()]),
            fixture_uuid_v4,
        )
        .unwrap();
        assert_eq!(next.bytes, existing.as_bytes());
    }
    for local in ["Comment", "Synonym"] {
        assert!(plan_borrow_object(
            parent.as_bytes(),
            None,
            Some(&[local.into()]),
            fixture_uuid_v4
        )
        .unwrap_err()
        .to_string()
        .contains("extension-owned"));
    }
}

#[test]
fn common_module_server_override_uses_platform_notify_and_survives_refresh() {
    let parent = include_str!(
        "../../../../tests/fixtures/formats/platform_xml/2.20/cfe_borrow/parent-common-module.xml"
    )
    .replace("<Global>false</Global>", "<Global>true</Global>");
    let existing = include_str!(
        "../../../../tests/fixtures/formats/platform_xml/2.20/cfe_borrow/extension-common-module.xml"
    );
    let next = plan_borrow_object(
        parent.as_bytes(),
        Some(existing.as_bytes()),
        Some(&["Server".into()]),
        fixture_uuid_v4,
    )
    .unwrap();
    let next_text = std::str::from_utf8(&next.bytes)
        .unwrap()
        .trim_start_matches('\u{feff}');
    let next_doc = Document::parse(next_text).unwrap();
    assert_eq!(
        next_doc
            .descendants()
            .find(|n| n.has_tag_name((MD, "Server")))
            .unwrap()
            .text(),
        Some("false")
    );
    assert_eq!(
        next_doc
            .descendants()
            .find(|n| n.has_tag_name((MD, "Global")))
            .unwrap()
            .text(),
        Some("true")
    );
    assert_eq!(
        next_doc
            .descendants()
            .find(|n| n.has_tag_name((XR, "State")))
            .unwrap()
            .text(),
        Some("Notify")
    );
    let first = plan_borrow_object(
        parent.as_bytes(),
        None,
        Some(&["Server".into()]),
        fixture_uuid_v4,
    )
    .unwrap();
    let first_text = std::str::from_utf8(&first.bytes).unwrap();
    let doc = Document::parse(first_text).unwrap();
    assert_eq!(
        doc.descendants()
            .find(|n| n.has_tag_name((XR, "State")))
            .unwrap()
            .text(),
        Some("Notify")
    );
    assert_eq!(
        doc.descendants()
            .find(|n| n.has_tag_name((MD, "Server")))
            .unwrap()
            .text(),
        Some("true")
    );
}

#[test]
fn changed_qname_binding_refreshes_type_even_when_its_text_is_unchanged() {
    let parent = parent("DefinedType").replace("version=\"2.20\"", "xmlns:p=\"urn:parent:one\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" version=\"2.20\"").replace("</Properties>", "<Type><v8:Type>p:Example</v8:Type></Type></Properties>");
    let first = plan_borrow_object(parent.as_bytes(), None, None, fixture_uuid_v4).unwrap();
    let first_text = std::str::from_utf8(&first.bytes).unwrap();
    let first_doc = Document::parse(first_text).unwrap();
    let first_type = first_doc
        .descendants()
        .find(|n| n.has_tag_name(("http://v8.1c.ru/8.1/data/core", "Type")))
        .unwrap();
    assert_eq!(
        first_type.lookup_namespace_uri(Some("p")),
        Some("urn:parent:one")
    );
    let changed = parent.replace("urn:parent:one", "urn:parent:two");
    let next = plan_borrow_object(
        changed.as_bytes(),
        Some(&first.bytes),
        None,
        fixture_uuid_v4,
    )
    .unwrap();
    let text = std::str::from_utf8(&next.bytes).unwrap();
    let doc = Document::parse(text).unwrap();
    let ty = doc
        .descendants()
        .find(|n| n.has_tag_name(("http://v8.1c.ru/8.1/data/core", "Type")))
        .unwrap();
    assert_eq!(ty.text(), Some("p:Example"));
    assert_eq!(ty.lookup_namespace_uri(Some("p")), Some("urn:parent:two"));
    assert_eq!(
        plan_borrow_object(changed.as_bytes(), Some(&next.bytes), None, fixture_uuid_v4)
            .unwrap()
            .bytes,
        next.bytes
    );
}

#[test]
fn refuses_damaged_identity_own_object_changed_parent_and_changed_overrides() {
    let original = String::from_utf8(borrow("Catalog", &[])).unwrap();
    let doc = Document::parse(&original).unwrap();
    let value_id = doc
        .descendants()
        .find(|n| n.has_tag_name((XR, "ValueId")))
        .unwrap();
    let mut damaged = original.clone();
    damaged.replace_range(value_id.range(), "");
    for bad in [
        damaged,
        original.replace("Adopted", "Own"),
        original.replace(
            "00000000-0000-4000-8000-000000000001",
            "00000000-0000-4000-8000-000000000002",
        ),
    ] {
        assert!(plan_borrow_object(
            parent("Catalog").as_bytes(),
            Some(bad.as_bytes()),
            None,
            fixture_uuid_v4
        )
        .is_err());
    }
    assert!(plan_borrow_object(
        parent("Catalog").as_bytes(),
        Some(original.as_bytes()),
        Some(&["DescriptionLength".into()]),
        fixture_uuid_v4
    )
    .is_err());
}

fn fixture_uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[test]
fn unseeded_test_planner_uses_the_explicit_entropy_supplier() {
    fn supplied_uuid() -> String {
        "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_string()
    }
    let source = parent("CommonModule");
    let planned = plan_borrow_object(source.as_bytes(), None, None, supplied_uuid).unwrap();
    let text = std::str::from_utf8(&planned.bytes).unwrap();
    let document = Document::parse(text).unwrap();
    assert_eq!(
        object(&document).unwrap().attribute("uuid"),
        Some("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"),
        "the required supplier must produce the borrowed object's actual identity"
    );
}

/// A descriptor borrowed before its kind was written in full gains what the
/// platform writes on the next borrowing, each on a line of its own: the
/// template type of a common template from the parent, and the empty content
/// of a subsystem, not the parent's content (#2996).
/// rule: INV.SOURCE.WRITABLE-PROFILE
#[test]
fn repeated_borrowing_completes_a_descriptor_borrowed_in_part() {
    for (kind, own, missing) in [
        (
            "CommonTemplate",
            "<TemplateType>SpreadsheetDocument</TemplateType>",
            "\t\t\t<TemplateType>SpreadsheetDocument</TemplateType>\n",
        ),
        (
            "Subsystem",
            "<Content><xr:Item xsi:type=\"xr:MDObjectRef\">Catalog.Orders</xr:Item></Content>",
            "\t\t\t<Content/>\n",
        ),
    ] {
        let parent = format!(
            r#"<MetaDataObject xmlns="{MD}" xmlns:xr="{XR}" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20"><{kind} uuid="00000000-0000-4000-8000-000000000001"><Properties><Name>Orders</Name><Comment/>{own}</Properties><ChildObjects/></{kind}></MetaDataObject>"#
        );
        let full = plan_borrow_object(parent.as_bytes(), None, None, fixture_uuid_v4)
            .unwrap()
            .bytes;
        let full = String::from_utf8(full).unwrap();
        assert!(full.contains(missing), "{kind}: {full}");
        let partial = full.replacen(missing, "", 1);
        let completed = plan_borrow_object(
            parent.as_bytes(),
            Some(partial.as_bytes()),
            None,
            fixture_uuid_v4,
        )
        .unwrap()
        .bytes;
        assert_eq!(String::from_utf8(completed).unwrap(), full, "{kind}");
    }
}

fn seeded_borrow(parent: &str, existing: Option<&str>) -> Result<String, BorrowObjectError> {
    plan_borrow_object_with_identity(
        parent.as_bytes(),
        existing.map(str::as_bytes),
        None,
        &CfeBorrowIdentity::for_plan([7; 32]),
        fixture_uuid_v4,
    )
    .map(|plan| String::from_utf8(plan.bytes).unwrap())
}

fn small_parent(kind: &str, version: &str, own: &str) -> String {
    format!(
        r#"<MetaDataObject xmlns="{MD}" xmlns:xr="{XR}" version="{version}"><{kind} uuid="00000000-0000-4000-8000-000000000001"><Properties><Name>Orders</Name><Comment/>{own}</Properties><ChildObjects/></{kind}></MetaDataObject>"#
    )
}

/// `text` without the lines from the one holding `from` through the next one
/// holding `to`.
fn without_lines(text: &str, from: &str, to: &str) -> String {
    let lines = text.split('\n').collect::<Vec<_>>();
    let start = lines.iter().position(|line| line.contains(from)).unwrap();
    let end = start
        + lines[start..]
            .iter()
            .position(|line| line.contains(to))
            .unwrap();
    [&lines[..start], &lines[end + 1..]].concat().join("\n")
}

/// A descriptor an earlier Unica borrowed without what the platform needs in
/// `InternalInfo` gains it on the next borrowing, keeping the identities it
/// has: the generated types of a sequence and a WS reference, the extended
/// property of a common form, a role and a subsystem (#2996).
/// rule: INV.SOURCE.BORROWED-OBJECT-IDENTITY
#[test]
fn repeated_borrowing_completes_the_internal_info_of_an_earlier_borrowing() {
    let sequence = small_parent("Sequence", "2.20", "");
    let full = seeded_borrow(&sequence, None).unwrap();
    let legacy = without_lines(
        &without_lines(&full, "\"SequenceRecord.Orders\"", "</xr:GeneratedType>"),
        "\"SequenceManager.Orders\"",
        "</xr:GeneratedType>",
    );
    assert!(!legacy.contains("SequenceManager"), "{legacy}");
    assert_eq!(seeded_borrow(&sequence, Some(&legacy)).unwrap(), full);
    // A sequence that lost a type the earlier profile had is damaged, not
    // borrowed in part, and is still refused.
    let damaged = without_lines(&full, "\"SequenceRecordSet.Orders\"", "</xr:GeneratedType>");
    assert!(seeded_borrow(&sequence, Some(&damaged))
        .unwrap_err()
        .to_string()
        .contains("missing GeneratedType SequenceRecordSet.Orders"));
    for kind in ["WSReference", "CommonForm", "Role", "Subsystem"] {
        let parent = small_parent(kind, "2.20", "");
        let full = seeded_borrow(&parent, None).unwrap();
        let legacy = without_lines(&full, "<InternalInfo>", "</InternalInfo>").replacen(
            "\t\t<Properties>",
            "\t\t<InternalInfo/>\n\t\t<Properties>",
            1,
        );
        assert!(legacy.contains("<InternalInfo/>"), "{kind}: {legacy}");
        assert_eq!(
            seeded_borrow(&parent, Some(&legacy)).unwrap(),
            full,
            "{kind}"
        );
    }
}

/// A common template of format 2.22 borrowed before its kind was written in
/// full gains its template type and its action on an extension change error
/// in the platform's order; a descriptor whose properties share one line
/// stays well formed (#2996).
/// rule: INV.SOURCE.WRITABLE-PROFILE
#[test]
fn repeated_borrowing_completes_a_common_template_of_format_2_22() {
    let parent = small_parent(
        "CommonTemplate",
        "2.22",
        "<TemplateType>SpreadsheetDocument</TemplateType>",
    );
    let full = seeded_borrow(&parent, None).unwrap();
    let missing = [
        "\t\t\t<TemplateType>SpreadsheetDocument</TemplateType>\n",
        "\t\t\t<ActionOnConfigurationExtensionChangeError>Warning</ActionOnConfigurationExtensionChangeError>\n",
    ];
    let legacy = missing
        .iter()
        .fold(full.clone(), |text, line| text.replacen(line, "", 1));
    assert_ne!(legacy, full);
    assert_eq!(seeded_borrow(&parent, Some(&legacy)).unwrap(), full);
    let one_line = {
        let start = legacy.find("\t\t\t<ObjectBelonging>").unwrap();
        let end = legacy.find("\t\t</Properties>").unwrap();
        let joined = legacy[start..end].replace(['\n', '\t'], "");
        format!("{}\t\t\t{joined}\n{}", &legacy[..start], &legacy[end..])
    };
    let completed = seeded_borrow(&parent, Some(&one_line)).unwrap();
    let doc = Document::parse(&completed).unwrap();
    for (prop, count) in [("Name", 1), ("TemplateType", 1), ("Comment", 1)] {
        assert_eq!(
            doc.descendants()
                .filter(|n| n.has_tag_name((MD, prop)))
                .count(),
            count,
            "{prop}: {completed}"
        );
    }
}
