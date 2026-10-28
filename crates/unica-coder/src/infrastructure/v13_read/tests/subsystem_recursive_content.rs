use super::*;

#[test]
fn recursive_content_deduplicates_sources_and_cursors_retain_child_snapshot() {
    let fixture = RealReaderFixture::new();
    let root = "main:Subsystem.Sales";
    let child = format!("{root}.Subsystem.Orders");
    let sibling = format!("{root}.Subsystem.Accounting");
    write(
        &fixture.source.join("Subsystems/Sales.xml"),
        &subsystem_descriptor("Sales", &["Catalog.Items"], &["Orders", "Accounting"]),
    );
    let child_path = fixture
        .source
        .join("Subsystems/Sales/Subsystems/Orders.xml");
    write(
        &child_path,
        &subsystem_descriptor("Orders", &["Catalog.Items", "Document.Order"], &["Returns"]),
    );
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Subsystems/Accounting.xml"),
        &subsystem_descriptor("Accounting", &["Catalog.Items"], &[]),
    );
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Subsystems/Orders/Subsystems/Returns.xml"),
        &subsystem_descriptor("Returns", &["Constant.MainCurrency"], &[]),
    );
    write(
        &fixture.source.join("Subsystems/Sales/Subsystems/Ghost.xml"),
        "invalid unregistered XML",
    );
    let service = fixture.view_service();
    let at = format!("{root}.Relation");
    let all = view_items(&service.view(ViewRequest::new(&at).unwrap()));
    assert_eq!(all.len(), 3);
    assert_eq!(all[0]["subsystems"], json!([root, child, sibling]));
    assert_eq!(all[1]["at"], "main:Document.Order");
    assert_eq!(all[2]["at"], "main:Constant.MainCurrency");
    let first = service.view(ViewRequest::new(&at).unwrap().with_limit(1).unwrap());
    let cursor = first.cursor.clone().expect("recursive page cursor");
    // Root descriptor remains byte-for-byte unchanged; only a child changes.
    write(
        &child_path,
        &subsystem_descriptor(
            "Orders",
            &["Catalog.Items", "Report.ParityReport"],
            &["Returns"],
        ),
    );
    let fresh = view_items(&service.view(ViewRequest::new(&at).unwrap()));
    assert_eq!(fresh[1]["at"], "main:Report.ParityReport");
    let request = || {
        ViewRequest::new(&at)
            .unwrap()
            .with_limit(1)
            .unwrap()
            .with_cursor(cursor.clone())
    };
    let old = view_items(&service.view(request()));
    assert_eq!(old, vec![all[1].clone()]);
    assert_eq!(view_items(&service.view(request())), old);
    // Nested reads exclude the selected node's siblings and ancestors.
    let nested = view_items(&service.view(ViewRequest::new(&format!("{child}.Relation")).unwrap()));
    assert_eq!(nested.len(), 3);
    assert_eq!(nested[0]["subsystems"], json!([child]));
    assert!(!nested.iter().any(|row| row["subsystems"]
        .as_array()
        .unwrap()
        .contains(&json!(sibling))));
    // Content references to other subsystems do not traverse their content.
    write(
        &child_path,
        &subsystem_descriptor("Orders", &["Subsystem.Sales.Subsystem.Accounting"], &[]),
    );
    let nested = view_items(&service.view(ViewRequest::new(&format!("{child}.Relation")).unwrap()));
    assert_eq!(nested.len(), 1);
    assert_eq!(nested[0]["at"], sibling);
}

#[test]
fn subtree_content_ignores_unrelated_descriptors_and_refuses_a_broken_registered_child() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture.source.join("Subsystems/Sales.xml"),
        &subsystem_descriptor("Sales", &[], &["Orders", "Accounting"]),
    );
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Subsystems/Orders.xml"),
        &subsystem_descriptor(
            "Orders",
            &["Document.Order", "0:78b4152e-0000-0000-0000-000000000000"],
            &[],
        ),
    );
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Subsystems/Accounting.xml"),
        "bad XML",
    );
    let service = fixture.view_service();
    let result =
        service.view(ViewRequest::new("main:Subsystem.Sales.Subsystem.Orders.Relation").unwrap());
    assert_eq!(view_items(&result).len(), 1);
    let limits = result.data.as_ref().unwrap()["limits"].to_string();
    assert!(limits.contains("0:78b4152e"));
    assert!(limits.contains("main:Subsystem.Sales.Subsystem.Orders"));
    let root = service.view(ViewRequest::new("main:Subsystem.Sales.Relation").unwrap());
    assert!(
        !root.ok,
        "broken registered child must not produce a complete partial set"
    );
    assert_eq!(root.diagnostics[0]["detailCode"], "source_unreadable");
}

#[test]
fn uncached_subsystem_routes_keep_registered_owner_proof() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture.source.join("Subsystems/Ghost.xml"),
        &subsystem_descriptor("Ghost", &[], &["Child"]),
    );
    let service = fixture.view_service();
    for suffix in ["", ".Relation", ".Subsystem"] {
        let result =
            service.view(ViewRequest::new(&format!("main:Subsystem.Ghost{suffix}")).unwrap());
        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["code"], "not_found");
    }
    write(
        &fixture.source.join("Subsystems/Sales.xml"),
        &subsystem_descriptor("Other", &[], &["Child"]),
    );
    for suffix in ["", ".Relation", ".Subsystem"] {
        let result =
            service.view(ViewRequest::new(&format!("main:Subsystem.Sales{suffix}")).unwrap());
        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["detailCode"], "source_unreadable");
    }
}

#[test]
fn unreadable_descendants_keep_plain_properties_without_a_false_composition_count() {
    for own_content in [&[][..], &["Catalog.Items"][..]] {
        for (nested, broken) in [(false, ""), (false, "not XML"), (true, "not XML")] {
            let fixture = RealReaderFixture::new();
            write(
                &fixture.source.join("Subsystems/Sales.xml"),
                &subsystem_descriptor("Sales", own_content, &["Orders"]),
            );
            let (at, path) = if nested {
                write(
                    &fixture
                        .source
                        .join("Subsystems/Sales/Subsystems/Orders.xml"),
                    &subsystem_descriptor("Orders", own_content, &["Returns"]),
                );
                (
                    "main:Subsystem.Sales.Subsystem.Orders",
                    "Subsystems/Sales/Subsystems/Orders/Subsystems/Returns.xml",
                )
            } else {
                (
                    "main:Subsystem.Sales",
                    "Subsystems/Sales/Subsystems/Orders.xml",
                )
            };
            if !broken.is_empty() {
                write(&fixture.source.join(path), broken);
            }
            let service = fixture.view_service();
            let node = service.view(ViewRequest::new(at).unwrap());
            assert!(
                node.ok,
                "own descriptor remains readable: {:?}",
                node.diagnostics
            );
            let data = node.data.unwrap();
            assert_eq!(data["kind"], "Subsystem");
            assert_eq!(data["title"], if nested { "Orders" } else { "Sales" });
            let relation = format!("{at}.Relation");
            assert!(!data["branches"]
                .as_array()
                .unwrap()
                .iter()
                .any(|branch| branch["at"] == relation));
            assert!(data["limits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|limit| limit.as_str().unwrap().contains(&relation)));
            let result = service.view(ViewRequest::new(&relation).unwrap());
            assert!(!result.ok);
            assert_eq!(result.diagnostics[0]["detailCode"], "source_unreadable");
        }
    }
}
