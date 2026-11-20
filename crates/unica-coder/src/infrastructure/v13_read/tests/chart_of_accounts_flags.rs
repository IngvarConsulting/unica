use super::*;

const LEDGER: &str = "ChartsOfAccounts/Ledger.xml";

fn fixture() -> RealReaderFixture {
    let f = RealReaderFixture::new();
    let config_path = f.source.join("Configuration.xml");
    let config = fs::read_to_string(&config_path).unwrap();
    write(
        &config_path,
        &config.replacen(
            "<Catalog>Items</Catalog>",
            "<Catalog>Items</Catalog><ChartOfAccounts>Ledger</ChartOfAccounts>",
            1,
        ),
    );
    write(
        &f.source.join(LEDGER),
        &fixture_text(
            "acceptance/workspace-chart-of-accounts-flags/src/ChartsOfAccounts/Ledger.xml",
        ),
    );
    f
}

fn read(f: &RealReaderFixture, at: &str) -> serde_json::Value {
    let r = f.view_service().view(ViewRequest::new(at).unwrap());
    assert!(r.ok, "{at}: {:?}", refusal_codes(&r));
    r.data.unwrap()
}

/// rule: CTR.SOURCE.EMPTY-BRANCHES
#[test]
fn accounting_flags_are_owner_branches_with_addressable_members() {
    let f = fixture();
    let before = fs::read(f.source.join(LEDGER)).unwrap();
    let root = read(&f, "main:ChartOfAccounts.Ledger");
    let branches = root["branches"].as_array().unwrap();
    for (at, count) in [
        ("main:ChartOfAccounts.Ledger.Attribute", 1),
        ("main:ChartOfAccounts.Ledger.AccountingFlag", 3),
        ("main:ChartOfAccounts.Ledger.ExtDimensionAccountingFlag", 2),
    ] {
        assert!(
            branches.contains(&json!({"at": at, "count": count})),
            "{at}: {branches:?}"
        );
    }
    for (kind, names) in [
        (
            "AccountingFlag",
            &["Currency", "Quantity", "TaxAccounting"][..],
        ),
        ("ExtDimensionAccountingFlag", &["Amount", "Currency"][..]),
    ] {
        let collection = read(&f, &format!("main:ChartOfAccounts.Ledger.{kind}"));
        let items = collection["items"].as_array().unwrap();
        assert_eq!(items.len(), names.len(), "{kind}");
        for (item, name) in items.iter().zip(names) {
            let at = format!("main:ChartOfAccounts.Ledger.{kind}.{name}");
            assert_eq!(item["at"], at);
            assert_eq!(item["kind"], kind);
            let detail = read(&f, &at);
            assert_eq!(detail["kind"], kind);
            assert_eq!(item["props"], detail["props"]);
            assert_eq!(
                detail["props"]["type"]["variants"],
                json!([{"kind": "boolean"}]),
                "{at}"
            );
        }
    }
    // Одноимённый обычный реквизит остаётся в своей ветви.
    let ordinary = read(&f, "main:ChartOfAccounts.Ledger.Attribute.Currency");
    assert_eq!(ordinary["kind"], "Attribute");
    // Русское написание вида ведёт к тому же узлу.
    let russian = read(&f, "main:ПланСчетов.Ledger.ПризнакУчетаСубконто.Amount");
    assert_eq!(
        russian["at"],
        "main:ChartOfAccounts.Ledger.ExtDimensionAccountingFlag.Amount"
    );
    assert_eq!(fs::read(f.source.join(LEDGER)).unwrap(), before);
}

/// rule: CTR.SOURCE.EMPTY-BRANCHES
#[test]
fn a_chart_without_flags_offers_empty_flag_branches() {
    let f = fixture();
    let path = f.source.join(LEDGER);
    let xml = fs::read_to_string(&path).unwrap();
    let kept = xml
        .lines()
        .filter(|line| !line.contains("AccountingFlag>"))
        .collect::<Vec<_>>()
        .join("\n");
    write(&path, &kept);
    let root = read(&f, "main:ChartOfAccounts.Ledger");
    let branches = root["branches"].as_array().unwrap();
    for kind in ["AccountingFlag", "ExtDimensionAccountingFlag"] {
        let at = format!("main:ChartOfAccounts.Ledger.{kind}");
        assert!(
            branches.contains(&json!({"at": at, "count": 0})),
            "{at}: {branches:?}"
        );
        assert_eq!(read(&f, &at)["items"], json!([]));
    }
}

#[test]
fn flags_belong_only_to_a_chart_of_accounts() {
    let f = fixture();
    let root = read(&f, "main:Catalog.Items");
    assert!(!root["branches"]
        .as_array()
        .unwrap()
        .iter()
        .any(|b| b["at"].as_str().unwrap().contains("AccountingFlag")));
    let r = f
        .view_service()
        .view(ViewRequest::new("main:Catalog.Items.AccountingFlag").unwrap());
    assert!(!r.ok);
}
