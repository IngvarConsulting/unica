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
