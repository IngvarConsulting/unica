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

/// Таблицы, кубы, таблицы измерения и функции живут во внешнем источнике:
/// корень конфигурации их ветвей не выдаёт, ни пустыми, ни со счётом.
/// rule: CTR.SOURCE.EMPTY-BRANCHES
#[test]
fn the_configuration_root_has_no_branches_of_nested_kinds() {
    let f = fixture();
    let root = read(&f, "main:Configuration");
    let branches = root["branches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|branch| branch["at"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(
        branches.contains(&"main:ExternalDataSource"),
        "{branches:?}"
    );
    for kind in ["Table", "Cube", "DimensionTable", "Function"] {
        assert!(
            !branches.contains(&format!("main:{kind}").as_str()),
            "{kind}: {branches:?}"
        );
    }
}
const EDS: &str = "main:ExternalDataSource.AllKindsExternalSource";

fn branches(node: &serde_json::Value) -> Vec<(String, u64)> {
    node["branches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|branch| {
            (
                branch["at"]
                    .as_str()
                    .unwrap()
                    .rsplit('.')
                    .next()
                    .unwrap()
                    .to_string(),
                branch["count"].as_u64().unwrap(),
            )
        })
        .collect()
}

fn titles(node: &serde_json::Value) -> Vec<&str> {
    node["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["title"].as_str().unwrap())
        .collect()
}

/// Таблицы, кубы и функции источника, поля таблиц, таблицы измерения,
/// измерения и ресурсы куба читаются на полной глубине; исходники не
/// меняются.
/// rule: INV.SOURCE.METADATA-NODE-PROPERTIES
#[test]
fn the_subordinates_of_an_external_data_source_are_readable_at_full_depth() {
    let f = fixture();
    let export = f.source.join("ExternalDataSources");
    let before = walk(&export)
        .into_iter()
        .map(|path| (fs::read(&path).unwrap(), path))
        .collect::<Vec<_>>();

    let source = read(&f, EDS);
    assert_eq!(
        branches(&source),
        [
            ("Table".to_string(), 1),
            ("Cube".to_string(), 1),
            ("Function".to_string(), 1)
        ]
    );
    let tables = read(&f, &format!("{EDS}.Table"));
    assert_eq!(titles(&tables), ["Items"]);
    assert_eq!(tables["items"][0]["at"], format!("{EDS}.Table.Items"));

    let table = read(&f, &format!("{EDS}.Table.Items"));
    assert_eq!(table["kind"], "Table");
    assert_eq!(
        branches(&table),
        [("Field".to_string(), 1), ("Module".to_string(), 3)]
    );
    let field = read(&f, &format!("{EDS}.Table.Items.Field.Code"));
    assert_eq!(field["kind"], "Field");
    assert_eq!(field["props"]["type"]["variants"][0]["kind"], "string");

    let cube = read(&f, &format!("{EDS}.Cube.Sales"));
    assert_eq!(
        branches(&cube),
        [
            ("DimensionTable".to_string(), 1),
            ("Dimension".to_string(), 1),
            ("Resource".to_string(), 1),
            ("Module".to_string(), 2)
        ]
    );
    let resource = read(&f, &format!("{EDS}.Cube.Sales.Resource.Amount"));
    assert_eq!(resource["props"]["type"]["variants"][0]["kind"], "string");
    // Тип измерения — ссылка на таблицу измерения куба; модель наблюдаемого
    // типа её не выражает и называет пределом, а не выдаёт пустой тип.
    let dimension = read(&f, &format!("{EDS}.Cube.Sales.Dimension.Product"));
    assert_eq!(dimension["props"]["incomplete"], true);
    assert!(
        dimension["limits"][0]
            .as_str()
            .unwrap()
            .contains("ExternalDataSourceCubeDimensionTableRef"),
        "{dimension}"
    );

    let dimension_table = read(&f, &format!("{EDS}.Cube.Sales.DimensionTable.Products"));
    assert_eq!(
        branches(&dimension_table),
        [("Field".to_string(), 1), ("Module".to_string(), 2)]
    );
    let fields = read(
        &f,
        &format!("{EDS}.Cube.Sales.DimensionTable.Products.Field"),
    );
    assert_eq!(titles(&fields), ["Code"]);

    let function = read(&f, &format!("{EDS}.Function.Total"));
    assert_eq!(function["kind"], "Function");
    assert_eq!(function["props"]["type"]["variants"][0]["length"], 10);

    // Русские написания видов ведут к тем же узлам.
    let russian = read(
        &f,
        "main:ВнешнийИсточникДанных.AllKindsExternalSource.Куб.Sales.ТаблицаИзмерения.Products.Поле.Code",
    );
    assert_eq!(
        russian["at"],
        format!("{EDS}.Cube.Sales.DimensionTable.Products.Field.Code")
    );

    for (bytes, path) in before {
        assert_eq!(fs::read(&path).unwrap(), bytes, "{}", path.display());
    }
}

/// Синоним таблицы читается в форме НСтр.
/// rule: INV.SOURCE.METADATA-NODE-PROPERTIES
#[test]
fn the_synonym_of_an_external_data_source_table_is_read() {
    let f = fixture();
    let table = f
        .source
        .join("ExternalDataSources/AllKindsExternalSource/Tables/Items.xml");
    // Первый синоним описания — синоним самой таблицы, до её полей.
    let text = fs::read_to_string(&table).unwrap().replacen(
        "<Synonym/>",
        "<Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Товары</v8:content></v8:item></Synonym>",
        1,
    );
    assert!(text.contains("Товары"));
    write(&table, &text);
    let node = read(&f, &format!("{EDS}.Table.Items"));
    assert_eq!(node["props"]["Synonym"], "ru = 'Товары'", "{node}");
}

/// Таблица, которую источник не регистрирует, и её поля не читаются: файл
/// на диске не доказывает объект.
/// rule: INV.SOURCE.LOGICAL-READER-PARITY
#[test]
fn an_unregistered_external_data_source_table_is_not_readable() {
    let f = fixture();
    let source = f.source.join(SOURCE);
    let text = fs::read_to_string(&source).unwrap();
    write(&source, &text.replacen("<Table>Items</Table>", "", 1));
    for at in [
        format!("{EDS}.Table.Items"),
        format!("{EDS}.Table.Items.Field.Code"),
    ] {
        let r = f.view_service().view(ViewRequest::new(&at).unwrap());
        assert_eq!(refusal_codes(&r), ["not_found"], "{at}: {r:?}");
    }
}

/// Форма вложенного объекта не адресуется: зарегистрированная форма
/// называется пределом, а не выдаётся ветвью без открываемых узлов.
#[test]
fn a_form_of_an_external_data_source_table_is_reported_as_a_limit() {
    let f = fixture();
    let table = f
        .source
        .join("ExternalDataSources/AllKindsExternalSource/Tables/Items.xml");
    let text = fs::read_to_string(&table).unwrap().replacen(
        "<ChildObjects>",
        "<ChildObjects><Form>ItemForm</Form>",
        1,
    );
    write(&table, &text);
    let node = read(&f, &format!("{EDS}.Table.Items"));
    assert!(
        !branches(&node).iter().any(|(kind, _)| kind == "Form"),
        "{node}"
    );
    assert!(
        node["limits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|limit| limit.as_str().unwrap().contains("forms of a nested object")),
        "{node}"
    );
}
