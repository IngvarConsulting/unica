use super::*;

pub(super) fn native_document_relations(
    native: &mut NativeProjections<'_, '_>,
    source: &Path,
    registrations: &[(String, String)],
) -> BTreeSet<String> {
    let mut covered = BTreeSet::new();
    for (kind, name) in registrations {
        let (field, relation) = match kind.as_str() {
            "Document" => ("RegisterRecords", "registerRecord"),
            "DocumentJournal" => ("RegisteredDocuments", "registeredDocument"),
            _ => continue,
        };
        let (_, declaration) = unica_address::kinds::declared(kind).unwrap();
        let path = source
            .join(declaration.layout.unwrap().directory)
            .join(format!("{name}.xml"));
        let xml = read_fixture_descriptor(&path);
        let document = parse_fixture_descriptor(&xml, &path);
        let artifact = fixture_artifact(&document, kind, &path);
        assert_eq!(fixture_properties_name(artifact), Some(name.as_str()));
        let properties = child(artifact, MD_CLASSES_NS, "Properties");
        let Some(container) = properties
            .children()
            .find(|node| node.has_tag_name((MD_CLASSES_NS, field)))
        else {
            continue;
        };
        let targets: Vec<_> = container
            .children()
            .filter(roxmltree::Node::is_element)
            .map(|item| {
                assert!(item.has_tag_name((READABLE_NS, "Item")), "{path:?}");
                let target = item.text().unwrap();
                let (kind, name) = target.split_once('.').unwrap();
                assert!(registrations
                    .iter()
                    .any(|(tag, item)| tag == kind && item == name));
                (format!("main:{target}"), kind)
            })
            .collect();
        if targets.is_empty() {
            continue;
        }

        let owner = format!("main:{kind}.{name}");
        let branch = format!("{owner}.Relation");
        let collection = read(native.authority, &branch);
        assert_eq!(collection["kind"], "Relation");
        let items = collection["items"].as_array().unwrap();
        check_native_branch_count(&read(native.authority, &owner), &branch, items.len());
        let projected: Vec<_> = items
            .iter()
            .filter(|item| item["relation"] == relation)
            .collect();
        assert_eq!(projected.len(), targets.len(), "{branch}");
        for (row, (target, target_kind)) in projected.into_iter().zip(targets) {
            assert_eq!(row["at"], target, "{branch}");
            assert_eq!(row["kind"], target_kind, "{branch}");
            assert_eq!(read(native.authority, &target)["kind"], target_kind);
        }
        check_native_projection_refusal(
            native.authority,
            &format!("{branch}.Missing"),
            "the relation branch lists targets; read the target at its own address",
        );
        native.record(
            &owner,
            "Relation",
            &fixture_origin(&path),
            BTreeSet::from([branch]),
        );
        assert!(covered.insert(owner));
    }
    for owner in [
        "main:Document.Приход",
        "main:Document.Orders",
        "main:DocumentJournal.Entries",
    ] {
        assert!(
            covered.contains(owner),
            "{owner}: missing native relation input"
        );
    }
    covered
}
