use super::*;

const ITEMS: &str = "<Item><Metadata>Catalog.Items</Metadata><AutoRecord>Deny</AutoRecord></Item>";
const ORDER: &str =
    "<Item><Metadata>Document.Order</Metadata><AutoRecord>Allow</AutoRecord></Item>";

/// Регистрирует план обмена `name` и, если `content` задан, пишет его
/// `Ext/Content.xml` с этими записями.
fn exchange_plan(fixture: &RealReaderFixture, name: &str, content: Option<&str>) {
    let config_path = fixture.source.join("Configuration.xml");
    let config = fs::read_to_string(&config_path).unwrap();
    write(
        &config_path,
        &config.replacen(
            "</ChildObjects>",
            &format!("<ExchangePlan>{name}</ExchangePlan></ChildObjects>"),
            1,
        ),
    );
    write(
        &fixture.source.join(format!("ExchangePlans/{name}.xml")),
        &format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><ExchangePlan uuid="11223344-5566-7788-99aa-{:012x}"><Properties><Name>{name}</Name><DistributedInfoBase>false</DistributedInfoBase></Properties><ChildObjects/></ExchangePlan></MetaDataObject>"#,
            name.len()
        ),
    );
    if let Some(content) = content {
        write(
            &fixture
                .source
                .join(format!("ExchangePlans/{name}/Ext/Content.xml")),
            &format!(
                r#"<ExchangePlanContent xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20">{content}</ExchangePlanContent>"#
            ),
        );
    }
}

/// Признак каждого элемента ветви: `Some(bool)` или `None`, если поля нет.
fn membership(
    fixture: &RealReaderFixture,
    branch: &str,
) -> (Vec<(String, Option<bool>)>, Vec<String>) {
    let result = fixture
        .view_service()
        .view(ViewRequest::new(branch).unwrap());
    assert!(result.ok, "{branch}: {:?}", refusal_codes(&result));
    let data = result.data.unwrap();
    let items = data["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["title"].as_str().unwrap().to_string(),
                item["props"]["inExchangePlan"].as_bool(),
            )
        })
        .collect();
    let limits = data["limits"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::to_string)
        .collect();
    (items, limits)
}

fn flag(items: &[(String, Option<bool>)], name: &str) -> Option<bool> {
    items
        .iter()
        .find(|(title, _)| title == name)
        .unwrap_or_else(|| panic!("{name} is not in {items:?}"))
        .1
}

/// Объект, входящий в два плана, и объект вне всех планов различаются
/// признаком ветви; режим авторегистрации объект из состава не исключает.
#[test]
fn branch_items_say_whether_an_object_is_in_any_exchange_plan() {
    let fixture = RealReaderFixture::new();
    exchange_plan(&fixture, "Shops", Some(&format!("{ITEMS}{ORDER}")));
    exchange_plan(
        &fixture,
        "Warehouses",
        Some(&ITEMS.replace("Deny", "Allow")),
    );
    let (catalogs, limits) = membership(&fixture, "main:Catalog");
    assert_eq!(flag(&catalogs, "Items"), Some(true));
    assert_eq!(flag(&catalogs, "Владельцы"), Some(false));
    assert!(limits.is_empty(), "{limits:?}");
    let (documents, _) = membership(&fixture, "main:Document");
    assert_eq!(flag(&documents, "Order"), Some(true));
    // Журнал документов в состав плана не входит: признака у него нет.
    let (journals, _) = membership(&fixture, "main:DocumentJournal");
    assert_eq!(flag(&journals, "Журнал"), None);
}

/// Пустой состав — известное «нет»: у плана без `Content.xml` и без планов
/// вовсе признак `false`, а не отсутствует.
#[test]
fn an_empty_composition_makes_every_object_absent() {
    let fixture = RealReaderFixture::new();
    let (catalogs, limits) = membership(&fixture, "main:Catalog");
    assert_eq!(flag(&catalogs, "Items"), Some(false));
    assert!(limits.is_empty(), "{limits:?}");

    let fixture = RealReaderFixture::new();
    exchange_plan(&fixture, "Shops", None);
    let (catalogs, limits) = membership(&fixture, "main:Catalog");
    assert_eq!(flag(&catalogs, "Items"), Some(false));
    assert_eq!(flag(&catalogs, "Владельцы"), Some(false));
    assert!(limits.is_empty(), "{limits:?}");
}

/// Непрочитанный состав не становится пустым: объект из прочитанного плана
/// остаётся в составе, у остальных признака нет, а ветвь называет план.
/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
#[test]
fn an_unreadable_composition_leaves_the_flag_unknown() {
    for damaged in [
        "<Item><Metadata>Document.Order</Metadata><AutoRecord>Unknown</AutoRecord></Item>",
        "<Item>",
    ] {
        let fixture = RealReaderFixture::new();
        exchange_plan(&fixture, "Shops", Some(ITEMS));
        exchange_plan(&fixture, "Warehouses", Some(damaged));
        let (catalogs, limits) = membership(&fixture, "main:Catalog");
        assert_eq!(flag(&catalogs, "Items"), Some(true), "{damaged}");
        assert_eq!(flag(&catalogs, "Владельцы"), None, "{damaged}");
        assert!(
            limits
                .iter()
                .any(|limit| limit.starts_with("inExchangePlan")
                    && limit.contains("ExchangePlan.Warehouses")),
            "{damaged}: {limits:?}"
        );
    }
}

/// Состав плана, который конфигурация не регистрирует, признака не даёт.
/// rule: INV.SOURCE.LOGICAL-READER-PARITY
#[test]
fn an_unregistered_plan_does_not_add_members() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture.source.join("ExchangePlans/Ghost/Ext/Content.xml"),
        &format!(
            r#"<ExchangePlanContent xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20">{ITEMS}</ExchangePlanContent>"#
        ),
    );
    let (catalogs, _) = membership(&fixture, "main:Catalog");
    assert_eq!(flag(&catalogs, "Items"), Some(false));
}

/// Потоковое чтение большого `Configuration.xml` даёт тот же признак, что
/// и ограниченное.
#[test]
fn the_streaming_reader_gives_the_same_membership() {
    let fixture = RealReaderFixture::new();
    exchange_plan(&fixture, "Shops", Some(ITEMS));
    let bounded = membership(&fixture, "main:Catalog");
    assert_eq!(flag(&bounded.0, "Items"), Some(true));
    let path = fixture.source.join("Configuration.xml");
    let xml = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        xml.replace(
            "</MetaDataObject>",
            &format!("<!--{}--></MetaDataObject>", "x".repeat(8 * 1024 * 1024)),
        ),
    )
    .unwrap();
    assert_eq!(membership(&fixture, "main:Catalog"), bounded);
}

/// Повреждённое описание плана не отнимает у ветви её объекты: признак
/// объектов вне прочитанных составов неизвестен, ветвь называет план.
/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
#[test]
fn a_damaged_plan_descriptor_keeps_the_branch_readable() {
    let fixture = RealReaderFixture::new();
    exchange_plan(&fixture, "Shops", Some(ITEMS));
    exchange_plan(&fixture, "Warehouses", Some(ORDER));
    fs::remove_file(fixture.source.join("ExchangePlans/Warehouses.xml")).unwrap();
    let (catalogs, limits) = membership(&fixture, "main:Catalog");
    assert_eq!(flag(&catalogs, "Items"), Some(true));
    assert_eq!(flag(&catalogs, "Владельцы"), None);
    assert!(
        limits
            .iter()
            .any(|limit| limit.contains("ExchangePlan.Warehouses")),
        "{limits:?}"
    );
}

/// Изменённый состав виден следующему вызову: сохранённое между вызовами
/// не переживает правку файла.
#[test]
fn a_changed_composition_is_seen_by_the_next_call() {
    let fixture = RealReaderFixture::new();
    exchange_plan(&fixture, "Shops", Some(ITEMS));
    let (catalogs, _) = membership(&fixture, "main:Catalog");
    assert_eq!(flag(&catalogs, "Items"), Some(true));
    write(
        &fixture.source.join("ExchangePlans/Shops/Ext/Content.xml"),
        &format!(
            r#"<ExchangePlanContent xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20">{ORDER}</ExchangePlanContent>"#
        ),
    );
    let (catalogs, _) = membership(&fixture, "main:Catalog");
    assert_eq!(flag(&catalogs, "Items"), Some(false));
}
