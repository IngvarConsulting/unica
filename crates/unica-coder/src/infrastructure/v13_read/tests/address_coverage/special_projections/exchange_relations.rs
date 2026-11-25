use super::*;

const OWNER: &str = "main:ExchangePlan.Codes";
const BRANCH: &str = "main:ExchangePlan.Codes.Relation";
const CONTENT_PATH: &str = "ExchangePlans/Codes/Ext/Content.xml";
const CONTENT_NS: &str = "http://v8.1c.ru/8.3/xcf/extrnprops";

pub(super) fn native_exchange_relations(
    native: &mut NativeProjections<'_, '_>,
    all_kinds: &Path,
    version: &str,
    line: PlatformLine,
) {
    check_branch(
        native,
        all_kinds,
        &all_kinds.join(CONTENT_PATH),
        version,
        false,
    );

    let evidence = fixture_path(&format!("acceptance/native-exchange-relation/{version}"));
    let language = fixture_path(&format!(
        "acceptance/workspace-native-form-parameter/src-{version}"
    ));
    let fresh = tempfile::tempdir().unwrap();
    for (origin, relative) in [
        (evidence.as_path(), "Configuration.xml"),
        (evidence.as_path(), "Constants/Disabled.xml"),
        (evidence.as_path(), CONTENT_PATH),
        (language.as_path(), "Languages/Русский.xml"),
        (all_kinds, "Constants/Enabled.xml"),
        (all_kinds, "ExchangePlans/Codes.xml"),
    ] {
        let destination = fresh.path().join(relative);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::copy(origin.join(relative), destination).unwrap();
    }
    let cancellation = CancellationToken::new();
    let authority = native_authority(
        &cancellation,
        fresh.path(),
        PlatformProfile::for_line(line).unwrap(),
    );
    check_branch(
        &mut NativeProjections {
            authority: &authority,
            evidence: &mut *native.evidence,
            required: &mut *native.required,
        },
        fresh.path(),
        &evidence.join(CONTENT_PATH),
        version,
        true,
    );
}

fn check_branch(
    native: &mut NativeProjections<'_, '_>,
    source: &Path,
    content: &Path,
    version: &str,
    populated: bool,
) {
    let registrations = root_registrations(source, version);
    assert!(registrations
        .iter()
        .any(|(kind, name)| kind == "ExchangePlan" && name == "Codes"));
    let xml = read_fixture_descriptor(content);
    let document = parse_fixture_descriptor(&xml, content);
    let root = document.root_element();
    assert!(root.has_tag_name((CONTENT_NS, "ExchangePlanContent")));
    assert_eq!(root.attribute("version"), Some(version));
    let expected: Vec<_> = root
        .children()
        .filter(roxmltree::Node::is_element)
        .map(|item| {
            assert!(item.has_tag_name((CONTENT_NS, "Item")));
            let target = child(item, CONTENT_NS, "Metadata").text().unwrap();
            let mode = child(item, CONTENT_NS, "AutoRecord").text().unwrap();
            let (kind, name) = target.split_once('.').unwrap();
            assert!(registrations
                .iter()
                .any(|(tag, item)| tag == kind && item == name));
            (format!("main:{target}"), kind, mode)
        })
        .collect();
    if populated {
        assert_eq!(expected.len(), 2);
        assert_eq!(
            expected
                .iter()
                .map(|(_, _, mode)| *mode)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["Allow", "Deny"]),
        );
    } else {
        assert!(expected.is_empty());
    }

    let view = read(native.authority, BRANCH);
    assert_eq!(view["kind"], "Relation");
    let items = view["items"].as_array().unwrap();
    check_native_branch_count(&read(native.authority, OWNER), BRANCH, items.len());
    let included: Vec<_> = items
        .iter()
        .filter(|item| item["relation"] == "content")
        .collect();
    assert_eq!(included.len(), expected.len(), "{content:?}");
    for (item, (target, kind, mode)) in included.into_iter().zip(expected) {
        assert_eq!(item["at"], target);
        assert_eq!(item["kind"], kind);
        assert_eq!(item["autoRecord"], mode);
        // The address in the row names a separate object, not the row itself.
        assert_eq!(read(native.authority, &target)["kind"], kind);
    }
    check_native_projection_refusal(
        native.authority,
        &format!("{BRANCH}.Enabled"),
        "the relation branch lists targets; read the target at its own address",
    );
    native.record(
        OWNER,
        "Relation",
        &fixture_origin(content),
        BTreeSet::from([BRANCH.to_owned()]),
    );
}
