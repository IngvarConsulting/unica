use super::*;

const EXPORT: &str = "../../tests/fixtures/formats/platform_xml/2.20/declared_creation";
const CONFIGURATOR: &str = "../../tests/fixtures/formats/platform_xml/2.20/configurator_creation";
const EXPORT_8_5_4: &str = "../../tests/fixtures/formats/platform_xml/2.22/declared_creation";

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
    ("Style", CONFIGURATOR, "Styles", "Стиль1"),
    ("Bot", CONFIGURATOR, "Bots", "Бот1"),
    (
        "WebSocketClient",
        CONFIGURATOR,
        "WebSocketClients",
        "WebSocketКлиент1",
    ),
    (
        "CommonCommand",
        CONFIGURATOR,
        "CommonCommands",
        "ОбщаяКоманда1",
    ),
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

/// A new common command gets the handler module the Configurator writes, in
/// the script variant of the configuration: Russian by default, English when
/// the configuration says so.
/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn a_common_command_takes_the_module_of_the_script_variant() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONFIGURATOR);
    for (english, name) in [(false, "ОбщаяКоманда1"), (true, "CommonCommand1")] {
        let fixture = Fixture::blank();
        if english {
            let path = fixture.source.join("Configuration.xml");
            let text = std::fs::read_to_string(&path).unwrap().replace(
                "</DefaultLanguage>",
                "</DefaultLanguage><ScriptVariant>English</ScriptVariant>",
            );
            std::fs::write(path, text).unwrap();
        }
        let runtime = runtime();
        publish(
            &runtime,
            &fixture,
            json!({"at":"main:Configuration","ops":[{"op":"object.create","args":{"values":{"kind":"CommonCommand","name":name}}}]}),
        );
        let module = format!("CommonCommands/{name}/Ext/CommandModule.bsl");
        assert_eq!(
            std::fs::read(fixture.source.join(&module)).unwrap(),
            std::fs::read(root.join(&module)).unwrap(),
            "{module}"
        );
        let descriptor = format!("CommonCommands/{name}.xml");
        assert_eq!(
            without_uuids(&without_synonym(
                &std::fs::read_to_string(fixture.source.join(&descriptor)).unwrap()
            )),
            without_uuids(&std::fs::read_to_string(root.join(&descriptor)).unwrap()),
            "{descriptor}"
        );
    }
}

/// A script variant the platform does not know leaves the handler language
/// undecided: the command is refused before anything is written.
/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn a_common_command_is_refused_under_an_unknown_script_variant() {
    let fixture = Fixture::blank();
    let path = fixture.source.join("Configuration.xml");
    let text = std::fs::read_to_string(&path).unwrap().replace(
        "</DefaultLanguage>",
        "</DefaultLanguage><ScriptVariant>Latin</ScriptVariant>",
    );
    std::fs::write(path, text).unwrap();
    let runtime = runtime();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":"main:Configuration","ops":[{"op":"object.create","args":{"values":{"kind":"CommonCommand","name":"Probe"}}}]}),
    );
    assert!(!refused.ok, "{refused:?}");
    assert!(refused.summary.contains("ScriptVariant"), "{refused:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.workspace),
        before
    );
}

/// A palette color of the 8.5.4 line is created as the platform exports one
/// imported without its color.
/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn a_palette_color_is_created_as_the_platform_exports_it() {
    let fixture = super::queue::line_8_5_4_fixture("");
    let runtime = runtime();
    publish(
        &runtime,
        &fixture,
        json!({"at":"main:Configuration","ops":[{"op":"object.create","args":{"values":{"kind":"PaletteColor","name":"Probe"}}}]}),
    );
    let relative = "PaletteColors/Probe.xml";
    let platform = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(EXPORT_8_5_4)
            .join(relative),
    )
    .unwrap();
    let written = std::fs::read_to_string(fixture.source.join(relative)).unwrap();
    assert_eq!(
        without_uuids(&without_synonym(&written)),
        without_uuids(&platform)
    );
    assert!(
        std::fs::read_to_string(fixture.source.join("Configuration.xml"))
            .unwrap()
            .contains("<PaletteColor>Probe</PaletteColor>")
    );
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
