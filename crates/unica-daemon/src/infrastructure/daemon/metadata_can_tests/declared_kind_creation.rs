use super::*;

const EXPORT: &str = "../../tests/fixtures/formats/platform_xml/2.20/declared_creation";
const CONFIGURATOR: &str = "../../tests/fixtures/formats/platform_xml/2.20/configurator_creation";

/// The declared kinds outside the typed set that `object.create` writes on
/// every described line, with the reference descriptor, its directory and
/// name. The platform export is the reference where the platform loads the
/// new object; the Configurator's own creation is the reference where it
/// leaves a required property empty.
const KINDS: &[(&str, &str, &str, &str)] = &[
    (
        "SessionParameter",
        EXPORT,
        "SessionParameters",
        "SessionProbe",
    ),
    ("CommonAttribute", EXPORT, "CommonAttributes", "Probe"),
    ("StyleItem", EXPORT, "StyleItems", "Probe"),
    ("SettingsStorage", EXPORT, "SettingsStorages", "Probe"),
    ("WSReference", EXPORT, "WSReferences", "Probe"),
    ("DocumentNumerator", EXPORT, "DocumentNumerators", "Sales"),
    (
        "FunctionalOption",
        CONFIGURATOR,
        "FunctionalOptions",
        "ФункциональнаяОпция1",
    ),
    (
        "FunctionalOptionsParameter",
        CONFIGURATOR,
        "FunctionalOptionsParameters",
        "ПараметрФункциональныхОпций1",
    ),
    ("Sequence", CONFIGURATOR, "Sequences", "Последовательность1"),
    ("Language", CONFIGURATOR, "Languages", "Язык1"),
];

/// The descriptor with every UUID replaced: the object, type and value
/// identifiers are fresh in each creation.
fn without_uuids(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(|ch: char| ch.is_ascii_hexdigit()) {
        let candidate = &rest[at..];
        if candidate.len() >= 36
            && candidate.is_char_boundary(36)
            && uuid::Uuid::parse_str(&candidate[..36]).is_ok()
        {
            out.push_str(&rest[..at]);
            out.push_str("<uuid>");
            rest = &candidate[36..];
        } else {
            out.push_str(&rest[..=at]);
            rest = &rest[at + 1..];
        }
    }
    out.push_str(rest);
    out
}

/// The descriptor with its object `Synonym` emptied. Unica derives the
/// synonym of every new object from its name; the Configurator leaves the
/// synonym of its default name empty.
fn without_synonym(text: &str) -> String {
    match (text.find("			<Synonym>"), text.find("</Synonym>")) {
        (Some(start), Some(end)) => {
            format!(
                "{}			<Synonym/>{}",
                &text[..start],
                &text[end + "</Synonym>".len()..]
            )
        }
        _ => text.to_string(),
    }
}

/// Each kind is offered by `can`, written byte for byte as its reference,
/// registered in the configuration and
/// read back by `view`.
/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn declared_kinds_are_created_as_their_reference_descriptor() {
    let fixture = Fixture::blank();
    let runtime = runtime();
    let can = contract(&runtime, &fixture, "main:Configuration", "object.create");
    let offered = can["argsSchema"]["properties"]["values"]["properties"]["kind"]["enum"]
        .as_array()
        .unwrap()
        .clone();
    for (kind, reference, directory, name) in KINDS {
        assert!(offered.contains(&json!(kind)), "{kind}: {offered:?}");
        publish(
            &runtime,
            &fixture,
            json!({"at":"main:Configuration","ops":[{"op":"object.create","args":{"values":{"kind":kind,"name":name}}}]}),
        );
        let relative = format!("{directory}/{name}.xml");
        let written = std::fs::read_to_string(fixture.source.join(&relative)).unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(reference);
        let platform = std::fs::read_to_string(root.join(&relative)).unwrap();
        let written = if *reference == CONFIGURATOR {
            without_synonym(&written)
        } else {
            written
        };
        assert_eq!(without_uuids(&written), without_uuids(&platform), "{kind}");
        assert!(
            std::fs::read_to_string(fixture.source.join("Configuration.xml"))
                .unwrap()
                .contains(&format!("<{kind}>{name}</{kind}>")),
            "{kind}"
        );
        let node = view(&runtime, &fixture, &format!("main:{kind}.{name}"));
        assert_eq!(
            (&node["title"], &node["kind"]),
            (&json!(name), &json!(kind)),
            "{node}"
        );
    }
}

/// Each new language takes the first free numbered code, as the Configurator
/// gives it, also when an earlier operation of the same apply took one.
/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn new_languages_take_the_next_free_code() {
    let fixture = Fixture::blank();
    let runtime = runtime();
    let code = |name: &str| {
        let text =
            std::fs::read_to_string(fixture.source.join(format!("Languages/{name}.xml"))).unwrap();
        let start = text.find("<LanguageCode>").unwrap() + "<LanguageCode>".len();
        text[start..start + text[start..].find('<').unwrap()].to_string()
    };
    let create = |name: &str| json!({"op":"object.create","args":{"values":{"kind":"Language","name":name}}});
    publish(
        &runtime,
        &fixture,
        json!({"at":"main:Configuration","ops":[create("First")]}),
    );
    assert_eq!(code("First"), "ru1");
    publish(
        &runtime,
        &fixture,
        json!({"at":"main:Configuration","ops":[create("Second"), create("Third")]}),
    );
    assert_eq!(
        (code("Second"), code("Third")),
        ("ru2".into(), "ru3".into())
    );
}

/// An extension borrows its languages: `can` does not offer the kind there
/// and `apply` refuses it without writing.
/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn an_extension_does_not_create_a_language() {
    let fixture = Fixture::with_extension();
    let runtime = runtime();
    let can = contract(&runtime, &fixture, "ext:Configuration", "object.create");
    let offered = can["argsSchema"]["properties"]["values"]["properties"]["kind"]["enum"]
        .as_array()
        .unwrap()
        .clone();
    assert!(offered.contains(&json!("SessionParameter")), "{offered:?}");
    assert!(!offered.contains(&json!("Language")), "{offered:?}");
    let extension = fixture.workspace.join("src-ext");
    let before = unica_workspace::test_support::tree_snapshot(&extension);
    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":"ext:Configuration","ops":[{"op":"object.create","args":{"values":{"kind":"Language","name":"Own"}}}]}),
    );
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(refused.at.as_deref(), Some("ops[0].args.values.kind"));
    assert_eq!(
        refused.summary,
        "an extension borrows `Language` from the main configuration instead of creating it"
    );
    assert_eq!(refused.diagnostics[0]["code"], "bad_value");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&extension),
        before
    );
}

/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn an_empty_integration_service_matches_the_platform_descriptor() {
    let relative = Path::new("IntegrationServices/Probe.xml");
    for (version, platform_path) in [
        (
            "2.20",
            "../../tests/fixtures/formats/platform_xml/2.20/all_kinds/src/IntegrationServices/Probe.xml",
        ),
        (
            "2.21",
            "../../tests/fixtures/formats/platform_xml/2.21/all_kinds/src/IntegrationServices/Probe.xml",
        ),
        (
            "2.22",
            "../../tests/fixtures/formats/platform_xml/2.22/all_kinds/src/IntegrationServices/Probe.xml",
        ),
    ] {
        let fixture = Fixture::blank();
        for path in ["Configuration.xml", "Languages/Русский.xml"] {
            let path = fixture.source.join(path);
            let xml = std::fs::read_to_string(&path).unwrap();
            std::fs::write(
                path,
                xml.replace("version=\"2.20\"", &format!("version=\"{version}\"")),
            )
            .unwrap();
        }
        let runtime = runtime();
        let at = "main:Configuration";
        let can = contract(&runtime, &fixture, at, "object.create");
        let args = json!({"values":{"kind":"IntegrationService","name":"Probe"}});
        assert!(
            jsonschema::validator_for(&can["argsSchema"])
                .unwrap()
                .is_valid(&args),
            "{version}"
        );
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        publish(
            &runtime,
            &fixture,
            json!({"at":at,"ops":[{"op":"object.create","args":args}]}),
        );

        let written = std::fs::read_to_string(fixture.source.join(relative)).unwrap();
        let platform =
            std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(platform_path))
                .unwrap();
        let normalize = |xml: &str| {
            without_uuids(&without_synonym(xml.trim_start_matches('\u{feff}')))
                .lines()
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert_eq!(normalize(&written), normalize(&platform), "{version}");
        let after = unica_workspace::test_support::tree_snapshot(&fixture.source);
        assert_eq!(
            changed_paths(&before, &after),
            (
                BTreeSet::from([relative.to_path_buf()]),
                BTreeSet::from([PathBuf::from("Configuration.xml")]),
                BTreeSet::new(),
            ),
            "{version}"
        );
        let node = view(&runtime, &fixture, "main:IntegrationService.Probe");
        assert_eq!(node["kind"], "IntegrationService", "{version}");
        assert_eq!(node["title"], "Probe", "{version}");
    }
}

#[test]
fn synonym_is_emptied_and_the_rest_is_kept() {
    assert_eq!(
        without_synonym(
            "			<Name>A</Name>
			<Synonym>
				<v8:item/>
			</Synonym>
			<Comment/>"
        ),
        "			<Name>A</Name>
			<Synonym/>
			<Comment/>"
    );
}

#[test]
fn uuids_are_masked_and_the_rest_is_kept() {
    assert_eq!(
        without_uuids("<a uuid=\"1d66d9f6-0206-5440-a2f2-9a91db9621b7\">ab-1 #000000 Синоним</a>"),
        "<a uuid=\"<uuid>\">ab-1 #000000 Синоним</a>"
    );
}
#[test]
fn filter_criterion_create_advertises_only_its_own_type_arguments() {
    let fixture = Fixture::blank();
    let runtime = runtime();
    let at = "main:Configuration";
    let can = contract(&runtime, &fixture, at, "object.create");
    let schema = &can["argsSchema"];
    assert_eq!(
        schema["properties"]["values"]["additionalProperties"],
        false
    );
    let text_type = json!({"variants":[{"kind":"string","length":20,"allowedLength":"variable"}]});
    let cases = [
        (
            json!({"values":{"kind":"FilterCriterion","name":"Criterion","type":text_type}}),
            true,
        ),
        (
            json!({"values":{"kind":"FilterCriterion","name":"Criterion","type":{
                "variants":[{"kind":"reference","metadataPath":"Справочник.Products"}]
            }}}),
            true,
        ),
        (
            json!({"values":{"kind":"FilterCriterion","name":"Criterion"}}),
            false,
        ),
        (
            json!({"values":{"kind":"FilterCriterion","name":"Criterion","type":null}}),
            false,
        ),
        (
            json!({"values":{"kind":"FilterCriterion","name":"Criterion","type":{"variants":[]}}}),
            false,
        ),
        (
            json!({"values":{"kind":"FilterCriterion","name":"Criterion","type":{
                "variants":[{"kind":"reference","metadataPath":"CommonModule.NotAReference"}]
            }}}),
            false,
        ),
        (
            json!({"values":{"kind":"FilterCriterion","name":"Criterion","type":{
                "variants":[{"kind":"reference","metadataPath":"other:Catalog.Products"}]
            }}}),
            false,
        ),
        (
            json!({"values":{"kind":"FilterCriterion","name":"Criterion","type":text_type,"content":[]}}),
            false,
        ),
        (
            json!({"values":{"kind":"Catalog","name":"Products","type":text_type}}),
            false,
        ),
        (json!({"values":{"kind":"Catalog","name":"Products"}}), true),
    ];
    assert_parity(schema, &fixture.binding(false), at, "object.create", &cases);
    let raw = unica_metadata::contracts::metadata_object_create();
    for (args, expected) in [
        (&cases[0].0, true),
        (&cases[2].0, false),
        (&cases[8].0, false),
        (&cases[9].0, true),
    ] {
        assert_eq!(raw.normalize(args, "args").is_ok(), expected, "{args}");
    }
}

#[test]
fn filter_criterion_create_matches_native_descriptors_in_each_writable_format() {
    const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
    const XR: &str = "http://v8.1c.ru/8.3/xcf/readable";
    for (version, platform_path) in [
        (
            "2.20",
            "../../tests/fixtures/formats/platform_xml/2.20/all_kinds/src/FilterCriteria/Probe.xml",
        ),
        (
            "2.21",
            "../../tests/fixtures/formats/platform_xml/2.21/all_kinds/src/FilterCriteria/Probe.xml",
        ),
        (
            "2.22",
            "../../tests/fixtures/formats/platform_xml/2.22/all_kinds/src/FilterCriteria/Probe.xml",
        ),
    ] {
        let fixture = Fixture::blank();
        for relative in ["Configuration.xml", "Languages/Русский.xml"] {
            let path = fixture.source.join(relative);
            let xml = std::fs::read_to_string(&path).unwrap();
            std::fs::write(
                &path,
                xml.replace("version=\"2.20\"", &format!("version=\"{version}\"")),
            )
            .unwrap();
        }
        let runtime = runtime();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let values = json!({"kind":"FilterCriterion","name":"Probe","type":{"variants":[
            {"kind":"string","length":20,"allowedLength":"variable"}
        ]}});
        let args = json!({"values":values});
        let can = contract(&runtime, &fixture, "main:Configuration", "object.create");
        assert!(jsonschema::validator_for(&can["argsSchema"])
            .unwrap()
            .is_valid(&args));
        publish(
            &runtime,
            &fixture,
            json!({"at":"main:Configuration","ops":[{"op":"object.create","args":args}]}),
        );

        let relative = "FilterCriteria/Probe.xml";
        let observed = std::fs::read_to_string(fixture.source.join(relative)).unwrap();
        let native =
            std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(platform_path))
                .unwrap();
        let actual = roxmltree::Document::parse(observed.trim_start_matches('\u{feff}')).unwrap();
        let oracle = roxmltree::Document::parse(native.trim_start_matches('\u{feff}')).unwrap();
        let actual_root = actual.root_element();
        assert_eq!(actual_root.attribute("version"), Some(version));
        let metadata = actual_root.first_element_child().unwrap();
        assert!(metadata.has_tag_name((MD, "FilterCriterion")));
        let generated = metadata
            .descendants()
            .filter(|node| node.has_tag_name((XR, "GeneratedType")))
            .map(|node| {
                (
                    node.attribute("name").unwrap(),
                    node.attribute("category").unwrap(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            generated,
            [
                ("FilterCriterionManager.Probe", "Manager"),
                ("FilterCriterionList.Probe", "List")
            ]
        );
        let ids = metadata
            .descendants()
            .filter(|n| n.has_tag_name((XR, "TypeId")) || n.has_tag_name((XR, "ValueId")))
            .map(|n| uuid::Uuid::parse_str(n.text().unwrap()).unwrap())
            .collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), 4);
        let properties = |doc: &str| {
            let doc = roxmltree::Document::parse(doc.trim_start_matches('\u{feff}')).unwrap();
            doc.root_element()
                .first_element_child()
                .unwrap()
                .children()
                .find(|node| node.has_tag_name((MD, "Properties")))
                .unwrap()
                .children()
                .filter(|node| node.is_element() && !node.has_tag_name((MD, "Synonym")))
                .map(|node| doc.input_text()[node.range()].replace("\r\n", "\n"))
                .collect::<Vec<_>>()
        };
        assert_eq!(properties(&observed), properties(&native), "{version}");
        let children = metadata
            .children()
            .find(|n| n.has_tag_name((MD, "ChildObjects")))
            .unwrap();
        assert!(children.children().all(|n| !n.is_element()));
        assert!(oracle
            .root_element()
            .first_element_child()
            .unwrap()
            .children()
            .any(|n| n.has_tag_name((MD, "ChildObjects"))));
        let config = std::fs::read_to_string(fixture.source.join("Configuration.xml")).unwrap();
        assert!(config.contains("<FilterCriterion>Probe</FilterCriterion>"));
        let after = unica_workspace::test_support::tree_snapshot(&fixture.source);
        assert_eq!(
            changed_paths(&before, &after),
            (
                BTreeSet::from([PathBuf::from(relative)]),
                BTreeSet::from([PathBuf::from("Configuration.xml")]),
                BTreeSet::new(),
            )
        );

        let duplicate = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":"main:Configuration","ops":[{"op":"object.create","args":args}]}),
        );
        assert!(!duplicate.ok, "{duplicate:?}");
        assert_eq!(duplicate.diagnostics[0]["code"], "invalid_state");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            after
        );
    }
}

#[test]
fn filter_criterion_create_resolves_references_in_the_staged_source_set() {
    let fixture = Fixture::blank();
    let runtime = runtime();
    let target = |name| {
        json!({"op":"object.create","args":{"values":{
            "kind":"FilterCriterion","name":"ForProducts","type":{"variants":[
                {"kind":"reference","metadataPath":name}
            ]}
        }}})
    };
    let catalog =
        json!({"op":"object.create","args":{"values":{"kind":"Catalog","name":"Products"}}});
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    for (name, code) in [
        ("Department.Products", "bad_value"),
        ("Catalog.Missing", "not_found"),
    ] {
        let refused = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({
                "at":"main:Configuration","ops":[catalog,target(name)]
            }),
        );
        assert!(!refused.ok, "{refused:?}");
        assert_eq!(
            refused.at.as_deref(),
            Some("ops[1].args.values.type.variants[0].metadataPath")
        );
        assert_eq!(refused.diagnostics[0]["code"], code);
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
    publish(
        &runtime,
        &fixture,
        json!({
            "at":"main:Configuration","ops":[catalog, target("Справочник.Products")]
        }),
    );
    let xml =
        std::fs::read_to_string(fixture.source.join("FilterCriteria/ForProducts.xml")).unwrap();
    assert!(xml.contains("<v8:Type>cfg:CatalogRef.Products</v8:Type>"));
}

/// rule: INV.SOURCE.REVISION-PROJECTION-CAPTURE-EQUALITY
/// rule: INV.SOURCE.RETAINED-APPLY-WRITE-FREE
#[test]
fn filter_criterion_create_refuses_missing_wrong_and_stale_reference_descriptors() {
    for (fault, code) in [
        ("missing", "not_found"),
        ("identity", "invalid_source"),
        ("format", "invalid_source"),
        ("stale", "stale_revision"),
    ] {
        let fixture = Fixture::blank();
        let runtime = runtime();
        publish(
            &runtime,
            &fixture,
            json!({"at":"main:Configuration","ops":[
                {"op":"object.create","args":{"values":{"kind":"Catalog","name":"Products"}}}
            ]}),
        );
        let request = json!({"at":"main:Configuration","ops":[
            {"op":"object.create","args":{"values":{
                "kind":"FilterCriterion","name":"ByProduct",
                "type":{"variants":[{"kind":"reference","metadataPath":"Catalog.Products"}]}
            }}}
        ]});
        let descriptor = fixture.source.join("Catalogs/Products.xml");
        let original = std::fs::read_to_string(&descriptor).unwrap();
        let execution = match fault {
            "missing" => {
                std::fs::remove_file(&descriptor).unwrap();
                request
            }
            "identity" | "format" => {
                let (old, new) = if fault == "identity" {
                    ("<Name>Products</Name>", "<Name>Services</Name>")
                } else {
                    ("version=\"2.20\"", "version=\"2.21\"")
                };
                assert!(original.contains(old), "{fault}: {original}");
                std::fs::write(&descriptor, original.replace(old, new)).unwrap();
                request
            }
            _ => {
                let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
                let planned = call(&runtime, &fixture, ToolIdentity::Apply, request);
                assert!(planned.ok, "{planned:?}");
                assert_eq!(
                    unica_workspace::test_support::tree_snapshot(&fixture.source),
                    before
                );
                assert!(original.contains("<Comment/>"));
                std::fs::write(
                    &descriptor,
                    original.replace("<Comment/>", "<Comment>Independent change</Comment>"),
                )
                .unwrap();
                json!({"executionToken":planned.data.unwrap()["executionToken"].as_str().unwrap()})
            }
        };

        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let refused = call(&runtime, &fixture, ToolIdentity::Apply, execution);
        assert!(!refused.ok, "{fault}: {refused:?}");
        assert!(
            refused
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic["code"] == code),
            "{fault}: {refused:?}"
        );
        if fault != "stale" {
            assert_eq!(
                refused.at.as_deref(),
                Some("ops[0].args.values.type.variants[0].metadataPath"),
                "{fault}: {refused:?}"
            );
        }
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before,
            "{fault}"
        );
    }
}
