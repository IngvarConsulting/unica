use super::*;
use crate::test_support::tree_snapshot;

const SCHEMA: &str = "main:Report.ParityReport.Template.MainSchema";

/// Добавляет XML параметров без предварительного разбора, чтобы тесты
/// проверяли декодирование выражений штатным читателем СКД.
fn add_parameters(fixture: &RealReaderFixture, parameters: &str) {
    let path = fixture
        .source
        .join("Reports/ParityReport/Templates/MainSchema/Ext/Template.xml");
    let xml = fs::read_to_string(&path).unwrap();
    write(
        &path,
        &xml.replace(
            "</DataCompositionSchema>",
            &format!("{parameters}</DataCompositionSchema>"),
        ),
    );
}

#[test]
fn dcs_parameter_expression_reaches_collection_and_detail_without_writes() {
    let fixture = RealReaderFixture::new();
    add_parameters(
        &fixture,
        r#"<parameter>
            <name>НачалоПериода</name>
            <valueType><v8:Type>xs:dateTime</v8:Type></valueType>
            <expression>&amp;Период.ДатаНачала</expression>
            <useRestriction>true</useRestriction><availableAsField>false</availableAsField>
        </parameter>
        <parameter>
            <name>Условие</name>
            <valueType><v8:Type>xs:boolean</v8:Type></valueType>
            <expression>&amp;НачалоПериода &lt; &amp;КонецПериода</expression>
        </parameter>"#,
    );
    let before = tree_snapshot(&fixture.source);
    let service = fixture.view_service();
    let expected = [
        (
            "Period",
            json!({"type":"DateTime", "value":null, "expression":null,
            "restricted":false, "availableAsField":null}),
        ),
        (
            "НачалоПериода",
            json!({"type":"DateTime", "value":null,
            "expression":"&Период.ДатаНачала", "restricted":true, "availableAsField":false}),
        ),
        (
            "Условие",
            json!({"type":"Boolean", "value":null,
            "expression":"&НачалоПериода < &КонецПериода", "restricted":false, "availableAsField":null}),
        ),
    ];
    for suffix in ["Parameter", "DataSet.MainData.Parameter"] {
        let branch = format!("{SCHEMA}.{suffix}");
        let request = ViewRequest::new(&branch).unwrap().with_limit(1).unwrap();
        let mut cursor = None;
        for (index, (name, props)) in expected.iter().enumerate() {
            let mut page_request = request.clone();
            if let Some(previous) = cursor.take() {
                page_request = page_request.with_cursor(previous);
            }
            let page = service.view(page_request);
            assert!(page.ok, "{:?}", refusal_codes(&page));
            let data = page.data.unwrap();
            let items = data["items"].as_array().unwrap();
            assert_eq!(items.len(), 1);
            let item = &items[0];
            assert_eq!(item["at"], format!("{branch}.{name}"));
            assert_eq!(item["kind"], "Parameter");
            assert_eq!(item["title"], *name);
            assert_eq!(&item["props"], props);
            let detail = service.view(ViewRequest::new(item["at"].as_str().unwrap()).unwrap());
            assert!(detail.ok, "{:?}", refusal_codes(&detail));
            let detail = detail.data.unwrap();
            assert_eq!(detail["at"], item["at"]);
            assert_eq!(&detail["props"], props);
            cursor = page.cursor;
            assert_eq!(cursor.is_some(), index + 1 < expected.len());
        }
    }
    assert_eq!(tree_snapshot(&fixture.source), before);
}

#[test]
fn dcs_parameter_expression_obeys_compact_property_byte_limit() {
    let at_limit = format!("\"{}\"", "Я".repeat(1023));
    let over_limit = format!("\"{}a\"", "Я".repeat(1023));
    assert_eq!(at_limit.len(), 2048);
    assert_eq!(over_limit.len(), 2049);
    let fixture = RealReaderFixture::new();
    add_parameters(
        &fixture,
        &format!(
            "<parameter><name>Fits</name><expression>{at_limit}</expression></parameter>\
             <parameter><name>Oversized</name><expression>{over_limit}</expression></parameter>"
        ),
    );
    let service = fixture.view_service();
    for suffix in ["Parameter", "DataSet.MainData.Parameter"] {
        for (name, expected) in [("Fits", Some(json!(at_limit))), ("Oversized", None)] {
            let result =
                service.view(ViewRequest::new(&format!("{SCHEMA}.{suffix}.{name}")).unwrap());
            assert!(result.ok, "{:?}", refusal_codes(&result));
            let data = result.data.unwrap();
            assert_eq!(data["props"].get("expression"), expected.as_ref());
            assert_eq!(data["props"]["restricted"], false);
            assert_eq!(data["title"], name);
        }
    }
}
