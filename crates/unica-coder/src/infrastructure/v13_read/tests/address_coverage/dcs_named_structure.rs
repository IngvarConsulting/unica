use super::*;

const SHARD: &str = "native_dcs_named_structure_follows_the_exported_names";
const REPORT: &str = "АнализВерсийОбъектов";
const TEMPLATE: &str = "ОсновнаяСхемаКомпоновкиДанных";
const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";

/// rule: CTR.SOURCE.QUALIFIED-LOGICAL-ADDRESS
#[test]
fn native_dcs_named_structure_follows_the_exported_names() {
    let captures = fixture_path("acceptance/native-dcs-addresses");
    let receipt: Value =
        serde_json::from_str(&read_fixture_descriptor(&captures.join("receipt.json"))).unwrap();
    let registrations = [
        PathBuf::from("Configuration.xml"),
        PathBuf::from(format!("Reports/{REPORT}.xml")),
        PathBuf::from(format!("Reports/{REPORT}/Templates/{TEMPLATE}.xml")),
    ];
    let overlay_at = PathBuf::from(format!(
        "Reports/{REPORT}/Templates/{TEMPLATE}/Ext/Template.xml"
    ));
    let mut evidence = EvidenceRows::new(SHARD, SourceSetKind::Configuration);
    for version in ["2.20", "2.21", "2.22"] {
        let capture = receipt["captures"]
            .as_array()
            .unwrap()
            .iter()
            .find(|capture| capture["formatVersion"] == version)
            .unwrap();
        let source = fixture_path(&format!("formats/platform_xml/{version}/all_kinds/src"));
        let native = captures.join(version).join("Template.xml");
        let overlay = tempfile::tempdir().unwrap();
        let resource = overlay.path().join(&overlay_at);
        fs::create_dir_all(resource.parent().unwrap()).unwrap();
        for path in &registrations {
            fs::copy(source.join(path), overlay.path().join(path)).unwrap();
        }
        fs::copy(&native, resource).unwrap();

        let raw = read_fixture_descriptor(&native);
        let document = parse_fixture_descriptor(&raw, &native);
        let schema = document.root_element();
        assert!(schema.has_tag_name((DCS_NS, "DataCompositionSchema")));
        let variant = child(schema, DCS_NS, "settingsVariant");
        let name = child(variant, DCS_SETTINGS_NS, "name").text().unwrap();
        let settings = child(variant, DCS_SETTINGS_NS, "settings");
        let [(parent_name, parent)] = named_groups(settings).try_into().unwrap();
        let [(child_name, _)] = named_groups(parent).try_into().unwrap();
        let line = export_format_line(version).unwrap();
        let cancellation = CancellationToken::new();
        let authority = native_authority(
            &cancellation,
            overlay.path(),
            PlatformProfile::for_line(line).unwrap(),
        );
        let template = format!("main:Report.{REPORT}.Template.{TEMPLATE}");
        let at = format!("{template}.Setting.{name}");
        let origin = fixture_origin(&native);
        let anonymous = settings
            .children()
            .filter(|node| {
                node.has_tag_name((DCS_SETTINGS_NS, "item"))
                    && node.attribute((XSI, "type")) == Some("dcsset:StructureItemChart")
            })
            .map(|chart| {
                1 + chart
                    .children()
                    .filter(|node| node.has_tag_name((DCS_SETTINGS_NS, "point")))
                    .count()
            })
            .sum::<usize>();
        assert_eq!(
            anonymous, 4,
            "native charts and their anonymous point groups"
        );
        let group = check_group(&authority, &at, parent_name, anonymous);
        let nested = check_group(&authority, &group, child_name, 0);
        for (owner, member) in [(&at, &group), (&group, &nested)] {
            for address in [&format!("{owner}.Item"), member] {
                evidence.record(
                    line,
                    owner,
                    "Item",
                    None,
                    &origin,
                    AddressEvidence::Read(address),
                );
            }
        }
        for absent in [
            format!("{at}.Item.{child_name}"),
            format!("{group}.Item.MissingGroup"),
        ] {
            check_native_projection_refusal(&authority, &absent, "was not found uniquely");
        }
        let field = child(schema, DCS_NS, "calculatedField");
        let field_name = child(field, DCS_NS, "dataPath").text().unwrap();
        let expression = child(field, DCS_NS, "expression").text().unwrap();
        assert_eq!(capture["calculatedField"]["dataPath"], field_name);
        assert_eq!(capture["calculatedField"]["expression"], expression);
        let fields = format!("{template}.CalculatedField");
        let member = QualifiedAddress::parse(&fields)
            .unwrap()
            .member(field_name)
            .unwrap()
            .to_string();
        let collection = read(&authority, &fields);
        assert_eq!(collection["kind"], "CalculatedField");
        assert_eq!(
            addresses(&collection, "items"),
            BTreeSet::from([member.clone()])
        );
        let view = read(&authority, &member);
        assert_eq!(view["kind"], "CalculatedField");
        assert_eq!(view["props"]["expression"], expression);
        for address in [&fields, &member] {
            evidence.record(
                line,
                &template,
                "CalculatedField",
                None,
                &origin,
                AddressEvidence::Read(address),
            );
        }
        check_native_projection_refusal(
            &authority,
            &format!("{fields}.MissingCalculatedField"),
            "was not found",
        );
    }
    evidence.finish();
}

fn named_groups<'a, 'i: 'a>(
    container: roxmltree::Node<'a, 'i>,
) -> Vec<(&'a str, roxmltree::Node<'a, 'i>)> {
    container
        .children()
        .filter(|node| node.has_tag_name((DCS_SETTINGS_NS, "item")))
        .filter_map(|item| {
            let name = item
                .children()
                .find(|node| node.has_tag_name((DCS_SETTINGS_NS, "name")))?
                .text()
                .unwrap();
            assert_eq!(
                item.attribute((XSI, "type")),
                Some("dcsset:StructureItemGroup")
            );
            Some((name, item))
        })
        .collect()
}

fn check_group(
    authority: &LogicalViewReadAuthority<'_>,
    owner: &str,
    name: &str,
    anonymous: usize,
) -> String {
    let collection = QualifiedAddress::parse(&format!("{owner}.Item")).unwrap();
    let member = collection.member(name).unwrap().to_string();
    let view = read(authority, &collection.to_string());
    assert_eq!(view["kind"], "Item");
    let rows = view["items"].as_array().unwrap();
    assert_eq!(rows.len(), anonymous + 1);
    let published: Vec<_> = rows.iter().filter_map(|row| row.get("at")).collect();
    assert_eq!(published.len(), 1);
    assert_eq!(published[0], &json!(member));
    assert!(rows
        .iter()
        .filter(|row| row.get("at").is_none())
        .all(|row| {
            row["name"].is_null() && matches!(row["kind"].as_str(), Some("Chart" | "Group"))
        }));
    assert_eq!(read(authority, &member)["kind"], "Item");
    member
}
