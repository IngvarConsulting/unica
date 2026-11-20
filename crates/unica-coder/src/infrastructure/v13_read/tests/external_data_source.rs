use super::*;

const SOURCE: &str = "ExternalDataSources/AllKindsExternalSource.xml";
const EXPORT: &str = "formats/platform_xml/2.20/all_kinds/src";

/// Рабочая область с внешним источником данных из выгрузки платформы:
/// описание источника и файлы его таблиц и кубов.
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
    for entry in walk(&export.join("ExternalDataSources")) {
        let relative = entry.strip_prefix(&export).unwrap();
        write(
            &f.source.join(relative),
            &fs::read_to_string(&entry).unwrap(),
        );
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

#[test]
fn an_external_data_source_is_a_readable_root_object() {
    let f = fixture();
    let before = fs::read(f.source.join(SOURCE)).unwrap();
    let source = read(&f, "main:ExternalDataSource.AllKindsExternalSource");
    assert_eq!(source["kind"], "ExternalDataSource");
    // Русское написание вида ведёт к тому же узлу.
    let russian = read(&f, "main:ВнешнийИсточникДанных.AllKindsExternalSource");
    assert_eq!(
        russian["at"],
        "main:ExternalDataSource.AllKindsExternalSource"
    );
    let branch = read(&f, "main:ExternalDataSource");
    assert!(
        branch["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["at"] == "main:ExternalDataSource.AllKindsExternalSource"),
        "{branch}"
    );
    assert_eq!(fs::read(f.source.join(SOURCE)).unwrap(), before);
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

const DIMENSION_TABLE_MODULE: &str = "ExternalDataSources/AllKindsExternalSource/Cubes/Sales/DimensionTables/Products/Ext/ObjectModule.bsl";

/// Модули таблицы, куба и таблицы измерения куба читаются по адресу полной
/// глубины.
#[test]
fn the_modules_of_external_data_source_objects_are_readable_at_full_depth() {
    let f = fixture();
    write(
        &f.source.join(DIMENSION_TABLE_MODULE),
        "Процедура Проверить() Экспорт\r\nКонецПроцедуры\r\n",
    );
    for at in [
        "main:ExternalDataSource.AllKindsExternalSource.Table.Items.Module.Object",
        "main:ExternalDataSource.AllKindsExternalSource.Table.Items.Module.RecordSet",
        "main:ExternalDataSource.AllKindsExternalSource.Cube.Sales.Module.Manager",
        "main:ExternalDataSource.AllKindsExternalSource.Cube.Sales.DimensionTable.Products.Module.Manager",
    ] {
        read(&f, at);
    }
    let method = read(
        &f,
        "main:ВнешнийИсточникДанных.AllKindsExternalSource.Куб.Sales.ТаблицаИзмерения.Products.Module.Object.Method.Проверить",
    );
    assert_eq!(
        method["at"],
        "main:ExternalDataSource.AllKindsExternalSource.Cube.Sales.DimensionTable.Products.Module.Object.Method.Проверить"
    );
}

/// Модуль таблицы измерения, которую куб не регистрирует, не читается: файл
/// на диске не доказывает объект.
/// rule: INV.SOURCE.LOGICAL-READER-PARITY
#[test]
fn a_module_of_an_unregistered_dimension_table_is_not_readable() {
    let f = fixture();
    let cube = f
        .source
        .join("ExternalDataSources/AllKindsExternalSource/Cubes/Sales.xml");
    let text = fs::read_to_string(&cube).unwrap();
    write(
        &cube,
        &text.replacen("<DimensionTable>Products</DimensionTable>", "", 1),
    );
    let r = f.view_service().view(
        ViewRequest::new(
            "main:ExternalDataSource.AllKindsExternalSource.Cube.Sales.DimensionTable.Products.Module.Object",
        )
        .unwrap(),
    );
    assert_eq!(refusal_codes(&r), ["not_found"], "{r:?}");
}
