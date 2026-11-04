use super::*;

const OWNERS: &[(&str, &str, &str)] = &[
    ("Catalog", "Catalogs", "Codes"),
    ("Document", "Documents", "Приход"),
    ("Constant", "Constants", "Setting"),
    ("InformationRegister", "InformationRegisters", "Цены"),
    ("AccumulationRegister", "AccumulationRegisters", "Остатки"),
    ("AccountingRegister", "AccountingRegisters", "Бухгалтерский"),
    ("CalculationRegister", "CalculationRegisters", "Начисления"),
    ("ChartOfAccounts", "ChartsOfAccounts", "Хозрасчетный"),
    (
        "ChartOfCharacteristicTypes",
        "ChartsOfCharacteristicTypes",
        "Characteristics",
    ),
    (
        "ChartOfCalculationTypes",
        "ChartsOfCalculationTypes",
        "ВидыРасчета",
    ),
    ("BusinessProcess", "BusinessProcesses", "Approval"),
    ("Task", "Tasks", "Work"),
    ("ExchangePlan", "ExchangePlans", "Exchange"),
];

fn verify_owners(owners: &[(&str, &str, &str)]) {
    let fixture = Fixture::new("workspace-object-data-lock");
    let runtime = runtime();
    for (kind, directory, name) in owners {
        let at = format!("main:{kind}.{name}");
        let owner = PathBuf::from(format!("{directory}/{name}.xml"));
        let original = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let initial = String::from_utf8(original[&owner].clone()).unwrap();
        let can = contract(&runtime, &fixture, &at, "props.set");
        assert_eq!(
            can["argsSchema"]["properties"]["values"]["properties"]["DataLockControlMode"]["enum"],
            json!(["Automatic", "Managed"])
        );
        assert_parity(
            &can["argsSchema"],
            &fixture.binding(false),
            &at,
            "props.set",
            &[
                (json!({"values":{"DataLockControlMode":"Managed"}}), true),
                (
                    json!({"values":{"DataLockControlMode":"AutomaticAndManaged"}}),
                    false,
                ),
                (json!({"values":{"DataLockControlMode":true}}), false),
                (json!({"values":{"DataLockControlMode":"Unknown"}}), false),
            ],
        );
        for mode in ["Managed", "Managed", "Automatic"] {
            publish(
                &runtime,
                &fixture,
                json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"DataLockControlMode":mode}}}]}),
            );
            let mut expected = original.clone();
            expected.insert(
                owner.clone(),
                initial
                    .replace(
                        "<DataLockControlMode>Automatic</DataLockControlMode>",
                        &format!("<DataLockControlMode>{mode}</DataLockControlMode>"),
                    )
                    .into_bytes(),
            );
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                expected
            );
            assert_eq!(
                view(&runtime, &fixture, &at)["props"]["DataLockControlMode"],
                mode
            );
        }
        if *kind == "Catalog" {
            let root_search = "\n\t\t\t<FullTextSearch>Use</FullTextSearch>";
            assert_eq!(initial.matches(root_search).count(), 1);
            let expected_xml = initial
                .replace(
                    "<DataLockControlMode>Automatic</DataLockControlMode>",
                    "<DataLockControlMode>Managed</DataLockControlMode>",
                )
                .replacen(
                    root_search,
                    "\n\t\t\t<FullTextSearch>DontUse</FullTextSearch>",
                    1,
                );
            let mut expected = original.clone();
            expected.insert(owner.clone(), expected_xml.into_bytes());
            for _ in 0..2 {
                publish(
                    &runtime,
                    &fixture,
                    json!({"at":at,"ops":[{"op":"props.set","args":{"values":{
                        "DataLockControlMode":"Managed","FullTextSearch":"DontUse"
                    }}}]}),
                );
                assert_eq!(
                    unica_workspace::test_support::tree_snapshot(&fixture.source),
                    expected
                );
                let observed = view(&runtime, &fixture, &at);
                assert_eq!(observed["props"]["DataLockControlMode"], "Managed");
                assert_eq!(observed["props"]["FullTextSearch"], "DontUse");
            }
        }
    }
}

#[test]
fn object_data_lock_preserves_constants_documents_and_registers() {
    verify_owners(&OWNERS[..7]);
}
#[test]
fn object_data_lock_preserves_charts_processes_and_exchange() {
    verify_owners(&OWNERS[7..]);
}

#[test]
fn object_data_lock_refuses_damaged_source_and_stale_owner() {
    let fixture = Fixture::new("workspace-object-data-lock");
    let runtime = runtime();
    let at = "main:Catalog.Codes";
    let path = fixture.source.join("Catalogs/Codes.xml");
    let initial = std::fs::read_to_string(&path).unwrap();
    let old = "<DataLockControlMode>Automatic</DataLockControlMode>";
    for damaged in ["","<DataLockControlMode/>","<DataLockControlMode>AutomaticAndManaged</DataLockControlMode>","<DataLockControlMode>Automatic<!-- split -->Unknown</DataLockControlMode>","<DataLockControlMode xmlns=\"foreign\">Automatic</DataLockControlMode>","<DataLockControlMode><value>Automatic</value></DataLockControlMode>","<DataLockControlMode>Automatic</DataLockControlMode><DataLockControlMode>Managed</DataLockControlMode>"] {
        std::fs::write(&path,initial.replace(old,damaged)).unwrap();
        let before=unica_workspace::test_support::tree_snapshot(&fixture.source);
        let refused=call(&runtime,&fixture,ToolIdentity::Apply,json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"DataLockControlMode":"Managed"}}}]}));
        assert!(!refused.ok,"{damaged}: {refused:?}");
        assert_eq!(unica_workspace::test_support::tree_snapshot(&fixture.source),before);
    }
    std::fs::write(&path, &initial).unwrap();
    let plan = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"DataLockControlMode":"Managed"}}}]}),
    );
    assert!(plan.ok, "{plan:?}");
    std::fs::write(
        &path,
        initial.replace("<Comment/>", "<Comment>Changed</Comment>"),
    )
    .unwrap();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"executionToken":plan.data.unwrap()["executionToken"]}),
    );
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}

#[test]
fn object_data_lock_preserves_a_fully_prefixed_descriptor() {
    let fixture = Fixture::new("workspace-object-data-lock");
    let runtime = runtime();
    let owner = PathBuf::from("Catalogs/Codes.xml");
    let path = fixture.source.join(&owner);
    let original = std::fs::read_to_string(&path).unwrap();
    let tags = regex::Regex::new(r"<(/?)([A-Za-z_][A-Za-z0-9_]*)([ >/\t\r\n])").unwrap();
    let prefixed = tags.replace_all(&original, "<${1}md:${2}${3}").replace(
        "xmlns=\"http://v8.1c.ru/8.3/MDClasses\"",
        "xmlns:md=\"http://v8.1c.ru/8.3/MDClasses\"",
    );
    assert!(!prefixed.contains("xmlns=\""));
    let doc = roxmltree::Document::parse(prefixed.trim_start_matches('\u{feff}')).unwrap();
    assert!(doc
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "DataLockControlMode")
        .all(|node| node.tag_name().namespace() == Some("http://v8.1c.ru/8.3/MDClasses")));
    std::fs::write(&path, &prefixed).unwrap();
    let at = "main:Catalog.Codes";
    assert_eq!(
        view(&runtime, &fixture, at)["props"]["DataLockControlMode"],
        "Automatic"
    );
    let mut expected = unica_workspace::test_support::tree_snapshot(&fixture.source);
    expected.insert(
        owner,
        prefixed
            .replace(
                "<md:DataLockControlMode>Automatic</md:DataLockControlMode>",
                "<md:DataLockControlMode>Managed</md:DataLockControlMode>",
            )
            .into_bytes(),
    );
    for _ in 0..2 {
        publish(
            &runtime,
            &fixture,
            json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"DataLockControlMode":"Managed"}}}]}),
        );
        assert_eq!(
            view(&runtime, &fixture, at)["props"]["DataLockControlMode"],
            "Managed"
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            expected
        );
    }
}
