use super::*;

const SOURCE: &str = "ExternalDataSources/AllKindsExternalSource.xml";
const EXPORT: &str = "formats/platform_xml/2.20/all_kinds/src";

/// Рабочая область с внешним источником данных из выгрузки платформы:
/// описание источника и файлы его таблиц, кубов и функций.
fn fixture() -> RealReaderFixture {
    let f = RealReaderFixture::new();
    let config_path = f.source.join("Configuration.xml");
    let config = fs::read_to_string(&config_path).unwrap();
    write(
        &config_path,
        &config.replacen(
            "<Catalog>Items</Catalog>",
            "<Catalog>Items</Catalog><ExternalDataSource>AllKindsExternalSource</ExternalDataSource>",
            1,
        ),
    );
    let export = fixture_path(EXPORT);
    let owner = export.join("ExternalDataSources");
    for entry in walk(&owner) {
        let relative = entry.strip_prefix(&export).unwrap();
        write(&f.source.join(relative), &fs::read_to_string(&entry).unwrap());
    }
    f
}

fn walk(directory: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else {
            files.push(path);
        }
    }
    files
}

fn read(f: &RealReaderFixture, at: &str) -> serde_json::Value {
    let r = f.view_service().view(ViewRequest::new(at).unwrap());
    assert!(r.ok, "{at}: {:?}", refusal_codes(&r));
    r.data.unwrap()
}

/// Число объектов вида в `ChildObjects` описания источника: таблицы и кубы
/// он называет по имени, функции описывает на месте.
fn registered(f: &RealReaderFixture, kind: &str) -> usize {
    let text = fs::read_to_string(f.source.join(SOURCE)).unwrap();
    text.matches(&format!("<{kind}>")).count() + text.matches(&format!("<{kind} uuid=")).count()
}

/// rule: CTR.SOURCE.EMPTY-BRANCHES
#[test]
fn an_external_data_source_lists_its_tables_cubes_and_functions() {
    let f = fixture();
    let before = fs::read(f.source.join(SOURCE)).unwrap();
    let root = read(&f, "main:ExternalDataSource.AllKindsExternalSource");
    assert_eq!(root["kind"], "ExternalDataSource");
    let branches = root["branches"].as_array().unwrap();
    for kind in ["Table", "Cube", "Function"] {
        let at = format!("main:ExternalDataSource.AllKindsExternalSource.{kind}");
        let count = registered(&f, kind);
        assert!(
            branches.contains(&json!({"at": at, "count": count})),
            "{at}: {branches:?}"
        );
    }
    let tables = read(&f, "main:ExternalDataSource.AllKindsExternalSource.Table");
    assert_eq!(
        tables["items"][0]["at"],
        "main:ExternalDataSource.AllKindsExternalSource.Table.Items"
    );
    // Русское написание вида ведёт к тому же узлу.
    let russian = read(&f, "main:ВнешнийИсточникДанных.AllKindsExternalSource");
    assert_eq!(
        russian["at"],
        "main:ExternalDataSource.AllKindsExternalSource"
    );
    assert_eq!(fs::read(f.source.join(SOURCE)).unwrap(), before);
}

/// rule: CTR.SOURCE.EMPTY-BRANCHES
#[test]
fn a_source_without_cubes_and_functions_offers_their_empty_branches() {
    let f = fixture();
    let path = f.source.join(SOURCE);
    let xml = fs::read_to_string(&path).unwrap();
    let start = xml.find("<Cube>").unwrap();
    let end = xml.find("</ChildObjects>").unwrap();
    write(&path, &format!("{}{}", &xml[..start], &xml[end..]));
    let root = read(&f, "main:ExternalDataSource.AllKindsExternalSource");
    let branches = root["branches"].as_array().unwrap();
    for kind in ["Cube", "Function"] {
        let at = format!("main:ExternalDataSource.AllKindsExternalSource.{kind}");
        assert!(
            branches.contains(&json!({"at": at, "count": 0})),
            "{at}: {branches:?}"
        );
        assert_eq!(read(&f, &at)["items"], json!([]));
    }
}

/// Роль с правом на внешний источник читается целиком: вид владельца права
/// известен.
/// rule: INV.SOURCE.ROLE-RIGHT-ADDRESSES
#[test]
fn a_role_with_a_right_on_an_external_data_source_is_readable() {
    let f = fixture();
    let rights_path = f.source.join("Roles/SalesReader/Ext/Rights.xml");
    let rights = fs::read_to_string(&rights_path).unwrap().replacen(
        "</Rights>",
        "<object><name>ExternalDataSource.AllKindsExternalSource</name><right><name>Use</name><value>true</value></right></object></Rights>",
        1,
    );
    write(&rights_path, &rights);
    read(&f, "main:Role.SalesReader");
    let right = read(
        &f,
        "main:Role.SalesReader.Right.ExternalDataSource_AllKindsExternalSource",
    );
    assert_eq!(right["props"]["objectKind"], "ExternalDataSource");
    assert_eq!(right["props"]["allowedCount"], 1);
}
